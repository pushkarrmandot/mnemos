//! `mnemos-mcp-server` binary discovery (moved out of `claude::spawn` — chat
//! backend design doc §2.1). This is Mnemos' own MCP server, not a vendor
//! one: every chat runner that wants MCP tools (Project/Everything scope)
//! points `--mcp-config` at the same binary and the same server name, so
//! this belongs to the runner layer as a whole, not to any one adapter.
//! Living inside `claude/spawn.rs` was a real coupling that would have made
//! `commands::chat` reach into the `claude` module for every future runner
//! too — see the design doc's §2.1 correction.

use std::path::PathBuf;

use crate::error::AppError;

/// Name the `mnemos-mcp-server` binary is registered under in the `mcp.json`
/// any runner writes — also what `--allowedTools` pre-approves
/// (`mcp__mnemos`).
pub const MCP_SERVER_NAME: &str = "mnemos";

/// The second `[[bin]]` target shipped in the same package as this one —
/// not a separate crate. No production
/// sidecar-bundler packaging exists yet (same gap `lib.rs::worker_config`
/// already flags for the Python worker/Swift sidecar), so dev and prod both
/// resolve it the same way: a sibling of this process's own executable —
/// `cargo` already places `mnemos_mcp_server` next to `mnemos-tauri` in
/// `target/{debug,release}/`, and a future sidecar bundler would place it
/// next to the bundled app binary too.
///
/// The on-disk filename is underscored (`mnemos_mcp_server`), not hyphenated
/// like the source directory (`src/bin/mnemos-mcp-server/`) — the `[[bin]]`
/// target in `Cargo.toml` is named with an underscore on purpose, to match
/// the PDB filename rustc derives from the crate name, which the Tauri NSIS
/// bundler also keys off of. See the comment on that `[[bin]]` entry.
pub fn find_mcp_server_binary(configured_path: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = configured_path {
        let path = PathBuf::from(p);
        return path.is_file().then_some(path);
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let name = if cfg!(windows) {
        "mnemos_mcp_server.exe"
    } else {
        "mnemos_mcp_server"
    };
    let candidate = dir.join(name);
    candidate.is_file().then_some(candidate)
}

pub fn mcp_server_missing_error() -> AppError {
    AppError::WorkerUnavailable { retry_after_ms: 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_mcp_server_binary_uses_configured_path_override() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("my-mcp-server");
        std::fs::write(&bin, b"").unwrap();

        let found = find_mcp_server_binary(Some(bin.to_str().unwrap()));
        assert_eq!(found, Some(bin));
    }

    #[test]
    fn find_mcp_server_binary_none_for_a_bogus_override() {
        assert_eq!(
            find_mcp_server_binary(Some("/definitely/not/a/real/path/mnemos-mcp-server")),
            None
        );
    }

    #[test]
    fn find_mcp_server_binary_none_resolves_next_to_the_current_process() {
        // Exercises the actual sibling-of-`current_exe()` resolution path
        // (`None` — no override), not just the override branch: drops a
        // fake binary next to the real `cargo test` harness executable
        // (guarded by `claude`'s PATH-mutating-test lock, since this also
        // mutates shared process-wide filesystem state next to a
        // well-known location) and cleans it up unconditionally afterward.
        let _guard = crate::ipc::runner::claude::path_env_test_lock().blocking_lock();
        let exe = std::env::current_exe().unwrap();
        let dir = exe.parent().unwrap().to_path_buf();
        let name = if cfg!(windows) {
            "mnemos_mcp_server.exe"
        } else {
            "mnemos_mcp_server"
        };
        let candidate = dir.join(name);
        let already_present = candidate.is_file();
        if !already_present {
            std::fs::write(&candidate, "#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o755))
                    .unwrap();
            }
        }
        let found = find_mcp_server_binary(None);
        if !already_present {
            let _ = std::fs::remove_file(&candidate);
        }
        assert_eq!(found, Some(candidate));
    }
}
