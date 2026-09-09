//! Mnemos Tauri host. `main.rs` is a shim over `run()`; everything real lives
//! here so the crate stays testable and mobile-ready.

pub mod capture;
pub mod commands;
pub mod db;
pub mod error;
pub mod events;
pub mod fs;
pub mod ipc;
pub mod logging;
pub mod memory;
pub mod metrics;
pub mod procutil;
pub mod state;

use tauri::Manager;
use tauri_specta::{collect_commands, collect_events, Builder};

use crate::error::AppError;
use crate::state::AppState;

/// Where the generated TypeScript bindings land, relative to `src-tauri/`.
/// Checked into git; CI fails if a regen produces a diff.
pub const BINDINGS_PATH: &str = "../bindings/tauri.ts";

/// The single specta registry. Every command and event is registered here
/// exactly once — `collect_commands!` also feeds Tauri's `invoke_handler`, so
/// the two lists cannot drift.
pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::ping,
            commands::recording::start_recording,
            commands::recording::stop_recording,
            commands::recording::subscribe_transcript,
            commands::recording::unsubscribe_transcript,
            commands::recording::subscribe_mic_level,
            commands::recording::unsubscribe_mic_level,
            commands::recording::pause_recording,
            commands::recording::resume_recording,
            commands::recording::list_interrupted_recordings,
            commands::recording::discard_interrupted_recording,
            commands::recording::recover_interrupted_recording,
            commands::recording::list_stuck_processing,
            commands::recording::discard_stuck_processing,
            commands::recording::resume_stuck_processing,
            commands::conversation::conversation_retry_step,
            commands::conversation::get_conversation_detail,
            commands::conversation::conversation_set_action_item_done,
            commands::conversation::conversation_set_title,
            commands::conversation::conversation_set_notes,
            commands::conversation::conversation_delete,
            commands::conversation::conversation_create_action_item,
            commands::conversation::list_conversations,
            commands::conversation::conversation_set_project,
            commands::project::project_refresh_memory,
            commands::project::list_projects,
            commands::project::create_project,
            commands::project::get_project,
            commands::project::project_set_name,
            commands::project::get_project_memory,
            commands::conversation::count_conversations,
            commands::conversation::create_standalone_action_item,
            commands::conversation::list_my_action_items,
            commands::conversation::set_action_item_assignee,
            commands::conversation::set_open_question_owner,
            commands::conversation::set_open_question_resolved,
            commands::conversation::conversation_set_summary,
            commands::conversation::conversation_delete_extraction_item,
            commands::conversation::conversation_restore_extraction_item,
            commands::conversation::conversation_set_extraction_text,
            commands::project::dashboard_get_project_pulse,
            commands::project::project_get_memory_status,
            commands::project::project_list_action_items,
            commands::project::project_list_decisions,
            commands::project::project_list_open_questions,
            commands::chat::chat_send_prompt,
            commands::chat::chat_cancel_turn,
            commands::chat::chat_get_session_history,
            commands::chat::chat_delete_session,
            commands::chat::chat_rename_session,
            commands::chat::chat_list_sessions,
            commands::chat::chat_resolve_session,
            commands::models::list_transcription_models,
            commands::onboarding::onboarding_get_status,
            commands::onboarding::onboarding_set_user_name,
            commands::onboarding::onboarding_complete,
            commands::onboarding::onboarding_dismiss_calendar_checklist,
            commands::onboarding::onboarding_check_claude_cli,
            commands::onboarding::onboarding_check_permissions,
            commands::onboarding::onboarding_request_mic_permission,
            commands::onboarding::onboarding_request_screen_permission,
            commands::onboarding::onboarding_open_system_settings,
            commands::onboarding::onboarding_subscribe_model_download,
            commands::metrics::track_event,
            commands::tray::tray_quit_confirmed,
            commands::tray::app_ready,
            commands::updater::updater_check_now,
            commands::updater::updater_install_and_relaunch,
            commands::updater::updater_get_settings,
            commands::updater::updater_set_auto_check_enabled,
            commands::meeting_detection::meeting_detection_get_settings,
            commands::meeting_detection::meeting_detection_set_enabled,
            commands::meeting_detection::meeting_notification_start_recording,
            commands::meeting_detection::meeting_notification_dismiss,
            commands::meeting_detection::meeting_notification_resize,
        ])
        .events(collect_events![
            events::TrayConfirmQuit,
            events::TrayPauseRecording,
            events::TrayResumeRecording,
            events::TrayStopRecording,
            events::TrayStartRecording,
            events::LiveTranscriptionWarmup,
            events::RecordingWarning,
            events::ProjectMemoryUpdated,
            events::ConversationReady,
            events::ProcessingProgress,
            events::ProjectMemoryRefreshFailed,
        ])
}

