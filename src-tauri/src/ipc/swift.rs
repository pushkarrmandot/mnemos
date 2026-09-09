//! macOS-only: spawns and controls the `mnemos-audio` Swift sidecar for one
//! recording session. Sidecar state is disjoint
//! from the Python worker's — this module knows nothing about JSON-RPC
//! framing or the worker's registry, only the sidecar's ~9-message
//! line-delimited JSON protocol.
//!
//! `SidecarEvent::Ready` is consumed during the spawn handshake and never
//! forwarded; every event after that is normalized into `capture::CaptureEvent`
//! so `commands::recording`'s reader never special-cases which
//! platform produced it (see `capture::CaptureEvent`'s cross-platform
//! vocabulary).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex as AsyncMutex};

use crate::capture::CaptureEvent;
use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct SidecarConfig {
    pub conversation_id: String,
    pub mic_path: PathBuf,
    pub system_path: PathBuf,
    pub mic_device_id: Option<String>,
}

pub struct SidecarHandle {
    pub events: mpsc::Receiver<CaptureEvent>,
    pub control: SidecarControl,
}

#[derive(Clone)]
pub struct SidecarControl {
    stdin_tx: mpsc::Sender<Vec<u8>>,
    child: Arc<AsyncMutex<Child>>,
}

