//! `AgentRunner` — Mnemos' single, pluggable point of contact with LLM
//! providers (LLD-07). Every LLM turn (chat, memory extraction) flows
//! through this trait; nothing else in the codebase talks to a model.
//!
//! **v1 scope (W8):** `ClaudeRunner` only, text-only. No tool-calling loop
//! (MCP/W16 doesn't exist yet) — a `tool_use` content block the CLI happens
//! to emit is translated into an inert `AgentEvent::ToolCall` and logged,
//! never dispatched or awaited for a result. No other vendor adapters
//! (Codex/OpenCode/Gemini/Ollama are W8b, v1.1).

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

/// Six-method shape, verbatim from HLD §4.4 / LLD-07 §3.1.
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

    async fn dispose(self: Box<Self>) -> Result<(), AppError>;
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct RunnerConfig {
    /// Wire-level model id, e.g. "claude-sonnet-5" (LLD-07 §4.4).
    pub model: String,

    /// Absolute cap on the entire stream; `None` means "no cap". Chat uses
    /// `None`; extraction uses `Some(30_000)`.
    pub timeout_ms: Option<u64>,

    /// Optional system prompt. If `None`, provider's default is used.
    pub system_prompt: Option<String>,

    /// Tool definitions. Empty in v1 — no MCP bridge exists yet (W16).
    /// `ClaudeRunner` does not act on this field at all this wave; it exists
    /// so the trait shape does not have to change when W16/LLD-08 land.
    pub tools: Vec<ToolDef>,

    /// Approval policy for tool calls. Latent in v1 (no tools ship), kept
    /// for the same forward-compat reason as `tools`.
    pub approval_policy: ApprovalPolicy,

    /// MCP server this runner should expose to the CLI (W13a). `None` means
    /// no `mcp.json` is written and the CLI's built-in tools are disabled
    /// outright (`--tools ""`) — used by extraction and Conversation-scope
    /// chat, where context is stuffed directly into `system_prompt` instead
    /// of fetched via tool calls (LLD-07 §6.1).
    pub mcp: Option<McpConfig>,
}

/// Points a chat runner at the real `mnemos-mcp-server` binary (LLD-08).
/// Scoping (Project vs. Everything) is *not* done here — the CLI's MCP
/// client has no notion of a scope filter, so the caller instead tells the
/// model which `project_id` to pass via `RunnerConfig.system_prompt`
/// (verified against a real `claude` CLI run this wave: the model reliably
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

    /// Prior turns for context. Unused by `ClaudeRunner` — the CLI's
    /// `--session-id` machinery rehydrates its own context (LLD-07 §5.1).
    /// Kept on the wire shape so a future vendor adapter without
    /// server-side session memory can use it.
    pub history: Vec<TurnRecord>,

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

/// Latent in v1 — no MCP bridge exists yet (W16). Shape kept so LLD-08 can
/// slot in without changing `RunnerConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub human_readable_template: String,
    pub destructive: bool,
    pub timeout_ms: Option<u64>,
}

/// The stream envelope (LLD-07 §3.3). A discriminated union versioned by
/// shape, not an integer — consumers `match` and route unrecognized shapes
/// to `Notice{Info}` for forwards-compat.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    /// A slice of assistant text. Consumers append verbatim.
    TokenDelta { turn_id: TurnId, text: String },

    /// The model emitted a `tool_use` block. v1 has no tool-calling loop
    /// (no MCP config is ever passed to the CLI), so this is purely
    /// informational — logged, never dispatched, no matching `ToolResult`
    /// will ever follow it this wave.
    ToolCall {
        turn_id: TurnId,
        call_id: String,
        tool_name: String,
        args: serde_json::Value,
        human_readable: String,
    },

    /// Latent in v1 — reserved for W16's tool-calling loop.
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

    /// Free-form status message. `unknown_x -> Notice{Info}` per SUPERSET.
    ///
    /// Deviation from LLD-07 §3.3's literal sketch: the field is named
    /// `notice_kind`, not `kind` — the LLD's own sketch names both the
    /// enum's serde tag (`#[serde(tag = "kind")]`) and this field `kind`,
    /// which `specta`/`serde` reject as a tag/field name collision. Wire
    /// shape is otherwise unchanged.
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
