//! Integration tests for `memory::extract_conversation` /
//! `memory::refresh_project` against the **real** `python -m mnemos_worker`
//! process (same convention as `tests/worker_supervisor_integration.rs`),
//! with a scripted fake registered for `run_agent_extraction` (swapped out
//! between scenarios via `register_reverse_rpc`, on one shared worker
//! process) so the suite never needs the real `claude` CLI on PATH — the
//! Python job handlers (schema validation, retry policy, diff guardrail)
//! are exercised for real; only the agent turn itself is canned, matching
//! the "FakeHarnessAdapter... exercises the full flow" strategy,
//! just realized as a fake reverse-RPC handler at the process boundary
//! instead of a fake `HarnessAdapter`.
//!
//! `HOME` is overridden for this binary's one test function so `fs::paths`
//! resolves under a tempdir — same one-test-per-file convention
//! `tests/storage_integration.rs` uses, for the same reason (no risk of two
//! `#[tokio::test]`s racing on that process-wide env var).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::Mutex as AsyncMutex;

use mnemos_tauri_lib::db;
use mnemos_tauri_lib::db::models::{NewConversation, NewProject};
use mnemos_tauri_lib::db::service::{SqliteStorageService, StorageService};
use mnemos_tauri_lib::fs::{atomic, paths};
use mnemos_tauri_lib::ipc::python::{
    HealthCheck, ReverseRpcError, ReverseRpcHandler, SupervisorConfig, WorkerSupervisor,
};
use mnemos_tauri_lib::memory;

fn python_bin() -> PathBuf {
    PathBuf::from(if cfg!(windows) { "python" } else { "python3" })
}

fn src_python_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src-python")
}

fn fast_config(state_dir: PathBuf) -> SupervisorConfig {
    let mut cfg = SupervisorConfig::new(python_bin(), src_python_dir(), state_dir);
    cfg.handshake_timeout = Duration::from_secs(45);
    cfg.health_check_interval = Duration::from_secs(3600);
    cfg.heartbeat_timeout = Duration::from_secs(3600);
    cfg.restart_stable_after = Duration::from_secs(3600);
    cfg.shutdown_grace = Duration::from_secs(5);
    cfg
}

/// Scripted `run_agent_extraction` fake (the FakeHarnessAdapter approach,
/// realized as a reverse-RPC handler). Each call pops the next canned
/// response and records the params it was invoked with, so a test can
/// assert on what the worker's prompt actually contained.
struct ScriptedAgent {
    responses: AsyncMutex<VecDeque<Value>>,
    calls: AsyncMutex<Vec<Value>>,
}

