//! Resolves everything under `~/Mnemos/`. The only place that knows the layout
//! (HLD §5.2). W4 extends this with ID-based conversation/project resolution.

use std::path::PathBuf;
use std::sync::OnceLock;

use regex_lite::Regex;

use crate::error::AppError;

/// `~/Mnemos` on macOS/Linux, `%LOCALAPPDATA%\Mnemos` on Windows — the app's
/// single data root, overridable via `$MNEMOS_HOME` (LLD-08 §4's
/// `--data-dir` sketch: "defaults to `$MNEMOS_HOME` env var, then
/// `~/Mnemos/`"). The main app never sets this; `mnemos-mcp-server`'s
/// `--data-dir` flag does, by setting the env var once at startup before
/// any path is resolved — letting both binaries share one path resolver
/// and letting integration tests point either one at a temp directory
/// without touching the real data root.
///
/// Windows uses `%LOCALAPPDATA%` rather than `%USERPROFILE%` (Windows parity
/// audit finding #20): on machines with OneDrive Known-Folder-Redirection
/// enabled for the profile root — common on managed corporate Windows — a
/// `%USERPROFILE%\Mnemos` data root would sit inside a cloud-synced,
/// Files-On-Demand folder, and OneDrive's placeholder/locking behavior is a
/// known SQLite corruption risk. `%LOCALAPPDATA%` is never
/// redirection-synced by OneDrive and is the conventional home for
/// per-machine app state on Windows.
pub fn data_root() -> Result<PathBuf, AppError> {
    if let Some(over) = std::env::var_os("MNEMOS_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(over));
    }
    #[cfg(windows)]
    {
        let local_appdata = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| AppError::internal("no LOCALAPPDATA directory"))?;
        return Ok(local_appdata.join("Mnemos"));
    }
    #[cfg(not(windows))]
    {
        let home = home_dir().ok_or_else(|| AppError::internal("no home directory"))?;
        Ok(home.join("Mnemos"))
    }
}

/// `~/Mnemos/logs`
pub fn logs_dir() -> Result<PathBuf, AppError> {
    Ok(data_root()?.join("logs"))
}

/// `~/Mnemos/mnemos.db`
pub fn db_path() -> Result<PathBuf, AppError> {
    Ok(data_root()?.join("mnemos.db"))
}

/// `~/Mnemos/backups`
pub fn backups_dir() -> Result<PathBuf, AppError> {
    Ok(data_root()?.join("backups"))
}

/// `~/Mnemos/state` — worker manifest + job-queue snapshots (LLD-02 §5.3, §6).
pub fn state_dir() -> Result<PathBuf, AppError> {
    Ok(data_root()?.join("state"))
}

/// `~/Mnemos/state/worker-manifest.json` (LLD-02 §5.3).
pub fn worker_manifest_path() -> Result<PathBuf, AppError> {
    Ok(state_dir()?.join("worker-manifest.json"))
}

/// `~/Mnemos/state/pending_jobs.json` — worker-written, replayed by the
/// supervisor on restart (LLD-02 §6; reserved but unused by LLD-01/W4).
pub fn pending_jobs_path() -> Result<PathBuf, AppError> {
    Ok(state_dir()?.join("pending_jobs.json"))
}

/// `~/Mnemos/state/current_job.json` — the single in-flight job (LLD-02 §6).
pub fn current_job_path() -> Result<PathBuf, AppError> {
    Ok(state_dir()?.join("current_job.json"))
}

/// `~/Mnemos/projects`
pub fn projects_root() -> Result<PathBuf, AppError> {
    Ok(data_root()?.join("projects"))
}

/// `~/Mnemos/recordings` — every conversation's on-disk home, filed or not,
/// keyed by conversation id alone. Project assignment lives purely in
/// `conversations.project_id`; it never affects this path. Through W16 this
/// was project-scoped (`projects/<id>/conversations/<convId>/`), mirroring
/// LanceDB's real need for a per-project directory it can atomically
/// `rm -rf` on project delete (HLD §5.4/v1.3) — but plain audio/transcript
/// blobs have no such requirement, and nesting them anyway meant every
/// project reassignment had to rename a live directory on disk, which
/// silently broke anything that had independently cached the old path (the
/// mac sidecar's live-transcription mic.wav location was one). Flat and
/// DB-owned removes that whole class of bug instead of patching each cache
/// site. LanceDB, when it ships, keeps its own `projects/<id>/lancedb/` —
/// that isolation need is real and stays project-scoped.
pub fn recordings_root() -> Result<PathBuf, AppError> {
    Ok(data_root()?.join("recordings"))
}

/// `~/Mnemos/runtime/mcp` — per-chat-session `mcp.json` files a `ClaudeRunner`
/// writes on `start()` and deletes on `dispose()` (LLD-07 §6.1, W13a).
pub fn mcp_config_dir() -> Result<PathBuf, AppError> {
    Ok(data_root()?.join("runtime").join("mcp"))
}

/// `~/Mnemos/runtime/mcp/<session_id>.json`. `session_id` is the runner's own
/// generated `--session-id` (a UUID), not user input — not run through
/// `validate_uuid` since callers here never see anything else.
pub fn mcp_config_path(session_id: &str) -> Result<PathBuf, AppError> {
    Ok(mcp_config_dir()?.join(format!("{session_id}.json")))
}

