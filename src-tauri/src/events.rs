//! Typed event payloads for `tauri_specta`'s generated event catalogue.
//!
//! Each struct here mirrors one wire event Rust emits today. `#[derive(tauri_specta::Event)]`
//! kebab-cases the struct's own name into the wire name (verified against the installed
//! `tauri-specta-macros` source, not assumed) — e.g. `ProcessingProgress` -> `processing-progress`,
//! byte-identical to the string literal the call site used before this struct existed. Fields stay
//! snake_case, matching the serde wire shape the frontend already expects (see `src/ipc/events.ts`).
//!
//! Added one at a time, in the order and by the process `product_docs/EVENTS_TYPING_MIGRATION_PLAN.md`
//! specifies — see that document before adding another.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;

/// Tray Quit found a recording in flight and is asking before exiting.
/// Payload-less, matching today's `app.emit(EVENT_CONFIRM_QUIT, ())`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct TrayConfirmQuit;

/// Menu-bar Pause. Payload-less, matching today's `app.emit(EVENT_PAUSE, ())`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct TrayPauseRecording;

/// Menu-bar Resume. Payload-less, matching today's `app.emit(EVENT_RESUME, ())`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct TrayResumeRecording;

/// Menu-bar Stop. Payload-less, matching today's `app.emit(EVENT_STOP, ())`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct TrayStopRecording;

/// The tray's (or the meeting-detection overlay's) "start a recording from
/// outside the frontend's own UI" trigger — see `commands::tray::EVENT_START`'s
/// doc comment for why this one Rust-side event backs both entry points.
/// `project_id` mirrors `app.emit(EVENT_START, json!({ "project_id": ... }))`'s
/// shape exactly; `None` for the tray's plain Start item, `Some` for a
/// "start in project X" submenu item or the meeting overlay's chosen project.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct TrayStartRecording {
    pub project_id: Option<String>,
}

/// Forwards the Python worker's `live_transcription_warmup` notification.
/// `ready: false` means live transcription is blocked on `ParakeetModel`
/// warm-up; `ready: true` clears it. Fired once per state transition.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct LiveTranscriptionWarmup {
    pub conversation_id: String,
    pub ready: bool,
}

/// Emitted by the capture-watch task before the terminal `RECORDING ->
/// STOPPING` transition on a mic-disconnect, a sidecar crash (`Exited`), or a
/// disk-full write failure — the UI's cue to show a banner/toast with a
/// "View partial" affordance rather than silently losing the recording.
/// `kind` mirrors the capture error/exit vocabulary already defined
/// Rust-side (`mic_disconnected` | `sidecar_exited` | `disk_full`).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct RecordingWarning {
    pub conversation_id: String,
    pub kind: String,
    pub message: String,
}

/// A project's memory doc (Decisions/Open Questions/Action Items) finished
/// an auto- or manual-refresh cycle. Fired from 4 call sites — retrying a
/// step, finishing a conversation's pipeline, setting a conversation's
/// project, and the manual "Refresh memory" action — all funneling through
/// `memory::maybe_auto_refresh`/`memory::refresh_project`, so this is one
/// event with one shape regardless of which triggered it.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct ProjectMemoryUpdated {
    pub project_id: String,
    pub significant_change: bool,
}

/// A conversation's post-recording pipeline (transcribe -> diarize ->
/// extract -> save) finished; the summary/detail view has data to show.
/// `project_id` is `None` for an unfiled conversation (deliberate — unfiled
/// is first-class, not an error state).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct ConversationReady {
    pub conversation_id: String,
    pub project_id: Option<String>,
}

/// One tick of a conversation's post-recording pipeline. `pct` is `None` for
/// steps whose duration cannot be measured — saving the recording (sub-second)
/// and extraction (a streaming model call with no fraction to read); only
/// `transcribing` reports a real fraction, fed from the worker's per-chunk
/// `job_progress` reports. `status` is always one of `running` | `done` |
/// `failed` at every call site today, but stays `String` rather than an enum
/// here to keep this struct's wire shape identical to what every caller of
/// `emit_progress` already sends — narrowing it is a separate decision.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct ProcessingProgress {
    pub conversation_id: String,
    pub step: String,
    pub status: String,
    pub pct: Option<f64>,
}

/// A project's memory auto/manual-refresh (see [`ProjectMemoryUpdated`])
/// failed instead. Fired from the same 4 call sites, same
/// `err.to_string()` shape. No frontend listener exists for this one today
/// — it is typed and registered so the Rust side has a real event to emit
/// through rather than a bare string, but deliberately left unlistened;
/// adding a listener is a separate decision, not part of this migration.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct ProjectMemoryRefreshFailed {
    pub project_id: String,
    pub error_kind: String,
}
