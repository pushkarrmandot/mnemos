//! `WorkerSupervisor` — the Rust-side owner of the persistent Python
//! `mnemos-worker` process lifetime (LLD-02 §3–§7). Spawns it, frames
//! JSON-RPC 2.0 over its stdio, health-checks it, restarts it with backoff,
//! and replays `pending_jobs.json` / `current_job.json` across a crash.
//!
//! No Swift sidecar (W7) and no concrete reverse-RPC handler (the
//! `run_agent_extraction` / `secrets.*` handlers are W8+ — see this wave's
//! "Implementation status" in `lld/LLD_02_WORKER_SUPERVISOR.md` for why).

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex as AsyncMutex};

use crate::error::AppError;
use crate::ipc::framing::{read_frame, write_frame};

// ---------------------------------------------------------------------
// Public config + request/notification traits
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    /// Python interpreter (bundled at build time in production; a plain
    /// `python3` on PATH in dev — no sidecar bundler exists yet, see the
    /// LLD's "Implementation status" for why that's out of scope here).
    pub python_bin: PathBuf,
    /// `-m <worker_module>` — defaults to `mnemos_worker`.
    pub worker_module: String,
    /// Working directory the interpreter is launched from (`src-python/`).
    pub cwd: PathBuf,
    /// `~/Mnemos/state` — manifest + pending_jobs.json + current_job.json live here.
    pub state_dir: PathBuf,
    pub protocol_version: u32,
    pub handshake_timeout: Duration,
    pub health_check_interval: Duration,
    pub health_check_timeout: Duration,
    pub heartbeat_timeout: Duration,
    pub ttl_sweep_interval: Duration,
    pub restart_backoff_base: Duration,
    pub restart_backoff_cap: Duration,
    pub restart_stable_after: Duration,
    pub restart_budget_max_failures: usize,
    pub restart_budget_window: Duration,
    pub shutdown_grace: Duration,
    /// macOS only: path to the compiled `mnemos-audio` sidecar binary. No
    /// production sidecar-bundler packaging exists yet (same gap as
    /// `python_bin` — see this LLD's "Implementation status"), so in dev
    /// this points at the locally built `swift/mnemos-audio/.build/*`
    /// binary.
    #[cfg(target_os = "macos")]
    pub sidecar_bin: PathBuf,
}

impl SupervisorConfig {
    /// Production defaults (LLD-02 §5.1, §5.4, BACKEND §2). Callers still
    /// must set `python_bin`, `cwd`, `state_dir`.
    pub fn new(python_bin: PathBuf, cwd: PathBuf, state_dir: PathBuf) -> Self {
        Self {
            python_bin,
            worker_module: "mnemos_worker".to_string(),
            cwd,
            state_dir,
            protocol_version: 1,
            handshake_timeout: Duration::from_secs(10),
            health_check_interval: Duration::from_secs(30),
            health_check_timeout: Duration::from_secs(5),
            heartbeat_timeout: Duration::from_secs(45),
            ttl_sweep_interval: Duration::from_secs(5),
            restart_backoff_base: Duration::from_secs(1),
            restart_backoff_cap: Duration::from_secs(16),
            restart_stable_after: Duration::from_secs(60),
            restart_budget_max_failures: 3,
            restart_budget_window: Duration::from_secs(5 * 60),
            shutdown_grace: Duration::from_secs(30),
            #[cfg(target_os = "macos")]
            sidecar_bin: PathBuf::from("mnemos-audio"),
        }
    }
}

pub trait WorkerRequest: Serialize {
    type Response: DeserializeOwned;
    const METHOD: &'static str;
    /// Default TTL for the request/response registry + TTL sweep (§4.6).
    fn ttl(&self) -> Duration {
        Duration::from_secs(30)
    }
}

pub trait WorkerNotification: Serialize {
    const METHOD: &'static str;
}

/// `ping` — the one real job kind this wave's Python skeleton ships.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Ping {
    /// Test/diagnostic hook: makes the job run for `delay_ms` before
    /// replying, so a caller can simulate a slow/in-flight job.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<u64>,
    /// Idempotency key for job-queue replay (LLD-02 §6). Omitted, the
    /// worker treats each `ping` as unique.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PingResponse {
    pub pong: bool,
}

impl WorkerRequest for Ping {
    type Response = PingResponse;
    const METHOD: &'static str = "ping";
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthCheck {}

#[derive(Debug, Clone, Deserialize)]
pub struct HealthCheckResponse {
    pub ok: bool,
}

impl WorkerRequest for HealthCheck {
    type Response = HealthCheckResponse;
    const METHOD: &'static str = "health_check";

    fn ttl(&self) -> Duration {
        Duration::from_secs(5)
    }
}

#[derive(Debug, Clone, Serialize)]
struct Handshake {
    protocol_version: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct HandshakeResponse {
    protocol_version: u32,
}

impl WorkerRequest for Handshake {
    type Response = HandshakeResponse;
    const METHOD: &'static str = "handshake";

