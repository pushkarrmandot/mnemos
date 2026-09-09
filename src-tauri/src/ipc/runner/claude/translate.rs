//! Pure per-frame translation: one `claude` stream-json frame -> zero or
//! more `AgentEvent`s.
//!
//! Frame shapes here are VERIFIED against real captures from `claude`
//! 2.1.239 (single-text turns, a multi-turn persistent process, and an
//! invalid-model error run) — see
//! `tests/fixtures/claude/*.jsonl` for exactly what that verification
//! covered. Notable findings:
//!
//! - `system`/`init` recurs on **every turn** of a persistent
//!   `--input-format stream-json` process, not only once at handshake. All
//!   of them are swallowed, not just the first.
//! - There is no separate top-level `error` frame `type` in practice —
//!   failure surfaces as `is_error: true` on the terminal `result` frame.
//!   A `Frame::Error`-shaped match arm is kept for forward-compat but is
//!   UNVERIFIED.
//! - A real `rate_limit_event` frame type exists — mapped to
//!   `Notice{RateLimit}`.
//! - No `input_json_delta` streaming was observed without
//!   `--include-partial-messages`, which this runner never passes (v1 is
//!   text-only, no tool loop to justify the extra complexity) — each
//!   content block arrives complete in one `assistant` frame.
//!
//! Verified against a real `claude` 2.1.240 install with a
//! real `--mcp-config` (`mnemos-mcp-server`): tool dispatch is entirely
//! the CLI's own job — it calls the MCP server directly, never asking Rust
//! to. What Rust sees is a `tool_use` block on an `assistant` frame
//! (translated, but purely informational — see `AgentEvent::ToolCall`'s
//! doc comment) followed later by a `user`
//! frame whose `message.content` holds a `tool_result` block keyed by the
//! same `tool_use_id`. The tool's name arrives CLI-mangled —
//! `mcp__<server>__<tool_name_with_dots_replaced_by_underscores>` (e.g.
//! `mnemos.list_action_items` on server `mnemos` becomes
//! `mcp__mnemos__mnemos_list_action_items`) — passed straight through
//! rather than reverse-engineered; the chat UI owns pretty-printing it
//! for the tool-disclosure row.

use serde_json::Value;

use crate::error::AppError;
use crate::ipc::runner::{AgentEvent, NoticeKind, StopReason, TurnId, Usage};

/// Translates one already-parsed frame into zero or more events. The
/// caller (`claude/mod.rs`'s turn-drain loop) is responsible for treating a
/// `Complete`/`Error` event as terminal.
pub fn translate(turn_id: &TurnId, frame: &Value) -> Vec<AgentEvent> {
    match frame.get("type").and_then(Value::as_str).unwrap_or("") {
        "system" => translate_system(turn_id, frame),
        "assistant" => translate_assistant(turn_id, frame),
        "user" => translate_user(turn_id, frame),
        "rate_limit_event" => translate_rate_limit(turn_id, frame).into_iter().collect(),
        "result" => vec![translate_result(turn_id, frame)],
        // UNVERIFIED — no live invocation during this wave produced a
        // top-level `error` frame; kept for forward-compat per the LLD.
        "error" => vec![translate_error_frame(turn_id, frame)],
        other => vec![AgentEvent::Notice {
            turn_id: turn_id.clone(),
            notice_kind: NoticeKind::Info,
            text: format!("unmapped: {other}"),
        }],
    }
}

pub fn is_terminal(events: &[AgentEvent]) -> bool {
    events
        .iter()
        .any(|e| matches!(e, AgentEvent::Complete { .. } | AgentEvent::Error { .. }))
}

fn translate_system(turn_id: &TurnId, frame: &Value) -> Vec<AgentEvent> {
    let subtype = frame.get("subtype").and_then(Value::as_str).unwrap_or("");
    if subtype == "init" {
        return vec![]; // swallowed every time it appears, not just the first.
    }
    let text = frame
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| subtype.to_string());
    vec![AgentEvent::Notice {
        turn_id: turn_id.clone(),
        notice_kind: NoticeKind::Info,
        text: format!("system.{subtype}: {text}"),
    }]
}

