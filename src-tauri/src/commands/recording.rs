//! `recording.*` Tauri commands. This is the glue
//! layer the design docs' "Implementation status" sections left unbuilt:
//! a session registry mapping a small `u32` session id (what the
//! React store already calls `sessionId`) to the real
//! `conversation_id`, the capture control handle, and the running post-stop
//! pipeline — plus the Channel/notification forwarding that turns the
//! worker's `live_transcript_chunk` broadcast and the platform capture
//! stream into what `useLiveTranscriptChannel` (already built) expects.
//!
//! Mic-level Channel (`subscribe_mic_level`/`unsubscribe_mic_level`) and
//! `pause`/`resume` were added later (debug-session patch): the
//! capture-side transport for both (`CaptureEvent::Level`, Swift/WASAPI
//! `pause`/`resume`) already existed — only the Tauri command layer
//! and Channel forwarding were missing.
//!
//! Windows parity audit findings #6/#7: `CAPTURE_EVENT_TOPIC` is now
//! subscribed to on the non-mac branch of `start_platform_capture` too, the
//! same way the mac branch drains its sidecar's event channel — `Level`
//! feeds this session's `level_tx` (what `subscribe_mic_level` reads from),
//! `Warning` is log-only (mirrors mac), and `Error`/`Stopped` (Windows has
//! no separate process to crash-exit the way the mac sidecar does, so a
//! `Stopped` the Rust side didn't itself request by calling `StopCapture`
//! first is just as terminal as an `Error` — see `capture::CaptureEvent`'s
//! doc comment) both route through `handle_capture_failure`, no longer
//! `#[cfg(target_os = "macos")]`-gated. Diarization remains out of v1 scope.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex as StdMutex;

use serde::Serialize;
use specta::Type;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;
use tokio::sync::broadcast;

use crate::capture::CaptureEvent;
use crate::db::models::{Conversation, ConversationStatus, NewConversation, PipelineStep};
use crate::db::service::StorageService;
use crate::error::AppError;
use crate::fs::paths;
use crate::ipc::python::{
    SubscribeLiveTranscript, TranscribeFinal, UnsubscribeLiveTranscript,
    LIVE_TRANSCRIPTION_WARMUP_TOPIC, LIVE_TRANSCRIPT_CHUNK_TOPIC,
};
use crate::memory::DEFAULT_CONVERSATION_TITLE;
use crate::state::AppState;

/// One turn streamed to the React `LiveTranscriptStream` (`TranscriptChunk`,
/// mirrored field-for-field so `useLiveTranscriptChannel`
/// needs no change to consume the real transport).
#[derive(Debug, Clone, Serialize, Type)]
pub struct TranscriptChunk {
    pub session_id: u32,
    pub speaker_label_hint: Option<String>,
    pub text: String,
    pub ts_start_ms: i64,
    pub ts_end_ms: i64,
    /// Client-side supersede-in-place logic already lives in
    /// `useRecordingStore.appendTranscript` (matching `tsStartMs` +
    /// text-prefix) — this transport never needs to flag it itself.
    pub superseded: bool,
}

/// 100ms mic/system dB sample streamed to the React level meter, mirroring
/// `LevelSample` in `src/ipc/streams.ts` field-for-field.
#[derive(Debug, Clone, Copy, Serialize, Type)]
pub struct LevelSample {
    pub session_id: u32,
    pub mic_db: f32,
    pub system_db: f32,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct StartRecordingResult {
    pub session_id: u32,
    pub conversation_id: String,
    /// Always `None` at start — recordings never require a project (a
    /// deliberate design decision). Assignable any time via the project chip.
    pub project_id: Option<String>,
    pub started_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct StopRecordingResult {
    pub conversation_id: String,
    /// True when the recording had under 5s of audio in both files —
    /// mirrors `recover_interrupted_recording`'s and the capture-failure
    /// path's identical threshold. The
    /// conversation row is deleted rather than handed to a pipeline that
    /// would either crash on an effectively-empty WAV or produce a useless
    /// empty transcript. The frontend shows a toast and stays put instead
    /// of navigating to a conversation that no longer exists.
    pub discarded_no_audio: bool,
}

struct ActiveSession {
    conversation_id: String,
    project_id: Option<String>,
    mic_path: PathBuf,
    system_path: PathBuf,
    started_at_ms: i64,
    /// When the recording was paused, or `None` while it's running. Doubles
    /// as the paused flag and as the frozen elapsed-time reading the tray
    /// shows — while paused the frontend's clock stops
    /// (`useRecordingTick` skips the `paused` state), so the menu bar has to
    /// stop at the same number rather than keep counting.
    paused_at_ms: Option<i64>,
    transcript_forward_task: Option<tokio::task::JoinHandle<()>>,
    /// The `LIVE_TRANSCRIPTION_WARMUP_TOPIC` forwarding task spawned
    /// alongside `transcript_forward_task` in `subscribe_transcript` — kept
    /// separate (not folded into one task) but tracked the same way so it
    /// doesn't outlive the session it was spawned for.
    warmup_forward_task: Option<tokio::task::JoinHandle<()>>,
    /// Publishes raw `(mic_db, system_db)` samples; `subscribe_mic_level`
    /// hands each caller its own receiver off this. Always present (even on
    /// platforms that never publish to it yet) so the command handlers below
    /// don't need per-platform branches.
    level_tx: broadcast::Sender<(f32, f32)>,
    level_forward_task: Option<tokio::task::JoinHandle<()>>,
    #[cfg(target_os = "macos")]
    sidecar: Option<crate::ipc::swift::SidecarControl>,
    /// Drains the platform capture-event stream (mac sidecar events or, per
    /// Windows parity audit finding #6, the worker's `CAPTURE_EVENT_TOPIC`
    /// notifications) for this session. No longer mac-only.
    capture_watch_task: Option<tokio::task::JoinHandle<()>>,
}

/// Session registry — at most one active recording in v1 in practice, but
/// keyed so a stray late command against an already-stopped session id is a
/// clean `NotFound`, not a panic.
#[derive(Default)]
pub struct RecordingRegistry {
    sessions: StdMutex<HashMap<u32, ActiveSession>>,
    next_id: AtomicU32,
}

impl RecordingRegistry {
    /// Keeps an in-flight recording's cached `project_id` in sync after a
    /// project (re)assignment (the chip is deliberately editable
    /// "before, during, after recording, or never") — `stop_recording`'s
    /// post-pipeline memory-refresh trigger reads this cached value to know
    /// which project's memory doc to refresh. `mic_path`/`system_path` need
    /// no equivalent resync: conversation directories are flat and keyed by
    /// id alone (`fs::paths::recordings_root`), so a project reassignment
    /// never moves them. No-op (`false`) if no session is currently
    /// recording this conversation.
    pub fn resync_project(&self, conversation_id: &str, project_id: Option<String>) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        let Some(session) = sessions
            .values_mut()
            .find(|s| s.conversation_id == conversation_id)
        else {
            return false;
        };
        session.project_id = project_id;
        true
    }

    pub fn new() -> Self {
        Self {
            sessions: StdMutex::new(HashMap::new()),
            next_id: AtomicU32::new(1),
        }
    }

    /// Removes and returns the session for `conversation_id`, if one is
    /// still registered. Used by the mid-recording failure path
    /// (`handle_capture_failure` below): the
    /// capture-watch task only knows the `conversation_id`, not the
    /// ephemeral `session_id` the React store uses, so it can't call the
    /// ordinary `stop_recording` command handler (which is keyed by
    /// `session_id` and is meant to be driven by the user's Stop click, not
    /// the backend itself).
    fn take_by_conversation_id(&self, conversation_id: &str) -> Option<ActiveSession> {
        let mut sessions = self.sessions.lock().unwrap();
        let id = sessions
            .iter()
            .find(|(_, s)| s.conversation_id == conversation_id)
            .map(|(id, _)| *id)?;
        sessions.remove(&id)
    }

    /// Whether any recording is currently in flight — `commands::updater`
    /// checks this before installing an update, since relaunching the app
    /// mid-recording would kill the in-progress capture, and
    /// `commands::tray` before letting Quit through.
    pub fn has_active_session(&self) -> bool {
        !self.sessions.lock().unwrap().is_empty()
    }

    /// Marks the session paused (or running again), returning `false` if it
    /// has already been torn down. The timestamp is what freezes the tray's
    /// elapsed clock; see `ActiveSession::paused_at_ms`.
    fn set_paused(&self, session_id: u32, paused: bool) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        let Some(session) = sessions.get_mut(&session_id) else {
            return false;
        };
        session.paused_at_ms = paused.then(now_ms);
        true
    }

