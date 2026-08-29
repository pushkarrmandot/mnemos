import time

import pytest

from mnemos_worker.capture.manager import CAPTURE_METHODS, CaptureAlreadyActive, CaptureManager
from mnemos_worker.capture.wasapi import StreamPair


class FakeStream:
    sample_rate = 16000
    channels = 1

    def read(self, num_frames: int) -> bytes:
        time.sleep(0.01)
        return b"\x00\x00" * num_frames

    def close(self) -> None:
        pass


def _manager(tmp_path):
    events = []

    def notify(method, params):
        events.append((method, params))

    mgr = CaptureManager(notify=notify, open_streams=lambda: StreamPair(mic=FakeStream(), system=FakeStream()))
    return mgr, events


def test_start_then_stop_returns_byte_counts(tmp_path):
    mgr, _ = _manager(tmp_path)
    result = mgr.start_capture(
        {
            "conversation_id": "c1",
            "mic_path": str(tmp_path / "mic.wav"),
            "system_path": str(tmp_path / "system.wav"),
        }
    )
    assert "started_at_ms" in result
    time.sleep(0.1)
    stopped = mgr.stop_capture({})
    assert stopped["mic_bytes"] >= 0
    assert stopped["system_bytes"] >= 0


def test_double_start_raises_capture_already_active(tmp_path):
    mgr, _ = _manager(tmp_path)
    mgr.start_capture(
        {
            "conversation_id": "c1",
            "mic_path": str(tmp_path / "mic1.wav"),
            "system_path": str(tmp_path / "sys1.wav"),
        }
    )
    with pytest.raises(CaptureAlreadyActive):
        mgr.start_capture(
            {
                "conversation_id": "c2",
                "mic_path": str(tmp_path / "mic2.wav"),
                "system_path": str(tmp_path / "sys2.wav"),
            }
        )
    mgr.stop_capture({})


def test_stop_with_no_active_session_is_idempotent(tmp_path):
    mgr, _ = _manager(tmp_path)
    result = mgr.stop_capture({})
    assert result == {"mic_bytes": 0, "system_bytes": 0}


def test_capture_methods_table_covers_all_four_rpcs():
    assert set(CAPTURE_METHODS) == {
        "start_capture",
        "stop_capture",
        "pause_capture",
        "resume_capture",
    }
