//! One module per feature area, mirroring `src/features/*` on the frontend:
//! the liveness probe here, plus `chat`, `conversation`, `meeting_detection`,
//! `metrics`, `models`, `onboarding`, `project`, `recording`, `tray`, and
//! `updater`.

pub mod chat;
pub mod conversation;
pub mod meeting_detection;
pub mod metrics;
pub mod models;
pub mod onboarding;
pub mod project;
pub mod recording;
pub mod tray;
pub mod updater;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::error::AppError;
use crate::state::AppState;

/// Reply from [`ping`]. `worker_ready` is hard-coded `false` — the field
/// exists so the shape doesn't change if it's wired to real worker-readiness
/// later.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct Pong {
    pub app_version: String,
    pub worker_ready: bool,
}

/// Liveness probe: proves the Rust ↔ React bridge and the generated bindings
/// are wired end-to-end.
#[tauri::command]
#[specta::specta]
pub async fn ping(state: State<'_, AppState>) -> Result<Pong, AppError> {
    tracing::debug!(component = "commands", "ping");
    Ok(Pong {
        app_version: state.app_version.clone(),
        worker_ready: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pong_round_trips_through_serde() {
        let pong = Pong {
            app_version: "0.1.0".into(),
            worker_ready: false,
        };
        let json = serde_json::to_string(&pong).expect("Pong must serialize");
        let back: Pong = serde_json::from_str(&json).expect("Pong must deserialize");
        assert_eq!(back.app_version, "0.1.0");
        assert!(!back.worker_ready);
    }
}
