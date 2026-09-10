//! Binary discovery + argv construction for the `claude` CLI subprocess.
//!
//! Flag behavior verified against a real `claude` 2.1.239/2.1.240 install
//! (`claude --help` plus several live invocations). Notable findings:
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
//! - Verified against a real `claude` 2.1.240 install with a
//!   real `mnemos-mcp-server` child + `--mcp-config`: none of
//!   `--permission-mode default`, `acceptEdits`, or `dontAsk` let an MCP tool
//!   call through without an interactive approval prompt — every one of them
//!   produced a `permission_denied` system frame and a denied `tool_result`
//!   on a first-party MCP server with `readOnlyHint: true` tools. The
//!   mechanism that actually works is `--allowedTools "mcp__<server-name>"`
//!   (pre-approves every tool namespaced under that MCP server by name,
//!   independent of `--permission-mode`), combined with `--tools ""` (empties
//!   the CLI's *built-in* tool set — Bash/Read/Write/WebSearch — so the model
//!   has no tool surface except the MCP tools we explicitly allow; verified
//!   it still refuses a same-turn "also run `whoami`" instruction). This is
//!   why v1 needs no approval-flow UI: every v1 MCP tool is read-only,
//!   so pre-approving the whole `mnemos` server at spawn time is
//!   safe and `--permission-mode` never has to move off the safe `default`
//!   (`bypassPermissions` is never passed, chat or extraction).

use std::path::PathBuf;

use crate::error::AppError;
use crate::ipc::runner::mcp_shared::MCP_SERVER_NAME;

/// Manual PATH scan — one binary lookup does not justify the `which` crate
/// dependency. `configured_path` is the (not-yet-built) Settings override;
/// `None` in v1 since no such Settings surface exists.
///
/// Windows parity audit finding #18: extension candidates are
/// `PATHEXT`-aware (standard Windows executable-search semantics) instead
/// of a fixed `.exe`/`.cmd` list, so a `.ps1`/`.bat`/other shimmed `claude`
/// install (e.g. from a package manager whose shim isn't one of those two)
/// is found too.
///
/// **Unresolved risk, not fixed here (finding #18):** if the resolved
/// binary is a `.cmd` shim, spawning it via `std::process::Command` goes
/// through `cmd.exe`'s batch-argument escaping, which — since the Rust
/// 1.77 CVE fix — returns `InvalidInput` for arguments it can't safely
/// escape. `build_argv` below can pass a full multi-line `--system-prompt`
/// argument, which is exactly the shape that can trip this. No workaround
/// is attempted: manually re-implementing cmd.exe quoting is a known
/// injection-bug source and explicitly out of scope. This module's argv surface has no `--system-prompt-file`/
/// stdin alternative today — `build_argv` always passes `--system-prompt`
/// as a literal argument when `opts.system_prompt` is `Some` — so if this
/// is ever hit in practice, verify against a real `claude.cmd` on real
/// Windows before choosing a fix (add a file/stdin-based flag to the CLI's
/// actual argument surface if one exists, rather than hand-rolling escaping
/// here).
/// A user-supplied `claude` location, applied process-wide.
///
/// Deliberately global rather than threaded through `RunnerConfig`: this is
/// machine configuration in exactly the way `PATH` is, and the three places
/// that resolve the binary (onboarding detection, chat spawn, extraction
/// spawn) would otherwise each need it plumbed in separately and could
/// disagree. Set once at startup from the `runner.claude_path` setting and
/// again whenever the user changes it.
static CONFIGURED_PATH: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// `None` clears the override and returns resolution to PATH + probing.
pub fn set_configured_claude_path(path: Option<PathBuf>) {
    if let Ok(mut guard) = CONFIGURED_PATH.write() {
        *guard = path;
    }
}

pub fn configured_claude_path() -> Option<PathBuf> {
    CONFIGURED_PATH.read().ok().and_then(|g| g.clone())
}

