//! `StorageService` (LLD-01 §3.1) — the only path any command layer uses to
//! reach persistent state. Trimmed to the v1 slice: no speaker/contact
//! methods (v1.3 diarization/contacts), no vector search (v1.2), no
//! calendar/integrations (v1.4), no export/import (v1.1).

use std::path::PathBuf;

use async_trait::async_trait;
use sqlx::Row;

use crate::db::models::{
    unix_now, ActionItem, ActionItemFilter, ActionItemWithSource, ChatEventRecord, ChatScopeType,
    ChatSession, Conversation, ConversationFilter, ConversationOrder, ConversationStatus, Decision,
    DecisionFilter, ExtractionBundle, FtsHit, FtsHitKind, HintSource, NewActionItem,
    NewChatSession, NewConversation, NewProject, OpenQuestion, OpenQuestionFilter,
    OpenQuestionWithSource, PipelineStep, Project, ProjectActivityStat, ProjectFilter,
    ProjectPatch,
};
use crate::db::pending_deletes::{self, StuckDelete};
use crate::db::{with_write_tx, DbPools};
use crate::error::AppError;
use crate::fs::{atomic, paths};

fn db_err(e: sqlx::Error) -> AppError {
    AppError::storage(e.to_string())
}

/// Escapes the three characters `LIKE` treats specially so a title filter of
/// `100%` matches a literal `100%` rather than "anything starting with 100".
/// Paired with an explicit `ESCAPE '\'` in the SQL — SQLite has no default
/// escape character, so without the clause the backslashes we insert here
/// would themselves be matched literally.
fn escape_like(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// The `WHERE` clause for `list_conversations` and `count_conversations`.
///
/// Both derive their predicate from this one function on purpose. The count
/// is what the UI renders as `Conversations (128)` and `108 remaining`; if it
/// were built from a separately-maintained clause, the two could drift and
/// produce a "Show 20 more" button that fetches an empty page. Placeholders
/// are positional `?` and [`bind_conversation_filter`] binds in exactly this
/// order — the two must be edited together.
fn conversation_where(filter: &ConversationFilter) -> String {
    let mut sql = String::from(" WHERE deleted_at IS NULL");
    if filter.project_id.is_some() {
        sql.push_str(" AND project_id = ?");
    }
    if filter.unfiled_only {
        sql.push_str(" AND project_id IS NULL");
    }
    if !filter.include_archived {
        sql.push_str(" AND archived = 0");
    }
    if filter.starred_only {
        sql.push_str(" AND starred = 1");
    }
    if filter.since.is_some() {
        sql.push_str(" AND started_at >= ?");
    }
    if filter.until.is_some() {
        sql.push_str(" AND started_at <= ?");
    }
    if filter.title_query.is_some() {
        sql.push_str(" AND title LIKE ? ESCAPE '\\'");
    }
    sql
}

/// `WHERE` for the cross-conversation action-item reads. Same contract as
/// [`conversation_where`]: the list and the count must share one predicate,
/// and [`bind_action_item_filter`] binds in exactly this order.
fn action_item_where(filter: &ActionItemFilter) -> String {
    // `c` is a LEFT JOIN — null for a standalone item (`ai.conv_id IS NULL`).
    // The deleted-conversation check only applies when there IS a
    // conversation; a standalone item has nothing to be soft-deleted.
    let mut sql =
        String::from(" WHERE ai.dismissed = 0 AND (ai.conv_id IS NULL OR c.deleted_at IS NULL)");
    if filter.project_id.is_some() {
        // Matches either the item's own `project_id` (standalone, scoped to
        // a project) or its conversation's (the ordinary, derived case) —
        // never both, per the table's CHECK constraint.
        sql.push_str(" AND COALESCE(ai.project_id, c.project_id) = ?");
    }
    if filter.assigned_to_me {
        // 'You' is a fixed literal (see the field's doc comment), not a bind
        // parameter — nothing user-controlled reaches this string.
        sql.push_str(" AND ai.assignee_hint = 'You'");
    }
    if !filter.include_done {
        sql.push_str(" AND ai.done = 0");
    }
    if filter.since.is_some() {
        sql.push_str(" AND COALESCE(ai.source_ts, ai.created_at) >= ?");
    }
    if filter.until.is_some() {
        sql.push_str(" AND COALESCE(ai.source_ts, ai.created_at) <= ?");
    }
    sql
}

macro_rules! bind_action_item_filter {
    ($query:expr, $filter:expr) => {{
        let mut q = $query;
        if let Some(project_id) = &$filter.project_id {
            q = q.bind(project_id.clone());
        }
        if let Some(since) = $filter.since {
            q = q.bind(since);
        }
        if let Some(until) = $filter.until {
            q = q.bind(until);
        }
        q
    }};
}

/// `WHERE` for the cross-conversation decision reads.
fn decision_where(filter: &DecisionFilter) -> String {
    let mut sql = String::from(" WHERE (d.conv_id IS NULL OR c.deleted_at IS NULL)");
    if filter.project_id.is_some() {
        sql.push_str(" AND COALESCE(d.project_id, c.project_id) = ?");
    }
    if filter.since.is_some() {
        sql.push_str(" AND COALESCE(d.source_ts, d.created_at) >= ?");
    }
    if filter.until.is_some() {
        sql.push_str(" AND COALESCE(d.source_ts, d.created_at) <= ?");
    }
    sql
}

macro_rules! bind_decision_filter {
    ($query:expr, $filter:expr) => {{
        let mut q = $query;
        if let Some(project_id) = &$filter.project_id {
            q = q.bind(project_id.clone());
        }
        if let Some(since) = $filter.since {
            q = q.bind(since);
        }
        if let Some(until) = $filter.until {
            q = q.bind(until);
        }
        q
    }};
}

/// `WHERE` for the cross-conversation open-question reads.
fn open_question_where(filter: &OpenQuestionFilter) -> String {
    let mut sql = String::from(" WHERE (oq.conv_id IS NULL OR c.deleted_at IS NULL)");
    if filter.project_id.is_some() {
        sql.push_str(" AND COALESCE(oq.project_id, c.project_id) = ?");
    }
    if filter.resolved_only {
        sql.push_str(" AND oq.resolved_conv_id IS NOT NULL");
    } else if !filter.include_resolved {
        sql.push_str(" AND oq.resolved_conv_id IS NULL");
    }
    if filter.since.is_some() {
        sql.push_str(" AND COALESCE(oq.source_ts, oq.created_at) >= ?");
    }
    if filter.until.is_some() {
        sql.push_str(" AND COALESCE(oq.source_ts, oq.created_at) <= ?");
    }
    sql
}

macro_rules! bind_open_question_filter {
    ($query:expr, $filter:expr) => {{
        let mut q = $query;
        if let Some(project_id) = &$filter.project_id {
            q = q.bind(project_id.clone());
        }
        if let Some(since) = $filter.since {
            q = q.bind(since);
        }
        if let Some(until) = $filter.until {
            q = q.bind(until);
        }
        q
    }};
}

/// Binds [`conversation_where`]'s placeholders, in its order. A macro rather
/// than a function because the two call sites have different sqlx query types
/// (`QueryAs` for the rows, `QueryScalar` for the count) with no shared trait
/// exposing `bind`.
macro_rules! bind_conversation_filter {
    ($query:expr, $filter:expr) => {{
        let mut q = $query;
        if let Some(project_id) = &$filter.project_id {
            q = q.bind(project_id.clone());
        }
        if let Some(since) = $filter.since {
            q = q.bind(since);
        }
        if let Some(until) = $filter.until {
            q = q.bind(until);
        }
        if let Some(title_query) = &$filter.title_query {
            q = q.bind(format!("%{}%", escape_like(title_query)));
        }
        q
    }};
}

/// Shared by `open_chat_session` and `start_new_chat_session` — a
/// Project/Conversation-scope session with no `scope_id` is meaningless
/// (which project? which conversation?); Everything is the one scope that's
/// legitimately global.
fn validate_chat_scope(new: &NewChatSession) -> Result<(), AppError> {
    if !matches!(new.scope_type, ChatScopeType::Everything) && new.scope_id.is_none() {
        return Err(AppError::Validation {
            message: "scope_id is required unless scope_type is everything".into(),
            field: Some("scope_id".into()),
        });
    }
    Ok(())
}

#[async_trait]
pub trait StorageService: Send + Sync {
    // -- Project CRUD -----------------------------------------------------
    async fn list_projects(&self, filter: ProjectFilter) -> Result<Vec<Project>, AppError>;
    async fn count_projects(&self, include_archived: bool) -> Result<u32, AppError>;
    async fn get_project(&self, id: &str) -> Result<Project, AppError>;
    async fn create_project(&self, input: NewProject) -> Result<Project, AppError>;
    async fn update_project(&self, id: &str, patch: ProjectPatch) -> Result<Project, AppError>;
    /// Enqueues the delete (LLD-01 §7). Returns as soon as the Phase 0 mark
    /// transaction commits; the remaining phases run via
    /// [`StorageService::resume_pending_deletes`].
    async fn delete_project(&self, id: &str) -> Result<(), AppError>;

    // -- Conversation CRUD --------------------------------------------------
    async fn list_conversations(
        &self,
        filter: ConversationFilter,
    ) -> Result<Vec<Conversation>, AppError>;
    /// Size of the full result set `list_conversations` would return for the
    /// same filter, ignoring its `limit`/`offset`.
    async fn count_conversations(&self, filter: ConversationFilter) -> Result<u32, AppError>;
    async fn get_conversation(&self, id: &str) -> Result<Conversation, AppError>;
    async fn insert_conversation(&self, row: NewConversation) -> Result<Conversation, AppError>;
    async fn update_conversation_status(
        &self,
        id: &str,
        status: ConversationStatus,
        ended_at: Option<i64>,
        duration_s: Option<i64>,
    ) -> Result<(), AppError>;
    async fn delete_conversation(&self, id: &str) -> Result<(), AppError>;
    /// Crash recovery (12_CORNER_CASES.md "App crashes & recovery" §Mid-
    /// recording crash). A conversation is only ever left at `status =
    /// 'recording'` while a live session holds it in
    /// `RecordingRegistry` — if the process exits (crash, force-quit) before
    /// `stop_recording` runs, the row is orphaned at this status forever
    /// with no in-memory session to finish it. Scanned once at startup by
    /// the frontend (`recording.list_interrupted`) rather than reconciled
    /// automatically, since only the user can say whether a partial
    /// recording is worth keeping.
    async fn list_interrupted_recordings(&self) -> Result<Vec<Conversation>, AppError>;
    /// W17b — mid-processing counterpart: a conversation can only be
    /// `status = 'processing'` while a live `run_post_recording_pipeline`
    /// task holds it. A crash/force-quit before that task finishes orphans
    /// the row here forever with nothing left to publish `processing-
    /// progress` events. Same "scan once at boot" pattern as
    /// `list_interrupted_recordings` — the registry that would hold a live
    /// task is always empty this early.
    async fn list_stuck_processing(&self) -> Result<Vec<Conversation>, AppError>;
    /// Conversation Detail's editable title (LLD-11 §3.2 `<EditableTitle>`).
    /// Rejects an empty/whitespace-only title with `AppError::Validation`.
    async fn update_conversation_title(
        &self,
        id: &str,
        title: &str,
    ) -> Result<Conversation, AppError>;
    /// Conversation Detail's persisted notes (debug-session patch — the
    /// recording screen's `<NotesPane>` draft used to be local-only,
    /// LLD-11 §3.1). `notes` may be empty/whitespace; unlike the title this
    /// has no non-empty requirement — clearing the notes field is valid.
    async fn update_conversation_notes(
        &self,
        id: &str,
        notes: &str,
    ) -> Result<Conversation, AppError>;
    /// The DB half of project (re)assignment — `None` files under "no
    /// project" (W15 design decision: always a valid, permanent choice, not
    /// a stopgap). The filesystem move (old conversation dir -> new) is the
    /// caller's job (`commands::conversation::conversation_set_project`) —
    /// this layer only guarantees the FK is valid (bad `project_id` fails
    /// the `UPDATE` via the `projects(id)` foreign key).
    async fn update_conversation_project(
        &self,
        id: &str,
        project_id: Option<&str>,
    ) -> Result<Conversation, AppError>;

    // -- Structured extraction items -----------------------------------------
    /// One transaction: every row or none (LLD-01 §4.4).
    async fn bulk_insert_extraction(
        &self,
        conv_id: &str,
        items: ExtractionBundle,
    ) -> Result<(), AppError>;
    /// LLD-05 §4.4 / §7, §10 Q6 — the re-extraction write used by
    /// `extracting`: one write-tx that deletes every non-manual
    /// `action_items`/`decisions`/`open_questions` row for `conv_id`,
    /// inserts the new agent output, and inserts only the agent-suggested
    /// `bookmarks` rows that aren't already present (user-tapped bookmarks
    /// are never touched by this method — bookmarks are never bulk-deleted).
    async fn replace_extraction_rows(
        &self,
        conv_id: &str,
        items: ExtractionBundle,
    ) -> Result<(), AppError>;
    async fn set_action_item_done(&self, id: &str, done: bool) -> Result<ActionItem, AppError>;
    /// Sets who owes an action item, and stamps the row `assignee_source =
    /// 'manual'` so `replace_extraction_rows` carries the correction across
    /// the next re-extraction. `None` clears the assignment and is *also*
    /// manual: a deliberately-emptied assignee must not be re-guessed either.
    /// The value is free text, not a contact id — there is no contacts table
    /// until v1.3, and requiring one before a person can fix a wrong name
    /// would make the correction path harder than the mistake it fixes.
    async fn set_action_item_assignee(
        &self,
        id: &str,
        assignee_hint: Option<String>,
    ) -> Result<ActionItem, AppError>;
    /// Sets who owes the *answer* to an open question — never
    /// `raised_by_hint`, which records who asked and is not editable. Same
    /// free-text and same manual-stamping rules as the assignee above.
    async fn set_open_question_owner(
        &self,
        id: &str,
        owner_hint: Option<String>,
    ) -> Result<OpenQuestion, AppError>;
    /// Marks an open question answered by (or reopened from) a conversation.
    async fn set_open_question_resolved(
        &self,
        id: &str,
        resolved_conv_id: Option<&str>,
    ) -> Result<OpenQuestion, AppError>;
    /// A user-authored action item (debug-session patch — there was no way
    /// to add one outside the extraction pipeline). Always writes
    /// `added_manually = true`, so `replace_extraction_rows`'s
    /// `WHERE added_manually = 0` re-extraction delete never touches it.
    async fn insert_action_item(
        &self,
        conv_id: &str,
        row: NewActionItem,
    ) -> Result<ActionItem, AppError>;
    /// A manually-added action item with no source conversation — Home's "+"
    /// (`project_id: None`, fully unfiled) or a Project page's "+"
    /// (`project_id: Some`, scoped without a conversation). Returns
    /// `ActionItemWithSource` rather than `ActionItem`: the caller needs
    /// `project_id` back, and there is no `conv_id` to derive one through.
    async fn insert_standalone_action_item(
        &self,
        project_id: Option<&str>,
        text: &str,
        assignee_hint: Option<&str>,
    ) -> Result<ActionItemWithSource, AppError>;
    /// Reads for Conversation Detail (W12b) — no bulk-read method existed
    /// for any of these three tables before this wave (only the write path,
    /// `bulk_insert_extraction`/`replace_extraction_rows`, and the one
    /// row-level `set_action_item_done`). Ordered oldest-first, matching
    /// transcript/extraction order.
    async fn list_action_items(&self, conv_id: &str) -> Result<Vec<ActionItem>, AppError>;
    async fn list_decisions(&self, conv_id: &str) -> Result<Vec<Decision>, AppError>;
    async fn list_open_questions(&self, conv_id: &str) -> Result<Vec<OpenQuestion>, AppError>;

    // -- Cross-conversation reads (W16 / LLD-08 §3.6-§3.7 `mnemos-mcp-server`
    //    tools — one joined query each, distinct from the per-conversation
    //    methods above which back Conversation Detail) ----------------------
    async fn list_action_items_global(
        &self,
        filter: ActionItemFilter,
    ) -> Result<Vec<ActionItemWithSource>, AppError>;
    async fn count_action_items_global(&self, filter: ActionItemFilter) -> Result<u32, AppError>;
    /// Cross-conversation decisions, ordered oldest -> newest to match
    /// 05_PROJECT_MEMORY.md §2's chronological Decisions list (the two
    /// filters above order newest-first because their consumers are
    /// "what's outstanding" views; a decision log reads forwards).
    async fn list_decisions_global(
        &self,
        filter: DecisionFilter,
    ) -> Result<Vec<Decision>, AppError>;
    async fn count_decisions_global(&self, filter: DecisionFilter) -> Result<u32, AppError>;
    async fn list_open_questions_global(
        &self,
        filter: OpenQuestionFilter,
    ) -> Result<Vec<OpenQuestionWithSource>, AppError>;
    async fn count_open_questions_global(
        &self,
        filter: OpenQuestionFilter,
    ) -> Result<u32, AppError>;

    async fn project_activity_stats(&self) -> Result<Vec<ProjectActivityStat>, AppError>;
    /// FTS5 keyword search across conversation titles, decisions, action
    /// items, and open questions (W16 / LLD-08 §3.2, v1 keyword-only tier —
    /// no vector fusion; see the migration `500_fts5_search.sql`). Scores
    /// are raw per-table `bm25()` values, not normalized across tables —
    /// good enough to rank v1's single merged list, not a substitute for
    /// the RRF fusion the vector tier (W14) will own.
    async fn fts_search(
        &self,
        query: &str,
        project_id: Option<&str>,
        k: u32,
    ) -> Result<Vec<FtsHit>, AppError>;
    /// `None` when `project_memory.json` doesn't exist yet (no refresh has
    /// run for this project) — mirrors `read_transcript`/`read_summary`'s
    /// `read_json_or_none` convention.
    async fn read_project_memory(
        &self,
        project_id: &str,
    ) -> Result<Option<serde_json::Value>, AppError>;

    // -- Chat journal + projection --------------------------------------------
    async fn open_chat_session(&self, new: NewChatSession) -> Result<ChatSession, AppError>;
    /// One transaction: append to `chat_journal` AND upsert the
    /// `chat_sessions` projection row (SUPERSET §7 journal-then-projection).
    async fn append_chat_event(
        &self,
        session_id: &str,
        epoch: &str,
        seq: i64,
        event: serde_json::Value,
    ) -> Result<(), AppError>;
    async fn read_chat_history(
        &self,
        session_id: &str,
        before_seq: Option<i64>,
        limit: u32,
    ) -> Result<Vec<ChatEventRecord>, AppError>;
    /// Looks up the one persistent session for a `(runner_id, scope_type,
    /// scope_id)` tuple (06_CHAT.md's "Sessions — per (runner, scope)
    /// tuple"). `None` when no session has ever been opened for this scope
    /// yet — the caller (`commands::chat`, W13a) then calls
    /// `open_chat_session`. Added this wave — nothing needed a scope-keyed
    /// lookup before `chat.send_prompt` existed.
    async fn find_chat_session_by_scope(
        &self,
        runner_id: Option<&str>,
        scope_type: crate::db::models::ChatScopeType,
        scope_id: Option<&str>,
    ) -> Result<Option<ChatSession>, AppError>;
    /// Persists the runner CLI's own opaque `--session-id` onto the
    /// `chat_sessions` row once a runner has actually started (LLD-07 §5.1
    /// — lets a crashed session resume on the CLI's side via the same id on
    /// a fresh spawn). Added this wave for the same reason as
    /// `find_chat_session_by_scope`.
    async fn set_chat_session_runner_session_id(
        &self,
        id: &str,
        session_id: &str,
    ) -> Result<(), AppError>;
    /// "New chat" (design doc §2.5/§2.9 US-9): opens a fresh session for
    /// `new`'s `(runner, scope)`, superseding whichever session is
    /// currently active for it (if any) — one transaction, so there is
    /// never a moment with two active rows for the same scope, nor an old
    /// row left pointing at nothing. A scope with no prior session at all
    /// behaves exactly like `open_chat_session`.
    async fn start_new_chat_session(&self, new: NewChatSession) -> Result<ChatSession, AppError>;
    /// `chat_sessions.title` (W13-history wave — the column existed since
    /// `001_init.sql`, nothing had ever set it). An empty/whitespace-only
    /// title is rejected: the row already has a real way to say "no title"
    /// (`NULL`), so an empty string would just be a second, confusing
    /// spelling of the same thing.
    async fn update_chat_session_title(&self, id: &str, title: &str) -> Result<(), AppError>;
    /// Every session — active *and* superseded (a "New chat" keeps its
    /// predecessor around, findable here, not deleted) — newest-updated
    /// first. `before_updated_at` paginates the same shape as
    /// `read_chat_history`'s `before_seq`.
    async fn list_chat_sessions(
        &self,
        before_updated_at: Option<i64>,
        limit: u32,
    ) -> Result<Vec<ChatSession>, AppError>;

    // -- Pipeline state -------------------------------------------------------
    async fn set_pipeline_step(
        &self,
        conv_id: &str,
        step: PipelineStep,
        error: Option<String>,
    ) -> Result<(), AppError>;
    async fn get_incomplete_pipelines(&self) -> Result<Vec<String>, AppError>;
    /// `None` if `conv_id` has no `pipeline_state` row yet (recording never
    /// reached `stop_recording`'s `finalizing` write) — W12a's
    /// `conversation.retry_step` uses this to reject a retry against a
    /// conversation whose pipeline never started.
    async fn get_pipeline_step(&self, conv_id: &str) -> Result<Option<PipelineStep>, AppError>;
    /// The `pipeline_state.error` column `get_pipeline_step` doesn't surface
    /// — Conversation Detail's failure state (W12b) needs the message, not
    /// just the `Failed` variant.
    async fn get_pipeline_error(&self, conv_id: &str) -> Result<Option<String>, AppError>;

    // -- Settings --------------------------------------------------------------
    async fn get_setting(&self, key: &str) -> Result<Option<serde_json::Value>, AppError>;
    async fn set_setting(&self, key: &str, value: serde_json::Value) -> Result<(), AppError>;

    // -- Filesystem writes owned by Storage ------------------------------------
    async fn write_transcript(
        &self,
        conv_id: &str,
        json: &serde_json::Value,
    ) -> Result<(), AppError>;
    /// Appends one line to the live `transcript.jsonl` buffer (LLD-01 §6.2
    /// append semantics — the whole file can't be atomic-renamed while a
    /// recording is still growing it).
    async fn append_transcript_chunk(&self, conv_id: &str, line_json: &str)
        -> Result<(), AppError>;
    async fn write_extraction(
        &self,
        conv_id: &str,
        json: &serde_json::Value,
    ) -> Result<(), AppError>;
    async fn write_summary(&self, conv_id: &str, md: &str) -> Result<(), AppError>;
    async fn write_project_memory(
        &self,
        project_id: &str,
        json: &serde_json::Value,
    ) -> Result<(), AppError>;
    /// `None` when the file doesn't exist yet (pipeline hasn't reached
    /// `transcribing`/`extracting` for this conversation) — mirrors
    /// `memory::extract_conversation`'s own `read_json_or_none` convention.
    async fn read_transcript(&self, conv_id: &str) -> Result<Option<serde_json::Value>, AppError>;
    async fn read_summary(&self, conv_id: &str) -> Result<Option<String>, AppError>;

    // -- pending_deletes recovery entrypoint (called by main.rs) -----------
    async fn resume_pending_deletes(&self) -> Result<(), AppError>;
    async fn list_stuck_deletes(&self) -> Result<Vec<StuckDelete>, AppError>;

    // -- Backup (BACKEND_STANDARDS §5 — locked v1 behaviour) -----------------
    async fn snapshot_backup_now(&self) -> Result<PathBuf, AppError>;
}

/// The single `StorageService` implementation shipped in v1. Composes the
/// split pool, the atomic-write helper, and the `pending_deletes` machinery.
///
/// `Clone` (W13a addition — see `DbPools`'s doc comment): cheap, no new
/// connections opened.
#[derive(Clone)]
pub struct SqliteStorageService {
    pub pools: DbPools,
}

impl SqliteStorageService {
    pub fn new(pools: DbPools) -> Self {
        Self { pools }
    }

    /// Re-read of a single row, shared by the `set_*` methods so each of them
    /// returns the persisted row rather than the values it just sent. Not on
    /// the trait: nothing outside these writers needs to fetch one item by id.
    async fn get_action_item(&self, id: &str) -> Result<ActionItem, AppError> {
        sqlx::query_as::<_, ActionItem>(
            "SELECT id, conv_id, text, assignee_hint, assignee_source, due_hint, source_ts, \
             done, dismissed, added_manually, created_at, updated_at \
             FROM action_items WHERE id = ?1",
        )
        .bind(id)
        .fetch_one(&self.pools.read)
        .await
        .map_err(db_err)
    }

    async fn get_open_question(&self, id: &str) -> Result<OpenQuestion, AppError> {
        sqlx::query_as::<_, OpenQuestion>(
            "SELECT id, conv_id, question, raised_by_hint, owner_hint, owner_source, source_ts, \
             resolved_conv_id, resolved_at, added_manually, created_at \
             FROM open_questions WHERE id = ?1",
        )
        .bind(id)
        .fetch_one(&self.pools.read)
        .await
        .map_err(db_err)
    }
}

#[async_trait]
impl StorageService for SqliteStorageService {
    async fn list_projects(&self, filter: ProjectFilter) -> Result<Vec<Project>, AppError> {
        let mut sql = String::from(
            "SELECT id, name, description, pinned, archived, deleted_at, created_at, updated_at \
             FROM projects WHERE deleted_at IS NULL",
        );
        if !filter.include_archived {
            sql.push_str(" AND archived = 0");
        }
        sql.push_str(" ORDER BY pinned DESC, updated_at DESC, id DESC");
        if filter.limit.is_some() {
            sql.push_str(" LIMIT ? OFFSET ?");
        }
        let mut query = sqlx::query_as::<_, Project>(&sql);
        if let Some(limit) = filter.limit {
            query = query.bind(limit).bind(filter.offset);
        }
        query.fetch_all(&self.pools.read).await.map_err(db_err)
    }

    async fn count_projects(&self, include_archived: bool) -> Result<u32, AppError> {
        let sql = if include_archived {
            "SELECT COUNT(*) FROM projects WHERE deleted_at IS NULL"
        } else {
            "SELECT COUNT(*) FROM projects WHERE deleted_at IS NULL AND archived = 0"
        };
        let count: i64 = sqlx::query_scalar(sql)
            .fetch_one(&self.pools.read)
            .await
            .map_err(db_err)?;
        Ok(count as u32)
    }

    async fn get_project(&self, id: &str) -> Result<Project, AppError> {
        sqlx::query_as::<_, Project>(
            "SELECT id, name, description, pinned, archived, deleted_at, created_at, updated_at \
             FROM projects WHERE id = ?1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pools.read)
        .await
        .map_err(db_err)?
        .ok_or_else(|| AppError::NotFound {
            entity: "project".into(),
            id: id.to_string(),
        })
    }

    async fn create_project(&self, input: NewProject) -> Result<Project, AppError> {
        if input.name.trim().is_empty() {
            return Err(AppError::Validation {
                message: "project name must not be empty".into(),
                field: Some("name".into()),
            });
        }
        let id = crate::db::models::new_id();
        let now = unix_now();
        sqlx::query(
            "INSERT INTO projects (id, name, description, pinned, archived, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 0, 0, ?4, ?4)",
        )
        .bind(&id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(now)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?;
        self.get_project(&id).await
    }

    async fn update_project(&self, id: &str, patch: ProjectPatch) -> Result<Project, AppError> {
        let current = self.get_project(id).await?;
        let name = patch.name.unwrap_or(current.name);
        let description = patch.description.or(current.description);
        let pinned = patch.pinned.unwrap_or(current.pinned);
        let archived = patch.archived.unwrap_or(current.archived);
        let now = unix_now();
        sqlx::query(
            "UPDATE projects SET name = ?1, description = ?2, pinned = ?3, archived = ?4, \
             updated_at = ?5 WHERE id = ?6",
        )
        .bind(&name)
        .bind(&description)
        .bind(pinned)
        .bind(archived)
        .bind(now)
        .bind(id)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?;
        self.get_project(id).await
    }

    async fn delete_project(&self, id: &str) -> Result<(), AppError> {
        pending_deletes::enqueue_project_delete(&self.pools.write, id).await
    }

    async fn list_conversations(
        &self,
        filter: ConversationFilter,
    ) -> Result<Vec<Conversation>, AppError> {
        let mut sql = String::from(
            "SELECT id, project_id, title, started_at, ended_at, duration_s, status, runner_id, \
             starred, archived, notes, deleted_at, created_at, updated_at \
             FROM conversations",
        );
        sql.push_str(&conversation_where(&filter));
        sql.push_str(match filter.order {
            ConversationOrder::StartedDesc => " ORDER BY started_at DESC, id DESC",
            ConversationOrder::StartedAsc => " ORDER BY started_at ASC, id ASC",
        });
        // `id` is the tiebreaker on both orders for a real reason: `started_at`
        // has one-second resolution, so two conversations started in the same
        // second have an order SQLite is free to vary between calls. Under
        // `LIMIT`/`OFFSET` that is not cosmetic — an unstable sort can show
        // the same row on two consecutive pages and drop another entirely.
        if let Some(limit) = filter.limit {
            sql.push_str(" LIMIT ? OFFSET ?");
            let query = bind_conversation_filter!(sqlx::query_as::<_, Conversation>(&sql), filter)
                .bind(limit)
                .bind(filter.offset);
            return query.fetch_all(&self.pools.read).await.map_err(db_err);
        }
        bind_conversation_filter!(sqlx::query_as::<_, Conversation>(&sql), filter)
            .fetch_all(&self.pools.read)
            .await
            .map_err(db_err)
    }

    async fn count_conversations(&self, filter: ConversationFilter) -> Result<u32, AppError> {
        let sql = format!(
            "SELECT COUNT(*) FROM conversations{}",
            conversation_where(&filter)
        );
        let count: i64 = bind_conversation_filter!(sqlx::query_scalar(&sql), filter)
            .fetch_one(&self.pools.read)
            .await
            .map_err(db_err)?;
        Ok(count as u32)
    }

    async fn get_conversation(&self, id: &str) -> Result<Conversation, AppError> {
        sqlx::query_as::<_, Conversation>(
            "SELECT id, project_id, title, started_at, ended_at, duration_s, status, runner_id, \
             starred, archived, notes, deleted_at, created_at, updated_at \
             FROM conversations WHERE id = ?1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pools.read)
        .await
        .map_err(db_err)?
        .ok_or_else(|| AppError::NotFound {
            entity: "conversation".into(),
            id: id.to_string(),
        })
    }

    async fn insert_conversation(&self, row: NewConversation) -> Result<Conversation, AppError> {
        let id = crate::db::models::new_id();
        let now = unix_now();
        sqlx::query(
            "INSERT INTO conversations \
             (id, project_id, title, started_at, status, runner_id, starred, archived, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, 'recording', ?5, 0, 0, ?6, ?6)",
        )
        .bind(&id)
        .bind(&row.project_id)
        .bind(&row.title)
        .bind(row.started_at)
        .bind(&row.runner_id)
        .bind(now)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?;
        self.get_conversation(&id).await
    }

    async fn update_conversation_status(
        &self,
        id: &str,
        status: ConversationStatus,
        ended_at: Option<i64>,
        duration_s: Option<i64>,
    ) -> Result<(), AppError> {
        let now = unix_now();
        // `COALESCE(?, col)`, not a bare bind: `None` means "leave this
        // column alone", not "write NULL". Every failure-path caller passes
        // `(None, None)` — with a bare bind those calls silently erased the
        // conversation's `ended_at`/`duration_s`, so a recording that failed
        // in post-processing lost its end time and duration even though both
        // were known and already stored.
        let touched = sqlx::query(
            "UPDATE conversations SET status = ?1, \
             ended_at = COALESCE(?2, ended_at), \
             duration_s = COALESCE(?3, duration_s), \
             updated_at = ?4 \
             WHERE id = ?5 AND deleted_at IS NULL",
        )
        .bind(status)
        .bind(ended_at)
        .bind(duration_s)
        .bind(now)
        .bind(id)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?
        .rows_affected();
        if touched == 0 {
            return Err(AppError::NotFound {
                entity: "conversation".into(),
                id: id.to_string(),
            });
        }
        Ok(())
    }

    async fn delete_conversation(&self, id: &str) -> Result<(), AppError> {
        // Existence check only — the enqueued row no longer needs to
        // remember `project_id`, since `conversation_dir` is flat and
        // keyed by `id` alone (see `fs::paths::recordings_root`).
        self.get_conversation(id).await?;
        pending_deletes::enqueue_conversation_delete(&self.pools.write, id).await
    }

    async fn list_interrupted_recordings(&self) -> Result<Vec<Conversation>, AppError> {
        sqlx::query_as::<_, Conversation>(
            "SELECT id, project_id, title, started_at, ended_at, duration_s, status, runner_id, \
             starred, archived, notes, deleted_at, created_at, updated_at \
             FROM conversations WHERE status = 'recording' AND deleted_at IS NULL \
             ORDER BY started_at DESC",
        )
        .fetch_all(&self.pools.read)
        .await
        .map_err(db_err)
    }

    async fn list_stuck_processing(&self) -> Result<Vec<Conversation>, AppError> {
        sqlx::query_as::<_, Conversation>(
            "SELECT id, project_id, title, started_at, ended_at, duration_s, status, runner_id, \
             starred, archived, notes, deleted_at, created_at, updated_at \
             FROM conversations WHERE status = 'processing' AND deleted_at IS NULL \
             ORDER BY started_at DESC",
        )
        .fetch_all(&self.pools.read)
        .await
        .map_err(db_err)
    }

    async fn update_conversation_title(
        &self,
        id: &str,
        title: &str,
    ) -> Result<Conversation, AppError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::Validation {
                message: "title cannot be empty".into(),
                field: Some("title".into()),
            });
        }
        let now = unix_now();
        let touched = sqlx::query(
            "UPDATE conversations SET title = ?1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
        )
        .bind(title)
        .bind(now)
        .bind(id)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?
        .rows_affected();
        if touched == 0 {
            return Err(AppError::NotFound {
                entity: "conversation".into(),
                id: id.to_string(),
            });
        }
        self.get_conversation(id).await
    }

    async fn update_conversation_notes(
        &self,
        id: &str,
        notes: &str,
    ) -> Result<Conversation, AppError> {
        let now = unix_now();
        let touched = sqlx::query(
            "UPDATE conversations SET notes = ?1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
        )
        .bind(notes)
        .bind(now)
        .bind(id)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?
        .rows_affected();
        if touched == 0 {
            return Err(AppError::NotFound {
                entity: "conversation".into(),
                id: id.to_string(),
            });
        }
        self.get_conversation(id).await
    }

    async fn update_conversation_project(
        &self,
        id: &str,
        project_id: Option<&str>,
    ) -> Result<Conversation, AppError> {
        let now = unix_now();
        let touched = sqlx::query(
            "UPDATE conversations SET project_id = ?1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
        )
        .bind(project_id)
        .bind(now)
        .bind(id)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?
        .rows_affected();
        if touched == 0 {
            return Err(AppError::NotFound {
                entity: "conversation".into(),
                id: id.to_string(),
            });
        }
        self.get_conversation(id).await
    }

    async fn bulk_insert_extraction(
        &self,
        conv_id: &str,
        items: ExtractionBundle,
    ) -> Result<(), AppError> {
        let conv_id = conv_id.to_string();
        with_write_tx(&self.pools.write, move |tx| {
            Box::pin(async move {
                let now = unix_now();
                for item in &items.action_items {
                    sqlx::query(
                        "INSERT INTO action_items \
                         (id, conv_id, text, assignee_hint, due_hint, source_ts, done, dismissed, \
                          added_manually, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0, 0, ?7, ?7)",
                    )
                    .bind(crate::db::models::new_id())
                    .bind(&conv_id)
                    .bind(&item.text)
                    .bind(&item.assignee_hint)
                    .bind(&item.due_hint)
                    .bind(item.source_ts)
                    .bind(now)
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;
                }
                for item in &items.decisions {
                    sqlx::query(
                        "INSERT INTO decisions \
                         (id, conv_id, statement, quote, decided_by_hint, source_ts, added_manually, created_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
                    )
                    .bind(crate::db::models::new_id())
                    .bind(&conv_id)
                    .bind(&item.statement)
                    .bind(&item.quote)
                    .bind(&item.decided_by_hint)
                    .bind(item.source_ts)
                    .bind(now)
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;
                }
                for item in &items.open_questions {
                    sqlx::query(
                        "INSERT INTO open_questions \
                         (id, conv_id, question, raised_by_hint, source_ts, added_manually, created_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
                    )
                    .bind(crate::db::models::new_id())
                    .bind(&conv_id)
                    .bind(&item.question)
                    .bind(&item.raised_by_hint)
                    .bind(item.source_ts)
                    .bind(now)
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;
                }
                Ok(())
            })
        })
        .await
    }

    async fn replace_extraction_rows(
        &self,
        conv_id: &str,
        items: ExtractionBundle,
    ) -> Result<(), AppError> {
        let conv_id = conv_id.to_string();
        with_write_tx(&self.pools.write, move |tx| {
            Box::pin(async move {
                let now = unix_now();

                // Manual assignments must survive re-extraction. The rows
                // themselves cannot: the model re-derives the whole set, so
                // keeping the old rows would duplicate every item. Instead we
                // carry the corrections across on the item's own text, which
                // is the only key stable between two extractions of the same
                // conversation. Known limit, and the honest one to accept: if
                // the model rewords an item, its manual assignee does not
                // follow. Losing a correction on a reworded line is better
                // than dropping every correction on every regeneration, which
                // is what happened before this.
                let kept_assignees: Vec<(String, Option<String>)> = sqlx::query_as(
                    "SELECT text, assignee_hint FROM action_items \
                     WHERE conv_id = ?1 AND added_manually = 0 AND assignee_source = 'manual'",
                )
                .bind(&conv_id)
                .fetch_all(&mut **tx)
                .await
                .map_err(db_err)?;
                let kept_owners: Vec<(String, Option<String>)> = sqlx::query_as(
                    "SELECT question, owner_hint FROM open_questions \
                     WHERE conv_id = ?1 AND added_manually = 0 AND owner_source = 'manual'",
                )
                .bind(&conv_id)
                .fetch_all(&mut **tx)
                .await
                .map_err(db_err)?;

                sqlx::query("DELETE FROM action_items WHERE conv_id = ?1 AND added_manually = 0")
                    .bind(&conv_id)
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;
                sqlx::query("DELETE FROM decisions WHERE conv_id = ?1 AND added_manually = 0")
                    .bind(&conv_id)
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;
                sqlx::query(
                    "DELETE FROM open_questions WHERE conv_id = ?1 AND added_manually = 0",
                )
                .bind(&conv_id)
                .execute(&mut **tx)
                .await
                .map_err(db_err)?;

                for item in &items.action_items {
                    sqlx::query(
                        "INSERT INTO action_items \
                         (id, conv_id, text, assignee_hint, due_hint, source_ts, done, dismissed, \
                          added_manually, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0, 0, ?7, ?7)",
                    )
                    .bind(crate::db::models::new_id())
                    .bind(&conv_id)
                    .bind(&item.text)
                    .bind(&item.assignee_hint)
                    .bind(&item.due_hint)
                    .bind(item.source_ts)
                    .bind(now)
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;
                }
                for item in &items.decisions {
                    sqlx::query(
                        "INSERT INTO decisions \
                         (id, conv_id, statement, quote, decided_by_hint, source_ts, added_manually, created_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
                    )
                    .bind(crate::db::models::new_id())
                    .bind(&conv_id)
                    .bind(&item.statement)
                    .bind(&item.quote)
                    .bind(&item.decided_by_hint)
                    .bind(item.source_ts)
                    .bind(now)
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;
                }
                for item in &items.open_questions {
                    sqlx::query(
                        "INSERT INTO open_questions \
                         (id, conv_id, question, raised_by_hint, source_ts, added_manually, created_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
                    )
                    .bind(crate::db::models::new_id())
                    .bind(&conv_id)
                    .bind(&item.question)
                    .bind(&item.raised_by_hint)
                    .bind(item.source_ts)
                    .bind(now)
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;
                }
                // Re-apply the corrections captured above. `assignee_source`
                // is set back to 'manual' so the next regeneration carries
                // them again.
                for (text, assignee_hint) in &kept_assignees {
                    sqlx::query(
                        "UPDATE action_items SET assignee_hint = ?1, assignee_source = 'manual' \
                         WHERE conv_id = ?2 AND text = ?3",
                    )
                    .bind(assignee_hint)
                    .bind(&conv_id)
                    .bind(text)
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;
                }
                for (question, owner_hint) in &kept_owners {
                    sqlx::query(
                        "UPDATE open_questions SET owner_hint = ?1, owner_source = 'manual' \
                         WHERE conv_id = ?2 AND question = ?3",
                    )
                    .bind(owner_hint)
                    .bind(&conv_id)
                    .bind(question)
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;
                }

                // Agent-suggested bookmarks: insert only rows not already
                // present (LLD-05 §4.4 — user-tapped bookmarks are never
                // bulk-deleted, so this is an upsert-by-absence, not a
                // delete-then-insert like the three tables above).
                for item in &items.bookmarks {
                    let exists: Option<(i64,)> = sqlx::query_as(
                        "SELECT 1 FROM bookmarks WHERE conv_id = ?1 AND ts_ms = ?2 AND label IS ?3",
                    )
                    .bind(&conv_id)
                    .bind(item.ts_ms)
                    .bind(&item.label)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(db_err)?;
                    if exists.is_some() {
                        continue;
                    }
                    sqlx::query(
                        "INSERT INTO bookmarks (id, conv_id, ts_ms, label, created_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                    )
                    .bind(crate::db::models::new_id())
                    .bind(&conv_id)
                    .bind(item.ts_ms)
                    .bind(&item.label)
                    .bind(now)
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;
                }
                Ok(())
            })
        })
        .await
    }

    async fn set_action_item_done(&self, id: &str, done: bool) -> Result<ActionItem, AppError> {
        let now = unix_now();
        let touched =
            sqlx::query("UPDATE action_items SET done = ?1, updated_at = ?2 WHERE id = ?3")
                .bind(done)
                .bind(now)
                .bind(id)
                .execute(&self.pools.write)
                .await
                .map_err(db_err)?
                .rows_affected();
        if touched == 0 {
            return Err(AppError::NotFound {
                entity: "action_item".into(),
                id: id.to_string(),
            });
        }
        sqlx::query_as::<_, ActionItem>(
            "SELECT id, conv_id, text, assignee_hint, assignee_source, due_hint, source_ts, done, \
             dismissed, \
             added_manually, created_at, updated_at FROM action_items WHERE id = ?1",
        )
        .bind(id)
        .fetch_one(&self.pools.read)
        .await
        .map_err(db_err)
    }

    async fn set_action_item_assignee(
        &self,
        id: &str,
        assignee_hint: Option<String>,
    ) -> Result<ActionItem, AppError> {
        let now = unix_now();
        let touched = sqlx::query(
            "UPDATE action_items SET assignee_hint = ?1, assignee_source = 'manual', \
             updated_at = ?2 WHERE id = ?3",
        )
        .bind(&assignee_hint)
        .bind(now)
        .bind(id)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?
        .rows_affected();
        if touched == 0 {
            return Err(AppError::NotFound {
                entity: "action_item".into(),
                id: id.to_string(),
            });
        }
        self.get_action_item(id).await
    }

    async fn set_open_question_owner(
        &self,
        id: &str,
        owner_hint: Option<String>,
    ) -> Result<OpenQuestion, AppError> {
        let touched = sqlx::query(
            "UPDATE open_questions SET owner_hint = ?1, owner_source = 'manual' WHERE id = ?2",
        )
        .bind(&owner_hint)
        .bind(id)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?
        .rows_affected();
        if touched == 0 {
            return Err(AppError::NotFound {
                entity: "open_question".into(),
                id: id.to_string(),
            });
        }
        self.get_open_question(id).await
    }

    async fn set_open_question_resolved(
        &self,
        id: &str,
        resolved_conv_id: Option<&str>,
    ) -> Result<OpenQuestion, AppError> {
        // `resolved_at` and `resolved_conv_id` move together — a question with
        // one set and not the other reads as resolved to one query and open to
        // another, and `open_question_where` filters on `resolved_conv_id`.
        let resolved_at = resolved_conv_id.map(|_| unix_now());
        let touched = sqlx::query(
            "UPDATE open_questions SET resolved_conv_id = ?1, resolved_at = ?2 WHERE id = ?3",
        )
        .bind(resolved_conv_id)
        .bind(resolved_at)
        .bind(id)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?
        .rows_affected();
        if touched == 0 {
            return Err(AppError::NotFound {
                entity: "open_question".into(),
                id: id.to_string(),
            });
        }
        self.get_open_question(id).await
    }

    async fn insert_action_item(
        &self,
        conv_id: &str,
        row: NewActionItem,
    ) -> Result<ActionItem, AppError> {
        if row.text.trim().is_empty() {
            return Err(AppError::Validation {
                message: "text cannot be empty".into(),
                field: Some("text".into()),
            });
        }
        let id = crate::db::models::new_id();
        let now = unix_now();
        sqlx::query(
            "INSERT INTO action_items \
             (id, conv_id, text, assignee_hint, due_hint, source_ts, done, dismissed, \
              added_manually, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0, 1, ?7, ?7)",
        )
        .bind(&id)
        .bind(conv_id)
        .bind(&row.text)
        .bind(&row.assignee_hint)
        .bind(&row.due_hint)
        .bind(row.source_ts)
        .bind(now)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?;
        sqlx::query_as::<_, ActionItem>(
            "SELECT id, conv_id, text, assignee_hint, assignee_source, due_hint, source_ts, done, \
             dismissed, \
             added_manually, created_at, updated_at FROM action_items WHERE id = ?1",
        )
        .bind(&id)
        .fetch_one(&self.pools.read)
        .await
        .map_err(db_err)
    }

    async fn insert_standalone_action_item(
        &self,
        project_id: Option<&str>,
        text: &str,
        assignee_hint: Option<&str>,
    ) -> Result<ActionItemWithSource, AppError> {
        if text.trim().is_empty() {
            return Err(AppError::Validation {
                message: "text cannot be empty".into(),
                field: Some("text".into()),
            });
        }
        let id = crate::db::models::new_id();
        let now = unix_now();
        // `assignee_hint` is set in the same INSERT, not a follow-up
        // `UPDATE` — Home's "+" needs to self-assign ("You") for the item to
        // appear in a list filtered to `assigned_to_me`, and a two-call
        // version left a window where the create succeeded, the assign
        // failed, and the row became a permanently invisible orphan (no
        // conv_id, no project_id, no assignee — nothing lists it). One
        // INSERT means there is no such window: it either has the assignee
        // from the start or the whole write failed and nothing was created.
        let assignee_source = if assignee_hint.is_some() {
            HintSource::Manual
        } else {
            HintSource::Model
        };
        sqlx::query(
            "INSERT INTO action_items \
             (id, conv_id, project_id, text, assignee_hint, assignee_source, done, dismissed, \
              added_manually, created_at, updated_at) \
             VALUES (?1, NULL, ?2, ?3, ?4, ?5, 0, 0, 1, ?6, ?6)",
        )
        .bind(&id)
        .bind(project_id)
        .bind(text.trim())
        .bind(assignee_hint)
        .bind(assignee_source)
        .bind(now)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?;
        Ok(ActionItemWithSource {
            id,
            conv_id: None,
            project_id: project_id.map(str::to_string),
            text: text.trim().to_string(),
            assignee_hint: assignee_hint.map(str::to_string),
            assignee_source,
            due_hint: None,
            source_ts: None,
            done: false,
            dismissed: false,
            created_at: now,
        })
    }

    async fn list_action_items(&self, conv_id: &str) -> Result<Vec<ActionItem>, AppError> {
        sqlx::query_as::<_, ActionItem>(
            "SELECT id, conv_id, text, assignee_hint, assignee_source, due_hint, source_ts, done, \
             dismissed, \
             added_manually, created_at, updated_at FROM action_items \
             WHERE conv_id = ?1 AND dismissed = 0 ORDER BY created_at ASC",
        )
        .bind(conv_id)
        .fetch_all(&self.pools.read)
        .await
        .map_err(db_err)
    }

    async fn list_decisions(&self, conv_id: &str) -> Result<Vec<Decision>, AppError> {
        sqlx::query_as::<_, Decision>(
            "SELECT id, conv_id, statement, quote, decided_by_hint, source_ts, added_manually, \
             created_at FROM decisions WHERE conv_id = ?1 ORDER BY created_at ASC",
        )
        .bind(conv_id)
        .fetch_all(&self.pools.read)
        .await
        .map_err(db_err)
    }

    async fn list_open_questions(&self, conv_id: &str) -> Result<Vec<OpenQuestion>, AppError> {
        sqlx::query_as::<_, OpenQuestion>(
            "SELECT id, conv_id, question, raised_by_hint, owner_hint, owner_source, source_ts, \
             resolved_conv_id, \
             resolved_at, added_manually, created_at FROM open_questions \
             WHERE conv_id = ?1 ORDER BY created_at ASC",
        )
        .bind(conv_id)
        .fetch_all(&self.pools.read)
        .await
        .map_err(db_err)
    }

    async fn list_action_items_global(
        &self,
        filter: ActionItemFilter,
    ) -> Result<Vec<ActionItemWithSource>, AppError> {
        let sql = format!(
            "SELECT ai.id, ai.conv_id, COALESCE(ai.project_id, c.project_id) AS project_id, \
             ai.text, ai.assignee_hint, ai.assignee_source, ai.due_hint, ai.source_ts, ai.done, \
             ai.dismissed, ai.created_at \
             FROM action_items ai LEFT JOIN conversations c ON c.id = ai.conv_id{} \
             ORDER BY COALESCE(ai.source_ts, ai.created_at) DESC, ai.id DESC LIMIT ? OFFSET ?",
            action_item_where(&filter)
        );
        bind_action_item_filter!(sqlx::query_as::<_, ActionItemWithSource>(&sql), filter)
            .bind(filter.limit)
            .bind(filter.offset)
            .fetch_all(&self.pools.read)
            .await
            .map_err(db_err)
    }

    async fn count_action_items_global(&self, filter: ActionItemFilter) -> Result<u32, AppError> {
        let sql = format!(
            "SELECT COUNT(*) FROM action_items ai LEFT JOIN conversations c ON c.id = ai.conv_id{}",
            action_item_where(&filter)
        );
        let count: i64 = bind_action_item_filter!(sqlx::query_scalar(&sql), filter)
            .fetch_one(&self.pools.read)
            .await
            .map_err(db_err)?;
        Ok(count as u32)
    }

    async fn list_decisions_global(
        &self,
        filter: DecisionFilter,
    ) -> Result<Vec<Decision>, AppError> {
        let sql = format!(
            "SELECT d.id, d.conv_id, COALESCE(d.project_id, c.project_id) AS project_id, \
             d.statement, d.quote, d.decided_by_hint, d.source_ts, d.added_manually, d.created_at \
             FROM decisions d LEFT JOIN conversations c ON c.id = d.conv_id{} \
             ORDER BY COALESCE(d.source_ts, d.created_at) ASC, d.id ASC LIMIT ? OFFSET ?",
            decision_where(&filter)
        );
        bind_decision_filter!(sqlx::query_as::<_, Decision>(&sql), filter)
            .bind(filter.limit)
            .bind(filter.offset)
            .fetch_all(&self.pools.read)
            .await
            .map_err(db_err)
    }

    async fn count_decisions_global(&self, filter: DecisionFilter) -> Result<u32, AppError> {
        let sql = format!(
            "SELECT COUNT(*) FROM decisions d LEFT JOIN conversations c ON c.id = d.conv_id{}",
            decision_where(&filter)
        );
        let count: i64 = bind_decision_filter!(sqlx::query_scalar(&sql), filter)
            .fetch_one(&self.pools.read)
            .await
            .map_err(db_err)?;
        Ok(count as u32)
    }

    async fn list_open_questions_global(
        &self,
        filter: OpenQuestionFilter,
    ) -> Result<Vec<OpenQuestionWithSource>, AppError> {
        let sql = format!(
            "SELECT oq.id, oq.conv_id, COALESCE(oq.project_id, c.project_id) AS project_id, \
             oq.question, oq.raised_by_hint, oq.owner_hint, oq.owner_source, oq.source_ts, \
             oq.resolved_conv_id, oq.resolved_at, oq.created_at \
             FROM open_questions oq LEFT JOIN conversations c ON c.id = oq.conv_id{} \
             ORDER BY COALESCE(oq.source_ts, oq.created_at) DESC, oq.id DESC LIMIT ? OFFSET ?",
            open_question_where(&filter)
        );
        bind_open_question_filter!(sqlx::query_as::<_, OpenQuestionWithSource>(&sql), filter)
            .bind(filter.limit)
            .bind(filter.offset)
            .fetch_all(&self.pools.read)
            .await
            .map_err(db_err)
    }

    async fn count_open_questions_global(
        &self,
        filter: OpenQuestionFilter,
    ) -> Result<u32, AppError> {
        let sql = format!(
            "SELECT COUNT(*) FROM open_questions oq LEFT JOIN conversations c ON c.id = oq.conv_id{}",
            open_question_where(&filter)
        );
        let count: i64 = bind_open_question_filter!(sqlx::query_scalar(&sql), filter)
            .fetch_one(&self.pools.read)
            .await
            .map_err(db_err)?;
        Ok(count as u32)
    }

    async fn project_activity_stats(&self) -> Result<Vec<ProjectActivityStat>, AppError> {
        sqlx::query_as::<_, ProjectActivityStat>(
            "SELECT project_id, COUNT(*) AS conversation_count, MAX(started_at) AS last_activity_at \
             FROM conversations WHERE deleted_at IS NULL AND project_id IS NOT NULL GROUP BY project_id",
        )
        .fetch_all(&self.pools.read)
        .await
        .map_err(db_err)
    }

    async fn fts_search(
        &self,
        query: &str,
        project_id: Option<&str>,
        k: u32,
    ) -> Result<Vec<FtsHit>, AppError> {
        let mut hits = Vec::new();

        let rows = sqlx::query(
            "SELECT c.id AS conv_id, c.project_id, c.started_at, bm25(conversations_fts) AS score, \
             snippet(conversations_fts, -1, '', '', '…', 10) AS snippet \
             FROM conversations_fts JOIN conversations c ON c.rowid = conversations_fts.rowid \
             WHERE conversations_fts MATCH ?1 AND c.deleted_at IS NULL \
             AND (?2 IS NULL OR c.project_id = ?2) ORDER BY score LIMIT ?3",
        )
        .bind(query)
        .bind(project_id)
        .bind(k as i64)
        .fetch_all(&self.pools.read)
        .await
        .map_err(db_err)?;
        for row in rows {
            hits.push(FtsHit {
                conversation_id: row.try_get("conv_id").map_err(db_err)?,
                project_id: row.try_get("project_id").map_err(db_err)?,
                kind: FtsHitKind::ConversationTitle,
                snippet: row.try_get("snippet").map_err(db_err)?,
                score: row.try_get("score").map_err(db_err)?,
                source_ts: row.try_get("started_at").map_err(db_err)?,
            });
        }

        let rows = sqlx::query(
            "SELECT d.conv_id, COALESCE(d.project_id, c.project_id) AS project_id, d.source_ts, \
             bm25(decisions_fts) AS score, \
             snippet(decisions_fts, -1, '', '', '…', 10) AS snippet \
             FROM decisions_fts JOIN decisions d ON d.rowid = decisions_fts.rowid \
             LEFT JOIN conversations c ON c.id = d.conv_id \
             WHERE decisions_fts MATCH ?1 AND (d.conv_id IS NULL OR c.deleted_at IS NULL) \
             AND (?2 IS NULL OR COALESCE(d.project_id, c.project_id) = ?2) ORDER BY score LIMIT ?3",
        )
        .bind(query)
        .bind(project_id)
        .bind(k as i64)
        .fetch_all(&self.pools.read)
        .await
        .map_err(db_err)?;
        for row in rows {
            hits.push(FtsHit {
                conversation_id: row.try_get("conv_id").map_err(db_err)?,
                project_id: row.try_get("project_id").map_err(db_err)?,
                kind: FtsHitKind::Decision,
                snippet: row.try_get("snippet").map_err(db_err)?,
                score: row.try_get("score").map_err(db_err)?,
                source_ts: row.try_get("source_ts").map_err(db_err)?,
            });
        }

        let rows = sqlx::query(
            "SELECT ai.conv_id, COALESCE(ai.project_id, c.project_id) AS project_id, ai.source_ts, \
             bm25(action_items_fts) AS score, \
             snippet(action_items_fts, -1, '', '', '…', 10) AS snippet \
             FROM action_items_fts JOIN action_items ai ON ai.rowid = action_items_fts.rowid \
             LEFT JOIN conversations c ON c.id = ai.conv_id \
             WHERE action_items_fts MATCH ?1 AND (ai.conv_id IS NULL OR c.deleted_at IS NULL) \
             AND (?2 IS NULL OR COALESCE(ai.project_id, c.project_id) = ?2) ORDER BY score LIMIT ?3",
        )
        .bind(query)
        .bind(project_id)
        .bind(k as i64)
        .fetch_all(&self.pools.read)
        .await
        .map_err(db_err)?;
        for row in rows {
            hits.push(FtsHit {
                conversation_id: row.try_get("conv_id").map_err(db_err)?,
                project_id: row.try_get("project_id").map_err(db_err)?,
                kind: FtsHitKind::ActionItem,
                snippet: row.try_get("snippet").map_err(db_err)?,
                score: row.try_get("score").map_err(db_err)?,
                source_ts: row.try_get("source_ts").map_err(db_err)?,
            });
        }

        let rows = sqlx::query(
            "SELECT oq.conv_id, COALESCE(oq.project_id, c.project_id) AS project_id, oq.source_ts, \
             bm25(open_questions_fts) AS score, \
             snippet(open_questions_fts, -1, '', '', '…', 10) AS snippet \
             FROM open_questions_fts JOIN open_questions oq ON oq.rowid = open_questions_fts.rowid \
             LEFT JOIN conversations c ON c.id = oq.conv_id \
             WHERE open_questions_fts MATCH ?1 AND (oq.conv_id IS NULL OR c.deleted_at IS NULL) \
             AND (?2 IS NULL OR COALESCE(oq.project_id, c.project_id) = ?2) ORDER BY score LIMIT ?3",
        )
        .bind(query)
        .bind(project_id)
        .bind(k as i64)
        .fetch_all(&self.pools.read)
        .await
        .map_err(db_err)?;
        for row in rows {
            hits.push(FtsHit {
                conversation_id: row.try_get("conv_id").map_err(db_err)?,
                project_id: row.try_get("project_id").map_err(db_err)?,
                kind: FtsHitKind::OpenQuestion,
                snippet: row.try_get("snippet").map_err(db_err)?,
                score: row.try_get("score").map_err(db_err)?,
                source_ts: row.try_get("source_ts").map_err(db_err)?,
            });
        }

        // bm25() is more-negative-is-better; merge the four per-table lists
        // by that raw score (§ doc comment on the trait method — not a
        // cross-table-normalized rank) and cap to k.
        hits.sort_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(k as usize);
        Ok(hits)
    }

    async fn read_project_memory(
        &self,
        project_id: &str,
    ) -> Result<Option<serde_json::Value>, AppError> {
        let path = paths::project_memory_path(project_id)?;
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)?;
        let json = serde_json::from_slice(&bytes)
            .map_err(|e| AppError::storage(format!("parse {}: {e}", path.display())))?;
        Ok(Some(json))
    }

    async fn open_chat_session(&self, new: NewChatSession) -> Result<ChatSession, AppError> {
        validate_chat_scope(&new)?;
        let id = crate::db::models::new_id();
        let epoch = crate::db::models::new_id();
        let now = unix_now();
        sqlx::query(
            "INSERT INTO chat_sessions \
             (id, runner_id, scope_type, scope_id, epoch, status, title, message_count, \
              total_input_tokens, total_output_tokens, cost_micros, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'idle', ?6, 0, 0, 0, 0, ?7, ?7)",
        )
        .bind(&id)
        .bind(&new.runner_id)
        .bind(new.scope_type)
        .bind(&new.scope_id)
        .bind(&epoch)
        .bind(&new.title)
        .bind(now)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?;
        sqlx::query_as::<_, ChatSession>(
            "SELECT id, runner_id, scope_type, scope_id, session_id, epoch, status, title, \
             superseded_by_id, message_count, total_input_tokens, total_output_tokens, \
             cost_micros, created_at, updated_at \
             FROM chat_sessions WHERE id = ?1",
        )
        .bind(&id)
        .fetch_one(&self.pools.read)
        .await
        .map_err(db_err)
    }

    async fn append_chat_event(
        &self,
        session_id: &str,
        epoch: &str,
        seq: i64,
        event: serde_json::Value,
    ) -> Result<(), AppError> {
        let session_id = session_id.to_string();
        let epoch = epoch.to_string();
        with_write_tx(&self.pools.write, move |tx| {
            Box::pin(async move {
                let now = unix_now();
                let event_json = event.to_string();
                sqlx::query(
                    "INSERT INTO chat_journal (session_id, epoch, seq, ts, event_json) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )
                .bind(&session_id)
                .bind(&epoch)
                .bind(seq)
                .bind(now)
                .bind(&event_json)
                .execute(&mut **tx)
                .await
                .map_err(db_err)?;
                let touched = sqlx::query(
                    "UPDATE chat_sessions SET message_count = message_count + 1, updated_at = ?1 \
                     WHERE id = ?2",
                )
                .bind(now)
                .bind(&session_id)
                .execute(&mut **tx)
                .await
                .map_err(db_err)?
                .rows_affected();
                if touched == 0 {
                    return Err(AppError::NotFound {
                        entity: "chat_session".into(),
                        id: session_id,
                    });
                }
                Ok(())
            })
        })
        .await
    }

    async fn read_chat_history(
        &self,
        session_id: &str,
        before_seq: Option<i64>,
        limit: u32,
    ) -> Result<Vec<ChatEventRecord>, AppError> {
        let rows: Vec<(String, String, i64, i64, String)> = if let Some(before) = before_seq {
            sqlx::query_as(
                "SELECT session_id, epoch, seq, ts, event_json FROM chat_journal \
                 WHERE session_id = ?1 AND seq < ?2 ORDER BY seq DESC LIMIT ?3",
            )
            .bind(session_id)
            .bind(before)
            .bind(limit as i64)
            .fetch_all(&self.pools.read)
            .await
            .map_err(db_err)?
        } else {
            sqlx::query_as(
                "SELECT session_id, epoch, seq, ts, event_json FROM chat_journal \
                 WHERE session_id = ?1 ORDER BY seq DESC LIMIT ?2",
            )
            .bind(session_id)
            .bind(limit as i64)
            .fetch_all(&self.pools.read)
            .await
            .map_err(db_err)?
        };
        let mut out: Vec<ChatEventRecord> = rows
            .into_iter()
            .map(|(session_id, epoch, seq, ts, event_json)| ChatEventRecord {
                session_id,
                epoch,
                seq,
                ts,
                event_json: serde_json::from_str(&event_json).unwrap_or(serde_json::Value::Null),
            })
            .collect();
        out.reverse(); // ascending seq, matching journal order
        Ok(out)
    }

    async fn find_chat_session_by_scope(
        &self,
        runner_id: Option<&str>,
        scope_type: crate::db::models::ChatScopeType,
        scope_id: Option<&str>,
    ) -> Result<Option<ChatSession>, AppError> {
        // `IS` (not `=`) so a NULL `runner_id`/`scope_id` (Everything scope
        // has no `scope_id`; v1 only ever sets `runner_id = Some("claude")`
        // but the column is nullable) matches NULL correctly — `= NULL` is
        // never true in SQLite.
        // `superseded_by_id IS NULL`: a "New chat" (`start_new_session`)
        // keeps the old row around (renameable, listable) instead of
        // overwriting it, so this lookup — "the one *active* session for
        // this scope" — must exclude superseded rows explicitly rather than
        // relying on `ORDER BY updated_at DESC` alone (design doc §2.5's
        // review-caught gap: a superseded row's `updated_at` isn't
        // guaranteed older at read time).
        sqlx::query_as::<_, ChatSession>(
            "SELECT id, runner_id, scope_type, scope_id, session_id, epoch, status, title, \
             superseded_by_id, message_count, total_input_tokens, total_output_tokens, \
             cost_micros, created_at, updated_at \
             FROM chat_sessions \
             WHERE runner_id IS ?1 AND scope_type = ?2 AND scope_id IS ?3 \
               AND superseded_by_id IS NULL \
             ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(runner_id)
        .bind(scope_type)
        .bind(scope_id)
        .fetch_optional(&self.pools.read)
        .await
        .map_err(db_err)
    }

    async fn set_chat_session_runner_session_id(
        &self,
        id: &str,
        session_id: &str,
    ) -> Result<(), AppError> {
        let touched =
            sqlx::query("UPDATE chat_sessions SET session_id = ?1, updated_at = ?2 WHERE id = ?3")
                .bind(session_id)
                .bind(unix_now())
                .bind(id)
                .execute(&self.pools.write)
                .await
                .map_err(db_err)?
                .rows_affected();
        if touched == 0 {
            return Err(AppError::NotFound {
                entity: "chat_session".into(),
                id: id.to_string(),
            });
        }
        Ok(())
    }

    async fn start_new_chat_session(&self, new: NewChatSession) -> Result<ChatSession, AppError> {
        validate_chat_scope(&new)?;
        let id = crate::db::models::new_id();
        let epoch = crate::db::models::new_id();

        with_write_tx(&self.pools.write, move |tx| {
            Box::pin(async move {
                let now = unix_now();

                // Deferred within this transaction only (SQLite
                // auto-resets it at commit/rollback) — the old row must be
                // superseded *before* the new row is inserted (see below),
                // which means its `superseded_by_id` briefly references an
                // id that doesn't exist in `chat_sessions` yet. Unlike
                // `PRAGMA foreign_keys`, `defer_foreign_keys` is documented
                // as safe to toggle mid-transaction.
                sqlx::query("PRAGMA defer_foreign_keys = ON")
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;

                // The one active row for this scope, if any — same
                // predicate as `find_chat_session_by_scope`, run inside
                // this transaction so it can't race a concurrent
                // `start_new_chat_session`/`send_prompt` for the same
                // scope (the UNIQUE index is the final backstop; this read
                // is what lets a normal, non-racing call supersede the
                // *correct* row instead of just relying on that backstop
                // to reject a bad insert after the fact).
                let previous_active_id: Option<String> = sqlx::query_scalar(
                    "SELECT id FROM chat_sessions \
                     WHERE runner_id IS ?1 AND scope_type = ?2 AND scope_id IS ?3 \
                       AND superseded_by_id IS NULL",
                )
                .bind(&new.runner_id)
                .bind(new.scope_type)
                .bind(&new.scope_id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(db_err)?;

                // Supersede the old row *before* inserting the new one:
                // both the old row (until this UPDATE lands) and a
                // just-inserted new row satisfy "active" (`superseded_by_id
                // IS NULL`), and the UNIQUE index enforces at most one
                // active row per scope at every statement boundary, not
                // just at commit — inserting first would violate it the
                // instant the INSERT ran, with the old row still active.
                if let Some(previous_id) = &previous_active_id {
                    sqlx::query(
                        "UPDATE chat_sessions SET superseded_by_id = ?1, updated_at = ?2 \
                         WHERE id = ?3",
                    )
                    .bind(&id)
                    .bind(now)
                    .bind(previous_id)
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;
                }

                sqlx::query(
                    "INSERT INTO chat_sessions \
                     (id, runner_id, scope_type, scope_id, epoch, status, title, message_count, \
                      total_input_tokens, total_output_tokens, cost_micros, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, 'idle', ?6, 0, 0, 0, 0, ?7, ?7)",
                )
                .bind(&id)
                .bind(&new.runner_id)
                .bind(new.scope_type)
                .bind(&new.scope_id)
                .bind(&epoch)
                .bind(&new.title)
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(db_err)?;

                sqlx::query_as::<_, ChatSession>(
                    "SELECT id, runner_id, scope_type, scope_id, session_id, epoch, status, \
                     title, superseded_by_id, message_count, total_input_tokens, \
                     total_output_tokens, cost_micros, created_at, updated_at \
                     FROM chat_sessions WHERE id = ?1",
                )
                .bind(&id)
                .fetch_one(&mut **tx)
                .await
                .map_err(db_err)
            })
        })
        .await
    }

    async fn update_chat_session_title(&self, id: &str, title: &str) -> Result<(), AppError> {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            return Err(AppError::Validation {
                message: "title must not be empty".into(),
                field: Some("title".into()),
            });
        }
        let touched =
            sqlx::query("UPDATE chat_sessions SET title = ?1, updated_at = ?2 WHERE id = ?3")
                .bind(trimmed)
                .bind(unix_now())
                .bind(id)
                .execute(&self.pools.write)
                .await
                .map_err(db_err)?
                .rows_affected();
        if touched == 0 {
            return Err(AppError::NotFound {
                entity: "chat_session".into(),
                id: id.to_string(),
            });
        }
        Ok(())
    }

    async fn list_chat_sessions(
        &self,
        before_updated_at: Option<i64>,
        limit: u32,
    ) -> Result<Vec<ChatSession>, AppError> {
        let base = "SELECT id, runner_id, scope_type, scope_id, session_id, epoch, status, \
                     title, superseded_by_id, message_count, total_input_tokens, \
                     total_output_tokens, cost_micros, created_at, updated_at \
                     FROM chat_sessions";
        if let Some(before) = before_updated_at {
            sqlx::query_as::<_, ChatSession>(&format!(
                "{base} WHERE updated_at < ?1 ORDER BY updated_at DESC LIMIT ?2"
            ))
            .bind(before)
            .bind(limit as i64)
            .fetch_all(&self.pools.read)
            .await
            .map_err(db_err)
        } else {
            sqlx::query_as::<_, ChatSession>(&format!("{base} ORDER BY updated_at DESC LIMIT ?1"))
                .bind(limit as i64)
                .fetch_all(&self.pools.read)
                .await
                .map_err(db_err)
        }
    }

    async fn set_pipeline_step(
        &self,
        conv_id: &str,
        step: PipelineStep,
        error: Option<String>,
    ) -> Result<(), AppError> {
        let now = unix_now();
        sqlx::query(
            "INSERT INTO pipeline_state (conv_id, step_completed, started_at, updated_at, error) \
             VALUES (?1, ?2, ?3, ?3, ?4) \
             ON CONFLICT(conv_id) DO UPDATE SET \
               step_completed = excluded.step_completed, \
               updated_at = excluded.updated_at, \
               error = excluded.error",
        )
        .bind(conv_id)
        .bind(step)
        .bind(now)
        .bind(&error)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_incomplete_pipelines(&self) -> Result<Vec<String>, AppError> {
        let rows = sqlx::query(
            "SELECT conv_id FROM pipeline_state WHERE step_completed NOT IN ('done', 'failed')",
        )
        .fetch_all(&self.pools.read)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|r| r.get::<String, _>("conv_id"))
            .collect())
    }

    async fn get_pipeline_step(&self, conv_id: &str) -> Result<Option<PipelineStep>, AppError> {
        let row: Option<(PipelineStep,)> =
            sqlx::query_as("SELECT step_completed FROM pipeline_state WHERE conv_id = ?1")
                .bind(conv_id)
                .fetch_optional(&self.pools.read)
                .await
                .map_err(db_err)?;
        Ok(row.map(|(step,)| step))
    }

    async fn get_pipeline_error(&self, conv_id: &str) -> Result<Option<String>, AppError> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT error FROM pipeline_state WHERE conv_id = ?1")
                .bind(conv_id)
                .fetch_optional(&self.pools.read)
                .await
                .map_err(db_err)?;
        Ok(row.and_then(|(error,)| error))
    }

    async fn get_setting(&self, key: &str) -> Result<Option<serde_json::Value>, AppError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?1")
            .bind(key)
            .fetch_optional(&self.pools.read)
            .await
            .map_err(db_err)?;
        row.map(|(v,)| {
            serde_json::from_str(&v)
                .map_err(|e| AppError::storage(format!("corrupted setting {key}: {e}")))
        })
        .transpose()
    }

    async fn set_setting(&self, key: &str, value: serde_json::Value) -> Result<(), AppError> {
        let now = unix_now();
        let value_str = value.to_string();
        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(&value_str)
        .bind(now)
        .execute(&self.pools.write)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn write_transcript(
        &self,
        conv_id: &str,
        json: &serde_json::Value,
    ) -> Result<(), AppError> {
        let path = paths::transcript_json_path(conv_id)?;
        atomic::atomic_write_json(&path, json)
    }

    async fn append_transcript_chunk(
        &self,
        conv_id: &str,
        line_json: &str,
    ) -> Result<(), AppError> {
        use std::io::Write;
        let path = paths::transcript_jsonl_path(conv_id)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        // Windows parity audit finding #13: same honest gap as
        // `fs::atomic::atomic_write`'s tmp-file open — no Windows ACL
        // restriction is attempted here either, for the same reasons (see
        // that function's comment). Transcript chunks land with default
        // ACLs on Windows.
        let mut f = opts.open(&path)?;
        writeln!(f, "{line_json}")?;
        f.sync_all()?;
        Ok(())
    }

    async fn write_extraction(
        &self,
        conv_id: &str,
        json: &serde_json::Value,
    ) -> Result<(), AppError> {
        let path = paths::extraction_json_path(conv_id)?;
        atomic::atomic_write_json(&path, json)
    }

    async fn write_summary(&self, conv_id: &str, md: &str) -> Result<(), AppError> {
        let path = paths::summary_md_path(conv_id)?;
        atomic::atomic_write(&path, md.as_bytes())
    }

    async fn write_project_memory(
        &self,
        project_id: &str,
        json: &serde_json::Value,
    ) -> Result<(), AppError> {
        let path = paths::project_memory_path(project_id)?;
        atomic::atomic_write_json(&path, json)
    }

    async fn read_transcript(&self, conv_id: &str) -> Result<Option<serde_json::Value>, AppError> {
        let path = paths::transcript_json_path(conv_id)?;
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)?;
        let json = serde_json::from_slice(&bytes)
            .map_err(|e| AppError::storage(format!("parse {}: {e}", path.display())))?;
        Ok(Some(json))
    }

    async fn read_summary(&self, conv_id: &str) -> Result<Option<String>, AppError> {
        let path = paths::summary_md_path(conv_id)?;
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(std::fs::read_to_string(&path)?))
    }

    async fn resume_pending_deletes(&self) -> Result<(), AppError> {
        pending_deletes::resume_pending_deletes(&self.pools).await
    }

    async fn list_stuck_deletes(&self) -> Result<Vec<StuckDelete>, AppError> {
        pending_deletes::list_stuck_deletes(&self.pools).await
    }

    async fn snapshot_backup_now(&self) -> Result<PathBuf, AppError> {
        let dir = paths::backups_dir()?;
        std::fs::create_dir_all(&dir)?;
        let out = dir.join(format!("mnemos-{}.db", unix_now()));
        sqlx::query("VACUUM INTO ?1")
            .bind(out.to_string_lossy().to_string())
            .execute(&self.pools.write)
            .await
            .map_err(db_err)?;
        Ok(out)
    }
}
