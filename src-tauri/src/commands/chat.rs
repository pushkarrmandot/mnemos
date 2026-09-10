//! `chat.*` Tauri command surface — wires the long-lived
//! chat-session pattern and
//! the real tool-calling loop (see
//! `ipc::runner::claude`) into an actual command React can call.
//!
//! One command in v1, `chat_send_prompt`: sends into one specific,
//! already-existing `chat_sessions` row, named explicitly by its id — the
//! frontend always knows which session it's talking to (`chat_resolve_session`
//! on first open, a caller-minted id for a brand-new one,
//! or whichever row it opened from history) and passes that id in, so the
//! backend never has to guess which of a scope's sessions a message
//! belongs to. Starts (or reuses) the long-lived `ClaudeRunner` for that
//! session, journals the user turn, enqueues the prompt, and returns
//! immediately — every `AgentEvent` streams back over the caller's
//! `Channel` while a detached task journals each one into `chat_journal`
//! (+ `chat_sessions` projection, same transaction, per
//! `StorageService::append_chat_event`).
//!
//! Scope -> context mapping (no vector search exists yet — that's a later
//! release):
//! - **Conversation** scope: the conversation's full `transcript.json` is
//!   stuffed directly into the system prompt (small enough) — no MCP tools,
//!   `RunnerConfig.mcp = None`.
//! - **Project** scope: MCP tools (`mnemos-mcp-server`) + a system-prompt
//!   instruction to always pass this project's `project_id` to every tool
//!   call — verified against a real `claude` CLI + a real `mnemos-mcp-server`
//!   process that the model reliably does this when told to.
//! - **Everything** scope: same MCP tools, no project instruction — the
//!   tools themselves treat a missing `project_id` as unfiltered (the
//!   `mnemos.list_action_items`/`list_open_questions` call straight through
//!   to `StorageService::list_action_items_global`/`list_open_questions_global`
//!   when no `project_id` arg is given; there is no separate "_global" MCP
//!   tool name).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
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
/// session's system prompt. Ten is the query's own bound, rather than
/// loading every conversation and truncating client-side.
const RUNNER_CONFIG_RECENT_LIMIT: u32 = 10;

/// Only vendor in v1 (Codex/OpenCode/Gemini/Ollama siblings are deferred to
/// a later release) — hard-coded rather than plumbed through from Settings,
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

    /// Reconstructs the scope a session was opened under from its own
    /// stored `scope_type`/`scope_id` — `chat_send_prompt` derives the
    /// scope this way rather than taking one as a separate argument, so
    /// there's no way for a caller to pass a `session_id` and a mismatched
    /// `scope` that disagree about where a message should go.
    fn from_session(session: &ChatSession) -> Self {
        match session.scope_type {
            ChatScopeType::Everything => ChatScopeInput::Everything,
            ChatScopeType::Project => ChatScopeInput::Project {
                project_id: session.scope_id.clone().unwrap_or_default(),
            },
            ChatScopeType::Conversation => ChatScopeInput::Conversation {
                conversation_id: session.scope_id.clone().unwrap_or_default(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ChatSendPromptAck {
    pub session_id: String,
}

struct ChatRunnerEntry {
    runner: AsyncMutex<Box<dyn AgentRunner>>,
    /// Next `chat_journal.seq` for this session. Seeded from the journal's
    /// own `MAX(seq) + 1` (`next_chat_seq`), never from
    /// `chat_sessions.message_count`: those count different things, so
    /// seeding from the projection restarts `seq` below `MAX(seq)` after an
    /// app restart and every later write fails the `(session_id, seq)`
    /// primary key — silently, because journal write failures are logged
    /// and swallowed.
    next_seq: AtomicI64,
    /// True while a turn is in flight for this session. `ClaudeRunner`
    /// keeps exactly one `current_turn`/`pending_cancel` pair, so a second
    /// concurrent prompt drops the first turn's cancel channel — which
    /// fires that turn's cancel arm, reports it `Cancelled` while the CLI
    /// is still streaming, and lets the second turn consume the first
    /// turn's frames under its own id. The UI's disabled Send button is not
    /// load-bearing enough for that; this is (design §3).
    turn_in_flight: AtomicBool,
}

/// One long-lived `ClaudeRunner` per `chat_sessions` row,
/// keyed by that row's id. Never disposed in v1 — no `chat.clear_session`/
/// app-quit hook exists yet; the process exit
/// that ends the app also reaps every child `claude` process via
/// `kill_on_drop`.
/// `Clone` is cheap and shares one map (an `Arc` inside): the detached
/// per-turn forwarder needs to be able to evict a session whose runner
/// turned out to be unusable, and it outlives the command that spawned it.
#[derive(Default, Clone)]
pub struct ChatRegistry {
    sessions: Arc<AsyncMutex<HashMap<String, Arc<ChatRunnerEntry>>>>,
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
    resume: Option<String>,
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
                resume,
            });
        }
        ChatScopeInput::Project { project_id } => {
            let project = storage.get_project(project_id).await?;

            // Orientation context, embedded once at session-open rather than
            // requiring a tool round trip before the model can answer even
            // the most basic "what have we been talking about" question —
            // both are cheap: `recent_conversations` is id/title/date only
            // (not full transcripts), and project memory is deliberately a
            // *short* synthesized doc, not the raw transcript
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
        resume,
    })
}

