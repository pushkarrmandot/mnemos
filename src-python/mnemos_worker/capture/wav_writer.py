"""Mono 16-bit PCM WAV writer that flushes on a fixed cadence and keeps the
RIFF/data chunk sizes patched after every flush (the mac sidecar does the
identical trick) — so a file killed mid-recording is still a valid, playable
WAV instead of one whose header claims zero frames.
"""

from __future__ import annotations

import os
import threading


class ChunkedWavWriter:
    def __init__(
        self,
        path: str,
        sample_rate: int = 16000,
        channels: int = 1,
        sample_width: int = 2,
    ) -> None:
        self._sample_rate = sample_rate
        self._channels = channels
        self._sample_width = sample_width
        self._lock = threading.Lock()
        self._pending = bytearray()
        self.bytes_written = 0
        self._closed = False
        # `FILE_SHARE_READ | FILE_SHARE_WRITE` equivalent on Windows is the
        # platform default for a plain open() — no AV-lock retry needed
        # here; retry-on-PermissionError lives in the caller, not in the
        # writer itself.
        self._fh = open(path, "wb")
        self._fh.write(self._header(0))
        self._fh.flush()

    def _header(self, data_bytes: int) -> bytes:
        byte_rate = self._sample_rate * self._channels * self._sample_width
        block_align = self._channels * self._sample_width
        return (
            b"RIFF"
            + (36 + data_bytes).to_bytes(4, "little")
            + b"WAVE"
            + b"fmt "
            + (16).to_bytes(4, "little")
            + (1).to_bytes(2, "little")
            + self._channels.to_bytes(2, "little")
            + self._sample_rate.to_bytes(4, "little")
            + byte_rate.to_bytes(4, "little")
            + block_align.to_bytes(2, "little")
            + (self._sample_width * 8).to_bytes(2, "little")
            + b"data"
            + data_bytes.to_bytes(4, "little")
        )

    def append(self, pcm: bytes) -> None:
        with self._lock:
            self._pending.extend(pcm)

    def flush(self) -> int:
        """Writes and fsyncs whatever is pending, then patches the header in
        place. Returns bytes flushed (0 if nothing was pending)."""
        with self._lock:
            if not self._pending:
                return 0
            chunk = bytes(self._pending)
            self._pending.clear()
            self._fh.seek(0, os.SEEK_END)
            self._fh.write(chunk)
            self.bytes_written += len(chunk)
            self._fh.seek(4)
            self._fh.write((36 + self.bytes_written).to_bytes(4, "little"))
            self._fh.seek(40)
            self._fh.write(self.bytes_written.to_bytes(4, "little"))
            self._fh.seek(0, os.SEEK_END)
            self._fh.flush()
            os.fsync(self._fh.fileno())
            return len(chunk)

    def close(self) -> None:
        self.flush()
        with self._lock:
            if not self._closed:
                self._fh.close()
                self._closed = True
