"""Single-active-session coordinator for Windows capture. Dispatched
directly from `__main__`'s read loop — never through the job executor (see
that module's comment) — so `start_capture`/`stop_capture` behave like
synchronous RPCs, not queued jobs.
"""

from __future__ import annotations

import threading
from typing import Callable

from mnemos_worker.capture.wasapi import NotifyFn, StreamPair, WindowsCapture, open_wasapi_streams


class CaptureAlreadyActive(RuntimeError):
    def __init__(self) -> None:
        super().__init__("a recording is already active")


class CaptureManager:
    def __init__(
        self,
        notify: NotifyFn,
        open_streams: Callable[[], StreamPair] = open_wasapi_streams,
    ) -> None:
        self._notify = notify
        self._open_streams = open_streams
        self._active: WindowsCapture | None = None
        self._lock = threading.Lock()

    def start_capture(self, params: dict) -> dict:
        with self._lock:
            if self._active is not None:
                raise CaptureAlreadyActive
            capture = WindowsCapture(
                conversation_id=params["conversation_id"],
                mic_path=params["mic_path"],
                system_path=params["system_path"],
                notify=self._notify,
                open_streams=self._open_streams,
            )
            started_at_ms = capture.start()
            self._active = capture
        return {"started_at_ms": started_at_ms}

    def stop_capture(self, params: dict) -> dict:
        # Idempotent — a second call for no/unknown active session is a
        # no-op returning zero counts, matching
        # `unsubscribe_live_transcript`'s idempotency for the analogous case.
        with self._lock:
            capture = self._active
            self._active = None
        if capture is None:
            return {"mic_bytes": 0, "system_bytes": 0}
        mic_bytes, system_bytes = capture.stop()
        return {"mic_bytes": mic_bytes, "system_bytes": system_bytes}

    def pause_capture(self, params: dict) -> dict:
        del params
        if self._active is not None:
            self._active.pause()
        return {}

    def resume_capture(self, params: dict) -> dict:
        del params
        if self._active is not None:
            self._active.resume()
        return {}


CAPTURE_METHODS: dict[str, Callable[[CaptureManager, dict], dict]] = {
    "start_capture": CaptureManager.start_capture,
    "stop_capture": CaptureManager.stop_capture,
    "pause_capture": CaptureManager.pause_capture,
    "resume_capture": CaptureManager.resume_capture,
}
