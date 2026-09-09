//! Menu-bar / system-tray icon and menu.
//!
//! Two rules shape everything here, both from [AGENTS.md](../../../AGENTS.md):
//!
//! **The tray is a view of recording state, never a second source of it.**
//! [`refresh`] reads `RecordingRegistry` and renders the icon, menu, tooltip
//! and title from what it finds; callers only say *"state changed"*, never
//! *"show a red icon"*. The previous shape — a `set_tray_recording(bool)`
//! every caller had to remember — had already produced a bug where an early
//! `?` between removing a session and resetting the icon left the menu bar
//! claiming a recording that had already stopped.
//!
//! **Menu actions that start or change a recording go through the frontend.**
//! Starting a recording is a sequence the React side owns (arm → command →
//! mark → subscribe, plus the "still transcribing" confirmation gate), so the
//! tray focuses the window and emits an event that the existing hooks handle,
//! rather than calling `start_recording` behind their back. Only the two
//! actions with no frontend state to keep in sync — Show and Quit — are
//! handled natively here.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::db::models::ProjectFilter;
use crate::db::service::StorageService;
use crate::error::AppError;
use crate::state::AppState;

/// Looked up by [`refresh`] via `app.tray_by_id`.
const TRAY_ID: &str = "mnemos-tray";

/// Menu item ids. `ID_START_IN_PROJECT_PREFIX` is followed by a project id —
/// the only id parsed rather than matched, since those items are built from
/// whatever projects exist.
const ID_START: &str = "tray.start";
const ID_START_IN_PROJECT_PREFIX: &str = "tray.start-in-project:";
const ID_PAUSE: &str = "tray.pause";
const ID_RESUME: &str = "tray.resume";
const ID_STOP: &str = "tray.stop";
const ID_SHOW: &str = "tray.show";
const ID_QUIT: &str = "tray.quit";

/// Rendered at 18pt tall by `tray-icon` (it scales the bitmap to the menu
/// bar's height, preserving aspect), so these are deliberately oversized —
/// 140×100 is roughly 5.5×, crisp on every current display density.
///
/// Generated from `public/mnemos-mark.svg` — the same Mnemos M the app shell
/// draws — but *not* at the shell's proportions. The app-shell mark is drawn
/// with 216-unit strokes filling its viewBox edge to edge, which in a menu
/// bar reads as both too heavy and too large next to system glyphs. These use
/// 150-unit strokes and a generous viewBox inset, putting the mark at ~11.9pt
/// tall with ~2.7pt strokes inside the 18pt slot — in the weight range macOS
/// status items sit at.
///
/// `idle` is pure black + alpha because macOS renders it as a *template*
/// image and tints it to match the menu bar; the other two carry brand colour
/// and so cannot be templates. Their colours mirror `src/design/tokens.css`
/// (`--recording`, `--accent-primary`).
const ICON_IDLE: &[u8] = include_bytes!("../../icons/tray/idle.png");
const ICON_RECORDING: &[u8] = include_bytes!("../../icons/tray/recording.png");
const ICON_PAUSED: &[u8] = include_bytes!("../../icons/tray/paused.png");

/// What the tray should be showing, derived wholly from the registry.
///
/// Built by `RecordingRegistry::tray_snapshot`, which owns the derivation so
/// the session lock is never held across an `.await` in here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrayState {
    Idle,
    /// `elapsed_s` ticks with the clock.
    Recording {
        elapsed_s: i64,
    },
    /// `elapsed_s` is frozen at the moment of the pause, matching the
    /// frontend's timer (`useRecordingTick` stops ticking while paused).
    Paused {
        elapsed_s: i64,
    },
}

impl TrayState {
    /// Which menu layout this state needs. Two states with the same
    /// discriminant differ only in elapsed seconds, which lives in the title
    /// and tooltip — never in the menu — so the menu can be left alone.
    fn discriminant(self) -> u8 {
        match self {
            TrayState::Idle => 0,
            TrayState::Recording { .. } => 1,
            TrayState::Paused { .. } => 2,
        }
    }
}

/// `m:ss`, matching the frontend's `formatDuration` so the menu bar and the
/// in-app timer never disagree on how the same number is written.
fn format_elapsed(total_s: i64) -> String {
    let total_s = total_s.max(0);
    format!("{}:{:02}", total_s / 60, total_s % 60)
}

/// Reads the registry. The only place tray state is decided.
fn snapshot(app: &AppHandle) -> TrayState {
    let state = app.state::<AppState>();
    state.recording.tray_snapshot()
}

