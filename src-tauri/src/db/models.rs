//! Domain types for the v1 storage slice. IDs are plain `String` UUIDs for
//! this wave rather than the newtype wrappers LLD-01 §3.1 sketches — path
//! traversal safety is enforced at the `fs::paths` boundary regardless
//! (`validate_uuid`), and a full `ProjectId`/`ConversationId` newtype system
//! is deferred to whichever wave first needs it on the `tauri-specta`
//! boundary. `Conversation` serves both list and detail reads for the same
//! reason — LLD-01's `ConversationSummary`/`Conversation` split is a
//! frontend read-shape optimization, not a storage-layer correctness need.

use serde::{Deserialize, Serialize};

pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs() as i64
}

pub(crate) fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub pinned: bool,
    pub archived: bool,
    pub deleted_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewProject {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub pinned: Option<bool>,
    pub archived: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub enum ConversationStatus {
    Recording,
    Processing,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Conversation {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_s: Option<i64>,
    pub status: ConversationStatus,
    pub runner_id: Option<String>,
    pub starred: bool,
    pub archived: bool,
    pub deleted_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewConversation {
    pub project_id: String,
    pub title: String,
    pub started_at: i64,
    pub runner_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConversationFilter {
    /// `None` = every project (Everything scope).
    pub project_id: Option<String>,
    pub include_archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ActionItem {
    pub id: String,
    pub conv_id: String,
    pub text: String,
    pub assignee_hint: Option<String>,
    pub due_hint: Option<String>,
    pub source_ts: Option<i64>,
    pub done: bool,
    pub dismissed: bool,
    pub added_manually: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewActionItem {
    pub text: String,
    pub assignee_hint: Option<String>,
    pub due_hint: Option<String>,
    pub source_ts: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewDecision {
    pub statement: String,
    pub quote: Option<String>,
    pub decided_by_hint: Option<String>,
    pub source_ts: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewOpenQuestion {
    pub question: String,
    pub raised_by_hint: Option<String>,
    pub source_ts: Option<i64>,
}

/// Extraction agent output for one conversation, written in a single
/// transaction (LLD-01 §4.4) — never a partial set of rows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractionBundle {
    pub action_items: Vec<NewActionItem>,
    pub decisions: Vec<NewDecision>,
    pub open_questions: Vec<NewOpenQuestion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub enum ChatScopeType {
    Everything,
    Project,
    Conversation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewChatSession {
    pub runner_id: Option<String>,
    pub scope_type: ChatScopeType,
    /// Required unless `scope_type` is `Everything`.
    pub scope_id: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChatSession {
    pub id: String,
    pub runner_id: Option<String>,
    pub scope_type: ChatScopeType,
    pub scope_id: Option<String>,
    pub session_id: Option<String>,
    pub epoch: String,
    pub status: String,
    pub title: Option<String>,
    pub message_count: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub cost_micros: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One journal row's payload. The event shape itself belongs to LLD-07/12c;
/// this layer only guarantees journal-then-projection atomicity around it.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChatEventRecord {
    pub session_id: String,
    pub epoch: String,
    pub seq: i64,
    pub ts: i64,
    pub event_json: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub enum PipelineStep {
    Finalizing,
    Transcribing,
    Extracting,
    Done,
    Failed,
}
