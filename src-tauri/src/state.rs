//! Application state held by `tauri::State`. Every command reaches its
//! dependencies through here — never through a global static (BACKEND §1).

use crate::db::service::SqliteStorageService;

/// W1 held only what `ping` needed. W4 adds the storage layer. Later waves
/// add: `python` worker handle (W5), `swift` sidecar handle (W7), `runner`
/// registry (W8).
pub struct AppState {
    pub app_version: String,
    pub storage: SqliteStorageService,
}

impl AppState {
    pub fn new(app_version: impl Into<String>, storage: SqliteStorageService) -> Self {
        Self {
            app_version: app_version.into(),
            storage,
        }
    }
}