/// Drains one turn's stream: journals the durable items, streams
/// everything to the caller's `Channel`, and writes the assistant's reply
/// as a single row when the turn ends.
///
/// **Journaling is independent of delivery** (design §1, I1). A dropped
/// channel — the chat pane closed, the window went away — stops forwarding
/// but never stops draining, because the durable write for this turn's
/// reply happens at the *end*. Returning early on a send failure would
/// discard the entire answer, which is also what `06_CHAT.md` forbids
/// ("closes right pane while streaming → response continues in
/// background").
///
/// **Token deltas are streamed, not stored.** They are fragments of a thing
/// that has a final form; we accumulate them here and persist the final
/// form once. Storing each fragment meant one write transaction per token
/// and made any bounded history read cover a fraction of a single answer.
/// Everything one turn's forwarder needs that isn't the stream itself.
struct TurnContext {
    channel: Channel<AgentEvent>,
    storage: SqliteStorageService,
    chat: ChatRegistry,
    metrics: crate::metrics::Metrics,
    session_id: String,
    entry: Arc<ChatRunnerEntry>,
    /// `Some(text)` when this turn opened the chat — the only turn worth
    /// naming the chat from. `None` for every later turn.
    first_turn: Option<String>,
}

async fn forward_turn(mut stream: AgentStream, ctx: TurnContext) {
    let TurnContext {
        channel,
        storage,
        chat,
        metrics,
        session_id,
        entry,
        first_turn,
    } = ctx;
    let turn_start = std::time::Instant::now();
    let mut assistant_text = String::new();
    let mut turn_id: Option<TurnId> = None;
    let mut delivering = true;

    while let Some(event) = stream.next().await {
        if turn_id.is_none() {
            turn_id = Some(event_turn_id(&event));
        }

        // Durable items get a row as they arrive; deltas only accumulate.
        match &event {
            AgentEvent::TokenDelta { text, .. } => assistant_text.push_str(text),
            AgentEvent::ToolCall { .. } => {
                journal(&storage, &entry, &session_id, &event).await;
            }
            _ => {}
        }

        let terminal = match &event {
            AgentEvent::Complete { usage, .. } => Some((true, Some(usage.clone()))),
            AgentEvent::Error { .. } => Some((false, None)),
            _ => None,
        };

        // The runner no longer has the conversation we asked it to resume.
        // Forget the id and drop the process so the next message starts a
        // fresh one, instead of failing this way forever.
        if let AgentEvent::Error { error, .. } = &event {
            if error.to_string().contains(RESUME_MISS_MARKER) {
                handle_resume_miss(&storage, &chat, &session_id).await;
            }
        }

        if terminal.is_some() {
            // One assistant row per turn, written whatever the terminal
            // reason — a turn that dies mid-stream still persists what it
            // produced.
            if !assistant_text.is_empty() {
                let row = serde_json::json!({
                    "kind": "assistant_message",
                    "turn_id": turn_id.clone().unwrap_or_default(),
                    "text": assistant_text,
                });
                journal_value(&storage, &entry, &session_id, row).await;
            }
            if let AgentEvent::Error { error, .. } = &event {
                let row = serde_json::json!({
                    "kind": "error",
                    "turn_id": turn_id.clone().unwrap_or_default(),
                    "message": error.to_string(),
                });
                journal_value(&storage, &entry, &session_id, row).await;
            }
        }

        if delivering && channel.send(event).is_err() {
            // Keep draining and journaling; just stop forwarding.
            delivering = false;
        }

        if let Some((success, usage)) = terminal {
            if let Some(usage) = usage {
                let input = i64::from(usage.input_tokens)
                    + i64::from(usage.cache_creation_input_tokens.unwrap_or(0))
                    + i64::from(usage.cache_read_input_tokens.unwrap_or(0));
                if let Err(e) = storage
                    .add_chat_session_usage(&session_id, input, i64::from(usage.output_tokens), 0)
                    .await
                {
                    tracing::warn!(error = %e, session_id = %session_id, "chat.usage_write_failed");
                }
            }
            // A chat's opening exchange is the only thing worth naming it
            // from, and it has just finished.
            if success {
                if let Some(asked) = first_turn.clone() {
                    if !assistant_text.is_empty() {
                        tokio::spawn(retitle_from_first_exchange(
                            storage.clone(),
                            session_id.clone(),
                            asked,
                            assistant_text.clone(),
                        ));
                    }
                }
            }
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
            break;
        }
    }

    entry.turn_in_flight.store(false, Ordering::SeqCst);
}

