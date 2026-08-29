//! Runner registry (chat backend design doc §2.1) — the seam a second
//! coding-agent CLI (codex, opencode, gemini, ...) plugs into. Only
//! `Claude` exists today; adding the next one is:
//!
//! 1. A new module `ipc::runner::<name>/` with its own `spawn.rs`
//!    (command-line construction) and `translate.rs` (pure
//!    `fn translate(frame) -> Vec<AgentEvent>`, fixture-replay-tested —
//!    mirror `claude::translate`).
//! 2. One new `RunnerKind` variant here, wired into `id`/`default_chat_model`
//!    /`create`.
//!
//! Nothing in `commands::chat` changes after that: `send_prompt` and
//! `build_runner_config` are already dispatched on `RunnerKind`, not
//! hardcoded to `claude::*` — that hardcoding was the actual blocker (design
//! doc's §2.1 correction), not the registry shape itself, which
//! `AgentRunner` (`ipc/runner.rs`) already supported.

use super::claude::ClaudeRunner;
use super::AgentRunner;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerKind {
    Claude,
}

impl RunnerKind {
    /// The `chat_sessions.runner_id` value this kind resolves/creates
    /// sessions under (`find_chat_session_by_scope`'s first argument).
    pub const fn id(self) -> &'static str {
        match self {
            RunnerKind::Claude => "claude",
        }
    }

    /// This runner's default chat model — each vendor pins its own (LLD-07
    /// §4.4); there is no cross-runner "default model" concept.
    pub const fn default_chat_model(self) -> &'static str {
        match self {
            RunnerKind::Claude => super::claude::MODEL_IDS.chat_default,
        }
    }

    pub fn create(self) -> Box<dyn AgentRunner> {
        match self {
            RunnerKind::Claude => Box::new(ClaudeRunner::new()),
        }
    }

    /// Onboarding's proactive "is this agent installed" check (W15) — the
    /// same PATH-scan `find_claude_binary` already uses internally when a
    /// chat/extraction runner is actually spawned, exposed here so
    /// onboarding can gate on it *before* first use instead of only
    /// discovering a missing CLI mid-recording. Deliberately does not also
    /// try to verify login state: the LLD-07 corrections log flags that
    /// heuristic (stderr substring matching) as genuinely unverified against
    /// a real logged-out install, so onboarding only hard-gates on
    /// "installed", matching the locked `01_ONBOARDING.md` spec.
    pub fn detect(self) -> RunnerDetection {
        match self {
            RunnerKind::Claude => {
                let path = super::claude::spawn::find_claude_binary(None);
                RunnerDetection {
                    installed: path.is_some(),
                    path: path.map(|p| p.to_string_lossy().into_owned()),
                }
            }
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct RunnerDetection {
    pub installed: bool,
    pub path: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_id_and_default_model_are_stable_wire_values() {
        // `chat_sessions.runner_id` and the model string sent to the CLI —
        // changing either is a behavior change, not a refactor, so this
        // test exists to make that change deliberate.
        assert_eq!(RunnerKind::Claude.id(), "claude");
        assert_eq!(
            RunnerKind::Claude.default_chat_model(),
            crate::ipc::runner::claude::MODEL_IDS.chat_default
        );
    }
}