/// Resolves the Python worker's working directory at runtime instead of
/// baking in a compile-time developer path.
///
/// TODO(packaging): this only fixes the "runs from a built binary anywhere
/// on disk, with `src-python/` copied alongside it" case — it does NOT wire
/// up real production packaging. A proper fix still needs a bundled Python
/// interpreter shipped via `externalBin`/`resources` in `tauri.conf.json`,
/// which is a much larger effort requiring testing against an
/// actual built bundle. Until that lands, anyone who copies the built
/// executable without also copying `src-python/` next to it (in the layout
/// this function expects) still gets a worker that fails to start — this
/// change does not claim to solve that; it only stops the binary from
/// hard-coding this machine's absolute source path (Windows parity audit
/// finding #9).
///
/// Release layout: `<exe_dir>/src-python`, i.e. `src-python/` copied next to
/// the built executable (sibling directory, not a subdirectory of it).
/// Debug layout: falls back to `$CARGO_MANIFEST_DIR/../src-python` (the
/// dev-tree layout `cargo tauri dev` runs from) if the sibling directory
/// isn't found next to the dev binary (e.g. `target/debug/`).
fn worker_cwd() -> Result<std::path::PathBuf, AppError> {
    let exe = std::env::current_exe()
        .map_err(|e| AppError::internal(format!("resolve current_exe: {e}")))?;
    let exe_dir = exe
        .parent()
        .ok_or_else(|| AppError::internal("current_exe has no parent directory"))?;
    let sibling = exe_dir.join("src-python");
    if sibling.is_dir() {
        return Ok(sibling);
    }

    #[cfg(debug_assertions)]
    {
        let dev_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src-python");
        if dev_path.is_dir() {
            return Ok(dev_path);
        }
    }

    // Neither layout matched. Return the sibling-of-exe path anyway (rather
    // than erroring here) so the failure surfaces as the worker's normal
    // spawn-failure/restart-backoff path with a clear "not found" error,
    // instead of a distinct code path here.
    Ok(sibling)
}

/// Locates a locally-built bundle from `PACKAGING_DESIGN.md` section A, if
/// one exists — `src-tauri/bundled/python-<platform>-<arch>/`, either as a
/// sibling of the running exe (the shape a real packaged resource dir will
/// eventually have) or at its dev-tree location (so the bundle can be
/// exercised via a plain `cargo build`/`cargo tauri dev` before real Tauri
/// resource bundling — `tauri.conf.json`'s `bundle.resources` — is wired
/// up; that's a separate, later step, not done here). Returns `None` when
/// no bundle has been built, which is the common case today.
fn bundled_python_dir() -> Option<std::path::PathBuf> {
    let name = if cfg!(windows) {
        "python-windows-x64"
    } else if cfg!(target_os = "macos") {
        "python-macos-arm64"
    } else {
        return None;
    };
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join(name));
        }
    }
    // Dev-tree fallback only — `CARGO_MANIFEST_DIR` is a compile-time
    // constant baked into the binary at build time. Left ungated here
    // (unlike `worker_cwd`'s equivalent fallback a few lines above), a
    // release build carries whichever machine built it's literal path
    // permanently, and stats it on every launch — on macOS, if that
    // path happens to sit under the build machine's `~/Documents` (a
    // common repo location, e.g. iCloud-synced Documents or GitHub
    // Desktop's default clone folder), that stat trips a real TCC
    // Documents-access prompt on every install built from such a
    // machine, not just this dev machine. Found and fixed after a real
    // build from this exact repo location reproduced exactly that
    // prompt.
    #[cfg(debug_assertions)]
    candidates.push(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("bundled")
            .join(name),
    );
    candidates.into_iter().find(|p| p.is_dir())
}

