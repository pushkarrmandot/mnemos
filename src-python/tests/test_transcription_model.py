import threading
import time
import wave

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
    segments = model.transcribe_pcm(
        b"\x00" * 100, sample_rate=16000, stream_ctx="conv1"
    )
    assert segments == [Segment(text="hi", ts_start_ms=0, ts_end_ms=500)]


def test_transcribe_file_returns_segments():
    model = ParakeetModel(backend=FakeBackend(delay_s=0))
    segments = model.transcribe_file("/tmp/mic.wav")
    assert segments[0].text == "hello file"


def test_infer_lock_serializes_pcm_and_file_calls():
    """Live-poll calls and post-batch calls never enter the
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
        "mnemos_worker.models.transcription._load_backend",
        lambda: FakeBackend(delay_s=0),
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
    monkeypatch.setattr(
        "mnemos_worker.models.transcription.platform.system", lambda: "Linux"
    )
    with pytest.raises(RuntimeError):
        from mnemos_worker.models.transcription import _load_backend

        _load_backend()


class TestDownloadProgressHook:
    """`_report_download_progress` shipped broken twice over, and neither
    failure was visible: the hook raised on import and swallowed it at
    `debug` level, and once that was fixed it read `self.n` — which
    `tqdm.update()` never advances on a disabled bar, and Mnemos runs
    headless so the bar is always disabled. The onboarding UI showed its
    indeterminate placeholder through an entire 2.3GB download and looked
    plausible doing it, which is why nothing caught either one.

    These run entirely offline against the real installed `huggingface_hub`
    tqdm class, so they fail if a future version moves the machinery again.
    """

    @staticmethod
    def _hf_tqdm():
        import importlib

        return importlib.import_module("huggingface_hub.utils.tqdm").tqdm

    def test_hook_patches_the_real_module(self):
        """The original bug: `import huggingface_hub.utils.tqdm as m` binds
        the tqdm *class* (utils/__init__ re-exports it, shadowing the
        submodule), so `m.tqdm` raised AttributeError and the hook silently
        never installed itself."""
        from mnemos_worker.models.transcription import _report_download_progress

        before = self._hf_tqdm()
        with _report_download_progress():
            assert self._hf_tqdm() is not before, "hook did not patch the class"
        assert self._hf_tqdm() is before, "hook did not restore the class"

    def test_progress_advances_on_a_disabled_bar(self):
        """`tqdm.update()` returns early on a disabled bar without touching
        `self.n`, so progress has to come from the `n` argument itself."""
        from mnemos_worker.models.transcription import (
            _progress,
            _report_download_progress,
        )

        _progress.set(0, 0, done=False)
        with _report_download_progress():
            bar = self._hf_tqdm()(total=1000, disable=True)
            for _ in range(4):
                bar.update(250)
            snap = _progress.snapshot()
        assert bar.n == 0, "precondition: a disabled bar never advances self.n"
        assert snap["received_bytes"] == 1000
        assert snap["total_bytes"] == 1000

    def test_inflating_transfer_bar_never_becomes_the_denominator(self):
        """Xet downloads drive two bars into this one hook. The transfer
        bar's `total` is deliberately inflated to `n * 1.25` on every update
        (huggingface_hub/utils/_xet_progress_reporting.py), so reporting it
        would make the UI's denominator climb forever."""
        from mnemos_worker.models.transcription import (
            _progress,
            _report_download_progress,
        )

        real_total = 1_000_000
        _progress.set(0, 0, done=False)
        received = []
        with _report_download_progress():
            cls = self._hf_tqdm()
            recon = cls(total=real_total, disable=True)
            transfer = cls(total=real_total, disable=True)
            step = real_total // 10
            for i in range(10):
                recon.update(step)
                received.append(_progress.snapshot())
                transfer.total = int((i + 1) * step * 1.25) + 1
                transfer.update(step)
                received.append(_progress.snapshot())

        assert {s["total_bytes"] for s in received} == {real_total}
        counts = [s["received_bytes"] for s in received]
        assert all(b >= a for a, b in zip(counts, counts[1:])), (
            "progress went backwards"
        )
        assert counts[-1] >= real_total * 0.99


