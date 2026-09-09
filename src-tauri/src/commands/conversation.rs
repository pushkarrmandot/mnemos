//! `conversation.*` Tauri commands: `retry_step("extraction")`, the thin
//! command wrapper over `memory::extract_conversation`, plus
//! `get_conversation_detail`, the
//! single read the Conversation Detail page uses to render everything but
//! live processing progress (which stays event-driven — see
//! `processing-progress` in `commands::recording`).

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;

use crate::db::models::{
    ActionItem, Conversation, ConversationFilter, ConversationStatus, Decision, DeletedExtraction,
    ExtractionKind, NewActionItem, OpenQuestion, Page, PipelineStep,
};
use crate::db::service::StorageService;
use crate::error::AppError;
use crate::state::AppState;

/// Conversation titles run longer than project names in practice — often
/// auto-generated, sometimes a full phrase — and are shown in roomier
/// contexts (page headers, table cells) than the nav-badge-tight spaces a
/// project name has to survive.
const MAX_CONVERSATION_TITLE_LEN: usize = 200;

/// Trims, rejects empty, rejects over-length. Mirrors `commands::project`'s
/// `validate_name` — the frontend's `EditableTitle` also
/// trims/disables-on-empty, but the command itself needs its own
/// server-side check too.
fn validate_name(raw: &str, max_len: usize, field: &str) -> Result<String, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::Validation {
            message: "must not be empty".into(),
            field: Some(field.into()),
        });
    }
    if trimmed.chars().count() > max_len {
        return Err(AppError::Validation {
            message: format!("must be {max_len} characters or fewer"),
            field: Some(field.into()),
        });
    }
    Ok(trimmed.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RetryableStep {
    Extraction,
}

/// One turn of `transcript.json`, verified against
/// `mnemos_worker/jobs/process_conversation.py::merge_transcripts`.
/// `speaker_label` is `"You"` (mic) / `"Them"` (system) only in v1 — no
/// diarization model runs yet, so `speaker_label_source` is always
/// `"source_file"` and `contact_id` is always `null`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TranscriptTurn {
    pub text: String,
    pub ts_start_ms: i64,
    pub ts_end_ms: i64,
    pub source: String,
    pub speaker_label: String,
    pub speaker_label_source: String,
    pub contact_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TranscriptDoc {
    pub schema_version: u32,
    pub conversation_id: String,
    pub duration_ms: i64,
    pub turns: Vec<TranscriptTurn>,
}

/// Everything Conversation Detail renders in one round
/// trip. `transcript`/`summary_markdown` are `None` until the pipeline has
/// written them (`transcribing`/`extracting` respectively) — the route
/// falls back to `<ProcessingOverlay>` + `processing-progress` events for
/// the in-between state, so a `None` here just means "not there yet", not
/// an error.
#[derive(Debug, Clone, Serialize, Type)]
pub struct ConversationDetail {
    pub conversation: Conversation,
    /// `None` when the conversation has no project assigned — a permanent,
    /// valid state, not "not loaded yet".
    pub project_name: Option<String>,
    pub pipeline_step: Option<PipelineStep>,
    pub pipeline_error: Option<String>,
    pub transcript: Option<TranscriptDoc>,
    pub summary_markdown: Option<String>,
    pub action_items: Vec<ActionItem>,
    pub decisions: Vec<Decision>,
    pub open_questions: Vec<OpenQuestion>,
}

/// Assigns or reassigns a conversation's project, `None` meaning "no
/// project" — always a valid, permanent choice, never
/// gated on recording having stopped. Purely a DB update: conversation
/// directories are flat and keyed by id alone
/// (`fs::paths::recordings_root`), so there is no filesystem move tied to
/// this anymore. If this conversation is still actively recording, also
/// updates the live session's cached `project_id` — `stop_recording`'s
/// post-pipeline memory-refresh trigger reads that cached value to know
/// which project's memory doc to refresh (the project chip is
/// editable "before, during, after recording, or never").
#[tauri::command]
#[specta::specta]
pub async fn conversation_set_project(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    project_id: Option<String>,
) -> Result<Conversation, AppError> {
    let conversation = state.storage.get_conversation(&conversation_id).await?;
    if conversation.project_id == project_id {
        return Ok(conversation);
    }

    let updated = state
        .storage
        .update_conversation_project(&conversation_id, project_id.as_deref())
        .await?;

    state
        .recording
        .resync_project(&conversation_id, project_id.clone());

    // Queues the conversation into the *new* project's normal
    // pending-refresh batch, same mechanism `maybe_auto_refresh` already
    // uses after a recording finishes processing — so its decisions/action
    // items/open questions (already correct for free, via the join) get
    // folded into that project's synthesized Overview/Scope-drift too,
    // without a dedicated "move" refresh path. Safe to call even if this
    // conversation hasn't finished processing yet: `refresh_project` reads
    // `extraction.json` via `read_json_or_none` and treats a missing file as
    // "nothing to add yet" rather than an error, so a still-processing
    // conversation just sits harmlessly in the pending list until its own
    // pipeline completion re-adds it (or a later refresh already cleared it).
    //
    // The OLD project is a known, accepted gap — see `ProjectChip`'s
    // confirmation-modal doc comment. There is no "this was removed" concept
    // for `refresh_project` to act on, and building one is out of scope here.
    if let Some(new_project_id) = project_id.clone() {
        let app_for_task = app.clone();
        let conv_id_for_task = conversation_id.clone();
        tokio::spawn(async move {
            let state = app_for_task.state::<AppState>();
            let Ok(project) = state.storage.get_project(&new_project_id).await else {
                return;
            };
            let result = crate::memory::maybe_auto_refresh(
                &state.storage,
                &state.python,
                &state.metrics,
                &new_project_id,
                &project.name,
                &conv_id_for_task,
            )
            .await;
            match result {
                Ok(Some(outcome)) => {
                    let _ = crate::events::ProjectMemoryUpdated {
                        project_id: new_project_id,
                        significant_change: outcome.significant_change,
                    }
                    .emit(&app_for_task);
                }
                Ok(None) => {}
                Err(err) => {
                    tracing::error!(
                        project_id = new_project_id,
                        error = %err,
                        "conversation.set_project_auto_refresh_failed"
                    );
                    let _ = crate::events::ProjectMemoryRefreshFailed {
                        project_id: new_project_id,
                        error_kind: err.to_string(),
                    }
                    .emit(&app_for_task);
                }
            }
        });
    }

    Ok(updated)
}

/// Hard ceiling on a single page, applied no matter what the caller asks for.
/// The frontend is the only caller and always sets a sane `limit`, but this is
/// the layer where "unbounded" stops being expressible from outside — a bug or
/// a future caller cannot reintroduce a full-table read here.
const MAX_CONVERSATION_PAGE: u32 = 200;

/// Lists conversations — `project_id: None` returns every conversation
/// regardless of project (Dashboard's "Recent Conversations", which is the
/// permanent home for unfiled conversations, not a stopgap), `Some(id)`
/// scopes to one project (Project Detail), and
/// `unfiled_only` scopes to conversations with no project at all.
///
/// Returns one page plus the total it was drawn from, so the caller can
/// render `Conversations (128)` and `108 remaining` without a second round
/// trip or a second source of truth.
#[tauri::command]
#[specta::specta]
pub async fn list_conversations(
    state: State<'_, AppState>,
    filter: ConversationFilter,
) -> Result<Page<Conversation>, AppError> {
    let filter = ConversationFilter {
        limit: Some(
            filter
                .limit
                .unwrap_or(MAX_CONVERSATION_PAGE)
                .min(MAX_CONVERSATION_PAGE),
        ),
        ..filter
    };
    // Count first, then the page. Both run against the same read pool on a
    // single-writer SQLite database, so the only way they can disagree is a
    // write landing between them — which costs a stale "N remaining" until
    // the next invalidation, not a wrong page.
    let total = state.storage.count_conversations(filter.clone()).await?;
    let items = state.storage.list_conversations(filter).await?;
    Ok(Page { items, total })
}

/// Just the size of a scope, for surfaces that render a number and no rows —
/// the left nav's per-project badges and the Recordings header. A cheap
/// count-only endpoint avoids fetching every row and taking `.length`, per
/// expanded project, on every nav render.
#[tauri::command]
#[specta::specta]
pub async fn count_conversations(
    state: State<'_, AppState>,
    filter: ConversationFilter,
) -> Result<u32, AppError> {
    state.storage.count_conversations(filter).await
}

/// A standalone action item — Home's "+" (`project_id: None`) or a Project
/// page's "+" (`project_id: Some`). No conversation, so no `conversationId`
/// param: there is nothing for this command to attach the item to.
///
/// `assignee_hint`/`assignee_is_self` let Home self-assign in the same write
/// that creates the row — see `insert_standalone_action_item`'s comment for
/// why that has to be atomic rather than a create-then-assign chain.
#[tauri::command]
#[specta::specta]
pub async fn create_standalone_action_item(
    state: State<'_, AppState>,
    project_id: Option<String>,
    text: String,
    assignee_hint: Option<String>,
    assignee_is_self: bool,
) -> Result<crate::db::models::ActionItemWithSource, AppError> {
    let result = state
        .storage
        .insert_standalone_action_item(
            project_id.as_deref(),
            &text,
            assignee_hint.as_deref(),
            assignee_is_self,
        )
        .await;
    if result.is_ok() {
        state.metrics.track(
            crate::metrics::events::ACTION_ITEM_ADDED_MANUALLY,
            crate::metrics::properties::EventProperties::from([(
                "scope",
                crate::metrics::properties::PropertyValue::Enum(if project_id.is_some() {
                    "project"
                } else {
                    "unfiled"
                }),
            )]),
        );
    }
    result
}

/// One page of the action items assigned to the user, across every project
/// and every unfiled conversation — Home's "Your to-dos". `done` is an exact
/// match, not a superset flag: Home's Open/Done tabs are two exclusive
/// queries, never "everything," so this command's own param is a plain
/// mandatory `bool` (unlike the storage-layer `ActionItemFilter.done`, which
/// is `Option<bool>` for the MCP tool's "no restriction" case).
#[tauri::command]
#[specta::specta]
pub async fn list_my_action_items(
    state: State<'_, AppState>,
    done: bool,
    limit: Option<u32>,
    offset: u32,
) -> Result<Page<crate::db::models::ActionItemWithSource>, AppError> {
    let filter = crate::db::models::ActionItemFilter {
        assigned_to_me: true,
        done: Some(done),
        limit: limit.unwrap_or(20).clamp(1, 200),
        offset,
        ..Default::default()
    };
    let total = state
        .storage
        .count_action_items_global(filter.clone())
        .await?;
    let items = state.storage.list_action_items_global(filter).await?;
    Ok(Page { items, total })
}

/// Sets who owes an action item. Free text, not a contact id, and not
/// validated against anything: there is no contacts table until v1.3, and the
/// correction path for a wrong guess must be cheaper than the guess. An empty
/// or whitespace-only string means "unassigned" and is normalised to `None`
/// here rather than stored as `""`, which would render as an empty badge.
#[tauri::command]
#[specta::specta]
pub async fn set_action_item_assignee(
    state: State<'_, AppState>,
    item_id: String,
    assignee_hint: Option<String>,
    assignee_is_self: bool,
) -> Result<ActionItem, AppError> {
    state
        .storage
        .set_action_item_assignee(&item_id, normalize_hint(assignee_hint), assignee_is_self)
        .await
}

/// Sets who owes the *answer* to an open question. Never touches
/// `raised_by_hint` — who asked is a fact about the past.
#[tauri::command]
#[specta::specta]
pub async fn set_open_question_owner(
    state: State<'_, AppState>,
    question_id: String,
    owner_hint: Option<String>,
    owner_is_self: bool,
) -> Result<OpenQuestion, AppError> {
    state
        .storage
        .set_open_question_owner(&question_id, normalize_hint(owner_hint), owner_is_self)
        .await
}

/// Marks an open question answered by this conversation, or reopens it.
#[tauri::command]
#[specta::specta]
pub async fn set_open_question_resolved(
    state: State<'_, AppState>,
    question_id: String,
    resolved_by_conversation_id: Option<String>,
) -> Result<OpenQuestion, AppError> {
    state
        .storage
        .set_open_question_resolved(&question_id, resolved_by_conversation_id.as_deref())
        .await
}

/// Saves a summary the user rewrote by hand.
///
/// Writing `summary.md` is all this needs to do: `memory::summary_is_user_edited`
/// decides whether a later regeneration may overwrite the file by comparing
/// its mtime against `extraction.json`'s, so the edit protects itself the
/// moment it lands. No flag to set and none to forget to set.
#[tauri::command]
#[specta::specta]
pub async fn conversation_set_summary(
    state: State<'_, AppState>,
    conversation_id: String,
    summary_markdown: String,
) -> Result<(), AppError> {
    state
        .storage
        .write_summary(&conversation_id, &summary_markdown)
        .await
}

/// Removes one extracted item the model got wrong.
///
/// Returns the deleted row whole so the undo toast can hand it straight back
/// to [`conversation_restore_extraction_item`] — the item comes back with its
/// assignee, due date and quote intact rather than as a bare line of text.
#[tauri::command]
#[specta::specta]
pub async fn conversation_delete_extraction_item(
    state: State<'_, AppState>,
    kind: ExtractionKind,
    item_id: String,
) -> Result<DeletedExtraction, AppError> {
    state.storage.delete_extraction_item(kind, &item_id).await
}

/// Undo for [`conversation_delete_extraction_item`].
#[tauri::command]
#[specta::specta]
pub async fn conversation_restore_extraction_item(
    state: State<'_, AppState>,
    item: DeletedExtraction,
) -> Result<(), AppError> {
    state.storage.restore_extraction_item(item).await
}

/// Rewrites an extracted item's text, which also claims it: the row stops
/// being model-owned, so regenerating leaves it alone. Returns nothing —
/// the caller already knows the text it sent, and every list that shows this
/// row patches its own cache optimistically.
#[tauri::command]
#[specta::specta]
pub async fn conversation_set_extraction_text(
    state: State<'_, AppState>,
    kind: ExtractionKind,
    item_id: String,
    text: String,
) -> Result<(), AppError> {
    state
        .storage
        .set_extraction_text(kind, &item_id, &text)
        .await
}

/// `Some("  ")` and `Some("")` both mean the user cleared the field.
fn normalize_hint(hint: Option<String>) -> Option<String> {
    hint.map(|h| h.trim().to_string()).filter(|h| !h.is_empty())
}

#[tauri::command]
#[specta::specta]
pub async fn get_conversation_detail(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<ConversationDetail, AppError> {
    let conversation = state.storage.get_conversation(&conversation_id).await?;
    let project_name = match &conversation.project_id {
        Some(pid) => Some(state.storage.get_project(pid).await?.name),
        None => None,
    };

    let pipeline_step = state.storage.get_pipeline_step(&conversation_id).await?;
    let pipeline_error = state.storage.get_pipeline_error(&conversation_id).await?;

    let transcript_json = state.storage.read_transcript(&conversation_id).await?;
    let transcript = transcript_json
        .map(serde_json::from_value::<TranscriptDoc>)
        .transpose()
        .map_err(|e| AppError::storage(format!("parse transcript.json: {e}")))?;

    let summary_markdown = state.storage.read_summary(&conversation_id).await?;

    let action_items = state.storage.list_action_items(&conversation_id).await?;
    let decisions = state.storage.list_decisions(&conversation_id).await?;
    let open_questions = state.storage.list_open_questions(&conversation_id).await?;

    Ok(ConversationDetail {
        conversation,
        project_name,
        pipeline_step,
        pipeline_error,
        transcript,
        summary_markdown,
        action_items,
        decisions,
        open_questions,
    })
}

/// Toggles one `<ActionItemRow>` checkbox. Thin wrapper over
/// the storage method — Conversation Detail is the first UI surface that
/// renders action items at all.
#[tauri::command]
#[specta::specta]
pub async fn conversation_set_action_item_done(
    state: State<'_, AppState>,
    action_item_id: String,
    done: bool,
) -> Result<ActionItem, AppError> {
    state
        .storage
        .set_action_item_done(&action_item_id, done)
        .await
}

/// Renames a conversation (`<EditableTitle>`).
#[tauri::command]
#[specta::specta]
pub async fn conversation_set_title(
    state: State<'_, AppState>,
    conversation_id: String,
    title: String,
) -> Result<Conversation, AppError> {
    let title = validate_name(&title, MAX_CONVERSATION_TITLE_LEN, "title")?;
    state
        .storage
        .update_conversation_title(&conversation_id, &title)
        .await
}

/// Persists Conversation Detail's Notes tab — the
/// recording screen's notes draft is saved here rather than staying
/// local-only.
#[tauri::command]
#[specta::specta]
pub async fn conversation_set_notes(
    state: State<'_, AppState>,
    conversation_id: String,
    notes: String,
) -> Result<Conversation, AppError> {
    state
        .storage
        .update_conversation_notes(&conversation_id, &notes)
        .await
}

/// Conversation Detail's overflow-menu Delete (12_CORNER_CASES.md
/// "Data delete flows"). Thin wrapper: reuses the same atomic,
/// crash-resumable `enqueue_conversation_delete` path everything else in
/// the app uses (crash-recovery's Discard actions, in
/// `commands::recording`), so there is nothing new to get wrong here.
#[tauri::command]
#[specta::specta]
pub async fn conversation_delete(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<(), AppError> {
    state.storage.delete_conversation(&conversation_id).await
}

/// Adds a user-authored action item — the other way an `action_items`
/// row can exist is via extraction.
#[tauri::command]
#[specta::specta]
pub async fn conversation_create_action_item(
    state: State<'_, AppState>,
    conversation_id: String,
    text: String,
) -> Result<ActionItem, AppError> {
    let result = state
        .storage
        .insert_action_item(
            &conversation_id,
            NewActionItem {
                text,
                assignee_hint: None,
                assignee_is_self: false,
                due_hint: None,
                source_ts: None,
            },
        )
        .await;
    if result.is_ok() {
        state.metrics.track(
            crate::metrics::events::ACTION_ITEM_ADDED_MANUALLY,
            crate::metrics::properties::EventProperties::from([(
                "scope",
                crate::metrics::properties::PropertyValue::Enum("conversation"),
            )]),
        );
    }
    result
}

/// What a regeneration actually did, so the UI can say so instead of
/// guessing.
///
/// `summary_written` is `false` when the user had rewritten `summary.md` by
/// hand and it was therefore left alone — the ordinary, expected outcome of
/// regenerating a conversation whose summary you have edited, not an error.
/// Before this the command returned `()`, so the frontend announced "Summary
/// regenerated" in both cases, one of which was untrue.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RegenerateOutcome {
    pub summary_written: bool,
}

/// Re-runs one conversation's extraction turn (idempotent
/// re-extraction), then — same as the post-recording pipeline — attempts the
/// project-memory auto-refresh trigger. `force_overwrite` is sourced from the
/// UI's "Overwrite your edits?" modal confirmation (`pages/04`); the auto
/// pipeline always calls this with `force_overwrite=false`.
#[tauri::command]
#[specta::specta]
pub async fn conversation_retry_step(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    step: RetryableStep,
    force_overwrite: bool,
) -> Result<RegenerateOutcome, AppError> {
    let RetryableStep::Extraction = step;

    let conversation = state.storage.get_conversation(&conversation_id).await?;

    // If the pipeline never reached extracting, this errors
    // AppError::Validation — no `pipeline_state` row at all
    // means `stop_recording` never even reached `finalizing`.
    if state
        .storage
        .get_pipeline_step(&conversation_id)
        .await?
        .is_none()
    {
        return Err(AppError::Validation {
            message: "conversation has no pipeline state yet".into(),
            field: Some("pipeline_state".into()),
        });
    }

    let outcome = crate::memory::extract_conversation(
        &state.storage,
        &state.python,
        &conversation_id,
        force_overwrite,
    )
    .await?;
    tracing::info!(
        conversation_id,
        action_items = outcome.action_items,
        decisions = outcome.decisions,
        open_questions = outcome.open_questions,
        summary_written = outcome.summary_written,
        "conversation.retry_step_extraction_done"
    );
    state.metrics.track(
        crate::metrics::events::EXTRACTION_COMPLETED,
        crate::metrics::properties::EventProperties::from([
            (
                "action_items_count",
                crate::metrics::properties::PropertyValue::UInt(outcome.action_items as u64),
            ),
            (
                "decisions_count",
                crate::metrics::properties::PropertyValue::UInt(outcome.decisions as u64),
            ),
            (
                "open_questions_count",
                crate::metrics::properties::PropertyValue::UInt(outcome.open_questions as u64),
            ),
            (
                "bookmarks_count",
                crate::metrics::properties::PropertyValue::UInt(outcome.bookmarks as u64),
            ),
        ]),
    );

    // Project memory auto-refresh only applies when this conversation
    // belongs to a project — unfiled conversations have no memory doc to
    // refresh (recordings never require a project).
    if let Some(project_id) = conversation.project_id.clone() {
        if let Ok(project) = state.storage.get_project(&project_id).await {
            match crate::memory::maybe_auto_refresh(
                &state.storage,
                &state.python,
                &state.metrics,
                &project_id,
                &project.name,
                &conversation_id,
            )
            .await
            {
                Ok(Some(refresh)) => {
                    let _ = crate::events::ProjectMemoryUpdated {
                        project_id,
                        significant_change: refresh.significant_change,
                    }
                    .emit(&app);
                }
                Ok(None) => {}
                Err(err) => {
                    tracing::error!(project_id, error = %err, "conversation.retry_step_auto_refresh_failed");
                    let _ = crate::events::ProjectMemoryRefreshFailed {
                        project_id,
                        error_kind: err.to_string(),
                    }
                    .emit(&app);
                }
            }
        }
    }

    state
        .storage
        .set_pipeline_step(
            &conversation_id,
            crate::db::models::PipelineStep::Done,
            None,
        )
        .await?;

    // Clear the conversation-level `failed` status too, not just the
    // pipeline step. Without this a retry that fully succeeds still leaves
    // the row at `ConversationStatus::Failed`, so every list view goes on
    // labelling it failed while the detail page shows a complete summary.
    // `(None, None)` preserves `ended_at`/`duration_s` — see
    // `update_conversation_status`'s COALESCE.
    if conversation.status == ConversationStatus::Failed {
        state
            .storage
            .update_conversation_status(&conversation_id, ConversationStatus::Ready, None, None)
            .await?;
    }

    Ok(RegenerateOutcome {
        summary_written: outcome.summary_written,
    })
}

#[cfg(test)]
mod validate_name_tests {
    use super::*;

    #[test]
    fn trims_and_accepts_a_normal_title() {
        assert_eq!(
            validate_name("  Kickoff sync  ", 200, "title").unwrap(),
            "Kickoff sync"
        );
    }

    #[test]
    fn rejects_empty_and_whitespace_only() {
        assert!(validate_name("", 200, "title").is_err());
        assert!(validate_name("   ", 200, "title").is_err());
    }

    #[test]
    fn rejects_over_length_by_character_count() {
        assert!(validate_name(&"a".repeat(200), 200, "title").is_ok());
        assert!(validate_name(&"a".repeat(201), 200, "title").is_err());
    }
}