/// Resolution order: explicit argument, then the user's configured path,
/// then `PATH`, then the locations installers actually use.
///
/// That last step exists because a macOS app launched from Finder does NOT
/// inherit the shell's `PATH` — launchd hands it `/usr/bin:/bin:/usr/sbin:/sbin`,
/// and `.zshrc` never runs. Claude Code installs to `~/.local/bin`, which is
/// not on that list, so a PATH-only scan finds nothing for essentially every
/// end user who double-clicks the app, while working perfectly when the same
/// binary is launched from a terminal.
///
/// Probing is a convenience, never a guarantee: a managed environment can put
/// the CLI somewhere no list will ever contain (an Amazon-issued laptop keeps
/// it in `~/.toolbox/bin`). The configured path is the real answer for those,
/// and the reason this cannot just be a longer list of guesses.
pub fn find_claude_binary(configured_path: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = configured_path {
        let path = PathBuf::from(p);
        return path.is_file().then_some(path);
    }

    if let Some(path) = configured_claude_path() {
        if path.is_file() {
            return Some(path);
        }
    }

    let candidates: Vec<String> = if cfg!(windows) {
        windows_candidates("claude")
    } else {
        vec!["claude".to_string()]
    };

    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for candidate in &candidates {
                let full = dir.join(candidate);
                if full.is_file() {
                    return Some(full);
                }
            }
        }
    }

    if !probe_enabled() {
        return None;
    }

    for dir in common_install_dirs() {
        for candidate in &candidates {
            let full = dir.join(candidate);
            if full.is_file() {
                return Some(full);
            }
        }
    }
    None
}

/// Probing is on in the real app and off inside tests that assert the
/// "nothing installed" path.
///
/// Without this seam those tests cannot express their case at all: they
/// clear `PATH`, but the probe would then find the developer's own real
/// `claude` in `~/.local/bin` or `/opt/homebrew/bin` and the assertion would
/// depend on whose machine ran it. Every test that flips this already holds
/// `path_env_test_lock`, the same lock guarding `PATH` mutation.
#[cfg(test)]
static PROBE_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Turns install-location probing off until the returned guard drops.
///
/// A test that asserts "no runner installed" clears `PATH`, but probing would
/// then find the developer's own `claude` in `~/.local/bin` or
/// `/opt/homebrew/bin` and the assertion would pass or fail depending on
/// whose machine ran it. Restoring on drop rather than by hand means an
/// early return or a panicking assertion cannot leak `false` into whichever
/// test runs next in this process.
#[cfg(test)]
#[must_use = "probing stays disabled only while the guard is alive"]
pub(crate) fn probe_disabled_for_test() -> ProbeGuard {
    PROBE_ENABLED.store(false, std::sync::atomic::Ordering::SeqCst);
    ProbeGuard
}

#[cfg(test)]
pub(crate) struct ProbeGuard;

#[cfg(test)]
impl Drop for ProbeGuard {
    fn drop(&mut self) {
        PROBE_ENABLED.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
fn probe_enabled() -> bool {
    PROBE_ENABLED.load(std::sync::atomic::Ordering::SeqCst)
}

#[cfg(not(test))]
fn probe_enabled() -> bool {
    true
}

/// Where the CLI's own installers put it, for the launchd-PATH case above.
/// Checked in install-method order, most common first.
fn common_install_dirs() -> Vec<PathBuf> {
    // `HOME`/`USERPROFILE` directly rather than pulling in a crate for one
    // lookup — `fs::paths` already resolves the home directory this way.
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty());
    let mut dirs_out: Vec<PathBuf> = Vec::new();
    if let Some(home) = home.as_ref() {
        dirs_out.extend([
            home.join(".local/bin"),    // Claude Code's native installer
            home.join(".claude/local"), // its older local-install layout
            home.join(".bun/bin"),
            home.join(".volta/bin"),
            home.join(".npm-global/bin"),
            home.join("node_modules/.bin"),
        ]);
    }
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            dirs_out.push(PathBuf::from(appdata).join("npm"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs_out.push(PathBuf::from(local).join("Programs"));
        }
    } else {
        dirs_out.extend([
            PathBuf::from("/opt/homebrew/bin"), // Apple Silicon Homebrew
            PathBuf::from("/usr/local/bin"),    // Intel Homebrew, manual installs
        ]);
    }
    dirs_out
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
    /// Either a fresh uuid to *name* a new conversation, or the id of an
    /// existing one to *resume* — `resuming` picks which.
    pub session_id: &'a str,
    /// `--resume <id>` instead of `--session-id <id>`. These are different
    /// flags with different jobs: `--session-id` names a conversation,
    /// `--resume` loads one. Verified against a real CLI — passing
    /// `--session-id` with a previously-used id does *not* rehydrate
    /// anything, which is the bug this flag exists to fix.
    pub resuming: bool,
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
/// chat session or an ephemeral extraction call: `--session-id` is always
/// generated and passed. It's a no-op
/// for extraction (nothing ever resumes it) and is what lets a chat runner
/// be resumed by a fresh spawn after a crash.
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
        if opts.resuming {
            "--resume".to_string()
        } else {
            "--session-id".to_string()
        },
        opts.session_id.to_string(),
    ];
    if let Some(sp) = opts.system_prompt {
        argv.push("--system-prompt".to_string());
        argv.push(sp.to_string());
    }
    // Never give the model the CLI's built-in tools — only the `mnemos` MCP
    // surface, and only when `mcp_config_path` is set (see this module's
    // doc comment for why this needs no approval-flow UI).
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

