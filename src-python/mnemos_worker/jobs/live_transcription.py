"""Live-transcription poll loop (LLD-03 §5.1). One `LiveTranscriptionThread`
per active recording; v1 asserts at most one recording in flight (matches
Rust's `active: Mutex<Option<ActiveSession>>`). `LiveTranscriptionManager`
dispatches `subscribe_live_transcript`/`unsubscribe_live_transcript` directly
from `__main__`'s read loop — same reasoning as `CaptureManager`
(`mnemos_worker/capture/manager.py`): these are fast, Ack-only operations
that must never queue behind a slow `transcribe_final` job.
"""

from __future__ import annotations

import os
import threading
from typing import Any, Callable

from mnemos_worker.logging_config import get_logger
from mnemos_worker.models.transcription import ParakeetModel, Segment

log = get_logger(component="live-transcription")

# `ChunkedWavWriter` always writes a fixed 44-byte RIFF/fmt/data header (no
# extra chunks) — the cursor starts past it, per LLD-03 §5.1's
# `_wav_data_offset`.
WAV_HEADER_BYTES = 44
BYTES_PER_SEC = 16_000 * 2  # 16 kHz mono 16-bit PCM
#: Live transcription reads the mic channel only, so every segment it emits is
#: the user. Matches `merge_transcripts`' mic label so the live pane and the
#: final transcript agree.
MIC_SPEAKER_LABEL = "You"
POLL_SEC = 5.0
MIN_AUDIO_SEC = 2.0

NotifyFn = Callable[[str, dict[str, Any]], None]
TranscribePcmFn = Callable[..., list[Segment]]


class LiveTranscriptionThread(threading.Thread):
    def __init__(
        self,
        conversation_id: str,
        mic_path: str,
        notify: NotifyFn,
        transcribe_pcm: TranscribePcmFn | None = None,
    ) -> None:
        super().__init__(name=f"live-tx-{conversation_id[:8]}", daemon=True)
        self._conv = conversation_id
        self._mic_path = mic_path
        self._notify = notify
        # Resolved lazily on first `_tick()` (this thread), not here: this
        # constructor runs synchronously inside `subscribe_live_transcript`,
        # which `__main__`'s single-threaded read loop dispatches directly
        # (module docstring above) so it "must never queue behind a slow
        # job." `ParakeetModel.get()` blocks on `_instance_lock` until
        # warm-up finishes loading real weights (W9 2026-08-22 fix) — doing
        # that here would block every other RPC, including the
        # `start_recording` caller waiting on this very subscribe call, for
        # however long model load takes (confirmed live: 30-40s).
        self._transcribe_pcm = transcribe_pcm
        self._cursor = WAV_HEADER_BYTES
        self._should_exit = threading.Event()
        self._joined = threading.Event()
        # F (debug-session patch): logged at most once per thread — see the
        # warm-up check in `_tick()`.
        self._warned_not_ready = False
        # W17b: last readiness state actually notified to the frontend —
        # `None` until the first tick, so the first check always fires (even
        # if that first observation is already "ready", the UI still needs
        # to be told at least once). Distinct from `_warned_not_ready` above,
        # which only gates the *log* line — this gates the user-visible
        # `live_transcription_warmup` notification.
        self._last_ready_notified: bool | None = None

    def stop_and_drain(self, timeout: float = 10.0) -> None:
        """Idempotent-from-the-caller's-view: safe to call once per thread.
        Sets `should_exit`, waits for a final drain tick to run, joins."""
        self._should_exit.set()
        self._joined.wait(timeout)

    def run(self) -> None:
        try:
            while True:
                exiting = self._should_exit.is_set()
                self._tick()
                if exiting:
                    break
                self._should_exit.wait(POLL_SEC)
        finally:
            self._joined.set()

    def _notify_warmup_state_if_changed(self) -> None:
        """W17b: tells the frontend when live transcription is blocked on
        model warm-up, and when it stops being blocked — the gap this closes
        is real: before this, `is_ready()` being False just made `_tick()`
        return early with a log line nobody sees, so a user who hit Record
        before warm-up finished (first-ever launch, or right after the
        worker restarts) saw a silent "Listening…" for however long warm-up
        takes, with nothing distinguishing "no one has spoken yet" from
        "the model isn't loaded yet". Checked unconditionally at the top of
        every tick — independent of whether there's new audio to transcribe
        — so it fires on the very first tick, before any audio-length gating.
        """
        ready = self._transcribe_pcm is not None or ParakeetModel.is_ready()
        if ready == self._last_ready_notified:
            return
        self._last_ready_notified = ready
        self._notify(
            "live_transcription_warmup",
            {"conversation_id": self._conv, "ready": ready},
        )

    def _tick(self) -> None:
        self._notify_warmup_state_if_changed()
        try:
            size = os.stat(self._mic_path).st_size
        except OSError as exc:
            log.info("live_transcription.tick_stat_failed", conv=self._conv, error=str(exc))
            return  # file not written yet, or already torn down
        new_bytes = size - self._cursor
        if new_bytes <= 0:
            log.info("live_transcription.tick_no_new_bytes", conv=self._conv, size=size, cursor=self._cursor)
            return
        # Final tick (should_exit already set) always drains whatever is
        # left, regardless of the 2s minimum.
        if not self._should_exit.is_set() and new_bytes < BYTES_PER_SEC * MIN_AUDIO_SEC:
            log.info(
                "live_transcription.tick_below_min_audio",
                conv=self._conv,
                new_bytes=new_bytes,
                min_bytes=int(BYTES_PER_SEC * MIN_AUDIO_SEC),
            )
            return

        # Warm-up race: `ParakeetModel.get()` blocks synchronously loading
        # real model weights (confirmed 30-40s) the first time anything
        # calls it. If a recording starts before the background `warm_up()`
        # (kicked off in `__main__.py`) finishes, resolving `transcribe_pcm`
        # here would silently block this thread — and therefore the whole
        # live-transcript stream — for the rest of that load, with no signal
        # to the UI at all. Skip this tick instead, *before* touching the
        # cursor, so no audio is lost: the next tick re-reads this same span
        # (plus whatever's accumulated since) once warm-up has actually
        # finished. Only real production use (`transcribe_pcm=None`) can hit
        # this — tests always inject a fake and are exempt.
        if self._transcribe_pcm is None and not ParakeetModel.is_ready():
            if not self._warned_not_ready:
                log.info("live_transcription.waiting_for_model_warmup")
                self._warned_not_ready = True
            return

        pcm = self._read_pcm(self._cursor, new_bytes)
        base_cursor = self._cursor
        self._cursor += new_bytes

        if self._transcribe_pcm is None:
            self._transcribe_pcm = ParakeetModel.get().transcribe_pcm
        try:
            segments = self._transcribe_pcm(pcm, sample_rate=16000, stream_ctx=self._conv)
        except Exception as exc:  # noqa: BLE001 — becomes a job_error notification, never crashes the thread
            log.exception("live_transcription.tick_failed", conv=self._conv)
            self._notify(
                "job_error",
                {
                    "error_class": "live_transcription",
                    "message": str(exc),
                    "conversation_id": self._conv,
                },
            )
            return

        log.info(
            "live_transcription.tick_transcribed",
            conv=self._conv,
            new_bytes=new_bytes,
            segments=len(segments),
        )
        base_ms = self._cursor_to_ms(base_cursor)
        for seg in segments:
            self._notify(
                "live_transcript_chunk",
                {
                    "conversation_id": self._conv,
                    "chunk": {
                        # W17c: this job only ever reads `mic.wav`, so every
                        # live segment is the user by construction — the same
                        # rule `merge_transcripts` applies to the mic channel
                        # in the final pass. Parakeet exposes no speaker hint
                        # of its own, so this was always `None`, and the live
                        # pane rendered every turn as an anonymous "…" while
                        # the final transcript correctly said "You".
                        "speaker_label_hint": seg.speaker_label_hint or MIC_SPEAKER_LABEL,
                        "text": seg.text,
                        "ts_start_ms": base_ms + seg.ts_start_ms,
                        "ts_end_ms": base_ms + seg.ts_end_ms,
                    },
                },
            )

    def _read_pcm(self, offset: int, length: int) -> bytes:
        with open(self._mic_path, "rb") as f:
            f.seek(offset)
            return f.read(length)

    @staticmethod
    def _cursor_to_ms(offset_bytes: int) -> int:
        return int((offset_bytes - WAV_HEADER_BYTES) * 1000 / BYTES_PER_SEC)


