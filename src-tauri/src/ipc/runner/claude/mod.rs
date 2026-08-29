//! `ClaudeRunner` — the v1 `AgentRunner` implementation: a subprocess
//! supervisor around the `claude` CLI (LLD-07 §4).
//!
//! Both of LLD-07 §5's lifecycle shapes are the *same* runner code: `start`
//! always spawns a fresh `claude` process with a freshly generated
//! `--session-id`, and `prompt` writes one stream-json user turn to its
//! stdin per call. A chat caller holds one `ClaudeRunner` across many
//! `prompt()` calls (long-lived); the `run_agent_extraction` handler
//! constructs one, calls `prompt()` exactly once, and disposes it
//! (ephemeral). Nothing in this file needs to know which pattern its
//! caller is using.
//!
//! **Major correction versus LLD-07 §4.2's original sketch, found by
//! testing against a real `claude` 2.1.239 install this wave:** `start()`
//! does *not* block on a handshake frame. Under `--input-format
//! stream-json`, `claude` prints absolutely nothing — not even
//! `system/init` — until it has received the first stream-json
//! user-message line on stdin. The LLD assumed the init frame was an
//! independent pre-turn handshake; live testing showed it arrives bundled
//! with (immediately before) the *first turn's* response. Blocking
//! `start()` on a frame that only ever arrives after the first `prompt()`
//! write is a deadlock, not a slow start — confirmed by hand with a named
//! pipe (write withheld for several seconds -> zero bytes of output the
//! whole time; write one line -> `init` + the full turn arrive together
//! moments later). `start()` therefore only spawns the process and wires
//! up the reader/stderr tasks; `system/init` is swallowed like any other
//! frame during the first turn's drain, and a genuinely broken CLI (bad
//! binary, immediate crash, not logged in) surfaces through the ordinary
//! EOF/error handling in `drain_turn` on that first `prompt()` call.

mod mcp_config;
pub(crate) mod spawn;
mod stream;
mod translate;

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, Mutex as AsyncMutex};
use tokio_stream::wrappers::ReceiverStream;

use crate::error::AppError;
use crate::ipc::runner::{
    AgentEvent, AgentRunner, AgentStream, ApprovalDecision, ApprovalId, NoticeKind, PromptRequest,
    RunnerConfig, RunnerMode, StopReason, TurnId, Usage, UserContent,
};
use stream::{FrameReader, RawLine};

/// Pinned model ids (LLD-07 §4.4). `claude-fable-5` is a Settings-only
/// alternate — not wired in v1 (no Settings surface exists yet).
pub const MODEL_IDS: ModelIds = ModelIds {
    chat_default: "claude-haiku-4-5-20251001",
    // Kept on Sonnet: the "Processing failed during extracting" the user hit
    // right after switching this to Haiku is almost certainly this — Rust's
    // JSON parse (`extraction_handler.rs`) requires the model's entire
    // response to be pure JSON, and small models are meaningfully less
    // reliable at obeying "no prose, no code fences" than Sonnet, even with
    // the schema-retry nudge. LLD-07 §10's original design table already
    // flagged this: "Extraction ... claude-sonnet-5 ... No in v1 —
    // extraction quality is load-bearing." Chat has no such structured-output
    // requirement, so it stays on Haiku.
    extraction: "claude-sonnet-5",
};

pub struct ModelIds {
    pub chat_default: &'static str,
    pub extraction: &'static str,
}

enum RawFrameMsg {
    Frame(Value),
    Malformed,
}

const MAX_FRAMES_PER_TURN: u32 = 5_000;
const MALFORMED_STREAK_LIMIT: u32 = 3;
const STDERR_TAIL_CAP_BYTES: usize = 64 * 1024;