    fn ttl(&self) -> Duration {
        Duration::from_secs(10)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Shutdown {}

impl WorkerNotification for Shutdown {
    const METHOD: &'static str = "shutdown";
}

// ---------------------------------------------------------------------
// Windows capture (LLD-03 §3.2, §4.2) — the mic+loopback WASAPI thread
// runs inside the persistent worker, off the job executor. macOS has no
// use for these; capture there goes through `ipc::swift` instead.
// ---------------------------------------------------------------------

/// Topic name for the Windows capture thread's `capture_event`
/// notifications (LLD-03 §3.2). Normalized into `capture::CaptureEvent` via
/// `capture::capture_event_from_notification`.
pub const CAPTURE_EVENT_TOPIC: &str = "capture_event";

#[derive(Debug, Clone, Serialize)]
pub struct StartCapture {
    pub conversation_id: String,
    pub mic_path: PathBuf,
    pub system_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mic_device_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StartCaptureResponse {
    pub started_at_ms: i64,
}

impl WorkerRequest for StartCapture {
    type Response = StartCaptureResponse;
    const METHOD: &'static str = "start_capture";
}

#[derive(Debug, Clone, Serialize)]
pub struct StopCapture {
    pub conversation_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StopCaptureResponse {
    pub mic_bytes: u64,
    pub system_bytes: u64,
}

impl WorkerRequest for StopCapture {
    type Response = StopCaptureResponse;
    const METHOD: &'static str = "stop_capture";
}

#[derive(Debug, Clone, Serialize)]
pub struct PauseCapture {
    pub conversation_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResumeCapture {
    pub conversation_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaptureAck {}

impl WorkerRequest for PauseCapture {
    type Response = CaptureAck;
    const METHOD: &'static str = "pause_capture";
}

impl WorkerRequest for ResumeCapture {
    type Response = CaptureAck;
    const METHOD: &'static str = "resume_capture";
}

// ---------------------------------------------------------------------
// Transcription (LLD-03 §3.2, §5, §6) — the worker's live-transcription
// poll thread and its `transcribe_final` post-processing pass. Mirrors the
// capture section above: these are the RPC types + notification topic a
// future `RecordingService` (W9) sends/subscribes through — no
// `RecordingService`, no `recording.*` Tauri command, and no reader task
// that forwards to a Tauri `Channel<TranscriptChunk>` exist yet (see this
// wave's "Implementation status" in LLD-03 for the rationale, same
// deviation W7a already took for `StartCapture`/`StopCapture`).
// ---------------------------------------------------------------------

/// Topic name for `live_transcript_chunk` notifications (LLD-03 §5.2).
/// Payload shape is `LiveTranscriptChunkNotification` below.
pub const LIVE_TRANSCRIPT_CHUNK_TOPIC: &str = "live_transcript_chunk";

#[derive(Debug, Clone, Serialize)]
pub struct SubscribeLiveTranscript {
    pub conversation_id: String,
    pub mic_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubscribeLiveTranscriptResponse {}

impl WorkerRequest for SubscribeLiveTranscript {
    type Response = SubscribeLiveTranscriptResponse;
    const METHOD: &'static str = "subscribe_live_transcript";
}

#[derive(Debug, Clone, Serialize)]
pub struct UnsubscribeLiveTranscript {
    pub conversation_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UnsubscribeLiveTranscriptResponse {}

impl WorkerRequest for UnsubscribeLiveTranscript {
    type Response = UnsubscribeLiveTranscriptResponse;
    const METHOD: &'static str = "unsubscribe_live_transcript";

    fn ttl(&self) -> Duration {
        // Handler joins the live thread with its own 10s cap (LLD-03 §3.2)
        // before replying — give the request registry enough room not to
        // time out first.
        Duration::from_secs(15)
    }
}

/// The `transcribe_final` post-processing pass (LLD-03 §3.2, §6.2): two
/// full-file Parakeet passes + merge, run through the worker's job queue
/// (not the fast path above) since it can take up to ~30s.
#[derive(Debug, Clone, Serialize)]
pub struct TranscribeFinal {
    pub conversation_id: String,
    pub mic_path: PathBuf,
    pub system_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TranscribeFinalResponse {
    pub transcript_path: PathBuf,
    pub segment_count: u32,
    pub duration_ms: u64,
}

impl WorkerRequest for TranscribeFinal {
    type Response = TranscribeFinalResponse;
    const METHOD: &'static str = "transcribe_final";

    fn ttl(&self) -> Duration {
        // HLD NFR budget: post-batch holds `_infer_lock` ≤30s per pass;
        // two passes plus queueing behind an in-flight live-tx drain tick
        // gives this some margin (LLD-03 §7).
        Duration::from_secs(60)
    }
}

// ---------------------------------------------------------------------
// Model download progress (W15 onboarding) — status poll + notification
// topic for the one v1-required model (Parakeet TDT 0.6B). No "start
// download" request exists deliberately: `ParakeetModel.warm_up()` already
// triggers the real download eagerly at worker boot (Wave-5-Patch), so
// onboarding only ever observes it. See `models/transcription.py`'s
// `MODEL_METHODS`/`_DownloadProgress` for the Python side.
// ---------------------------------------------------------------------

/// Topic name for `model_download_progress` notifications.
pub const MODEL_DOWNLOAD_PROGRESS_TOPIC: &str = "model_download_progress";

#[derive(Debug, Clone, Serialize)]
pub struct ModelDownloadStatus {}

#[derive(Debug, Clone, Deserialize, Serialize, specta::Type)]
pub struct ModelDownloadStatusResponse {
    pub model_id: String,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub done: bool,
}

impl WorkerRequest for ModelDownloadStatus {
    type Response = ModelDownloadStatusResponse;
    const METHOD: &'static str = "model_download_status";
}

/// Payload of one `live_transcript_chunk` notification (LLD-03 §5.1's
/// `_tick`). `conversation_id` disambiguates when more than one
/// conversation's live thread could theoretically be running (v1 asserts
/// at most one).
#[derive(Debug, Clone, Deserialize)]
pub struct LiveTranscriptChunkNotification {
    pub conversation_id: String,
    pub chunk: LiveTranscriptChunkPayload,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LiveTranscriptChunkPayload {
    pub speaker_label_hint: Option<String>,
    pub text: String,
    pub ts_start_ms: i64,
    pub ts_end_ms: i64,
}

/// W17b: topic name for `live_transcription_warmup` notifications
/// (`live_transcription.py`'s `_notify_warmup_state_if_changed`). Fired once
/// per readiness transition, not per poll tick — `ready: false` means live
/// transcription is blocked on `ParakeetModel` warm-up.
pub const LIVE_TRANSCRIPTION_WARMUP_TOPIC: &str = "live_transcription_warmup";

#[derive(Debug, Clone, Deserialize)]
pub struct LiveTranscriptionWarmupNotification {
    pub conversation_id: String,
    pub ready: bool,
}

// ---------------------------------------------------------------------
// ---------------------------------------------------------------------
// Memory system (LLD-05 §3.2, §4.2, §5.2) — the two forward jobs
// `extract_memory` / `refresh_project_memory`. Each job's handler makes
// its own `run_agent_extraction` reverse-RPC call(s) (§7 below) to drive
// the agent turn; what comes back here is the worker's already-validated,
// already-schema-checked JSON (LLD-05 §4.3/§5.3) — this layer's only job is
// to persist it (`memory::extract_conversation` / `memory::refresh_project`).
// ---------------------------------------------------------------------

/// One `action_items[]` entry per LLD-05 §4.3. `assignee_contact_id` is
/// accepted-but-dropped in v1 — contacts don't exist yet (this wave's brief:
/// "pass an empty list; do not build contact lookup").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedActionItem {
    pub text: String,
    #[serde(default)]
    pub assignee_hint: Option<String>,
    #[serde(default)]
    pub assignee_contact_id: Option<String>,
    #[serde(default)]
    pub due_hint: Option<String>,
    #[serde(default)]
    pub source_timestamp_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedDecision {
    pub statement: String,
    #[serde(default)]
    pub decided_by_hint: Option<String>,
    #[serde(default)]
    pub quote: Option<String>,
    #[serde(default)]
    pub source_timestamp_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedOpenQuestion {
    pub question: String,
    #[serde(default)]
    pub raised_by_hint: Option<String>,
    #[serde(default)]
    pub source_timestamp_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedBookmark {
    pub text: String,
    pub timestamp_ms: i64,
}

/// `extract_memory` (LLD-05 §3.2/§4.2) — forward job: Rust hands the worker
/// the raw transcript + context, the worker's job handler is the one that
/// actually calls `run_agent_extraction` (§7.2) and validates/retries the
/// schema; this request/response pair is just the forward-RPC envelope
/// around that.
#[derive(Debug, Clone, Serialize)]
pub struct ExtractMemory {
    pub conversation_id: String,
    /// Raw `transcript.json` contents (LLD-03 §6 shape — `turns[]` with
    /// `speaker_label`/`text`/`ts_start_ms`/`ts_end_ms`).
    pub transcript: Value,
    /// v1: always `[]` — contacts don't exist yet (this wave's brief).
    #[serde(default)]
    pub contacts: Vec<Value>,
    #[serde(default)]
    pub notes: Option<String>,
    pub conversation_meta: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractMemoryResponse {
    pub summary_markdown: String,
    #[serde(default)]
    pub action_items: Vec<ExtractedActionItem>,
    #[serde(default)]
    pub decisions: Vec<ExtractedDecision>,
    #[serde(default)]
    pub open_questions: Vec<ExtractedOpenQuestion>,
    #[serde(default)]
    pub bookmarks: Vec<ExtractedBookmark>,
}

impl WorkerRequest for ExtractMemory {
    type Response = ExtractMemoryResponse;
    const METHOD: &'static str = "extract_memory";

    fn ttl(&self) -> Duration {
        // One `run_agent_extraction` turn (30s, §4.2) plus one schema-retry
        // turn (§4.5) plus scheduling slack.
        Duration::from_secs(75)
    }
}

/// `refresh_project_memory` (LLD-05 §3.2/§5.2). `current_memory: None` is
/// the `# NEW PROJECT` sentinel (§5.2/§6.2). The worker computes the diff
/// guardrail (§5.4) itself — `difflib.SequenceMatcher` has no Rust-side
/// equivalent worth adding for one bool.
#[derive(Debug, Clone, Serialize)]
pub struct RefreshProjectMemory {
    pub project_id: String,
    pub current_memory: Option<Value>,
    pub new_extractions: Vec<Value>,
    pub project_meta: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RefreshProjectMemoryResponse {
    pub overview_markdown: String,
    pub scope_drift_markdown: String,
    #[serde(default)]
    pub supersessions: Vec<Value>,
    pub significant_change: bool,
    pub diff_ratio: f64,
}

impl WorkerRequest for RefreshProjectMemory {
    type Response = RefreshProjectMemoryResponse;
    const METHOD: &'static str = "refresh_project_memory";

    fn ttl(&self) -> Duration {
        // One `run_agent_extraction` turn (60s, §5.2) plus one schema-retry
        // turn plus scheduling slack.
        Duration::from_secs(135)
    }
}

// ---------------------------------------------------------------------
// Reverse RPC (§7) — generic dispatch mechanism only. No production
// handler is registered by this wave (see module doc comment).
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ReverseRpcError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

#[async_trait::async_trait]
pub trait ReverseRpcHandler: Send + Sync + 'static {
    async fn handle(&self, params: Value) -> Result<Value, ReverseRpcError>;
}

// ---------------------------------------------------------------------
// Internal wire + registry types
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
struct JsonRpcErrorObj {
    code: i64,
    message: String,
    /// Read by `map_json_rpc_error` for `-32023`'s `resets_at`. Note the
    /// *job* path never populates it: `job_executor.py` builds its error as
    /// `{"code", "message"}` only, because `WorkerJobError` has no data
    /// field. So `resets_at` arrives `None` for extraction/refresh failures
    /// and is only meaningful on paths that surface the error directly.
    /// Threading it through the worker is deliberate future work, not an
    /// oversight — nothing consumes it yet.
    data: Option<Value>,
}

struct PendingEntry {
    tx: oneshot::Sender<Result<Value, JsonRpcErrorObj>>,
    #[allow(dead_code)]
    method: &'static str,
    deadline: Instant,
}

struct TransportHandle {
    writer_tx: mpsc::Sender<Vec<u8>>,
    child: Arc<AsyncMutex<Child>>,
}

#[derive(Default)]
struct RestartState {
    backoff: Duration,
    failures: VecDeque<Instant>,
    permanent_reason: Option<String>,
}

// ---------------------------------------------------------------------
// WorkerSupervisor
// ---------------------------------------------------------------------

pub struct WorkerSupervisor {
    cfg: SupervisorConfig,
    next_id: AtomicU64,
    registry: StdMutex<HashMap<u64, PendingEntry>>,
    transport: StdMutex<Option<TransportHandle>>,
    reverse_handlers: StdMutex<HashMap<&'static str, Arc<dyn ReverseRpcHandler>>>,
    notify_topics: StdMutex<HashMap<String, broadcast::Sender<Value>>>,
    last_heartbeat: StdMutex<Instant>,
    restart_state: StdMutex<RestartState>,
    /// At most one live sidecar in v1 (LLD-02 §8.2). `shutdown()` walks
    /// this to stop it before killing the worker.
    #[cfg(target_os = "macos")]
    live_sidecar: StdMutex<Option<crate::ipc::swift::SidecarControl>>,
}

impl WorkerSupervisor {
    /// Boots the worker. Always returns `Ok` — a missing interpreter or a
    /// dead worker is represented as a permanently-unavailable internal
    /// state (surfaced by every `send()` as `WorkerUnavailable`) rather than
    /// a hard error here, so app boot never depends on the worker being
    /// reachable (LLD-02 §5.5's "permanent" classification, applied at the
    /// `send()` boundary instead of at `spawn()` — see this wave's
    /// "Implementation status" for the rationale).
    pub async fn spawn(cfg: SupervisorConfig) -> Result<Arc<Self>, AppError> {
        let sup = Arc::new(Self {
            cfg,
            next_id: AtomicU64::new(1),
            registry: StdMutex::new(HashMap::new()),
            transport: StdMutex::new(None),
            reverse_handlers: StdMutex::new(HashMap::new()),
            notify_topics: StdMutex::new(HashMap::new()),
            last_heartbeat: StdMutex::new(Instant::now()),
            restart_state: StdMutex::new(RestartState {
                backoff: Duration::from_secs(1),
                ..Default::default()
            }),
            #[cfg(target_os = "macos")]
            live_sidecar: StdMutex::new(None),
        });

        match sup.establish_connection().await {
            Ok(()) => {}
            Err(outcome) => {
                let permanent = matches!(outcome, EstablishOutcome::Permanent(_));
                let message = outcome.into_message();
                tracing::error!(permanent, error = %message, "worker.spawn_failed");
                if permanent {
                    sup.restart_state.lock().unwrap().permanent_reason = Some(message);
                } else {
                    // Transient (e.g. handshake raced a slow boot) — let the
                    // ordinary restart loop take over instead of failing app
                    // boot.
                    sup.clone().trigger_restart();
                }
            }
        }

        Ok(sup)
    }

    /// Register a reverse-RPC handler. Built-in handlers (`secrets.*`,
    /// `run_agent_extraction`) are NOT registered by this wave — see the
    /// module doc comment. Tests use this to inject fakes.
    pub fn register_reverse_rpc(&self, method: &'static str, handler: Arc<dyn ReverseRpcHandler>) {
        self.reverse_handlers
            .lock()
            .unwrap()
            .insert(method, handler);
    }

    /// Onboarding's permission-preflight commands (W15) spawn the sidecar
    /// binary directly, one-shot, outside the persistent-session protocol
    /// this struct otherwise owns — this just exposes the same path
    /// `spawn_sidecar` already resolves, so there's one source of truth for
    /// "where is `mnemos-audio`," not a second guess.
    #[cfg(target_os = "macos")]
    pub fn sidecar_bin(&self) -> &std::path::Path {
        &self.cfg.sidecar_bin
    }

    pub fn subscribe(&self, topic: &str) -> broadcast::Receiver<Value> {
        let mut topics = self.notify_topics.lock().unwrap();
        topics
            .entry(topic.to_string())
            .or_insert_with(|| broadcast::channel(256).0)
            .subscribe()
    }

    pub async fn send<Req>(&self, req: Req) -> Result<Req::Response, AppError>
    where
        Req: WorkerRequest + Send,
    {
        let value = self.send_raw(Req::METHOD, &req, req.ttl()).await?;
        serde_json::from_value(value)
            .map_err(|e| AppError::internal(format!("malformed {} response: {e}", Req::METHOD)))
    }

    pub async fn notify<N>(&self, n: N) -> Result<(), AppError>
    where
        N: WorkerNotification + Send,
    {
        let writer_tx = self.writer_tx_or_unavailable()?;
        let frame = build_frame(&NotificationFrame {
            jsonrpc: "2.0",
            method: N::METHOD,
            params: &n,
        })?;
        writer_tx
            .send(frame)
            .await
            .map_err(|_| self.unavailable_error())
    }

    /// Spawn a Swift sidecar for one recording session (LLD-02 §8, LLD-03
    /// §4.1). Sidecar state is disjoint from the Python worker's — this
    /// call does not touch `self.transport`/`self.registry` at all.
    #[cfg(target_os = "macos")]
    pub async fn spawn_sidecar(
        &self,
        cfg: crate::ipc::swift::SidecarConfig,
    ) -> Result<crate::ipc::swift::SidecarHandle, AppError> {
        let handle = crate::ipc::swift::spawn(&self.cfg.sidecar_bin, cfg).await?;
        *self.live_sidecar.lock().unwrap() = Some(handle.control.clone());
        Ok(handle)
    }

    /// Graceful shutdown of the worker. Called from Quit.
    pub async fn shutdown(&self) -> Result<(), AppError> {
        #[cfg(target_os = "macos")]
        {
            let sidecar = self.live_sidecar.lock().unwrap().take();
            if let Some(sidecar) = sidecar {
                let _ = sidecar.stop().await;
            }
        }

        let old = self.transport.lock().unwrap().take();
        let Some(handle) = old else {
            return Ok(());
        };
        let frame = build_frame(&NotificationFrame {
            jsonrpc: "2.0",
            method: Shutdown::METHOD,
            params: &Shutdown {},
        })?;
        let _ = handle.writer_tx.send(frame).await;
        self.registry.lock().unwrap().clear();

        let mut child = handle.child.lock().await;
        let waited = tokio::time::timeout(self.cfg.shutdown_grace, child.wait()).await;
        if waited.is_err() {
            tracing::warn!("worker.shutdown_grace_expired");
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
        Ok(())
    }

    // -- internals ---------------------------------------------------

    fn writer_tx_or_unavailable(&self) -> Result<mpsc::Sender<Vec<u8>>, AppError> {
        self.transport
            .lock()
            .unwrap()
            .as_ref()
            .map(|t| t.writer_tx.clone())
            .ok_or_else(|| self.unavailable_error())
    }

    fn unavailable_error(&self) -> AppError {
        let rs = self.restart_state.lock().unwrap();
        let retry_after_ms = if rs.permanent_reason.is_some() {
            0
        } else {
            rs.backoff.as_millis() as u64
        };
        AppError::WorkerUnavailable { retry_after_ms }
    }

    async fn send_raw(
        &self,
        method: &'static str,
        params: &impl Serialize,
        ttl: Duration,
    ) -> Result<Value, AppError> {
        let writer_tx = self.writer_tx_or_unavailable()?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut reg = self.registry.lock().unwrap();
            reg.insert(
                id,
                PendingEntry {
                    tx,
                    method,
                    deadline: Instant::now() + ttl,
                },
            );
        }
        let frame = build_frame(&RequestFrame {
            jsonrpc: "2.0",
            id: id.to_string(),
            method,
            params,
        })?;
        if writer_tx.send(frame).await.is_err() {
            self.registry.lock().unwrap().remove(&id);
            return Err(self.unavailable_error());
        }
        match rx.await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(map_json_rpc_error(err)),
            // Sender dropped without a reply: disconnect drained the
            // registry, or the TTL sweep evicted this entry.
            Err(_) => Err(self.unavailable_error()),
        }
    }

    /// Spawns the child, handshakes, starts the reader/writer/stderr/health
    /// tasks, and replays job-queue state files (LLD-02 §4.1, §6).
    async fn establish_connection(self: &Arc<Self>) -> Result<(), EstablishOutcome> {
        adopt_or_kill_stale_manifest(&self.cfg).await;

        let mut command = Command::new(&self.cfg.python_bin);
        command
            .arg("-m")
            .arg(&self.cfg.worker_module)
            .arg("--state-dir")
            .arg(&self.cfg.state_dir)
            .current_dir(&self.cfg.cwd)
            .env_clear()
            .env("PYTHONUNBUFFERED", "1")
            // Forces UTF-8 I/O regardless of the OS locale codepage —
            // required unconditionally (not just re-forwarded from the
            // parent env, since we `env_clear()`) to avoid
            // `UnicodeEncodeError` when structlog writes non-ASCII to
            // stderr under Windows' default cp1252 stderr encoding. See
            // Windows parity audit findings #4 and #15.
            .env("PYTHONUTF8", "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        // Allowlist kept intentionally narrow (not a blanket env passthrough
        // like the `claude` CLI spawn), but must include what Windows
        // CPython needs to start at all: `SystemRoot`/`TEMP`/`TMP` for
        // random-seed and winsock init, `COMSPEC`/`PATHEXT` for subprocess
        // and executable resolution, `APPDATA`/`LOCALAPPDATA` for anything
        // Python or its deps touch under the user profile. See Windows
        // parity audit finding #4. `SystemRoot` and `SYSTEMROOT` are both
        // listed since Windows env var lookups are case-insensitive but
        // `std::env::var` here is not, and different tools have historically
        // written either casing.
        for var in [
            "PATH",
            "HOME",
            "USERPROFILE",
            "SystemRoot",
            "SYSTEMROOT",
            "TEMP",
            "TMP",
            "COMSPEC",
            "PATHEXT",
            "APPDATA",
            "LOCALAPPDATA",
        ] {
            if let Ok(v) = std::env::var(var) {
                command.env(var, v);
            }
        }
        crate::procutil::suppress_console_window(&mut command);

        let mut child = command.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                EstablishOutcome::Permanent(format!("python interpreter not found: {e}"))
            } else {
                EstablishOutcome::Transient(format!("spawn failed: {e}"))
            }
        })?;

        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let pid = child.id();

        if let Some(pid) = pid {
            let manifest = WorkerManifest {
                pid,
                spawn_at: chrono_now_iso(),
                protocol_version: self.cfg.protocol_version,
            };
            let path = self.cfg.state_dir.join("worker-manifest.json");
            let _ = crate::fs::atomic::atomic_write_json(&path, &manifest);
        }

        // Handshake happens on the raw pipes, before the registry-backed
        // reader/writer tasks exist to route it (§4.1 step 7).
        let mut stdout_buf = BufReader::new(stdout);
        let handshake_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = RequestFrame {
            jsonrpc: "2.0",
            id: handshake_id.to_string(),
            method: Handshake::METHOD,
            params: &Handshake {
                protocol_version: self.cfg.protocol_version,
            },
        };
        let body = serde_json::to_vec(&req)
            .map_err(|e| EstablishOutcome::Transient(format!("encode handshake: {e}")))?;
        let mut stdin = stdin;
        write_frame(&mut stdin, &body)
            .await
            .map_err(|e| EstablishOutcome::Transient(format!("write handshake: {e}")))?;

        let handshake_result = tokio::time::timeout(self.cfg.handshake_timeout, async {
            loop {
                let body = read_frame(&mut stdout_buf)
                    .await
                    .map_err(|e| format!("framing error during handshake: {e}"))?
                    .ok_or_else(|| "worker closed stdout during handshake".to_string())?;
                let parsed: Value = serde_json::from_slice(&body)
                    .map_err(|e| format!("bad handshake JSON: {e}"))?;
                if parsed.get("id").and_then(Value::as_str) == Some(&handshake_id.to_string()) {
                    return Ok(parsed);
                }
                // Anything else this early (e.g. a heartbeat) is ignored.
            }
        })
        .await
        .map_err(|_| EstablishOutcome::Transient("handshake timed out".to_string()))?
        .map_err(EstablishOutcome::Transient)?;

        if let Some(err) = handshake_result.get("error") {
            let _ = child.start_kill();
            return Err(EstablishOutcome::Permanent(format!(
                "worker rejected handshake: {err}"
            )));
        }
        let reported: HandshakeResponse = handshake_result
            .get("result")
            .cloned()
            .and_then(|r| serde_json::from_value(r).ok())
            .unwrap_or(HandshakeResponse {
                protocol_version: 0,
            });
        if reported.protocol_version != self.cfg.protocol_version {
            let _ = child.start_kill();
            return Err(EstablishOutcome::Permanent(format!(
                "protocol version mismatch: worker={} host={}",
                reported.protocol_version, self.cfg.protocol_version
            )));
        }

        // Install the transport, then start the long-lived tasks.
        let (writer_tx, writer_rx) = mpsc::channel::<Vec<u8>>(32);
        let child = Arc::new(AsyncMutex::new(child));
        {
            let mut t = self.transport.lock().unwrap();
            *t = Some(TransportHandle {
                writer_tx: writer_tx.clone(),
                child: child.clone(),
            });
        }
        *self.last_heartbeat.lock().unwrap() = Instant::now();

        tokio::spawn(writer_task(stdin, writer_rx));
        tokio::spawn(reader_task(Arc::clone(self), stdout_buf));
        tokio::spawn(stderr_task(stderr));
        tokio::spawn(ttl_sweep_task(Arc::clone(self)));
        tokio::spawn(health_task(Arc::clone(self)));
        tokio::spawn(stable_reset_task(Arc::clone(self)));

        self.replay_state_files(&writer_tx).await;

        tracing::info!(component = "python-worker", "worker.connected");
        Ok(())
    }

    /// LLD-02 §6: reads `current_job.json` then `pending_jobs.json` and
    /// re-issues each as a fresh request. Nothing awaits the replies here —
    /// idempotency (owned by the job executor) makes a discarded late reply
    /// safe, and there is no original caller left to notify after a
    /// worker-only restart.
    async fn replay_state_files(&self, writer_tx: &mpsc::Sender<Vec<u8>>) {
        let current = self.cfg.state_dir.join("current_job.json");
        let pending = self.cfg.state_dir.join("pending_jobs.json");

        if let Some(job) = read_state_file_entry(&current) {
            self.replay_one(writer_tx, job).await;
        }
        if let Some(jobs) = read_state_file_jobs(&pending) {
            for job in jobs {
                self.replay_one(writer_tx, job).await;
            }
        }
    }

    async fn replay_one(&self, writer_tx: &mpsc::Sender<Vec<u8>>, job: ReplayJob) {
        let frame = match build_frame(&RequestFrame {
            jsonrpc: "2.0",
            id: job.id.clone(),
            method: &job.kind,
            params: &job.params,
        }) {
            Ok(f) => f,
            Err(e) => {
                tracing::error!(error = %e, "worker.replay.encode_failed");
                return;
            }
        };
        if writer_tx.send(frame).await.is_err() {
            tracing::warn!(job_id = %job.id, "worker.replay.send_failed");
        } else {
            tracing::info!(job_id = %job.id, kind = %job.kind, "worker.replay.sent");
        }
    }

    fn trigger_restart(self: Arc<Self>) {
        tokio::spawn(async move {
            self.restart_loop().await;
        });
    }

    async fn restart_loop(self: Arc<Self>) {
        loop {
            let backoff = {
                let mut rs = self.restart_state.lock().unwrap();
                if rs.permanent_reason.is_some() {
                    return;
                }
                let cutoff = Instant::now() - self.cfg.restart_budget_window;
                while rs.failures.front().is_some_and(|t| *t < cutoff) {
                    rs.failures.pop_front();
                }
                if rs.failures.len() >= self.cfg.restart_budget_max_failures {
                    rs.permanent_reason = Some("restart budget exceeded (3x/5min)".to_string());
                    tracing::error!("worker.restart_budget_exceeded");
                    return;
                }
                rs.backoff
            };

            tokio::time::sleep(backoff).await;

            match self.establish_connection().await {
                Ok(()) => {
                    let mut rs = self.restart_state.lock().unwrap();
                    // Backoff itself resets only after `restart_stable_after`
                    // of uptime (stable_reset_task); a fresh connection does
                    // not immediately clear it, matching LLD-02 §5.4.
                    let _ = &mut rs;
                    return;
                }
                Err(outcome) => {
                    let mut rs = self.restart_state.lock().unwrap();
                    if let EstablishOutcome::Permanent(msg) = &outcome {
                        rs.permanent_reason = Some(msg.clone());
                        tracing::error!(error = %msg, "worker.restart_permanent");
                        return;
                    }
                    rs.failures.push_back(Instant::now());
                    rs.backoff = (rs.backoff * 2).min(self.cfg.restart_backoff_cap);
                    tracing::warn!(error = %outcome.into_message(), "worker.restart_failed");
                }
            }
        }
    }

    /// Idempotent: only the first caller after a live connection actually
    /// tears anything down.
    fn handle_disconnect(self: &Arc<Self>, reason: &'static str) {
        let old = self.transport.lock().unwrap().take();
        let Some(handle) = old else {
            return;
        };
        tracing::warn!(reason, "worker.disconnected");
        self.registry.lock().unwrap().clear();

        let sup = Arc::clone(self);
        tokio::spawn(async move {
            // Best-effort: the process may still be alive but wedged (the
            // stuck-health-check path), so make sure it's actually gone
            // before restarting.
            let mut child = handle.child.lock().await;
            let _ = child.start_kill();
            drop(child);
            sup.restart_loop().await;
        });
    }
}

enum EstablishOutcome {
    Permanent(String),
    Transient(String),
}

impl EstablishOutcome {
    fn into_message(self) -> String {
        match self {
            EstablishOutcome::Permanent(m) | EstablishOutcome::Transient(m) => m,
        }
    }
}

fn map_json_rpc_error(err: JsonRpcErrorObj) -> AppError {
    // Codes per LLD-02 §4.4 / BACKEND §2.
    match err.code {
        -32001 => AppError::WorkerUnavailable { retry_after_ms: 0 },
        -32020 => AppError::Cancelled,
        -32010 => AppError::Validation {
            message: err.message,
            field: None,
        },
        // LLD-05 §4.5 — the worker's `extract_memory`/`refresh_project_memory`
        // handlers raise this (via `WorkerJobError`) after the agent's JSON
        // still fails schema validation on the one allowed retry.
        -32022 => AppError::Runner {
            runner: "claude".to_string(),
            message: err.message,
            correlation_id: crate::error::correlation_id(),
        },
        // Provider-side usage limit. The message was authored for the user
        // in `translate.rs` and passed through the worker unwrapped, so it
        // is used verbatim here — this variant's `Display` is what lands in
        // `pipeline_state.error` and renders in the failure banner.
        -32023 => AppError::RunnerBlocked {
            runner: "claude".to_string(),
            resets_at: err
                .data
                .as_ref()
                .and_then(|d| d.get("resets_at"))
                .and_then(serde_json::Value::as_i64),
            message: err.message,
            correlation_id: crate::error::correlation_id(),
        },
        _ => AppError::Internal {
            message: err.message,
            correlation_id: crate::error::correlation_id(),
        },
    }
}

// ---------------------------------------------------------------------
// Background tasks
// ---------------------------------------------------------------------

async fn writer_task(mut stdin: tokio::process::ChildStdin, mut rx: mpsc::Receiver<Vec<u8>>) {
    while let Some(frame) = rx.recv().await {
        if let Err(e) = stdin.write_all(&frame).await {
            tracing::warn!(error = %e, "worker.writer.write_failed");
            return;
        }
        if stdin.flush().await.is_err() {
            return;
        }
    }
}

async fn reader_task(
    sup: Arc<WorkerSupervisor>,
    mut stdout: BufReader<tokio::process::ChildStdout>,
) {
    loop {
        let body = match read_frame(&mut stdout).await {
            Ok(Some(b)) => b,
            Ok(None) => {
                sup.handle_disconnect("stdout EOF");
                return;
            }
            Err(e) => {
                tracing::warn!(error = %e, "worker.reader.framing_error");
                sup.handle_disconnect("framing error");
                return;
            }
        };
        let parsed: Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "worker.reader.bad_json");
                continue;
            }
        };
        dispatch_incoming(&sup, parsed).await;
    }
}

async fn dispatch_incoming(sup: &Arc<WorkerSupervisor>, msg: Value) {
    let has_id = msg.get("id").is_some();
    let method = msg
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string);

    match (has_id, method) {
        (true, Some(method)) => {
            // Server-to-client request (reverse RPC, §7).
            let id = msg.get("id").cloned().unwrap_or(Value::Null);
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            let writer_tx = sup
                .transport
                .lock()
                .unwrap()
                .as_ref()
                .map(|t| t.writer_tx.clone());
            if let Some(writer_tx) = writer_tx {
                dispatch_reverse_rpc(Arc::clone(sup), id, method, params, writer_tx);
            }
        }
        (true, None) => {
            // Response to a forward request.
            let Some(id_str) = msg.get("id").and_then(Value::as_str) else {
                return;
            };
            let Ok(id) = id_str.parse::<u64>() else {
                return;
            };
            let entry = sup.registry.lock().unwrap().remove(&id);
            let Some(entry) = entry else {
                return; // Late/unknown reply — replayed job or dropped caller.
            };
            if let Some(err) = msg.get("error") {
                let obj = JsonRpcErrorObj {
                    code: err.get("code").and_then(Value::as_i64).unwrap_or(-32000),
                    message: err
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("worker error")
                        .to_string(),
                    data: err.get("data").cloned(),
                };
                let _ = entry.tx.send(Err(obj));
            } else {
                let _ = entry
                    .tx
                    .send(Ok(msg.get("result").cloned().unwrap_or(Value::Null)));
            }
        }
        (false, Some(method)) => {
            // Notification.
            if method == "heartbeat" {
                *sup.last_heartbeat.lock().unwrap() = Instant::now();
                return;
            }
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            let topics = sup.notify_topics.lock().unwrap();
            if let Some(tx) = topics.get(&method) {
                let _ = tx.send(params);
            }
        }
        (false, None) => {}
    }
}

/// Free function (not a `WorkerSupervisor` method) so it can be unit-tested
/// against a synthetic `writer_tx` without a real child process — see the
/// `dispatch_reverse_rpc_*` tests below.
fn dispatch_reverse_rpc(
    sup: Arc<WorkerSupervisor>,
    id: Value,
    method: String,
    params: Value,
    writer_tx: mpsc::Sender<Vec<u8>>,
) {
    tokio::spawn(async move {
        let handler = sup
            .reverse_handlers
            .lock()
            .unwrap()
            .get(method.as_str())
            .cloned();
        let response = match handler {
            None => JsonRpcResponseOut::error(id.clone(), -32601, "method not found".to_string()),
            Some(h) => {
                // `tokio::spawn` isolates a handler panic into a `JoinError`
                // instead of taking the reader task down with it — the same
                // guarantee LLD-02 §7.1 asks `catch_unwind` for.
                let task = tokio::spawn(async move { h.handle(params).await });
                match task.await {
                    Ok(Ok(value)) => JsonRpcResponseOut::result(id.clone(), value),
                    Ok(Err(e)) => JsonRpcResponseOut::error(id.clone(), e.code, e.message),
                    Err(join_err) => JsonRpcResponseOut::error(
                        id.clone(),
                        -32000,
                        format!("handler panicked: {join_err}"),
                    ),
                }
            }
        };
        if let Ok(frame) = build_frame(&response) {
            let _ = writer_tx.send(frame).await;
        }
    });
}

async fn stderr_task(stderr: tokio::process::ChildStderr) {
    use tokio::io::AsyncBufReadExt;
    let mut lines = tokio::io::BufReader::new(stderr).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => forward_stderr_line(&line),
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(error = %e, "worker.stderr.read_error");
                return;
            }
        }
    }
}

fn forward_stderr_line(line: &str) {
    match serde_json::from_str::<Value>(line) {
        Ok(v) => {
            let level = v.get("level").and_then(Value::as_str).unwrap_or("info");
            let event = v.get("event").and_then(Value::as_str).unwrap_or("");
            match level {
                "error" | "critical" => {
                    tracing::error!(component = "python-worker", event, raw = line)
                }
                "warning" | "warn" => {
                    tracing::warn!(component = "python-worker", event, raw = line)
                }
                "debug" => tracing::debug!(component = "python-worker", event, raw = line),
                _ => tracing::info!(component = "python-worker", event, raw = line),
            }
        }
        Err(_) => tracing::warn!(
            component = "python-worker",
            event = "worker.stderr.malformed_line",
            raw = line
        ),
    }
}

async fn ttl_sweep_task(sup: Arc<WorkerSupervisor>) {
    let interval = sup.cfg.ttl_sweep_interval;
    loop {
        tokio::time::sleep(interval).await;
        if sup.transport.lock().unwrap().is_none() {
            return; // superseded by a restart's own fresh sweep task
        }
        let now = Instant::now();
        let expired: Vec<u64> = {
            let reg = sup.registry.lock().unwrap();
            reg.iter()
                .filter(|(_, e)| e.deadline < now)
                .map(|(id, _)| *id)
                .collect()
        };
        if expired.is_empty() {
            continue;
        }
        let mut reg = sup.registry.lock().unwrap();
        for id in expired {
            reg.remove(&id); // dropping the sender fails the caller's `rx.await`
        }
    }
}

async fn health_task(sup: Arc<WorkerSupervisor>) {
    let mut ticker = tokio::time::interval(sup.cfg.health_check_interval);
    ticker.tick().await; // first tick fires immediately; skip it
    loop {
        ticker.tick().await;
        if sup.transport.lock().unwrap().is_none() {
            return;
        }
        let since_heartbeat = sup.last_heartbeat.lock().unwrap().elapsed();
        if since_heartbeat > sup.cfg.heartbeat_timeout {
            sup.handle_disconnect("no heartbeat");
            return;
        }
        if sup.send(HealthCheck {}).await.is_err() {
            sup.handle_disconnect("health_check failed");
            return;
        }
    }
}

async fn stable_reset_task(sup: Arc<WorkerSupervisor>) {
    tokio::time::sleep(sup.cfg.restart_stable_after).await;
    if sup.transport.lock().unwrap().is_none() {
        return; // died before becoming stable — the new restart loop owns backoff now
    }
    let mut rs = sup.restart_state.lock().unwrap();
    rs.backoff = sup.cfg.restart_backoff_base;
    rs.failures.clear();
}

// ---------------------------------------------------------------------
// Wire helpers
// ---------------------------------------------------------------------

#[derive(Serialize)]
struct RequestFrame<'a, T: Serialize> {
    jsonrpc: &'a str,
    id: String,
    method: &'a str,
    params: &'a T,
}

#[derive(Serialize)]
struct NotificationFrame<'a, T: Serialize> {
    jsonrpc: &'a str,
    method: &'a str,
    params: &'a T,
}

#[derive(Serialize)]
struct JsonRpcResponseOut {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcErrorOut>,
}

#[derive(Serialize)]
struct JsonRpcErrorOut {
    code: i32,
    message: String,
}

impl JsonRpcResponseOut {
    fn result(id: Value, value: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(value),
            error: None,
        }
    }
    fn error(id: Value, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcErrorOut { code, message }),
        }
    }
}

