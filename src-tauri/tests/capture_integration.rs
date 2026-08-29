//! macOS-only integration tests for the `mnemos-audio` Swift sidecar (LLD-03
//! §4.1, LLD-02 §8) — spawns the real compiled binary (`swift build -c
//! release` in `swift/mnemos-audio/`, not a stub) and drives it through
//! `WorkerSupervisor::spawn_sidecar`.
//!
//! Covers this wave's DoD: a real recording produces two playable 16kHz
//! mono WAVs, and killing the sidecar mid-recording is detected as an
//! `Exited` event rather than a hang or a panic.

#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::time::Duration;

use mnemos_tauri_lib::capture::CaptureEvent;
use mnemos_tauri_lib::ipc::python::{SupervisorConfig, WorkerSupervisor};
use mnemos_tauri_lib::ipc::swift::SidecarConfig;

fn python_bin() -> PathBuf {
    PathBuf::from(if cfg!(windows) { "python" } else { "python3" })
}

fn src_python_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src-python")
}

fn sidecar_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../swift/mnemos-audio/.build/release/mnemos-audio")
}

/// Builds a supervisor whose Python side never has to actually come up —
/// these tests only exercise `spawn_sidecar`, which LLD-02 §8 documents as
/// disjoint state from the Python worker transport.
async fn supervisor(state_dir: PathBuf) -> std::sync::Arc<WorkerSupervisor> {
    let mut cfg = SupervisorConfig::new(python_bin(), src_python_dir(), state_dir);
    cfg.sidecar_bin = sidecar_bin();
    WorkerSupervisor::spawn(cfg)
        .await
        .expect("spawn always returns Ok")
}

fn wav_header_data_bytes(path: &std::path::Path) -> u32 {
    let bytes = std::fs::read(path).expect("wav file must exist");
    assert!(bytes.len() >= 44, "wav file too short to have a header");
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]])
}

#[tokio::test]
async fn records_and_produces_two_playable_16khz_mono_wavs() {
    assert!(
        sidecar_bin().exists(),
        "build the sidecar first: cd swift/mnemos-audio && swift build -c release"
    );
    let tmp = tempfile::tempdir().unwrap();
    let sup = supervisor(tmp.path().join("state")).await;

    let mic_path = tmp.path().join("mic.wav");
    let system_path = tmp.path().join("system.wav");

    let mut handle = sup
        .spawn_sidecar(SidecarConfig {
            conversation_id: "11111111-1111-1111-1111-111111111111".to_string(),
            mic_path: mic_path.clone(),
            system_path: system_path.clone(),
            mic_device_id: None,
        })
        .await
        .expect("spawn_sidecar must succeed with a real binary");

    let started = tokio::time::timeout(Duration::from_secs(5), handle.events.recv())
        .await
        .expect("must not time out waiting for Started")
        .expect("channel must not close before Started");
    assert!(
        matches!(started, CaptureEvent::Started { .. }),
        "first event must be Started, got {started:?}"
    );

    // Record for long enough to guarantee at least one 500ms flush per
    // source landed on disk before Stop.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    handle.control.stop().await.expect("stop command must send");
    let mut saw_stopped = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(1), handle.events.recv()).await {
            Ok(Some(CaptureEvent::Stopped { .. })) => {
                saw_stopped = true;
                break;
            }
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => break,
        }
    }
    assert!(
        saw_stopped,
        "must observe a Stopped event within 5s of Stop"
    );

    // Give the process a moment to fully exit and flush its final header
    // patch before we read the files back.
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(mic_path.exists(), "mic.wav must exist");
    assert!(system_path.exists(), "system.wav must exist");
    assert!(
        wav_header_data_bytes(&mic_path) > 0,
        "mic.wav header must declare a nonzero data size (playable)"
    );
    assert!(
        wav_header_data_bytes(&system_path) > 0,
        "system.wav header must declare a nonzero data size (playable)"
    );
}

#[tokio::test]
async fn killing_the_sidecar_mid_recording_is_detected_as_exited_not_a_hang() {
    assert!(
        sidecar_bin().exists(),
        "build the sidecar first: cd swift/mnemos-audio && swift build -c release"
    );
    let tmp = tempfile::tempdir().unwrap();
    let sup = supervisor(tmp.path().join("state")).await;

    let mut handle = sup
        .spawn_sidecar(SidecarConfig {
            conversation_id: "22222222-2222-2222-2222-222222222222".to_string(),
            mic_path: tmp.path().join("mic.wav"),
            system_path: tmp.path().join("system.wav"),
            mic_device_id: None,
        })
        .await
        .expect("spawn_sidecar must succeed with a real binary");

    let started = tokio::time::timeout(Duration::from_secs(5), handle.events.recv())
        .await
        .expect("must not time out waiting for Started")
        .expect("channel must not close before Started");
    assert!(matches!(started, CaptureEvent::Started { .. }));

    // Simulate a crash mid-recording — no `stop` command sent.
    handle.control.force_kill().await;

    let mut saw_exited = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(1), handle.events.recv()).await {
            Ok(Some(CaptureEvent::Exited { .. })) => {
                saw_exited = true;
                break;
            }
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => break,
        }
    }
    assert!(
        saw_exited,
        "a killed sidecar must surface Exited, not a silently-hanging channel"
    );
}
