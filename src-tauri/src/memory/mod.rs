//! Orchestration for the two memory pipelines: (A)
//! per-conversation extraction and (B) project-memory refresh.
//!
//! Both entry points are plain async functions over `&dyn StorageService` +
//! `&WorkerSupervisor`, not `#[tauri::command]`s — the command-layer wrappers
//! that call into them live in `commands::conversation`/`commands::recording`
//! (extraction) and `commands::project` (refresh).
//!
//! The agent turn itself — including the JSON-schema validation and the
//! one-retry-then-fail policy — happens inside the Python
//! worker's job handler, which is the one that actually calls
//! `run_agent_extraction`. What comes back over the forward
//! `extract_memory`/`refresh_project_memory` RPC is already validated; this
//! module's only job is the storage-write orchestration around it.

use std::path::Path;

use serde_json::Value;

use crate::db::models::{
    ExtractionBundle, NewActionItem, NewBookmark, NewDecision, NewOpenQuestion, PipelineStep,
};
use crate::db::service::StorageService;
use crate::error::AppError;
use crate::fs::{atomic, paths};
use crate::ipc::python::{
    ExtractMemory, ExtractMemoryResponse, RefreshProjectMemory, WorkerSupervisor,
};

/// The title a conversation row is created with (`recording.rs::start_recording`)
/// before extraction ever runs. Doubles as the "hasn't been titled yet" sentinel
/// `extract_conversation` checks below — a conversation still carrying this
/// exact string is fair game for the auto-generated title; anything else,
/// whether a manual rename or a prior auto-title, is left alone.
pub const DEFAULT_CONVERSATION_TITLE: &str = "Untitled Conversation";

impl From<crate::ipc::python::ExtractedActionItem> for NewActionItem {
    fn from(a: crate::ipc::python::ExtractedActionItem) -> Self {
        NewActionItem {
            text: a.text,
            assignee_hint: a.assignee_hint,
            assignee_is_self: a.assignee_is_self,
            due_hint: a.due_hint,
            source_ts: a.source_timestamp_ms,
        }
    }
}

impl From<crate::ipc::python::ExtractedDecision> for NewDecision {
    fn from(d: crate::ipc::python::ExtractedDecision) -> Self {
        NewDecision {
            statement: d.statement,
            quote: d.quote,
            decided_by_hint: d.decided_by_hint,
            decided_by_is_self: d.decided_by_is_self,
            source_ts: d.source_timestamp_ms,
        }
    }
}

impl From<crate::ipc::python::ExtractedOpenQuestion> for NewOpenQuestion {
    fn from(q: crate::ipc::python::ExtractedOpenQuestion) -> Self {
        NewOpenQuestion {
            question: q.question,
            raised_by_hint: q.raised_by_hint,
            raised_by_is_self: q.raised_by_is_self,
            source_ts: q.source_timestamp_ms,
        }
    }
}

impl From<crate::ipc::python::ExtractedBookmark> for NewBookmark {
    fn from(b: crate::ipc::python::ExtractedBookmark) -> Self {
        NewBookmark {
            label: b.text,
            ts_ms: b.timestamp_ms,
        }
    }
}

/// `metrics::events::PROJECT_MEMORY_REFRESHED` — shared by both trigger
/// paths (`maybe_auto_refresh` here, and `commands::project::project_refresh_memory`'s
/// manual trigger), same event shape either way: `trigger` is the only
/// thing that tells them apart.
pub(crate) fn track_refresh_completed(
    metrics: &crate::metrics::Metrics,
    trigger: &'static str,
    outcome: &RefreshOutcome,
    elapsed: std::time::Duration,
) {
    metrics.track(
        crate::metrics::events::PROJECT_MEMORY_REFRESHED,
        crate::metrics::properties::EventProperties::from([
            (
                "trigger",
                crate::metrics::properties::PropertyValue::Enum(trigger),
            ),
            (
                "significant_change",
                crate::metrics::properties::PropertyValue::Bool(outcome.significant_change),
            ),
            (
                "duration_ms",
                crate::metrics::properties::PropertyValue::UInt(elapsed.as_millis() as u64),
            ),
        ]),
    );
}

#[derive(Debug, Clone)]
pub struct ExtractionOutcome {
    pub action_items: usize,
    pub decisions: usize,
    pub open_questions: usize,
    pub bookmarks: usize,
    /// `false` when `summary.md` was skipped because the user edited it
    /// since the last extraction and `force_overwrite` wasn't set (§8
    /// "user has manually edited summary.md").
    pub summary_written: bool,
}

