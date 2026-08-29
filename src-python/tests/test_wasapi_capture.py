import time

from mnemos_worker.capture.wasapi import StreamPair, WindowsCapture


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


def test_stream_open_failure_emits_error_not_a_hang(tmp_path):
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
    capture.start()  # must not raise/hang even though the factory fails
    capture.stop()

    kinds = [e["kind"] for e in events]
    assert kinds[0] == "error"
    assert events[0]["error_kind"] == "mic_disconnected"


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