impl ScriptedAgent {
    fn new(responses: Vec<Value>) -> Arc<Self> {
        Arc::new(Self {
            responses: AsyncMutex::new(responses.into()),
            calls: AsyncMutex::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl ReverseRpcHandler for ScriptedAgent {
    async fn handle(&self, params: Value) -> Result<Value, ReverseRpcError> {
        self.calls.lock().await.push(params);
        self.responses
            .lock()
            .await
            .pop_front()
            .ok_or_else(|| ReverseRpcError {
                code: -32000,
                message: "ScriptedAgent: no more canned responses".to_string(),
                data: None,
            })
    }
}

/// `spawn()` always returns `Ok`, but a failed first connection attempt
/// just schedules a background
/// reconnect on its own backoff rather than blocking `spawn()`
/// until it succeeds — so a caller that needs the worker up before
/// proceeding (every test here) has to wait for that reconnect itself, the
/// same way `tests/worker_supervisor_integration.rs` does.
async fn spawn_worker_and_wait_ready(state_dir: PathBuf) -> Arc<WorkerSupervisor> {
    let sup = WorkerSupervisor::spawn(fast_config(state_dir))
        .await
        .expect("spawn always returns Ok");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    loop {
        if sup.send(HealthCheck {}).await.is_ok() {
            return sup;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("worker never became ready within 90s");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn fresh_service(db_path: &std::path::Path) -> SqliteStorageService {
    let pools = db::init(db_path).await.expect("db init");
    SqliteStorageService::new(pools)
}

async fn row_count(svc: &SqliteStorageService, table: &str, conv_id: &str) -> i64 {
    let (n,): (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {table} WHERE conv_id = ?1"))
        .bind(conv_id)
        .fetch_one(&svc.pools.read)
        .await
        .unwrap();
    n
}

fn seed_transcript(conv_id: &str) {
    let path = paths::transcript_json_path(conv_id).unwrap();
    atomic::atomic_write_json(
        &path,
        &json!({
            "schema_version": 1,
            "conversation_id": conv_id,
            "duration_ms": 60_000,
            "turns": [
                {"text": "Let's ship the auth spec today.", "speaker_label": "You", "ts_start_ms": 0, "ts_end_ms": 2000, "source": "mic", "speaker_label_source": "source_file", "contact_id": null},
                {"text": "Sounds good, I'll review it.", "speaker_label": "Them", "ts_start_ms": 4000, "ts_end_ms": 6000, "source": "system", "speaker_label_source": "source_file", "contact_id": null},
            ],
        }),
    )
    .unwrap();
}

fn canned_extraction(action_item_text: &str) -> Value {
    json!({
        "title": "Auth Spec Handoff",
        "summary_markdown": "# Overview\nShipping the auth spec.",
        "action_items": [{"text": action_item_text, "assignee_hint": "David", "due_hint": "today", "source_timestamp_ms": 0}],
        "decisions": [{"statement": "OAuth for v1", "decided_by_hint": "David", "quote": null, "source_timestamp_ms": 0}],
        "open_questions": [{"question": "Support Google Workspace SSO?", "raised_by_hint": "Craig", "source_timestamp_ms": 0}],
        "bookmarks": [],
    })
}

#[tokio::test]
async fn memory_system_end_to_end() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("mnemos_tauri_lib=debug")
        .try_init();
    let home = tempfile::tempdir().unwrap();
    // Keep the real HF cache reachable under the overridden `HOME` below —
    // otherwise `ParakeetModel.warm_up()` (unrelated to this suite, but run
    // unconditionally at worker startup) redownloads its ~600MB model on
    // every test run instead of reusing what's already cached.
    if let Some(real_home) = std::env::var_os("HOME") {
        std::env::set_var(
            "HF_HOME",
            std::path::Path::new(&real_home).join(".cache/huggingface"),
        );
    }
    // Force cache-only (no HTTP HEAD/redirect calls at all) — the freshness
    // check alone is enough to flake under HF's unauthenticated rate limit
    // when the suite (or a developer's repeated local runs) spawns the
    // worker more than a couple of times in quick succession.
    std::env::set_var("HF_HUB_OFFLINE", "1");
    // The Xet storage backend's read-token refresh has been observed to
    // phone home even under `HF_HUB_OFFLINE=1` — disable it outright.
    std::env::set_var("HF_HUB_DISABLE_XET", "1");
    std::env::set_var("HOME", home.path());
    let db_path = home.path().join("mnemos-test.db");
    let state_dir = home.path().join("state");

    // One worker process for the whole suite (`register_reverse_rpc`
    // overwrites the handler between scenarios) — spawning it fresh per
    // scenario worked but multiplied `ParakeetModel.warm_up()`'s network
    // round-trips (unrelated to this suite, run unconditionally at worker
    // startup) enough to flake under parallel `cargo test` load.
    let worker = spawn_worker_and_wait_ready(state_dir).await;

    let svc = fresh_service(&db_path).await;
    let project = svc
        .create_project(NewProject {
            name: "Acme".into(),
            description: None,
        })
        .await
        .unwrap();
    let conv = svc
        .insert_conversation(NewConversation {
            project_id: Some(project.id.clone()),
            title: "Kickoff".into(),
            started_at: 0,
            runner_id: None,
        })
        .await
        .unwrap();
    seed_transcript(&conv.id);

    // ---- extraction_happy_path -----------------------------------------
    let agent = ScriptedAgent::new(vec![canned_extraction("Send David the auth spec")]);
    worker.register_reverse_rpc("run_agent_extraction", agent);

    let outcome = memory::extract_conversation(&svc, &worker, &conv.id, false)
        .await
        .expect("extraction should succeed");
    assert_eq!(outcome.action_items, 1);
    assert_eq!(outcome.decisions, 1);
    assert_eq!(outcome.open_questions, 1);
    assert!(outcome.summary_written);

    let extraction_path = paths::extraction_json_path(&conv.id).unwrap();
    assert!(extraction_path.exists());
    let extraction_json: Value =
        serde_json::from_slice(&std::fs::read(&extraction_path).unwrap()).unwrap();
    assert_eq!(
        extraction_json["summary_markdown"],
        "# Overview\nShipping the auth spec."
    );

    let summary_path = paths::summary_md_path(&conv.id).unwrap();
    assert!(std::fs::read_to_string(&summary_path)
        .unwrap()
        .starts_with("# Overview"));

    assert_eq!(row_count(&svc, "action_items", &conv.id).await, 1);
    assert_eq!(row_count(&svc, "decisions", &conv.id).await, 1);
    assert_eq!(row_count(&svc, "open_questions", &conv.id).await, 1);

    // ---- extraction_replaces_prior_but_preserves_manual ------------------
    // Seed two manually-added action items directly (as a user "Add
    // action item" flow would).
    for text in ["Manual item A", "Manual item B"] {
        sqlx::query(
            "INSERT INTO action_items (id, conv_id, text, done, added_manually, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 0, 1, 0, 0)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&conv.id)
        .bind(text)
        .execute(&svc.pools.write)
        .await
        .unwrap();
    }
    assert_eq!(row_count(&svc, "action_items", &conv.id).await, 3); // 1 auto + 2 manual

    let agent2 = ScriptedAgent::new(vec![canned_extraction("Send David the updated spec")]);
    worker.register_reverse_rpc("run_agent_extraction", agent2);
    let outcome2 = memory::extract_conversation(&svc, &worker, &conv.id, true)
        .await
        .expect("re-extraction should succeed");
    assert_eq!(outcome2.action_items, 1); // the new auto row only
    assert_eq!(row_count(&svc, "action_items", &conv.id).await, 3); // 1 new auto + 2 manual survive

    let (manual_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM action_items WHERE conv_id = ?1 AND added_manually = 1",
    )
    .bind(&conv.id)
    .fetch_one(&svc.pools.read)
    .await
    .unwrap();
    assert_eq!(manual_count, 2);

    let (stale_auto,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM action_items WHERE conv_id = ?1 AND text = 'Send David the auth spec'",
    )
    .bind(&conv.id)
    .fetch_one(&svc.pools.read)
    .await
    .unwrap();
    assert_eq!(
        stale_auto, 0,
        "the old auto-inserted row must be gone, not duplicated"
    );

    // ---- refresh_snapshot_created_before_overwrite + preserves-user-text -
    let manual_line = "MANUALLY EDITED: pricing target is now €22.";
    svc.write_project_memory(
        &project.id,
        &json!({
            "overview_markdown": format!("Kickoff notes.\n{manual_line}"),
            "scope_drift_markdown": "Initial scope.",
        }),
    )
    .await
    .unwrap();
    let pre_refresh_content: Value = serde_json::from_slice(
        &std::fs::read(paths::project_memory_path(&project.id).unwrap()).unwrap(),
    )
    .unwrap();

    let refresh_agent = ScriptedAgent::new(vec![json!({
        "overview_markdown": format!("Kickoff notes, now with a second conversation.\n{manual_line}"),
        "scope_drift_markdown": "Initial scope.",
        "supersessions": [],
    })]);
    worker.register_reverse_rpc("run_agent_extraction", refresh_agent.clone());
    let refresh_outcome = memory::refresh_project(
        &svc,
        &worker,
        &project.id,
        std::slice::from_ref(&conv.id),
        "Acme",
    )
    .await
    .expect("refresh should succeed");
    assert!(!refresh_outcome.significant_change);

    // The manual line was fed into the prompt (proving current_memory was
    // passed through) and survives in the final document (the fake agent
    // echoed it back, standing in for a real agent following §5.4's
    // preserve-user-edits contract).
    let calls = refresh_agent.calls.lock().await;
    let prompt = calls[0]["prompt"].as_str().unwrap();
    assert!(prompt.contains(manual_line));
    drop(calls);

    let new_doc: Value = serde_json::from_slice(
        &std::fs::read(paths::project_memory_path(&project.id).unwrap()).unwrap(),
    )
    .unwrap();
    assert!(new_doc["overview_markdown"]
        .as_str()
        .unwrap()
        .contains(manual_line));

    let snapshot_path = refresh_outcome.snapshot_path.expect("snapshot must exist");
    assert!(snapshot_path.exists());
    let snapshot_content: Value =
        serde_json::from_slice(&std::fs::read(&snapshot_path).unwrap()).unwrap();
    assert_eq!(
        snapshot_content, pre_refresh_content,
        "snapshot must match pre-refresh content"
    );

    // ---- refresh_significant_change_flagged ------------------------------
    let large_agent = ScriptedAgent::new(vec![json!({
        "overview_markdown": "x",
        "scope_drift_markdown": "",
        "supersessions": [],
    })]);
    worker.register_reverse_rpc("run_agent_extraction", large_agent);
    let refresh_outcome2 = memory::refresh_project(
        &svc,
        &worker,
        &project.id,
        std::slice::from_ref(&conv.id),
        "Acme",
    )
    .await
    .expect("refresh should succeed even when it churns most content");
    assert!(refresh_outcome2.significant_change);
    assert!(refresh_outcome2.diff_ratio < 0.20);
    assert!(refresh_outcome2.snapshot_path.is_some());

    let history_dir = paths::project_memory_history_dir(&project.id).unwrap();
    let snapshot_count = std::fs::read_dir(&history_dir).unwrap().count();
    assert_eq!(snapshot_count, 2, "one snapshot per refresh call so far");

    worker.shutdown().await.unwrap();
}
