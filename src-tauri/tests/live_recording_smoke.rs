//! W9 opt-in live smoke test (LLD-07 §9's "live-call opt-in" pattern,
//! already used by `ipc/runner/extraction_handler.rs`'s
//! `run_extraction_against_the_real_claude_cli`). Not run in CI.
//!
//! Exercises the exact `WorkerSupervisor` + sidecar + Python-worker call
//! sequence `commands::recording::{start_recording,subscribe_transcript,
//! stop_recording}` make — just invoked directly instead of through the
//! `#[tauri::command]`/`AppHandle`/`AppState` layer, which this test has no
//! running Tauri `App` to construct. It is the closest a headless test gets
//! to "click Record in the app": real sidecar, real mic + system capture,
//! real Parakeet weights, real live-transcript chunks, real
//! `transcribe_final`.
//!
//! Run with:
//! `MNEMOS_LIVE_RECORDING=1 cargo test --test live_recording_smoke -- --ignored --nocapture`

#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::time::Duration;

use mnemos_tauri_lib::ipc::python::{
    SubscribeLiveTranscript, SupervisorConfig, TranscribeFinal, UnsubscribeLiveTranscript,
    WorkerSupervisor, LIVE_TRANSCRIPT_CHUNK_TOPIC,
};
use mnemos_tauri_lib::ipc::swift::SidecarConfig;

fn python_bin() -> PathBuf {
    // The `python3` `start_recording`'s real `worker_config()` resolves on
    // PATH in this environment — the one with `parakeet_mlx` installed
    // (confirmed this wave: `which python3` -> `/opt/anaconda3/bin/python3`).
    PathBuf::from("python3")
}

fn src_python_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src-python")
}

fn sidecar_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../swift/mnemos-audio/.build/release/mnemos-audio")
}

#[tokio::test]
#[ignore]
async fn real_recording_produces_a_real_live_transcript_and_final_transcript() {
    if std::env::var("MNEMOS_LIVE_RECORDING").as_deref() != Ok("1") {
        eprintln!("skipping: set MNEMOS_LIVE_RECORDING=1 to run against real mic/system capture");
        return;
    }
    assert!(sidecar_bin().exists(), "build the sidecar first");

    let _ = tracing_subscriber::fmt()
        .with_env_filter("debug")
        .try_init();

    let tmp = tempfile::tempdir().unwrap();
    let cfg = {
        let mut c = SupervisorConfig::new(python_bin(), src_python_dir(), tmp.path().join("state"));
        c.handshake_timeout = Duration::from_secs(30); // real Parakeet warm-up can be slow on first run
        c.sidecar_bin = sidecar_bin();
        c
    };
    let sup = WorkerSupervisor::spawn(cfg).await.expect("spawn always Ok");

    // Give the worker's ParakeetModel.warm_up() (best-effort, at startup)
    // real time to finish loading weights before we start the clock.
    tokio::time::sleep(Duration::from_secs(5)).await;

    let conv_id = "live-smoke-test".to_string();
    let mic_path = tmp.path().join("mic.wav");
    let system_path = tmp.path().join("system.wav");

    let handle = sup
        .spawn_sidecar(SidecarConfig {
            conversation_id: conv_id.clone(),
            mic_path: mic_path.clone(),
            system_path: system_path.clone(),
            mic_device_id: None,
        })
        .await
        .expect("real sidecar spawn");

    sup.send(SubscribeLiveTranscript {
        conversation_id: conv_id.clone(),
        mic_path: mic_path.clone(),
    })
    .await
    .expect("real subscribe_live_transcript");

    let mut rx = sup.subscribe(LIVE_TRANSCRIPT_CHUNK_TOPIC);
    let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let conv_id_for_task = conv_id.clone();
    let listener = tokio::spawn(async move {
        while let Ok(value) = rx.recv().await {
            if let Ok(notif) = serde_json::from_value::<
                mnemos_tauri_lib::ipc::python::LiveTranscriptChunkNotification,
            >(value)
            {
                if notif.conversation_id == conv_id_for_task {
                    eprintln!("LIVE CHUNK: {:?}", notif.chunk.text);
                    let _ = chunk_tx.send(notif.chunk.text);
                }
            }
        }
    });

    // Real audio: play synthesized speech aloud through the speakers for
    // ~18s (real time to speak into a mic for) so the physical mic — not a
    // software injection — actually picks it up, same as a human talking.
    let speech = std::process::Command::new("say")
        .args([
            "-v",
            "Samantha",
            "-o",
            "/tmp/mnemos_live_smoke.aiff",
            &speech_text(),
        ])
        .status();
    assert!(
        speech.map(|s| s.success()).unwrap_or(false),
        "say(1) failed"
    );
    let convert = std::process::Command::new("afconvert")
        .args([
            "-f",
            "WAVE",
            "-d",
            "LEI16@16000",
            "-c",
            "1",
            "/tmp/mnemos_live_smoke.aiff",
            "/tmp/mnemos_live_smoke.wav",
        ])
        .status();
    assert!(
        convert.map(|s| s.success()).unwrap_or(false),
        "afconvert failed"
    );

    tokio::time::sleep(Duration::from_secs(2)).await;
    let _ = std::process::Command::new("afplay")
        .arg("/tmp/mnemos_live_smoke.wav")
        .status();

    // Give the live-tx poll loop (5s cadence) time to pick up and transcribe
    // the just-played speech.
    tokio::time::sleep(Duration::from_secs(12)).await;

    handle.control.stop().await.expect("real sidecar stop");
    tokio::time::sleep(Duration::from_secs(1)).await;

    let _ = sup
        .send(UnsubscribeLiveTranscript {
            conversation_id: conv_id.clone(),
        })
        .await;
    listener.abort();

    let mut live_chunks = Vec::new();
    while let Ok(text) = chunk_rx.try_recv() {
        live_chunks.push(text);
    }
    eprintln!("live chunks received: {live_chunks:?}");
    assert!(
        !live_chunks.is_empty(),
        "expected at least one real live_transcript_chunk from the real mic"
    );

    let final_result = sup
        .send(TranscribeFinal {
            conversation_id: conv_id,
            mic_path,
            system_path,
        })
        .await
        .expect("real transcribe_final");
    eprintln!("transcribe_final: {final_result:?}");
    assert!(
        final_result.segment_count > 0,
        "expected real segments in the final transcript"
    );
}

fn speech_text() -> String {
    "The quarterly budget meeting starts now. We need to finalize the roadmap for next quarter."
        .to_string()
}