fn translate_assistant(turn_id: &TurnId, frame: &Value) -> Vec<AgentEvent> {
    let Some(content) = frame.pointer("/message/content").and_then(Value::as_array) else {
        return vec![];
    };
    let mut out = Vec::with_capacity(content.len());
    for block in content {
        match block.get("type").and_then(Value::as_str).unwrap_or("") {
            "text" => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    out.push(AgentEvent::TokenDelta {
                        turn_id: turn_id.clone(),
                        text: text.to_string(),
                    });
                }
            }
            // v1 does not surface chain-of-thought — a short
            // notice only, never the `thinking` text itself.
            "thinking" => out.push(AgentEvent::Notice {
                turn_id: turn_id.clone(),
                notice_kind: NoticeKind::Info,
                text: "assistant thinking".to_string(),
            }),
            "tool_use" => {
                let call_id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let tool_name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let args = block.get("input").cloned().unwrap_or(Value::Null);
                tracing::info!(
                    turn_id = %turn_id,
                    call_id = %call_id,
                    tool_name = %tool_name,
                    "runner.claude.tool_use"
                );
                out.push(AgentEvent::ToolCall {
                    turn_id: turn_id.clone(),
                    call_id,
                    human_readable: format!("Tool call: {tool_name}"),
                    tool_name,
                    args,
                });
            }
            _ => {}
        }
    }
    out
}