class TestWavFastPath:
    """Mnemos records 16 kHz mono 16-bit WAVs, which is exactly what
    `parakeet-mlx` shells out to ffmpeg to produce — so the subprocess
    converted the format into itself, and its absence broke every
    transcription on any Mac without Homebrew. These cover the replacement
    and, critically, that ffmpeg is no longer required for our own audio.
    """

    @staticmethod
    def _write_wav(path, *, channels=1, width=2, rate=16000, frames=1600):
        import struct

        with wave.open(str(path), "wb") as w:
            w.setnchannels(channels)
            w.setsampwidth(width)
            w.setframerate(rate)
            w.writeframes(
                struct.pack("<" + "h" * frames, *range(-frames // 2, frames // 2))
            )

    def test_decodes_our_own_recording_shape(self, tmp_path):
        pytest.importorskip("mlx.core")
        from mnemos_worker.models.transcription import _load_wav_fast

        path = tmp_path / "mic.wav"
        self._write_wav(path)
        audio = _load_wav_fast(path, 16000)

        assert audio.shape == (1600,)
        # float32, NOT the `dtype` default: upstream accepts the argument and
        # ignores it, always returning float32. Honouring `bfloat16` instead
        # blew up inside the mel transform — a failure only a real
        # transcription surfaced.
        assert str(audio.dtype).endswith("float32")

    def test_ignores_the_dtype_argument_exactly_as_upstream_does(self, tmp_path):
        mx = pytest.importorskip("mlx.core")
        from mnemos_worker.models.transcription import _load_wav_fast

        path = tmp_path / "mic.wav"
        self._write_wav(path)

        assert str(_load_wav_fast(path, 16000, mx.bfloat16).dtype).endswith("float32")

    def test_rejects_anything_that_is_not_our_recording_shape(self, tmp_path):
        pytest.importorskip("mlx.core")
        from mnemos_worker.models.transcription import _UnsupportedWav, _load_wav_fast

        stereo = tmp_path / "stereo.wav"
        self._write_wav(stereo, channels=2)
        with pytest.raises(_UnsupportedWav):
            _load_wav_fast(stereo, 16000)

        wrong_rate = tmp_path / "44k.wav"
        self._write_wav(wrong_rate, rate=44100)
        with pytest.raises(_UnsupportedWav):
            _load_wav_fast(wrong_rate, 16000)

    def test_our_audio_decodes_with_ffmpeg_unavailable(self, tmp_path, monkeypatch):
        """The regression that matters: this must not need ffmpeg.

        `shutil.which` is forced to find nothing, which is the exact condition
        `parakeet_mlx.audio.load_audio` checks before raising "FFmpeg is not
        installed or not in your PATH."
        """
        pytest.importorskip("mlx.core")
        from mnemos_worker.models import transcription as T

        monkeypatch.setattr("shutil.which", lambda _name: None)
        path = tmp_path / "mic.wav"
        self._write_wav(path)

        assert T._load_wav_fast(path, 16000).shape == (1600,)

    def test_install_patches_the_binding_the_library_actually_resolves(self):
        """`parakeet.py` does `from parakeet_mlx.audio import load_audio`, so
        it holds its own reference — patching `parakeet_mlx.audio` would do
        nothing. Same trap that left the download-progress hook dead."""
        pk = pytest.importorskip("parakeet_mlx.parakeet")
        from mnemos_worker.models.transcription import (
            _WAV_FAST_PATH_SENTINEL,
            install_wav_fast_path,
        )

        original = pk.load_audio
        try:
            assert install_wav_fast_path() is True
            assert getattr(pk.load_audio, _WAV_FAST_PATH_SENTINEL, False) is True
            # Idempotent: a second call must not wrap the wrapper.
            patched = pk.load_audio
            assert install_wav_fast_path() is True
            assert pk.load_audio is patched
        finally:
            pk.load_audio = original

    def test_unsupported_wav_falls_back_to_the_original_loader(self, tmp_path):
        """ffmpeg stays the general-purpose decoder — this narrows the
        dependency to unusual inputs rather than removing the escape hatch."""
        pk = pytest.importorskip("parakeet_mlx.parakeet")
        from mnemos_worker.models.transcription import install_wav_fast_path

        original = pk.load_audio
        called: list[str] = []
        try:
            pk.load_audio = lambda *a, **k: called.append("original") or "delegated"
            install_wav_fast_path()

            stereo = tmp_path / "stereo.wav"
            self._write_wav(stereo, channels=2)
            assert pk.load_audio(stereo, 16000) == "delegated"
            assert called == ["original"]
        finally:
            pk.load_audio = original
