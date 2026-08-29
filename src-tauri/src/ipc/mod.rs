//! Child-process supervision. `python.rs` is the persistent worker (W5).
//! `swift.rs` is the macOS-only per-recording capture sidecar (W7a).
//! `runner.rs` is the AI CLI runner (`ClaudeRunner`, W8).

pub mod framing;
pub mod python;
pub mod runner;
#[cfg(target_os = "macos")]
pub mod swift;