/// `type: "user"` frames are the CLI echoing a tool result back into the
/// transcript (the dispatch itself already happened, entirely inside the
/// CLI's own MCP client — see this module's doc comment).
/// Ordinary conversational `user` frames (the CLI echoing our own submitted
/// turn) carry a plain-text content block instead of `tool_result` and
/// produce no events here — nothing new to tell a consumer.
fn translate_user(turn_id: &TurnId, frame: &Value) -> Vec<AgentEvent> {
    let Some(content) = frame.pointer("/message/content").and_then(Value::as_array) else {
        return vec![];
    };
    content
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
        .map(|block| {
            let call_id = block
                .get("tool_use_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let is_error = block
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let raw = block.get("content").cloned().unwrap_or(Value::Null);
            AgentEvent::ToolResult {
                turn_id: turn_id.clone(),
                call_id,
                ok: !is_error,
                summary: summarize_tool_result(&raw),
                raw,
            }
        })
        .collect()
}

/// A short, human-scannable summary for the tool-disclosure row (the chat
/// UI owns the actual rendering) — the raw field already carries the full payload.
/// `content` arrives as a JSON-encoded string for every real v1 MCP tool
/// (`mnemos-mcp-server`'s `tools_call_result` always stringifies its
/// `structuredContent`), so that's the common case; the block/array shape a
/// built-in CLI tool result might use is handled generically as a fallback.
fn summarize_tool_result(raw: &Value) -> String {
    let text = match raw {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    const MAX_CHARS: usize = 300;
    if text.chars().count() <= MAX_CHARS {
        text
    } else {
        let truncated: String = text.chars().take(MAX_CHARS).collect();
        format!("{truncated}…")
    }
}

/// Real capture (`tests/fixtures/claude/rate_limit.jsonl`): the CLI emits
/// this frame near the start of *every* turn once utilization has crossed
/// `surpassedThreshold` — not once per threshold-crossing, so the raw
/// `status` string alone would repeat verbatim on every single message once
/// in the warning zone and tell the user nothing about *why*.
/// `utilization`/`rateLimitType` are real fields on the same frame; using
/// them makes the toast self-explanatory instead of looking like a raw
/// error code. Only `status: "allowed_warning"` has been observed against a real
/// account — other values (e.g. a `"denied"`/severe-warning tier) are
/// plausible from the field name alone but unverified, so `status` itself
/// is always included rather than guessed-at wording built around it.
/// Utilization at or above this surfaces a notice.
///
/// The CLI's own `surpassedThreshold` is 0.75 and it re-sends a
/// `rate_limit_event` near the start of *every* turn from there on, so
/// mirroring its threshold meant a banner on essentially every message. It
/// also emits plain `status: "allowed"` frames with no `utilization` or
/// `rateLimitType` at all during normal use, which fell through to the raw
/// `"rate_limit: allowed"` fallback below — a message that told the user
/// nothing and appeared constantly. Notices only earn their place when the
/// user is close enough to the limit to act on it.
const RATE_LIMIT_NOTICE_THRESHOLD: f64 = 0.95;

/// `None` when the frame isn't worth interrupting the user for. A status
/// that actually blocks work is always surfaced regardless of utilization
/// (it may arrive with no utilization field at all), since that one is not
/// noise — it explains why the turn failed.
fn translate_rate_limit(turn_id: &TurnId, frame: &Value) -> Option<AgentEvent> {
    let status = frame
        .pointer("/rate_limit_info/status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let utilization = frame
        .pointer("/rate_limit_info/utilization")
        .and_then(Value::as_f64);
    let window = frame
        .pointer("/rate_limit_info/rateLimitType")
        .and_then(Value::as_str)
        .map(humanize_rate_limit_window);
    // Substring match, not an equality list: only `allowed`/`allowed_warning`
    // have been seen against a real account, so the blocking tier's exact
    // spelling is unknown. Matching loosely errs toward showing a genuine
    // block rather than silently swallowing it.
    let blocked = status.contains("denied")
        || status.contains("blocked")
        || status.contains("exceeded")
        || status.contains("rejected");

    // `is_some_and`, not `is_none_or`: the latter is stable only from Rust
    // 1.82 and this crate's MSRV is 1.80.
    if !blocked && !utilization.is_some_and(|u| u >= RATE_LIMIT_NOTICE_THRESHOLD) {
        return None;
    }

    let text = match (utilization, window) {
        (Some(u), Some(w)) => {
            format!(
                "Claude usage: {}% of your {w} limit ({status}).",
                (u * 100.0).round() as i64
            )
        }
        _ => format!("rate_limit: {status}"),
    };
    Some(AgentEvent::Notice {
        turn_id: turn_id.clone(),
        notice_kind: NoticeKind::RateLimit,
        text,
    })
}

fn humanize_rate_limit_window(raw: &str) -> &str {
    match raw {
        "seven_day" => "weekly",
        "five_hour" => "5-hour",
        // Unverified window names pass through as-is rather than being
        // guessed at — still readable, just not prettified.
        other => other,
    }
}

/// Recognizes a provider-side usage-limit refusal in a terminal error
/// result, returning `Some(resets_at_epoch_seconds)` — with `None` inside
/// the `Some` when the text carries no timestamp.
///
/// **PROVISIONAL.** Unlike the rest of this module, this is *not* verified
/// against a real capture: no fixture in `tests/fixtures/claude/` contains a
/// genuine hard block (see this file's header on what is and isn't
/// verified). The `"Claude AI usage limit reached|<epoch>"` shape is the
/// documented public format, and the bare substring fallbacks below cover
/// wording drift. Deliberately conservative: an unrecognized refusal simply
/// stays an `AppError::Runner`, which is exactly today's behavior, so a
/// miss degrades to the status quo rather than to something worse. Replace
/// the matching with a fixture-backed parse once a real block is captured.
fn detect_usage_limit(message: &str) -> Option<Option<i64>> {
    let lowered = message.to_ascii_lowercase();
    if !(lowered.contains("usage limit") || lowered.contains("rate limit")) {
        return None;
    }
    // `Claude AI usage limit reached|1700000000` — the epoch is whatever
    // follows the final `|`, when present.
    let resets_at = message
        .rsplit_once('|')
        .and_then(|(_, tail)| tail.trim().parse::<i64>().ok());
    Some(resets_at)
}

fn translate_result(turn_id: &TurnId, frame: &Value) -> AgentEvent {
    let is_error = frame
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if is_error {
        // Some failures carry their text in `result`, others only in an
        // `errors` array with no `result` at all — a refused `--resume` is
        // the latter. Reading only `result` replaced the real cause with a
        // generic string, which is what made a stale session id look like
        // an unexplained turn failure.
        let message = frame
            .get("result")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                frame
                    .get("errors")
                    .and_then(Value::as_array)
                    .map(|errors| {
                        errors
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join("; ")
                    })
                    .filter(|joined| !joined.is_empty())
            })
            .unwrap_or_else(|| "claude CLI reported an error result".to_string());
        if let Some(resets_at) = detect_usage_limit(&message) {
            return AgentEvent::Error {
                turn_id: turn_id.clone(),
                error: AppError::RunnerBlocked {
                    runner: "claude".to_string(),
                    resets_at,
                    // User-facing verbatim (this variant's `Display` is the
                    // bare message) — so it says what happened, that
                    // nothing was lost, and what to do. The provider's own
                    // wording is logged separately below, not shown.
                    message:
                        "Claude usage limit reached. Your recording and transcript are saved — \
                              regenerate the summary once your limit resets."
                            .to_string(),
                    correlation_id: crate::error::correlation_id(),
                },
            };
        }
        return AgentEvent::Error {
            turn_id: turn_id.clone(),
            error: AppError::Runner {
                runner: "claude".to_string(),
                message,
                correlation_id: crate::error::correlation_id(),
            },
        };
    }
    let stop_reason = match frame.get("stop_reason").and_then(Value::as_str) {
        Some("max_tokens") => StopReason::MaxTokens,
        Some("stop_sequence") => StopReason::StopSequence,
        Some("tool_use") => StopReason::ToolUse,
        _ => StopReason::EndTurn,
    };
    let usage = frame.get("usage").map(parse_usage).unwrap_or_default();
    AgentEvent::Complete {
        turn_id: turn_id.clone(),
        stop_reason,
        usage,
    }
}

fn parse_usage(u: &Value) -> Usage {
    Usage {
        input_tokens: u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
        output_tokens: u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
        cache_creation_input_tokens: u
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .map(|v| v as u32),
        cache_read_input_tokens: u
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .map(|v| v as u32),
    }
}

fn translate_error_frame(turn_id: &TurnId, frame: &Value) -> AgentEvent {
    let message = frame
        .get("error")
        .and_then(Value::as_str)
        .or_else(|| frame.pointer("/error/message").and_then(Value::as_str))
        .unwrap_or("claude CLI emitted an error frame")
        .to_string();
    AgentEvent::Error {
        turn_id: turn_id.clone(),
        error: AppError::Runner {
            runner: "claude".to_string(),
            message,
            correlation_id: crate::error::correlation_id(),
        },
    }
}

#[cfg(test)]
mod tests {
    /// A refused `--resume` reports its cause in an `errors` array with no
    /// `result` field at all. Reading only `result` replaced that with a
    /// generic string, so a stale session id surfaced as an unexplained
    /// failure and the recovery path (which matches on the text) could
    /// never fire.
    #[test]
    fn an_error_frame_with_only_an_errors_array_keeps_its_real_message() {
        let frame = serde_json::json!({
            "type": "result",
            "subtype": "error_during_execution",
            "is_error": true,
            "session_id": "s1",
            "errors": ["No conversation found with session ID: abc-123"],
        });
        let event = translate_result(&"t1".to_string(), &frame);
        let AgentEvent::Error { error, .. } = event else {
            panic!("expected an error event");
        };
        assert!(
            error
                .to_string()
                .contains("No conversation found with session ID"),
            "the real cause must survive translation: {error}"
        );
    }

    use super::*;

    fn fixture(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/claude")
            .join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
    }

    fn replay(fixture_name: &str) -> Vec<AgentEvent> {
        let turn_id = "t1".to_string();
        let mut events = Vec::new();
        for line in fixture(fixture_name).lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(frame) = serde_json::from_str::<Value>(line) else {
                continue; // malformed-line handling is stream.rs's job, not translate's.
            };
            events.extend(translate(&turn_id, &frame));
        }
        events
    }

    #[test]
    fn simple_text_turn_yields_token_deltas_then_complete() {
        let events = replay("simple_text_turn.jsonl");
        assert!(events
            .iter()
            .any(|e| matches!(e, AgentEvent::TokenDelta { text, .. } if text == "OK")));
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Complete {
                stop_reason: StopReason::EndTurn,
                ..
            })
        ));
        // system/init (appears at least once in the fixture) must never
        // surface as a Notice.
        assert!(!events
            .iter()
            .any(|e| matches!(e, AgentEvent::Notice { text, .. } if text.contains("system.init"))));
    }

    #[test]
    fn model_error_result_yields_a_terminal_error_event() {
        let events = replay("model_error.jsonl");
        assert!(matches!(events.last(), Some(AgentEvent::Error { .. })));
    }

    /// The fixture's real frame is `allowed_warning` at 81% — above the
    /// CLI's own 0.75 warning threshold but below ours, and the CLI repeats
    /// it on every turn from 75% on. Staying quiet here is the point of
    /// `RATE_LIMIT_NOTICE_THRESHOLD`.
    #[test]
    fn rate_limit_below_threshold_is_not_surfaced() {
        let events = replay("rate_limit.jsonl");
        assert!(!events.iter().any(|e| matches!(
            e,
            AgentEvent::Notice {
                notice_kind: NoticeKind::RateLimit,
                ..
            }
        )));
    }

    #[test]
    fn rate_limit_at_threshold_is_surfaced_with_readable_text() {
        let turn_id: TurnId = "t1".to_string();
        let frame = serde_json::json!({
            "type": "rate_limit_event",
            "rate_limit_info": {
                "status": "allowed_warning",
                "rateLimitType": "seven_day",
                "utilization": 0.96
            }
        });
        let event = translate_rate_limit(&turn_id, &frame);
        assert!(matches!(
            event,
            Some(AgentEvent::Notice { ref text, .. })
                if text == "Claude usage: 96% of your weekly limit (allowed_warning)."
        ));
    }

    /// Plain `allowed` with no utilization at all is what the CLI sends on
    /// ordinary turns. It used to render as the meaningless string
    /// "rate_limit: allowed" on essentially every message.
    #[test]
    fn ordinary_allowed_frame_is_silent() {
        let turn_id: TurnId = "t1".to_string();
        let frame = serde_json::json!({
            "type": "rate_limit_event",
            "rate_limit_info": { "status": "allowed" }
        });
        assert!(translate_rate_limit(&turn_id, &frame).is_none());
    }

    /// A blocking status is surfaced even with no `utilization` field, and
    /// still falls back to the raw status text — it explains why a turn
    /// failed, so swallowing it would be worse than showing a terse string.
    #[test]
    fn blocking_status_is_surfaced_without_utilization() {
        let turn_id: TurnId = "t1".to_string();
        let frame = serde_json::json!({
            "type": "rate_limit_event",
            "rate_limit_info": { "status": "denied" }
        });
        let event = translate_rate_limit(&turn_id, &frame);
        assert!(matches!(
            event,
            Some(AgentEvent::Notice { ref text, .. }) if text == "rate_limit: denied"
        ));
    }

    /// A usage-limit refusal must become `RunnerBlocked`, not the generic
    /// `Runner` — the two get opposite user-facing treatment: the
    /// generic path gets retried by `agent_call.py` as a "malformed JSON"
    /// failure, which a usage-limit refusal must never be.
    #[test]
    fn usage_limit_result_becomes_runner_blocked_with_reset_time() {
        let turn_id: TurnId = "t1".to_string();
        let frame = serde_json::json!({
            "type": "result",
            "is_error": true,
            "result": "Claude AI usage limit reached|1700000000"
        });
        let event = translate_result(&turn_id, &frame);
        match event {
            AgentEvent::Error {
                error:
                    AppError::RunnerBlocked {
                        resets_at, message, ..
                    },
                ..
            } => {
                assert_eq!(resets_at, Some(1_700_000_000));
                // User-facing copy, not the provider's raw string.
                assert!(message.contains("usage limit"), "{message}");
                assert!(message.contains("saved"), "{message}");
            }
            other => panic!("expected RunnerBlocked, got {other:?}"),
        }
    }

    #[test]
    fn usage_limit_without_timestamp_still_blocks_with_no_reset_time() {
        let turn_id: TurnId = "t1".to_string();
        let frame = serde_json::json!({
            "type": "result",
            "is_error": true,
            "result": "You have hit your usage limit for this account."
        });
        assert!(matches!(
            translate_result(&turn_id, &frame),
            AgentEvent::Error {
                error: AppError::RunnerBlocked {
                    resets_at: None,
                    ..
                },
                ..
            }
        ));
    }

    /// The conservative half of the contract: an ordinary failure must NOT
    /// be misread as a quota block, or a genuinely broken CLI would be
    /// reported to the user as "come back later".
    #[test]
    fn ordinary_error_result_stays_a_generic_runner_error() {
        let turn_id: TurnId = "t1".to_string();
        let frame = serde_json::json!({
            "type": "result",
            "is_error": true,
            "result": "invalid model name: sonnet-99"
        });
        assert!(matches!(
            translate_result(&turn_id, &frame),
            AgentEvent::Error {
                error: AppError::Runner { .. },
                ..
            }
        ));
    }

    #[test]
    fn detect_usage_limit_ignores_a_trailing_non_numeric_pipe_segment() {
        // Guards the `rsplit_once('|')` parse against text that happens to
        // contain a pipe but no epoch — must block, just without a time.
        assert_eq!(
            detect_usage_limit("usage limit reached | contact your admin"),
            Some(None)
        );
    }

    #[test]
    fn unknown_top_level_type_degrades_to_info_notice() {
        let events = replay("unknown_frame.jsonl");
        assert!(events.iter().any(|e| matches!(
            e,
            AgentEvent::Notice { notice_kind: NoticeKind::Info, text, .. } if text.starts_with("unmapped:")
        )));
    }

    #[test]
    fn tool_use_with_no_result_frame_yet_still_surfaces_the_call() {
        // A `tool_use` whose turn ends before any `tool_result` frame arrives
        // is still a real, if edge-case, shape: e.g. the CLI process is
        // killed mid-dispatch.
        let events = replay("tool_use_inert.jsonl");
        let tool_calls: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::ToolCall { .. }))
            .collect();
        assert_eq!(tool_calls.len(), 1);
        assert!(!events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolResult { .. })));
        assert!(matches!(events.last(), Some(AgentEvent::Complete { .. })));
    }

    #[test]
    fn tool_use_followed_by_a_real_tool_result_frame_dispatches_both_events() {
        // `tool_use_and_result.jsonl` is hand-built (not a raw scrub) from
        // the exact frame shapes a real `claude` 2.1.240 run against a real
        // `mnemos-mcp-server` produced — the CLI
        // mangles the MCP tool name to `mcp__<server>__<tool>` and
        // stringifies the tool's JSON result into `tool_result.content`.
        let events = replay("tool_use_and_result.jsonl");
        let call = events
            .iter()
            .find_map(|e| match e {
                AgentEvent::ToolCall {
                    call_id, tool_name, ..
                } => Some((call_id.clone(), tool_name.clone())),
                _ => None,
            })
            .expect("expected a ToolCall event");
        assert_eq!(call.0, "toolu_scrubbed");
        assert_eq!(call.1, "mcp__mnemos__mnemos_list_action_items");

        let result = events
            .iter()
            .find_map(|e| match e {
                AgentEvent::ToolResult {
                    call_id,
                    ok,
                    summary,
                    ..
                } => Some((call_id.clone(), *ok, summary.clone())),
                _ => None,
            })
            .expect("expected a ToolResult event");
        assert_eq!(result.0, "toolu_scrubbed");
        assert!(result.1);
        assert!(result.2.contains("Send updated BOM"));

        assert!(events
            .iter()
            .any(|e| matches!(e, AgentEvent::TokenDelta { text, .. } if text.contains("one open action item"))));
        assert!(matches!(events.last(), Some(AgentEvent::Complete { .. })));
    }

    #[test]
    fn tool_result_with_is_error_true_maps_to_ok_false() {
        let turn_id = "t1".to_string();
        let frame = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "toolu_x",
                    "is_error": true,
                    "content": "permission denied"
                }]
            }
        });
        let events = translate(&turn_id, &frame);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            AgentEvent::ToolResult { ok: false, call_id, summary, .. }
                if call_id == "toolu_x" && summary == "permission denied"
        ));
    }

    #[test]
    fn is_terminal_matches_complete_and_error_only() {
        let turn_id = "t1".to_string();
        assert!(!is_terminal(&[AgentEvent::TokenDelta {
            turn_id: turn_id.clone(),
            text: "x".into()
        }]));
        assert!(is_terminal(&[AgentEvent::Complete {
            turn_id: turn_id.clone(),
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
        }]));
    }
}
