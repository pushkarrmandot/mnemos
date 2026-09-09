//! Integration tests for `WorkerSupervisor` — spawns the real
//! `python -m mnemos_worker` process (not a stub) since the Python
//! side already speaks the full protocol (handshake/ping/health_check/
//! shutdown + job-queue park/replay). Backoff/health/TTL timings are
//! shortened via `SupervisorConfig` so the suite runs in well under a
//! minute.

use std::path::PathBuf;
use std::time::Duration;

use mnemos_tauri_lib::commands::models::list_transcription_models;
use mnemos_tauri_lib::ipc::python::{
    HealthCheck, ModelDownloadStatus, Ping, SupervisorConfig, WorkerSupervisor,
};

fn python_bin() -> PathBuf {
    PathBuf::from(if cfg!(windows) { "python" } else { "python3" })
}

fn src_python_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src-python")
}

fn fast_config(state_dir: PathBuf) -> SupervisorConfig {
    let mut cfg = SupervisorConfig::new(python_bin(), src_python_dir(), state_dir);
    cfg.handshake_timeout = Duration::from_secs(10);
    cfg.health_check_interval = Duration::from_millis(200);
    cfg.health_check_timeout = Duration::from_millis(500);
    cfg.heartbeat_timeout = Duration::from_secs(3600); // not exercised by these tests
    cfg.ttl_sweep_interval = Duration::from_millis(100);
    cfg.restart_backoff_base = Duration::from_millis(50);
    cfg.restart_backoff_cap = Duration::from_millis(200);
    cfg.restart_stable_after = Duration::from_secs(3600); // not exercised by these tests
    cfg.shutdown_grace = Duration::from_secs(5);
    cfg
}

async fn wait_until<F: Fn() -> bool>(timeout: Duration, poll: Duration, cond: F) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if cond() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(poll).await;
    }
}

fn manifest_pid(state_dir: &std::path::Path) -> Option<u32> {
    let bytes = std::fs::read(state_dir.join("worker-manifest.json")).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("pid")?.as_u64().map(|p| p as u32)
}

#[tokio::test]
async fn spawn_handshakes_and_responds_to_ping() {
    let tmp = tempfile::tempdir().unwrap();
    let sup = WorkerSupervisor::spawn(fast_config(tmp.path().to_path_buf()))
        .await
        .expect("spawn always returns Ok");

    let reply = sup
        .send(Ping::default())
        .await
        .expect("ping should succeed");
    assert!(reply.pong);

    let health = sup
        .send(HealthCheck {})
        .await
        .expect("health_check should succeed");
    assert!(health.ok);

    sup.shutdown().await.expect("graceful shutdown");
}

#[tokio::test]
async fn kill_mid_request_fails_pending_then_restarts_and_replays() {
    let tmp = tempfile::tempdir().unwrap();
    let state_dir = tmp.path().to_path_buf();
    let sup = WorkerSupervisor::spawn(fast_config(state_dir.clone()))
        .await
        .expect("spawn always returns Ok");

    // Prove the worker is up before we start.
    sup.send(Ping::default()).await.expect("initial ping");

    // Fire a slow ping so it's "current" server-side (current_job.json
    // written) when we kill the process out from under it.
    let sup_clone = sup.clone();
    let slow = tokio::spawn(async move {
        sup_clone
            .send(Ping {
                delay_ms: Some(3000),
                job_id: None,
            })
            .await
    });

    // Wait for the worker to actually start executing the job (visible on
    // disk), then kill -9 the OS process directly — a hard crash mid-job.
    let current_job_path = state_dir.join("current_job.json");
    assert!(
        wait_until(Duration::from_secs(3), Duration::from_millis(20), || {
            current_job_path.exists()
        })
        .await,
        "worker never parked the slow job into current_job.json"
    );
    let pid = manifest_pid(&state_dir).expect("manifest must record a pid");
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status()
            .expect("kill -9 must run");
    }

    // The in-flight request must resolve to WorkerUnavailable, not hang.
    let outcome = slow.await.expect("join");
    assert!(
        matches!(
            outcome,
            Err(mnemos_tauri_lib::error::AppError::WorkerUnavailable { .. })
        ),
        "expected WorkerUnavailable, got {outcome:?}"
    );

    // current_job.json survives the kill (the old process never got to
    // delete it) — this is exactly what §6 replay reads.
    assert!(current_job_path.exists());

    // Restart with backoff, then replay: the new worker process picks the
    // parked job back up and finishes it, clearing current_job.json.
    assert!(
        wait_until(Duration::from_secs(10), Duration::from_millis(50), || {
            !current_job_path.exists()
        })
        .await,
        "current_job.json was never cleared after restart+replay"
    );

    // And the supervisor is healthy again on the new process.
    let reply = sup
        .send(Ping::default())
        .await
        .expect("worker must be usable again after restart");
    assert!(reply.pong);

    sup.shutdown().await.expect("graceful shutdown");
}

/// The registry in `commands::models` (Rust) and `PARAKEET_MODEL_ID`
/// (`src-python/mnemos_worker/models/transcription.py`) are two constants in
/// two processes describing the same real model — they can't be a single
/// shared value across the language boundary, so nothing stops them from
/// silently drifting apart except a test that asks the real running worker
/// what its model id actually is. `ModelDownloadStatus` is a synchronous,
/// no-network status read (`_DownloadProgress.snapshot()`) — it never
/// triggers the real ~600MB download, so this is safe and fast to run on
/// every `cargo test`, unlike the model itself ever being exercised for
/// real (that's what the `MNEMOS_LIVE_RECORDING`-gated tests are for).
#[tokio::test]
async fn worker_reported_model_id_matches_the_rust_side_registry() {
    let tmp = tempfile::tempdir().unwrap();
    let sup = WorkerSupervisor::spawn(fast_config(tmp.path().to_path_buf()))
        .await
        .expect("spawn always returns Ok");

    let status = sup
        .send(ModelDownloadStatus {})
        .await
        .expect("model_download_status should succeed");

    let known_ids: Vec<String> = list_transcription_models()
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert!(
        known_ids.contains(&status.model_id),
        "worker reported model id {:?}, not one of the Rust-side registry's ids {known_ids:?}",
        status.model_id,
    );

    sup.shutdown().await.expect("graceful shutdown");
}
