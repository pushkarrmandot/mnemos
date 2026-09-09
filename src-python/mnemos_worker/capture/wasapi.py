"""`WindowsCapture` — one thread pumping mic + WASAPI-loopback streams for
one recording session. Spawned by the `start_capture` handler, joined by
`stop_capture`; never touches the job executor — capture is not a queued
job, it needs to start/stop synchronously from the caller's point of view.

`open_streams` is dependency-injected so this class is unit-testable with
fake in-memory streams on any OS; the real factory
(`open_wasapi_streams`, below) requires `pyaudiowpatch` and only works on
Windows — it is never imported at module load time so importing this
module elsewhere (tests, macOS CI) doesn't require the Windows-only dep.
"""

from __future__ import annotations

import threading
import time
from dataclasses import dataclass
from typing import Callable, Protocol

from mnemos_worker.capture.errors import classify_wasapi_error
from mnemos_worker.capture.level import rms_dbfs
from mnemos_worker.capture.resample import resample_to_16k_mono
from mnemos_worker.capture.wav_writer import ChunkedWavWriter

LEVEL_INTERVAL_S = 0.1
CHUNK_INTERVAL_S = 0.5
READ_FRAMES = 1600  # ~100ms at 16kHz; scaled to the source rate internally
# TODO(Windows parity): mic and loopback are read sequentially on one thread
# below, and READ_FRAMES is passed straight to devices running at their
# native rate (typically 48kHz, not 16kHz), so this is likely closer to
# ~33ms per read than the "~100ms" the constant's name implies. Both need
# real Windows hardware to measure and fix correctly — not guessed here.

# Mirrors the mac sidecar's silence-detection threshold/duration exactly
# (`noSignalDbThreshold`/`noSignalWarningSeconds`,
# swift/mnemos-audio/Sources/mnemos-audio/main.swift:24-25) so a muted/
# wrong-device mic warns identically on both platforms.
NO_SIGNAL_DB_THRESHOLD = -60.0
NO_SIGNAL_WARNING_SECONDS = 30.0


class AudioStream(Protocol):
    sample_rate: int
    channels: int

    def read(self, num_frames: int) -> bytes: ...  # raw PCM16, native format
    def close(self) -> None: ...


@dataclass
class StreamPair:
    mic: AudioStream
    system: AudioStream


NotifyFn = Callable[[str, dict], None]


