import threading
import time

import pytest

from mnemos_worker.models.transcription import ParakeetModel, Segment


class FakeBackend:
    def __init__(self, delay_s: float = 0.05) -> None:
        self.delay_s = delay_s
        self.calls: list[str] = []
        self._concurrent = 0
        self.max_concurrent = 0
        self._concurrency_lock = threading.Lock()

    def _record_entry(self, name: str) -> None:
        with self._concurrency_lock:
            self._concurrent += 1
            self.max_concurrent = max(self.max_concurrent, self._concurrent)
        self.calls.append(name)

    def _record_exit(self) -> None:
        with self._concurrency_lock:
            self._concurrent -= 1

    def transcribe_pcm(self, pcm: bytes, sample_rate: int) -> list[Segment]:
        self._record_entry("pcm")
        try:
            time.sleep(self.delay_s)
            return [Segment(text="hi", ts_start_ms=0, ts_end_ms=500)]
        finally:
            self._record_exit()

    def transcribe_file(self, path: str, on_progress=None) -> list[Segment]:
        self._record_entry("file")
        try:
            time.sleep(self.delay_s)
            # Mirrors a real backend reporting as it goes, so the fake also
            # exercises the progress contract rather than only its signature.
            if on_progress is not None:
                on_progress(1, 1)
            return [Segment(text="hello file", ts_start_ms=0, ts_end_ms=1000)]
        finally:
            self._record_exit()


def test_transcribe_pcm_returns_segments():
    model = ParakeetModel(backend=FakeBackend(delay_s=0))
    segments = model.transcribe_pcm(b"\x00" * 100, sample_rate=16000, stream_ctx="conv1")
    assert segments == [Segment(text="hi", ts_start_ms=0, ts_end_ms=500)]


def test_transcribe_file_returns_segments():
    model = ParakeetModel(backend=FakeBackend(delay_s=0))
    segments = model.transcribe_file("/tmp/mic.wav")
    assert segments[0].text == "hello file"


def test_infer_lock_serializes_pcm_and_file_calls():
    """LLD-03 §7: live-poll calls and post-batch calls never enter the
    (non-thread-safe) model concurrently."""
    backend = FakeBackend(delay_s=0.05)
    model = ParakeetModel(backend=backend)

    errors: list[Exception] = []

    def run_pcm():
        try:
            model.transcribe_pcm(b"\x00" * 10, sample_rate=16000, stream_ctx="c")
        except Exception as exc:  # noqa: BLE001
            errors.append(exc)

    def run_file():
        try:
            model.transcribe_file("/tmp/system.wav")
        except Exception as exc:  # noqa: BLE001
            errors.append(exc)

    threads = [threading.Thread(target=run_pcm), threading.Thread(target=run_file)]
    for t in threads:
        t.start()
    for t in threads:
        t.join(timeout=2.0)

    assert not errors
    assert backend.max_concurrent == 1


def test_get_returns_singleton(monkeypatch):
    ParakeetModel._instance = None
    monkeypatch.setattr(
        "mnemos_worker.models.transcription._load_backend", lambda: FakeBackend(delay_s=0)
    )
    a = ParakeetModel.get()
    b = ParakeetModel.get()
    assert a is b
    ParakeetModel._instance = None


def test_warm_up_swallows_backend_load_failure(monkeypatch):
    ParakeetModel._instance = None

    def _raise():
        raise ImportError("no parakeet package installed")

    monkeypatch.setattr("mnemos_worker.models.transcription._load_backend", _raise)
    ParakeetModel.warm_up()  # must not raise
    assert ParakeetModel._instance is None
    ParakeetModel._instance = None


def test_unsupported_platform_raises(monkeypatch):
    monkeypatch.setattr("mnemos_worker.models.transcription.platform.system", lambda: "Linux")
    with pytest.raises(RuntimeError):
        from mnemos_worker.models.transcription import _load_backend

        _load_backend()