impl SidecarControl {
    pub async fn pause(&self) -> Result<(), AppError> {
        self.write_command(br#"{"method":"pause"}"#).await
    }

    pub async fn resume(&self) -> Result<(), AppError> {
        self.write_command(br#"{"method":"resume"}"#).await
    }

    /// Sends `stop`, waits (bounded) for the caller to observe `Stopped` on
    /// the event channel, then guarantees the process is gone —
    /// SIGTERM/kill_on_drop backstop if the sidecar
    /// ignores the request. This method only sends the command; the caller
    /// is expected to have already seen (or given up waiting for)
    /// `CaptureEvent::Stopped` on `events` before calling it, then calls
    /// `force_kill` if the process is still alive.
    pub async fn stop(&self) -> Result<(), AppError> {
        self.write_command(br#"{"method":"stop"}"#).await
    }

    /// Hard-kill backstop, used when `Stopped` doesn't arrive within the
    /// 2s timeout.
    pub async fn force_kill(&self) {
        let mut child = self.child.lock().await;
        let _ = child.start_kill();
    }

    async fn write_command(&self, body: &'static [u8]) -> Result<(), AppError> {
        let mut line = body.to_vec();
        line.push(b'\n');
        self.stdin_tx
            .send(line)
            .await
            .map_err(|_| AppError::Storage {
                message: "recording_failed:sidecar_exited".into(),
                correlation_id: crate::error::correlation_id(),
            })
    }
}

/// Spawn sequence: launch, await `Ready` (2s timeout), send
/// `start`, await `Started` — folded into `CaptureEvent::Started` on the
/// returned handle's channel by the time this returns.
pub async fn spawn(sidecar_bin: &Path, cfg: SidecarConfig) -> Result<SidecarHandle, AppError> {
    let mut command = Command::new(sidecar_bin);
    // Never leave a child on the inherited cwd. A Finder-launched `.app`
    // inherits `launchd`'s `/`, which makes any path-relative work a
    // subprocess does land on the filesystem root — and on macOS, walking
    // out of `/` into the user's home is what trips TCC consent prompts
    // for Documents/Desktop/iCloud. This sidecar only ever writes absolute
    // paths, so this is defence in depth rather than a fix for an observed
    // bug, but the cost is one line.
    if let Ok(root) = crate::fs::paths::data_root() {
        if root.is_dir() {
            command.current_dir(root);
        }
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = command.spawn().map_err(|e| AppError::Storage {
        message: format!("recording_failed:sidecar_spawn_failed:{e}"),
        correlation_id: crate::error::correlation_id(),
    })?;

    let stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");

    let mut lines = BufReader::new(stdout).lines();

    // handshake
    let mut stdin = stdin;
    stdin
        .write_all(br#"{"method":"handshake","host_version":"1.0.0"}"#)
        .await
        .map_err(|e| AppError::Storage {
            message: format!("recording_failed:sidecar_write_failed:{e}"),
            correlation_id: crate::error::correlation_id(),
        })?;
    stdin.write_all(b"\n").await.ok();
    stdin.flush().await.ok();

    let ready = tokio::time::timeout(Duration::from_secs(2), lines.next_line())
        .await
        .map_err(|_| AppError::Storage {
            message: "recording_failed:sidecar_ready_timeout".into(),
            correlation_id: crate::error::correlation_id(),
        })?
        .map_err(|e| AppError::Storage {
            message: format!("recording_failed:sidecar_read_failed:{e}"),
            correlation_id: crate::error::correlation_id(),
        })?
        .ok_or_else(|| AppError::Storage {
            message: "recording_failed:sidecar_exited".into(),
            correlation_id: crate::error::correlation_id(),
        })?;
    let ready_json: Value = serde_json::from_str(&ready).map_err(|e| AppError::Storage {
        message: format!("recording_failed:sidecar_bad_ready:{e}"),
        correlation_id: crate::error::correlation_id(),
    })?;
    if ready_json.get("event").and_then(Value::as_str) != Some("ready") {
        return Err(AppError::Storage {
            message: format!("recording_failed:sidecar_unexpected_first_event:{ready_json}"),
            correlation_id: crate::error::correlation_id(),
        });
    }

    // start
    let start_cmd = serde_json::json!({
        "method": "start",
        "conversation_id": cfg.conversation_id,
        "mic_path": cfg.mic_path,
        "system_path": cfg.system_path,
        "mic_device_id": cfg.mic_device_id,
    });
    let mut start_line = serde_json::to_vec(&start_cmd)
        .map_err(|e| AppError::internal(format!("encode sidecar start command: {e}")))?;
    start_line.push(b'\n');
    stdin
        .write_all(&start_line)
        .await
        .map_err(|e| AppError::Storage {
            message: format!("recording_failed:sidecar_write_failed:{e}"),
            correlation_id: crate::error::correlation_id(),
        })?;
    stdin.flush().await.ok();

    let (events_tx, events_rx) = mpsc::channel(256);
    let (stdin_tx, stdin_rx) = mpsc::channel::<Vec<u8>>(8);

    tokio::spawn(stdin_writer_task(stdin, stdin_rx));
    tokio::spawn(stdout_reader_task(lines, events_tx));
    tokio::spawn(stderr_forward_task(stderr));

    let child = Arc::new(AsyncMutex::new(child));
    Ok(SidecarHandle {
        events: events_rx,
        control: SidecarControl { stdin_tx, child },
    })
}

async fn stdin_writer_task(mut stdin: tokio::process::ChildStdin, mut rx: mpsc::Receiver<Vec<u8>>) {
    while let Some(line) = rx.recv().await {
        if stdin.write_all(&line).await.is_err() {
            return;
        }
        if stdin.flush().await.is_err() {
            return;
        }
    }
}

async fn stdout_reader_task(
    mut lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    tx: mpsc::Sender<CaptureEvent>,
) {
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                if let Some(event) = parse_sidecar_line(&line) {
                    if tx.send(event).await.is_err() {
                        return; // receiver dropped
                    }
                }
            }
            Ok(None) => {
                // Sidecar closed stdout without a Stopped event: "exited" is
                // never actually sent; the sidecar process just closes
                // stdout.
                let _ = tx
                    .send(CaptureEvent::Exited {
                        code: None,
                        signal: None,
                    })
                    .await;
                return;
            }
            Err(e) => {
                tracing::warn!(error = %e, "sidecar.stdout.read_error");
                let _ = tx
                    .send(CaptureEvent::Exited {
                        code: None,
                        signal: None,
                    })
                    .await;
                return;
            }
        }
    }
}

async fn stderr_forward_task(stderr: tokio::process::ChildStderr) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::warn!(component = "mnemos-audio", "{line}");
    }
}

