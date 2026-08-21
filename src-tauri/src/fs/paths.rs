//! Resolves everything under `~/Mnemos/`. The only place that knows the layout
//! (HLD §5.2). W4 extends this with ID-based conversation/project resolution.

use std::path::PathBuf;
use std::sync::OnceLock;

use regex_lite::Regex;

use crate::error::AppError;

/// `~/Mnemos` — the app's single data root.
pub fn data_root() -> Result<PathBuf, AppError> {
    let home = home_dir().ok_or_else(|| AppError::internal("no home directory"))?;
    Ok(home.join("Mnemos"))
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

/// `~/Mnemos/projects`
pub fn projects_root() -> Result<PathBuf, AppError> {
    Ok(data_root()?.join("projects"))
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

/// `~/Mnemos/projects/<projectId>/project_memory_history/<iso>.json`
pub fn project_memory_history_path(project_id: &str, iso_ts: &str) -> Result<PathBuf, AppError> {
    Ok(project_dir(project_id)?
        .join("project_memory_history")
        .join(format!("{iso_ts}.json")))
}

/// `~/Mnemos/projects/<projectId>/conversations/<conversationId>/`
pub fn conversation_dir(project_id: &str, conversation_id: &str) -> Result<PathBuf, AppError> {
    validate_uuid(conversation_id)?;
    Ok(project_dir(project_id)?
        .join("conversations")
        .join(conversation_id))
}

/// `.../conversations/<conversationId>/transcript.json`
pub fn transcript_json_path(project_id: &str, conversation_id: &str) -> Result<PathBuf, AppError> {
    Ok(conversation_dir(project_id, conversation_id)?.join("transcript.json"))
}

/// `.../conversations/<conversationId>/transcript.jsonl` — append-only live buffer,
/// per LLD-01 §6.2. Retained until the pipeline reaches `done`, then deleted.
pub fn transcript_jsonl_path(project_id: &str, conversation_id: &str) -> Result<PathBuf, AppError> {
    Ok(conversation_dir(project_id, conversation_id)?.join("transcript.jsonl"))
}

/// `.../conversations/<conversationId>/extraction.json`
pub fn extraction_json_path(project_id: &str, conversation_id: &str) -> Result<PathBuf, AppError> {
    Ok(conversation_dir(project_id, conversation_id)?.join("extraction.json"))
}

/// `.../conversations/<conversationId>/summary.md`
pub fn summary_md_path(project_id: &str, conversation_id: &str) -> Result<PathBuf, AppError> {
    Ok(conversation_dir(project_id, conversation_id)?.join("summary.md"))
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    let var = "HOME";
    #[cfg(windows)]
    let var = "USERPROFILE";

    std::env::var_os(var)
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
    fn conversation_dir_rejects_traversal_in_either_id() {
        let good = "550e8400-e29b-41d4-a716-446655440000";
        assert!(conversation_dir("../../etc", good).is_err());
        assert!(conversation_dir(good, "../../etc").is_err());
    }
}
