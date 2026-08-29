//! Domain types for the v1 storage slice. IDs are plain `String` UUIDs for
//! this wave rather than the newtype wrappers LLD-01 §3.1 sketches — path
//! traversal safety is enforced at the `fs::paths` boundary regardless
//! (`validate_uuid`), and a full `ProjectId`/`ConversationId` newtype system
//! is deferred to whichever wave first needs it on the `tauri-specta`
//! boundary. `Conversation` serves both list and detail reads for the same
//! reason — LLD-01's `ConversationSummary`/`Conversation` split is a
//! frontend read-shape optimization, not a storage-layer correctness need.

use serde::{Deserialize, Serialize};
use specta::Type;

pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs() as i64
}

pub(crate) fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, Type)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub enum ConversationStatus {
    Recording,
    Processing,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, Type)]
pub struct Conversation {
    pub id: String,
    /// Recordings never require a project (W15 design decision) — `None`
    /// means unfiled, a first-class, permanent state, not a placeholder.
    pub project_id: Option<String>,
    pub title: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_s: Option<i64>,
    pub status: ConversationStatus,
    pub runner_id: Option<String>,
    pub starred: bool,
    pub archived: bool,
    pub notes: Option<String>,
    pub deleted_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewConversation {
    pub project_id: Option<String>,
    pub title: String,
    pub started_at: i64,
    pub runner_id: Option<String>,
}

/// Sort order for [`ConversationFilter`]. Sorting is a *query* concern, not a
/// view concern: once a list is a page rather than the whole set, sorting it
/// in the client sorts only the rows that happen to be loaded, which is worse
/// than not sorting at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ConversationOrder {
    /// Most recent first — every list surface's default.
    #[default]
    StartedDesc,
    StartedAsc,
}

/// W18: every field past `include_archived` was added to make the six
/// unbounded `list_conversations` callers bounded. `limit: None` still means
/// "every row" and is deliberately kept, not removed — a few internal callers
/// (crash-recovery scans) genuinely want the whole set and are bounded by
/// something other than library size. What is *not* allowed is a UI surface
/// leaving it `None`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct ConversationFilter {
    /// `None` = every project (Everything scope). Note this is not the same
    /// as `unfiled_only`: `None` includes filed *and* unfiled conversations.
    pub project_id: Option<String>,
    /// `project_id IS NULL` — the Recordings page's "unfiled" scope (W15:
    /// unfiled is a permanent first-class state, never a project).
    pub unfiled_only: bool,
    pub include_archived: bool,
    pub starred_only: bool,
    /// Inclusive bounds on `started_at`.
    pub since: Option<i64>,
    pub until: Option<i64>,
    /// Case-insensitive substring match on the title. Deliberately `LIKE`
    /// rather than the FTS5 index: FTS is a ranked keyword search over four
    /// content kinds and cannot be composed with these filters or with
    /// `LIMIT`/`OFFSET` paging without ranking the whole corpus first. Title
    /// filtering here is a *filter*, not a search — ⌘K remains the search.
    pub title_query: Option<String>,
    pub order: ConversationOrder,
    /// `None` = unbounded. Every UI caller must set it.
    pub limit: Option<u32>,
    pub offset: u32,
}

/// Projects are a human-curated set, so this exists for the MCP tool layer
/// (which must bound every response it hands an agent) rather than because
/// the app's own project list is at risk of growing without bound.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectFilter {
    /// Archived projects are excluded unless this is set. Filtered in SQL,
    /// not by the caller: a filter applied *after* `LIMIT` returns short
    /// pages and makes "is there another page" unanswerable.
    pub include_archived: bool,
    /// `None` = unbounded.
    pub limit: Option<u32>,
    pub offset: u32,
}

/// One page of a list, plus the size of the full result set the page was
/// drawn from. `total` is what lets a section title read `Conversations (128)`
/// and a reveal control read `108 remaining` without loading 128 rows to
/// count them — it is computed by a `COUNT(*)` over the same filter, in the
/// same call, so the count and the rows can never disagree.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: u32,
}

/// Provenance of a `*_hint` a person can edit. The distinction is load-bearing:
/// `replace_extraction_rows` rebuilds every model-derived row on a
/// re-extraction, so without this flag a user's correction is indistinguishable
/// from the guess it replaced and gets thrown away with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, sqlx::Type, Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub enum HintSource {
    #[default]
    Model,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, Type)]
