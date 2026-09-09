"""`ParakeetModel` — one process-wide instance shared by three callers
(live-transcription poll thread, `process_conversation`'s transcribing step,
and this class's own warm-up call at worker startup), all serialized through
a single-worker `ThreadPoolExecutor` (see `_executor` on the class below) so
every call runs on the exact OS thread the backend was loaded on. Direct
access to `ParakeetModel._instance` from outside this module is banned by a
ruff custom rule — callers always go through `ParakeetModel.get()`.

A few backend quirks shape this module's design:

- `parakeet-mlx`'s real API differs from its docs in two places this module
  works around: `BaseParakeet` has no `transcribe_pcm` — only `transcribe(path)`
  and `transcribe_stream()` — so `_pcm_as_temp_wav` bridges raw PCM to a
  temp file (see that function's docstring); and its result shape is
  `AlignedResult.sentences`, not the docs' description, which
  `_segments_from_aligned_result` accounts for. This module is written
  against the actual installed package's behavior. Windows uses `onnx_asr`
  instead (see `_ParakeetOnnxBackend`'s docstring) — its API and download
  mechanism were confirmed by reading its real source, not assumed; end-to-
  end transcription on real Windows hardware still needs to be exercised.
- MLX (`mlx.core`) binds a loaded model's weight arrays to the exact OS
  thread that first evaluated them — calling `transcribe`/`transcribe_pcm`
  from any other thread raises `RuntimeError: There is no Stream(cpu, N) in
  current thread`, reproducibly, regardless of which `mx.core` stream APIs
  (`new_stream`, `new_thread_local_stream`, `set_default_stream`) are used
  from the calling thread. `ParakeetModel` therefore loads its backend on —
  and routes every call through — one dedicated single-worker thread (see
  `_executor` below), so warm-up, live-poll ticks, and `transcribe_final`
  all execute on the same thread the model was born on.
- Checking cached weights for freshness on every worker start means a live
  Hugging Face HEAD-request round trip on every single launch —
  network-dependent latency and log noise. `_load_parakeet_weights` sets
  `HF_HUB_OFFLINE=1` (native to `hf_hub_download`) once a marker file shows
  we checked within the last week, so that round trip only happens roughly
  weekly instead of always.
"""

from __future__ import annotations

import importlib
import os
import platform
import tempfile
import threading
import time
import wave
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Iterator, Protocol

from mnemos_worker.logging_config import get_logger

log = get_logger(component="parakeet-model")

# Public, short model id onboarding/the frontend refer to (distinct from the
# HF repo id / gguf filename each backend uses internally) — the
# `models.download_model` command and `model_download_progress` notification
# topic key their payloads on this string.
PARAKEET_MODEL_ID = "parakeet-tdt-0.6b-v3"


@dataclass(frozen=True)
class Segment:
    """One transcribed turn. `ts_start_ms`/`ts_end_ms` are relative to the
    start of the audio buffer/file passed to `transcribe_pcm`/`transcribe_file`
    — callers convert to absolute timestamps (see `_cursor_to_ms`)."""

    text: str
    ts_start_ms: int
    ts_end_ms: int
    speaker_label_hint: str | None = None


ProgressFn = Callable[[int, int], None]
"""`(processed_samples, total_samples)`. Fractional progress is the caller's
to compute; backends report raw counts so nothing has to agree on units."""


class Backend(Protocol):
    def transcribe_pcm(self, pcm: bytes, sample_rate: int) -> list[Segment]: ...

    def transcribe_file(
        self, path: str, on_progress: ProgressFn | None = None
    ) -> list[Segment]:
        """`on_progress` is part of the contract on *every* platform, not an
        optimisation macOS happens to support.

        Transcription time scales with audio length and with the machine, so
        the only device-independent way to tell "slow" from "dead" is for the
        backend to keep saying it is alive (see `job_progress`). A backend
        that silently ignores this argument therefore reintroduces the
        fixed-timeout bug on its platform — long recordings will fail there
        and nowhere else. Declaring it here means a new backend cannot be
        added without confronting the requirement.
        """
        ...


def _load_backend() -> Backend:
    system = platform.system()
    if system == "Darwin":
        return _ParakeetMlxBackend()
    if system == "Windows":
        return _ParakeetOnnxBackend()
    raise RuntimeError(f"no Parakeet backend for platform: {system}")


