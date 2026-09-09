//! Background meeting auto-detect (product_docs/MEETING_AUTO_DETECT_DESIGN.md):
//! the `meeting_detection.enabled` setting, live start/stop of the
//! `mnemos-meeting-watcher` process, and the overlay notification window
//! its `meeting_detected` signal triggers.
//!
//! macOS-only for now (the watcher process itself is), so every entry point
//! here is a no-op on other platforms rather than a compile error — the
//! settings command still needs to exist cross-platform so the frontend
//! doesn't have to special-case the toggle's presence.

use std::collections::HashMap;
use std::sync::Mutex as StdMutex;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;

use crate::db::service::StorageService;
use crate::error::AppError;
use crate::state::AppState;

pub(crate) const KEY_ENABLED: &str = "meeting_detection.enabled";
/// Dismissing a prompt suppresses further prompts for the same app for this
/// long — declining once should not mean re-nagging every debounce cycle if
/// the mic keeps toggling during the same call (design doc "Failure modes").
const COOLDOWN_MS: i64 = 5 * 60 * 1000;
const OVERLAY_WINDOW_LABEL: &str = "meeting-notification";

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_millis() as i64
}

/// In-memory only, like `commands::project::RefreshDebounce` — a restart
/// clearing cooldowns/the watcher handle is harmless, nothing here is a
/// correctness invariant across a relaunch.
#[derive(Default)]
pub struct MeetingDetectionState {
    cooldown_until_ms: StdMutex<HashMap<String, i64>>,
    #[cfg(target_os = "macos")]
    watcher_handle: StdMutex<Option<tokio::task::AbortHandle>>,
}