pub struct ClaudeRunner {
    config: Option<RunnerConfig>,
    child: Option<Arc<AsyncMutex<Child>>>,
    stdin: Option<ChildStdin>,
    raw_rx: Option<Arc<AsyncMutex<mpsc::Receiver<RawFrameMsg>>>>,
    stderr_tail: Option<Arc<StdMutex<String>>>,
    current_turn: Option<TurnId>,
    pending_cancel: Option<oneshot::Sender<()>>,
    /// Set only when `RunnerConfig.mcp` was `Some` at `start()` — the
    /// written `mcp.json`'s path, deleted on `dispose()` (LLD-07 §6.1).
    mcp_config_path: Option<std::path::PathBuf>,
    /// `set_mode` is latent in v1 (LLD-07 §4.6's tool-driven mapping has no
    /// tools to react to yet — no MCP config ever ships this wave). The CLI
    /// only accepts `--permission-mode` at spawn time anyway, so there is
    /// nothing a live-process mode change could do; this just records the
    /// call for whichever later wave adds a respawn-on-mode-change path.
    #[allow(dead_code)]
    pending_mode: Option<RunnerMode>,
}

impl Default for ClaudeRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaudeRunner {
    pub fn new() -> Self {
        Self {
            config: None,
            child: None,
            stdin: None,
            raw_rx: None,
            stderr_tail: None,
            current_turn: None,
            pending_cancel: None,
            mcp_config_path: None,
            pending_mode: None,
        }
    }

    #[cfg(test)]
    fn with_binary_override(binary: std::path::PathBuf) -> BoundClaudeRunner {
        BoundClaudeRunner {
            runner: Self::new(),
            binary_override: Some(binary),
        }
    }
}

/// Test-only seam: points `start()` at a fake `claude` binary (a scripted
/// shell process) instead of scanning `PATH`. Production always goes
/// through `ClaudeRunner::new()` + `spawn::find_claude_binary(None)`.
#[cfg(test)]
struct BoundClaudeRunner {
    runner: ClaudeRunner,
    binary_override: Option<std::path::PathBuf>,
}

#[cfg(test)]
#[async_trait::async_trait]
impl AgentRunner for BoundClaudeRunner {
    async fn start(&mut self, config: RunnerConfig) -> Result<(), AppError> {
        self.runner
            .start_with_binary(self.binary_override.take(), config)
            .await
    }
    async fn prompt(&mut self, req: PromptRequest) -> Result<AgentStream, AppError> {
        self.runner.prompt(req).await
    }
    async fn cancel_turn(&mut self, turn_id: TurnId) -> Result<(), AppError> {
        self.runner.cancel_turn(turn_id).await
    }
    async fn respond_to_approval(
        &mut self,
        req_id: ApprovalId,
        decision: ApprovalDecision,
    ) -> Result<(), AppError> {
        self.runner.respond_to_approval(req_id, decision).await
    }
    async fn set_mode(&mut self, mode: RunnerMode) -> Result<(), AppError> {
        self.runner.set_mode(mode).await
    }
    async fn dispose(self: Box<Self>) -> Result<(), AppError> {
        Box::new(self.runner).dispose().await
    }
}

#[cfg(test)]
impl BoundClaudeRunner {
    async fn debug_child_alive(&self) -> bool {
        let Some(child) = &self.runner.child else {
            return false;
        };
        let mut c = child.lock().await;
        matches!(c.try_wait(), Ok(None))
    }
}

#[async_trait::async_trait]
impl AgentRunner for ClaudeRunner {
    async fn start(&mut self, config: RunnerConfig) -> Result<(), AppError> {
        self.start_with_binary(None, config).await
    }

