import time

import pytest

from mnemos_worker.capture.wasapi import (
    NO_SIGNAL_DB_THRESHOLD,
    NO_SIGNAL_WARNING_SECONDS,
    StreamPair,
    WindowsCapture,
)


class FakeStream:
    """Deterministic 16kHz mono int16 silence-ish stream for tests — real
    device I/O is exercised only on Windows via `open_wasapi_streams`."""

    def __init__(self, value: int = 100):
        self.sample_rate = 16000
        self.channels = 1
        self._value = value
        self.closed = False

    def read(self, num_frames: int) -> bytes:
        time.sleep(0.01)  # avoid a hot spin loop in the test
        return (self._value.to_bytes(2, "little", signed=True)) * num_frames

    def close(self) -> None:
        self.closed = True


class FailingStream(FakeStream):
    def read(self, num_frames: int) -> bytes:
        raise OSError("AUDCLNT_E_DEVICE_INVALIDATED")


def _collect_events():
    events = []

    def notify(method, params):
        assert method == "capture_event"
        events.append(params)

    return events, notify


def test_start_stop_produces_started_and_stopped_events(tmp_path):
    events, notify = _collect_events()
    mic = FakeStream(1000)
    system = FakeStream(-1000)

    capture = WindowsCapture(
        conversation_id="c1",
        mic_path=str(tmp_path / "mic.wav"),
        system_path=str(tmp_path / "system.wav"),
        notify=notify,
        open_streams=lambda: StreamPair(mic=mic, system=system),
    )
    started_at_ms = capture.start()
    assert started_at_ms > 0

    time.sleep(0.6)  # cross at least one 500ms chunk-flush boundary
    mic_bytes, system_bytes = capture.stop()

    assert mic_bytes > 0
    assert system_bytes > 0
    assert mic.closed and system.closed

    kinds = [e["kind"] for e in events]
    assert kinds[0] == "started"
    assert "chunk" in kinds
    assert "level" in kinds
    assert kinds[-1] == "stopped"


def test_pause_resume_emit_events_and_stop_streams_writing(tmp_path):
    events, notify = _collect_events()
    mic = FakeStream()
    system = FakeStream()

    capture = WindowsCapture(
        conversation_id="c2",
        mic_path=str(tmp_path / "mic.wav"),
        system_path=str(tmp_path / "system.wav"),
        notify=notify,
        open_streams=lambda: StreamPair(mic=mic, system=system),
    )
    capture.start()
    capture.pause()
    capture.resume()
    capture.stop()

    kinds = [e["kind"] for e in events]
    assert "paused" in kinds
    assert "resumed" in kinds


def test_stream_open_failure_raises_from_start_not_a_silent_success(tmp_path):
    # Finding #7: a failed stream open must surface as a real `start()`
    # failure (not hang, but also not a silent success) — the caller
    # (`CaptureManager.start_capture`) turns this into a JSON-RPC error
    # instead of reporting a recording that captures nothing.
    events, notify = _collect_events()

    def open_streams():
        raise OSError("AUDCLNT_E_DEVICE_INVALIDATED: no mic")

    capture = WindowsCapture(
        conversation_id="c3",
        mic_path=str(tmp_path / "mic.wav"),
        system_path=str(tmp_path / "system.wav"),
        notify=notify,
        open_streams=open_streams,
    )
    with pytest.raises(OSError):
        capture.start()
    capture.stop()  # idempotent no-op: thread already exited

    kinds = [e["kind"] for e in events]
    assert kinds[0] == "error"
    assert events[0]["error_kind"] == "mic_disconnected"


def test_pause_keeps_reading_but_stops_writing_and_leveling(tmp_path):
    # Finding #16: Pause must keep draining the stream (so the WASAPI ring
    # buffer never overruns) but must not write frames to the WAV files or
    # emit level/silence events while paused — mirrors the mac sidecar's
    # "keep taps running, drop buffers" semantic.
    events, notify = _collect_events()
    mic = FakeStream(1000)
    system = FakeStream(-1000)

    capture = WindowsCapture(
        conversation_id="c5",
        mic_path=str(tmp_path / "mic.wav"),
        system_path=str(tmp_path / "system.wav"),
        notify=notify,
        open_streams=lambda: StreamPair(mic=mic, system=system),
    )
    capture.start()
    capture.pause()
    time.sleep(0.3)  # several read iterations while paused
    mic_bytes_while_paused = capture._mic_writer.bytes_written
    capture.resume()
    time.sleep(0.2)
    # `stop()` joins the thread with a timeout — if pause still stopped the
    # read loop entirely (the pre-fix behavior) this call still succeeds, so
    # the real regression check is `mic_bytes_while_paused == 0` below, not
    # this call completing.
    mic_bytes, system_bytes = capture.stop()

    assert mic_bytes_while_paused == 0  # nothing written while paused
    assert mic_bytes > 0 and system_bytes > 0  # writing resumed after Resume
    kinds = [e["kind"] for e in events]
    assert "paused" in kinds and "resumed" in kinds


def test_mic_silence_warning_fires_after_threshold_and_clears_on_signal():
    # Finding #17 — unit-level check of `_track_mic_silence` directly
    # (avoids a real `NO_SIGNAL_WARNING_SECONDS`-long sleep in the test
    # suite): silence below threshold for the configured duration emits
    # exactly one `no_mic_signal` warning, and audible signal clears the
    # state so a later silence period can warn again.
    events, notify = _collect_events()
    capture = WindowsCapture(
        conversation_id="c6",
        mic_path="/tmp/unused-mic.wav",
        system_path="/tmp/unused-system.wav",
        notify=notify,
        open_streams=lambda: (_ for _ in ()).throw(AssertionError("not used")),
    )

    silent_db = NO_SIGNAL_DB_THRESHOLD - 10.0
    loud_db = NO_SIGNAL_DB_THRESHOLD + 10.0

    # Simulate the silence having started NO_SIGNAL_WARNING_SECONDS + 1 ago.
    capture._mic_silence_start = time.monotonic() - (NO_SIGNAL_WARNING_SECONDS + 1)
    capture._track_mic_silence(silent_db)
    capture._track_mic_silence(silent_db)  # second call must not re-warn

    warnings = [e for e in events if e["kind"] == "warning"]
    assert len(warnings) == 1
    assert warnings[0]["warning_kind"] == "no_mic_signal"

    # Audible signal clears the warned/silence-start state.
    capture._track_mic_silence(loud_db)
    assert capture._mic_silence_start is None
    assert capture._mic_warned is False


def test_mid_stream_read_failure_emits_error_and_still_stops(tmp_path):
    events, notify = _collect_events()
    mic = FailingStream()
    system = FakeStream()

    capture = WindowsCapture(
        conversation_id="c4",
        mic_path=str(tmp_path / "mic.wav"),
        system_path=str(tmp_path / "system.wav"),
        notify=notify,
        open_streams=lambda: StreamPair(mic=mic, system=system),
    )
    capture.start()
    capture.stop()

    kinds = [e["kind"] for e in events]
    assert "error" in kinds
    assert kinds[-1] == "stopped"
