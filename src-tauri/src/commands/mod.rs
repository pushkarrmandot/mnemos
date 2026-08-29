//! One module per feature area, mirroring `src/features/*` on the frontend
//! (BACKEND §1). W1 shipped only the liveness probe; W9 adds `recording`.
//! Later waves add `conversation.rs`, `project.rs`, `contact.rs`, `chat.rs`,
//! `settings.rs`.

pub mod chat;
pub mod conversation;
pub mod metrics;
pub mod onboarding;
pub mod project;
pub mod recording;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::error::AppError;
use crate::state::AppState;

/// Reply from [`ping`]. `worker_ready` is hard-coded `false` until W5 owns the
/// Python worker handle; the field exists now so the shape doesn't change then.
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
