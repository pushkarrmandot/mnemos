import struct
import time

from mnemos_worker.jobs.live_transcription import (
    BYTES_PER_SEC,
    WAV_HEADER_BYTES,
    LiveTranscriptionManager,
    LiveTranscriptionThread,
)
from mnemos_worker.models.transcription import Segment


def _write_wav_header(path, data_bytes: int = 0) -> None:
    byte_rate = 16000 * 1 * 2
    header = (
        b"RIFF"
        + struct.pack("<I", 36 + data_bytes)
        + b"WAVE"
        + b"fmt "
        + struct.pack("<I", 16)
        + struct.pack("<H", 1)
        + struct.pack("<H", 1)
        + struct.pack("<I", 16000)
        + struct.pack("<I", byte_rate)
        + struct.pack("<H", 2)
        + struct.pack("<H", 16)
        + b"data"
        + struct.pack("<I", data_bytes)
    )
    assert len(header) == WAV_HEADER_BYTES
    with open(path, "wb") as f:
        f.write(header)


def _append_pcm(path, num_seconds: float) -> None:
    num_bytes = int(BYTES_PER_SEC * num_seconds)
    with open(path, "r+b") as f:
        f.seek(0, 2)
        f.write(b"\x00" * num_bytes)


def test_tick_skips_below_min_audio_threshold(tmp_path):
    mic_path = tmp_path / "mic.wav"
    _write_wav_header(mic_path)
    _append_pcm(mic_path, 0.5)  # below MIN_AUDIO_SEC=2.0

    events = []
    thread = LiveTranscriptionThread(
        "conv1", str(mic_path), notify=lambda m, p: events.append((m, p)),
        transcribe_pcm=lambda pcm, **kw: [Segment(text="x", ts_start_ms=0, ts_end_ms=100)],
    )
    thread._tick()
    # W17b: the model-readiness signal fires on the very first tick
    # regardless of audio state; the below-min-audio gate below it is
    # unaffected.
    assert events == [("live_transcription_warmup", {"conversation_id": "conv1", "ready": True})]


def test_tick_emits_chunk_with_absolute_timestamps(tmp_path):
    mic_path = tmp_path / "mic.wav"
    _write_wav_header(mic_path)
    _append_pcm(mic_path, 3.0)  # above MIN_AUDIO_SEC

    events = []
    thread = LiveTranscriptionThread(
        "conv1",
        str(mic_path),
        notify=lambda m, p: events.append((m, p)),
        transcribe_pcm=lambda pcm, **kw: [Segment(text="hello", ts_start_ms=100, ts_end_ms=600)],
    )
    thread._tick()

    # W17b: first event is the readiness signal, second is the real chunk.
    assert len(events) == 2
    assert events[0] == ("live_transcription_warmup", {"conversation_id": "conv1", "ready": True})
    method, params = events[1]
    assert method == "live_transcript_chunk"
    assert params["conversation_id"] == "conv1"
    chunk = params["chunk"]
    assert chunk["text"] == "hello"
    # base_ms for the first tick is 0 (cursor starts right after header)
    assert chunk["ts_start_ms"] == 100
    assert chunk["ts_end_ms"] == 600


def test_tick_advances_cursor_so_next_tick_is_relative(tmp_path):
    mic_path = tmp_path / "mic.wav"
    _write_wav_header(mic_path)
    _append_pcm(mic_path, 3.0)

    calls = []

    def fake_transcribe(pcm, **kw):
        calls.append(len(pcm))
        return [Segment(text="t", ts_start_ms=0, ts_end_ms=100)]

    thread = LiveTranscriptionThread(
        "conv1", str(mic_path), notify=lambda m, p: None, transcribe_pcm=fake_transcribe
    )
    thread._tick()
    first_cursor = thread._cursor
    assert first_cursor == WAV_HEADER_BYTES + int(BYTES_PER_SEC * 3.0)

    _append_pcm(mic_path, 3.0)
    thread._tick()
    assert thread._cursor == first_cursor + int(BYTES_PER_SEC * 3.0)
    assert len(calls) == 2


def test_tick_reports_job_error_on_transcribe_failure(tmp_path):
    mic_path = tmp_path / "mic.wav"
    _write_wav_header(mic_path)
    _append_pcm(mic_path, 3.0)

    events = []

    def boom(pcm, **kw):
        raise RuntimeError("model exploded")

    thread = LiveTranscriptionThread(
        "conv1", str(mic_path), notify=lambda m, p: events.append((m, p)), transcribe_pcm=boom
    )
    thread._tick()

    # W17b: first event is the readiness signal, second is the job error.
    assert len(events) == 2
    assert events[0] == ("live_transcription_warmup", {"conversation_id": "conv1", "ready": True})
    method, params = events[1]
    assert method == "job_error"
    assert params["error_class"] == "live_transcription"
    assert params["conversation_id"] == "conv1"


def test_stop_and_drain_runs_final_tick_and_joins(tmp_path):
    mic_path = tmp_path / "mic.wav"
    _write_wav_header(mic_path)
    _append_pcm(mic_path, 0.5)  # below MIN_AUDIO_SEC, but final tick must still drain it

    events = []
    thread = LiveTranscriptionThread(
        "conv1",
        str(mic_path),
        notify=lambda m, p: events.append((m, p)),
        transcribe_pcm=lambda pcm, **kw: [Segment(text="tail", ts_start_ms=0, ts_end_ms=50)],
    )
    thread.start()
    time.sleep(0.05)
    thread.stop_and_drain(timeout=2.0)

    assert not thread.is_alive()
    assert any(m == "live_transcript_chunk" and p["chunk"]["text"] == "tail" for m, p in events)


def _fake_transcribe_pcm(pcm, **kw):
    return [Segment(text="x", ts_start_ms=0, ts_end_ms=100)]


def test_manager_subscribe_is_idempotent(tmp_path):
    mic_path = tmp_path / "mic.wav"
    _write_wav_header(mic_path)

    mgr = LiveTranscriptionManager(notify=lambda m, p: None, transcribe_pcm=_fake_transcribe_pcm)
    r1 = mgr.subscribe_live_transcript({"conversation_id": "c1", "mic_path": str(mic_path)})
    r2 = mgr.subscribe_live_transcript({"conversation_id": "c1", "mic_path": str(mic_path)})
    assert r1 == {}
    assert r2 == {}
    assert len(mgr._active) == 1

    mgr.unsubscribe_live_transcript({"conversation_id": "c1"})
    assert mgr._active == {}


def test_manager_unsubscribe_unknown_conversation_is_noop():
    mgr = LiveTranscriptionManager(notify=lambda m, p: None)
    result = mgr.unsubscribe_live_transcript({"conversation_id": "unknown"})
    assert result == {}


def test_manager_stop_all_drains_every_active_thread(tmp_path):
    mic_path = tmp_path / "mic.wav"
    _write_wav_header(mic_path)

    mgr = LiveTranscriptionManager(notify=lambda m, p: None, transcribe_pcm=_fake_transcribe_pcm)
    mgr.subscribe_live_transcript({"conversation_id": "c1", "mic_path": str(mic_path)})
    mgr.subscribe_live_transcript({"conversation_id": "c2", "mic_path": str(mic_path)})

    threads = list(mgr._active.values())
    mgr.stop_all(timeout=2.0)

    assert mgr._active == {}
    for t in threads:
        assert not t.is_alive()