/// Parses one line of sidecar stdout into the cross-platform
/// `CaptureEvent`. `ready` is not representable here — it is only ever
/// consumed inline during `spawn()`. An unrecognized/malformed line is
/// dropped (logged), matching the worker's own "malformed line" tolerance.
fn parse_sidecar_line(line: &str) -> Option<CaptureEvent> {
    let v: Value = serde_json::from_str(line).ok()?;
    let event = v.get("event")?.as_str()?;
    match event {
        "started" => Some(CaptureEvent::Started {
            started_at_ms: v.get("started_at_ms")?.as_i64()?,
        }),
        "level" => Some(CaptureEvent::Level {
            mic_db: v.get("mic_db")?.as_f64()? as f32,
            system_db: v.get("system_db")?.as_f64()? as f32,
        }),
        "chunk" => Some(CaptureEvent::Chunk {
            source: serde_json::from_value(v.get("source")?.clone()).ok()?,
            bytes_written: v.get("bytes_written")?.as_u64()?,
        }),
        "warning" => Some(CaptureEvent::Warning {
            kind: v.get("kind")?.as_str()?.to_string(),
            message: v
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "paused" => Some(CaptureEvent::Paused),
        "resumed" => Some(CaptureEvent::Resumed),
        "stopped" => Some(CaptureEvent::Stopped {
            mic_bytes: v.get("mic_bytes")?.as_u64()?,
            system_bytes: v.get("system_bytes")?.as_u64()?,
        }),
        "error" => Some(CaptureEvent::Error {
            kind: v.get("kind")?.as_str()?.to_string(),
            message: v
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        // Which system-audio backend actually ran, and — when the Core Audio
        // tap came up but produced nothing — why we abandoned it. Not a
        // `CaptureEvent`: nothing downstream branches on the backend, but
        // "which path did this recording take" is the first question asked
        // whenever system audio comes back silent, and without this the
        // answer isn't recoverable after the fact.
        "system_audio_backend" => {
            tracing::info!(
                backend = v
                    .get("backend")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown"),
                fell_back_from = v
                    .get("fell_back_from")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
                reason = v
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
                "capture.system_audio_backend"
            );
            None
        }
        _ => {
            tracing::warn!(event, "sidecar.stdout.unknown_event");
            None
        }
    }
}

/// Onboarding permission preflight — a one-shot invocation of the
/// sidecar binary with `argv[1]` set to one of `check-permissions` /
/// `request-mic-permission` / `request-screen-permission`, distinct from
/// `spawn()`'s persistent per-recording session above. Reads exactly one
/// stdout line, parses it as JSON, and returns it raw — callers (all in
/// `commands::onboarding`) pull out whichever fields their subcommand
/// actually emits.
pub async fn run_permission_subcommand(
    sidecar_bin: &Path,
    subcommand: &str,
) -> Result<Value, AppError> {
    let output = Command::new(sidecar_bin)
        .arg(subcommand)
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| AppError::Storage {
            message: format!("onboarding_failed:permission_subcommand_spawn:{e}"),
            correlation_id: crate::error::correlation_id(),
        })?;

    let line = String::from_utf8_lossy(&output.stdout);
    let line = line.lines().next().unwrap_or_default();
    serde_json::from_str(line).map_err(|e| AppError::Storage {
        message: format!("onboarding_failed:permission_subcommand_parse:{e}"),
        correlation_id: crate::error::correlation_id(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_documented_event_kind() {
        assert_eq!(
            parse_sidecar_line(r#"{"event":"started","started_at_ms":1}"#),
            Some(CaptureEvent::Started { started_at_ms: 1 })
        );
        assert_eq!(
            parse_sidecar_line(r#"{"event":"level","mic_db":-1.5,"system_db":-2.5}"#),
            Some(CaptureEvent::Level {
                mic_db: -1.5,
                system_db: -2.5
            })
        );
        assert_eq!(
            parse_sidecar_line(r#"{"event":"chunk","source":"mic","bytes_written":16000}"#),
            Some(CaptureEvent::Chunk {
                source: crate::capture::CaptureSource::Mic,
                bytes_written: 16000
            })
        );
        assert_eq!(
            parse_sidecar_line(r#"{"event":"paused"}"#),
            Some(CaptureEvent::Paused)
        );
        assert_eq!(
            parse_sidecar_line(r#"{"event":"resumed"}"#),
            Some(CaptureEvent::Resumed)
        );
        assert_eq!(
            parse_sidecar_line(r#"{"event":"stopped","mic_bytes":10,"system_bytes":20}"#),
            Some(CaptureEvent::Stopped {
                mic_bytes: 10,
                system_bytes: 20
            })
        );
        assert_eq!(
            parse_sidecar_line(
                r#"{"event":"error","kind":"mic_disconnected","message":"unplugged"}"#
            ),
            Some(CaptureEvent::Error {
                kind: "mic_disconnected".into(),
                message: "unplugged".into()
            })
        );
        assert_eq!(
            parse_sidecar_line(r#"{"event":"warning","kind":"no_mic_signal","message":"quiet"}"#),
            Some(CaptureEvent::Warning {
                kind: "no_mic_signal".into(),
                message: "quiet".into()
            })
        );
    }

    #[test]
    fn ready_is_not_a_capture_event() {
        assert_eq!(
            parse_sidecar_line(r#"{"event":"ready","sidecar_version":"1.0.0"}"#),
            None
        );
    }

    #[test]
    fn malformed_line_is_dropped_not_panicked() {
        assert_eq!(parse_sidecar_line("not json"), None);
        assert_eq!(parse_sidecar_line(r#"{"no_event_field":true}"#), None);
    }
}