/// Worker config: PACKAGING_DESIGN.md section B, now actually wired up.
///
/// Resolution order for the Python interpreter dir:
/// 1. The real packaged resource dir (`app.path().resolve("python",
///    BaseDirectory::Resource)`) — what a `pnpm tauri build` bundle
///    actually has, once `tauri.<platform>.conf.json` declares
///    `bundle.resources` pointing `bundled/python-<platform>-<arch>/` at
///    `python/`. This is the only branch a real distributed build uses.
/// 2. `bundled_python_dir()` — a locally-built bundle sitting next to the
///    exe or in the dev tree, for exercising a bundle before wiring up
///    real Tauri resource bundling (or in case `resolve` fails to find one,
///    e.g. a platform whose resources config doesn't exist yet).
/// 3. `python3`/`python` on PATH running `src-python/` in place — the
///    original dev-mode behavior, unconditional fallback so dev/CI keeps
///    working with no bundle present at all.
fn worker_config(app: &tauri::AppHandle) -> Result<crate::ipc::python::SupervisorConfig, AppError> {
    use tauri::path::BaseDirectory;

    let resource_dir = app
        .path()
        .resolve("python", BaseDirectory::Resource)
        .ok()
        .filter(|p| p.is_dir());

    let (python_bin, cwd) = match resource_dir.or_else(bundled_python_dir) {
        Some(dir) => {
            let bin = if cfg!(windows) {
                dir.join("python.exe")
            } else {
                dir.join("bin").join("python3")
            };
            (bin, dir)
        }
        None => {
            let bin = std::path::PathBuf::from(if cfg!(windows) { "python" } else { "python3" });
            (bin, worker_cwd()?)
        }
    };
    let state_dir = crate::fs::paths::state_dir()?;
    #[allow(unused_mut)]
    let mut cfg = crate::ipc::python::SupervisorConfig::new(python_bin, cwd, state_dir);
    // Swift audio sidecar: `bundle.externalBin` (PACKAGING_DESIGN.md section
    // D, wired in `tauri.macos.conf.json`) places this binary next to the
    // main executable itself (`Contents/MacOS/`), NOT under
    // `Contents/Resources/` — unlike the Python interpreter above, this is
    // not a `BaseDirectory::Resource` lookup. Verified against a real
    // packaged `.app`: `Contents/Resources/` only had `python/` and
    // `icon.icns`; `mnemos-audio` was sitting in `Contents/MacOS/` beside
    // `mnemos-tauri`. `current_exe()`'s own directory is the right lookup.
    #[cfg(target_os = "macos")]
    {
        let sidecar = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join("mnemos-audio")))
            .filter(|p| p.is_file());
        #[cfg(debug_assertions)]
        let sidecar = sidecar.or_else(|| {
            let dev = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../swift/mnemos-audio/.build/release/mnemos-audio");
            dev.is_file().then_some(dev)
        });
        if let Some(bin) = sidecar {
            cfg.sidecar_bin = bin;
        }
    }
    Ok(cfg)
}

