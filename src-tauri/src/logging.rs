//! `tracing` setup: JSON lines to `~/Mnemos/logs/tauri.log`, plus a plain
//! stderr layer in dev. No `println!` anywhere (BACKEND §7).

use std::path::PathBuf;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Kept alive for the process lifetime — dropping it drops buffered log lines.
pub struct LogGuard(#[allow(dead_code)] tracing_appender::non_blocking::WorkerGuard);

/// Initializes the global subscriber. Call once, early in `setup()`.
pub fn init(log_dir: PathBuf) -> std::io::Result<LogGuard> {
    std::fs::create_dir_all(&log_dir)?;

    // BACKEND §7 asks for 10MB size-rotation, 5 files retained.
    // `tracing-appender` only rotates on time, so W1 ships daily rotation with a
    // 5-file cap; swap in a size-rotating writer when log volume justifies it.
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("tauri")
        .filename_suffix("log")
        .max_log_files(5)
        .build(&log_dir)
        .map_err(std::io::Error::other)?;
    let (writer, guard) = tracing_appender::non_blocking(appender);

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter()));

    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(true)
        .with_writer(writer);

    let registry = tracing_subscriber::registry().with(filter).with(file_layer);

    #[cfg(debug_assertions)]
    let registry = registry.with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr));

    registry.init();

    Ok(LogGuard(guard))
}

fn default_filter() -> &'static str {
    if cfg!(debug_assertions) {
        "info,mnemos=debug"
    } else {
        "info"
    }
}