fn read_json(path: &Path) -> Result<Value, AppError> {
    let bytes = std::fs::read(path)
        .map_err(|e| AppError::storage(format!("read {}: {e}", path.display())))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| AppError::storage(format!("parse {}: {e}", path.display())))
}

fn read_json_or_none(path: &Path) -> Result<Option<Value>, AppError> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(read_json(path)?))
}

/// "User has manually edited summary.md": `summary.md`'s mtime
/// newer than `extraction.json`'s means the user's edit hasn't been
/// re-extracted over yet. No `extraction.json` yet (first run) or no
/// `summary.md` yet both mean "not user-edited" — write freely.
fn summary_is_user_edited(conv_id: &str) -> Result<bool, AppError> {
    let summary_path = paths::summary_md_path(conv_id)?;
    let extraction_path = paths::extraction_json_path(conv_id)?;
    let (Ok(summary_meta), Ok(extraction_meta)) = (
        std::fs::metadata(&summary_path),
        std::fs::metadata(&extraction_path),
    ) else {
        return Ok(false);
    };
    let (Ok(summary_mtime), Ok(extraction_mtime)) =
        (summary_meta.modified(), extraction_meta.modified())
    else {
        return Ok(false);
    };
    Ok(summary_mtime > extraction_mtime)
}

/// The one "contact" v1 can supply: the user themselves, from the name
/// captured during onboarding.
///
/// Until this, `contacts` was hardcoded `[]`, so the model was asked to
/// attribute action items in a meeting without being told who the user was.
/// It had exactly two labels available — `"You"` for the mic channel and
/// `"Them"` for everything else — which is why a fourteen-person meeting
/// produced items assigned to `"Them"`, a value the UI now suppresses because
/// it means "one of fourteen, unknown". With the name present the model can
/// resolve "Priya, can you send that over" to the user in a transcript where
/// the mic channel never says it.
///
/// `is_self` is what makes this more than a name in a list: it is the flag the
/// prompt keys on to emit `"You"` rather than the literal name, so downstream
/// code has one representation of the user, not two.
async fn self_contact(storage: &dyn StorageService) -> Result<Vec<Value>, AppError> {
    let read = |key: &'static str| async move {
        Ok::<_, AppError>(
            storage
                .get_setting(key)
                .await?
                .and_then(|v| v.as_str().map(str::to_string))
                .filter(|s| !s.trim().is_empty()),
        )
    };
    let first = read("onboarding.user_first_name").await?;
    let last = read("onboarding.user_last_name").await?;

    let display = match (first.as_deref(), last.as_deref()) {
        (None, None) => return Ok(vec![]),
        (Some(f), Some(l)) => format!("{f} {l}"),
        (Some(f), None) => f.to_string(),
        (None, Some(l)) => l.to_string(),
    };
    Ok(vec![serde_json::json!({
        "display_name": display,
        "first_name": first,
        "is_self": true,
    })])
}

/// Runs one conversation's extraction turn end-to-end: reads
/// `transcript.json`, asks the worker for a validated extraction payload,
/// then persists in this write order — `write_extraction ->
/// write_summary -> replace_extraction_rows -> set_pipeline_step` (files
/// first, single SQLite commit last).
pub async fn extract_conversation(
    storage: &dyn StorageService,
    worker: &WorkerSupervisor,
    conv_id: &str,
    force_overwrite: bool,
) -> Result<ExtractionOutcome, AppError> {
    let transcript_path = paths::transcript_json_path(conv_id)?;
    let transcript = read_json(&transcript_path)?;
    let duration_s = transcript
        .get("duration_ms")
        .and_then(Value::as_i64)
        .map(|ms| ms / 1000);
    let conversation_meta = serde_json::json!({ "duration_s": duration_s });

    let resp: ExtractMemoryResponse = worker
        .send(ExtractMemory {
            conversation_id: conv_id.to_string(),
            transcript,
            // v1 has no contacts table, but it does know one person: the
            // user. See `self_contact`.
            contacts: self_contact(storage).await?,
            notes: None,
            conversation_meta,
        })
        .await?;

    // extraction.json preserves the wire shape verbatim (hint
    // field names, not the internal `action_items`/`decisions` column
    // names) since it's a standalone artifact Conversation Detail reads
    // directly.
    let extraction_json = serde_json::to_value(&resp)
        .map_err(|e| AppError::storage(format!("serialise extraction.json: {e}")))?;
    storage.write_extraction(conv_id, &extraction_json).await?;

    // Only claim the title while it's still the placeholder a recording
    // starts with — a title already changed (by the user, or by an earlier
    // extraction pass) is left alone, the same "don't clobber" stance
    // `summary_is_user_edited` takes for the summary below. `force_overwrite`
    // doesn't reach this: it exists for the summary's file-mtime heuristic,
    // which can go stale; the title has no such heuristic to go stale.
    let title = resp.title.trim();
    if !title.is_empty() {
        let conversation = storage.get_conversation(conv_id).await?;
        if conversation.title == DEFAULT_CONVERSATION_TITLE {
            storage.update_conversation_title(conv_id, title).await?;
        }
    }

    let user_edited = !force_overwrite && summary_is_user_edited(conv_id)?;
    let summary_written = if user_edited {
        false
    } else {
        storage
            .write_summary(conv_id, &resp.summary_markdown)
            .await?;
        true
    };

    let counts = (
        resp.action_items.len(),
        resp.decisions.len(),
        resp.open_questions.len(),
        resp.bookmarks.len(),
    );
    let bundle = ExtractionBundle {
        action_items: resp.action_items.into_iter().map(Into::into).collect(),
        decisions: resp.decisions.into_iter().map(Into::into).collect(),
        open_questions: resp.open_questions.into_iter().map(Into::into).collect(),
        bookmarks: resp.bookmarks.into_iter().map(Into::into).collect(),
    };
    storage.replace_extraction_rows(conv_id, bundle).await?;

    storage
        .set_pipeline_step(conv_id, PipelineStep::Extracting, None)
        .await?;

    Ok(ExtractionOutcome {
        action_items: counts.0,
        decisions: counts.1,
        open_questions: counts.2,
        bookmarks: counts.3,
        summary_written,
    })
}

