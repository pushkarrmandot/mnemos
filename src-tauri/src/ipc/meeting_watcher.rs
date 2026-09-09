//! macOS-only: spawns and supervises the long-lived `mnemos-meeting-watcher`
//! background process (product_docs/MEETING_AUTO_DETECT_DESIGN.md).
//!
//! Unlike `ipc::swift`'s per-recording sidecar, this process has no
//! handshake/start protocol — it begins emitting events the moment it
//! launches — and it is expected to run for the app's entire lifetime, not
//! one recording. A crash is therefore not fatal to anything in flight; it's
//! just a gap in detection, so this module restarts it with a bounded
//! backoff rather than surfacing an error anywhere.

use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

#[derive(Debug, Clone, PartialEq)]
pub enum MeetingEvent {
    Detected { service: String, bundle_id: String },
    Ended { service: String, bundle_id: String },
}

/// Parses one line of watcher stdout. `started` carries no information a
/// caller needs (unlike the recording sidecar's `ready`, nothing here blocks
/// on it), so it — and anything unrecognized — is dropped rather than
/// surfaced.
fn parse_watcher_line(line: &str) -> Option<MeetingEvent> {
    let v: Value = serde_json::from_str(line).ok()?;
    let event = v.get("event")?.as_str()?;
    let service = v.get("service")?.as_str()?.to_string();
    let bundle_id = v.get("bundle_id")?.as_str()?.to_string();
    match event {
        "meeting_detected" => Some(MeetingEvent::Detected { service, bundle_id }),
        "meeting_ended" => Some(MeetingEvent::Ended { service, bundle_id }),
        _ => None,
    }
}

async fn spawn_once(bin: &Path) -> std::io::Result<Child> {
    let mut command = Command::new(bin);
    // Same defensive cwd pin as the recording sidecar (`ipc::swift::spawn`)
    // — never leave a child on an inherited `/`.
    if let Ok(root) = crate::fs::paths::data_root() {
        if root.is_dir() {
            command.current_dir(root);
        }
    }
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    command.spawn()
}

/// Runs until the process is deliberately stopped (the returned
/// `tokio::task::JoinHandle` is aborted) or the retry budget below is
/// exhausted. Each detected/ended signal is forwarded to `on_event`.
///
/// Backoff: up to 10 restarts, doubling from 1s to a 60s ceiling, reset
/// after 5 minutes of continuously healthy running — the same shape as
/// `ProcessTapSource`'s own rebuild budget in the Swift sidecar (bounded,
/// not infinite; a machine where this never stays up is better surfaced by
/// silence than a spinning retry loop).
pub fn spawn_supervised(
    bin: std::path::PathBuf,
    on_event: impl Fn(MeetingEvent) + Send + 'static,
) -> tokio::task::AbortHandle {
    let handle = tokio::spawn(async move {
        let mut backoff = Duration::from_secs(1);
        let mut healthy_since;
        let mut attempts = 0u32;

        loop {
            if attempts >= 10 {
                tracing::warn!("meeting_watcher.retry_budget_exhausted; giving up");
                return;
            }
            let mut child = match spawn_once(&bin).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(error = %e, "meeting_watcher.spawn_failed");
                    attempts += 1;
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                    continue;
                }
            };

            let stdout = child.stdout.take().expect("piped stdout");
            let stderr = child.stderr.take().expect("piped stderr");
            let mut lines = BufReader::new(stdout).lines();

            tokio::spawn(async move {
                let mut err_lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = err_lines.next_line().await {
                    tracing::warn!(component = "mnemos-meeting-watcher", "{line}");
                }
            });

            healthy_since = tokio::time::Instant::now();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(event) = parse_watcher_line(&line) {
                    on_event(event);
                }
            }
            let _ = child.wait().await;

            if healthy_since.elapsed() > Duration::from_secs(300) {
                attempts = 0;
                backoff = Duration::from_secs(1);
            } else {
                attempts += 1;
            }
            tracing::warn!(attempts, "meeting_watcher.exited; restarting");
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(60));
        }
    });
    handle.abort_handle()
}

/// Same `current_exe()`-relative lookup `lib.rs::worker_config` uses for the
/// recording sidecar, mirrored here rather than shared — the two binaries
/// live at different well-known names and the recording sidecar's resolver
/// is entangled with the Python supervisor config it also builds, not worth
/// generalizing for a second one-off caller.
pub fn resolve_bin() -> Option<std::path::PathBuf> {
    let bin = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("mnemos-meeting-watcher")))
        .filter(|p| p.is_file());
    #[cfg(debug_assertions)]
    let bin = bin.or_else(|| {
        let dev = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../swift/mnemos-audio/.build/release/mnemos-meeting-watcher");
        dev.is_file().then_some(dev)
    });
    bin
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_detected_and_ended() {
        assert_eq!(
            parse_watcher_line(
                r#"{"event":"meeting_detected","service":"zoom","bundle_id":"us.zoom.xos","source":"app","at_ms":1}"#
            ),
            Some(MeetingEvent::Detected {
                service: "zoom".into(),
                bundle_id: "us.zoom.xos".into()
            })
        );
        assert_eq!(
            parse_watcher_line(
                r#"{"event":"meeting_ended","service":"teams","bundle_id":"com.microsoft.teams","at_ms":2}"#
            ),
            Some(MeetingEvent::Ended {
                service: "teams".into(),
                bundle_id: "com.microsoft.teams".into()
            })
        );
    }

    #[test]
    fn started_and_malformed_lines_are_dropped() {
        assert_eq!(parse_watcher_line(r#"{"event":"started","at_ms":1}"#), None);
        assert_eq!(parse_watcher_line("not json"), None);
        assert_eq!(parse_watcher_line(r#"{"no_event_field":true}"#), None);
    }
}
