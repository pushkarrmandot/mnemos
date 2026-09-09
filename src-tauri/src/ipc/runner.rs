//! `AgentRunner` — Mnemos' single, pluggable point of contact with LLM
//! providers. Every LLM turn (chat, memory extraction) flows
//! through this trait; nothing else in the codebase talks to a model.
//!
//! **v1 scope:** `ClaudeRunner` only, text-only. Project/Everything-scope
//! chat does pass `--mcp-config` (see `RunnerConfig.mcp`), so the CLI has
//! real MCP tools available — but the CLI resolves those tool calls
//! internally; this crate never dispatches one itself. A `tool_use`
//! content block the CLI emits back over the stream is only translated
//! into an inert `AgentEvent::ToolCall` and logged, never awaited for a
//! result on the Rust side. No other vendor adapters
//! (Codex/OpenCode/Gemini/Ollama) exist yet.

pub mod claude;
pub mod extraction_handler;
pub mod mcp_shared;
pub mod registry;

use std::pin::Pin;

use futures_core::Stream;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::AppError;

pub type TurnId = String;
pub type ApprovalId = String;

pub type AgentStream = Pin<Box<dyn Stream<Item = AgentEvent> + Send>>;

/// Six-method shape.
#[async_trait::async_trait]
pub trait AgentRunner: Send + Sync {
    async fn start(&mut self, config: RunnerConfig) -> Result<(), AppError>;

    async fn prompt(&mut self, req: PromptRequest) -> Result<AgentStream, AppError>;

    async fn cancel_turn(&mut self, turn_id: TurnId) -> Result<(), AppError>;

    async fn respond_to_approval(
        &mut self,
        req_id: ApprovalId,
        decision: ApprovalDecision,
    ) -> Result<(), AppError>;

    async fn set_mode(&mut self, mode: RunnerMode) -> Result<(), AppError>;

    /// This runner's own session id, once it is known — the value a later
    /// cold spawn passes back as `RunnerConfig.resume`. `None` if the
    /// runner has not started, or if this vendor has no resumable session
    /// concept.
    ///
    /// **Async on purpose.** For Claude it resolves immediately: the CLI
    /// accepts a caller-supplied `--session-id`, so the id is known at
    /// spawn. Other vendors assign their own asynchronously (codex mints a
    /// thread id in an RPC its `start()` does not await), so a synchronous
    /// accessor would return `None` for them and silently never persist —
    /// leaving those chats permanently unresumable. Awaiting here keeps one
    /// call site correct for both.
    async fn runner_session_id(&self) -> Option<String>;