/// Builds the menu for `state`. Rebuilt (rather than mutated item-by-item)
/// on a discriminant change: the layouts differ in which items exist at all,
/// and holding `MenuItem` handles across threads isn't possible anyway.
async fn build_menu(app: &AppHandle, state: TrayState) -> Result<Menu<tauri::Wry>, AppError> {
    let menu = Menu::new(app).map_err(tray_err)?;

    match state {
        TrayState::Idle => {
            menu.append(
                &MenuItem::with_id(app, ID_START, "Start Recording", true, None::<&str>)
                    .map_err(tray_err)?,
            )
            .map_err(tray_err)?;

            // Only worth a submenu if there's something in it — an empty
            // "Start Recording in ▸" reads as broken.
            let projects = app
                .state::<AppState>()
                .storage
                .list_projects(ProjectFilter {
                    include_archived: false,
                    limit: None,
                    offset: 0,
                })
                .await?;
            if !projects.is_empty() {
                let submenu = Submenu::new(app, "Start Recording in", true).map_err(tray_err)?;
                for project in &projects {
                    let item = MenuItem::with_id(
                        app,
                        format!("{ID_START_IN_PROJECT_PREFIX}{}", project.id),
                        &project.name,
                        true,
                        None::<&str>,
                    )
                    .map_err(tray_err)?;
                    submenu.append(&item).map_err(tray_err)?;
                }
                menu.append(&submenu).map_err(tray_err)?;
            }
        }
        TrayState::Recording { .. } => {
            menu.append(
                &MenuItem::with_id(app, ID_PAUSE, "Pause Recording", true, None::<&str>)
                    .map_err(tray_err)?,
            )
            .map_err(tray_err)?;
            menu.append(
                &MenuItem::with_id(app, ID_STOP, "Stop & Save", true, None::<&str>)
                    .map_err(tray_err)?,
            )
            .map_err(tray_err)?;
        }
        TrayState::Paused { .. } => {
            menu.append(
                &MenuItem::with_id(app, ID_RESUME, "Resume Recording", true, None::<&str>)
                    .map_err(tray_err)?,
            )
            .map_err(tray_err)?;
            menu.append(
                &MenuItem::with_id(app, ID_STOP, "Stop & Save", true, None::<&str>)
                    .map_err(tray_err)?,
            )
            .map_err(tray_err)?;
        }
    }

    menu.append(&PredefinedMenuItem::separator(app).map_err(tray_err)?)
        .map_err(tray_err)?;
    menu.append(
        &MenuItem::with_id(app, ID_SHOW, "Show Mnemos", true, None::<&str>).map_err(tray_err)?,
    )
    .map_err(tray_err)?;
    menu.append(&MenuItem::with_id(app, ID_QUIT, "Quit", true, None::<&str>).map_err(tray_err)?)
        .map_err(tray_err)?;

    Ok(menu)
}

/// Icon + template flag for a state. Only the idle mark is a template image:
/// the other two exist precisely to carry a colour macOS would otherwise
/// tint away.
fn icon_for(state: TrayState) -> (&'static [u8], bool) {
    match state {
        TrayState::Idle => (ICON_IDLE, true),
        TrayState::Recording { .. } => (ICON_RECORDING, false),
        TrayState::Paused { .. } => (ICON_PAUSED, false),
    }
}

/// The menu-bar title and tooltip for a state.
///
/// Idle's title is an empty string, never `None`, and that is load-bearing:
/// `set_title(None)` looks like the way to clear the clock and silently
/// isn't. `tray-icon`'s macOS `set_title_inner` is `if let Some(title) =
/// title`, so `None` never reaches `setTitle` and whatever was there stays in
/// the menu bar for the life of the process — stopping a recording left its
/// final elapsed time frozen up there because of it. Clearing means writing
/// an empty title, so this returns one.
///
/// Split out from [`apply_text`] so that rule is testable without a live
/// tray, which is the only reason the bug above could ship unnoticed.
fn text_for(state: TrayState) -> (String, String) {
    match state {
        TrayState::Idle => (String::new(), "Mnemos".to_string()),
        TrayState::Recording { elapsed_s } => {
            let t = format_elapsed(elapsed_s);
            (t.clone(), format!("Mnemos — Recording {t}"))
        }
        TrayState::Paused { elapsed_s } => {
            let t = format_elapsed(elapsed_s);
            (t.clone(), format!("Mnemos — Paused {t}"))
        }
    }
}

