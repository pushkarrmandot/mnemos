//! `project.*` Tauri commands. Currently just
//! `refresh_memory` — the manual-refresh thin wrapper over
//! `memory::refresh_project` (no command layer existed
//! before this). Fires-and-returns: the actual refresh runs in a detached task and
//! reports back via `project-memory-updated` / `project-memory-refresh-failed`.

use std::collections::HashMap;
use std::sync::Mutex as StdMutex;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;

use crate::db::models::{NewProject, Page, Project, ProjectPatch};
use crate::db::service::StorageService;
use crate::error::AppError;
use crate::state::AppState;

/// Short label — nav badges, breadcrumbs, dropdown triggers all render this
/// in tight space. 80 chars leaves room to be descriptive without any of
/// those surfaces needing more than CSS truncation to stay usable.
const MAX_PROJECT_NAME_LEN: usize = 80;

/// Trims, rejects empty, rejects over-length. Shared by every project-name
/// write path — `create_project` had neither check for a while; `project_set_name`
/// had only the empty check. A name typed through the UI never hits either
/// branch (both inputs already trim/disable on empty client-side), so this is
/// defense against anything that calls the command directly, and the only
/// place enforcing the length cap at all.
fn validate_name(raw: &str, max_len: usize, field: &str) -> Result<String, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::Validation {
            message: "name must not be empty".into(),
            field: Some(field.into()),
        });
    }
    if trimmed.chars().count() > max_len {
        return Err(AppError::Validation {
            message: format!("name must be {max_len} characters or fewer"),
            field: Some(field.into()),
        });
    }
    Ok(trimmed.to_string())
}

/// `project_memory.json`'s shape (`pages/05_PROJECT_MEMORY.md`
/// "Storage schema"). Read-only render for now — the inline-editable
/// prose blocks the "Editing behavior" section describes come later.
///
/// `supersessions` stays untyped `Value` deliberately, matching
/// `ipc::python::ExtractMemoryResponse`'s own `Vec<Value>` — it comes
/// straight from the worker's LLM response with no schema enforced on it
/// anywhere in the pipeline, so a typed struct here would be one bad
/// generation away from failing to parse the whole file. Note `last_refresh_at`
/// is `unix_now()`, an `i64`, not the ISO-string the page doc's example
/// sketch showed — `memory::refresh_project` writes it directly, verified
/// against a real `project_memory.json` on disk.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ProjectMemory {
    pub overview_markdown: String,
    pub scope_drift_markdown: String,
    #[serde(default)]
    pub supersessions: Vec<serde_json::Value>,
    pub last_refresh_at: Option<i64>,
    pub last_refresh_runner: Option<String>,
}

/// Every non-deleted project, pinned first then last-active (`list_projects`'s
/// ordering) — backs the left nav's project tree.
#[tauri::command]
#[specta::specta]
pub async fn list_projects(state: State<'_, AppState>) -> Result<Vec<Project>, AppError> {
    // Unbounded on purpose: the project list is a human-curated set that the
    // left nav and every project picker render in full, and a user with
    // enough projects to need paging here has a different problem. The
    // `ProjectFilter` bound exists for the MCP tool layer, which must cap
    // anything it hands an agent.
    state
        .storage
        .list_projects(crate::db::models::ProjectFilter::default())
        .await
}

/// Real "+New Project" (replaces the earlier stub toast). Name-only in v1 — the
/// description field exists on the model but no UI writes it yet.
#[tauri::command]
#[specta::specta]
pub async fn create_project(state: State<'_, AppState>, name: String) -> Result<Project, AppError> {
    let name = validate_name(&name, MAX_PROJECT_NAME_LEN, "name")?;
    let result = state
        .storage
        .create_project(NewProject {
            name,
            description: None,
        })
        .await;
    if result.is_ok() {
        state.metrics.track(
            crate::metrics::events::PROJECT_CREATED,
            crate::metrics::properties::EventProperties::new(),
        );
    }
    result
}

