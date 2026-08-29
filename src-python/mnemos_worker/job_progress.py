"""Progress notifications from long-running job handlers.

Exists because a fixed request TTL cannot bound work whose duration scales
with input size. `transcribe_final` on a 33-minute recording took ~148s on
an M-series Mac against a 60s TTL: Rust gave up, marked the pipeline failed,
and routed the user to a retry page — while the worker went on to finish
successfully and write a perfectly good `transcript.json` that was then
ignored. Picking a bigger constant only moves the cliff, and moves it
differently on every machine (an 8 GB laptop and an M3 Max are not within a
small factor of each other).

So the deadline stops timing *the work* and starts timing *silence*. A
handler reports progress as it goes; each report pushes the request's
deadline forward. A slow machine simply reports further apart and is never
penalised for it, while a genuinely wedged or dead worker still fails fast
because nothing arrives at all. No hardware assumptions, nothing to tune
per platform.

Wired the same way `rpc_client.configure_rpc_client` is: a module-level
notifier configured once at startup, so handlers registered via
`@method(...)` — which only ever receive `params` — can emit without
threading a notify callable through every signature.
"""

from __future__ import annotations

from typing import Any, Callable

Notifier = Callable[[str, dict[str, Any]], None]

TOPIC = "job_progress"

_notify: Notifier | None = None
# The JSON-RPC request id of the job currently executing. Set by
# `JobExecutor` around each handler call. A plain module global is safe here
# precisely because the job executor is single-threaded by design (models
# are not thread-safe — see `job_executor`'s docstring); if that ever gains
# real concurrency this must become a `contextvars.ContextVar`, or reports
# will be attributed to the wrong request.
_current_request_id: str | None = None


def configure_progress_notifier(notify: Notifier) -> None:
    global _notify
    _notify = notify


def set_current_request_id(request_id: str | None) -> None:
    global _current_request_id
    _current_request_id = request_id


def report(kind: str, fraction: float, **extra: Any) -> None:
    """Emit one progress tick. Best-effort and never raises: this runs from
    inside inference loops, and a broken notifier must not be able to fail a
    transcription that is otherwise succeeding."""
    if _notify is None or _current_request_id is None:
        return
    payload: dict[str, Any] = {
        "request_id": _current_request_id,
        "kind": kind,
        # Clamped because callers derive it from sample counts, and the
        # final chunk can overshoot the total by the overlap window.
        "fraction": max(0.0, min(1.0, fraction)),
    }
    payload.update(extra)
    try:
        _notify(TOPIC, payload)
    except Exception:  # noqa: BLE001 — progress is telemetry, never load-bearing
        pass
