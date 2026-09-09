//! Onboarding funnel — first-run state, Claude CLI
//! detection, mic/Screen-Recording permission preflight (macOS), and the
//! Parakeet model-download progress observer. v1/Claude-only: no runner
//! picker (see `ipc::runner::registry::RunnerKind` for the multi-runner
//! seam this reuses without exposing it).
//!
//! First-run persistence rides the existing generic `settings` k/v store
//! (`StorageService::get_setting`/`set_setting`) rather than a new
//! table — onboarding needed exactly the shape that store already has.
//! Deliberately *not* a step-index/progress-pointer: each screen re-derives
//! its own status from real device state on mount (CLI detected? permission
//! granted? model downloaded?), so "resume from last completed screen"
//! (`01_ONBOARDING.md`'s corner-case table) falls out for free instead of
//! needing separately-maintained resume state that could drift from reality.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::State;

use crate::db::service::StorageService;
use crate::error::AppError;
use crate::ipc::python::{ModelDownloadStatus, MODEL_DOWNLOAD_PROGRESS_TOPIC};
use crate::ipc::runner::registry::{RunnerDetection, RunnerKind};
use crate::state::AppState;

const KEY_HAS_ONBOARDED: &str = "onboarding.has_onboarded";
const KEY_FIRST_NAME: &str = "onboarding.user_first_name";
const KEY_LAST_NAME: &str = "onboarding.user_last_name";
const KEY_CALENDAR_CHECKLIST_DISMISSED: &str = "onboarding.calendar_checklist_dismissed";

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct OnboardingStatus {
    pub has_onboarded: bool,
    pub user_first_name: Option<String>,
    pub user_last_name: Option<String>,
    /// Dashboard's first-run checklist (`01_ONBOARDING.md`'s "Landing"
    /// section) has two rows: "record your first conversation" — derived
    /// for free from whether any conversation exists, never stored here —
    /// and "connect your calendar," which this field tracks. Calendar
    /// *integration* itself is a later release; v1 has nothing to actually
    /// connect, so "Connect" navigates to the `/integrations` stub and
    /// clicking it is treated as satisfying the row (there is no real
    /// "connected" signal in v1 to check against instead).
    pub calendar_checklist_dismissed: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn onboarding_get_status(
    state: State<'_, AppState>,
) -> Result<OnboardingStatus, AppError> {
    let has_onboarded = state
        .storage
        .get_setting(KEY_HAS_ONBOARDED)
        .await?
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let user_first_name = state
        .storage
        .get_setting(KEY_FIRST_NAME)
        .await?
        .and_then(|v| v.as_str().map(str::to_string));
    let user_last_name = state
        .storage
        .get_setting(KEY_LAST_NAME)
        .await?
        .and_then(|v| v.as_str().map(str::to_string));
    let calendar_checklist_dismissed = state
        .storage
        .get_setting(KEY_CALENDAR_CHECKLIST_DISMISSED)
        .await?
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Ok(OnboardingStatus {
        has_onboarded,
        user_first_name,
        user_last_name,
        calendar_checklist_dismissed,
    })
}

/// Dashboard's checklist "Connect" click (or its own dismiss control) — see
/// `OnboardingStatus.calendar_checklist_dismissed`'s doc comment.
#[tauri::command]
#[specta::specta]
pub async fn onboarding_dismiss_calendar_checklist(
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    state
        .storage
        .set_setting(
            KEY_CALENDAR_CHECKLIST_DISMISSED,
            serde_json::Value::Bool(true),
        )
        .await
}

/// The Welcome screen requires a first name before either of its
/// exits (Continue, "I've used Mnemos before") proceeds — attribution
/// (`memory::self_contact`) depends on it, so the "zero form-filling"
/// `01_ONBOARDING.md` goal lost to that. `last_name` stays optional. The
/// command itself still accepts `None` for either and doesn't enforce the
/// requirement server-side: the gate is UX policy for the funnel, not an
/// invariant of stored settings, so a future editable-in-Settings path isn't
/// blocked from clearing the name if that's ever wanted.
#[tauri::command]
#[specta::specta]
pub async fn onboarding_set_user_name(
    state: State<'_, AppState>,
    first_name: Option<String>,
    last_name: Option<String>,
) -> Result<(), AppError> {
    let first = first_name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let last = last_name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match first {
        Some(v) => state.storage.set_setting(KEY_FIRST_NAME, v.into()).await?,
        None => {
            state
                .storage
                .set_setting(KEY_FIRST_NAME, serde_json::Value::Null)
                .await?
        }
    }
    match last {
        Some(v) => state.storage.set_setting(KEY_LAST_NAME, v.into()).await?,
        None => {
            state
                .storage
                .set_setting(KEY_LAST_NAME, serde_json::Value::Null)
                .await?
        }
    }
    Ok(())
}

/// Called once, landing on Dashboard — sets the flag the root route's
/// `beforeLoad` guard checks so onboarding never shows again.
#[tauri::command]
#[specta::specta]
pub async fn onboarding_complete(state: State<'_, AppState>) -> Result<(), AppError> {
    let result = state
        .storage
        .set_setting(KEY_HAS_ONBOARDED, serde_json::Value::Bool(true))
        .await;
    if result.is_ok() {
        state.metrics.track(
            crate::metrics::events::ONBOARDING_COMPLETED,
            crate::metrics::properties::EventProperties::new(),
        );
    }
    result
}

/// Screen 2 — proactive CLI detection, reusing the same PATH-scan
/// `find_claude_binary` already uses when a chat/extraction runner actually
/// spawns. Does not attempt to verify login state (see `RunnerKind::detect`'s
/// doc comment for why) — a not-logged-in CLI is caught by the existing
/// runtime error path the first time it's actually used, not gated here.
#[tauri::command]
#[specta::specta]
pub fn onboarding_check_claude_cli() -> RunnerDetection {
    RunnerKind::Claude.detect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Granted,
    Denied,
    Undetermined,
    /// Windows: v1 has no verified proactive check for this permission (see
    /// module doc). Onboarding treats this the same as `Granted` for gating
    /// purposes — never blocks Continue on something it cannot actually
    /// verify — while still rendering distinctly so the UI doesn't claim a
    /// grant that was never confirmed.
    NotApplicable,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct PermissionStatus {
    pub mic: PermissionState,
    pub screen: PermissionState,
}

#[cfg(target_os = "macos")]
mod mac_permissions {
    use super::{AppError, PermissionState, PermissionStatus};
    use crate::ipc::swift::run_permission_subcommand;
    use crate::state::AppState;
    use tauri::State;

    fn mic_state(raw: &str) -> PermissionState {
        match raw {
            "granted" => PermissionState::Granted,
            "denied" => PermissionState::Denied,
            _ => PermissionState::Undetermined,
        }
    }

    /// `CGPreflightScreenCaptureAccess` (what `check-permissions` calls)
    /// cannot distinguish "never asked" from "denied" — a real Apple API
    /// limitation, not a gap in this wiring. Only the `request` path below
    /// (which actually calls `CGRequestScreenCaptureAccess`) can tell them
    /// apart, so the check-only path never reports `Denied` for screen.
    fn screen_state(raw: &str) -> PermissionState {
        if raw == "granted" {
            PermissionState::Granted
        } else {
            PermissionState::Undetermined
        }
    }

    pub async fn check(state: &State<'_, AppState>) -> Result<PermissionStatus, AppError> {
        let v = run_permission_subcommand(state.python.sidecar_bin(), "check-permissions").await?;
        Ok(PermissionStatus {
            mic: mic_state(v.get("mic").and_then(|x| x.as_str()).unwrap_or("")),
            screen: screen_state(v.get("screen").and_then(|x| x.as_str()).unwrap_or("")),
        })
    }

    pub async fn request_mic(state: &State<'_, AppState>) -> Result<PermissionState, AppError> {
        let v =
            run_permission_subcommand(state.python.sidecar_bin(), "request-mic-permission").await?;
        Ok(mic_state(
            v.get("mic").and_then(|x| x.as_str()).unwrap_or(""),
        ))
    }

    pub async fn request_screen(state: &State<'_, AppState>) -> Result<PermissionState, AppError> {
        let v = run_permission_subcommand(state.python.sidecar_bin(), "request-screen-permission")
            .await?;
        // `CGRequestScreenCaptureAccess` cannot distinguish denied from
        // never-asked either — it never prompts synchronously for this
        // service (`tccd`: "Service kTCCServiceScreenCapture does not allow
        // prompting; returning denied") and returns `false` immediately in
        // both cases, with macOS posting its own consent alert out of band
        // and the grant only applying after a relaunch. The sidecar reports
        // that ambiguous `false` as `pending` rather than `denied`, since
        // calling the normal first-run path a hard denial would leave every
        // new user staring at a permanently "denied" row they never refused.
        // `Undetermined` keeps Continue correctly gated without asserting a
        // refusal that may not have happened; the UI routes it to System
        // Settings + relaunch, which is the real path forward either way.
        Ok(
            match v.get("screen").and_then(|x| x.as_str()).unwrap_or("") {
                "granted" => PermissionState::Granted,
                "denied" => PermissionState::Denied,
                _ => PermissionState::Undetermined,
            },
        )
    }
}

/// Windows: no verified proactive permission check exists in v1 (no Windows
/// dev machine — same gap this codebase already flags for `pyaudiowpatch`/
/// WASAPI elsewhere). Mic access is requested implicitly by the OS the first
/// time a real capture stream opens (existing `WindowsCapture` behavior);
/// Screen Recording has no Windows equivalent card at all
/// (mac-only TCC gate, already hidden from this OS by the frontend). Both
/// report `NotApplicable` rather than a guessed status, and onboarding never
/// blocks Continue on a permission it cannot verify.
#[cfg(not(target_os = "macos"))]
mod mac_permissions {
    use super::{AppError, PermissionState, PermissionStatus};
    use crate::state::AppState;
    use tauri::State;

    pub async fn check(_state: &State<'_, AppState>) -> Result<PermissionStatus, AppError> {
        Ok(PermissionStatus {
            mic: PermissionState::NotApplicable,
            screen: PermissionState::NotApplicable,
        })
    }

    pub async fn request_mic(_state: &State<'_, AppState>) -> Result<PermissionState, AppError> {
        Ok(PermissionState::NotApplicable)
    }

    pub async fn request_screen(_state: &State<'_, AppState>) -> Result<PermissionState, AppError> {
        Ok(PermissionState::NotApplicable)
    }
}

#[tauri::command]
#[specta::specta]
pub async fn onboarding_check_permissions(
    state: State<'_, AppState>,
) -> Result<PermissionStatus, AppError> {
    mac_permissions::check(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn onboarding_request_mic_permission(
    state: State<'_, AppState>,
) -> Result<PermissionState, AppError> {
    let result = mac_permissions::request_mic(&state).await;
    track_permission_result(&state, "microphone", &result);
    result
}

#[tauri::command]
#[specta::specta]
pub async fn onboarding_request_screen_permission(
    state: State<'_, AppState>,
) -> Result<PermissionState, AppError> {
    let result = mac_permissions::request_screen(&state).await;
    track_permission_result(&state, "screen_recording", &result);
    result
}

/// Windows parity audit finding #10: `granted` used to also count
/// `NotApplicable` (Windows always reports this — no verified proactive
/// permission check exists there, see `mac_permissions` above), so Windows
/// permission-grant rates read as a false 100%. `not_applicable` is now its
/// own bucket instead of being folded into `granted` — this is additive
/// (new field, `granted` narrowed to only its literal meaning) rather than
/// repurposing the existing field, so nothing downstream reading `granted`
/// silently changes meaning for a platform/build that never reports
/// `NotApplicable` (mac).
fn track_permission_result(
    state: &State<'_, AppState>,
    kind: &'static str,
    result: &Result<PermissionState, AppError>,
) {
    let Ok(state_value) = result else { return };
    state.metrics.track(
        crate::metrics::events::ONBOARDING_PERMISSION_RESULT,
        crate::metrics::properties::EventProperties::from([
            (
                "kind",
                crate::metrics::properties::PropertyValue::Enum(kind),
            ),
            (
                "granted",
                crate::metrics::properties::PropertyValue::Bool(matches!(
                    state_value,
                    PermissionState::Granted
                )),
            ),
            (
                "not_applicable",
                crate::metrics::properties::PropertyValue::Bool(matches!(
                    state_value,
                    PermissionState::NotApplicable
                )),
            ),
        ]),
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SettingsPane {
    Microphone,
    ScreenRecording,
}

/// Deep-links to the exact System Settings pane for a denied permission
/// (`01_ONBOARDING.md` Screen 3's "expandable 'How to fix in System
/// Settings'"). Best-effort: opening a settings pane is not something a
/// failure here should block onboarding on, so this never returns
/// `AppError` — it logs and no-ops instead.
#[tauri::command]
#[specta::specta]
pub fn onboarding_open_system_settings(pane: SettingsPane) {
    #[cfg(target_os = "macos")]
    let url = match pane {
        SettingsPane::Microphone => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        }
        SettingsPane::ScreenRecording => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
    };
    #[cfg(target_os = "macos")]
    let spawn_result = std::process::Command::new("open").arg(url).spawn();

    // Windows parity audit finding #11: `ms-settings:privacy-microphone` is
    // verified correct and stays. There is no Windows Settings pane
    // equivalent to macOS's Screen Recording TCC gate at all (Windows has
    // no such permission concept) — the old `ms-settings:privacy` guess
    // pointed at nothing meaningful, and per finding #10 this arm is
    // unreachable in practice anyway (`onboarding_check_permissions`
    // reports `NotApplicable` on Windows, never `Denied`, so nothing calls
    // this with `ScreenRecording` there). No-op rather than guessing
    // another URI that can't be verified without a real Windows machine.
    #[cfg(target_os = "windows")]
    let url = match pane {
        SettingsPane::Microphone => Some("ms-settings:privacy-microphone"),
        SettingsPane::ScreenRecording => None,
    };
    #[cfg(target_os = "windows")]
    let spawn_result = url.map(|url| {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", "", url]);
        crate::procutil::suppress_console_window_std(&mut command);
        command.spawn()
    });

    #[cfg(target_os = "macos")]
    if let Err(e) = spawn_result {
        tracing::warn!(error = %e, "onboarding.open_system_settings_failed");
    }
    #[cfg(target_os = "windows")]
    if let Some(Err(e)) = spawn_result {
        tracing::warn!(error = %e, "onboarding.open_system_settings_failed");
    }
}

/// Screen 4 — subscribes to `model_download_progress` and immediately pushes
/// a synchronous status snapshot first, so a screen that mounts after the
/// download already finished (or already started, due to the eager
/// `warm_up()`) doesn't sit on a stuck 0% bar waiting for a change event that
/// may never come again. No separate "start" command exists — see
/// `ModelDownloadStatus`'s doc comment for why.
#[tauri::command]
#[specta::specta]
pub async fn onboarding_subscribe_model_download(
    state: State<'_, AppState>,
    channel: Channel<crate::ipc::python::ModelDownloadStatusResponse>,
) -> Result<(), AppError> {
    if let Ok(snapshot) = state.python.send(ModelDownloadStatus {}).await {
        let _ = channel.send(snapshot);
    }

    let mut rx = state.python.subscribe(MODEL_DOWNLOAD_PROGRESS_TOPIC);
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(value) => {
                    let Ok(payload) = serde_json::from_value::<
                        crate::ipc::python::ModelDownloadStatusResponse,
                    >(value) else {
                        continue;
                    };
                    let done = payload.done;
                    if channel.send(payload).is_err() {
                        return;
                    }
                    if done {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::service::SqliteStorageService;

    async fn test_storage() -> SqliteStorageService {
        let dir = tempfile::tempdir().unwrap();
        // Leak the tempdir for the test's lifetime — dropped `TempDir`s
        // delete on drop, and this fn returns before the caller is done
        // with the db file (same convention as `commands::chat`'s own
        // `test_storage`).
        let path = Box::leak(Box::new(dir)).path().join("mnemos.db");
        let pools = crate::db::init(&path).await.expect("db init");
        SqliteStorageService::new(pools)
    }

    #[tokio::test]
    async fn status_defaults_to_not_onboarded_with_no_name() {
        let storage = test_storage().await;
        let status = get_status_direct(&storage).await;
        assert!(!status.has_onboarded);
        assert_eq!(status.user_first_name, None);
        assert_eq!(status.user_last_name, None);
        assert!(!status.calendar_checklist_dismissed);
    }

    #[tokio::test]
    async fn dismiss_calendar_checklist_persists() {
        let storage = test_storage().await;
        storage
            .set_setting(
                KEY_CALENDAR_CHECKLIST_DISMISSED,
                serde_json::Value::Bool(true),
            )
            .await
            .unwrap();
        let status = get_status_direct(&storage).await;
        assert!(status.calendar_checklist_dismissed);
    }

    #[tokio::test]
    async fn set_user_name_persists_and_trims_blank_to_none() {
        let storage = test_storage().await;
        set_user_name_direct(&storage, Some("  Mike  ".into()), Some("".into())).await;
        let status = get_status_direct(&storage).await;
        assert_eq!(status.user_first_name.as_deref(), Some("Mike"));
        assert_eq!(status.user_last_name, None);
    }

    #[tokio::test]
    async fn complete_flips_has_onboarded() {
        let storage = test_storage().await;
        complete_direct(&storage).await;
        let status = get_status_direct(&storage).await;
        assert!(status.has_onboarded);
    }

    #[test]
    fn check_claude_cli_reports_not_installed_when_absent_from_path() {
        let _guard = crate::ipc::runner::claude::path_env_test_lock().blocking_lock();
        let dir = tempfile::tempdir().unwrap();
        let old = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", dir.path()) };
        let detection = onboarding_check_claude_cli();
        if let Some(old) = old {
            unsafe { std::env::set_var("PATH", old) };
        }
        assert!(!detection.installed);
        assert_eq!(detection.path, None);
    }

    // Thin async helpers so tests call the same logic the `#[tauri::command]`
    // wrappers do without needing a real `tauri::State` extractor in a unit
    // test (mirrors this codebase's existing testable-fn convention, e.g.
    // `commands::chat`'s `get_session_history`/`start_new_session`).
    async fn get_status_direct(storage: &SqliteStorageService) -> OnboardingStatus {
        let has_onboarded = storage
            .get_setting(KEY_HAS_ONBOARDED)
            .await
            .unwrap()
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let user_first_name = storage
            .get_setting(KEY_FIRST_NAME)
            .await
            .unwrap()
            .and_then(|v| v.as_str().map(str::to_string));
        let user_last_name = storage
            .get_setting(KEY_LAST_NAME)
            .await
            .unwrap()
            .and_then(|v| v.as_str().map(str::to_string));
        let calendar_checklist_dismissed = storage
            .get_setting(KEY_CALENDAR_CHECKLIST_DISMISSED)
            .await
            .unwrap()
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        OnboardingStatus {
            has_onboarded,
            user_first_name,
            user_last_name,
            calendar_checklist_dismissed,
        }
    }

    async fn set_user_name_direct(
        storage: &SqliteStorageService,
        first_name: Option<String>,
        last_name: Option<String>,
    ) {
        let first = first_name
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let last = last_name
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        match first {
            Some(v) => storage.set_setting(KEY_FIRST_NAME, v.into()).await.unwrap(),
            None => storage
                .set_setting(KEY_FIRST_NAME, serde_json::Value::Null)
                .await
                .unwrap(),
        }
        match last {
            Some(v) => storage.set_setting(KEY_LAST_NAME, v.into()).await.unwrap(),
            None => storage
                .set_setting(KEY_LAST_NAME, serde_json::Value::Null)
                .await
                .unwrap(),
        }
    }

    async fn complete_direct(storage: &SqliteStorageService) {
        storage
            .set_setting(KEY_HAS_ONBOARDED, serde_json::Value::Bool(true))
            .await
            .unwrap();
    }
}