    /// The registry's own answer to "what should the menu bar show" — the
    /// single source `commands::tray` renders from. Lives here rather than in
    /// that module so the lock is never held across a `.await`.
    pub(crate) fn tray_snapshot(&self) -> crate::commands::tray::TrayState {
        use crate::commands::tray::TrayState;

        let sessions = self.sessions.lock().unwrap();
        // v1 never has two, but "the oldest one" is a defined answer rather
        // than whichever the hash map happens to yield first.
        let Some(session) = sessions.values().min_by_key(|s| s.started_at_ms) else {
            return TrayState::Idle;
        };
        match session.paused_at_ms {
            Some(paused_at_ms) => TrayState::Paused {
                elapsed_s: (paused_at_ms - session.started_at_ms) / 1000,
            },
            None => TrayState::Recording {
                elapsed_s: (now_ms() - session.started_at_ms) / 1000,
            },
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_millis() as i64
}

/// Bucketed for `metrics::events::RECORDING_STOPPED` — a coarse duration
/// bucket carries no content, unlike the raw second count, which is
/// precise enough to sometimes work as a fingerprint for a specific
/// meeting when cross-referenced with other data.
fn duration_bucket(duration_s: i64) -> &'static str {
    match duration_s {
        s if s < 300 => "<5m",
        s if s < 900 => "5-15m",
        s if s < 3600 => "15-60m",
        _ => "60m+",
    }
}

/// `metrics::events::PIPELINE_STEP_COMPLETED` for a successfully finished
/// pipeline step — `step_duration_ms` plus `meeting_duration_bucket`
/// (rather than raw meeting duration) is enough to compute "how long does
/// transcription/extraction take relative to meeting length" directly in
/// PostHog without carrying the more identifying raw value.
fn track_step_completed(
    state: &State<'_, AppState>,
    step: &'static str,
    elapsed: std::time::Duration,
    meeting_bucket: &'static str,
) {
    state.metrics.track(
        crate::metrics::events::PIPELINE_STEP_COMPLETED,
        crate::metrics::properties::EventProperties::from([
            (
                "step",
                crate::metrics::properties::PropertyValue::Enum(step),
            ),
            (
                "step_duration_ms",
                crate::metrics::properties::PropertyValue::UInt(elapsed.as_millis() as u64),
            ),
            (
                "meeting_duration_bucket",
                crate::metrics::properties::PropertyValue::Enum(meeting_bucket),
            ),
        ]),
    );
}

fn session_not_found(session_id: u32) -> AppError {
    AppError::NotFound {
        entity: "recording_session".into(),
        id: session_id.to_string(),
    }
}

/// `pct` is `None` for steps whose duration cannot be measured — saving the
/// recording (sub-second) and extraction (a streaming model call with no
/// fraction to read). Those render as a spinner and a label with no number,
/// which is honest; the previous hardcoded 0.05/0.6/0.8 milestones looked
/// like measurements and were not. Only `transcribing` reports a real
/// fraction, fed from the worker's per-chunk `job_progress` reports.
/// The terminal "this conversation finished processing" progress event.
///
/// Shared with `conversation_retry_step` so a recovered conversation emits
/// the identical signal a first-time success does — one definition, so the
/// two cannot drift into meaning subtly different things.
pub(crate) fn emit_pipeline_done(app: &AppHandle, conversation_id: &str) {
    emit_progress(app, conversation_id, "done", "done", None);
}

fn emit_progress(
    app: &AppHandle,
    conversation_id: &str,
    step: &str,
    status: &str,
    pct: Option<f64>,
) {
    emit_progress_with_error(app, conversation_id, step, status, pct, None);
}

fn emit_progress_with_error(
    app: &AppHandle,
    conversation_id: &str,
    step: &str,
    status: &str,
    pct: Option<f64>,
    error: Option<String>,
) {
    let _ = crate::events::ProcessingProgress {
        conversation_id: conversation_id.to_string(),
        step: step.to_string(),
        status: status.to_string(),
        pct,
        error,
    }
    .emit(app);
}

/// Starts a recording session: creates the `conversations` row, spawns
/// platform capture (mac sidecar / Windows WASAPI thread via the worker),
/// and starts the worker's live-transcription poll loop. The
/// `idle -> arming -> recording` UI flow: this command is what `arming` waits on.
#[tauri::command]
#[specta::specta]
pub async fn start_recording(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: Option<String>,
) -> Result<StartRecordingResult, AppError> {
    // Recordings never *require* a project (a deliberate design decision) —
    // Record starts with zero project gate, and `None` here is a
    // first-class, permanent state. The chip stays editable before, during,
    // after, or never.
    //
    // But when the caller *does* know the project (Project Detail's Record
    // button, the top bar's project picker), it is set at row creation
    // rather than hardcoded `None` with the frontend firing a follow-up
    // `conversation_set_project` whose failure was silently swallowed — that
    // older shape let a recording started from a project land unfiled with
    // no error shown anywhere, with both the chip and the Recent
    // Conversations row quietly reading "No project".
    let started_at = now_ms() / 1000;

    let conversation = state
        .storage
        .insert_conversation(NewConversation {
            project_id: project_id.clone(),
            title: DEFAULT_CONVERSATION_TITLE.into(),
            started_at,
            runner_id: None,
        })
        .await?;
    let conv_id = conversation.id.clone();

    let dir = paths::conversation_dir(&conv_id)?;
    std::fs::create_dir_all(&dir)?;
    let mic_path = paths::mic_wav_path(&conv_id)?;
    let system_path = paths::system_wav_path(&conv_id)?;

    let (level_tx, _) = broadcast::channel::<(f32, f32)>(64);

    let start_result = start_platform_capture(
        app.clone(),
        &state,
        &conv_id,
        &mic_path,
        &system_path,
        level_tx.clone(),
    )
    .await;
    let (sidecar, capture_watch_task) = match start_result {
        Ok(parts) => parts,
        Err(err) => {
            let _ = state
                .storage
                .update_conversation_status(&conv_id, ConversationStatus::Failed, None, None)
                .await;
            return Err(err);
        }
    };
    // `sidecar`'s type is `Option<()>` on non-mac (see `SidecarParts`) —
    // nothing to store there, only `capture_watch_task` is used below.
    #[cfg(not(target_os = "macos"))]
    let _ = &sidecar;

    if let Err(err) = state
        .python
        .send(SubscribeLiveTranscript {
            conversation_id: conv_id.clone(),
            mic_path: mic_path.clone(),
        })
        .await
    {
        // Capture is already running — losing live transcription is not
        // fatal (post-stop `transcribe_final` still produces the real
        // transcript), so log and keep recording rather than aborting.
        tracing::warn!(error = %err, conv_id, "recording.subscribe_live_transcript_failed");
    } else {
        tracing::info!(conv_id, mic_path = %mic_path.display(), "recording.subscribe_live_transcript_sent");
    }

    let session_id = state.recording.next_id.fetch_add(1, Ordering::SeqCst);
    let started_at_ms = now_ms();
    state.recording.sessions.lock().unwrap().insert(
        session_id,
        ActiveSession {
            conversation_id: conv_id.clone(),
            project_id: project_id.clone(),
            mic_path,
            system_path,
            started_at_ms,
            paused_at_ms: None,
            transcript_forward_task: None,
            warmup_forward_task: None,
            level_tx,
            level_forward_task: None,
            #[cfg(target_os = "macos")]
            sidecar,
            capture_watch_task,
        },
    );

    crate::commands::tray::refresh(&app).await;

    state.metrics.track(
        crate::metrics::events::RECORDING_STARTED,
        crate::metrics::properties::EventProperties::from([(
            "has_project",
            crate::metrics::properties::PropertyValue::Bool(project_id.is_some()),
        )]),
    );

    Ok(StartRecordingResult {
        session_id,
        conversation_id: conv_id,
        project_id,
        started_at_ms,
    })
}

#[cfg(target_os = "macos")]
type SidecarParts = (
    Option<crate::ipc::swift::SidecarControl>,
    Option<tokio::task::JoinHandle<()>>,
);
#[cfg(not(target_os = "macos"))]
type SidecarParts = (Option<()>, Option<tokio::task::JoinHandle<()>>);

#[cfg(target_os = "macos")]
async fn start_platform_capture(
    app: AppHandle,
    state: &State<'_, AppState>,
    conv_id: &str,
    mic_path: &Path,
    system_path: &Path,
    level_tx: broadcast::Sender<(f32, f32)>,
) -> Result<SidecarParts, AppError> {
    let handle = state
        .python
        .spawn_sidecar(crate::ipc::swift::SidecarConfig {
            conversation_id: conv_id.to_string(),
            mic_path: mic_path.to_path_buf(),
            system_path: system_path.to_path_buf(),
            mic_device_id: None,
        })
        .await?;

    // Drain capture events so the mpsc channel never backs up. `Level`
    // publishes onto this session's broadcast channel for the mic-level
    // meter (`subscribe_mic_level`). `Error`/`Exited`
    // are both terminal for this recording, so both
    // route through the same `handle_capture_failure` — it emits
    // `events.recordingWarning` for the UI, then either salvages the
    // partial audio through the normal pipeline or discards it, exactly
    // like `recover_interrupted_recording`'s crash-recovery path does for a
    // *whole-app* crash (this is the same failure, just detected live
    // instead of at next launch). `Warning` (non-fatal — `no_mic_signal`
    // etc.) stays log-only; the recording keeps running.
    let conv_id_owned = conv_id.to_string();
    let mut events = handle.events;
    let watch_task = tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                CaptureEvent::Level { mic_db, system_db } => {
                    let _ = level_tx.send((mic_db, system_db));
                }
                CaptureEvent::Exited { code, signal } => {
                    tracing::warn!(
                        conv_id = %conv_id_owned,
                        code,
                        signal,
                        "recording.sidecar_exited_unexpectedly"
                    );
                    handle_capture_failure(
                        &app,
                        &conv_id_owned,
                        "sidecar_exited",
                        &format!("The recording process exited unexpectedly (code={code:?}, signal={signal:?})."),
                    )
                    .await;
                    return;
                }
                CaptureEvent::Warning { kind, message } => {
                    tracing::warn!(conv_id = %conv_id_owned, kind, message, "recording.capture_warning");
                }
                CaptureEvent::Error { kind, message } => {
                    tracing::error!(conv_id = %conv_id_owned, kind, message, "recording.capture_error");
                    handle_capture_failure(&app, &conv_id_owned, &kind, &message).await;
                    return;
                }
                _ => {}
            }
        }
    });

    Ok((Some(handle.control), Some(watch_task)))
}

