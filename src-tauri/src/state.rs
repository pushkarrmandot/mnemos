//! Application state held by `tauri::State`. Every command reaches its
//! dependencies through here — never through a global static (BACKEND §1).

/// W1 holds only what `ping` needs. Later waves add fields:
/// `db_read` / `db_write` pools (W4), `python` worker handle (W5),
/// `swift` sidecar handle (W7), `runner` registry (W8).
#[derive(Debug)]
pub struct AppState {
    pub app_version: String,
}

impl AppState {
    pub fn new(app_version: impl Into<String>) -> Self {
        Self {
            app_version: app_version.into(),
        }
    }
}