/// Title and tooltip. Cheap enough to reapply every tick; the menu is not,
/// which is why elapsed time lives only here.
fn apply_text(tray: &TrayIcon, state: TrayState) {
    let (title, tooltip) = text_for(state);

    // The menu-bar title is macOS-only: Tauri documents it as unsupported on
    // Windows, and on Linux it only renders in some panels. Everywhere else
    // the same information is in the tooltip and the menu.
    #[cfg(target_os = "macos")]
    let _ = tray.set_title(Some(&title));
    #[cfg(not(target_os = "macos"))]
    let _ = &title;

    let _ = tray.set_tooltip(Some(&tooltip));
}

/// Re-renders the tray from current registry state.
///
/// Safe and cheap to call on any state change, and called once a second
/// while a recording is in flight. The menu is rebuilt only when the *kind*
/// of state changed — rebuilding it every tick would fight with an open
/// menu, and nothing in it depends on the clock.
pub async fn refresh(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let state = snapshot(app);

    // `Ordering::Relaxed` is enough: this is a render cache, and the only
    // cost of a stale read is one redundant menu rebuild.
    use std::sync::atomic::{AtomicU8, Ordering};
    static LAST_DISCRIMINANT: AtomicU8 = AtomicU8::new(u8::MAX);
    let discriminant = state.discriminant();
    if LAST_DISCRIMINANT.swap(discriminant, Ordering::Relaxed) != discriminant {
        let (bytes, is_template) = icon_for(state);
        match tauri::image::Image::from_bytes(bytes) {
            Ok(icon) => {
                let _ = tray.set_icon(Some(icon));
                let _ = tray.set_icon_as_template(is_template);
            }
            Err(err) => tracing::warn!(error = %err, "tray.icon_decode_failed"),
        }
        match build_menu(app, state).await {
            Ok(menu) => {
                let _ = tray.set_menu(Some(menu));
            }
            Err(err) => tracing::warn!(error = %err, "tray.menu_build_failed"),
        }
    }

    apply_text(&tray, state);
}

/// Shows and focuses the main window. Also the first half of every menu
/// action that hands off to the frontend — an event is useless if the user
/// can't see what it did.
///
/// Public because the Dock reopen handler in `lib.rs` needs the same thing:
/// closing the window hides it rather than destroying it (see
/// `CloseRequested` in [`build`]), so "bring Mnemos back" has exactly one
/// implementation whatever asks for it — tray item, Dock icon, or
/// `cmd-tab`.
pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Reveals the window once the frontend has actually painted.
///
/// The window is created hidden (`tauri.conf.json`'s `"visible": false`).
/// Shown at creation instead, the user watches an empty 1200x800 frame
/// appear, jump to maximized as the saved geometry is restored, and only then
/// fill in — which reads as the app opening twice.
///
/// Idempotent: [`show_main_window`] is a no-op on an already-visible window,
/// so the launch fallback in `lib.rs` racing the frontend costs nothing.
#[tauri::command]
#[specta::specta]
pub fn app_ready(app: AppHandle) {
    show_main_window(&app);
}

/// Quit, guarded.
///
/// A recording in flight is the one thing a quit must not silently discard,
/// so this hands off to the frontend to confirm (and to stop the recording
/// through the same path the in-app Stop button uses). With nothing
/// recording it exits directly, shutting the worker down first so the warm
/// Parakeet process this app deliberately keeps alive across a window close
/// isn't orphaned.
fn quit(app: &AppHandle) {
    if app.state::<AppState>().recording.has_active_session() {
        show_main_window(app);
        let _ = crate::events::TrayConfirmQuit.emit(app);
        return;
    }
    shutdown_and_exit(app);
}

/// The unguarded exit. Public so `commands::tray::quit_confirmed` — the
/// command the confirmation dialog calls once it has stopped the recording —
/// reaches the same shutdown path rather than duplicating it.
fn shutdown_and_exit(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        if let Err(err) = state.python.shutdown().await {
            tracing::warn!(error = %err, "tray.quit_shutdown_failed");
        }
        app.exit(0);
    });
}

/// Called by the quit-confirmation dialog after it has dealt with the
/// in-flight recording (stopped it, or decided to abandon it). Deliberately
/// unguarded: the guard already ran, and the user answered it.
#[tauri::command]
#[specta::specta]
pub async fn tray_quit_confirmed(app: AppHandle) -> Result<(), AppError> {
    shutdown_and_exit(&app);
    Ok(())
}

