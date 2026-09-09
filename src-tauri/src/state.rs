//! Application state held by `tauri::State`. Every command reaches its
//! dependencies through here — never through a global static.

use std::sync::Arc;

use crate::commands::chat::ChatRegistry;
use crate::commands::project::RefreshDebounce;
use crate::commands::recording::RecordingRegistry;
use crate::db::service::SqliteStorageService;
use crate::ipc::python::WorkerSupervisor;

/// Holds the storage layer, the Python worker supervisor, the
/// active-recording session registry (`spawn_sidecar`'s live handle and the
/// transcript-forwarding tasks live there, not here — see
/// `commands::recording`), the manual `project.refresh_memory` 5s debounce,
/// the long-lived chat-runner registry, and the product analytics client
/// (`crate::metrics`) — cheap to `Clone` (an `mpsc::Sender` under the hood),
/// so commands read it off `state.metrics` the same way they read
/// `state.storage`.
pub struct AppState {
    pub app_version: String,
    pub storage: SqliteStorageService,
    pub python: Arc<WorkerSupervisor>,
    pub recording: RecordingRegistry,
    pub project_refresh_debounce: RefreshDebounce,
    pub chat: ChatRegistry,
    pub metrics: crate::metrics::Metrics,
}

impl AppState {
    pub fn new(
        app_version: impl Into<String>,
        storage: SqliteStorageService,
        python: Arc<WorkerSupervisor>,
        metrics: crate::metrics::Metrics,
    ) -> Self {
        Self {
            app_version: app_version.into(),
            storage,
            python,
            recording: RecordingRegistry::new(),
            project_refresh_debounce: RefreshDebounce::new(),
            chat: ChatRegistry::new(),
            metrics,
        }
    }
}