/// Called from the platform capture-watch task (mac sidecar events, or —
/// per Windows parity audit finding #6 — the Windows `CAPTURE_EVENT_TOPIC`
/// watch task above) the moment a terminal event arrives (`Error`/`Exited`
/// on mac, `Error`/`Stopped` on Windows) — there is no user "Stop" click
/// driving this, so it reconstructs the same tail
/// `stop_recording`/`recover_interrupted_recording` run: emit the UI-visible
/// warning first (so the toast lands *before* the conversation disappears
/// from "currently recording"), tear down the session, then either hand the
/// partial audio to the normal pipeline (≥5s captured — same threshold
/// `recover_interrupted_recording` uses) or discard the row (silent-save
/// behavior).
///
/// Disk-full (`kind == "disk_full"`) deliberately reuses this same event
/// rather than doubling up with `events.storageWarning`/`storageCritical`:
/// those two are the *proactive* low-disk poller (fires before any
/// write actually fails, while recording could still continue), this one is
/// the sidecar's own write failing outright (recording is already over) —
/// different moments, different UI treatment (inline "recording will stop
/// soon" banner vs. a terminal "here's what we saved" toast), so keeping
/// them separate avoids a confusing double notification for the same
/// underlying disk-full condition.
async fn handle_capture_failure(app: &AppHandle, conv_id: &str, kind: &str, message: &str) {
    let state = app.state::<AppState>();

    let _ = crate::events::RecordingWarning {
        conversation_id: conv_id.to_string(),
        kind: kind.to_string(),
        message: message.to_string(),
    }
    .emit(app);

    let Some(session) = state.recording.take_by_conversation_id(conv_id) else {
        // Already torn down by a concurrent `stop_recording` — nothing left
        // to do (e.g. the user clicked Stop in the same instant capture
        // failed; whichever won the race already ran the tail).
        return;
    };
    crate::commands::tray::refresh(app).await;

    if let Some(task) = &session.transcript_forward_task {
        task.abort();
    }
    if let Some(task) = &session.warmup_forward_task {
        task.abort();
    }
    if let Some(task) = &session.level_forward_task {
        task.abort();
    }
    #[cfg(target_os = "macos")]
    if let Some(sidecar) = &session.sidecar {
        // Best-effort: the process may already be gone (that's the whole
        // point of `Exited`), or unresponsive (`Error`). Either way there's
        // nothing more useful to do than try.
        let _ = sidecar.stop().await;
    }
    // Windows: the capture thread has already exited by the time this event
    // reached us (that's what makes `Error`/`Stopped` terminal here), but
    // `CaptureManager`'s worker-side `_active` slot doesn't know that until
    // `stop_capture` is called — best-effort, same reasoning as the mac
    // `sidecar.stop()` above.
    #[cfg(not(target_os = "macos"))]
    {
        let _ = state
            .python
            .send(crate::ipc::python::StopCapture {
                conversation_id: conv_id.to_string(),
            })
            .await;
    }
    let _ = state
        .python
        .send(UnsubscribeLiveTranscript {
            conversation_id: conv_id.to_string(),
        })
        .await;

    let mic_bytes = std::fs::metadata(&session.mic_path)
        .map(|m| m.len())
        .unwrap_or(0);
    let system_bytes = std::fs::metadata(&session.system_path)
        .map(|m| m.len())
        .unwrap_or(0);
    const MIN_AUDIO_BYTES: u64 = BYTES_PER_SEC * 5;

    if mic_bytes < MIN_AUDIO_BYTES && system_bytes < MIN_AUDIO_BYTES {
        // Below 5s captured: delete the row instead of running a pipeline
        // over nothing.
        if let Err(err) = state.storage.delete_conversation(conv_id).await {
            tracing::error!(conv_id, error = %err, "recording.capture_failure_delete_failed");
        }
        return;
    }

    let ended_at = now_ms() / 1000;
    let duration_s = (now_ms() - session.started_at_ms).max(0) / 1000;
    if let Err(err) = state
        .storage
        .update_conversation_status(
            conv_id,
            ConversationStatus::Processing,
            Some(ended_at),
            Some(duration_s),
        )
        .await
    {
        tracing::error!(conv_id, error = %err, "recording.capture_failure_status_update_failed");
        return;
    }
    if let Err(err) = state
        .storage
        .set_pipeline_step(conv_id, PipelineStep::Finalizing, None)
        .await
    {
        tracing::error!(conv_id, error = %err, "recording.capture_failure_pipeline_init_failed");
        return;
    }
    emit_progress(app, conv_id, "finalizing", "running", None);

    tokio::spawn(run_post_recording_pipeline(
        app.clone(),
        conv_id.to_string(),
        session.project_id.clone(),
        session.mic_path.clone(),
        session.system_path.clone(),
        ended_at,
        duration_s,
    ));
}

