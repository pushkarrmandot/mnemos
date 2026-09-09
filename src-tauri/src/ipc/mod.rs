//! Child-process supervision. `python.rs` is the persistent worker.
//! `swift.rs` is the macOS-only per-recording capture sidecar.
//! `runner.rs` is the AI CLI runner (`ClaudeRunner`).

pub mod framing;
#[cfg(target_os = "macos")]
pub mod meeting_watcher;
pub mod python;
pub mod runner;
#[cfg(target_os = "macos")]
pub mod swift;
