//! Binary discovery + argv construction for the `claude` CLI subprocess
//! (LLD-07 §4.2).
//!
//! Flag names were PROVISIONAL in the LLD; this wave verified the ones this
//! module uses against a real `claude` 2.1.239 install (`claude --help`
//! plus several live invocations — see LLD_07_AGENT_RUNNER.md's
//! "Implementation status" for exactly what was confirmed). Two corrections
//! versus the LLD's original sketch:
//!
//! - `--verbose` is a **hard requirement** alongside `-p --output-format
//!   stream-json` (`claude` refuses to start without it: "When using
//!   --print, --output-format=stream-json requires --verbose").
//! - `--permission-mode`'s documented choices in `--help` are `acceptEdits
//!   | auto | bypassPermissions | manual | dontAsk | plan` — no `default`
//!   is listed, yet `default` is what the CLI reports back in its own
//!   `system.init` frame when no flag is passed, and passing
//!   `--permission-mode default` explicitly is accepted. `default` is what
//!   we always pass.
//!
//! **W13a addition, verified against a real `claude` 2.1.240 install with a
//! real `mnemos-mcp-server` child + `--mcp-config`:** none of
//! `--permission-mode default`, `acceptEdits`, or `dontAsk` let an MCP tool
//! call through without an interactive approval prompt — every one of them
//! produced a `permission_denied` system frame and a denied `tool_result`
//! on a first-party MCP server with `readOnlyHint: true` tools. The
//! mechanism that actually works is `--allowedTools "mcp__<server-name>"`
//! (pre-approves every tool namespaced under that MCP server by name,
//! independent of `--permission-mode`), combined with `--tools ""` (empties
//! the CLI's *built-in* tool set — Bash/Read/Write/WebSearch — so the model
//! has no tool surface except the MCP tools we explicitly allow; verified
//! it still refuses a same-turn "also run `whoami`" instruction). This is
//! why v1 needs no approval-flow UI: every v1 MCP tool is read-only
//! (LLD-08 §3), so pre-approving the whole `mnemos` server at spawn time is
//! safe and `--permission-mode` never has to move off the safe `default`
//! LLD-07 §6.3 already warns never to leave (`bypassPermissions` is never
//! passed, chat or extraction).

use std::path::PathBuf;

use crate::error::AppError;
use crate::ipc::runner::mcp_shared::MCP_SERVER_NAME;

/// Manual PATH scan — one binary lookup does not justify the `which` crate
/// dependency. `configured_path` is the (not-yet-built) Settings override
/// from LLD-07 §10 OQ3; `None` in v1 since no such Settings surface exists.
///
/// Windows parity audit finding #18: extension candidates are now
/// `PATHEXT`-aware (standard Windows executable-search semantics) instead
/// of the fixed `.exe`/`.cmd` list — a `.ps1`/`.bat`/other shimmed `claude`
/// install (e.g. from a package manager whose shim isn't one of those two)
/// was previously missed outright.
///
/// **Unresolved risk, not fixed here (finding #18):** if the resolved
/// binary is a `.cmd` shim, spawning it via `std::process::Command` goes
/// through `cmd.exe`'s batch-argument escaping, which — since the Rust
/// 1.77 CVE fix — returns `InvalidInput` for arguments it can't safely
/// escape. `build_argv` below can pass a full multi-line `--system-prompt`
/// argument, which is exactly the shape that can trip this. No workaround
/// is attempted: manually re-implementing cmd.exe quoting is a known
/// injection-bug source and explicitly out of scope for this fix (see this
/// wave's brief). This module's argv surface has no `--system-prompt-file`/
/// stdin alternative today — `build_argv` always passes `--system-prompt`
/// as a literal argument when `opts.system_prompt` is `Some` — so if this
/// is ever hit in practice, verify against a real `claude.cmd` on real
/// Windows before choosing a fix (add a file/stdin-based flag to the CLI's
/// actual argument surface if one exists, rather than hand-rolling escaping
/// here).
pub fn find_claude_binary(configured_path: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = configured_path {
        let path = PathBuf::from(p);
        return path.is_file().then_some(path);
    }

    let path_var = std::env::var_os("PATH")?;
    let candidates: Vec<String> = if cfg!(windows) {
        windows_candidates("claude")
    } else {
        vec!["claude".to_string()]
    };
    for dir in std::env::split_paths(&path_var) {
        for candidate in &candidates {
            let full = dir.join(candidate);
            if full.is_file() {
                return Some(full);
            }
        }
    }
    None
}