    async fn dispose(self: Box<Self>) -> Result<(), AppError>;
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct RunnerConfig {
    /// Wire-level model id, e.g. "claude-sonnet-5".
    pub model: String,

    /// Absolute cap on the entire stream; `None` means "no cap". Chat uses
    /// `None`; extraction uses `Some(30_000)`.
    pub timeout_ms: Option<u64>,

    /// Optional system prompt. If `None`, provider's default is used.
    pub system_prompt: Option<String>,

    /// Tool definitions. Empty in v1 — `ClaudeRunner` does not act on this
    /// field at all; real MCP tools reach the CLI via `RunnerConfig.mcp`
    /// instead, resolved by the CLI itself, not through this list.
    pub tools: Vec<ToolDef>,

    /// Approval policy for tool calls. Latent in v1 (no tools ship), kept
    /// for the same forward-compat reason as `tools`.
    pub approval_policy: ApprovalPolicy,

    /// MCP server this runner should expose to the CLI. `None` means
    /// no `mcp.json` is written and the CLI's built-in tools are disabled
    /// outright (`--tools ""`) — used by extraction and Conversation-scope
    /// chat, where context is stuffed directly into `system_prompt` instead
    /// of fetched via tool calls.
    pub mcp: Option<McpConfig>,

    /// The runner's own session id to resume, from
    /// `chat_sessions.runner_session_id`. `None` starts a fresh
    /// conversation; `Some(id)` continues that one, with the vendor
    /// rehydrating its own context — including prior tool calls and their
    /// results, which is why Mnemos never replays history itself.
    /// Extraction always passes `None` (nothing ever resumes it).
    pub resume: Option<String>,
}

/// Points a chat runner at the real `mnemos-mcp-server` binary.
/// Scoping (Project vs. Everything) is *not* done here — the CLI's MCP
/// client has no notion of a scope filter, so the caller instead tells the
/// model which `project_id` to pass via `RunnerConfig.system_prompt`
/// (verified against a real `claude` CLI run: the model reliably
/// passes an instructed `project_id` argument on every scoped tool call).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct McpConfig {
    /// Absolute path to the `mnemos-mcp-server` binary.
    pub server_binary: String,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct PromptRequest {
    /// New user content for this turn.
    pub content: Vec<UserContent>,

    /// Optional caller-provided `TurnId`. If `None`, the runner assigns one.
    pub turn_id: Option<TurnId>,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub enum UserContent {
    Text(String),
    // Image { .. } in v1.1.
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct TurnRecord {
    pub role: TurnRole,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub enum TurnRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub enum ApprovalPolicy {
    /// Emit `ApprovalRequest`; block the turn until `respond_to_approval`.
    Interactive,
    /// Auto-approve read-only tools; auto-deny anything destructive.
    AutoDenyDestructive,
    /// Auto-approve everything. Used only in tests.
    #[cfg(test)]
    AutoApprove,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub enum ApprovalDecision {
    Approve,
    Deny,
    /// The turn was cancelled while a decision was still pending.
    Cancelled,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub enum RunnerMode {
    ChatInteractive,
    ChatAutoAccept,
    ExtractionReadOnly,
    /// v1.1 surface only — accepted but not exercised in v1.
    Plan,
}

/// Latent in v1 — `ClaudeRunner` never populates this (real MCP tools flow
/// through `RunnerConfig.mcp` instead, resolved by the CLI itself). Shape
/// kept so a Rust-side tool-calling loop can slot in without changing
/// `RunnerConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub human_readable_template: String,
    pub destructive: bool,
    pub timeout_ms: Option<u64>,
}

/// The stream envelope. A discriminated union versioned by
/// shape, not an integer — consumers `match` and route unrecognized shapes
/// to `Notice{Info}` for forwards-compat.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    /// A slice of assistant text. Consumers append verbatim.
    TokenDelta { turn_id: TurnId, text: String },

    /// The model emitted a `tool_use` block. v1 has no Rust-side
    /// tool-calling loop — the CLI resolves any MCP tool call itself, so
    /// this is purely informational — logged, never dispatched, no matching
    /// `ToolResult` will ever follow it.
    ToolCall {
        turn_id: TurnId,
        call_id: String,
        tool_name: String,
        args: serde_json::Value,
        human_readable: String,
    },

    /// Latent in v1 — reserved for a future Rust-side tool-calling loop.
    ToolResult {
        turn_id: TurnId,
        call_id: String,
        ok: bool,
        summary: String,
        raw: serde_json::Value,
    },

    /// Latent in v1 — no tools ship, so the CLI never emits a
    /// permission-request frame in practice.
    ApprovalRequest {
        turn_id: TurnId,
        request_id: ApprovalId,
        tool_name: String,
        args: serde_json::Value,
        destructive: bool,
    },

    /// Free-form status message. Unrecognized shapes route here as `Info`.
    ///
    /// The field is named `notice_kind`, not `kind` — the enum's own serde
    /// tag (`#[serde(tag = "kind")]`) already claims `kind`, and
    /// `specta`/`serde` reject a tag/field name collision.
    Notice {
        turn_id: TurnId,
        notice_kind: NoticeKind,
        text: String,
    },

    /// Terminal success. Consumers stop reading after this.
    Complete {
        turn_id: TurnId,
        stop_reason: StopReason,
        usage: Usage,
    },

    /// Terminal failure. Consumers stop reading after this.
    Error { turn_id: TurnId, error: AppError },
}

#[derive(Debug, Clone, Serialize, Type)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    StopSequence,
    ToolUse,
    Cancelled,
}

#[derive(Debug, Clone, Default, Serialize, Type)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_input_tokens: Option<u32>,
    pub cache_read_input_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub enum NoticeKind {
    Info,
    Warn,
    RateLimit,
}