pub struct ActionItem {
    pub id: String,
    pub conv_id: String,
    pub text: String,
    pub assignee_hint: Option<String>,
    pub assignee_source: HintSource,
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

/// Read model for a `decisions` row (LLD-01 §3.1 sketches this; W4's
/// Implementation status never added it — only `ActionItem` got a read
/// struct — because nothing read decisions back before W12b's Conversation
/// Detail page. Added here, following `ActionItem`'s shape.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, Type)]
pub struct Decision {
    pub id: String,
    /// `None` for a standalone decision (schema supports it — see
    /// `action_items`' migration comment — though nothing creates one yet;
    /// only action items have a "+" as of W19).
    pub conv_id: Option<String>,
    pub project_id: Option<String>,
    pub statement: String,
    pub quote: Option<String>,
    pub decided_by_hint: Option<String>,
    pub source_ts: Option<i64>,
    pub added_manually: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewOpenQuestion {
    pub question: String,
    pub raised_by_hint: Option<String>,
    pub source_ts: Option<i64>,
}

/// Read model for an `open_questions` row — same rationale as [`Decision`].
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, Type)]
pub struct OpenQuestion {
    pub id: String,
    pub conv_id: String,
    pub question: String,
    pub raised_by_hint: Option<String>,
    /// Who owes the answer — distinct from `raised_by_hint`, which records
    /// who asked and is never edited. The model never populates this in v1.
    pub owner_hint: Option<String>,
    pub owner_source: HintSource,
    pub source_ts: Option<i64>,
    pub resolved_conv_id: Option<String>,
    pub resolved_at: Option<i64>,
    pub added_manually: bool,
    pub created_at: i64,
}

/// Agent-suggested bookmark (LLD-05 §4.3, §10 Q1). Never deletes a
/// user-tapped `bookmarks` row — [`crate::db::service::StorageService::replace_extraction_rows`]
/// only inserts rows whose `(conv_id, ts_ms, label)` isn't already present.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewBookmark {
    pub label: String,
    pub ts_ms: i64,
}

/// Extraction agent output for one conversation, written in a single
/// transaction (LLD-01 §4.4) — never a partial set of rows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractionBundle {
    pub action_items: Vec<NewActionItem>,
    pub decisions: Vec<NewDecision>,
    pub open_questions: Vec<NewOpenQuestion>,
    pub bookmarks: Vec<NewBookmark>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, Type)]
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

/// `Type` (W13-history wave): exposed to the frontend by
/// `chat_start_new_session`/`chat_list_sessions` (design doc §2.5, §4).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, Type)]
pub struct ChatSession {
    pub id: String,
    pub runner_id: Option<String>,
    pub scope_type: ChatScopeType,
    pub scope_id: Option<String>,
    pub session_id: Option<String>,
    pub epoch: String,
    pub status: String,
    pub title: Option<String>,
    /// `None` = this is the active session for its `(runner, scope)` —
    /// `find_chat_session_by_scope` only ever returns one of these.
    /// `Some(id)` = a previous "New chat" (`start_new_session`) replaced
    /// this row with `id`; still renameable/listable, just not the one a
    /// new message resolves to (design doc §2.5).
    pub superseded_by_id: Option<String>,
    pub message_count: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub cost_micros: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One journal row's payload. The event shape itself belongs to LLD-07/12c;
/// this layer only guarantees journal-then-projection atomicity around it.
/// `Type` (W13-history wave): exposed to the frontend via
/// `chat_get_session_history` so `MessageList` can project real history
/// instead of the permanent `[]` stub it used before.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, Type)]
pub struct ChatEventRecord {
    pub session_id: String,
    pub epoch: String,
    pub seq: i64,
    pub ts: i64,
    pub event_json: serde_json::Value,
}

/// One row of the `mnemos-mcp-server` (W16 / LLD-08 §3.1) project listing —
/// `Project` itself carries no aggregate counters, so `list_projects` joins
/// this in-memory by `project_id`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProjectActivityStat {
    pub project_id: String,
    pub conversation_count: i64,
    pub last_activity_at: Option<i64>,
}