#[derive(Debug, Clone)]
pub struct RefreshOutcome {
    pub significant_change: bool,
    pub diff_ratio: f64,
    pub snapshot_path: Option<std::path::PathBuf>,
}

/// Days-since-epoch -> (year, month, day), Howard Hinnant's `civil_from_days`
/// (public-domain algorithm; no `chrono`/`time` dependency was already in
/// the tree, and one bool of date math doesn't justify adding one).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// `YYYY-MM-DDTHH-MM-SS-mmmZ` (dashes, not colons — filesystem-safe) for
/// `project_memory_history/` snapshot filenames. Includes a millisecond
/// field because two refreshes for the same project landing in the same
/// wall-clock second — easy to hit in a fast test, and not impossible from a
/// manual refresh racing an auto-refresh in production — would otherwise
/// silently collide on the same filename and one snapshot would clobber the
/// other. Lexical sort, which `prune_history` relies on, still equals
/// chronological order with the extra field.
fn iso_utc_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch");
    let secs = now.as_secs() as i64;
    let millis = now.subsec_millis();
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let (h, mi, s) = (sod / 3600, (sod % 3600) / 60, sod % 60);
    format!("{y:04}-{m:02}-{d:02}T{h:02}-{mi:02}-{s:02}-{millis:03}Z")
}

/// Runs one project's memory refresh: assembles the current
/// document + the newly-added conversations' extractions, asks the worker
/// for an updated document (the worker owns the modify-not-rewrite prompt
/// contract and computes the diff-guardrail ratio itself), then
/// snapshots the prior document *before* overwriting it and prunes
/// the history directory to the newest 10.
pub async fn refresh_project(
    storage: &dyn StorageService,
    worker: &WorkerSupervisor,
    project_id: &str,
    since_conversation_ids: &[String],
    project_name: &str,
) -> Result<RefreshOutcome, AppError> {
    let memory_path = paths::project_memory_path(project_id)?;
    let current_memory = read_json_or_none(&memory_path)?;

    let mut new_extractions = Vec::with_capacity(since_conversation_ids.len());
    for conv_id in since_conversation_ids {
        let extraction_path = paths::extraction_json_path(conv_id)?;
        if let Some(extraction) = read_json_or_none(&extraction_path)? {
            new_extractions.push(serde_json::json!({
                "conv_id": conv_id,
                "extraction": extraction,
            }));
        }
    }

    let resp = worker
        .send(RefreshProjectMemory {
            project_id: project_id.to_string(),
            current_memory: current_memory.clone(),
            new_extractions,
            // The refresh prompt writes prose *about the user's project*, so
            // it needs to know who the user is for the same reason the
            // extraction prompt does — otherwise it writes around them in the
            // third person or invents a name for them.
            project_meta: serde_json::json!({
                "name": project_name,
                "user": self_contact(storage).await?.first().cloned(),
            }),
        })
        .await?;

    // Snapshot-before-overwrite (§5.5 step 1) — only if a document already
    // existed; a brand-new project has nothing to protect.
    let snapshot_path = if let Some(current) = &current_memory {
        let path = paths::project_memory_history_path(project_id, &iso_utc_now())?;
        atomic::atomic_write_json(&path, current)?;
        Some(path)
    } else {
        None
    };

    let now = crate::db::models::unix_now();
    let new_doc = serde_json::json!({
        "overview_markdown": resp.overview_markdown,
        "scope_drift_markdown": resp.scope_drift_markdown,
        "supersessions": resp.supersessions,
        "last_refresh_at": now,
        "last_refresh_runner": "claude",
    });
    storage.write_project_memory(project_id, &new_doc).await?;

    prune_history(project_id, 10)?;

    Ok(RefreshOutcome {
        significant_change: resp.significant_change,
        diff_ratio: resp.diff_ratio,
        snapshot_path,
    })
}