impl MeetingDetectionState {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MeetingDetectionSettings {
    pub enabled: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn meeting_detection_get_settings(
    state: State<'_, AppState>,
) -> Result<MeetingDetectionSettings, AppError> {
    let enabled = state
        .storage
        .get_setting(KEY_ENABLED)
        .await?
        .and_then(|v| v.as_bool())
        // Default on, matching Otter/OpenWhispr's default-enabled behavior —
        // the whole point is detection that "just works" without a setup
        // step; a user who doesn't want it turns it off once.
        .unwrap_or(true);
    Ok(MeetingDetectionSettings { enabled })
}

#[tauri::command]
#[specta::specta]
pub async fn meeting_detection_set_enabled(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), AppError> {
    state
        .storage
        .set_setting(KEY_ENABLED, serde_json::Value::Bool(enabled))
        .await?;
    #[cfg(target_os = "macos")]
    {
        if enabled {
            start_watcher(&app);
        } else {
            stop_watcher(&app);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
    Ok(())
}

/// Overlay's Start Recording button. Deliberately does **not** call
/// `commands::recording::start_recording` directly — per AGENTS.md
/// ("Starting a recording is a frontend-owned sequence, not a single
/// command"), that would record audio while the store still reads `idle`
/// and no session would show anywhere. Instead this reuses the exact same
/// `TrayStartRecording` event the tray's own "start from outside the
/// frontend" path already emits — one implementation of "start recording
/// from outside the app's own UI", not a second one.
///
/// Always unfiled (`project_id: None`) — the overlay had a project picker
/// once, removed because clicking anywhere on this window (a native
/// `<select>` included) activates the whole app regardless of window
/// `focusable`/`always_on_top` settings, a macOS AppKit behavior with no
/// public Tauri/tao escape hatch short of a raw-Cocoa `NSPanel` rewrite —
/// so a project chooser here bought nothing a plain click didn't already
/// cost. Filing into a project is still one click away, from the main
/// window this brings forward.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notification_start_recording(app: AppHandle) -> Result<(), AppError> {
    crate::commands::tray::show_main_window(&app);
    let _ = crate::events::TrayStartRecording { project_id: None }.emit(&app);
    close_overlay(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn meeting_notification_dismiss(
    app: AppHandle,
    meeting_state: State<'_, MeetingDetectionState>,
    bundle_id: String,
) -> Result<(), AppError> {
    meeting_state
        .cooldown_until_ms
        .lock()
        .expect("cooldown mutex poisoned")
        .insert(bundle_id, now_ms() + COOLDOWN_MS);
    close_overlay(&app);
    Ok(())
}

/// The overlay sizes its own window: the card's height is decided by CSS
/// (icon, two lines of text, a button row), and the webview is the only
/// place that knows what that came out to. Called on mount and whenever the
/// card resizes, so the window is exactly as tall as its content instead of
/// whatever number was last hardcoded on this side.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notification_resize(app: AppHandle, height: f64) -> Result<(), AppError> {
    let Some(win) = app.get_webview_window(OVERLAY_WINDOW_LABEL) else {
        return Ok(());
    };
    if !height.is_finite() || height <= 0.0 {
        return Ok(());
    }
    let height = height.min(OVERLAY_MAX_HEIGHT).ceil();
    // Width is fixed and the window is anchored top-right, so growing
    // downward keeps it in place — no reposition needed.
    let _ = win.set_size(tauri::LogicalSize::new(OVERLAY_WIDTH, height));
    Ok(())
}

fn close_overlay(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(OVERLAY_WINDOW_LABEL) {
        let _ = win.close();
    }
}

#[cfg(target_os = "macos")]
pub fn start_watcher(app: &AppHandle) {
    let Some(bin) = crate::ipc::meeting_watcher::resolve_bin() else {
        tracing::warn!("meeting_detection.watcher_binary_not_found");
        return;
    };
    stop_watcher(app); // idempotent: never run two supervisors at once
    let app_for_events = app.clone();
    let handle = crate::ipc::meeting_watcher::spawn_supervised(bin, move |event| {
        on_watcher_event(&app_for_events, event);
    });
    let state = app.state::<MeetingDetectionState>();
    *state
        .watcher_handle
        .lock()
        .expect("watcher handle mutex poisoned") = Some(handle);
}

#[cfg(target_os = "macos")]
pub fn stop_watcher(app: &AppHandle) {
    let state = app.state::<MeetingDetectionState>();
    let taken = state
        .watcher_handle
        .lock()
        .expect("watcher handle mutex poisoned")
        .take();
    if let Some(handle) = taken {
        handle.abort();
    }
}

#[cfg(target_os = "macos")]
fn on_watcher_event(app: &AppHandle, event: crate::ipc::meeting_watcher::MeetingEvent) {
    use crate::ipc::meeting_watcher::MeetingEvent;
    match event {
        MeetingEvent::Detected { service, bundle_id } => {
            maybe_show_overlay(app, &service, &bundle_id)
        }
        MeetingEvent::Ended { bundle_id, .. } => {
            // Only close a still-open overlay for the exact app that just
            // stopped — a stale "Meeting detected" prompt sitting there
            // after the user has already left the call is worse than no
            // prompt at all.
            if let Some(win) = app.get_webview_window(OVERLAY_WINDOW_LABEL) {
                let matches = win
                    .url()
                    .ok()
                    .is_some_and(|u| u.query().is_some_and(|q| q.contains(&bundle_id)));
                if matches {
                    let _ = win.close();
                }
            }
        }
    }
}

/// The overlay window's width — fixed, and named once so the size passed to
/// `.inner_size()` and the width used to position it against the screen's
/// right edge can never independently drift, the way they already had
/// (`inner_size` had been narrowed from 360 to fit the simplified layout;
/// the position math's own hardcoded `360.0` was not).
const OVERLAY_WIDTH: f64 = 320.0;
/// Only the height the window is *born* with, before the webview has laid
/// its card out. The real height comes from `meeting_notification_resize`
/// below — a hardcoded height here and a card whose content decides its own
/// height in CSS is exactly the drift that left a tall window with a band of
/// empty white under the buttons. Keep this close to the expected height so
/// the corrective resize is imperceptible rather than a visible jump.
const OVERLAY_INITIAL_HEIGHT: f64 = 98.0;
/// A card taller than this is a layout bug (or a hostile `service` string),
/// not something to hand a window size from.
const OVERLAY_MAX_HEIGHT: f64 = 400.0;

#[cfg(target_os = "macos")]
fn maybe_show_overlay(app: &AppHandle, service: &str, bundle_id: &str) {
    let state = app.state::<AppState>();
    if state.recording.has_active_session() {
        return; // already recording — nothing to offer
    }
    if app.get_webview_window(OVERLAY_WINDOW_LABEL).is_some() {
        return; // one at a time (design doc "Two overlays racing")
    }
    let meeting_state = app.state::<MeetingDetectionState>();
    let in_cooldown = meeting_state
        .cooldown_until_ms
        .lock()
        .expect("cooldown mutex poisoned")
        .get(bundle_id)
        .is_some_and(|&until| now_ms() < until);
    if in_cooldown {
        return;
    }

    let url = format!(
        "index.html?view=meeting-notification&service={}&bundle_id={}",
        urlencoding_light(service),
        urlencoding_light(bundle_id)
    );
    let mut builder = tauri::WebviewWindowBuilder::new(
        app,
        OVERLAY_WINDOW_LABEL,
        tauri::WebviewUrl::App(url.into()),
    )
    .decorations(false)
    // Without this the window itself is opaque and square — the card's own
    // CSS border-radius has nothing to show against, since the area outside
    // the rounded div is the same solid color as the window behind it, not
    // the desktop. Requires the `macos-private-api` Cargo feature +
    // `macOSPrivateApi: true` in tauri.conf.json on macOS (Mac App Store
    // review policy only — irrelevant to Mnemos's direct/DMG distribution).
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .shadow(false)
    // Reduces, but doesn't eliminate, the window becoming key on click —
    // it does nothing about the separate AppKit behavior where a mouse
    // click on *any* window belonging to the app activates the whole app
    // and brings its other windows forward (no public Tauri/tao option
    // controls that; it would need a raw-Cocoa `NSPanel` rewrite). Buttons
    // still receive clicks fine either way — the overlay's project picker
    // was removed for exactly this reason, not worked around.
    .focusable(false)
    .inner_size(OVERLAY_WIDTH, OVERLAY_INITIAL_HEIGHT)
    .visible(false); // shown once positioned, avoids a flash at the wrong spot

    if let Ok(Some(monitor)) = app.primary_monitor() {
        // Work area, not `size()` — the latter includes the menu bar, so a
        // 16px top margin put the card *under* it and left macOS to clamp
        // the window down by an unpredictable amount.
        let area = monitor.work_area();
        let scale = monitor.scale_factor();
        let margin = (16.0 * scale) as i32;
        let width_px = (OVERLAY_WIDTH * scale) as i32;
        let x = area.position.x + area.size.width as i32 - width_px - margin;
        let y = area.position.y + margin;
        builder = builder.position(x as f64 / scale, y as f64 / scale);
    }

    match builder.build() {
        Ok(win) => {
            let _ = win.show();
        }
        Err(e) => tracing::warn!(error = %e, "meeting_detection.overlay_create_failed"),
    }
}

/// Query-string escaping for the two values this ever needs to carry — both
/// are bundle IDs / short service names (`[a-zA-Z0-9._-]`), so a full
/// percent-encoding crate is unneeded weight for this one call site.
#[cfg(target_os = "macos")]
fn urlencoding_light(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c.to_string()
            } else {
                format!("%{:02X}", c as u32)
            }
        })
        .collect()
}
