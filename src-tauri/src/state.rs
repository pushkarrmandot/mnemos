//! Application state held by `tauri::State`. Every command reaches its
//! dependencies through here — never through a global static (BACKEND §1).

use std::sync::Arc;

use crate::commands::chat::ChatRegistry;
use crate::commands::project::RefreshDebounce;
use crate::commands::recording::RecordingRegistry;
use crate::db::service::SqliteStorageService;
use crate::ipc::python::WorkerSupervisor;

/// W1 held only what `ping` needed. W4 added the storage layer. W5 adds the
/// Python worker supervisor. W9 adds the active-recording session registry
/// (`spawn_sidecar`'s live handle and the transcript-forwarding tasks live
/// there, not here — see `commands::recording`). W12a adds the manual
/// `project.refresh_memory` 5s debounce (LLD-05 §3.1). W13a adds the
/// long-lived chat-runner registry (LLD-07 §5.1's `AppState.chat_sessions`
/// pattern, wired to a real caller for the first time). Adds the product
/// analytics client (`crate::metrics`) — cheap to `Clone` (an `mpsc::Sender`
/// under the hood), so commands read it off `state.metrics` the same way
/// they read `state.storage`.
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
