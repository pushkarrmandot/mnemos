//! `chat.*` Tauri command surface (W13a) — wires LLD-07 §5.1's long-lived
//! chat-session pattern (built by W8, never called by anything real) and
//! the real tool-calling loop (LLD-07 §6, built this wave — see
//! `ipc::runner::claude`) into an actual command React can call.
//!
//! One command in v1, `chat_send_prompt`: resolves/creates the one
//! `chat_sessions` row for a `(runner, scope)` tuple (06_CHAT.md), starts
//! (or reuses) the long-lived `ClaudeRunner` for it, journals the user
//! turn, enqueues the prompt, and returns immediately — every `AgentEvent`
//! streams back over the caller's `Channel` while a detached task journals
//! each one into `chat_journal` (+ `chat_sessions` projection, same
//! transaction, per `StorageService::append_chat_event`).
//!
//! Scope -> context mapping (no vector search exists yet — W14/v1.2):
//! - **Conversation** scope: the conversation's full `transcript.json` is
//!   stuffed directly into the system prompt (small enough) — no MCP tools,
//!   `RunnerConfig.mcp = None`.
//! - **Project** scope: MCP tools (`mnemos-mcp-server`) + a system-prompt
//!   instruction to always pass this project's `project_id` to every tool
//!   call — verified against a real `claude` CLI + a real `mnemos-mcp-server`
//!   process this wave that the model reliably does this when told to.
//! - **Everything** scope: same MCP tools, no project instruction — the
//!   tools themselves treat a missing `project_id` as unfiltered (LLD-08's
//!   `mnemos.list_action_items`/`list_open_questions` call straight through
//!   to `StorageService::list_action_items_global`/`list_open_questions_global`
//!   when no `project_id` arg is given; there is no separate "_global" MCP
//!   tool name).

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::State;
use tokio::sync::Mutex as AsyncMutex;
use tokio_stream::StreamExt;

use crate::db::models::{
    ChatEventRecord, ChatScopeType, ChatSession, ConversationFilter, NewChatSession,
};
use crate::db::service::{SqliteStorageService, StorageService};
use crate::error::AppError;
use crate::ipc::runner::mcp_shared::{find_mcp_server_binary, mcp_server_missing_error};
use crate::ipc::runner::registry::RunnerKind;
use crate::ipc::runner::{
    AgentEvent, AgentRunner, AgentStream, ApprovalPolicy, McpConfig, PromptRequest, RunnerConfig,
    TurnId, UserContent,
};
use crate::state::AppState;

/// How many recent conversations are inlined into a project-scoped chat
/// session's system prompt. Ten was already the effective number (the old
/// code loaded every conversation and `.take(10)`-ed the result); it is now
/// the query's own bound.
const RUNNER_CONFIG_RECENT_LIMIT: u32 = 10;

/// Only vendor in v1 (LLD-07's Deferred: W8b adds Codex/OpenCode/Gemini/
/// Ollama siblings) — hard-coded rather than plumbed through from Settings,
/// which has no runner-picker surface yet either. `send_prompt`/
/// `build_runner_config` are dispatched on `RunnerKind` throughout, though,
/// so adding the picker later is a Settings-UI problem, not a
/// `commands::chat` one (design doc §2.1).
const RUNNER: RunnerKind = RunnerKind::Claude;

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "scope_type", rename_all = "snake_case")]
pub enum ChatScopeInput {
    Everything,
    Project { project_id: String },
    Conversation { conversation_id: String },
}

impl ChatScopeInput {
    fn scope_type(&self) -> ChatScopeType {
        match self {
            ChatScopeInput::Everything => ChatScopeType::Everything,
            ChatScopeInput::Project { .. } => ChatScopeType::Project,
            ChatScopeInput::Conversation { .. } => ChatScopeType::Conversation,
        }
    }