class LiveTranscriptionManager:
    """Single-active-thread-per-conversation coordinator, mirroring
    `CaptureManager`'s shape. `subscribe`/`unsubscribe` are idempotent: a
    repeat subscribe for an already-active conversation, or an unsubscribe
    for an unknown one, is a no-op returning `{}` (LLD-03 §3.2)."""

    def __init__(self, notify: NotifyFn, transcribe_pcm: TranscribePcmFn | None = None) -> None:
        self._notify = notify
        # Injected only by tests, so a fake model can stand in without
        # touching `ParakeetModel`'s real (mlx/parakeet.cpp) backend load.
        # Production always passes `None`, which lazily resolves to
        # `ParakeetModel.get()` per thread — already warmed at worker
        # startup (`ParakeetModel.warm_up()` in `__main__.py`).
        self._transcribe_pcm = transcribe_pcm
        self._active: dict[str, LiveTranscriptionThread] = {}
        self._lock = threading.Lock()

    def subscribe_live_transcript(self, params: dict[str, Any]) -> dict[str, Any]:
        conversation_id = params["conversation_id"]
        mic_path = params["mic_path"]
        with self._lock:
            if conversation_id in self._active:
                return {}
            thread = LiveTranscriptionThread(
                conversation_id, mic_path, self._notify, transcribe_pcm=self._transcribe_pcm
            )
            self._active[conversation_id] = thread
        thread.start()
        log.info("live_transcription.thread_started", conv=conversation_id, mic_path=mic_path)
        return {}

    def unsubscribe_live_transcript(self, params: dict[str, Any]) -> dict[str, Any]:
        conversation_id = params["conversation_id"]
        with self._lock:
            thread = self._active.pop(conversation_id, None)
        if thread is None:
            return {}
        thread.stop_and_drain(timeout=10.0)
        return {}

    def stop_all(self, timeout: float = 10.0) -> None:
        """Worker shutdown: drain every still-active thread. Not part of the
        LLD's RPC surface — just cleanup so a `shutdown` mid-recording
        doesn't leave a daemon thread mid-tick."""
        with self._lock:
            threads = list(self._active.values())
            self._active.clear()
        for thread in threads:
            thread.stop_and_drain(timeout=timeout)


LIVE_TRANSCRIPTION_METHODS: dict[str, Callable[[LiveTranscriptionManager, dict], dict]] = {
    "subscribe_live_transcript": LiveTranscriptionManager.subscribe_live_transcript,
    "unsubscribe_live_transcript": LiveTranscriptionManager.unsubscribe_live_transcript,
}