/// Substring of the CLI's own refusal when asked to resume a conversation
/// it no longer has: `{"subtype":"error_during_execution","is_error":true,
/// "errors":["No conversation found with session ID: ..."]}`. Matched as a
/// substring rather than on a field, because the text arrives in an
/// `errors` array on some frames and `result` on others.
const RESUME_MISS_MARKER: &str = "No conversation found with session ID";

/// A stored `runner_session_id` can go stale — the runner prunes old
/// sessions, or the user clears its state directory. Left alone, every
/// future send for that chat would resume the same missing session and fail
/// the same way, forever.
///
/// Recovery is deliberately "forget the id and evict the process", not
/// "silently retry the turn": the earlier context really is gone, and
/// quietly answering without it would be the same silent-amnesia failure
/// this whole design exists to prevent. The user is told once, and the next
/// message cold-starts a fresh conversation in the same chat.
async fn handle_resume_miss(storage: &SqliteStorageService, chat: &ChatRegistry, session_id: &str) {
    tracing::warn!(session_id = %session_id, "chat.resume_miss_clearing_runner_session");
    if let Err(e) = storage
        .clear_chat_session_runner_session_id(session_id)
        .await
    {
        tracing::warn!(error = %e, session_id = %session_id, "chat.resume_miss_clear_failed");
    }
    // The live entry is bound to the refused session; the next send must
    // take the cold path and start a genuinely new conversation.
    if let Some(entry) = chat.evict(session_id).await {
        if let Ok(entry) = Arc::try_unwrap(entry) {
            let _ = entry.runner.into_inner().dispose().await;
        }
    }
}

/// Replaces a chat's placeholder title with a short one the model writes,
/// once its first exchange has actually happened.
///
/// Detached and best-effort by design. The row already has a usable title —
/// the first message, truncated — written synchronously when the chat was
/// created, so this only ever *improves* it. Doing it inline would make the
/// first reply of every chat wait on a second model call to name something
/// the user can already see.
///
/// One extra call per chat, ever: gated on the chat's first turn.
async fn retitle_from_first_exchange(
    storage: SqliteStorageService,
    session_id: String,
    asked: String,
    answered: String,
) {
    let mut runner: Box<dyn AgentRunner> = RUNNER.create();
    let started = runner
        .start(RunnerConfig {
            model: RUNNER.default_chat_model().to_string(),
            timeout_ms: Some(20_000),
            system_prompt: Some(
                "You name conversations. Reply with a title of 3 to 6 words and nothing \
                 else — no quotes, no trailing punctuation, no preamble. Describe the \
                 subject, not the act of asking."
                    .to_string(),
            ),
            tools: vec![],
            approval_policy: ApprovalPolicy::AutoDenyDestructive,
            mcp: None,
            resume: None,
        })
        .await;
    if started.is_err() {
        return;
    }

    let prompt = format!("User asked:\n{asked}\n\nAssistant replied:\n{answered}");
    let stream = runner
        .prompt(PromptRequest {
            content: vec![UserContent::Text(prompt)],
            turn_id: None,
        })
        .await;
    let mut title = String::new();
    if let Ok(mut stream) = stream {
        while let Some(event) = stream.next().await {
            match event {
                AgentEvent::TokenDelta { text, .. } => title.push_str(&text),
                AgentEvent::Complete { .. } | AgentEvent::Error { .. } => break,
                _ => {}
            }
        }
    }
    let _ = runner.dispose().await;

    // A model that ignored the instruction and wrote a paragraph is worse
    // than the truncated first message we already have.
    let title = title.trim().trim_matches('"').trim();
    if title.is_empty() || title.chars().count() > 60 || title.contains('\n') {
        return;
    }
    if let Err(e) = storage.update_chat_session_title(&session_id, title).await {
        tracing::warn!(error = %e, session_id = %session_id, "chat.retitle_failed");
    }
}

