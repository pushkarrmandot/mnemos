//! Writes/removes the per-session `mcp.json` a chat `ClaudeRunner` points
//! `--mcp-config` at (LLD-07 §6.1, W13a). Only Project/Everything-scope
//! chat runners ever call this — extraction and Conversation-scope chat
//! pass `RunnerConfig.mcp = None` and never write one.
//!
//! Takes `config_path`/`data_dir` as explicit arguments rather than
//! resolving them itself via `fs::paths` — keeps this module pure (no
//! `$MNEMOS_HOME` env var reads), so its tests need no process-global state
//! that could race every other test in this crate that resolves
//! `fs::paths::data_root()` on its own (`cargo test` runs test fns on
//! multiple threads by default). Callers resolve real paths via
//! `fs::paths::mcp_config_path`/`data_root`.

use std::path::Path;

use crate::error::AppError;

use crate::ipc::runner::mcp_shared::MCP_SERVER_NAME;

/// Writes `config_path` (LLD-07 §6.1's shape) pointing `--mcp-config` at the
/// real `mnemos-mcp-server` binary. No scope args on the command line — see
/// `runner::McpConfig`'s doc comment for why scoping happens via the system
/// prompt instead of a startup flag (LLD-08's actual binary never grew a
/// `--scope-project`/`--scope-conversation` flag; tools take `project_id` as
/// a per-call argument instead).
pub fn write(config_path: &Path, server_binary: &str, data_dir: &Path) -> Result<(), AppError> {
    let config = serde_json::json!({
        "mcpServers": {
            MCP_SERVER_NAME: {
                "command": server_binary,
                "args": ["--data-dir", data_dir.to_string_lossy()],
                "env": {}
            }
        }
    });
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(&config)
        .map_err(|e| AppError::internal(format!("encode mcp.json: {e}")))?;
    std::fs::write(config_path, bytes)?;
    Ok(())
}

/// Best-effort delete on `dispose()` — a leftover file is harmless (it's
/// only ever read by a `--mcp-config` flag naming it explicitly), so a
/// missing-file error is not surfaced.
pub fn remove(config_path: &Path) {
    let _ = std::fs::remove_file(config_path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_valid_mcp_json_and_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("runtime/mcp/sess-1.json");
        let data_dir = dir.path().join("data");

        write(&config_path, "/abs/path/to/mnemos-mcp-server", &data_dir).unwrap();

        assert!(config_path.is_file());
        let raw = std::fs::read_to_string(&config_path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            json["mcpServers"]["mnemos"]["command"],
            "/abs/path/to/mnemos-mcp-server"
        );
        assert_eq!(json["mcpServers"]["mnemos"]["args"][0], "--data-dir");
        assert_eq!(
            json["mcpServers"]["mnemos"]["args"][1],
            data_dir.to_string_lossy().as_ref()
        );

        remove(&config_path);
        assert!(!config_path.is_file());
    }
}
