//! Update checking, install, and the launch-time auto-check — the app-side
//! half of the update pipeline (the other half is the signed release
//! workflow that publishes what this checks against).
//!
//! Persistence rides the existing generic `settings` k/v store
//! (`StorageService::get_setting`/`set_setting`), the same idiom
//! `commands::onboarding` uses — no new table needed for two small fields.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_updater::UpdaterExt;

use crate::db::service::StorageService;
use crate::error::AppError;
use crate::state::AppState;

const KEY_LAST_CHECKED_AT: &str = "updater.last_checked_at";
const KEY_AUTO_CHECK_ENABLED: &str = "updater.auto_check_enabled";

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct UpdateCheckResult {
    pub available: bool,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub checked_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct UpdaterSettings {
    pub auto_check_enabled: bool,
    pub last_checked_at: Option<i64>,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

async fn record_checked_at(state: &AppState) -> Result<i64, AppError> {
    let now = now_secs();
    state
        .storage
        .set_setting(KEY_LAST_CHECKED_AT, serde_json::Value::from(now))
        .await?;
    Ok(now)
}

/// Shared by the frontend-triggered `updater_check_now` and the launch-time
/// background check — one place decides what "available" means.
async fn run_check(app: &AppHandle, state: &AppState) -> Result<UpdateCheckResult, AppError> {
    let checked_at = record_checked_at(state).await?;
    let updater = app
        .updater()
        .map_err(|e| AppError::internal(format!("updater unavailable: {e}")))?;
    let update = updater
        .check()
        .await
        .map_err(|e| AppError::internal(format!("update check failed: {e}")))?;
    Ok(match update {
        Some(update) => UpdateCheckResult {
            available: true,
            version: Some(update.version),
            notes: update.body,
            checked_at,
        },
        None => UpdateCheckResult {
            available: false,
            version: None,
            notes: None,
            checked_at,
        },
    })
}

#[tauri::command]
#[specta::specta]
pub async fn updater_check_now(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UpdateCheckResult, AppError> {
    run_check(&app, &state).await
}

/// Called from `lib.rs`'s launch-time spawn, not exposed as an IPC command —
/// same auto-check gate the Settings toggle controls, so a user who turned
/// it off doesn't get a surprise check on the next launch either.
pub async fn check_on_launch(app: AppHandle) -> Result<(), AppError> {
    let state = app.state::<AppState>();
    let enabled = state
        .storage
        .get_setting(KEY_AUTO_CHECK_ENABLED)
        .await?
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if !enabled {
        return Ok(());
    }
    run_check(&app, &state).await?;
    Ok(())
}

/// Downloads and installs the update. Does not itself relaunch — the
/// frontend calls `@tauri-apps/plugin-process`'s `relaunch()` right after
/// this resolves, so a failed install never leaves the banner claiming a
/// restart is about to happen. Re-checks rather than reusing a prior
/// `Update` handle from `updater_check_now` — the plugin's `Update` type
/// isn't the kind of thing worth threading across a separate IPC round trip
/// and into `AppState` just to save one network call.
///
/// `force: false` and a recording is in flight → refused via `Validation`
/// with `field: "active_recording"`, so the frontend can show a "this will
/// end your current recording" confirmation and retry with `force: true`
/// rather than silently killing an in-progress capture.
#[tauri::command]
#[specta::specta]
pub async fn updater_install_and_relaunch(
    app: AppHandle,
    state: State<'_, AppState>,
    force: bool,
) -> Result<(), AppError> {
    if !force && state.recording.has_active_session() {
        return Err(AppError::Validation {
            message: "A recording is in progress".into(),
            field: Some("active_recording".into()),
        });
    }

    let updater = app
        .updater()
        .map_err(|e| AppError::internal(format!("updater unavailable: {e}")))?;
    let update = updater
        .check()
        .await
        .map_err(|e| AppError::internal(format!("update check failed: {e}")))?
        .ok_or_else(|| AppError::internal("no update available to install"))?;

    update
        .download_and_install(|_chunk_len, _content_len| {}, || {})
        .await
        .map_err(|e| AppError::internal(format!("update install failed: {e}")))?;

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn updater_get_settings(state: State<'_, AppState>) -> Result<UpdaterSettings, AppError> {
    let auto_check_enabled = state
        .storage
        .get_setting(KEY_AUTO_CHECK_ENABLED)
        .await?
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let last_checked_at = state
        .storage
        .get_setting(KEY_LAST_CHECKED_AT)
        .await?
        .and_then(|v| v.as_i64());
    Ok(UpdaterSettings {
        auto_check_enabled,
        last_checked_at,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn updater_set_auto_check_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), AppError> {
    state
        .storage
        .set_setting(KEY_AUTO_CHECK_ENABLED, serde_json::Value::Bool(enabled))
        .await
}