fn build_frame<T: Serialize>(payload: &T) -> Result<Vec<u8>, AppError> {
    let body = serde_json::to_vec(payload)
        .map_err(|e| AppError::internal(format!("failed to serialize JSON-RPC frame: {e}")))?;
    Ok(crate::ipc::framing::encode_frame(&body))
}

#[derive(Serialize, Deserialize)]
struct WorkerManifest {
    pid: u32,
    spawn_at: String,
    protocol_version: u32,
}

fn chrono_now_iso() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    // No `chrono`/`time` dependency in this crate yet — Unix-seconds is
    // precise enough for a diagnostics-only manifest field.
    format!("unix:{}", now.as_secs())
}

async fn adopt_or_kill_stale_manifest(cfg: &SupervisorConfig) {
    let path = cfg.state_dir.join("worker-manifest.json");
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    let Ok(manifest) = serde_json::from_slice::<WorkerManifest>(&bytes) else {
        return;
    };
    tracing::warn!(pid = manifest.pid, "worker.manifest.stale_found");
    kill_pid_best_effort(manifest.pid).await;
}

#[cfg(unix)]
async fn kill_pid_best_effort(pid: u32) {
    let _ = tokio::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .output()
        .await;
}

#[cfg(windows)]
async fn kill_pid_best_effort(pid: u32) {
    let _ = tokio::process::Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .output()
        .await;
}