/// `PATHEXT`-aware candidate list for `stem` on Windows: `stem` itself
/// (covers an already-extensioned name, and matches `is_file()` even with
/// no extension) followed by `stem<ext>` for each `PATHEXT` entry in order —
/// standard Windows executable-search semantics (`cmd.exe`/`CreateProcess`'s
/// own resolution order). Falls back to a sane default list when `PATHEXT`
/// is unset (e.g. a stripped-down spawn environment) rather than silently
/// searching nothing beyond `.exe`.
#[cfg(windows)]
fn windows_candidates(stem: &str) -> Vec<String> {
    const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| DEFAULT_PATHEXT.to_string());
    let mut out = vec![stem.to_string()];
    for ext in pathext.split(';') {
        let ext = ext.trim();
        if ext.is_empty() {
            continue;
        }
        let ext = if let Some(stripped) = ext.strip_prefix('.') {
            stripped
        } else {
            ext
        };
        out.push(format!("{stem}.{ext}"));
    }
    out
}

#[cfg(not(windows))]
fn windows_candidates(stem: &str) -> Vec<String> {
    vec![stem.to_string()]
}

pub fn binary_missing_error() -> AppError {
    AppError::WorkerUnavailable { retry_after_ms: 0 }
}

// `MCP_SERVER_NAME`/`find_mcp_server_binary`/`mcp_server_missing_error` moved
// to `ipc::runner::mcp_shared` (design doc §2.1 correction) — they were
// never Claude-specific, just placed here first since Claude was the only
// consumer. Re-imported above.

pub struct ArgvOptions<'a> {
    pub model: &'a str,
    pub permission_mode: &'a str,
    pub system_prompt: Option<&'a str>,
    pub session_id: &'a str,
    /// `Some(path)` writes `--mcp-config <path> --strict-mcp-config
    /// --allowedTools "mcp__mnemos"` (Project/Everything-scope chat).
    /// `None` means no MCP config for this run (extraction,
    /// Conversation-scope chat — context is stuffed into `system_prompt`
    /// instead). Either way `--tools ""` is always passed — this runner
    /// never gives the model the CLI's general-purpose built-in tools
    /// (Bash/Read/Write/WebSearch), only the `mnemos` MCP surface when one
    /// is configured.
    pub mcp_config_path: Option<&'a str>,
}