/// Windows parity audit findings #6/#7: subscribes to `CAPTURE_EVENT_TOPIC`
/// (`ipc::python::WorkerSupervisor::subscribe`, the same mechanism
/// `subscribe_transcript` already uses for `LIVE_TRANSCRIPT_CHUNK_TOPIC`)
/// and drains it for this conversation, mirroring the mac branch's sidecar
/// event-channel watch task above: `Level` publishes onto `level_tx` (what
/// `subscribe_mic_level` feeds the React level meter from), `Warning` is log-only (matches mac's
/// non-fatal `no_mic_signal`/etc. handling), and `Error`/`Stopped` are both
/// terminal — `capture::CaptureEvent`'s doc comment already anticipated
/// this: Windows has no separate process to `Exited`, so the capture
/// thread's own `Stopped` (emitted from `_run`'s `finally` on any exit, not
/// just a clean one) is exactly as terminal as an `Error`. `stop_recording`
/// aborts this task *before* sending `StopCapture` on a normal user-driven
/// stop, so the ordinary "stopped because Stop was clicked" path never
/// reaches this handler.
#[cfg(not(target_os = "macos"))]
async fn start_platform_capture(
    app: AppHandle,
    state: &State<'_, AppState>,
    conv_id: &str,
    mic_path: &Path,
    system_path: &Path,
    level_tx: broadcast::Sender<(f32, f32)>,
) -> Result<SidecarParts, AppError> {
    state
        .python
        .send(crate::ipc::python::StartCapture {
            conversation_id: conv_id.to_string(),
            mic_path: mic_path.to_path_buf(),
            system_path: system_path.to_path_buf(),
            mic_device_id: None,
        })
        .await?;

    let mut events = state
        .python
        .subscribe(crate::ipc::python::CAPTURE_EVENT_TOPIC);
    let conv_id_owned = conv_id.to_string();
    let watch_task = tokio::spawn(async move {
        loop {
            let value = match events.recv().await {
                Ok(v) => v,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            };
            if value
                .get("conversation_id")
                .and_then(serde_json::Value::as_str)
                != Some(conv_id_owned.as_str())
            {
                continue;
            }
            let Some(event) = crate::capture::capture_event_from_notification(&value) else {
                continue;
            };
            match event {
                CaptureEvent::Level { mic_db, system_db } => {
                    let _ = level_tx.send((mic_db, system_db));
                }
                CaptureEvent::Warning { kind, message } => {
                    tracing::warn!(conv_id = %conv_id_owned, kind, message, "recording.capture_warning");
                }
                CaptureEvent::Error { kind, message } => {
                    tracing::error!(conv_id = %conv_id_owned, kind, message, "recording.capture_error");
                    handle_capture_failure(&app, &conv_id_owned, &kind, &message).await;
                    return;
                }
                CaptureEvent::Stopped { .. } => {
                    tracing::warn!(
                        conv_id = %conv_id_owned,
                        "recording.capture_stopped_unexpectedly"
                    );
                    handle_capture_failure(
                        &app,
                        &conv_id_owned,
                        "capture_stopped_unexpectedly",
                        "The recording thread stopped unexpectedly.",
                    )
                    .await;
                    return;
                }
                _ => {}
            }
        }
    });

    Ok((None, Some(watch_task)))
}