    async fn prompt(&mut self, req: PromptRequest) -> Result<AgentStream, AppError> {
        let stdin = self.stdin.as_mut().ok_or_else(|| AppError::Runner {
            runner: "claude".to_string(),
            message: "prompt_before_start".to_string(),
            correlation_id: crate::error::correlation_id(),
        })?;
        let raw_rx = self.raw_rx.clone().ok_or_else(|| AppError::Runner {
            runner: "claude".to_string(),
            message: "prompt_before_start".to_string(),
            correlation_id: crate::error::correlation_id(),
        })?;
        let child = self.child.clone().ok_or_else(|| AppError::Runner {
            runner: "claude".to_string(),
            message: "prompt_before_start".to_string(),
            correlation_id: crate::error::correlation_id(),
        })?;
        let stderr_tail = self.stderr_tail.clone().unwrap_or_default();

        let turn_id = req
            .turn_id
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.current_turn = Some(turn_id.clone());

        let text: String = req
            .content
            .iter()
            .map(|c| match c {
                UserContent::Text(t) => t.as_str(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        let line = serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": [{"type": "text", "text": text}] }
        });
        let mut bytes = serde_json::to_vec(&line)
            .map_err(|e| AppError::internal(format!("encode prompt frame: {e}")))?;
        bytes.push(b'\n');
        stdin
            .write_all(&bytes)
            .await
            .map_err(|e| AppError::Runner {
                runner: "claude".to_string(),
                message: format!("stdin_write_failed: {e}"),
                correlation_id: crate::error::correlation_id(),
            })?;
        stdin.flush().await.map_err(|e| AppError::Runner {
            runner: "claude".to_string(),
            message: format!("stdin_flush_failed: {e}"),
            correlation_id: crate::error::correlation_id(),
        })?;

        let (cancel_tx, cancel_rx) = oneshot::channel();
        self.pending_cancel = Some(cancel_tx);

        let (out_tx, out_rx) = mpsc::channel(64);
        let timeout_ms = self.config.as_ref().and_then(|c| c.timeout_ms);
        tokio::spawn(drain_turn(
            raw_rx,
            turn_id,
            timeout_ms,
            child,
            stderr_tail,
            cancel_rx,
            out_tx,
        ));

        Ok(Box::pin(ReceiverStream::new(out_rx)))
    }

    async fn cancel_turn(&mut self, turn_id: TurnId) -> Result<(), AppError> {
        if self.current_turn.as_ref() != Some(&turn_id) {
            return Ok(()); // stale/idempotent cancel (LLD-07 §5.3 step 1).
        }
        // Unparks anything awaiting `respond_to_approval` (none exist in
        // v1, but the drain semantics are correct either way).
        if let Some(cancel_tx) = self.pending_cancel.take() {
            let _ = cancel_tx.send(());
        }
        if let Some(child) = self.child.clone() {
            {
                let mut c = child.lock().await;
                send_sigterm(&mut c).await;
            }
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let mut c = child.lock().await;
                if matches!(c.try_wait(), Ok(None)) {
                    let _ = c.start_kill();
                }
            });
        }
        Ok(())
    }

    async fn respond_to_approval(
        &mut self,
        _req_id: ApprovalId,
        _decision: ApprovalDecision,
    ) -> Result<(), AppError> {
        // Latent in v1 — no tools ship, so the CLI never emits a
        // permission-request frame and `ApprovalRequest` is never produced.
        // Idempotent no-op per LLD-07 §5.3's "late response after cancel"
        // rule generalizes cleanly to "no response is ever pending".
        Ok(())
    }

    async fn set_mode(&mut self, mode: RunnerMode) -> Result<(), AppError> {
        self.pending_mode = Some(mode);
        Ok(())
    }

    async fn dispose(mut self: Box<Self>) -> Result<(), AppError> {
        if let Some(cancel_tx) = self.pending_cancel.take() {
            let _ = cancel_tx.send(());
        }
        if let Some(child) = self.child.take() {
            let mut c = child.lock().await;
            send_sigterm(&mut c).await;
            if tokio::time::timeout(Duration::from_secs(2), c.wait())
                .await
                .is_err()
            {
                let _ = c.start_kill();
            }
        }
        if let Some(path) = self.mcp_config_path.take() {
            mcp_config::remove(&path);
        }
        Ok(())
    }
}