fn event_turn_id(event: &AgentEvent) -> TurnId {
    match event {
        AgentEvent::TokenDelta { turn_id, .. }
        | AgentEvent::ToolCall { turn_id, .. }
        | AgentEvent::ToolResult { turn_id, .. }
        | AgentEvent::ApprovalRequest { turn_id, .. }
        | AgentEvent::Notice { turn_id, .. }
        | AgentEvent::Complete { turn_id, .. }
        | AgentEvent::Error { turn_id, .. } => turn_id.clone(),
    }
}

/// Journals one `AgentEvent` as a durable row. Failures are logged, not
/// propagated — a lost journal row must not kill an in-flight turn.
async fn journal(
    storage: &SqliteStorageService,
    entry: &Arc<ChatRunnerEntry>,
    session_id: &str,
    event: &AgentEvent,
) {
    match serde_json::to_value(event) {
        Ok(value) => journal_value(storage, entry, session_id, value).await,
        Err(e) => tracing::error!(error = %e, "chat.event_encode_failed"),
    }
}

async fn journal_value(
    storage: &SqliteStorageService,
    entry: &Arc<ChatRunnerEntry>,
    session_id: &str,
    value: serde_json::Value,
) {
    let seq = entry.next_seq.fetch_add(1, Ordering::SeqCst);
    if let Err(e) = storage.append_chat_event(session_id, seq, value).await {
        tracing::warn!(error = %e, session_id = %session_id, "chat.journal_write_failed");
    }
}

/// Where a prompt is going. Two variants rather than an optional
/// `session_id` beside an optional `scope`, so "a session id *and* a
/// contradicting scope" is not a state a caller can express.
///
/// `NewChat` carries a caller-minted `session_id`. The frontend mints it so
/// local state (draft, streaming buffer, outbox) keys off the real id from
/// the first keystroke rather than a placeholder that must later be
/// re-keyed, and so a double-fired or retried first send resolves to the
/// same chat instead of creating a second one.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SendTarget {
    NewChat {
        session_id: String,
        scope: ChatScopeInput,
    },
    Existing {
        session_id: String,
    },
}

/// First line of a message, as a chat title: trimmed, whitespace-collapsed,
/// cut to `MAX` on a word boundary. Deliberately not an LLM call — a
/// generated title is a second failure path and a second cost for something
/// the first message already says.
fn title_from_message(text: &str) -> Option<String> {
    const MAX: usize = 50;
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return None;
    }
    if flat.chars().count() <= MAX {
        return Some(flat);
    }
    let truncated: String = flat.chars().take(MAX).collect();
    let cut = truncated
        .rfind(' ')
        .filter(|i| *i > MAX / 2)
        .unwrap_or(truncated.len());
    Some(format!("{}…", truncated[..cut].trim_end()))
}