/// TypeScript emit settings — shared by the dev-time export and the CI check so
/// both produce byte-identical output.
#[cfg(any(debug_assertions, test))]
fn typescript_config() -> specta_typescript::Typescript {
    specta_typescript::Typescript::default()
        .bigint(specta_typescript::BigIntExportBehavior::Number)
        // `@ts-nocheck`: the emitted file carries scaffolding (Channel import,
        // event proxy helper) that is unused until a later wave registers a
        // channel or event, and `noUnusedLocals` would reject it. Exported
        // types stay fully checked at every call site.
        .header("// @generated by tauri-specta. Do not edit by hand.\n// @ts-nocheck\n")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// macOS/Linux apps launched by double-clicking (Finder, Dock, a built
/// `.app`) get a minimal `PATH` from `launchd` — roughly
/// `/usr/bin:/bin:/usr/sbin:/sbin` plus a couple of system entries — never
/// the user's shell rc-file additions (Homebrew, `nvm`, `cargo`, `pipx`,
/// `~/.local/bin`, etc.). A terminal-launched process (`cargo tauri dev`,
/// or any dev workflow run from a shell) doesn't have this problem, which
/// is why `find_claude_binary`'s PATH scan (`ipc/runner/claude/spawn.rs`)
/// works in dev but silently fails to find a real `claude` install once
/// the app is actually built and launched normally — not a bug in that
/// scan itself, a missing environment fixup before it ever runs.
///
/// Fixes this the same way Electron/VS Code-style apps do: resolve the
/// user's real login-shell `PATH` once, here, before anything else in the
/// app runs, and overwrite this process's own `PATH` with it — every
/// subsequent PATH-based lookup (`find_claude_binary`, the dev-mode
/// `python3` fallback in `worker_config`, anything else) benefits for
/// free, with no changes needed at any call site. Best-effort: on any
/// failure (unknown/broken `$SHELL`, timeout, empty output), the process's
/// original `PATH` is left untouched rather than blocking startup or
/// clearing it. Windows GUI apps inherit the full user `PATH` via the
/// registry-backed environment block already — this is a no-op there.
#[cfg(not(windows))]
fn fix_gui_launch_path() {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    // `-lc` runs the user's login rc files, which can do arbitrary
    // path-relative work. Don't let that happen on the inherited `/` — a
    // Finder-launched `.app` starts there, and anything an rc file does
    // relative to the filesystem root can wander into TCC-protected user
    // folders and surface a permission prompt the user never asked for.
    let mut command = std::process::Command::new(&shell);
    command
        .args(["-lc", "echo -n \"$PATH\""])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    if let Ok(root) = crate::fs::paths::data_root() {
        if root.is_dir() {
            command.current_dir(root);
        }
    }
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(shell = %shell, error = %e, "gui_path_fixup.spawn_failed");
            return;
        }
    };

    // Bounded wait — a slow/hanging shell rc file (network calls, etc.)
    // must not block the whole app from ever starting. Polls rather than
    // `wait_timeout` (not a std API, and not worth a new dependency for a
    // one-shot startup check) — 3s in 50ms steps is plenty of resolution
    // without meaningfully spinning the CPU.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Ok(None) => {
                tracing::warn!(shell = %shell, "gui_path_fixup.timed_out");
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Err(e) => {
                tracing::warn!(shell = %shell, error = %e, "gui_path_fixup.wait_failed");
                break None;
            }
        }
    };

    let Some(status) = status else { return };
    if !status.success() {
        tracing::warn!(shell = %shell, "gui_path_fixup.shell_exited_nonzero");
        return;
    }
    let Some(stdout) = child.stdout.take() else {
        return;
    };
    use std::io::Read;
    let mut buf = String::new();
    if std::io::BufReader::new(stdout)
        .read_to_string(&mut buf)
        .is_err()
    {
        return;
    }
    let resolved = buf.trim();
    if resolved.is_empty() {
        return;
    }

    tracing::info!(path = resolved, "gui_path_fixup.applied");
    // SAFETY: called once, synchronously, before any other thread exists
    // (the very first line of `run()`) — no concurrent env access possible.
    unsafe { std::env::set_var("PATH", resolved) };
}