#[tauri::command]
#[specta::specta]
pub async fn get_project(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Project, AppError> {
    state.storage.get_project(&project_id).await
}

/// Project Detail's inline-editable name (`<EditableProjectName>`,
/// mirroring Conversation Detail's `<EditableTitle>`). Rejects an
/// empty/whitespace-only name the same way `update_conversation_title`
/// does — `StorageService::update_project` itself doesn't validate this,
/// so the check lives here.
#[tauri::command]
#[specta::specta]
pub async fn project_set_name(
    state: State<'_, AppState>,
    project_id: String,
    name: String,
) -> Result<Project, AppError> {
    let name = validate_name(&name, MAX_PROJECT_NAME_LEN, "name")?;
    state
        .storage
        .update_project(
            &project_id,
            ProjectPatch {
                name: Some(name),
                ..Default::default()
            },
        )
        .await
}

/// `None` when no refresh has run for this project yet (Project Memory
/// pane's "empty until first conversation is processed" state, per
/// `pages/05_PROJECT_MEMORY.md` §1 Overview).
#[tauri::command]
#[specta::specta]
pub async fn get_project_memory(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Option<ProjectMemory>, AppError> {
    let raw = state.storage.read_project_memory(&project_id).await?;
    raw.map(|value| {
        serde_json::from_value(value)
            .map_err(|e| AppError::storage(format!("parse project_memory.json: {e}")))
    })
    .transpose()
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct RefreshHandle {
    pub enqueued_at_ms: i64,
    pub batch_signature: String,
}

/// A second `project_refresh_memory` for the same project within 5s is a
/// no-op. Keyed in-memory, not persisted — a restart clearing
/// it just means the next call isn't debounced, which is harmless.
#[derive(Default)]
pub struct RefreshDebounce {
    last_triggered_at_ms: StdMutex<HashMap<String, i64>>,
}

impl RefreshDebounce {
    pub fn new() -> Self {
        Self::default()
    }
}

const DEBOUNCE_MS: i64 = 5_000;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_millis() as i64
}

/// Manual refresh: flushes the pending-count immediately (union of whatever
/// conversations are currently pending for this project) and enqueues
/// `refresh_project_memory`. Debounced — a repeat call inside the
/// 5s window is a no-op that returns the already-enqueued handle instead of
/// starting a second refresh.
#[tauri::command]
#[specta::specta]
pub async fn project_refresh_memory(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: String,
) -> Result<RefreshHandle, AppError> {
    let now = now_ms();
    let pending = crate::memory::read_pending(&state.storage, &project_id).await?;
    let signature = crate::memory::batch_signature(&pending);

    {
        let mut last = state
            .project_refresh_debounce
            .last_triggered_at_ms
            .lock()
            .unwrap();
        if let Some(&prev) = last.get(&project_id) {
            if now - prev < DEBOUNCE_MS {
                return Ok(RefreshHandle {
                    enqueued_at_ms: prev,
                    batch_signature: signature,
                });
            }
        }
        last.insert(project_id.clone(), now);
    }

    // Nothing pending — still a valid no-op call (e.g. right after a
    // successful auto-refresh already cleared the list), just don't spawn a
    // worker call for an empty batch.
    if pending.is_empty() {
        return Ok(RefreshHandle {
            enqueued_at_ms: now,
            batch_signature: signature,
        });
    }

    let project = state.storage.get_project(&project_id).await?;

    let app_for_task = app.clone();
    let project_id_for_task = project_id.clone();
    tokio::spawn(async move {
        let state = app_for_task.state::<AppState>();
        let refresh_start = std::time::Instant::now();
        let result = crate::memory::refresh_project(
            &state.storage,
            &state.python,
            &project_id_for_task,
            &pending,
            &project.name,
        )
        .await;
        match result {
            Ok(outcome) => {
                crate::memory::track_refresh_completed(
                    &state.metrics,
                    "manual",
                    &outcome,
                    refresh_start.elapsed(),
                );
                let _ = crate::memory::clear_pending(&state.storage, &project_id_for_task).await;
                let _ =
                    crate::memory::set_last_error(&state.storage, &project_id_for_task, None).await;
                let _ = crate::events::ProjectMemoryUpdated {
                    project_id: project_id_for_task,
                    significant_change: outcome.significant_change,
                }
                .emit(&app_for_task);
            }
            Err(err) => {
                tracing::error!(project_id = project_id_for_task, error = %err, "project.refresh_memory_failed");
                let _ = crate::memory::set_last_error(
                    &state.storage,
                    &project_id_for_task,
                    Some(&err.to_string()),
                )
                .await;
                let _ = crate::events::ProjectMemoryRefreshFailed {
                    project_id: project_id_for_task,
                    error_kind: err.to_string(),
                }
                .emit(&app_for_task);
            }
        }
    });

    Ok(RefreshHandle {
        enqueued_at_ms: now,
        batch_signature: signature,
    })
}

/// Project Memory's two reactive structured sections (05_PROJECT_MEMORY.md
/// §"Two kinds of content" — "straight SQL queries against DB, no agent
/// call, always current").
///
/// Deliberately carries **no action items**: 05 lists exactly five sections
/// and action items are not among them. They aggregate per *person* on the
/// Dashboard (02's YOUR TO-DOS, which spans every project), not per project
/// — a project's durable memory is its decisions and unresolved questions,
/// while action items are transient work that completes and disappears.
///
/// Rows carry `conv_id` but no conversation title: the Project page already
/// queries `list_conversations` for its Conversations section, so the
/// frontend joins locally instead of paying for a wider query here.
///
/// These are two paged commands because the two lists page independently:
/// decisions read forwards from the oldest and reveal *earlier* entries,
/// while open questions read newest-first and have an Open/Resolved split. A
/// shared `PROJECT_EXTRACTION_LIMIT = 500` ceiling that fetched both together
/// would truncate silently — the page showing 500 rows and saying nothing
/// about the rest.
/// Default page sizes, matching what the Project page reveals in one step.
const PROJECT_PAGE_DEFAULT: u32 = 20;
/// Ceiling on one page, the same guarantee `MAX_CONVERSATION_PAGE` gives:
/// "unbounded" is not expressible from outside this layer.
const PROJECT_PAGE_MAX: u32 = 200;

fn clamp_page(limit: Option<u32>) -> u32 {
    limit
        .unwrap_or(PROJECT_PAGE_DEFAULT)
        .clamp(1, PROJECT_PAGE_MAX)
}

/// Whether this project's synthesized memory is behind, and whether the last
/// attempt to catch it up failed.
///
/// `pending_count` alone is not a problem signal: refreshes batch (see
/// `DEFAULT_REFRESH_EVERY_N`), so a couple of pending conversations is the
/// system working. `last_error` is the signal — it is set only when a refresh
/// actually failed and cleared the moment one succeeds. The UI shows its
/// staleness banner on `last_error.is_some()`, never on `pending_count > 0`,
/// so a healthy backlog stays invisible and the one message that matters
/// keeps its meaning.
#[derive(Debug, Serialize, Deserialize, Type)]
pub struct ProjectMemoryStatus {
    /// Conversations recorded since the last synthesis, i.e. how far behind
    /// the Overview currently is.
    pub pending_count: u32,
    /// How many `pending_count` has to reach before the next rewrite. Sent
    /// to the UI so it can state the cadence instead of leaving a stale
    /// Overview looking broken.
    pub refresh_threshold: u32,
    pub last_error: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn project_get_memory_status(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<ProjectMemoryStatus, AppError> {
    let pending = crate::memory::read_pending(&state.storage, &project_id).await?;
    Ok(ProjectMemoryStatus {
        pending_count: pending.len() as u32,
        refresh_threshold: crate::memory::refresh_threshold(&state.storage).await? as u32,
        last_error: crate::memory::read_last_error(&state.storage, &project_id).await?,
    })
}

/// Home's "Project pulse" (`02_DASHBOARD_AND_NAV.md`'s PROJECT PULSE
/// section) — one row per project active enough to be worth surfacing, with
/// what changed recently.
#[derive(Debug, Serialize, Deserialize, Type)]
pub struct ProjectPulseItem {
    pub project_id: String,
    pub project_name: String,
    pub decisions_recent: u32,
    pub open_questions_recent: u32,
    pub last_activity_at: Option<i64>,
}

/// A project under this many conversations doesn't get a pulse card — per
/// `02_DASHBOARD_AND_NAV.md`: "only projects with >=5 conversations (avoids
/// empty-card churn)".
const PULSE_MIN_CONVERSATIONS: i64 = 5;

/// The window "recent" means for a pulse card. A fixed rolling lookback, not
/// "since you last opened the app" — nothing tracks a last-visited timestamp
/// today, and adding one for this alone isn't worth the new state.
const PULSE_WINDOW_DAYS: i64 = 7;

/// Eligible projects (>=5 conversations), sorted by recent activity,
/// each with its decision/open-question counts over the last
/// `PULSE_WINDOW_DAYS` days. One round trip — the alternative is one query
/// per eligible project from the frontend, which is the same N+1 shape
/// already removed from the conversation lists.
#[tauri::command]
#[specta::specta]
pub async fn dashboard_get_project_pulse(
    state: State<'_, AppState>,
) -> Result<Vec<ProjectPulseItem>, AppError> {
    let stats = state.storage.project_activity_stats().await?;
    let mut eligible: Vec<_> = stats
        .into_iter()
        .filter(|s| s.conversation_count >= PULSE_MIN_CONVERSATIONS)
        .collect();
    eligible.sort_by_key(|s| std::cmp::Reverse(s.last_activity_at));

    if eligible.is_empty() {
        return Ok(vec![]);
    }

    let projects = state
        .storage
        .list_projects(crate::db::models::ProjectFilter::default())
        .await?;
    let project_name = |id: &str| {
        projects
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.clone())
            .unwrap_or_default()
    };

    let since = crate::db::models::unix_now() - PULSE_WINDOW_DAYS * 86_400;
    let mut out = Vec::with_capacity(eligible.len());
    for stat in eligible {
        let decisions_recent = state
            .storage
            .count_decisions_global(crate::db::models::DecisionFilter {
                project_id: Some(stat.project_id.clone()),
                since: Some(since),
                limit: 0,
                ..Default::default()
            })
            .await?;
        let open_questions_recent = state
            .storage
            .count_open_questions_global(crate::db::models::OpenQuestionFilter {
                project_id: Some(stat.project_id.clone()),
                since: Some(since),
                limit: 0,
                ..Default::default()
            })
            .await?;
        out.push(ProjectPulseItem {
            project_name: project_name(&stat.project_id),
            project_id: stat.project_id,
            decisions_recent,
            open_questions_recent,
            last_activity_at: stat.last_activity_at,
        });
    }
    Ok(out)
}

/// One page of a project's decision log, oldest first (the order
/// `list_decisions_global` guarantees — a decision log reads forwards).
#[tauri::command]
#[specta::specta]
pub async fn project_list_decisions(
    state: State<'_, AppState>,
    project_id: String,
    limit: Option<u32>,
    offset: u32,
) -> Result<Page<crate::db::models::Decision>, AppError> {
    let filter = crate::db::models::DecisionFilter {
        project_id: Some(project_id),
        limit: clamp_page(limit),
        offset,
        ..Default::default()
    };
    let total = state.storage.count_decisions_global(filter.clone()).await?;
    let items = state.storage.list_decisions_global(filter).await?;
    Ok(Page { items, total })
}

/// One page of a project's action items — the model-derived ones (via the
/// conversations join) and the standalone ones added directly from this
/// page's own "+". `done` is an exact match — see `list_my_action_items`'s
/// doc comment for why this command's param is a plain mandatory `bool`
/// rather than the storage layer's `Option<bool>`.
#[tauri::command]
#[specta::specta]
pub async fn project_list_action_items(
    state: State<'_, AppState>,
    project_id: String,
    done: bool,
    limit: Option<u32>,
    offset: u32,
) -> Result<Page<crate::db::models::ActionItemWithSource>, AppError> {
    let filter = crate::db::models::ActionItemFilter {
        project_id: Some(project_id),
        done: Some(done),
        limit: clamp_page(limit),
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

/// One page of a project's open questions, newest first.
/// `resolved_only` backs the Open/Resolved split. The `include_resolved`
/// filter existed for a while with no caller ever setting it, so every
/// resolved question stayed in the Open list forever; the split needs the
/// complementary predicate too, so each tab pages and counts on its own.
#[tauri::command]
#[specta::specta]
pub async fn project_list_open_questions(
    state: State<'_, AppState>,
    project_id: String,
    resolved_only: bool,
    limit: Option<u32>,
    offset: u32,
) -> Result<Page<crate::db::models::OpenQuestionWithSource>, AppError> {
    let filter = crate::db::models::OpenQuestionFilter {
        project_id: Some(project_id),
        resolved_only,
        limit: clamp_page(limit),
        offset,
        ..Default::default()
    };
    let total = state
        .storage
        .count_open_questions_global(filter.clone())
        .await?;
    let items = state.storage.list_open_questions_global(filter).await?;
    Ok(Page { items, total })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_trims_and_accepts_a_normal_name() {
        assert_eq!(validate_name("  Acme  ", 80, "name").unwrap(), "Acme");
    }

    #[test]
    fn validate_name_rejects_empty_and_whitespace_only() {
        assert!(validate_name("", 80, "name").is_err());
        assert!(validate_name("   ", 80, "name").is_err());
    }

    #[test]
    fn validate_name_rejects_over_length_by_character_count_not_bytes() {
        let exactly_80 = "a".repeat(80);
        assert!(validate_name(&exactly_80, 80, "name").is_ok());
        let over_80 = "a".repeat(81);
        assert!(validate_name(&over_80, 80, "name").is_err());

        // Multi-byte characters must count as one character each, not one
        // byte each — a name of 80 emoji is 80 characters and well over 80
        // bytes; rejecting it on byte length would reject valid short names.
        let eighty_emoji = "🎉".repeat(80);
        assert!(validate_name(&eighty_emoji, 80, "name").is_ok());
    }

    #[test]
    fn validate_name_error_names_the_field() {
        let err = validate_name("", 80, "name").unwrap_err();
        match err {
            AppError::Validation { field, .. } => assert_eq!(field.as_deref(), Some("name")),
            other => panic!("expected Validation error, got {other:?}"),
        }
    }
}