/// Filter for [`crate::db::service::StorageService::list_action_items_global`]
/// (LLD-08 §3.6) — distinct from the per-conversation `list_action_items`
/// W12b added; this one fans out across every conversation (optionally
/// scoped to one project) via a single joined query rather than N per-conversation
/// calls. `contact_id` from the LLD-08 sketch is intentionally absent: there
/// is no `speakers`/`contacts` table until v1.3 diarization, so the MCP tool
/// layer rejects that arg before it would ever reach this filter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActionItemFilter {
    pub project_id: Option<String>,
    pub since: Option<i64>,
    pub until: Option<i64>,
    pub include_done: bool,
    /// Home's "Your to-dos" — exact match on `assignee_hint = 'You'`, the
    /// literal string the model (and the assignee picker) writes for the
    /// user's own speech. Not a contact id: there is no contacts table until
    /// v1.3, and "You" is the one value that is always unambiguous regardless
    /// of who is actually using the app.
    pub assigned_to_me: bool,
    pub limit: u32,
    /// Rows to skip before `limit` — the paging cursor for these lists.
    pub offset: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, Type)]
pub struct ActionItemWithSource {
    pub id: String,
    /// `None` for a standalone item (added from Home or a Project page, no
    /// source conversation) — `project_id` below is where it's scoped
    /// instead, when it has a scope at all.
    pub conv_id: Option<String>,
    pub project_id: Option<String>,
    pub text: String,
    pub assignee_hint: Option<String>,
    pub assignee_source: HintSource,
    pub due_hint: Option<String>,
    pub source_ts: Option<i64>,
    pub done: bool,
    pub dismissed: bool,
    pub created_at: i64,
}

/// Filter for [`crate::db::service::StorageService::list_decisions_global`]
/// — same rationale as [`ActionItemFilter`]. Added in W17c for Project
/// Memory's Decisions section (05_PROJECT_MEMORY.md §2), which is the one
/// reactive section that had no cross-conversation read: action items and
/// open questions both got theirs in W16 for the MCP tools, decisions did
/// not because no MCP tool needed them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DecisionFilter {
    pub project_id: Option<String>,
    pub since: Option<i64>,
    pub until: Option<i64>,
    pub limit: u32,
    /// Rows to skip before `limit` — the paging cursor for these lists.
    pub offset: u32,
}

/// Filter for [`crate::db::service::StorageService::list_open_questions_global`]
/// — same rationale as [`ActionItemFilter`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenQuestionFilter {
    pub project_id: Option<String>,
    pub since: Option<i64>,
    pub until: Option<i64>,
    /// Widens the default (unresolved only) to *include* resolved questions.
    pub include_resolved: bool,
    /// Narrows to resolved questions only. Takes precedence over
    /// `include_resolved`, which is a superset flag and cannot express "just
    /// the resolved ones" — the Resolved tab needs its own exact count and its
    /// own paging, not a client-side filter over a combined page.
    pub resolved_only: bool,
    pub limit: u32,
    /// Rows to skip before `limit` — the paging cursor for these lists.
    pub offset: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, Type)]
pub struct OpenQuestionWithSource {
    pub id: String,
    /// `None` for a standalone question — same rationale as
    /// [`ActionItemWithSource::conv_id`].
    pub conv_id: Option<String>,
    pub project_id: Option<String>,
    pub question: String,
    pub raised_by_hint: Option<String>,
    /// Who owes the answer — distinct from `raised_by_hint`, which records
    /// who asked and is never edited. The model never populates this in v1.
    pub owner_hint: Option<String>,
    pub owner_source: HintSource,
    pub source_ts: Option<i64>,
    pub resolved_conv_id: Option<String>,
    pub resolved_at: Option<i64>,
    pub created_at: i64,
}

/// What matched an FTS5 hit (LLD-08 §3.2's `kind` enum, trimmed to the four
/// SQLite-content kinds the W16 migration actually indexes — `transcript`
/// and `summary` are filesystem content, not SQLite rows, and stay out of
/// scope until the vector/RAG tier, W14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FtsHitKind {
    ConversationTitle,
    Decision,
    ActionItem,
    OpenQuestion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FtsHit {
    /// `None` for a hit on a standalone action item (no source conversation).
    pub conversation_id: Option<String>,
    pub project_id: Option<String>,
    pub kind: FtsHitKind,
    pub snippet: String,
    /// Raw SQLite `bm25()` value (more negative = more relevant). Not
    /// normalized across the four source tables — see
    /// [`crate::db::service::StorageService::fts_search`] doc comment.
    pub score: f64,
    pub source_ts: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub enum PipelineStep {
    Finalizing,
    Transcribing,
    Extracting,
    Done,
    Failed,
}
