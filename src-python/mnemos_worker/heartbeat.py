"""Emits `{"method": "heartbeat"}` on stdout every `interval_s` (30s in
production; the health task on the Rust side treats a 45s silence as stuck —
BACKEND_STANDARDS §2 "Health checks").
"""

from __future__ import annotations

import sys
import threading

from mnemos_worker.protocol import write_message


class Heartbeat:
    def __init__(
        self, stream=sys.stdout.buffer, lock: threading.Lock | None = None, interval_s: float = 30.0
    ) -> None:
        self._stream = stream
        self._lock = lock or threading.Lock()
        self._interval_s = interval_s
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, name="heartbeat", daemon=True)

    def start(self) -> None:
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()

    def _run(self) -> None:
        while not self._stop.wait(self._interval_s):
            write_message(self._stream, {"jsonrpc": "2.0", "method": "heartbeat"}, self._lock)
