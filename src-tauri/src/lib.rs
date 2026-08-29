//! Mnemos Tauri host. `main.rs` is a shim over `run()`; everything real lives
//! here so the crate stays testable and mobile-ready.

pub mod capture;
pub mod commands;
pub mod db;
pub mod error;
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
/// Checked into git; CI fails if a regen produces a diff (FRONTEND §3).
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
            commands::project::dashboard_get_project_pulse,
            commands::project::project_get_memory_status,
            commands::project::project_list_action_items,
            commands::project::project_list_decisions,
            commands::project::project_list_open_questions,
            commands::chat::chat_send_prompt,
            commands::chat::chat_cancel_turn,
            commands::chat::chat_get_session_history,
            commands::chat::chat_start_new_session,
            commands::chat::chat_rename_session,
            commands::chat::chat_list_sessions,
            commands::chat::chat_resolve_session,
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
        ])
        .events(collect_events![])
}

/// Resolves the Python worker's working directory at runtime instead of
/// baking in a compile-time developer path.
///
/// TODO(packaging): this only fixes the "runs from a built binary anywhere
/// on disk, with `src-python/` copied alongside it" case — it does NOT wire
/// up real production packaging. A proper fix still needs a bundled Python
/// interpreter shipped via `externalBin`/`resources` in `tauri.conf.json`
/// (LLD-02 §3), which is a much larger effort requiring testing against an
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

/// Dev-mode worker config: a `python3` on PATH running `src-python/` in
/// place. Production packaging (a bundled interpreter baked in by the Tauri
/// sidecar bundler, per LLD-02 §3) is not wired up yet — no bundler config
/// exists in `tauri.conf.json` this wave; see the LLD's "Implementation
/// status" for why that's deferred.
fn worker_config() -> Result<crate::ipc::python::SupervisorConfig, AppError> {
    let python_bin = std::path::PathBuf::from(if cfg!(windows) { "python" } else { "python3" });
    let cwd = worker_cwd()?;
    let state_dir = crate::fs::paths::state_dir()?;
    #[allow(unused_mut)]
    let mut cfg = crate::ipc::python::SupervisorConfig::new(python_bin, cwd, state_dir);
    // Dev-mode sidecar path (no bundler packaging yet — same gap as
    // `python_bin` above; see LLD-02's "Implementation status").
    #[cfg(target_os = "macos")]
    {
        cfg.sidecar_bin = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../swift/mnemos-audio/.build/release/mnemos-audio");
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
pub fn run() {
    let builder = specta_builder();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // Debug-session patch: remembers the main window's size/position/
        // maximized state across launches (restores automatically before
        // `setup` runs); `tauri.conf.json`'s 1200x800 + `maximized: true`
        // is only the very-first-launch fallback, before any state file
        // exists yet.
        .plugin(tauri_plugin_window_state::Builder::default().build())
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
                // Crash-resume (LLD-01 §7.5) runs before any command handler
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

                let python = crate::ipc::python::WorkerSupervisor::spawn(worker_config()?).await?;
                // First real reverse-RPC handler (LLD-02 §7.2 / LLD-07 §7):
                // W5 only wired the generic dispatch mechanism with dummy
                // test handlers.
                python.register_reverse_rpc(
                    "run_agent_extraction",
                    std::sync::Arc::new(
                        crate::ipc::runner::extraction_handler::ExtractionRpcHandler::new(),
                    ),
                );
                Ok::<_, AppError>((service, python, metrics))
            })?;
            app.manage(AppState::new(version, storage, python, metrics));

            // Menu-bar / tray red-dot indicator (LLD-11 §6, v1 slice — see
            // `commands::recording::build_tray`'s doc comment for scope).
            commands::recording::build_tray(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
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