/// Journals the user's turn, enqueues the prompt against this chat's runner
/// (starting or resuming one if none is live), and returns as soon as the
/// turn is *enqueued* — the response streams back over `channel` while a
/// detached `forward_turn` task journals it.
///
/// A plain function rather than the `#[tauri::command]` itself so it is
/// callable from a test with just a `SqliteStorageService` + `ChatRegistry`,
/// no running Tauri app — mirroring `ipc::runner::extraction_handler`'s
/// command-fn/testable-fn split.
async fn send_prompt(
    storage: &SqliteStorageService,
    chat: &ChatRegistry,
    metrics: &crate::metrics::Metrics,
    target: SendTarget,
    text: String,
    channel: Channel<AgentEvent>,
) -> Result<ChatSendPromptAck, AppError> {
    if text.trim().is_empty() {
        return Err(AppError::Validation {
            message: "text must not be empty".to_string(),
            field: Some("text".to_string()),
        });
    }

    // Resolve (or create) the chat row. `NewChat` is idempotent on the
    // caller's id, so a retry lands on the same chat.
    let (session, created) = match &target {
        SendTarget::Existing { session_id } => (
            storage
                .get_chat_session(session_id)
                .await?
                .ok_or_else(|| AppError::NotFound {
                    entity: "chat_session".into(),
                    id: session_id.clone(),
                })?,
            false,
        ),
        SendTarget::NewChat { session_id, scope } => {
            let existed = storage.get_chat_session(session_id).await?;
            match existed {
                Some(s) => (s, false),
                None => (
                    storage
                        .open_chat_session(NewChatSession {
                            id: session_id.clone(),
                            runner_id: Some(RUNNER.id().to_string()),
                            scope_type: scope.scope_type(),
                            scope_id: scope.scope_id(),
                            title: title_from_message(&text),
                        })
                        .await?,
                    true,
                ),
            }
        }
    };

    // Everything past here can fail; if we created the row above, it must
    // not survive that failure (design §3 — no empty chats on any path).
    let result = dispatch(storage, chat, metrics, &session, text, channel).await;
    if result.is_err() && created {
        if let Err(e) = storage.delete_chat_session(&session.id).await {
            tracing::warn!(error = %e, session_id = %session.id, "chat.rollback_failed");
        }
    }
    result
}

/// The part of a send that can fail after the row exists — kept separate so
/// `send_prompt` has exactly one rollback point.
async fn dispatch(
    storage: &SqliteStorageService,
    chat: &ChatRegistry,
    metrics: &crate::metrics::Metrics,
    session: &ChatSession,
    text: String,
    channel: Channel<AgentEvent>,
) -> Result<ChatSendPromptAck, AppError> {
    let scope = ChatScopeInput::from_session(session);
    // Read before the user's turn is journaled, which bumps the count.
    let opening_message = (session.message_count == 0).then(|| text.clone());

    let entry = match chat.get(&session.id).await {
        Some(entry) => entry,
        None => {
            // Cold path: no live process for this chat in this app run.
            // Reached on a chat's first message, after a cancel, and — the
            // case that matters — every time an existing chat is reopened
            // after a restart, which is exactly when `resume` earns its
            // keep.
            let config =
                build_runner_config(storage, RUNNER, &scope, session.runner_session_id.clone())
                    .await?;
            let runner = start_runner(storage, session, config).await?;
            let next_seq = storage.next_chat_seq(&session.id).await?;
            let new_entry = Arc::new(ChatRunnerEntry {
                runner: AsyncMutex::new(runner),
                next_seq: AtomicI64::new(next_seq),
                turn_in_flight: AtomicBool::new(false),
            });
            chat.insert_if_absent(&session.id, new_entry).await
        }
    };

    // One turn at a time per chat (design §1, I3).
    if entry
        .turn_in_flight
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(AppError::Validation {
            message: "this chat is still answering — wait for it to finish".to_string(),
            field: Some("session_id".to_string()),
        });
    }

    metrics.track(
        crate::metrics::events::CHAT_MESSAGE_SENT,
        crate::metrics::properties::EventProperties::from([
            ("scope_type", chat_scope_type_property(session.scope_type)),
            (
                "runner",
                crate::metrics::properties::PropertyValue::Enum(RUNNER.id()),
            ),
        ]),
    );

    // Anything that fails from here must clear the in-flight flag, or the
    // chat is wedged for the rest of the app run.
    let dispatched = async {
        let user_seq = entry.next_seq.fetch_add(1, Ordering::SeqCst);
        storage
            .append_chat_event(
                &session.id,
                user_seq,
                serde_json::json!({"kind": "user_message", "text": text}),
            )
            .await?;

        let mut runner = entry.runner.lock().await;
        runner
            .prompt(PromptRequest {
                content: vec![UserContent::Text(text.clone())],
                turn_id: None,
            })
            .await
    }
    .await;

    let stream = match dispatched {
        Ok(stream) => stream,
        Err(e) => {
            entry.turn_in_flight.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };

    tokio::spawn(forward_turn(
        stream,
        TurnContext {
            channel,
            storage: storage.clone(),
            chat: chat.clone(),
            metrics: metrics.clone(),
            session_id: session.id.clone(),
            entry,
            first_turn: opening_message,
        },
    ));

    Ok(ChatSendPromptAck {
        session_id: session.id.clone(),
    })
}