impl ClaudeRunner {
    async fn start_with_binary(
        &mut self,
        binary_override: Option<std::path::PathBuf>,
        config: RunnerConfig,
    ) -> Result<(), AppError> {
        let binary = match binary_override {
            Some(b) => b,
            None => spawn::find_claude_binary(None).ok_or_else(spawn::binary_missing_error)?,
        };
        let session_id = uuid::Uuid::new_v4().to_string();

        // `--allowedTools "mcp__mnemos"` (spawn.rs's argv builder) is what
        // makes v1's read-only MCP tools need no approval-flow UI —
        // `--permission-mode` itself never has to move off the safe
        // `default` LLD-07 §6.3 warns to stay on (see spawn.rs's doc
        // comment for the real-CLI verification behind this).
        let mcp_config_path = match &config.mcp {
            Some(mcp) => {
                let path = crate::fs::paths::mcp_config_path(&session_id)?;
                let data_dir = crate::fs::paths::data_root()?;
                mcp_config::write(&path, &mcp.server_binary, &data_dir)?;
                Some(path)
            }
            None => None,
        };

        let mcp_config_path_str = mcp_config_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        let argv = spawn::build_argv(&spawn::ArgvOptions {
            model: &config.model,
            permission_mode: "default",
            system_prompt: config.system_prompt.as_deref(),
            session_id: &session_id,
            mcp_config_path: mcp_config_path_str.as_deref(),
        });

        let mut command = Command::new(&binary);
        command
            .args(&argv)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            // Never inject API-key auth — the CLI owns its own OAuth
            // session (LLD-07 §4.3).
            .env_remove("ANTHROPIC_API_KEY")
            // PROVISIONAL courtesy attribution header (LLD-07 §4.2);
            // harmless if the CLI doesn't recognize it.
            .env("CLAUDE_CODE_ENTRYPOINT", "mnemos");

        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(e) => {
                if let Some(path) = &mcp_config_path {
                    mcp_config::remove(path);
                }
                return Err(AppError::Runner {
                    runner: "claude".to_string(),
                    message: format!("spawn_failed: {e}"),
                    correlation_id: crate::error::correlation_id(),
                });
            }
        };

        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let stderr_tail = Arc::new(StdMutex::new(String::new()));
        tokio::spawn(stderr_task(stderr, Arc::clone(&stderr_tail)));

        let (raw_tx, raw_rx) = mpsc::channel(256);
        tokio::spawn(reader_task(FrameReader::new(stdout), raw_tx));
        let raw_rx = Arc::new(AsyncMutex::new(raw_rx));

        // No separate handshake read here — see the module doc comment.
        // `claude` under `--input-format stream-json` prints *nothing*,
        // not even `system/init`, until it has received the first
        // stream-json user-message line on stdin. Blocking here for an
        // init frame (as LLD-07 §4.2's original sketch assumed) deadlocks:
        // nothing will ever arrive before `prompt()` writes the first
        // turn. `system/init` is instead swallowed as an ordinary frame at
        // the start of the first turn's drain (`translate::translate`),
        // and a genuinely broken CLI (bad binary, crash-on-start, not
        // logged in) surfaces through the existing EOF/error handling in
        // `drain_turn` on that first `prompt()` call instead.
        self.child = Some(Arc::new(AsyncMutex::new(child)));
        self.stdin = Some(stdin);
        self.raw_rx = Some(raw_rx);
        self.stderr_tail = Some(stderr_tail);
        self.mcp_config_path = mcp_config_path;
        self.config = Some(config);
        Ok(())
    }
}

async fn send_sigterm(child: &mut Child) {
    #[cfg(unix)]
    {
        if let Some(pid) = child.id() {
            // SAFETY: `pid` is a valid, currently-running child pid we own
            // (tokio hands it back from the live `Child`); `kill(2)` with a
            // valid pid and a standard signal number has no unsafe
            // preconditions beyond that.
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
        }
    }
    #[cfg(not(unix))]
    {
        // Windows has no SIGTERM; `start_kill` is `TerminateProcess`, the
        // LLD's documented equivalent (LLD-07 §5.3 step 2).
        let _ = child.start_kill();
    }
}