@contextmanager
def _pcm_as_temp_wav(pcm: bytes, sample_rate: int) -> Iterator[str]:
    """`parakeet-mlx`'s real `BaseParakeet` has no `transcribe_pcm` — only
    `transcribe(path)` (file) and `transcribe_stream()` (a context-manager
    object for continuous streaming, a bigger integration than this wave's
    5s-poll live-transcription loop needs). A short-lived WAV file over the
    same raw 16kHz mono 16-bit PCM bytes the live-tx thread already reads
    off `mic.wav` is the smallest correct bridge between the two."""
    with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as tmp:
        path = tmp.name
    try:
        with wave.open(path, "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(sample_rate)
            w.writeframes(pcm)
        yield path
    finally:
        os.unlink(path)


def _segments_from_aligned_result(result: object) -> list[Segment]:
    """`AlignedResult.sentences` — each an `AlignedSentence` with `start`/
    `end` in seconds (verified against the installed `parakeet-mlx`
    package). No per-sentence speaker hint in the real API — v1 has no
    diarization anyway, so `speaker_label_hint` stays the dataclass default
    (`None`)."""
    return [
        Segment(
            text=s.text,  # type: ignore[attr-defined]
            ts_start_ms=round(s.start * 1000),  # type: ignore[attr-defined]
            ts_end_ms=round(s.end * 1000),  # type: ignore[attr-defined]
        )
        for s in result.sentences  # type: ignore[attr-defined]
    ]


_OFFLINE_RECHECK_INTERVAL_S = 7 * 24 * 3600  # weekly
_CHECK_MARKER_NAME = ".mnemos_parakeet_last_check"


def _check_marker_path() -> Path:
    from huggingface_hub import constants  # type: ignore[import-not-found]

    return Path(constants.HF_HUB_CACHE) / _CHECK_MARKER_NAME


def _recently_checked() -> bool:
    try:
        age_s = time.time() - _check_marker_path().stat().st_mtime
        return age_s < _OFFLINE_RECHECK_INTERVAL_S
    except OSError:
        return False


def _touch_check_marker() -> None:
    try:
        marker = _check_marker_path()
        marker.parent.mkdir(parents=True, exist_ok=True)
        marker.touch()
    except OSError:
        pass


class _DownloadProgress:
    """Process-wide last-known byte progress for `PARAKEET_MODEL_ID`, plus the
    `notify(method, params)` closure `__main__.py` wires in (same shape
    `CaptureManager`/`LiveTranscriptionManager` already take). `set()` is
    called from whichever thread is actually driving the download — normally
    the background `warm_up()` thread at worker startup (started eagerly),
    but `ParakeetModel.get()` is a singleton behind one lock, so
    onboarding calling `download_status()`/triggering a second `get()` never
    causes a second download; it just observes or blocks on the same one."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._notify: Callable[[str, dict], None] | None = None
        self._received = 0
        self._total = 0
        self._done = False

    def configure(self, notify: Callable[[str, dict], None] | None) -> None:
        with self._lock:
            self._notify = notify

    def set(self, received: int, total: int, done: bool) -> None:
        with self._lock:
            self._received = received
            self._total = total
            self._done = done
            notify = self._notify
        if notify is not None:
            notify(
                "model_download_progress",
                {
                    "model_id": PARAKEET_MODEL_ID,
                    "received_bytes": received,
                    "total_bytes": total,
                    "done": done,
                },
            )

    def snapshot(self) -> dict[str, Any]:
        with self._lock:
            return {
                "model_id": PARAKEET_MODEL_ID,
                "received_bytes": self._received,
                "total_bytes": self._total,
                "done": self._done,
            }


_progress = _DownloadProgress()


@contextmanager
def _report_download_progress() -> Iterator[None]:
    """Best-effort: monkeypatches the `tqdm` class `huggingface_hub`'s
    downloader instantiates for its byte-progress bar, so real download
    progress can be forwarded to `_progress`/`notify` instead of only ever
    being printed to a terminal no one sees (Mnemos runs headless).

    PROVISIONAL — this hooks `huggingface_hub.utils.tqdm.tqdm`, the class
    `huggingface_hub`'s internal downloader constructs its progress bars
    from; it is not a formally stable public callback API. Both
    `parakeet-mlx` (Mac) and `onnx_asr` (Windows) route their model
    downloads through this same `huggingface_hub` machinery, confirmed by
    reading each library's source rather than assumed, so one hook serves
    both backends. Failure to patch degrades gracefully to no byte-level
    progress (the
    download still proceeds normally) rather than blocking or crashing the
    load."""
    try:
        # `importlib.import_module`, NOT `import huggingface_hub.utils.tqdm as
        # m`: `huggingface_hub/utils/__init__.py` does `from .tqdm import
        # tqdm`, so the *attribute* `huggingface_hub.utils.tqdm` is the tqdm
        # CLASS and shadows the submodule of the same name. The plain import
        # bound that class, `m.tqdm` raised `AttributeError: type object
        # 'tqdm' has no attribute 'tqdm'`, and the except below swallowed it —
        # so this hook silently never ran, and the onboarding bar sat at its
        # indeterminate placeholder through an entire 2.3GB download.
        # `import_module` returns the real module from `sys.modules`.
        hf_tqdm_mod = importlib.import_module("huggingface_hub.utils.tqdm")

        real_tqdm_cls = hf_tqdm_mod.tqdm

        # The Xet path runs TWO bars into this one hook (verified in
        # huggingface_hub/utils/_xet_progress_reporting.py): a
        # "reconstructing file" bar whose `total` is the real file size, and
        # a transfer bar whose `_update_transfer_bar` helper deliberately
        # inflates `bar.total` to `n * 1.25` on every update because network
        # bytes are unpredictable. Reporting both would make the UI's
        # denominator climb forever. So the first total seen is latched as
        # authoritative and any bar disagreeing with it is ignored — the
        # transfer bar disqualifies itself the moment it inflates, without
        # this code having to know either bar by name.
        latched: dict[str, int] = {"total": 0, "max_received": 0}

        class _ProgressTqdm(real_tqdm_cls):  # type: ignore[misc,valid-type]
            # Accumulate `n` ourselves rather than reading `self.n` after the
            # super() call. `tqdm.update()` opens with `if self.disable:
            # return` — it never touches `self.n` on a disabled bar, and the
            # bar IS disabled here because Mnemos runs headless with no TTY.
            # Measured against the real 2.3GB Xet download: 3078 update calls
            # arrived with the correct total and `self.n` pinned at 0 for
            # every one of them.
            _mnemos_received = 0

            def update(self, n: int = 1) -> Any:
                result = super().update(n)
                self._mnemos_received += int(n or 0)
                total = int(getattr(self, "total", None) or 0)
                if total <= 0:
                    return result  # no denominator yet — nothing worth showing
                if latched["total"] == 0:
                    latched["total"] = total
                if total != latched["total"]:
                    return result  # the inflating transfer bar
                received = min(self._mnemos_received, total)
                # Monotonic: both bars agree on the total early on, before the
                # transfer one inflates, so without this the reported figure
                # could tick backwards and the bar would visibly jump about.
                if received <= latched["max_received"]:
                    return result
                latched["max_received"] = received
                _progress.set(received, total, done=False)
                return result

        hf_tqdm_mod.tqdm = _ProgressTqdm  # type: ignore[misc]
        try:
            yield
        finally:
            hf_tqdm_mod.tqdm = real_tqdm_cls
    except Exception as exc:  # noqa: BLE001 — best-effort only, never blocks the real load
        # WARNING, not DEBUG. Release builds filter logs at `info`, so the
        # previous `debug` call meant a broken hook produced no evidence
        # anywhere — the download worked, the UI just lied about it forever.
        # This degrades gracefully by design, but it should degrade *loudly*.
        log.warning("parakeet.download_progress_hook.unavailable", error=str(exc))
        yield


def _load_parakeet_weights(from_pretrained: object, model_id: str) -> object:
    """`hf_hub_download` (what `from_pretrained` calls under the hood)
    natively honors `HF_HUB_OFFLINE=1` — skips its live HEAD-request
    freshness check entirely and reads straight from the local cache. No
    custom cache-detection needed, just the env var. We only let a real
    network check through once a week (tracked by a marker file's mtime
    next to the HF cache), so a stale local copy still gets refreshed
    eventually instead of a live call on every single worker start — and we
    never override an operator-set `HF_HUB_OFFLINE`, only fill in our own
    default when it's unset."""
    forced_offline = "HF_HUB_OFFLINE" not in os.environ and _recently_checked()
    if forced_offline:
        os.environ["HF_HUB_OFFLINE"] = "1"
    try:
        with _report_download_progress():
            try:
                model = from_pretrained(model_id)  # type: ignore[operator]
            except Exception:
                if not forced_offline:
                    raise
                # Cache vanished since the marker was written (e.g. cleared by
                # hand) — fall back to a real online load rather than a
                # permanent failure.
                os.environ.pop("HF_HUB_OFFLINE", None)
                model = from_pretrained(model_id)  # type: ignore[operator]
    finally:
        if forced_offline:
            os.environ.pop("HF_HUB_OFFLINE", None)
    _touch_check_marker()
    snap = _progress.snapshot()
    _progress.set(
        max(snap["received_bytes"], snap["total_bytes"]), snap["total_bytes"], done=True
    )
    return model


class _ParakeetMlxBackend:
    """`parakeet-mlx`'s real API, verified against the installed package.
    Model id is the v1 pin (matches Parakeet TDT 0.6B v3's expected input
    frontend).

    `transcribe(path)` with no `chunk_duration` runs the whole file as
    one non-causal attention pass — verified against this exact installed
    version that it throws `[metal::malloc]`/`kIOGPUCommandBufferCallback
    ErrorOutOfMemory` somewhere between 12 and 15 minutes of audio (a fixed
    Metal single-buffer ceiling, not a function of total system RAM, so no
    Mac is exempt). `chunk_duration`/`overlap_duration` are the library's own
    built-in answer for exactly this — verified clean on real 30-min and
    ~44-min meeting audio, well under a minute each. Both callers below pass
    them; `transcribe_pcm`'s inputs are always short (a live-tx poll window),
    so this is a no-op there today, kept only so neither call site can
    silently regress back to the crash if its usage ever changes.
    """

    MODEL_ID = "mlx-community/parakeet-tdt-0.6b-v3"
    CHUNK_DURATION_S = 120.0
    CHUNK_OVERLAP_S = 15.0

    def __init__(self) -> None:
        from parakeet_mlx import from_pretrained  # type: ignore[import-not-found]

        self._model = _load_parakeet_weights(from_pretrained, self.MODEL_ID)

    def transcribe_pcm(self, pcm: bytes, sample_rate: int) -> list[Segment]:
        with _pcm_as_temp_wav(pcm, sample_rate) as path:
            result = self._model.transcribe(
                path,
                chunk_duration=self.CHUNK_DURATION_S,
                overlap_duration=self.CHUNK_OVERLAP_S,
            )
        return _segments_from_aligned_result(result)

    def transcribe_file(
        self, path: str, on_progress: ProgressFn | None = None
    ) -> list[Segment]:
        # `parakeet-mlx` already chunks internally (we pass `chunk_duration`)
        # and already exposes a per-chunk hook — we simply were not passing
        # one. Its signature is `chunk_callback(processed_samples,
        # total_samples)`, which is exactly `ProgressFn`, so this needs no
        # adapter.
        #
        # Note it is NOT called when the audio is shorter than one chunk:
        # the library returns early in that case. Callers therefore cannot
        # treat "no progress yet" as "stalled" for short inputs — which is
        # why the caller reports completion itself rather than relying on a
        # final 100% tick from here.
        result = self._model.transcribe(
            path,
            chunk_duration=self.CHUNK_DURATION_S,
            overlap_duration=self.CHUNK_OVERLAP_S,
            chunk_callback=on_progress,
        )
        return _segments_from_aligned_result(result)


class _ParakeetOnnxBackend:
    """`onnx_asr` (https://pypi.org/project/onnx-asr/, MIT, verified against
    its real source at github.com/istupakov/onnx-asr rather than assumed) —
    a pure-Python, ONNX Runtime-backed ASR library with a published loader
    for this exact model: `onnx_asr.load_model("nemo-parakeet-tdt-0.6b-v3")`.
    Replaces the earlier `parakeet.cpp`-based stub, which depended on a
    package (`parakeet_cpp`) that was never a real, installable dependency.

    Two things confirmed by reading `onnx_asr`'s source before choosing it
    (not assumed from its README):
    - Model download genuinely goes through `huggingface_hub`'s
      `snapshot_download`/`hf_hub_download` — the exact functions
      `_report_download_progress` already monkeypatches for the Mac
      backend — so that hook is reused unmodified here via
      `_load_parakeet_weights`, giving Windows the same real byte-level
      download progress Mac gets, not a simulated one.
    - `onnxruntime` (its one real dependency) publishes genuine `win_arm64`
      wheels (checked against PyPI's release file listing) — unlike
      `pyaudiowpatch` (see the Windows parity audit's ARM64-wheel-gap
      finding), so this doesn't repeat that gap.

    `onnx_asr`'s own long-form/VAD recognition path exposes no per-chunk
    progress callback (also verified against its source) — so, exactly as
    this class's predecessor already specified before any of this was
    built, chunking happens at this layer: slice the WAV into fixed spans,
    call `.recognize()` per slice, report progress after each. This also
    sidesteps needing per-token timestamps from the library — each chunk's
    start/end is simply its own position in the file.
    """

    MODEL_ID = "nemo-parakeet-tdt-0.6b-v3"
    CHUNK_DURATION_S = 120.0

    def __init__(self) -> None:
        import onnx_asr  # type: ignore[import-not-found]

        self._model = _load_parakeet_weights(onnx_asr.load_model, self.MODEL_ID)

    def transcribe_pcm(self, pcm: bytes, sample_rate: int) -> list[Segment]:
        with _pcm_as_temp_wav(pcm, sample_rate) as path:
            text = self._model.recognize(path)  # type: ignore[attr-defined]
        duration_ms = round(len(pcm) / 2 / sample_rate * 1000)  # 16-bit mono PCM
        return [Segment(text=text, ts_start_ms=0, ts_end_ms=duration_ms)]

    def transcribe_file(
        self, path: str, on_progress: ProgressFn | None = None
    ) -> list[Segment]:
        segments: list[Segment] = []
        with wave.open(path, "rb") as src:
            channels = src.getnchannels()
            sampwidth = src.getsampwidth()
            framerate = src.getframerate()
            total_frames = src.getnframes()
            chunk_frames = max(1, round(self.CHUNK_DURATION_S * framerate))
            processed_frames = 0
            while processed_frames < total_frames:
                frames = src.readframes(chunk_frames)
                if not frames:
                    break
                n_read = len(frames) // (sampwidth * channels)
                start_ms = round(processed_frames / framerate * 1000)
                end_ms = round((processed_frames + n_read) / framerate * 1000)

                with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as tmp:
                    chunk_path = tmp.name
                try:
                    with wave.open(chunk_path, "wb") as dst:
                        dst.setnchannels(channels)
                        dst.setsampwidth(sampwidth)
                        dst.setframerate(framerate)
                        dst.writeframes(frames)
                    text: str = self._model.recognize(chunk_path)  # type: ignore[attr-defined]
                finally:
                    os.unlink(chunk_path)

                if text.strip():
                    segments.append(
                        Segment(text=text, ts_start_ms=start_ms, ts_end_ms=end_ms)
                    )
                processed_frames += n_read
                if on_progress is not None:
                    on_progress(processed_frames, total_frames)
        return segments


class ParakeetModel:
    """All three callers (live-transcription poll thread, `transcribe_final`
    job thread, and this class's own warm-up call) go through `_executor` —
    a single dedicated worker thread — so the backend is always loaded and
    called on the exact OS thread MLX bound its weight arrays to (see the
    module docstring). `ThreadPoolExecutor(max_workers=1)` also gives us the
    call serialization a lock would otherwise need to provide, for free."""

    _instance: "ParakeetModel | None" = None
    _instance_lock = threading.Lock()

    def __init__(self, backend: Backend | None = None) -> None:
        self._executor = ThreadPoolExecutor(
            max_workers=1, thread_name_prefix="parakeet-mlx"
        )
        load = (lambda: backend) if backend is not None else _load_backend
        self._backend: Backend = self._executor.submit(load).result()

    @classmethod
    def get(cls) -> "ParakeetModel":
        with cls._instance_lock:
            if cls._instance is None:
                cls._instance = cls()
            return cls._instance

    @classmethod
    def is_ready(cls) -> bool:
        """True once a `ParakeetModel` instance exists — `__init__` blocks
        (via `_executor`'s `.result()`) until the real backend weights are
        loaded, so `_instance` is only ever set *after* that finishes.
        Deliberately does not take `_instance_lock`: this is a cheap,
        non-blocking poll (`live_transcription.py`'s tick loop calls it every
        5s) that must never itself wait behind an in-flight `get()`/warm-up —
        a stale "not ready" read for one extra tick is harmless."""
        return cls._instance is not None

    @classmethod
    def warm_up(cls, notify: Callable[[str, dict], None] | None = None) -> None:
        """Loads the model once at worker startup, off the request path, so
        the first live-transcription tick or `transcribe_final` call isn't
        also paying model-load latency. Best-effort: a load failure is
        logged and swallowed so the rest of the worker (ping, health_check,
        capture) keeps working without Parakeet available.

        `notify` (onboarding's model-download screen) is wired here rather
        than at `download_status()`/onboarding's own RPC call because this —
        the eager warm-up thread started at worker boot — is what actually
        drives the real download in the common case (worker starts before
        onboarding's screen 4 mounts); a caller that only observes via
        `download_status()`/the notification topic still sees the same
        progress regardless of which thread triggered the load, since both
        funnel through the same `_progress` singleton."""
        if notify is not None:
            _progress.configure(notify)
        if cls.is_ready():
            # Already loaded (e.g. a second warm_up call, or onboarding's
            # own trigger racing an already-finished startup warm-up) —
            # report done immediately rather than staying silent forever.
            snap = _progress.snapshot()
            if not snap["done"]:
                _progress.set(
                    snap["received_bytes"] or 1, snap["total_bytes"] or 1, done=True
                )
            return
        try:
            cls.get()
            log.info("parakeet.warm_up.ok")
        except Exception as exc:  # noqa: BLE001 — best-effort warm-up, never fatal
            log.warning("parakeet.warm_up.failed", error=str(exc))
            _progress.set(0, 0, done=True)

    @classmethod
    def download_status(cls) -> dict[str, Any]:
        """Synchronous snapshot for a late subscriber (onboarding's screen
        mounting after warm-up already started, or already finished on a
        prior launch) — the notification topic alone would leave such a
        caller staring at 0% forever since it only ever emits on change."""
        if cls.is_ready():
            snap = _progress.snapshot()
            if not snap["done"]:
                return {
                    **snap,
                    "done": True,
                    "received_bytes": snap["total_bytes"] or snap["received_bytes"],
                }
            return snap
        return _progress.snapshot()

    def transcribe_pcm(
        self, pcm: bytes, *, sample_rate: int, stream_ctx: str
    ) -> list[Segment]:
        del (
            stream_ctx
        )  # reserved for future streaming-context reuse; unused by either backend today
        return self._executor.submit(
            self._backend.transcribe_pcm, pcm, sample_rate
        ).result()

    def transcribe_file(
        self, path: str, on_progress: ProgressFn | None = None
    ) -> list[Segment]:
        # `on_progress` fires on `_executor`'s thread, not the caller's. That
        # is fine for the only consumer (`job_progress.report`, which writes
        # to stdout behind `__main__`'s `write_lock`), but anything stateful
        # hooked up here must be thread-safe.
        return self._executor.submit(
            self._backend.transcribe_file, path, on_progress
        ).result()


def _model_download_status(params: dict[str, Any]) -> dict[str, Any]:
    del params  # v1 has exactly one downloadable model; a `model_id` param is unneeded until a second one exists
    return ParakeetModel.download_status()


# Fast-path method (mirrors `CAPTURE_METHODS`/`LIVE_TRANSCRIPTION_METHODS` in
# `__main__.py`) — a synchronous status poll must never queue behind a slow
# job. Note there's no `start_model_download` method: `warm_up()` already
# starts the real download eagerly at worker boot, so onboarding only ever
# needs to *observe* it (this poll, plus subscribing to
# the `model_download_progress` topic), never trigger it. `ParakeetModel.get()`
# from any other caller path (a live-transcription tick, `transcribe_final`)
# would also safely no-op into the same in-flight/cached singleton if one
# somehow raced ahead of warm-up, so there is no separate "trigger" RPC to add.
MODEL_METHODS: dict[str, Callable[[dict], dict]] = {
    "model_download_status": _model_download_status,
}