/// Starts a runner for `session` and persists the runner's own session id.
///
/// That id is what a later cold spawn hands to `--resume`, so persisting it
/// is the whole mechanism by which a chat still remembers itself after the
/// app restarts. Asked of the runner rather than assumed to be the id we
/// generated: Claude accepts a caller-supplied one, but other vendors mint
/// their own (see `AgentRunner::runner_session_id`).
///
/// A refused resume is *not* handled here: spawning succeeds even for a
/// session the runner has never heard of, and the refusal only surfaces
/// while draining the first turn. `forward_turn` handles it — see
/// `RESUME_MISS_MARKER`.
async fn start_runner(
    storage: &SqliteStorageService,
    session: &ChatSession,
    config: RunnerConfig,
) -> Result<Box<dyn AgentRunner>, AppError> {
    let mut runner: Box<dyn AgentRunner> = RUNNER.create();
    runner.start(config).await?;

    // Write only on change: on a resume the id comes back unchanged, so
    // this is a no-op for every cold spawn after the first.
    if let Some(id) = runner.runner_session_id().await {
        if session.runner_session_id.as_deref() != Some(id.as_str()) {
            storage
                .set_chat_session_runner_session_id(&session.id, &id)
                .await?;
        }
    }
    Ok(runner)
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
    target: SendTarget,
    text: String,
    channel: Channel<AgentEvent>,
) -> Result<ChatSendPromptAck, AppError> {
    send_prompt(
        &state.storage,
        &state.chat,
        &state.metrics,
        target,
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

/// Returns raw journaled events for a session, oldest-first (design doc
/// §2.3.1) — deliberately *not* pre-grouped into `Message`s here.
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

/// The most recently used session for a scope, if one has ever been opened
/// — without creating one. Lets the frontend
/// know a real backend session id to default into for a scope *before* the
/// user sends a first message this app session — needed to fetch history
/// for a scope you've chatted in before, and the missing piece that let
/// `chatSessionId` go permanently unset (nothing ever populated it on
/// mount, only a send's ack could, and a send required it to already be
/// set — a deadlock; found while wiring the redesigned chat UI to this
/// command). This is only ever a starting point the frontend may switch
/// away from (e.g. reopening an older session from history) — `send_prompt`
/// always takes the exact session id the frontend is currently showing,
/// never re-derives one from scope.
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

/// Deletes a chat and its whole transcript.
///
/// Evicts the live runner first: without that the `claude` process leaks
/// until the app quits, and an in-flight `forward_turn` keeps journaling
/// against a row that no longer exists (which fails as `NotFound` and is
/// swallowed as a warning). Journal rows go with the row via `ON DELETE
/// CASCADE`.
#[tauri::command]
#[specta::specta]
pub async fn chat_delete_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), AppError> {
    if let Some(entry) = state.chat.evict(&session_id).await {
        if let Ok(entry) = Arc::try_unwrap(entry) {
            let runner = entry.runner.into_inner();
            if let Err(e) = runner.dispose().await {
                tracing::warn!(error = %e, session_id = %session_id, "chat.dispose_failed");
            }
        }
    }
    state.storage.delete_chat_session(&session_id).await
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
/// (design doc US-8) — every session for every scope; "New chat" never
/// removes an old one from this list, it just adds a new row.
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
        async fn health(&self) -> crate::ipc::runner::RunnerHealth {
            crate::ipc::runner::RunnerHealth::Ready {
                version: None,
                account: None,
                plan: None,
            }
        }

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
        async fn runner_session_id(&self) -> Option<String> {
            None
        }
        async fn dispose(self: Box<Self>) -> Result<(), AppError> {
            Ok(())
        }
    }

    fn noop_entry(cancelled: Arc<AtomicBool>) -> Arc<ChatRunnerEntry> {
        Arc::new(ChatRunnerEntry {
            runner: AsyncMutex::new(Box::new(NoopRunner { cancelled }) as Box<dyn AgentRunner>),
            next_seq: AtomicI64::new(0),
            turn_in_flight: AtomicBool::new(false),
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

    /// Test stand-in for what a send does on the `NewChat` path: mint an
    /// id (the frontend's job in real use) and open the row.
    async fn open_session(storage: &SqliteStorageService, scope: ChatScopeInput) -> ChatSession {
        storage
            .open_chat_session(NewChatSession {
                id: crate::db::models::new_id(),
                runner_id: Some(RUNNER.id().to_string()),
                scope_type: scope.scope_type(),
                scope_id: scope.scope_id(),
                title: None,
            })
            .await
            .unwrap()
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
        let config = build_runner_config(&storage, RUNNER, &scope, None)
            .await
            .unwrap();
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
        let err = build_runner_config(&storage, RUNNER, &scope, None)
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
                id: crate::db::models::new_id(),
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
                0,
                serde_json::json!({"kind": "user_message", "text": "hello"}),
            )
            .await
            .unwrap();
        storage
            .append_chat_event(
                &session.id,
                1,
                serde_json::json!({"kind": "assistant_message", "turn_id": "t1", "text": "hi"}),
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
        assert_eq!(history[1].event_json["kind"], "assistant_message");
    }

    #[tokio::test]
    async fn a_newly_opened_session_is_what_its_scope_resolves_to() {
        let storage = test_storage().await;
        let scope = ChatScopeInput::Everything;

        let session = open_session(&storage, scope.clone()).await;

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

        let created = open_session(&storage, scope.clone()).await;
        let after = storage
            .find_chat_session_by_scope(Some(RUNNER.id()), scope.scope_type(), None)
            .await
            .unwrap();
        assert_eq!(after.unwrap().id, created.id);
    }

    #[tokio::test]
    async fn a_second_session_for_one_scope_is_independent_of_the_first() {
        let storage = test_storage().await;
        let scope = ChatScopeInput::Everything;

        let first = open_session(&storage, scope.clone()).await;
        let second = open_session(&storage, scope.clone()).await;

        assert_ne!(first.id, second.id);

        // Neither row is ever deleted or specially marked — both are still
        // independently findable by id.
        let refetched_first = storage.read_chat_history(&first.id, None, 1).await.unwrap();
        assert!(refetched_first.is_empty()); // never had any messages — just proves the id is still queryable, not gone

        let resolved = storage
            .find_chat_session_by_scope(Some(RUNNER.id()), scope.scope_type(), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            resolved.id, second.id,
            "scope resolution is a most-recently-updated hint, not an exclusivity guarantee"
        );

        let all = storage.list_chat_sessions(None, 10).await.unwrap();
        let ids: Vec<_> = all.iter().map(|s| s.id.clone()).collect();
        assert!(ids.contains(&first.id), "both sessions must be listed");
        assert!(ids.contains(&second.id));
    }

    #[tokio::test]
    async fn sessions_in_different_scopes_are_independent() {
        // A new session for one scope must never touch a session for a
        // different scope.
        let storage = test_storage().await;
        let project = storage
            .create_project(NewProject {
                name: "P".to_string(),
                description: None,
            })
            .await
            .unwrap();

        let everything_1 = open_session(&storage, ChatScopeInput::Everything).await;
        let project_scope = ChatScopeInput::Project {
            project_id: project.id.clone(),
        };
        let project_session = open_session(&storage, project_scope.clone()).await;
        let everything_2 = open_session(&storage, ChatScopeInput::Everything).await;

        assert_ne!(everything_1.id, everything_2.id);
        // The Project-scope session must be completely unaffected by the
        // second Everything-scope session created around it.
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

    /// The bug this guards is silent and total: `next_seq` used to be
    /// seeded from `chat_sessions.message_count`, which counts something
    /// else. Once the two diverge, a restart restarts `seq` below
    /// `MAX(seq)`, every later write fails the `(session_id, seq)` primary
    /// key, and — because journal write failures are logged and swallowed —
    /// the chat simply stops recording anything with no visible error.
    #[tokio::test]
    async fn seq_continues_from_the_journal_after_a_restart_not_from_message_count() {
        let storage = test_storage().await;
        let session = open_session(&storage, ChatScopeInput::Everything).await;

        // A turn's worth of rows: one user message, one tool call, one
        // reply — three journal rows but not three "messages".
        for (seq, kind) in ["user_message", "tool_call", "assistant_message"]
            .iter()
            .enumerate()
        {
            storage
                .append_chat_event(&session.id, seq as i64, serde_json::json!({"kind": kind}))
                .await
                .unwrap();
        }

        // What a cold start does: seed from the journal, not the projection.
        let next = storage.next_chat_seq(&session.id).await.unwrap();
        assert_eq!(next, 3, "must continue past MAX(seq), not restart");

        // And the continuation actually writes rather than colliding.
        storage
            .append_chat_event(
                &session.id,
                next,
                serde_json::json!({"kind": "user_message"}),
            )
            .await
            .expect("continuing from MAX(seq)+1 must not collide");

        let history = storage
            .read_chat_history(&session.id, None, 200)
            .await
            .unwrap();
        assert_eq!(history.len(), 4);
    }

    #[tokio::test]
    async fn opening_a_chat_twice_with_the_same_id_yields_one_chat() {
        // The first send is retryable — a double Enter, an outbox retry, a
        // slow ack. All of them must land on the same chat.
        let storage = test_storage().await;
        let id = crate::db::models::new_id();
        let mk = || NewChatSession {
            id: id.clone(),
            runner_id: Some(RUNNER.id().to_string()),
            scope_type: ChatScopeType::Everything,
            scope_id: None,
            title: Some("first message".to_string()),
        };

        let a = storage.open_chat_session(mk()).await.unwrap();
        let b = storage.open_chat_session(mk()).await.unwrap();

        assert_eq!(a.id, b.id);
        assert_eq!(storage.list_chat_sessions(None, 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn deleting_a_chat_takes_its_journal_with_it() {
        let storage = test_storage().await;
        let session = open_session(&storage, ChatScopeInput::Everything).await;
        storage
            .append_chat_event(&session.id, 0, serde_json::json!({"kind": "user_message"}))
            .await
            .unwrap();

        storage.delete_chat_session(&session.id).await.unwrap();

        assert!(storage
            .get_chat_session(&session.id)
            .await
            .unwrap()
            .is_none());
        assert!(storage
            .read_chat_history(&session.id, None, 10)
            .await
            .unwrap()
            .is_empty());
        assert!(matches!(
            storage.delete_chat_session(&session.id).await,
            Err(AppError::NotFound { .. })
        ));
    }

    #[test]
    fn a_title_is_the_first_message_trimmed_to_a_word_boundary() {
        assert_eq!(
            title_from_message("  hello   there  "),
            Some("hello there".into())
        );
        assert_eq!(title_from_message("   "), None);

        let long = "the quick brown fox jumps over the lazy dog and keeps on running forever";
        let title = title_from_message(long).unwrap();
        assert!(title.chars().count() <= 51, "got {title:?}");
        assert!(title.ends_with('…'));
        assert!(!title.contains("  "));
        // Cut on a word boundary, not mid-word.
        assert!(long.starts_with(title.trim_end_matches('…')));

        // A single unbroken token still gets cut rather than kept whole.
        let runon = "a".repeat(120);
        assert!(title_from_message(&runon).unwrap().chars().count() <= 51);
    }

    #[tokio::test]
    async fn update_chat_session_title_renames_and_rejects_blank() {
        let storage = test_storage().await;
        let session = open_session(&storage, ChatScopeInput::Everything).await;

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
        // Three calls on the same scope leaves three distinct,
        // individually-listed rows — nothing is ever replaced.
        let first = open_session(&storage, ChatScopeInput::Everything).await;
        let second = open_session(&storage, ChatScopeInput::Everything).await;
        let third = open_session(&storage, ChatScopeInput::Everything).await;

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