/// Routes a menu click. Everything that touches a recording is forwarded to
/// the frontend; see this module's header for why.
fn on_menu_event(app: &AppHandle, id: &str) {
    match id {
        ID_SHOW => show_main_window(app),
        ID_QUIT => quit(app),
        ID_START => {
            show_main_window(app);
            let _ = crate::events::TrayStartRecording { project_id: None }.emit(app);
        }
        ID_PAUSE => {
            let _ = crate::events::TrayPauseRecording.emit(app);
        }
        ID_RESUME => {
            let _ = crate::events::TrayResumeRecording.emit(app);
        }
        ID_STOP => {
            show_main_window(app);
            let _ = crate::events::TrayStopRecording.emit(app);
        }
        other => {
            if let Some(project_id) = other.strip_prefix(ID_START_IN_PROJECT_PREFIX) {
                show_main_window(app);
                let _ = crate::events::TrayStartRecording {
                    project_id: Some(project_id.to_string()),
                }
                .emit(app);
            }
        }
    }
}

/// Builds the tray at startup and wires the hide-on-close lifecycle: closing
/// the main window hides it so the already-warm worker survives, and only
/// Quit above actually exits. Called from `lib.rs`'s `setup()`.
pub fn build(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let idle = tauri::image::Image::from_bytes(ICON_IDLE)?;
    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(idle)
        .tooltip("Mnemos")
        // Left click opens the menu, which is the platform convention for a
        // status item and the only way the recording controls are reachable
        // without hunting for a right click.
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| on_menu_event(app, event.id.as_ref()))
        .build(app)?;
    let _ = tray.set_icon_as_template(true);

    if let Some(window) = app.get_webview_window("main") {
        let hide_target = window.clone();
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = hide_target.hide();
            }
        });
    }

    // Render the real initial state (and the idle menu, which `TrayIconBuilder`
    // has no async access to build) once the app handle exists.
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move { refresh(&handle).await });

    // One long-lived ticker rather than a task spawned per recording: there
    // is no start/stop race to get wrong, and a tick that finds no session
    // costs one mutex acquisition. It is what advances the menu-bar clock.
    let ticker = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            interval.tick().await;
            if ticker.state::<AppState>().recording.has_active_session() {
                refresh(&ticker).await;
            }
        }
    });

    Ok(())
}

fn tray_err(err: tauri::Error) -> AppError {
    AppError::internal(format!("tray: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_formats_like_the_frontend() {
        assert_eq!(format_elapsed(0), "0:00");
        assert_eq!(format_elapsed(9), "0:09");
        assert_eq!(format_elapsed(61), "1:01");
        assert_eq!(format_elapsed(600), "10:00");
        assert_eq!(format_elapsed(3661), "61:01");
    }

    /// A negative elapsed can only come from a clock adjustment mid-recording;
    /// it must render as `0:00`, never `-1:-1`.
    #[test]
    fn elapsed_clamps_negative() {
        assert_eq!(format_elapsed(-5), "0:00");
    }

    /// The regression that shipped once: going idle has to *write* an empty
    /// title, because the only other way to express "no title" — `None` —
    /// is silently ignored by `tray-icon` on macOS and leaves the last
    /// elapsed time frozen in the menu bar.
    #[test]
    fn going_idle_clears_the_clock_rather_than_leaving_it() {
        let (title, tooltip) = text_for(TrayState::Idle);
        assert_eq!(title, "", "idle must write an empty title, not skip it");
        assert_eq!(tooltip, "Mnemos");
    }

    #[test]
    fn recording_and_paused_put_the_clock_in_the_title() {
        let (title, tooltip) = text_for(TrayState::Recording { elapsed_s: 61 });
        assert_eq!(title, "1:01");
        assert_eq!(tooltip, "Mnemos — Recording 1:01");

        let (title, tooltip) = text_for(TrayState::Paused { elapsed_s: 61 });
        assert_eq!(title, "1:01");
        assert_eq!(
            tooltip, "Mnemos — Paused 1:01",
            "paused must be distinguishable from recording in the tooltip"
        );
    }

    #[test]
    fn only_the_kind_of_state_drives_a_menu_rebuild() {
        assert_eq!(
            TrayState::Recording { elapsed_s: 1 }.discriminant(),
            TrayState::Recording { elapsed_s: 999 }.discriminant()
        );
        assert_ne!(
            TrayState::Recording { elapsed_s: 1 }.discriminant(),
            TrayState::Paused { elapsed_s: 1 }.discriminant()
        );
        assert_ne!(
            TrayState::Idle.discriminant(),
            TrayState::Recording { elapsed_s: 0 }.discriminant()
        );
    }
}