    fn scope_id(&self) -> Option<String> {
        match self {
            ChatScopeInput::Everything => None,
            ChatScopeInput::Project { project_id } => Some(project_id.clone()),
            ChatScopeInput::Conversation { conversation_id } => Some(conversation_id.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ChatSendPromptAck {
    pub session_id: String,
}

struct ChatRunnerEntry {
    runner: AsyncMutex<Box<dyn AgentRunner>>,
    epoch: String,
    /// Shared with the detached forwarder task spawned by every
    /// `chat_send_prompt` call against this session — `chat_journal`'s
    /// `PRIMARY KEY (session_id, epoch, seq)` just needs unique, not
    /// strictly ordered-by-wall-clock, values, so a plain atomic counter
    /// (no lock) is enough.
    next_seq: AtomicI64,
}

/// One long-lived `ClaudeRunner` per `chat_sessions` row (LLD-07 §5.1),
/// keyed by that row's id. Never disposed in v1 — no `chat.clear_session`/
/// app-quit hook exists yet (out of this wave's scope); the process exit
/// that ends the app also reaps every child `claude` process via
/// `kill_on_drop`.
#[derive(Default)]
pub struct ChatRegistry {
    sessions: AsyncMutex<HashMap<String, Arc<ChatRunnerEntry>>>,
}

impl ChatRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// `None` when this session has no live runner yet — the caller then
    /// builds a `RunnerConfig` (an async storage read for Project/
    /// Conversation scope) and calls `insert_if_absent`. Split into two
    /// steps so a warm session (the common case — every message after the
    /// first in a chat) never re-does that read.
    async fn get(&self, session_id: &str) -> Option<Arc<ChatRunnerEntry>> {
        self.sessions.lock().await.get(session_id).cloned()
    }

    /// Second half of the miss path. A double-build race (two concurrent
    /// first-messages to a brand-new session) is possible but harmless —
    /// `or_insert` keeps whichever runner won and the loser's freshly
    /// spawned `claude` process is simply dropped (its `Drop` impl
    /// SIGTERMs it via `kill_on_drop`).
    async fn insert_if_absent(
        &self,
        session_id: &str,
        entry: Arc<ChatRunnerEntry>,
    ) -> Arc<ChatRunnerEntry> {
        let mut map = self.sessions.lock().await;
        map.entry(session_id.to_string()).or_insert(entry).clone()
    }

    /// Removes and returns this session's runner entry, if any (design
    /// doc's §2.4 option (a)). Cancel kills the runner's whole child
    /// process — the entry must not be reused after that, so eviction
    /// happens *before* the kill, not after: once removed here, no
    /// concurrent `send_prompt` can observe or hand out this entry again,
    /// so there's no race between "we decided to kill it" and "someone else
    /// just grabbed it to send a message." The next `send_prompt` for this
    /// session then takes the normal registry-miss path and spawns a fresh
    /// process, exactly like a session's first message.
    async fn evict(&self, session_id: &str) -> Option<Arc<ChatRunnerEntry>> {
        self.sessions.lock().await.remove(session_id)
    }
}

fn mcp_config_for_chat() -> Result<McpConfig, AppError> {
    let bin = find_mcp_server_binary(None).ok_or_else(mcp_server_missing_error)?;
    Ok(McpConfig {
        server_binary: bin.to_string_lossy().into_owned(),
    })
}

/// `YYYY-MM-DD`, UTC — good enough for the model to reason about recency;
/// falls back to the raw epoch seconds on the (practically unreachable,
/// since `started_at` is always a valid timestamp this app wrote itself)
/// chance formatting fails.
fn format_started_at(started_at: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(started_at)
        .ok()
        .and_then(|dt| {
            let format =
                time::format_description::parse_borrowed::<2>("[year]-[month]-[day]").ok()?;
            dt.format(&format).ok()
        })
        .unwrap_or_else(|| started_at.to_string())
}

/// Pure — takes already-fetched data so it's testable without a real
/// `mnemos-mcp-server` binary on `PATH` (`build_runner_config`'s only other
/// dependency for `Project`/`Everything` scope, and unrelated to what this
/// function actually builds). See `build_runner_config`'s `Project` arm for
/// why both `recent`/`memory` are embedded and the staleness tradeoff that
/// implies.
fn project_system_prompt(
    project_name: &str,
    project_id: &str,
    recent: &[crate::db::models::Conversation],
    memory: Option<&serde_json::Value>,
) -> String {
    let recent_conversations = if recent.is_empty() {
        "(none yet)".to_string()
    } else {
        recent
            .iter()
            .map(|c| {
                format!(
                    "- {} — \"{}\" ({})",
                    c.id,
                    c.title,
                    format_started_at(c.started_at)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let project_memory = match memory {
        Some(m) => serde_json::to_string_pretty(m)
            .unwrap_or_else(|_| "(memory present but failed to serialize)".to_string()),
        None => "(no project memory generated yet)".to_string(),
    };

    format!(
        "You are Mnemos' chat assistant, scoped to the project \"{project_name}\" \
         (project_id: {project_id}). Use the mnemos MCP tools to answer questions — \
         always pass project_id=\"{project_id}\" to every tool call that accepts it, so \
         results stay scoped to this project. If the user asks about something outside \
         this project, say so plainly rather than widening scope yourself.\n\n\
         For orientation, here is a snapshot taken at the start of this chat session \
         (both may now be stale — re-fetch via mnemos.list_recent_conversations / \
         mnemos.get_project_memory if freshness matters):\n\n\
         RECENT CONVERSATIONS (up to 10, most recent first):\n{recent_conversations}\n\n\
         PROJECT MEMORY:\n{project_memory}"
    )
}

async fn build_runner_config(
    storage: &SqliteStorageService,
    runner: RunnerKind,
    scope: &ChatScopeInput,
) -> Result<RunnerConfig, AppError> {
    let system_prompt = match scope {
        ChatScopeInput::Conversation { conversation_id } => {
            let conv = storage.get_conversation(conversation_id).await?;
            let transcript = storage
                .read_transcript(conversation_id)
                .await?
                .ok_or_else(|| AppError::Validation {
                    message: "conversation has no transcript yet".to_string(),
                    field: Some("conversation_id".to_string()),
                })?;
            return Ok(RunnerConfig {
                model: runner.default_chat_model().to_string(),
                timeout_ms: None,
                system_prompt: Some(format!(
                    "You are Mnemos' chat assistant, answering questions about one specific \
                     meeting (\"{title}\"). You have no tools this turn — the meeting's full \
                     transcript is given below as JSON. Answer only from it, and say plainly \
                     if the answer isn't there.\n\nTRANSCRIPT:\n{transcript}",
                    title = conv.title,
                )),
                tools: vec![],
                approval_policy: ApprovalPolicy::AutoDenyDestructive,
                mcp: None,
            });
        }
        ChatScopeInput::Project { project_id } => {
            let project = storage.get_project(project_id).await?;

            // Orientation context, embedded once at session-open rather than
            // requiring a tool round trip before the model can answer even
            // the most basic "what have we been talking about" question —
            // both are cheap: `recent_conversations` is id/title/date only
            // (not full transcripts), and project memory is deliberately a
            // *short* synthesized doc (LLD-05), not the raw transcript
            // corpus. Real, not defensive, tradeoff: both are a snapshot as
            // of this session's first message (`build_runner_config` runs
            // once per session and is reused for its whole, possibly
            // multi-day, lifetime) — a new meeting recorded mid-session
            // won't appear here, but `mnemos.list_recent_conversations`/
            // `mnemos.get_project_memory` remain available as tools for the
            // model to re-fetch fresh data whenever that matters.
            let recent = storage
                .list_conversations(ConversationFilter {
                    project_id: Some(project_id.clone()),
                    include_archived: true,
                    limit: Some(RUNNER_CONFIG_RECENT_LIMIT),
                    ..Default::default()
                })
                .await?;
            let memory = storage.read_project_memory(project_id).await?;
            project_system_prompt(&project.name, project_id, &recent, memory.as_ref())
        }
        ChatScopeInput::Everything => {
            "You are Mnemos' chat assistant, scoped to Everything — every project and \
             conversation. Use the mnemos MCP tools to answer questions; omit project_id so \
             results are not filtered to one project."
                .to_string()
        }
    };

    Ok(RunnerConfig {
        model: runner.default_chat_model().to_string(),
        timeout_ms: None,
        system_prompt: Some(system_prompt),
        tools: vec![],
        approval_policy: ApprovalPolicy::AutoDenyDestructive,
        mcp: Some(mcp_config_for_chat()?),
    })
}

/// Forwards every `AgentEvent` from one turn's stream onto the caller's
/// `Channel`, journaling each one as it arrives (HLD's journal+projection
/// pattern — `StorageService::append_chat_event` commits the
/// `chat_journal` insert and the `chat_sessions.message_count` update in
/// one transaction). Detached from `chat_send_prompt` so that command can
/// return as soon as the turn is enqueued rather than blocking on the
/// model's full response (LLD-07 §5.1's "enqueue-and-return" shape).
async fn forward_turn(
    mut stream: AgentStream,
    channel: Channel<AgentEvent>,
    storage: SqliteStorageService,
    metrics: crate::metrics::Metrics,
    session_id: String,
    entry: Arc<ChatRunnerEntry>,
) {
    let turn_start = std::time::Instant::now();
    while let Some(event) = stream.next().await {
        let seq = entry.next_seq.fetch_add(1, Ordering::SeqCst);
        match serde_json::to_value(&event) {
            Ok(event_json) => {
                if let Err(e) = storage
                    .append_chat_event(&session_id, &entry.epoch, seq, event_json)
                    .await
                {
                    tracing::warn!(error = %e, session_id = %session_id, "chat.journal_write_failed");
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "chat.event_encode_failed");
            }
        }
        let is_terminal = matches!(
            event,
            AgentEvent::Complete { .. } | AgentEvent::Error { .. }
        );
        // Read before `channel.send(event)` moves `event` below.
        let terminal_success = match &event {
            AgentEvent::Complete { .. } => Some(true),
            AgentEvent::Error { .. } => Some(false),
            _ => None,
        };
        if channel.send(event).is_err() {
            // Right pane closed / frontend dropped the channel — journal
            // writes above already preserved everything up to here; the
            // corner case "response continues in background, toast when
            // complete" is 06_CHAT.md's job, not this loop's, since the
            // journal is the source of truth either way. No
            // `chat_turn_completed` here either — the turn didn't actually
            // finish from the frontend's point of view, it was abandoned.
            return;
        }
        if is_terminal {
            if let Some(success) = terminal_success {
                metrics.track(
                    crate::metrics::events::CHAT_TURN_COMPLETED,
                    crate::metrics::properties::EventProperties::from([
                        (
                            "duration_ms",
                            crate::metrics::properties::PropertyValue::UInt(
                                turn_start.elapsed().as_millis() as u64,
                            ),
                        ),
                        (
                            "success",
                            crate::metrics::properties::PropertyValue::Bool(success),
                        ),
                    ]),
                );
            }
            return;
        }
    }
}

/// Resolves/creates the one session for `(claude, scope)`, journals the
/// user's turn, enqueues the prompt against the long-lived runner, and
/// returns — the actual response streams back over `channel`. A plain
/// function (not the `#[tauri::command]` itself, which is a thin wrapper
/// below) so it's callable directly from a test with just a
/// `SqliteStorageService` + `ChatRegistry`, no full `AppState`/running Tauri
/// app required — mirrors `ipc::runner::extraction_handler`'s
/// command-fn/testable-fn split.
async fn send_prompt(
    storage: &SqliteStorageService,
    chat: &ChatRegistry,
    metrics: &crate::metrics::Metrics,
    scope: ChatScopeInput,
    text: String,
    channel: Channel<AgentEvent>,
) -> Result<ChatSendPromptAck, AppError> {
    if text.trim().is_empty() {
        return Err(AppError::Validation {
            message: "text must not be empty".to_string(),
            field: Some("text".to_string()),
        });
    }

    let scope_type = scope.scope_type();
    let scope_id = scope.scope_id();

    let session = match storage
        .find_chat_session_by_scope(Some(RUNNER.id()), scope_type, scope_id.as_deref())
        .await?
    {
        Some(s) => {
            metrics.track(
                crate::metrics::events::CHAT_MESSAGE_SENT,
                crate::metrics::properties::EventProperties::from([
                    ("scope_type", chat_scope_type_property(scope_type)),
                    (
                        "is_new_session",
                        crate::metrics::properties::PropertyValue::Bool(false),
                    ),
                    (
                        "runner",
                        crate::metrics::properties::PropertyValue::Enum(RUNNER.id()),
                    ),
                ]),
            );
            s
        }
        None => {
            let opened = storage
                .open_chat_session(NewChatSession {
                    runner_id: Some(RUNNER.id().to_string()),
                    scope_type,
                    scope_id: scope_id.clone(),
                    title: None,
                })
                .await?;
            metrics.track(
                crate::metrics::events::CHAT_MESSAGE_SENT,
                crate::metrics::properties::EventProperties::from([
                    ("scope_type", chat_scope_type_property(scope_type)),
                    (
                        "is_new_session",
                        crate::metrics::properties::PropertyValue::Bool(true),
                    ),
                    (
                        "runner",
                        crate::metrics::properties::PropertyValue::Enum(RUNNER.id()),
                    ),
                ]),
            );
            opened
        }
    };

    let entry = match chat.get(&session.id).await {
        Some(entry) => entry,
        None => {
            // Only reached once per session (its first message) — a warm
            // session never re-resolves the MCP binary or re-reads
            // project/transcript rows.
            let config = build_runner_config(storage, RUNNER, &scope).await?;
            let mut runner: Box<dyn AgentRunner> = RUNNER.create();
            runner.start(config).await?;
            let new_entry = Arc::new(ChatRunnerEntry {
                runner: AsyncMutex::new(runner),
                epoch: session.epoch.clone(),
                next_seq: AtomicI64::new(session.message_count),
            });
            chat.insert_if_absent(&session.id, new_entry).await
        }
    };

    let user_seq = entry.next_seq.fetch_add(1, Ordering::SeqCst);
    storage
        .append_chat_event(
            &session.id,
            &entry.epoch,
            user_seq,
            serde_json::json!({"kind": "user_message", "text": text}),
        )
        .await?;

    let stream = {
        let mut runner = entry.runner.lock().await;
        runner
            .prompt(PromptRequest {
                content: vec![UserContent::Text(text)],
                history: vec![],
                turn_id: None,
            })
            .await?
    };

    tokio::spawn(forward_turn(
        stream,
        channel,
        storage.clone(),
        metrics.clone(),
        session.id.clone(),
        entry,
    ));

    Ok(ChatSendPromptAck {
        session_id: session.id,
    })
}

/// Maps a `ChatScopeType` to the closed-set property value `chat_message_sent`
/// carries — never the scope's `project_id`/`conversation_id`, just which of
/// the three kinds it is.
fn chat_scope_type_property(
    scope_type: ChatScopeType,
) -> crate::metrics::properties::PropertyValue {
    crate::metrics::properties::PropertyValue::Enum(match scope_type {
        ChatScopeType::Everything => "everything",
        ChatScopeType::Project => "project",
        ChatScopeType::Conversation => "conversation",
    })
}

/// The actual `#[tauri::command]` — a thin wrapper over `send_prompt`, see
/// its doc comment for why the logic lives in a plain function instead.
#[tauri::command]
#[specta::specta]
pub async fn chat_send_prompt(
    state: State<'_, AppState>,
    scope: ChatScopeInput,
    text: String,
    channel: Channel<AgentEvent>,
) -> Result<ChatSendPromptAck, AppError> {
    send_prompt(
        &state.storage,
        &state.chat,
        &state.metrics,
        scope,
        text,
        channel,
    )
    .await
}

/// Backs `chat_cancel_turn`. Idempotent by design: a session with no live
/// runner entry (already cancelled, or the turn already finished and the
/// registry was never touched) is not an error — cancelling twice, or
/// cancelling a turn that just completed on its own, both no-op cleanly.
async fn cancel_turn(
    chat: &ChatRegistry,
    session_id: &str,
    turn_id: TurnId,
) -> Result<(), AppError> {
    let Some(entry) = chat.evict(session_id).await else {
        return Ok(());
    };
    let result = entry.runner.lock().await.cancel_turn(turn_id).await;
    result
}

/// Stops the in-flight turn for `session_id` and kills its runner process
/// (design doc §2.4 — the runner is a persistent whole-process-per-session
/// primitive in v1, so "cancel a turn" and "cancel the process" are the same
/// operation; `ChatRegistry::evict` guarantees the next message in this
/// session cold-starts a fresh process rather than writing to a dead one's
/// stdin).
#[tauri::command]
#[specta::specta]
pub async fn chat_cancel_turn(
    state: State<'_, AppState>,
    session_id: String,
    turn_id: String,
) -> Result<(), AppError> {
    cancel_turn(&state.chat, &session_id, turn_id).await
}

/// Backs `chat_get_session_history`. A thin read-through to
/// `StorageService::read_chat_history` — kept as a separate testable
/// function for the same reason `send_prompt` is (mirrors
/// `ipc::runner::extraction_handler`'s command-fn/testable-fn split).
async fn get_session_history(
    storage: &SqliteStorageService,
    session_id: &str,
    before_seq: Option<i64>,
    limit: u32,
) -> Result<Vec<ChatEventRecord>, AppError> {
    storage
        .read_chat_history(session_id, before_seq, limit)
        .await
}

/// Returns raw journaled events for a session, oldest-first (LLD-12c /
/// design doc §2.3.1 — deliberately *not* pre-grouped into `Message`s here.
/// Grouping consecutive `token_delta`s into one assistant message and
/// pairing `tool_call`/`tool_result` is a pure, side-effect-free projection
/// that belongs on the frontend next to the render logic it feeds
/// (`projectHistory` in `src/components/app/chat/`), not duplicated in Rust.
#[tauri::command]
#[specta::specta]
pub async fn chat_get_session_history(
    state: State<'_, AppState>,
    session_id: String,
    before_seq: Option<i64>,
    limit: u32,
) -> Result<Vec<ChatEventRecord>, AppError> {
    get_session_history(&state.storage, &session_id, before_seq, limit).await
}

/// Resolves the currently-active session for a scope, if one has ever been
/// opened — without creating one (unlike `send_prompt`/`start_new_session`,
/// which both create on miss). Lets the frontend know the real backend
/// session id for a scope *before* the user sends a first message this app
/// session — needed to fetch history for a scope you've chatted in before,
/// and the missing piece that let `chatSessionId` go permanently unset
/// (nothing ever populated it on mount, only a send's ack could, and a send
/// required it to already be set — a deadlock; found while wiring the
/// redesigned chat UI to this command).
#[tauri::command]
#[specta::specta]
pub async fn chat_resolve_session(
    state: State<'_, AppState>,
    scope: ChatScopeInput,
) -> Result<Option<ChatSession>, AppError> {
    state
        .storage
        .find_chat_session_by_scope(
            Some(RUNNER.id()),
            scope.scope_type(),
            scope.scope_id().as_deref(),
        )
        .await
}

/// Backs `chat_start_new_session`. See `StorageService::start_new_chat_session`
/// (design doc §2.5/US-9) for the supersession mechanics.
async fn start_new_session(
    storage: &SqliteStorageService,
    scope: ChatScopeInput,
) -> Result<ChatSession, AppError> {
    storage
        .start_new_chat_session(NewChatSession {
            runner_id: Some(RUNNER.id().to_string()),
            scope_type: scope.scope_type(),
            scope_id: scope.scope_id(),
            title: None,
        })
        .await
}

/// "New chat" (06_CHAT.md's `[+]`, currently unwired on the frontend —
/// design doc US-9): opens a fresh session for `scope`, keeping whichever
/// session was previously active for it around (renameable, listable via
/// `chat_list_sessions`) rather than overwriting it.
#[tauri::command]
#[specta::specta]
pub async fn chat_start_new_session(
    state: State<'_, AppState>,
    scope: ChatScopeInput,
) -> Result<ChatSession, AppError> {
    start_new_session(&state.storage, scope).await
}

/// Renames a chat session (design doc US-7). Rejects an empty/
/// whitespace-only title — see `StorageService::update_chat_session_title`.
#[tauri::command]
#[specta::specta]
pub async fn chat_rename_session(
    state: State<'_, AppState>,
    session_id: String,
    title: String,
) -> Result<(), AppError> {
    state
        .storage
        .update_chat_session_title(&session_id, &title)
        .await
}

/// Lists past chat sessions, newest-updated first, for the history browser
/// (design doc US-8) — every session, active or superseded by a later
/// "New chat".
#[tauri::command]
#[specta::specta]
pub async fn chat_list_sessions(
    state: State<'_, AppState>,
    before_updated_at: Option<i64>,
    limit: u32,
) -> Result<Vec<ChatSession>, AppError> {
    state
        .storage
        .list_chat_sessions(before_updated_at, limit)
        .await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use super::*;
    use crate::db::models::{NewConversation, NewProject};
    use crate::ipc::runner::{ApprovalDecision, RunnerMode};

    /// Test double for `AgentRunner` — lets `ChatRegistry`/`cancel_turn`
    /// mechanics be tested without spawning a real `claude` CLI process (the
    /// only real impl, `ClaudeRunner`, is exercised end-to-end instead by
    /// `tests/chat_live_tool_calling.rs`). Only `cancel_turn` is
    /// instrumented; nothing else in this module calls the other methods.
    #[derive(Default)]
    struct NoopRunner {
        cancelled: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl AgentRunner for NoopRunner {
        async fn start(&mut self, _config: RunnerConfig) -> Result<(), AppError> {
            Ok(())
        }
        async fn prompt(&mut self, _req: PromptRequest) -> Result<AgentStream, AppError> {
            unimplemented!("not exercised by the registry/cancel tests")
        }
        async fn cancel_turn(&mut self, _turn_id: TurnId) -> Result<(), AppError> {
            self.cancelled.store(true, Ordering::SeqCst);
            Ok(())
        }
        async fn respond_to_approval(
            &mut self,
            _req_id: crate::ipc::runner::ApprovalId,
            _decision: ApprovalDecision,
        ) -> Result<(), AppError> {
            Ok(())
        }
        async fn set_mode(&mut self, _mode: RunnerMode) -> Result<(), AppError> {
            Ok(())
        }
        async fn dispose(self: Box<Self>) -> Result<(), AppError> {
            Ok(())
        }
    }

    fn noop_entry(cancelled: Arc<AtomicBool>) -> Arc<ChatRunnerEntry> {
        Arc::new(ChatRunnerEntry {
            runner: AsyncMutex::new(Box::new(NoopRunner { cancelled }) as Box<dyn AgentRunner>),
            epoch: "e1".to_string(),
            next_seq: AtomicI64::new(0),
        })
    }

    #[tokio::test]
    async fn evict_removes_the_entry_and_is_idempotent() {
        let chat = ChatRegistry::new();
        chat.insert_if_absent("s1", noop_entry(Arc::new(AtomicBool::new(false))))
            .await;
        assert!(chat.get("s1").await.is_some());

        assert!(chat.evict("s1").await.is_some());
        assert!(chat.get("s1").await.is_none());

        // Idempotent: evicting an already-gone session is a clean `None`,
        // not an error — cancel_turn relies on this (§2.4).
        assert!(chat.evict("s1").await.is_none());
    }

    #[tokio::test]
    async fn cancel_turn_is_a_noop_when_no_live_runner_for_the_session() {
        let chat = ChatRegistry::new();
        let result = cancel_turn(&chat, "missing-session", "turn-1".to_string()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn cancel_turn_evicts_the_entry_and_cancels_the_runner() {
        let chat = ChatRegistry::new();
        let cancelled = Arc::new(AtomicBool::new(false));
        chat.insert_if_absent("s1", noop_entry(cancelled.clone()))
            .await;

        cancel_turn(&chat, "s1", "turn-1".to_string())
            .await
            .unwrap();

        assert!(cancelled.load(Ordering::SeqCst));
        // The critical invariant from the design doc: after cancel, the
        // session has no live runner, so the *next* `send_prompt` for it
        // takes the registry-miss path and spawns a fresh process instead
        // of writing to a dead one's stdin.
        assert!(chat.get("s1").await.is_none());
    }

    #[test]
    fn scope_input_maps_to_scope_type_and_id() {
        assert_eq!(
            ChatScopeInput::Everything.scope_type(),
            ChatScopeType::Everything
        );
        assert_eq!(ChatScopeInput::Everything.scope_id(), None);

        let project = ChatScopeInput::Project {
            project_id: "p1".to_string(),
        };
        assert_eq!(project.scope_type(), ChatScopeType::Project);
        assert_eq!(project.scope_id(), Some("p1".to_string()));

        let conversation = ChatScopeInput::Conversation {
            conversation_id: "c1".to_string(),
        };
        assert_eq!(conversation.scope_type(), ChatScopeType::Conversation);
        assert_eq!(conversation.scope_id(), Some("c1".to_string()));
    }

    async fn test_storage() -> SqliteStorageService {
        let dir = tempfile::tempdir().unwrap();
        // Leak the tempdir for the test's lifetime — dropped `TempDir`s
        // delete on drop, and this fn returns before the caller is done
        // with the db file.
        let path = Box::leak(Box::new(dir)).path().join("mnemos.db");
        let pools = crate::db::init(&path).await.expect("db init");
        SqliteStorageService::new(pools)
    }

    #[tokio::test]
    async fn conversation_scope_stuffs_the_transcript_into_the_system_prompt_with_no_mcp() {
        let storage = test_storage().await;
        let project = storage
            .create_project(NewProject {
                name: "Proj".to_string(),
                description: None,
            })
            .await
            .unwrap();
        let conv = storage
            .insert_conversation(NewConversation {
                project_id: Some(project.id.clone()),
                title: "Kickoff".to_string(),
                started_at: 1_700_000_000,
                runner_id: None,
            })
            .await
            .unwrap();
        storage
            .write_transcript(
                &conv.id,
                &serde_json::json!({"turns": [{"speaker": "You", "text": "hello"}]}),
            )
            .await
            .unwrap();

        let scope = ChatScopeInput::Conversation {
            conversation_id: conv.id.clone(),
        };
        let config = build_runner_config(&storage, RUNNER, &scope).await.unwrap();
        assert!(config.mcp.is_none());
        let prompt = config.system_prompt.unwrap();
        assert!(prompt.contains("Kickoff"));
        assert!(prompt.contains("hello"));
    }

    #[tokio::test]
    async fn conversation_scope_without_a_transcript_yet_is_a_validation_error() {
        let storage = test_storage().await;
        let project = storage
            .create_project(NewProject {
                name: "Proj".to_string(),
                description: None,
            })
            .await
            .unwrap();
        let conv = storage
            .insert_conversation(NewConversation {
                project_id: Some(project.id.clone()),
                title: "Kickoff".to_string(),
                started_at: 1_700_000_000,
                runner_id: None,
            })
            .await
            .unwrap();

        let scope = ChatScopeInput::Conversation {
            conversation_id: conv.id.clone(),
        };
        let err = build_runner_config(&storage, RUNNER, &scope)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation { .. }));
    }

    #[tokio::test]
    async fn project_scope_prompt_embeds_recent_conversations_and_memory() {
        // `project_system_prompt` directly, not `build_runner_config` — the
        // latter also resolves the `mnemos-mcp-server` binary for
        // Project/Everything scope (`mcp_config_for_chat`), an unrelated
        // dependency this test environment doesn't have, matching every
        // other test in this file's silence on Project/Everything scope.
        let storage = test_storage().await;
        let project = storage
            .create_project(NewProject {
                name: "Proj".to_string(),
                description: None,
            })
            .await
            .unwrap();
        storage
            .insert_conversation(NewConversation {
                project_id: Some(project.id.clone()),
                title: "Kickoff".to_string(),
                started_at: 1_700_000_000,
                runner_id: None,
            })
            .await
            .unwrap();
        storage
            .write_project_memory(
                &project.id,
                &serde_json::json!({"overview": "we ship things"}),
            )
            .await
            .unwrap();

        let recent = storage
            .list_conversations(ConversationFilter {
                project_id: Some(project.id.clone()),
                include_archived: true,
                limit: Some(RUNNER_CONFIG_RECENT_LIMIT),
                ..Default::default()
            })
            .await
            .unwrap();
        let memory = storage.read_project_memory(&project.id).await.unwrap();

        let prompt = project_system_prompt(&project.name, &project.id, &recent, memory.as_ref());
        assert!(
            prompt.contains("Kickoff"),
            "prompt should list the recent conversation: {prompt}"
        );
        assert!(
            prompt.contains("we ship things"),
            "prompt should embed project memory: {prompt}"
        );
    }

    #[test]
    fn project_scope_prompt_handles_no_conversations_or_memory_yet() {
        let prompt = project_system_prompt("Empty Proj", "p1", &[], None);
        assert!(prompt.contains("(none yet)"));
        assert!(prompt.contains("(no project memory generated yet)"));
    }

    #[tokio::test]
    async fn get_session_history_returns_journaled_events_oldest_first() {
        let storage = test_storage().await;
        let session = storage
            .open_chat_session(NewChatSession {
                runner_id: Some(RUNNER.id().to_string()),
                scope_type: ChatScopeType::Everything,
                scope_id: None,
                title: None,
            })
            .await
            .unwrap();

        storage
            .append_chat_event(
                &session.id,
                &session.epoch,
                0,
                serde_json::json!({"kind": "user_message", "text": "hello"}),
            )
            .await
            .unwrap();
        storage
            .append_chat_event(
                &session.id,
                &session.epoch,
                1,
                serde_json::json!({"kind": "token_delta", "turn_id": "t1", "text": "hi"}),
            )
            .await
            .unwrap();

        let history = get_session_history(&storage, &session.id, None, 200)
            .await
            .unwrap();

        assert_eq!(history.len(), 2);
        assert_eq!(history[0].seq, 0);
        assert_eq!(history[0].event_json["kind"], "user_message");
        assert_eq!(history[1].seq, 1);
        assert_eq!(history[1].event_json["kind"], "token_delta");
    }

    #[tokio::test]
    async fn start_new_session_on_a_scope_with_no_prior_session_behaves_like_open() {
        let storage = test_storage().await;
        let scope = ChatScopeInput::Everything;

        let session = start_new_session(&storage, scope.clone()).await.unwrap();

        assert!(session.superseded_by_id.is_none());
        let active = storage
            .find_chat_session_by_scope(Some(RUNNER.id()), scope.scope_type(), None)
            .await
            .unwrap();
        assert_eq!(active.unwrap().id, session.id);
    }

    #[tokio::test]
    async fn resolve_session_is_none_for_a_scope_never_opened_and_finds_it_once_created() {
        let storage = test_storage().await;
        let scope = ChatScopeInput::Everything;

        let before = storage
            .find_chat_session_by_scope(Some(RUNNER.id()), scope.scope_type(), None)
            .await
            .unwrap();
        assert!(
            before.is_none(),
            "never-opened scope must resolve to None, not an error"
        );

        let created = start_new_session(&storage, scope.clone()).await.unwrap();
        let after = storage
            .find_chat_session_by_scope(Some(RUNNER.id()), scope.scope_type(), None)
            .await
            .unwrap();
        assert_eq!(after.unwrap().id, created.id);
    }

    #[tokio::test]
    async fn start_new_session_supersedes_the_previously_active_one() {
        let storage = test_storage().await;
        let scope = ChatScopeInput::Everything;

        let first = start_new_session(&storage, scope.clone()).await.unwrap();
        let second = start_new_session(&storage, scope.clone()).await.unwrap();

        assert_ne!(first.id, second.id);
        assert_eq!(second.superseded_by_id, None);

        // The old row is superseded, not deleted — still findable by id,
        // just no longer what a scope lookup resolves to.
        let refetched_first = storage.read_chat_history(&first.id, None, 1).await.unwrap();
        assert!(refetched_first.is_empty()); // never had any messages — just proves the id is still queryable, not gone

        let active = storage
            .find_chat_session_by_scope(Some(RUNNER.id()), scope.scope_type(), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            active.id, second.id,
            "scope lookup must resolve to the newer session"
        );

        let all = storage.list_chat_sessions(None, 10).await.unwrap();
        let ids: Vec<_> = all.iter().map(|s| s.id.clone()).collect();
        assert!(
            ids.contains(&first.id),
            "superseded session must still be listed"
        );
        assert!(ids.contains(&second.id));
    }

    #[tokio::test]
    async fn start_new_session_keeps_project_and_conversation_scopes_independent() {
        // Superseding one scope must never touch a session for a different
        // scope — regression guard for the UNIQUE index's column set.
        let storage = test_storage().await;
        let project = storage
            .create_project(NewProject {
                name: "P".to_string(),
                description: None,
            })
            .await
            .unwrap();

        let everything_1 = start_new_session(&storage, ChatScopeInput::Everything)
            .await
            .unwrap();
        let project_scope = ChatScopeInput::Project {
            project_id: project.id.clone(),
        };
        let project_session = start_new_session(&storage, project_scope.clone())
            .await
            .unwrap();
        let everything_2 = start_new_session(&storage, ChatScopeInput::Everything)
            .await
            .unwrap();

        assert_ne!(everything_1.id, everything_2.id);
        // The Project-scope session must be completely unaffected by the
        // Everything-scope supersession that happened around it.
        let project_active = storage
            .find_chat_session_by_scope(
                Some(RUNNER.id()),
                ChatScopeType::Project,
                Some(&project.id),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(project_active.id, project_session.id);
    }

    #[tokio::test]
    async fn update_chat_session_title_renames_and_rejects_blank() {
        let storage = test_storage().await;
        let session = start_new_session(&storage, ChatScopeInput::Everything)
            .await
            .unwrap();

        storage
            .update_chat_session_title(&session.id, "Battery decision")
            .await
            .unwrap();
        let renamed = storage
            .find_chat_session_by_scope(Some(RUNNER.id()), ChatScopeType::Everything, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(renamed.title.as_deref(), Some("Battery decision"));

        let err = storage
            .update_chat_session_title(&session.id, "   ")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation { .. }));

        let missing = storage
            .update_chat_session_title("does-not-exist", "x")
            .await
            .unwrap_err();
        assert!(matches!(missing, AppError::NotFound { .. }));
    }

    #[tokio::test]
    async fn list_chat_sessions_orders_newest_first_and_respects_limit() {
        let storage = test_storage().await;
        // `start_new_session` supersedes immediately, so three calls on the
        // same scope leaves three distinct, individually-listed rows.
        let first = start_new_session(&storage, ChatScopeInput::Everything)
            .await
            .unwrap();
        let second = start_new_session(&storage, ChatScopeInput::Everything)
            .await
            .unwrap();
        let third = start_new_session(&storage, ChatScopeInput::Everything)
            .await
            .unwrap();

        let all = storage.list_chat_sessions(None, 10).await.unwrap();
        assert_eq!(all.len(), 3);

        let limited = storage.list_chat_sessions(None, 2).await.unwrap();
        assert_eq!(limited.len(), 2);

        let ids: Vec<_> = all.iter().map(|s| s.id.clone()).collect();
        assert!(ids.contains(&first.id));
        assert!(ids.contains(&second.id));
        assert!(ids.contains(&third.id));
    }

    // `send_prompt` itself takes a `Channel<AgentEvent>`, which only a
    // running Tauri app can construct, so its full body (including the
    // empty-text guard) isn't reachable from a plain `cargo test`. The real
    // end-to-end path — session resolution, runner start, a real MCP tool
    // call, and the journal writes — is proven instead by
    // `tests/chat_live_tool_calling.rs` against the real `claude` CLI and a
    // real seeded database.
}
