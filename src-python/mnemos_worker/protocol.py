"""LSP-style Content-Length JSON-RPC 2.0 framing (BACKEND_STANDARDS §2).

Binary-safe UTF-8 framing over any `BufferedIOBase`-like stream. The reader
and writer are plain functions, not classes, so `__main__.py` and the tests
can point them at real stdio or an in-memory `io.BytesIO`.
"""

from __future__ import annotations

import json
import threading
from typing import Any

MAX_BODY_BYTES = 16 * 1024 * 1024  # 16 MiB cap — bounds a runaway peer.


class FramingError(Exception):
    """Malformed Content-Length header or truncated body."""


def encode_message(payload: dict[str, Any]) -> bytes:
    body = json.dumps(payload).encode("utf-8")
    header = f"Content-Length: {len(body)}\r\n\r\n".encode("ascii")
    return header + body


def write_message(stream: Any, payload: dict[str, Any], lock: threading.Lock) -> None:
    """Writes one frame. Callers share `lock` so two threads never interleave
    frames on the same stream (mirrors the Rust writer's single-channel rule).
    """
    frame = encode_message(payload)
    with lock:
        stream.write(frame)
        stream.flush()


def read_message(stream: Any) -> dict[str, Any] | None:
    """Reads one frame. Returns `None` on clean EOF (peer closed stdin)."""
    headers: dict[str, str] = {}
    while True:
        line = stream.readline()
        if line == b"":
            return None  # EOF before any header — clean shutdown of the pipe.
        line = line.rstrip(b"\r\n")
        if line == b"":
            break  # blank line ends the header block
        if b":" not in line:
            raise FramingError(f"malformed header line: {line!r}")
        key, _, value = line.partition(b":")
        headers[key.strip().lower().decode("ascii", "replace")] = value.strip().decode(
            "ascii", "replace"
        )

    if "content-length" not in headers:
        raise FramingError("missing Content-Length header")
    try:
        length = int(headers["content-length"])
    except ValueError as exc:
        raise FramingError("non-integer Content-Length") from exc
    if length < 0 or length > MAX_BODY_BYTES:
        raise FramingError(f"Content-Length out of bounds: {length}")

    body = stream.read(length)
    if body is None or len(body) < length:
        raise FramingError("body shorter than declared Content-Length")

    try:
        return json.loads(body.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise FramingError(f"invalid JSON body: {exc}") from exc
