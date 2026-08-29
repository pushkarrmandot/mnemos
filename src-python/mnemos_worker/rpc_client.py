"""Reverse-RPC caller (LLD-02 §7, LLD-05 §4.2/§5.2): lets a job-executor
thread call back into Rust (`run_agent_extraction`) and block for the reply,
while the main stdin-read loop keeps servicing everything else on the same
stdio transport.

This is genuinely new plumbing, not previously built: earlier waves only
built the *inbound* direction (Rust/`__main__.py` answering worker-served
requests). W11's `extract_memory`/`refresh_project_memory` job handlers are
the first callers that need the worker to *initiate* a request, so this
module + the two-line hook in `__main__.py`'s read loop is what makes that
possible. IDs are `rpc-<n>` strings — a disjoint namespace from Rust's own
outbound-request ids (plain integers), so no collision is possible on either
side.
"""

from __future__ import annotations

import itertools
import queue
import threading
from typing import Any, Callable


class ReverseRpcError(Exception):
    """Rust replied with a JSON-RPC error object, or no reply arrived in time."""

    def __init__(self, code: int, message: str, data: dict[str, Any] | None = None) -> None:
        super().__init__(message)
        self.code = code
        self.data = data or {}


class ReverseRpcClient:
    def __init__(self, send: Callable[[dict[str, Any]], None]) -> None:
        self._send = send
        self._counter = itertools.count(1)
        self._pending: dict[str, queue.Queue] = {}
        self._lock = threading.Lock()

    def call(self, method: str, params: dict[str, Any], timeout_s: float) -> Any:
        """Blocks the calling thread until Rust replies or `timeout_s`
        elapses. Safe to call from the job-executor thread — the main read
        loop remains free to read the reply frame and route it back via
        `resolve()`.
        """
        req_id = f"rpc-{next(self._counter)}"
        reply: queue.Queue = queue.Queue(maxsize=1)
        with self._lock:
            self._pending[req_id] = reply
        self._send({"jsonrpc": "2.0", "id": req_id, "method": method, "params": params})
        try:
            kind, payload = reply.get(timeout=timeout_s)
        except queue.Empty:
            with self._lock:
                self._pending.pop(req_id, None)
            raise ReverseRpcError(-32020, f"{method}: no reply within {timeout_s}s") from None
        if kind == "error":
            raise ReverseRpcError(
                payload.get("code", -32000),
                payload.get("message", "reverse rpc error"),
                payload.get("data"),
            )
        return payload

    def resolve(self, req_id: str, result: Any, error: dict[str, Any] | None) -> bool:
        """Called from the main read loop for every inbound frame that has an
        `id` but no `method` (a reply to one of our outbound calls, never a
        request the worker must dispatch). Returns `False` for a stale/
        unknown id (already timed out, or a bug) so the caller can log it.
        """
        with self._lock:
            pending = self._pending.pop(req_id, None)
        if pending is None:
            return False
        pending.put(("error", error) if error is not None else ("result", result))
        return True


# Process-wide default client (LLD-05 §4.2/§5.2's `ctx.reverse_rpc`). A real
# `JobCtx` plumbed through every `@method` handler's signature would be the
# textbook shape, but every existing job handler (`ping`, `transcribe_final`)
# takes a plain `dict[str, Any] -> dict[str, Any]` — changing that signature
# for every handler just so the two new jobs can reach a client wasn't worth
# it. `__main__.py` calls `configure()` once at startup; job modules import
# `call_reverse_rpc` directly.
_default: ReverseRpcClient | None = None


def configure(client: ReverseRpcClient) -> None:
    global _default
    _default = client


def call_reverse_rpc(method: str, params: dict[str, Any], timeout_s: float) -> Any:
    if _default is None:
        raise RuntimeError("rpc_client.configure() was never called")
    return _default.call(method, params, timeout_s)