/// The auto-refresh trigger policy's settings key,
/// `project_memory.pending_count.<project_id>` — stored
/// as the array of pending conversation ids (not a bare int) so the same
/// read also hands back `since_conversation_ids` for the refresh call.
fn pending_count_key(project_id: &str) -> String {
    format!("project_memory.pending_count.{project_id}")
}

const REFRESH_EVERY_N_SETTING: &str = "memory.refresh_every_n_conversations";

/// Re-synthesizing the whole project document after every single
/// conversation would be one LLM call per recording, forever, to
/// rewrite two paragraphs that usually do not change — and it would make
/// "there are conversations pending" a state the user should never see,
/// because it lasted seconds. Batching three makes a non-empty pending list
/// *normal*, which is why the UI's staleness banner keys off a failed
/// refresh rather than off a non-empty list.
const DEFAULT_REFRESH_EVERY_N: i64 = 3;

/// A pending list this long means refreshes have been failing for a while.
/// The list is kept on failure (deliberately — see `maybe_auto_refresh`), so
/// without a cap a persistently-broken worker grows one settings row without
/// bound and eventually sends an enormous batch to the model the first time
/// it recovers. Oldest entries are dropped first: the newest conversations
/// are the ones whose content the overview most needs.
const MAX_PENDING: usize = 50;

/// Set when a refresh fails, cleared when one succeeds. This is what the
/// Project page reads to decide whether to tell the user their overview is
/// out of date — the `project-memory-refresh-failed` event announces the
/// moment of failure, but an event cannot answer "is it still stale?" three
/// days later.
fn last_error_key(project_id: &str) -> String {
    format!("project_memory.last_error.{project_id}")
}

pub async fn read_last_error(
    storage: &dyn StorageService,
    project_id: &str,
) -> Result<Option<String>, AppError> {
    Ok(storage
        .get_setting(&last_error_key(project_id))
        .await?
        .and_then(|v| v.as_str().map(str::to_string)))
}

pub async fn set_last_error(
    storage: &dyn StorageService,
    project_id: &str,
    error: Option<&str>,
) -> Result<(), AppError> {
    storage
        .set_setting(
            &last_error_key(project_id),
            match error {
                Some(e) => serde_json::json!(e),
                None => serde_json::Value::Null,
            },
        )
        .await
}