    /// The resume branch is the whole point of storing
    /// `chat_sessions.runner_session_id`: `--session-id` *names* a
    /// conversation, `--resume` *loads* one. Getting this backwards means
    /// the model silently forgets everything across an app restart while
    /// the UI still shows the full transcript, so it is asserted
    /// explicitly — including that nothing else about the argv changes,
    /// since a resumed turn must still get the same model, tools and MCP
    /// surface as a fresh one.
    #[test]
    fn resuming_swaps_session_id_for_resume_and_changes_nothing_else() {
        let opts = |resuming| ArgvOptions {
            model: "claude-sonnet-5",
            permission_mode: "default",
            system_prompt: Some("sp"),
            session_id: "abc-123",
            resuming,
            mcp_config_path: Some("/tmp/mcp.json"),
        };
        let fresh = build_argv(&opts(false));
        let resumed = build_argv(&opts(true));

        assert!(fresh.windows(2).any(|w| w == ["--session-id", "abc-123"]));
        assert!(!fresh.iter().any(|a| a == "--resume"));
        assert!(resumed.windows(2).any(|w| w == ["--resume", "abc-123"]));
        assert!(!resumed.iter().any(|a| a == "--session-id"));

        // Same length, and identical everywhere except the one flag name.
        assert_eq!(fresh.len(), resumed.len());
        let diffs: Vec<_> = fresh
            .iter()
            .zip(resumed.iter())
            .filter(|(a, b)| a != b)
            .collect();
        assert_eq!(diffs.len(), 1, "only the flag name may differ: {diffs:?}");
    }

    #[test]
    fn argv_includes_the_hard_requirements() {
        let argv = build_argv(&ArgvOptions {
            model: "claude-sonnet-5",
            permission_mode: "default",
            system_prompt: None,
            session_id: "abc-123",
            resuming: false,
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
            resuming: false,
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
            resuming: false,
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
        let _probe_off = crate::ipc::runner::claude::spawn::probe_disabled_for_test();
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

    /// The office-laptop case end to end: a `claude` that is on no `PATH`
    /// and in none of the probed install directories is still resolved,
    /// because the user pointed us at it.
    ///
    /// `find_claude_binary_uses_configured_path_override` covers the
    /// *argument*, which nothing in the app passes. Every real resolution —
    /// onboarding detection, a chat spawn, an extraction job — calls
    /// `find_claude_binary(None)` and depends on the process-wide value that
    /// `runner_set_claude_path` writes. That is the path a managed machine
    /// depends on entirely, and it had no coverage at all.
    #[test]
    fn a_configured_path_resolves_when_nothing_else_would() {
        let _guard = super::super::path_env_test_lock().blocking_lock();
        let _probe_off = crate::ipc::runner::claude::spawn::probe_disabled_for_test();

        // Somewhere no PATH entry and no probe list will ever name — the
        // shape of `~/.toolbox/bin` on a corporate install.
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("claude");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let empty = tempfile::tempdir().unwrap();
        let old = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", empty.path()) };

        // Precondition: without the override this is genuinely unfindable,
        // so a pass below cannot come from PATH or probing.
        assert!(
            find_claude_binary(None).is_none(),
            "precondition: nothing should be resolvable before the override"
        );

        set_configured_claude_path(Some(bin.clone()));
        let found = find_claude_binary(None);

        // A path that no longer exists must not shadow PATH forever.
        set_configured_claude_path(Some(dir.path().join("deleted-since")));
        let after_stale = find_claude_binary(None);

        set_configured_claude_path(None);
        let after_clear = find_claude_binary(None);

        if let Some(old) = old {
            unsafe { std::env::set_var("PATH", old) };
        }

        assert_eq!(found, Some(bin), "the configured path must win");
        assert!(
            after_stale.is_none(),
            "a stale configured path must fall through, not pin"
        );
        assert!(
            after_clear.is_none(),
            "clearing must return to PATH + probing"
        );
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