/// Builds the argv (excluding argv[0]).
///
/// Shape is identical whether the resulting runner is used as a long-lived
/// chat session or an ephemeral extraction call (LLD-07 §5's two lifecycle
/// patterns): `--session-id` is always generated and passed. It's a no-op
/// for extraction (nothing ever resumes it) and is what lets a chat runner
/// be resumed by a fresh spawn after a crash (LLD-07 §5.1).
pub fn build_argv(opts: &ArgvOptions<'_>) -> Vec<String> {
    let mut argv = vec![
        "-p".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--input-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--model".to_string(),
        opts.model.to_string(),
        "--permission-mode".to_string(),
        opts.permission_mode.to_string(),
        "--session-id".to_string(),
        opts.session_id.to_string(),
    ];
    if let Some(sp) = opts.system_prompt {
        argv.push("--system-prompt".to_string());
        argv.push(sp.to_string());
    }
    // Never give the model the CLI's built-in tools — only the `mnemos` MCP
    // surface, and only when `mcp_config_path` is set (see this module's
    // "W13a addition" doc comment for why this needs no approval-flow UI).
    argv.push("--tools".to_string());
    argv.push(String::new());
    if let Some(mcp_config_path) = opts.mcp_config_path {
        argv.push("--mcp-config".to_string());
        argv.push(mcp_config_path.to_string());
        argv.push("--strict-mcp-config".to_string());
        argv.push("--allowedTools".to_string());
        argv.push(format!("mcp__{MCP_SERVER_NAME}"));
    }
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_includes_the_hard_requirements() {
        let argv = build_argv(&ArgvOptions {
            model: "claude-sonnet-5",
            permission_mode: "default",
            system_prompt: None,
            session_id: "abc-123",
            mcp_config_path: None,
        });
        // `--verbose` is required alongside `-p --output-format
        // stream-json` (verified against a real `claude` 2.1.239 — see
        // module doc comment); a regression here would silently break
        // every spawn.
        assert!(argv
            .windows(2)
            .any(|w| w == ["--output-format", "stream-json"]));
        assert!(argv.contains(&"--verbose".to_string()));
        assert!(argv.contains(&"-p".to_string()));
        assert!(argv
            .windows(2)
            .any(|w| w == ["--input-format", "stream-json"]));
        assert!(argv.windows(2).any(|w| w == ["--session-id", "abc-123"]));
        assert!(argv.windows(2).any(|w| w == ["--model", "claude-sonnet-5"]));
        assert!(argv
            .windows(2)
            .any(|w| w == ["--permission-mode", "default"]));
        // No MCP config passed -> built-in tools are still emptied, but no
        // `--mcp-config`/`--allowedTools` (extraction, Conversation-scope
        // chat never configure MCP — see this module's doc comment).
        assert!(argv.windows(2).any(|w| w == ["--tools", ""]));
        assert!(!argv.contains(&"--mcp-config".to_string()));
        assert!(!argv.contains(&"--allowedTools".to_string()));
    }

    #[test]
    fn argv_appends_system_prompt_when_present() {
        let argv = build_argv(&ArgvOptions {
            model: "claude-sonnet-5",
            permission_mode: "default",
            system_prompt: Some("You output ONLY JSON."),
            session_id: "abc-123",
            mcp_config_path: None,
        });
        assert!(argv
            .windows(2)
            .any(|w| w == ["--system-prompt", "You output ONLY JSON."]));
    }

    #[test]
    fn argv_pre_approves_the_mnemos_mcp_server_when_configured() {
        let argv = build_argv(&ArgvOptions {
            model: "claude-sonnet-5",
            permission_mode: "default",
            system_prompt: None,
            session_id: "abc-123",
            mcp_config_path: Some("/tmp/mcp.json"),
        });
        assert!(argv
            .windows(2)
            .any(|w| w == ["--mcp-config", "/tmp/mcp.json"]));
        assert!(argv.contains(&"--strict-mcp-config".to_string()));
        assert!(argv
            .windows(2)
            .any(|w| w == ["--allowedTools", "mcp__mnemos"]));
        // Verified against a real `claude` 2.1.240 install this wave:
        // `--permission-mode` alone (default/acceptEdits/dontAsk) still
        // denies MCP tool calls; `--allowedTools "mcp__<server>"` is the
        // mechanism that actually pre-approves them (see module doc
        // comment) — never `bypassPermissions`.
        assert!(!argv.contains(&"bypassPermissions".to_string()));
    }

    #[test]
    fn find_claude_binary_none_when_not_on_path() {
        let _guard = super::super::path_env_test_lock().blocking_lock();
        let dir = tempfile::tempdir().unwrap();
        let old = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", dir.path()) };
        let found = find_claude_binary(None);
        if let Some(old) = old {
            unsafe { std::env::set_var("PATH", old) };
        }
        assert!(found.is_none());
    }

    #[test]
    fn find_claude_binary_hits_a_matching_file_on_path() {
        let _guard = super::super::path_env_test_lock().blocking_lock();
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("claude");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let old = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", dir.path()) };
        let found = find_claude_binary(None);
        if let Some(old) = old {
            unsafe { std::env::set_var("PATH", old) };
        }
        assert_eq!(found, Some(bin));
    }

    #[test]
    fn find_claude_binary_uses_configured_path_override() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("my-claude");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        let found = find_claude_binary(Some(bin.to_str().unwrap()));
        assert_eq!(found, Some(bin));
    }

    // `find_mcp_server_binary_*` tests moved to `ipc::runner::mcp_shared`
    // with the functions themselves (design doc §2.1).
}