async fn reader_task(mut fr: FrameReader, tx: mpsc::Sender<RawFrameMsg>) {
    loop {
        match fr.next_raw().await {
            Ok(Some(RawLine::Frame(v))) => {
                if tx.send(RawFrameMsg::Frame(v)).await.is_err() {
                    return;
                }
            }
            Ok(Some(RawLine::Malformed)) => {
                if tx.send(RawFrameMsg::Malformed).await.is_err() {
                    return;
                }
            }
            Ok(None) => return, // clean EOF; dropping `tx` signals receivers.
            Err(e) => {
                tracing::warn!(error = %e, "runner.claude.stdout_read_error");
                return;
            }
        }
    }
}

async fn stderr_task(stderr: tokio::process::ChildStderr, tail: Arc<StdMutex<String>>) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let mut buf = tail.lock().unwrap();
        buf.push_str(&line);
        buf.push('\n');
        if buf.len() > STDERR_TAIL_CAP_BYTES {
            let start = buf.len() - STDERR_TAIL_CAP_BYTES;
            *buf = buf[start..].to_string();
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn drain_turn(
    raw_rx: Arc<AsyncMutex<mpsc::Receiver<RawFrameMsg>>>,
    turn_id: TurnId,
    timeout_ms: Option<u64>,
    child: Arc<AsyncMutex<Child>>,
    stderr_tail: Arc<StdMutex<String>>,
    cancel_rx: oneshot::Receiver<()>,
    out_tx: mpsc::Sender<AgentEvent>,
) {
    let body = drain_body(
        raw_rx,
        turn_id.clone(),
        &child,
        &stderr_tail,
        cancel_rx,
        &out_tx,
    );
    if let Some(ms) = timeout_ms {
        if tokio::time::timeout(Duration::from_millis(ms), body)
            .await
            .is_err()
        {
            send_sigterm(&mut *child.lock().await).await;
            let _ = out_tx
                .send(AgentEvent::Error {
                    turn_id,
                    error: AppError::Runner {
                        runner: "claude".to_string(),
                        message: "timeout".to_string(),
                        correlation_id: crate::error::correlation_id(),
                    },
                })
                .await;
        }
    } else {
        body.await;
    }
}

async fn drain_body(
    raw_rx: Arc<AsyncMutex<mpsc::Receiver<RawFrameMsg>>>,
    turn_id: TurnId,
    child: &Arc<AsyncMutex<Child>>,
    stderr_tail: &Arc<StdMutex<String>>,
    mut cancel_rx: oneshot::Receiver<()>,
    out_tx: &mpsc::Sender<AgentEvent>,
) {
    let mut malformed_streak: u32 = 0;
    let mut frame_count: u32 = 0;
    let mut rx = raw_rx.lock().await;
    loop {
        tokio::select! {
            biased;
            _ = &mut cancel_rx => {
                let _ = out_tx.send(AgentEvent::Complete {
                    turn_id,
                    stop_reason: StopReason::Cancelled,
                    usage: Usage::default(),
                }).await;
                return;
            }
            raw = rx.recv() => {
                match raw {
                    None => {
                        let (exit_code, stderr_snip) = exit_info(child, stderr_tail).await;
                        let message = classify_exit(exit_code, &stderr_snip);
                        let _ = out_tx.send(AgentEvent::Error {
                            turn_id,
                            error: AppError::Runner {
                                runner: "claude".to_string(),
                                message,
                                correlation_id: crate::error::correlation_id(),
                            },
                        }).await;
                        return;
                    }
                    Some(RawFrameMsg::Malformed) => {
                        malformed_streak += 1;
                        frame_count += 1;
                        let _ = out_tx.send(AgentEvent::Notice {
                            turn_id: turn_id.clone(),
                            notice_kind: NoticeKind::Warn,
                            text: "malformed_frame".to_string(),
                        }).await;
                        if malformed_streak >= MALFORMED_STREAK_LIMIT {
                            send_sigterm(&mut *child.lock().await).await;
                            let _ = out_tx.send(AgentEvent::Error {
                                turn_id,
                                error: AppError::Runner {
                                    runner: "claude".to_string(),
                                    message: "stream_corrupt".to_string(),
                                    correlation_id: crate::error::correlation_id(),
                                },
                            }).await;
                            return;
                        }
                    }
                    Some(RawFrameMsg::Frame(v)) => {
                        malformed_streak = 0;
                        frame_count += 1;
                        if frame_count > MAX_FRAMES_PER_TURN {
                            send_sigterm(&mut *child.lock().await).await;
                            let _ = out_tx.send(AgentEvent::Error {
                                turn_id,
                                error: AppError::Runner {
                                    runner: "claude".to_string(),
                                    message: "stream_runaway".to_string(),
                                    correlation_id: crate::error::correlation_id(),
                                },
                            }).await;
                            return;
                        }
                        let events = translate::translate(&turn_id, &v);
                        let terminal = translate::is_terminal(&events);
                        for ev in events {
                            if out_tx.send(ev).await.is_err() {
                                return; // consumer dropped the stream.
                            }
                        }
                        if terminal {
                            return;
                        }
                    }
                }
            }
        }
    }
}

async fn exit_info(
    child: &Arc<AsyncMutex<Child>>,
    stderr_tail: &Arc<StdMutex<String>>,
) -> (Option<i32>, String) {
    let mut c = child.lock().await;
    let code = c.try_wait().ok().flatten().and_then(|s| s.code());
    let stderr = stderr_tail.lock().unwrap().clone();
    (code, truncate(&stderr, 512))
}

/// Heuristic classification of a process exit with no terminal frame
/// (LLD-07 §8 items 2/3/5). The exact "not logged in" marker string is
/// PROVISIONAL — this wave never exercised a real logged-out `claude`
/// install (see LLD's Implementation status).
fn classify_exit(exit_code: Option<i32>, stderr_snippet: &str) -> String {
    let lower = stderr_snippet.to_lowercase();
    if lower.contains("not logged in")
        || lower.contains("claude login")
        || lower.contains("please authenticate")
    {
        "cli_not_logged_in".to_string()
    } else {
        format!("cli_error: exit_code={exit_code:?} stderr={stderr_snippet}")
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{truncated}…")
}

/// `std::env::set_var("PATH", ..)` is process-global and `cargo test` runs
/// tests on multiple threads by default — any test that temporarily
/// clobbers `PATH` (to exercise `find_claude_binary`'s miss/hit paths)
/// would otherwise race every other such test in the crate. Every test
/// that mutates `PATH` holds this lock for the duration.
#[cfg(test)]
pub(crate) fn path_env_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::runner::{ApprovalPolicy, PromptRequest};
    use tokio_stream::StreamExt;

    fn extraction_config(model: &str) -> RunnerConfig {
        RunnerConfig {
            model: model.to_string(),
            timeout_ms: Some(5_000),
            system_prompt: None,
            tools: vec![],
            approval_policy: ApprovalPolicy::AutoDenyDestructive,
            mcp: None,
        }
    }

    fn write_script(dir: &tempfile::TempDir, name: &str, body: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[tokio::test]
    async fn happy_path_completes_with_text_and_usage() {
        let dir = tempfile::tempdir().unwrap();
        let script = write_script(
            &dir,
            "fake_claude.sh",
            r#"#!/bin/sh
printf '%s\n' '{"type":"system","subtype":"init","session_id":"t"}'
printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}'
printf '%s\n' '{"is_error":false,"stop_reason":"end_turn","session_id":"t","usage":{"input_tokens":1,"output_tokens":1},"type":"result"}'
cat >/dev/null
"#,
        );

        let mut runner = ClaudeRunner::with_binary_override(script);
        runner
            .start(extraction_config("claude-sonnet-5"))
            .await
            .unwrap();
        let mut stream = runner
            .prompt(PromptRequest {
                content: vec![UserContent::Text("hi".into())],
                history: vec![],
                turn_id: None,
            })
            .await
            .unwrap();

        let mut saw_text = false;
        let mut saw_complete = false;
        while let Some(ev) = stream.next().await {
            match ev {
                AgentEvent::TokenDelta { text, .. } if text == "hi" => saw_text = true,
                AgentEvent::Complete {
                    stop_reason, usage, ..
                } => {
                    assert!(matches!(stop_reason, StopReason::EndTurn));
                    assert_eq!(usage.input_tokens, 1);
                    saw_complete = true;
                }
                _ => {}
            }
        }
        assert!(saw_text && saw_complete);
        Box::new(runner).dispose().await.unwrap();
    }

    #[tokio::test]
    async fn three_consecutive_malformed_lines_trip_stream_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let script = write_script(
            &dir,
            "fake_claude.sh",
            r#"#!/bin/sh
printf '%s\n' '{"type":"system","subtype":"init","session_id":"t"}'
printf '%s\n' 'bad1'
printf '%s\n' 'bad2'
printf '%s\n' 'bad3'
printf '%s\n' '{"is_error":false,"stop_reason":"end_turn","session_id":"t","usage":{"input_tokens":1,"output_tokens":1},"type":"result"}'
cat >/dev/null
"#,
        );

        let mut runner = ClaudeRunner::with_binary_override(script);
        runner
            .start(extraction_config("claude-sonnet-5"))
            .await
            .unwrap();
        let mut stream = runner
            .prompt(PromptRequest {
                content: vec![UserContent::Text("hi".into())],
                history: vec![],
                turn_id: None,
            })
            .await
            .unwrap();

        let mut warn_count = 0;
        let mut got_stream_corrupt = false;
        while let Some(ev) = stream.next().await {
            match ev {
                AgentEvent::Notice {
                    notice_kind: NoticeKind::Warn,
                    ..
                } => warn_count += 1,
                AgentEvent::Error {
                    error: AppError::Runner { message, .. },
                    ..
                } => {
                    got_stream_corrupt = message == "stream_corrupt";
                }
                _ => {}
            }
        }
        assert_eq!(warn_count, 3);
        assert!(got_stream_corrupt);
        Box::new(runner).dispose().await.unwrap();
    }

    #[tokio::test]
    async fn cancel_turn_terminates_the_stream_and_kills_the_child() {
        let dir = tempfile::tempdir().unwrap();
        let script = write_script(
            &dir,
            "fake_claude.sh",
            r#"#!/bin/sh
printf '%s\n' '{"type":"system","subtype":"init","session_id":"t"}'
printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"text","text":"partial"}]}}'
sleep 5
printf '%s\n' '{"is_error":false,"stop_reason":"end_turn","session_id":"t","usage":{"input_tokens":1,"output_tokens":1},"type":"result"}'
"#,
        );

        let mut runner = ClaudeRunner::with_binary_override(script);
        runner
            .start(extraction_config("claude-sonnet-5"))
            .await
            .unwrap();
        let mut stream = runner
            .prompt(PromptRequest {
                content: vec![UserContent::Text("hi".into())],
                history: vec![],
                turn_id: Some("turn-1".to_string()),
            })
            .await
            .unwrap();

        // Let the partial text frame land before cancelling.
        let first = stream.next().await;
        assert!(matches!(first, Some(AgentEvent::TokenDelta { .. })));

        runner.cancel_turn("turn-1".to_string()).await.unwrap();

        let mut saw_cancelled = false;
        while let Some(ev) = stream.next().await {
            if let AgentEvent::Complete {
                stop_reason: StopReason::Cancelled,
                ..
            } = ev
            {
                saw_cancelled = true;
            }
        }
        assert!(
            saw_cancelled,
            "expected Complete{{Cancelled}} after cancel_turn"
        );

        // Give the SIGTERM a moment to land, then verify the process is
        // actually gone (LLD-07 DoD: "cancel_turn works (SIGTERM verified)").
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !runner.debug_child_alive().await,
            "child should be terminated by SIGTERM"
        );
    }

    #[tokio::test]
    async fn cancel_turn_on_a_stale_turn_id_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let script = write_script(
            &dir,
            "fake_claude.sh",
            r#"#!/bin/sh
printf '%s\n' '{"type":"system","subtype":"init","session_id":"t"}'
printf '%s\n' '{"is_error":false,"stop_reason":"end_turn","session_id":"t","usage":{"input_tokens":1,"output_tokens":1},"type":"result"}'
cat >/dev/null
"#,
        );
        let mut runner = ClaudeRunner::with_binary_override(script);
        runner
            .start(extraction_config("claude-sonnet-5"))
            .await
            .unwrap();
        let _stream = runner
            .prompt(PromptRequest {
                content: vec![UserContent::Text("hi".into())],
                history: vec![],
                turn_id: Some("real-turn".to_string()),
            })
            .await
            .unwrap();

        // A stale cancel for a superseded turn id must not touch anything.
        runner
            .cancel_turn("some-other-turn".to_string())
            .await
            .unwrap();
        assert!(runner.debug_child_alive().await);
        Box::new(runner).dispose().await.unwrap();
    }

    #[tokio::test]
    async fn start_does_not_block_when_the_process_prints_nothing_up_front() {
        // Regression test for the deadlock this wave found: `start()` must
        // return immediately without waiting on any output, since a real
        // `claude` prints nothing at all until the first prompt is written.
        let dir = tempfile::tempdir().unwrap();
        let script = write_script(&dir, "fake_claude.sh", "#!/bin/sh\nsleep 30\n");
        let mut runner = ClaudeRunner::with_binary_override(script);
        let started = tokio::time::timeout(
            Duration::from_secs(2),
            runner.start(extraction_config("claude-sonnet-5")),
        )
        .await;
        assert!(started.is_ok(), "start() must not block waiting for output");
        assert!(started.unwrap().is_ok());
        Box::new(runner).dispose().await.unwrap();
    }

    #[tokio::test]
    async fn dead_process_surfaces_as_a_runner_error_on_first_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let script = write_script(&dir, "fake_claude.sh", "#!/bin/sh\nexit 1\n");
        let mut runner = ClaudeRunner::with_binary_override(script);
        runner
            .start(extraction_config("claude-sonnet-5"))
            .await
            .unwrap();
        // Give the already-doomed process a moment to actually exit before
        // we try to write to its stdin.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let result = runner
            .prompt(PromptRequest {
                content: vec![UserContent::Text("hi".into())],
                history: vec![],
                turn_id: None,
            })
            .await;
        // Depending on OS timing, the write itself may fail immediately
        // (EPIPE) or may succeed into the kernel pipe buffer with the
        // failure only surfacing once the drain loop hits EOF without a
        // terminal frame — either is an acceptable "dead process never
        // hangs the caller" outcome.
        match result {
            Err(_) => {}
            Ok(mut stream) => {
                let mut saw_error = false;
                while let Some(ev) = stream.next().await {
                    if matches!(ev, AgentEvent::Error { .. }) {
                        saw_error = true;
                    }
                }
                assert!(
                    saw_error,
                    "dead process must terminate the stream with an Error"
                );
            }
        }
    }

    #[tokio::test]
    async fn binary_missing_is_worker_unavailable() {
        let _guard = path_env_test_lock().lock().await;
        let mut runner = ClaudeRunner::new();
        let dir = tempfile::tempdir().unwrap(); // guaranteed empty, nothing named "claude" in it.
        let old = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", dir.path()) };
        let result = runner.start(extraction_config("claude-sonnet-5")).await;
        if let Some(old) = old {
            unsafe { std::env::set_var("PATH", old) };
        }
        assert!(matches!(result, Err(AppError::WorkerUnavailable { .. })));
    }
}