/// Forwards the worker's `live_transcript_chunk` broadcast (filtered to this
/// session's conversation) into the Tauri `Channel` React subscribed
/// through — the "reader task" the design docs' Implementation
/// status sections left unbuilt.
#[tauri::command]
#[specta::specta]
pub async fn subscribe_transcript(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: u32,
    channel: Channel<TranscriptChunk>,
) -> Result<(), AppError> {
    let conversation_id = {
        let sessions = state.recording.sessions.lock().unwrap();
        sessions
            .get(&session_id)
            .map(|s| s.conversation_id.clone())
            .ok_or_else(|| session_not_found(session_id))?
    };

    tracing::info!(
        session_id,
        conversation_id,
        "recording.subscribe_transcript_registered"
    );

    let python = state.python.clone();
    // A second small forwarding task, same shape as the one below but
    // for `LIVE_TRANSCRIPTION_WARMUP_TOPIC` — tracked in its own
    // `session.warmup_forward_task` slot (not folded into the transcript
    // one) so aborting either independently stays possible, but both get
    // aborted together everywhere the session itself is torn down.
    let mut warmup_rx = python.subscribe(LIVE_TRANSCRIPTION_WARMUP_TOPIC);
    let warmup_conversation_id = conversation_id.clone();
    let warmup_app = app.clone();
    let warmup_task = tokio::spawn(async move {
        loop {
            match warmup_rx.recv().await {
                Ok(value) => {
                    let notif: Result<crate::ipc::python::LiveTranscriptionWarmupNotification, _> =
                        serde_json::from_value(value);
                    let Ok(notif) = notif else { continue };
                    if notif.conversation_id != warmup_conversation_id {
                        continue;
                    }
                    let _ = crate::events::LiveTranscriptionWarmup {
                        conversation_id: notif.conversation_id,
                        ready: notif.ready,
                    }
                    .emit(&warmup_app);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });

    let mut rx = python.subscribe(LIVE_TRANSCRIPT_CHUNK_TOPIC);
    let task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(value) => {
                    let notif: Result<crate::ipc::python::LiveTranscriptChunkNotification, _> =
                        serde_json::from_value(value);
                    let Ok(notif) = notif else {
                        tracing::warn!(session_id, "recording.live_transcript_chunk_unparseable");
                        continue;
                    };
                    if notif.conversation_id != conversation_id {
                        continue;
                    }
                    tracing::info!(
                        session_id,
                        conversation_id,
                        text_len = notif.chunk.text.len(),
                        "recording.live_transcript_chunk_forwarded"
                    );
                    let chunk = TranscriptChunk {
                        session_id,
                        speaker_label_hint: notif.chunk.speaker_label_hint,
                        text: notif.chunk.text,
                        ts_start_ms: notif.chunk.ts_start_ms,
                        ts_end_ms: notif.chunk.ts_end_ms,
                        superseded: false,
                    };
                    if channel.send(chunk).is_err() {
                        tracing::warn!(session_id, "recording.live_transcript_channel_send_failed");
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        session_id,
                        skipped = n,
                        "recording.live_transcript_chunk_lagged"
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });

    if let Some(session) = state
        .recording
        .sessions
        .lock()
        .unwrap()
        .get_mut(&session_id)
    {
        session.transcript_forward_task = Some(task);
        session.warmup_forward_task = Some(warmup_task);
    } else {
        task.abort();
        warmup_task.abort();
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn unsubscribe_transcript(
    state: State<'_, AppState>,
    session_id: u32,
) -> Result<(), AppError> {
    if let Some(session) = state
        .recording
        .sessions
        .lock()
        .unwrap()
        .get_mut(&session_id)
    {
        if let Some(task) = session.transcript_forward_task.take() {
            task.abort();
        }
        if let Some(task) = session.warmup_forward_task.take() {
            task.abort();
        }
    }
    Ok(())
}

/// Forwards this session's `(mic_db, system_db)` broadcast onto the Tauri
/// `Channel` the level meter subscribed through — same shape as
/// `subscribe_transcript` above, but the source is the capture-watch task's
/// broadcast sender (`ActiveSession::level_tx`) rather than a worker topic.
#[tauri::command]
#[specta::specta]
pub async fn subscribe_mic_level(
    state: State<'_, AppState>,
    session_id: u32,
    channel: Channel<LevelSample>,
) -> Result<(), AppError> {
    let mut rx = {
        let sessions = state.recording.sessions.lock().unwrap();
        sessions
            .get(&session_id)
            .map(|s| s.level_tx.subscribe())
            .ok_or_else(|| session_not_found(session_id))?
    };

    let task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok((mic_db, system_db)) => {
                    let sample = LevelSample {
                        session_id,
                        mic_db,
                        system_db,
                    };
                    if channel.send(sample).is_err() {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    });

    if let Some(session) = state
        .recording
        .sessions
        .lock()
        .unwrap()
        .get_mut(&session_id)
    {
        session.level_forward_task = Some(task);
    } else {
        task.abort();
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn unsubscribe_mic_level(
    state: State<'_, AppState>,
    session_id: u32,
) -> Result<(), AppError> {
    if let Some(session) = state
        .recording
        .sessions
        .lock()
        .unwrap()
        .get_mut(&session_id)
    {
        if let Some(task) = session.level_forward_task.take() {
            task.abort();
        }
    }
    Ok(())
}

/// Suspends the platform capture. Only the transport differs per platform —
/// the registry bookkeeping around it is shared, so it lives in
/// [`pause_recording`] rather than being written twice.
#[cfg(target_os = "macos")]
async fn suspend_capture(state: &AppState, session_id: u32) -> Result<(), AppError> {
    let sidecar = {
        let sessions = state.recording.sessions.lock().unwrap();
        sessions
            .get(&session_id)
            .ok_or_else(|| session_not_found(session_id))?
            .sidecar
            .clone()
    };
    if let Some(sidecar) = sidecar {
        sidecar.pause().await?;
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
async fn suspend_capture(state: &AppState, session_id: u32) -> Result<(), AppError> {
    let conversation_id = {
        let sessions = state.recording.sessions.lock().unwrap();
        sessions
            .get(&session_id)
            .map(|s| s.conversation_id.clone())
            .ok_or_else(|| session_not_found(session_id))?
    };
    state
        .python
        .send(crate::ipc::python::PauseCapture { conversation_id })
        .await?;
    Ok(())
}

/// Resumes the platform capture. Mirror of [`suspend_capture`].
#[cfg(target_os = "macos")]
async fn unsuspend_capture(state: &AppState, session_id: u32) -> Result<(), AppError> {
    let sidecar = {
        let sessions = state.recording.sessions.lock().unwrap();
        sessions
            .get(&session_id)
            .ok_or_else(|| session_not_found(session_id))?
            .sidecar
            .clone()
    };
    if let Some(sidecar) = sidecar {
        sidecar.resume().await?;
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
async fn unsuspend_capture(state: &AppState, session_id: u32) -> Result<(), AppError> {
    let conversation_id = {
        let sessions = state.recording.sessions.lock().unwrap();
        sessions
            .get(&session_id)
            .map(|s| s.conversation_id.clone())
            .ok_or_else(|| session_not_found(session_id))?
    };
    state
        .python
        .send(crate::ipc::python::ResumeCapture { conversation_id })
        .await?;
    Ok(())
}

/// Pauses capture. The capture side genuinely stops writing samples (the
/// Swift sidecar drops sample buffers while paused, and the Windows worker
/// stops its capture) — a pause is a hole in the recording, not a UI-only
/// freeze.
///
/// Marking the registry paused *after* the transport succeeds keeps the two
/// from disagreeing when the sidecar refuses: a failed pause leaves the
/// session running, which is what the frontend's own rollback assumes.
#[tauri::command]
#[specta::specta]
pub async fn pause_recording(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: u32,
) -> Result<(), AppError> {
    suspend_capture(&state, session_id).await?;
    // `false` only if the session was torn down underneath us; the refresh
    // below then simply renders whatever the registry does hold.
    let _ = state.recording.set_paused(session_id, true);
    crate::commands::tray::refresh(&app).await;
    Ok(())
}

/// Resumes capture. See [`pause_recording`] for the ordering rationale.
#[tauri::command]
#[specta::specta]
pub async fn resume_recording(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: u32,
) -> Result<(), AppError> {
    unsuspend_capture(&state, session_id).await?;
    let _ = state.recording.set_paused(session_id, false);
    crate::commands::tray::refresh(&app).await;
    Ok(())
}

/// Stops capture, transitions the conversation to `Processing`, and kicks
/// off the rest of the pipeline (`transcribe_final` -> extraction -> done)
/// in a detached task so this command returns immediately — the caller
/// navigates to Conversation Detail on `onMutate` and watches
/// `processing-progress` events from there.
#[tauri::command]
#[specta::specta]
pub async fn stop_recording(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: u32,
) -> Result<StopRecordingResult, AppError> {
    let mut session = state
        .recording
        .sessions
        .lock()
        .unwrap()
        .remove(&session_id)
        .ok_or_else(|| session_not_found(session_id))?;

    // Immediately after the removal, not after the teardown below: the
    // registry is what the tray renders from, and every line between here
    // and the end of this function can return early. Leaving the refresh
    // downstream of a `?` is what previously stranded a "recording" icon in
    // the menu bar after a failed `stop_platform_capture`.
    crate::commands::tray::refresh(&app).await;

    if let Some(task) = session.transcript_forward_task.take() {
        task.abort();
    }
    if let Some(task) = session.warmup_forward_task.take() {
        task.abort();
    }
    // Aborted before `stop_platform_capture` below sends `StopCapture` on
    // Windows (or stops the sidecar on mac) so the normal "Stop was
    // clicked" `Stopped`/`Exited` event never races into
    // `handle_capture_failure` treating a clean stop as a failure.
    if let Some(task) = session.capture_watch_task.take() {
        task.abort();
    }

    let _ = state
        .python
        .send(UnsubscribeLiveTranscript {
            conversation_id: session.conversation_id.clone(),
        })
        .await;

    stop_platform_capture(&state, &session).await?;

    // Mirrors `recover_interrupted_recording`'s and the
    // capture-failure path's identical ≥5s-captured rule — a Stop clicked
    // before the sidecar's first flush (or within a couple of seconds of
    // Record) would otherwise sail straight into `run_post_recording_pipeline`
    // with an effectively-empty WAV, which `transcribe_final` then crashes
    // on outright (`ValueError: Negative dimensions not allowed`, verified).
    let mic_bytes = std::fs::metadata(&session.mic_path)
        .map(|m| m.len())
        .unwrap_or(0);
    let system_bytes = std::fs::metadata(&session.system_path)
        .map(|m| m.len())
        .unwrap_or(0);
    const MIN_AUDIO_BYTES: u64 = BYTES_PER_SEC * 5;
    if mic_bytes < MIN_AUDIO_BYTES && system_bytes < MIN_AUDIO_BYTES {
        let conversation_id = session.conversation_id.clone();
        state.storage.delete_conversation(&conversation_id).await?;
        return Ok(StopRecordingResult {
            conversation_id,
            discarded_no_audio: true,
        });
    }

    let ended_at = now_ms() / 1000;
    let duration_s = (ended_at - session.started_at_ms / 1000).max(0);
    state.metrics.track(
        crate::metrics::events::RECORDING_STOPPED,
        crate::metrics::properties::EventProperties::from([(
            "duration_bucket",
            crate::metrics::properties::PropertyValue::Enum(duration_bucket(duration_s)),
        )]),
    );
    state
        .storage
        .update_conversation_status(
            &session.conversation_id,
            ConversationStatus::Processing,
            Some(ended_at),
            Some(duration_s),
        )
        .await?;
    state
        .storage
        .set_pipeline_step(&session.conversation_id, PipelineStep::Finalizing, None)
        .await?;
    emit_progress(
        &app,
        &session.conversation_id,
        "finalizing",
        "running",
        None,
    );

    let conversation_id = session.conversation_id.clone();
    tokio::spawn(run_post_recording_pipeline(
        app,
        session.conversation_id,
        session.project_id,
        session.mic_path,
        session.system_path,
        ended_at,
        duration_s,
    ));

    Ok(StopRecordingResult {
        conversation_id,
        discarded_no_audio: false,
    })
}

#[cfg(target_os = "macos")]
async fn stop_platform_capture(
    _state: &State<'_, AppState>,
    session: &ActiveSession,
) -> Result<(), AppError> {
    if let Some(sidecar) = &session.sidecar {
        sidecar.stop().await?;
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
async fn stop_platform_capture(
    state: &State<'_, AppState>,
    session: &ActiveSession,
) -> Result<(), AppError> {
    state
        .python
        .send(crate::ipc::python::StopCapture {
            conversation_id: session.conversation_id.clone(),
        })
        .await?;
    Ok(())
}

async fn run_post_recording_pipeline(
    app: AppHandle,
    conv_id: String,
    project_id: Option<String>,
    mic_path: PathBuf,
    system_path: PathBuf,
    ended_at: i64,
    duration_s: i64,
) {
    let state = app.state::<AppState>();
    // Time-to-ready (`metrics::events::PIPELINE_COMPLETED`) and each step's
    // own share of it (`PIPELINE_STEP_COMPLETED`'s `step_duration_ms`) — the
    // "how long does transcription/extraction actually take for a 30-minute
    // meeting" question. `meeting_bucket` is computed once and reused on
    // every timing event from here on so they can all be correlated by
    // meeting length without carrying the raw (more identifying) duration.
    let pipeline_start = std::time::Instant::now();
    let meeting_bucket = duration_bucket(duration_s);

    emit_progress(&app, &conv_id, "transcribing", "running", Some(0.0));
    let transcribe_start = std::time::Instant::now();

    // Forward the worker's real per-chunk progress to the UI for as long as
    // this call runs.
    //
    // The worker already emits `job_progress` with a genuine fraction
    // spanning both audio files — it is what re-arms this request's deadline
    // — but nothing was consuming it on the way out, so the UI showed a
    // hardcoded 20% that sat still for the entire pass. On a two-hour
    // recording that is six and a half minutes of a frozen bar, which reads
    // as a hung app.
    //
    // No request-id correlation is needed: the worker's job executor is
    // single-threaded by design (the models are not thread-safe — see its
    // docstring), so at most one job is ever in flight and every tick that
    // arrives here belongs to this call. The task is aborted below, so it
    // cannot outlive the transcription and bleed ticks into a later step.
    let progress_task = {
        let app = app.clone();
        let conv_id = conv_id.clone();
        let mut rx = state
            .python
            .subscribe(crate::ipc::python::JOB_PROGRESS_TOPIC);
        tokio::spawn(async move {
            while let Ok(params) = rx.recv().await {
                if let Some(fraction) = params.get("fraction").and_then(|f| f.as_f64()) {
                    emit_progress(&app, &conv_id, "transcribing", "running", Some(fraction));
                }
            }
        })
    };

    let transcribe = state
        .python
        .send(TranscribeFinal {
            conversation_id: conv_id.clone(),
            mic_path,
            system_path,
        })
        .await;
    progress_task.abort();

    let transcribe = match transcribe {
        Ok(resp) => resp,
        Err(err) => {
            fail_pipeline(&app, &conv_id, "transcribing", &err).await;
            return;
        }
    };
    tracing::info!(
        conv_id,
        segment_count = transcribe.segment_count,
        duration_ms = transcribe.duration_ms,
        "recording.transcribe_final_done"
    );
    if let Err(err) = state
        .storage
        .set_pipeline_step(&conv_id, PipelineStep::Transcribing, None)
        .await
    {
        fail_pipeline(&app, &conv_id, "transcribing", &err).await;
        return;
    }
    track_step_completed(
        &state,
        "transcribing",
        transcribe_start.elapsed(),
        meeting_bucket,
    );
    emit_progress(&app, &conv_id, "transcribing", "done", Some(1.0));

    emit_progress(&app, &conv_id, "extracting", "running", None);
    let extracting_start = std::time::Instant::now();
    let outcome =
        match crate::memory::extract_conversation(&state.storage, &state.python, &conv_id, false)
            .await
        {
            Ok(outcome) => outcome,
            Err(err) => {
                fail_pipeline(&app, &conv_id, "extracting", &err).await;
                return;
            }
        };
    track_step_completed(
        &state,
        "extracting",
        extracting_start.elapsed(),
        meeting_bucket,
    );
    state.metrics.track(
        crate::metrics::events::EXTRACTION_COMPLETED,
        crate::metrics::properties::EventProperties::from([
            (
                "action_items_count",
                crate::metrics::properties::PropertyValue::UInt(outcome.action_items as u64),
            ),
            (
                "decisions_count",
                crate::metrics::properties::PropertyValue::UInt(outcome.decisions as u64),
            ),
            (
                "open_questions_count",
                crate::metrics::properties::PropertyValue::UInt(outcome.open_questions as u64),
            ),
            (
                "bookmarks_count",
                crate::metrics::properties::PropertyValue::UInt(outcome.bookmarks as u64),
            ),
        ]),
    );
    // `extract_conversation` already advanced `pipeline_state` to
    // `Extracting` internally — reflects here
    // as the "extracting done" progress tick before the (best-effort)
    // project-memory refresh and the final `Done` transition.
    emit_progress(&app, &conv_id, "extracting", "done", None);

    // Auto-refresh trigger. Best-effort: a refresh
    // failure is surfaced via `project-memory-refresh-failed`, not by
    // failing this conversation's own pipeline (refresh is project-scoped,
    // not a step of `PipelineStep`). Unfiled conversations (no project
    // assigned — recordings never require one) have no memory doc to refresh at all.
    if let Some(project_id) = project_id.clone() {
        match state.storage.get_project(&project_id).await {
            Ok(project) => {
                match crate::memory::maybe_auto_refresh(
                    &state.storage,
                    &state.python,
                    &state.metrics,
                    &project_id,
                    &project.name,
                    &conv_id,
                )
                .await
                {
                    Ok(Some(refresh)) => {
                        let _ = crate::events::ProjectMemoryUpdated {
                            project_id,
                            significant_change: refresh.significant_change,
                        }
                        .emit(&app);
                    }
                    Ok(None) => {}
                    Err(err) => {
                        tracing::error!(project_id, error = %err, "recording.auto_refresh_failed");
                        let _ = crate::events::ProjectMemoryRefreshFailed {
                            project_id,
                            error_kind: err.to_string(),
                        }
                        .emit(&app);
                    }
                }
            }
            Err(err) => {
                tracing::error!(project_id, error = %err, "recording.auto_refresh_project_lookup_failed");
            }
        }
    }

    if let Err(err) = state
        .storage
        .set_pipeline_step(&conv_id, PipelineStep::Done, None)
        .await
    {
        fail_pipeline(&app, &conv_id, "extracting", &err).await;
        return;
    }
    state.metrics.track(
        crate::metrics::events::PIPELINE_COMPLETED,
        crate::metrics::properties::EventProperties::from([
            (
                "total_duration_ms",
                crate::metrics::properties::PropertyValue::UInt(
                    pipeline_start.elapsed().as_millis() as u64,
                ),
            ),
            (
                "meeting_duration_bucket",
                crate::metrics::properties::PropertyValue::Enum(meeting_bucket),
            ),
        ]),
    );
    let _ = state
        .storage
        .update_conversation_status(
            &conv_id,
            ConversationStatus::Ready,
            Some(ended_at),
            Some(duration_s),
        )
        .await;
    emit_progress(&app, &conv_id, "done", "done", None);

    let _ = crate::events::ConversationReady {
        conversation_id: conv_id,
        project_id,
    }
    .emit(&app);
}

/// Coarse, closed-set classification of an `AppError` for
/// `metrics::events::PIPELINE_STEP_FAILED` — reads only the `#[serde(tag =
/// "kind")]` discriminator `AppError`'s own `Serialize` impl produces
/// (`"not_found"`, `"worker_unavailable"`, ...), never the free-text
/// `message`/`entity`/`id` fields serialized alongside it.
fn error_kind_of(err: &AppError) -> String {
    serde_json::to_value(err)
        .ok()
        .and_then(|v| v.get("kind").and_then(|k| k.as_str().map(str::to_string)))
        .unwrap_or_else(|| "unknown".to_string())
}

async fn fail_pipeline(app: &AppHandle, conv_id: &str, step: &str, err: &AppError) {
    let state = app.state::<AppState>();
    let message = err.to_string();
    tracing::error!(conv_id, step, error = %message, "recording.pipeline_failed");
    state.metrics.track(
        crate::metrics::events::PIPELINE_STEP_FAILED,
        crate::metrics::properties::EventProperties::from([
            (
                "step",
                crate::metrics::properties::PropertyValue::EnumOwned(step.to_string()),
            ),
            (
                "error_kind",
                crate::metrics::properties::PropertyValue::EnumOwned(error_kind_of(err)),
            ),
        ]),
    );
    let _ = state
        .storage
        .set_pipeline_step(conv_id, PipelineStep::Failed, Some(message.clone()))
        .await;
    let _ = state
        .storage
        .update_conversation_status(conv_id, ConversationStatus::Failed, None, None)
        .await;
    emit_progress_with_error(app, conv_id, step, "failed", None, Some(message));
}

/// Crash recovery (12_CORNER_CASES.md "App crashes & recovery" §Mid-recording
/// crash). Called once by the frontend on app launch
/// (`useCrashRecoveryCheck`). A conversation can only be at
/// `status = 'recording'` while a live `ActiveSession` holds it — if that
/// never happened this boot (fresh `RecordingRegistry`, empty), any row
/// still at that status was orphaned by an unclean shutdown of a *previous*
/// run. Trusting `status` alone is safe for exactly this reason: this
/// command's own caller (app boot) always runs before `start_recording` can
/// create a new one this session.
#[tauri::command]
#[specta::specta]
pub async fn list_interrupted_recordings(
    state: State<'_, AppState>,
) -> Result<Vec<Conversation>, AppError> {
    state.storage.list_interrupted_recordings().await
}

/// Crash recovery's "Discard" action (12_CORNER_CASES.md, same section).
/// Reuses `StorageService::delete_conversation` — the same atomic,
/// resumable-on-crash delete path `conversation.retry_step`/Conversation
/// Detail's (not-yet-built) delete affordance would use, so a discard that
/// itself gets interrupted mid-delete is cleaned up by the *existing*
/// `resume_pending_deletes()` pass in `lib.rs`'s `setup()` rather than
/// needing its own recovery story.
#[tauri::command]
#[specta::specta]
pub async fn discard_interrupted_recording(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<(), AppError> {
    let result = state.storage.delete_conversation(&conversation_id).await;
    if result.is_ok() {
        state.metrics.track(
            crate::metrics::events::RECORDING_DISCARDED_AFTER_CRASH,
            crate::metrics::properties::EventProperties::new(),
        );
    }
    result
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct RecoverInterruptedResult {
    /// `true` when there was nothing worth recovering (see doc comment
    /// below) and the row was auto-discarded instead of being sent through
    /// the pipeline — the frontend shows a different toast for this case
    /// than for "recovery started".
    pub discarded_no_audio: bool,
}

/// 16kHz mono 16-bit PCM, matching every other byte-count-to-duration
/// calculation in this file.
const BYTES_PER_SEC: u64 = 16_000 * 2;

/// Crash recovery's "Recover" action (12_CORNER_CASES.md "App crashes &
/// recovery" §Mid-recording crash: "[Recover] runs post-processing on the
/// partial audio"). There is no live `ActiveSession` to resume from — the
/// whole point of "orphaned" is that the process that held one is gone — so
/// this reconstructs just enough of `stop_recording`'s tail to hand the
/// on-disk `mic.wav`/`system.wav` to the exact same
/// `run_post_recording_pipeline` a normal Stop uses, rather than growing a
/// second, parallel post-processing path.
///
/// **Corner case — missing/near-empty audio.** A crash before the sidecar
/// ever flushed a chunk (or a conversation folder some other process already
/// touched) can leave `mic.wav`/`system.wav` at zero bytes or a few
/// milliseconds of audio. Running the full pipeline on that produces a
/// "transcription" with nothing in it and an extraction step that has
/// nothing to summarize — a confusing dead end, not a recovered conversation.
/// Follows the same rule as the rest of the pipeline: enqueue
/// `process_conversation` if at least 5s of audio was written, otherwise
/// delete the row. Below that threshold this auto-discards (same atomic path as the
/// "Discard" button) and tells the caller so, instead of offering a Recover
/// that can't recover anything or silently failing the pipeline a few
/// seconds later with a confusing "no speech" result.
#[tauri::command]
#[specta::specta]
pub async fn recover_interrupted_recording(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<RecoverInterruptedResult, AppError> {
    let conversation = state.storage.get_conversation(&conversation_id).await?;
    // Idempotent: a second click (or a second window's copy of the modal)
    // after this has already been recovered/discarded is a clean no-op, not
    // an error — the row is no longer `Recording` by the time either action
    // has run once.
    if conversation.status != ConversationStatus::Recording {
        return Ok(RecoverInterruptedResult {
            discarded_no_audio: false,
        });
    }

    let mic_path = paths::mic_wav_path(&conversation_id)?;
    let system_path = paths::system_wav_path(&conversation_id)?;
    let mic_bytes = std::fs::metadata(&mic_path).map(|m| m.len()).unwrap_or(0);
    let system_bytes = std::fs::metadata(&system_path)
        .map(|m| m.len())
        .unwrap_or(0);

    const MIN_AUDIO_BYTES: u64 = BYTES_PER_SEC * 5;
    if mic_bytes < MIN_AUDIO_BYTES && system_bytes < MIN_AUDIO_BYTES {
        state.storage.delete_conversation(&conversation_id).await?;
        state.metrics.track(
            crate::metrics::events::RECORDING_DISCARDED_AFTER_CRASH,
            crate::metrics::properties::EventProperties::new(),
        );
        return Ok(RecoverInterruptedResult {
            discarded_no_audio: true,
        });
    }

    let ended_at = now_ms() / 1000;
    // Best-effort duration from the longer file's byte length, not
    // `conversation.started_at` vs. "now" — the gap between the crash and
    // this Recover click (which could be minutes or days) would otherwise
    // be counted as recording time.
    let duration_s = (mic_bytes.max(system_bytes) / BYTES_PER_SEC) as i64;

    state
        .storage
        .update_conversation_status(
            &conversation_id,
            ConversationStatus::Processing,
            Some(ended_at),
            Some(duration_s),
        )
        .await?;
    state
        .storage
        .set_pipeline_step(&conversation_id, PipelineStep::Finalizing, None)
        .await?;
    emit_progress(&app, &conversation_id, "finalizing", "running", None);
    state.metrics.track(
        crate::metrics::events::RECORDING_RECOVERED,
        crate::metrics::properties::EventProperties::new(),
    );

    tokio::spawn(run_post_recording_pipeline(
        app,
        conversation_id,
        conversation.project_id,
        mic_path,
        system_path,
        ended_at,
        duration_s,
    ));

    Ok(RecoverInterruptedResult {
        discarded_no_audio: false,
    })
}

/// The mid-*processing* counterpart to `list_interrupted_recordings`
/// above (12_CORNER_CASES.md "App crashes & recovery" §Mid-processing crash:
/// "This conversation was still processing when Mnemos closed. Continue?").
/// This half of the corner-cases spec was never built: a conversation
/// crashed (or force-quit) mid-`run_post_recording_pipeline` was left at
/// `status = 'processing'` forever, with no boot-time reconciliation and no
/// UI — `ProcessingOverlay` just spins on a `processing-progress` topic
/// nothing will ever publish to again. Same reasoning as the recording-crash
/// case: the registry that would hold a live pipeline task is always empty
/// at boot, so any row still `processing` this early was orphaned by a
/// *previous* run.
#[tauri::command]
#[specta::specta]
pub async fn list_stuck_processing(
    state: State<'_, AppState>,
) -> Result<Vec<Conversation>, AppError> {
    state.storage.list_stuck_processing().await
}

/// "Discard" for a stuck-processing conversation — same atomic delete path
/// as everything else in this file, so a discard interrupted by *another*
/// crash is cleaned up by the existing `resume_pending_deletes()` boot pass.
#[tauri::command]
#[specta::specta]
pub async fn discard_stuck_processing(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<(), AppError> {
    let result = state.storage.delete_conversation(&conversation_id).await;
    if result.is_ok() {
        state.metrics.track(
            crate::metrics::events::STUCK_PROCESSING_DISCARDED,
            crate::metrics::properties::EventProperties::new(),
        );
    }
    result
}

/// "Continue" for a stuck-processing conversation. Unlike mid-recording
/// recovery there is no partial-audio judgment call to make here — `mic.wav`
/// /`system.wav` are already complete (capture finished normally; it was the
/// *pipeline* that got cut off), so this just re-runs the same
/// `run_post_recording_pipeline` a normal Stop uses, from the top. Simpler
/// than resuming from the exact last-completed step (transcription is
/// fast — thanks to chunking — and idempotent to redo), and correct
/// regardless of whether the crash landed mid-transcription or
/// mid-extraction.
#[tauri::command]
#[specta::specta]
pub async fn resume_stuck_processing(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<(), AppError> {
    let conversation = state.storage.get_conversation(&conversation_id).await?;
    // Idempotent: a second click after this has already been resumed/
    // discarded is a clean no-op, not an error.
    if conversation.status != ConversationStatus::Processing {
        return Ok(());
    }

    let mic_path = paths::mic_wav_path(&conversation_id)?;
    let system_path = paths::system_wav_path(&conversation_id)?;
    let ended_at = conversation.ended_at.unwrap_or_else(|| now_ms() / 1000);
    let duration_s = conversation.duration_s.unwrap_or(0);

    state
        .storage
        .set_pipeline_step(&conversation_id, PipelineStep::Finalizing, None)
        .await?;
    emit_progress(&app, &conversation_id, "finalizing", "running", None);
    state.metrics.track(
        crate::metrics::events::STUCK_PROCESSING_RESUMED,
        crate::metrics::properties::EventProperties::new(),
    );

    tokio::spawn(run_post_recording_pipeline(
        app,
        conversation_id,
        conversation.project_id,
        mic_path,
        system_path,
        ended_at,
        duration_s,
    ));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::tray::TrayState;

    /// Registers a synthetic session so the registry's derived state can be
    /// tested without standing up capture. Only the fields `tray_snapshot`
    /// reads carry meaning here.
    fn insert_session(registry: &RecordingRegistry, session_id: u32, started_at_ms: i64) {
        let (level_tx, _rx) = broadcast::channel(1);
        registry.sessions.lock().unwrap().insert(
            session_id,
            ActiveSession {
                conversation_id: format!("conv-{session_id}"),
                project_id: None,
                mic_path: PathBuf::from("mic.wav"),
                system_path: PathBuf::from("system.wav"),
                started_at_ms,
                paused_at_ms: None,
                transcript_forward_task: None,
                warmup_forward_task: None,
                level_tx,
                level_forward_task: None,
                #[cfg(target_os = "macos")]
                sidecar: None,
                capture_watch_task: None,
            },
        );
    }

    #[test]
    fn no_session_reads_as_idle() {
        let registry = RecordingRegistry::new();
        assert_eq!(registry.tray_snapshot(), TrayState::Idle);
        assert!(!registry.has_active_session());
    }

    #[test]
    fn a_running_session_reports_elapsed_from_its_start() {
        let registry = RecordingRegistry::new();
        insert_session(&registry, 1, now_ms() - 5_000);

        match registry.tray_snapshot() {
            TrayState::Recording { elapsed_s } => assert!(
                (4..=6).contains(&elapsed_s),
                "expected ~5s elapsed, got {elapsed_s}"
            ),
            other => panic!("expected Recording, got {other:?}"),
        }
    }

    /// The frontend's clock stops while paused, so the tray's has to stop at
    /// the same reading — not keep counting, and not reset.
    #[test]
    fn pausing_freezes_the_elapsed_reading() {
        let registry = RecordingRegistry::new();
        insert_session(&registry, 1, now_ms() - 30_000);
        assert!(registry.set_paused(1, true));

        let first = match registry.tray_snapshot() {
            TrayState::Paused { elapsed_s } => elapsed_s,
            other => panic!("expected Paused, got {other:?}"),
        };
        assert!(
            (29..=31).contains(&first),
            "expected ~30s frozen, got {first}"
        );

        // Time passing must not move a paused reading.
        std::thread::sleep(std::time::Duration::from_millis(1_100));
        match registry.tray_snapshot() {
            TrayState::Paused { elapsed_s } => assert_eq!(
                elapsed_s, first,
                "a paused reading must not advance with the clock"
            ),
            other => panic!("expected Paused, got {other:?}"),
        }
    }

    #[test]
    fn resuming_returns_to_a_running_clock() {
        let registry = RecordingRegistry::new();
        insert_session(&registry, 1, now_ms() - 10_000);
        assert!(registry.set_paused(1, true));
        assert!(registry.set_paused(1, false));

        assert!(matches!(
            registry.tray_snapshot(),
            TrayState::Recording { .. }
        ));
    }

    /// A stale session id must not panic — the same reasoning the registry is
    /// keyed by id at all.
    #[test]
    fn pausing_an_unknown_session_is_a_clean_no_op() {
        let registry = RecordingRegistry::new();
        assert!(!registry.set_paused(99, true));
        assert_eq!(registry.tray_snapshot(), TrayState::Idle);
    }
}