struct ReplayJob {
    id: String,
    kind: String,
    params: Value,
}

fn read_state_file_entry(path: &std::path::Path) -> Option<ReplayJob> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(v) => value_to_replay_job(v),
        Err(e) => {
            tracing::error!(path = %path.display(), error = %e, "worker.current_job.corrupt");
            move_aside(path);
            None
        }
    }
}

fn read_state_file_jobs(path: &std::path::Path) -> Option<Vec<ReplayJob>> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(v) => v.get("jobs").and_then(Value::as_array).map(|arr| {
            arr.iter()
                .filter_map(|j| value_to_replay_job(j.clone()))
                .collect()
        }),
        Err(e) => {
            tracing::error!(path = %path.display(), error = %e, "worker.pending_jobs.corrupt");
            move_aside(path);
            None
        }
    }
}

fn value_to_replay_job(v: Value) -> Option<ReplayJob> {
    let id = v.get("id")?.as_str()?.to_string();
    let kind = v.get("kind")?.as_str()?.to_string();
    let params = v.get("params").cloned().unwrap_or(Value::Null);
    Some(ReplayJob { id, kind, params })
}

fn move_aside(path: &std::path::Path) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut dest = path.as_os_str().to_os_string();
    dest.push(format!(".corrupt-{ts}"));
    let _ = std::fs::rename(path, dest);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bare_supervisor() -> Arc<WorkerSupervisor> {
        Arc::new(WorkerSupervisor {
            cfg: SupervisorConfig::new(
                PathBuf::from("python3"),
                PathBuf::from("."),
                PathBuf::from("."),
            ),
            next_id: AtomicU64::new(1),
            registry: StdMutex::new(HashMap::new()),
            transport: StdMutex::new(None),
            reverse_handlers: StdMutex::new(HashMap::new()),
            notify_topics: StdMutex::new(HashMap::new()),
            last_heartbeat: StdMutex::new(Instant::now()),
            restart_state: StdMutex::new(RestartState {
                backoff: Duration::from_secs(1),
                ..Default::default()
            }),
            #[cfg(target_os = "macos")]
            live_sidecar: StdMutex::new(None),
        })
    }

    struct EchoHandler;
    #[async_trait::async_trait]
    impl ReverseRpcHandler for EchoHandler {
        async fn handle(&self, params: Value) -> Result<Value, ReverseRpcError> {
            Ok(serde_json::json!({ "echo": params }))
        }
    }

    struct PanicHandler;
    #[async_trait::async_trait]
    impl ReverseRpcHandler for PanicHandler {
        async fn handle(&self, _params: Value) -> Result<Value, ReverseRpcError> {
            panic!("dummy handler panics on purpose");
        }
    }

    async fn recv_response_frame(rx: &mut mpsc::Receiver<Vec<u8>>) -> Value {
        let frame = rx.recv().await.expect("a response frame must be written");
        let mut reader = tokio::io::BufReader::new(&frame[..]);
        let body = crate::ipc::framing::read_frame(&mut reader)
            .await
            .expect("valid framing")
            .expect("a body, not EOF");
        serde_json::from_slice(&body).expect("valid JSON body")
    }

    #[tokio::test]
    async fn reverse_rpc_dispatches_to_a_registered_handler() {
        let sup = bare_supervisor();
        sup.register_reverse_rpc("dummy.echo", Arc::new(EchoHandler));
        let (tx, mut rx) = mpsc::channel(4);

        dispatch_reverse_rpc(
            Arc::clone(&sup),
            Value::String("1".into()),
            "dummy.echo".to_string(),
            serde_json::json!({"x": 1}),
            tx,
        );

        let resp = recv_response_frame(&mut rx).await;
        assert_eq!(resp["id"], "1");
        assert_eq!(resp["result"]["echo"]["x"], 1);
    }

    #[tokio::test]
    async fn reverse_rpc_unknown_method_is_dash32601() {
        let sup = bare_supervisor();
        let (tx, mut rx) = mpsc::channel(4);

        dispatch_reverse_rpc(
            Arc::clone(&sup),
            Value::String("2".into()),
            "nope".to_string(),
            Value::Null,
            tx,
        );

        let resp = recv_response_frame(&mut rx).await;
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn reverse_rpc_handler_panic_becomes_dash32000_not_a_crash() {
        let sup = bare_supervisor();
        sup.register_reverse_rpc("dummy.panic", Arc::new(PanicHandler));
        let (tx, mut rx) = mpsc::channel(4);

        dispatch_reverse_rpc(
            Arc::clone(&sup),
            Value::String("3".into()),
            "dummy.panic".to_string(),
            Value::Null,
            tx,
        );

        let resp = recv_response_frame(&mut rx).await;
        assert_eq!(resp["error"]["code"], -32000);
    }

    #[tokio::test]
    async fn live_transcript_chunk_notification_round_trips_through_subscribe() {
        // Exercises the path a future RecordingService reader task (W9)
        // will use: `sup.subscribe(LIVE_TRANSCRIPT_CHUNK_TOPIC)` then parse
        // each `Value` into `LiveTranscriptChunkNotification` — proves the
        // wire shape this wave's Python `live_transcript_chunk` notifier
        // emits (LLD-03 §5.1's `_tick`) is exactly what Rust expects.
        let sup = bare_supervisor();
        let mut rx = sup.subscribe(LIVE_TRANSCRIPT_CHUNK_TOPIC);

        let sender = sup
            .notify_topics
            .lock()
            .unwrap()
            .get(LIVE_TRANSCRIPT_CHUNK_TOPIC)
            .unwrap()
            .clone();
        sender
            .send(serde_json::json!({
                "conversation_id": "conv-1",
                "chunk": {
                    "speaker_label_hint": null,
                    "text": "hello there",
                    "ts_start_ms": 1500,
                    "ts_end_ms": 2200
                }
            }))
            .unwrap();

        let value = rx.recv().await.unwrap();
        let parsed: LiveTranscriptChunkNotification = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.conversation_id, "conv-1");
        assert_eq!(parsed.chunk.text, "hello there");
        assert_eq!(parsed.chunk.speaker_label_hint, None);
        assert_eq!(parsed.chunk.ts_start_ms, 1500);
        assert_eq!(parsed.chunk.ts_end_ms, 2200);
    }

    #[test]
    fn transcribe_final_request_serializes_with_snake_case_paths() {
        let req = TranscribeFinal {
            conversation_id: "conv-1".to_string(),
            mic_path: PathBuf::from("/tmp/mic.wav"),
            system_path: PathBuf::from("/tmp/system.wav"),
        };
        let value = serde_json::to_value(&req).unwrap();
        assert_eq!(value["conversation_id"], "conv-1");
        assert_eq!(value["mic_path"], "/tmp/mic.wav");
        assert_eq!(value["system_path"], "/tmp/system.wav");
        assert_eq!(TranscribeFinal::METHOD, "transcribe_final");
    }

    #[test]
    fn backoff_formula_doubles_and_caps() {
        let cfg = SupervisorConfig::new(
            PathBuf::from("python3"),
            PathBuf::from("."),
            PathBuf::from("."),
        );
        let mut backoff = cfg.restart_backoff_base;
        let mut seen = vec![backoff];
        for _ in 0..6 {
            backoff = (backoff * 2).min(cfg.restart_backoff_cap);
            seen.push(backoff);
        }
        assert_eq!(
            seen,
            vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(8),
                Duration::from_secs(16),
                Duration::from_secs(16),
                Duration::from_secs(16),
            ]
        );
    }

    #[test]
    fn restart_budget_trips_after_three_failures_in_window() {
        let sup = bare_supervisor();
        let now = Instant::now();
        {
            let mut rs = sup.restart_state.lock().unwrap();
            rs.failures.push_back(now);
            rs.failures.push_back(now);
            rs.failures.push_back(now);
        }
        let mut rs = sup.restart_state.lock().unwrap();
        let cutoff = Instant::now() - sup.cfg.restart_budget_window;
        while rs.failures.front().is_some_and(|t| *t < cutoff) {
            rs.failures.pop_front();
        }
        assert!(rs.failures.len() >= sup.cfg.restart_budget_max_failures);
    }

    #[tokio::test]
    async fn ttl_sweep_fails_an_expired_registry_entry() {
        let sup = bare_supervisor();
        let (tx, rx) = oneshot::channel();
        sup.registry.lock().unwrap().insert(
            42,
            PendingEntry {
                tx,
                method: "ping",
                deadline: Instant::now() - Duration::from_millis(1),
            },
        );

        let expired: Vec<u64> = {
            let now = Instant::now();
            let reg = sup.registry.lock().unwrap();
            reg.iter()
                .filter(|(_, e)| e.deadline < now)
                .map(|(id, _)| *id)
                .collect()
        };
        {
            let mut reg = sup.registry.lock().unwrap();
            for id in expired {
                reg.remove(&id);
            }
        }

        // Dropping the sender makes the receiver resolve to an error, which
        // `send()` maps to `WorkerUnavailable` — no leaked/hanging caller.
        assert!(rx.await.is_err());
    }
}