/// Rejects anything that is not a canonical UUID (36 chars, hex + dashes) —
/// no `..`, no absolute paths, no separators. Every ID-based path accessor
/// below runs its inputs through this first (LLD-01 §14.1): construction of
/// a path is validation, so nothing downstream needs to re-check.
pub fn validate_uuid(s: &str) -> Result<(), AppError> {
    static UUID_RE: OnceLock<Regex> = OnceLock::new();
    let re = UUID_RE.get_or_init(|| {
        Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
            .expect("static regex is valid")
    });
    if !re.is_match(s) {
        return Err(AppError::Validation {
            message: "invalid uuid".into(),
            field: Some("id".into()),
        });
    }
    Ok(())
}

/// `~/Mnemos/projects/<projectId>/`
pub fn project_dir(project_id: &str) -> Result<PathBuf, AppError> {
    validate_uuid(project_id)?;
    Ok(projects_root()?.join(project_id))
}

/// `~/Mnemos/projects/<projectId>/project_memory.json`
pub fn project_memory_path(project_id: &str) -> Result<PathBuf, AppError> {
    Ok(project_dir(project_id)?.join("project_memory.json"))
}

/// `~/Mnemos/projects/<projectId>/project_memory_history/`
pub fn project_memory_history_dir(project_id: &str) -> Result<PathBuf, AppError> {
    Ok(project_dir(project_id)?.join("project_memory_history"))
}

/// `~/Mnemos/projects/<projectId>/project_memory_history/<iso>.json`
pub fn project_memory_history_path(project_id: &str, iso_ts: &str) -> Result<PathBuf, AppError> {
    Ok(project_memory_history_dir(project_id)?.join(format!("{iso_ts}.json")))
}

/// `~/Mnemos/recordings/<conversationId>/` — see `recordings_root` for why
/// this no longer takes a `project_id`.
pub fn conversation_dir(conversation_id: &str) -> Result<PathBuf, AppError> {
    validate_uuid(conversation_id)?;
    Ok(recordings_root()?.join(conversation_id))
}

/// `.../recordings/<conversationId>/transcript.json`
pub fn transcript_json_path(conversation_id: &str) -> Result<PathBuf, AppError> {
    Ok(conversation_dir(conversation_id)?.join("transcript.json"))
}

/// `.../recordings/<conversationId>/transcript.jsonl` — append-only live buffer,
/// per LLD-01 §6.2. Retained until the pipeline reaches `done`, then deleted.
pub fn transcript_jsonl_path(conversation_id: &str) -> Result<PathBuf, AppError> {
    Ok(conversation_dir(conversation_id)?.join("transcript.jsonl"))
}

/// `.../recordings/<conversationId>/extraction.json`
pub fn extraction_json_path(conversation_id: &str) -> Result<PathBuf, AppError> {
    Ok(conversation_dir(conversation_id)?.join("extraction.json"))
}

/// `.../recordings/<conversationId>/summary.md`
pub fn summary_md_path(conversation_id: &str) -> Result<PathBuf, AppError> {
    Ok(conversation_dir(conversation_id)?.join("summary.md"))
}

/// `.../recordings/<conversationId>/mic.wav` (LLD-03 §4 — 16kHz mono
/// 16-bit PCM, written by the mac sidecar or the Windows capture thread).
pub fn mic_wav_path(conversation_id: &str) -> Result<PathBuf, AppError> {
    Ok(conversation_dir(conversation_id)?.join("mic.wav"))
}

/// `.../recordings/<conversationId>/system.wav`
pub fn system_wav_path(conversation_id: &str) -> Result<PathBuf, AppError> {
    Ok(conversation_dir(conversation_id)?.join("system.wav"))
}

// Only used on macOS/Linux now — Windows resolves its data root from
// `%LOCALAPPDATA%` directly in `data_root()` above (finding #20).
#[cfg(not(windows))]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_dir_sits_under_the_data_root() {
        let logs = logs_dir().expect("home dir must resolve in test env");
        assert!(logs.ends_with("Mnemos/logs"));
    }

    #[test]
    fn validate_uuid_accepts_canonical_form() {
        assert!(validate_uuid("550e8400-e29b-41d4-a716-446655440000").is_ok());
    }

    #[test]
    fn validate_uuid_rejects_traversal_and_malformed_input() {
        for bad in [
            "../../etc/passwd",
            "550e8400-e29b-41d4-a716-446655440000/../x",
            "not-a-uuid",
            "",
            "550e8400e29b41d4a716446655440000",
            "550E8400-E29B-41D4-A716-446655440000", // uppercase rejected
        ] {
            assert!(
                validate_uuid(bad).is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn conversation_dir_rejects_traversal() {
        assert!(conversation_dir("../../etc/passwd").is_err());
    }

    #[test]
    fn conversation_dir_sits_flat_under_recordings_root() {
        let good = "550e8400-e29b-41d4-a716-446655440000";
        let dir = conversation_dir(good).expect("home dir must resolve in test env");
        assert!(dir.ends_with(format!("Mnemos/recordings/{good}")));
    }
}