pub fn run() {
    #[cfg(not(windows))]
    fix_gui_launch_path();

    let builder = specta_builder();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // Debug-session patch: remembers the main window's size/position/
        // maximized state across launches (restores automatically before
        // `setup` runs); `tauri.conf.json`'s 1200x800 + `maximized: true`
        // is only the very-first-launch fallback, before any state file
        // exists yet.
        // Everything except `VISIBLE`. The plugin's default is `all()`, which
        // persists whether the window was on screen and re-applies it at
        // launch — and this app hides the window on close rather than
        // destroying it (`commands::tray::build`), so quitting from the tray
        // with the window closed saved `visible: false` and the *next* launch
        // would come up with no window at all. Visibility here is transient
        // runtime state, never a preference: Mnemos always opens showing.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all()
                        - tauri_plugin_window_state::StateFlags::VISIBLE,
                )
                .build(),
        )
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);

            let log_dir = crate::fs::paths::logs_dir()?;
            let guard = crate::logging::init(log_dir)?;
            // Held for the process lifetime so buffered log lines are flushed.
            app.manage(guard);

            // Regenerate bindings on every dev boot so a changed Rust signature
            // breaks the TypeScript compile now, not at the next CI run.
            #[cfg(debug_assertions)]
            if let Err(err) = builder.export(typescript_config(), BINDINGS_PATH) {
                tracing::warn!(error = %err, "specta binding export failed");
            }

            let version = app.package_info().version.to_string();
            tracing::info!(component = "host", version = %version, "mnemos starting");

            let db_path = crate::fs::paths::db_path()?;
            let (storage, python, metrics) = tauri::async_runtime::block_on(async {
                let pools = crate::db::init(&db_path).await?;
                let service = crate::db::service::SqliteStorageService::new(pools);
                // Crash-resume runs before any command handler
                // is reachable, so a half-finished delete from last session
                // never leaves stale data visible to the UI.
                use crate::db::service::StorageService;
                service.resume_pending_deletes().await?;

                // Product analytics (`metrics` module) — resolved right
                // after storage is ready, since it needs the settings table
                // for the enabled flag / install id. See the module's own
                // doc comment for why the MCP server binary constructs its
                // own separate instance of the same module instead of
                // sharing this one.
                let metrics_cfg = crate::metrics::config::MetricsConfig::resolve_for_app(
                    &service,
                    version.clone(),
                )
                .await;
                let metrics = crate::metrics::Metrics::init(metrics_cfg);
                metrics.track(
                    crate::metrics::events::HOST_STARTED,
                    crate::metrics::properties::EventProperties::from([(
                        "platform",
                        crate::metrics::properties::PropertyValue::Enum(std::env::consts::OS),
                    )]),
                );

                let python =
                    crate::ipc::python::WorkerSupervisor::spawn(worker_config(app.handle())?)
                        .await?;
                // The real reverse-RPC handler for agent extraction, invoked
                // by the Python worker through the generic dispatch
                // mechanism (which also supports dummy handlers for tests).
                python.register_reverse_rpc(
                    "run_agent_extraction",
                    std::sync::Arc::new(
                        crate::ipc::runner::extraction_handler::ExtractionRpcHandler::new(),
                    ),
                );
                Ok::<_, AppError>((service, python, metrics))
            })?;
            app.manage(AppState::new(version, storage, python, metrics));
            app.manage(commands::meeting_detection::MeetingDetectionState::new());

            commands::tray::build(app)?;

            // Meeting auto-detect: a long-lived background watcher, not
            // tied to any one recording (see
            // product_docs/MEETING_AUTO_DETECT_DESIGN.md "Lifecycle") — so
            // it starts here, once, rather than anywhere in the recording
            // path. Gated on the same setting the Settings toggle controls,
            // read once at launch; `meeting_detection_set_enabled` is what
            // starts/stops it live if the user flips it later.
            #[cfg(target_os = "macos")]
            {
                let watcher_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use crate::db::service::StorageService;
                    let state = watcher_handle.state::<AppState>();
                    let enabled = state
                        .storage
                        .get_setting(commands::meeting_detection::KEY_ENABLED)
                        .await
                        .ok()
                        .flatten()
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    if enabled {
                        commands::meeting_detection::start_watcher(&watcher_handle);
                    }
                });
            }

            // Safety net for the hidden-at-creation window above: if the
            // frontend never reaches its `app_ready` call — a bundling
            // mistake, a crash in a provider, a white-screen build — the app
            // would otherwise run with no window and no way to get one. A
            // late window is a bad launch; no window is an unusable app.
            let reveal_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                if reveal_handle
                    .get_webview_window("main")
                    .and_then(|w| w.is_visible().ok())
                    == Some(false)
                {
                    tracing::warn!("frontend never signalled ready; revealing the window anyway");
                    commands::tray::show_main_window(&reveal_handle);
                }
            });

            // Update check on launch — spawned rather than awaited inline,
            // so a slow/offline check never delays startup the way the
            // hard storage/worker dependencies above correctly do.
            let update_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(err) = commands::updater::check_on_launch(update_handle).await {
                    tracing::warn!(error = %err, "launch-time update check failed");
                }
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Closing the window hides it rather than destroying it
            // (`commands::tray::build`'s `CloseRequested` handler), which is
            // what keeps the warm Parakeet worker alive and the tray clock
            // running. macOS keeps the app in the Dock in that state, and
            // clicking that Dock icon raises `Reopen` — without handling it
            // the icon is inert and the only way back into a "closed" app is
            // the tray menu or force-quitting and relaunching.
            //
            // `has_visible_windows` is true when macOS already had a window
            // to raise and did it itself; showing again would be harmless but
            // it would also steal focus from whichever window the user was
            // actually pointing at.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } = event
            {
                commands::tray::show_main_window(app);
            }
            let _ = (app, event);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regenerates `bindings/tauri.ts`. CI runs this and fails if the working
    /// tree is dirty afterwards, which catches hand-edits and stale bindings.
    #[test]
    fn specta_bindings_are_up_to_date() {
        specta_builder()
            .export(typescript_config(), BINDINGS_PATH)
            .expect("specta bindings must export");
    }
}