class WindowsCapture:
    def __init__(
        self,
        conversation_id: str,
        mic_path: str,
        system_path: str,
        notify: NotifyFn,
        open_streams: Callable[[], StreamPair],
    ) -> None:
        self._conv = conversation_id
        self._mic_writer = ChunkedWavWriter(mic_path)
        self._system_writer = ChunkedWavWriter(system_path)
        self._notify = notify
        self._open_streams = open_streams
        self._should_exit = threading.Event()
        self._paused = threading.Event()
        self._started_evt = threading.Event()
        self._started_at_ms: int | None = None
        self._thread: threading.Thread | None = None
        # Set by `_run` when `_open_streams()` fails, so `_started_evt` never
        # signals "success" for a recording that captured nothing —
        # `start()` re-raises this instead of returning normally. See the
        # `_run` and `start` methods below for where this is set/consumed.
        self._start_error: Exception | None = None
        # Running silence duration/warned-once state for the mic stream,
        # checked every read (see `_track_mic_silence`).
        self._mic_silence_start: float | None = None
        self._mic_warned = False

    def start(self, timeout: float = 5.0) -> int:
        self._thread = threading.Thread(
            target=self._run, name=f"wasapi-{self._conv[:8]}", daemon=True
        )
        self._thread.start()
        if not self._started_evt.wait(timeout=timeout):
            raise TimeoutError("capture thread did not signal start in time")
        if self._start_error is not None:
            # See `_start_error`'s definition in `__init__` above.
            raise self._start_error
        assert self._started_at_ms is not None
        return self._started_at_ms

    def pause(self) -> None:
        self._paused.set()
        self._emit("paused")

    def resume(self) -> None:
        self._paused.clear()
        self._emit("resumed")

    def stop(self, timeout: float = 5.0) -> tuple[int, int]:
        self._should_exit.set()
        if self._thread is not None:
            self._thread.join(timeout=timeout)
        return self._mic_writer.bytes_written, self._system_writer.bytes_written

    def _emit(self, kind: str, **extra: object) -> None:
        payload = {"conversation_id": self._conv, "kind": kind, **extra}
        self._notify("capture_event", payload)

    def _run(self) -> None:
        try:
            streams = self._open_streams()
        except OSError as exc:
            # Sets `_start_error` (see `__init__`) so `start()` re-raises it
            # and `CaptureManager.start_capture` returns a real JSON-RPC
            # error rather than reporting a recording that captures nothing.
            self._emit("error", error_kind=classify_wasapi_error(exc), message=str(exc))
            self._start_error = exc
            self._started_evt.set()
            self._mic_writer.close()
            self._system_writer.close()
            return

        self._started_at_ms = int(time.time() * 1000)
        self._emit("started", started_at_ms=self._started_at_ms)
        self._started_evt.set()

        last_level = 0.0
        last_chunk = 0.0
        try:
            while not self._should_exit.is_set():
                # Always read (and resample) both streams every iteration,
                # paused or not — this is what actually drains the WASAPI
                # ring buffer. Only *whether we act on the frames* depends on
                # pause state, mirroring the mac sidecar's Pause (taps/streams
                # keep running, buffers are dropped;
                # swift/mnemos-audio/Sources/mnemos-audio/main.swift:377,434,
                # 554-557). Reading unconditionally here is what keeps the
                # ring buffer from overrunning while paused — if this loop
                # ever stopped reading during a pause, resume would write
                # back a burst of stale audio
                # (`exception_on_overflow=False`, see `_PyAudioStreamAdapter`
                # below).
                mic_pcm = self._read_and_resample(streams.mic)
                system_pcm = self._read_and_resample(streams.system)

                if self._paused.is_set():
                    continue

                mic_db = rms_dbfs(mic_pcm)
                system_db = rms_dbfs(system_pcm)
                self._track_mic_silence(mic_db)

                self._mic_writer.append(mic_pcm)
                self._system_writer.append(system_pcm)

                now = time.monotonic()
                if now - last_level >= LEVEL_INTERVAL_S:
                    last_level = now
                    self._emit(
                        "level",
                        mic_db=mic_db,
                        system_db=system_db,
                    )
                if now - last_chunk >= CHUNK_INTERVAL_S:
                    last_chunk = now
                    if self._mic_writer.flush():
                        self._emit(
                            "chunk",
                            source="mic",
                            bytes_written=self._mic_writer.bytes_written,
                        )
                    if self._system_writer.flush():
                        self._emit(
                            "chunk",
                            source="system",
                            bytes_written=self._system_writer.bytes_written,
                        )
        except OSError as exc:
            self._emit("error", error_kind=classify_wasapi_error(exc), message=str(exc))
        finally:
            for s in (streams.mic, streams.system):
                try:
                    s.close()
                except Exception:  # noqa: BLE001 — best-effort cleanup
                    pass
            self._mic_writer.close()
            self._system_writer.close()
            self._emit(
                "stopped",
                mic_bytes=self._mic_writer.bytes_written,
                system_bytes=self._system_writer.bytes_written,
            )

    def _read_and_resample(self, stream: AudioStream) -> bytes:
        raw = stream.read(READ_FRAMES)
        return resample_to_16k_mono(raw, stream.sample_rate, stream.channels)

    def _track_mic_silence(self, mic_db: float) -> None:
        """Mirrors the mac sidecar's `trackSilence`
        (swift/mnemos-audio/Sources/mnemos-audio/main.swift:488-499) —
        tracks how long the mic has been below `NO_SIGNAL_DB_THRESHOLD` and
        emits a `warning` event (once, until the level recovers) after
        `NO_SIGNAL_WARNING_SECONDS`. Uses `warning_kind` (not `kind`,
        already the event's own top-level discriminant) — the field name
        `capture::capture_event_from_notification` (Rust side) expects.
        """
        if mic_db < NO_SIGNAL_DB_THRESHOLD:
            if self._mic_silence_start is None:
                self._mic_silence_start = time.monotonic()
            elapsed = time.monotonic() - self._mic_silence_start
            if not self._mic_warned and elapsed >= NO_SIGNAL_WARNING_SECONDS:
                self._mic_warned = True
                self._emit(
                    "warning",
                    warning_kind="no_mic_signal",
                    message=f"no signal for {int(NO_SIGNAL_WARNING_SECONDS)}s",
                )
        else:
            self._mic_silence_start = None
            self._mic_warned = False


def open_wasapi_streams() -> StreamPair:
    """Production stream factory: regular WASAPI on the default input for
    mic, WASAPI loopback on the default output device for system audio.
    Imports `pyaudiowpatch` lazily — this function is only ever called on
    Windows.
    """
    import pyaudiowpatch as pyaudio  # noqa: PLC0415 — intentionally lazy, Windows-only

    p = pyaudio.PyAudio()

    mic_info = p.get_default_input_device_info()
    mic_stream = p.open(
        format=pyaudio.paInt16,
        channels=int(mic_info["maxInputChannels"]) or 1,
        rate=int(mic_info["defaultSampleRate"]),
        input=True,
        input_device_index=int(mic_info["index"]),
        frames_per_buffer=READ_FRAMES,
    )

    loopback_info = p.get_default_wasapi_loopback()
    system_stream = p.open(
        format=pyaudio.paInt16,
        channels=int(loopback_info["maxInputChannels"]) or 2,
        rate=int(loopback_info["defaultSampleRate"]),
        input=True,
        input_device_index=int(loopback_info["index"]),
        frames_per_buffer=READ_FRAMES,
    )

    return StreamPair(
        mic=_PyAudioStreamAdapter(mic_stream, int(mic_info["defaultSampleRate"]), int(mic_info["maxInputChannels"]) or 1),
        system=_PyAudioStreamAdapter(
            system_stream,
            int(loopback_info["defaultSampleRate"]),
            int(loopback_info["maxInputChannels"]) or 2,
        ),
    )


class _PyAudioStreamAdapter:
    """Adapts a `pyaudiowpatch` stream to the `AudioStream` protocol
    (`read(n) -> bytes`, `sample_rate`, `channels`, `close()`)."""

    def __init__(self, stream: object, sample_rate: int, channels: int) -> None:
        self._stream = stream
        self.sample_rate = sample_rate
        self.channels = channels

    def read(self, num_frames: int) -> bytes:
        return self._stream.read(num_frames, exception_on_overflow=False)  # type: ignore[attr-defined]

    def close(self) -> None:
        self._stream.stop_stream()  # type: ignore[attr-defined]
        self._stream.close()  # type: ignore[attr-defined]