pub async fn read_pending(
    storage: &dyn StorageService,
    project_id: &str,
) -> Result<Vec<String>, AppError> {
    Ok(storage
        .get_setting(&pending_count_key(project_id))
        .await?
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

pub async fn clear_pending(storage: &dyn StorageService, project_id: &str) -> Result<(), AppError> {
    storage
        .set_setting(&pending_count_key(project_id), serde_json::json!([]))
        .await
}

/// The batch threshold is deliberately bypassed for the *first*
/// document. Batching (N=3, see `DEFAULT_REFRESH_EVERY_N`) exists to avoid
/// a full re-synthesis per recording just to rewrite paragraphs that
/// usually barely move — but that
/// trade-off assumes there is already a document worth preserving. With no
/// memory at all the alternative isn't "slightly stale", it's "completely
/// empty", and a project that shows nothing until its third conversation
/// reads as broken. It is worst in the case that surfaced this: moving an
/// existing conversation into a brand-new project, where the user has just
/// explicitly asserted the two belong together and gets an empty Overview
/// telling them memory "fills in once the first conversation finishes
/// processing" — which had already happened. This also makes that copy
/// literally true rather than off by two.
fn should_refresh_now(has_memory: bool, pending_len: usize, threshold: usize) -> bool {
    !has_memory || pending_len >= threshold
}

/// How many conversations accumulate before the project document is
/// re-synthesized. `pub` because the UI surfaces this number: a user who
/// records a meeting and doesn't see it reflected in the Overview needs to
/// be told the cadence, or the staleness reads as a bug.
pub async fn refresh_threshold(storage: &dyn StorageService) -> Result<usize, AppError> {
    Ok(storage
        .get_setting(REFRESH_EVERY_N_SETTING)
        .await?
        .and_then(|v| v.as_i64())
        .unwrap_or(DEFAULT_REFRESH_EVERY_N)
        .max(1) as usize)
}

/// `hex(sha256(sorted(conv_id).join("\n")))`.
pub fn batch_signature(conversation_ids: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut sorted = conversation_ids.to_vec();
    sorted.sort();
    let mut hasher = Sha256::new();
    hasher.update(sorted.join("\n").as_bytes());
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Called after a conversation's extraction commits, since auto-refresh
/// happens after a conversation reaches pipeline `done`. Appends
/// `conv_id` to the project's pending list, and — once the list reaches
/// `memory.refresh_every_n_conversations` (setting; default 3) — runs the
/// refresh immediately and clears the list. On a refresh failure the pending
/// list is left intact (not cleared) so the next trigger — auto or manual —
/// retries with the same batch instead of losing it.
pub async fn maybe_auto_refresh(
    storage: &dyn StorageService,
    worker: &WorkerSupervisor,
    metrics: &crate::metrics::Metrics,
    project_id: &str,
    project_name: &str,
    conv_id: &str,
) -> Result<Option<RefreshOutcome>, AppError> {
    let mut pending = read_pending(storage, project_id).await?;
    if !pending.iter().any(|id| id == conv_id) {
        pending.push(conv_id.to_string());
    }
    if pending.len() > MAX_PENDING {
        let overflow = pending.len() - MAX_PENDING;
        tracing::warn!(
            project_id,
            dropped = overflow,
            "memory.pending_backlog_truncated"
        );
        pending.drain(..overflow);
    }
    storage
        .set_setting(&pending_count_key(project_id), serde_json::json!(pending))
        .await?;

    let threshold = refresh_threshold(storage).await?;
    let has_memory = storage.read_project_memory(project_id).await?.is_some();
    if !should_refresh_now(has_memory, pending.len(), threshold) {
        return Ok(None);
    }

    let refresh_start = std::time::Instant::now();
    match refresh_project(storage, worker, project_id, &pending, project_name).await {
        Ok(outcome) => {
            storage
                .set_setting(&pending_count_key(project_id), serde_json::json!([]))
                .await?;
            set_last_error(storage, project_id, None).await?;
            track_refresh_completed(metrics, "auto", &outcome, refresh_start.elapsed());
            Ok(Some(outcome))
        }
        Err(err) => {
            // The pending list is deliberately NOT cleared, so the next
            // trigger retries the same batch. Recording *why* is the new
            // part: without it the only trace of a failure was a log line
            // and a transient event, and the page went on rendering a stale
            // overview as though it were current.
            set_last_error(storage, project_id, Some(&err.to_string())).await?;
            Err(err)
        }
    }
}

/// §5.5 step 3: keep only the newest `keep` history snapshots (ISO-lexical
/// filename sort — a correct proxy for chronological order at this
/// timestamp format).
fn prune_history(project_id: &str, keep: usize) -> Result<(), AppError> {
    let dir = paths::project_memory_history_dir(project_id)?;
    if !dir.exists() {
        return Ok(());
    }
    let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| AppError::storage(format!("read {}: {e}", dir.display())))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    entries.sort();
    if entries.len() <= keep {
        return Ok(());
    }
    for stale in &entries[..entries.len() - keep] {
        let _ = std::fs::remove_file(stale);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_document_is_generated_without_waiting_for_the_batch() {
        // The reported case: one conversation moved into a brand-new
        // project. Before this, the project sat visibly empty until two
        // more conversations arrived.
        assert!(should_refresh_now(false, 1, 3));
    }

    #[test]
    fn an_existing_document_still_waits_for_the_full_batch() {
        // Batching's actual purpose — don't re-synthesize per recording.
        assert!(!should_refresh_now(true, 1, 3));
        assert!(!should_refresh_now(true, 2, 3));
        assert!(should_refresh_now(true, 3, 3));
    }

    #[test]
    fn empty_pending_list_still_refreshes_when_there_is_no_document_yet() {
        // Reachable via the manual refresh path, which does not append.
        // Harmless: `refresh_project` treats a missing `extraction.json` as
        // "nothing to add" rather than an error.
        assert!(should_refresh_now(false, 0, 3));
    }
}
