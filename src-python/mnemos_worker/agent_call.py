"""Shared one-call-plus-one-retry policy for both memory jobs:
call `run_agent_extraction`, validate the JSON, and on a schema mismatch
retry exactly once with the validator's error appended as a nudge. A true
infra failure (agent timeout, CLI missing/not-logged-in/stream-corrupt)
never retries — it fails the job immediately with a code
`job_executor.py`/Rust's `map_json_rpc_error` can classify correctly.
"""

from __future__ import annotations

import threading
from typing import Any, Callable

from mnemos_worker import job_progress
from mnemos_worker.errors import (
    AGENT_BLOCKED,
    CANCELLED,
    RUNNER_SCHEMA_FAILURE,
    WORKER_UNAVAILABLE,
    WorkerJobError,
)
from mnemos_worker.extraction_schema import SchemaValidationError
from mnemos_worker.prompts import RETRY_NUDGE
from mnemos_worker.rpc_client import ReverseRpcError, call_reverse_rpc

_NO_RETRY_KINDS = {"cli_missing", "cli_not_logged_in", "stream_corrupt"}

# How often to tell the host we are still here while blocked on the agent.
# Only needs to be comfortably under the host's silence window for these jobs
# (`ExtractMemory::ttl`, 60s); frequent enough to survive one missed tick.
_ALIVE_TICK_S = 10.0


class _AliveTicker:
    """Reports progress on a daemon thread for the duration of an agent call.

    `call_reverse_rpc` blocks the job-executor thread on a `queue.Queue`, so
    the thread doing the work cannot report anything while it waits — and the
    host's request deadline bounds *silence* (`job_progress`). Without this,
    an agent call longer than that window is swept mid-flight and the job
    fails while the agent is still answering.

    Ticking on a timer rather than on token arrival is deliberate. Token-driven
    liveness sounds truer, but a corporate gateway that buffers the whole
    response and returns it in one piece emits no tokens at all until it is
    done (observed: `first_token_ms` within 10ms of a 100s `elapsed_ms`) —
    which is exactly the case that was failing. A timer is indifferent to that.

    This does not weaken the real bound on the agent: `call_reverse_rpc`'s own
    `timeout_s` still caps the call, and a genuinely dead worker stops writing
    frames entirely, so the host still fails it fast.

    Writing frames from a second thread is already how this worker works — the
    `Heartbeat` thread does it under the same `write_lock`.
    """

    def __init__(self, kind: str) -> None:
        self._kind = kind
        self._done = threading.Event()
        self._thread: threading.Thread | None = None

    def __enter__(self) -> "_AliveTicker":
        self._thread = threading.Thread(target=self._run, daemon=True, name="agent-alive")
        self._thread.start()
        return self

    def __exit__(self, *_exc: object) -> None:
        self._done.set()
        if self._thread is not None:
            self._thread.join(timeout=1.0)

    def _run(self) -> None:
        # `wait` returning True means __exit__ fired; only tick on a timeout.
        while not self._done.wait(_ALIVE_TICK_S):
            # `report` is best-effort and never raises (see `job_progress`),
            # so a broken notifier cannot fail an extraction that is working.
            job_progress.report(self._kind, 0.0, waiting_on="agent")


def _agent_call(prompt: str, system_prompt: str, timeout_ms: int) -> Any:
    timeout_s = timeout_ms / 1000 + 10
    try:
        with _AliveTicker("extract"):
            return call_reverse_rpc(
                "run_agent_extraction",
                {"prompt": prompt, "system_prompt": system_prompt, "timeout_ms": timeout_ms},
                timeout_s,
            )
    except ReverseRpcError as exc:
        if exc.code == -32020:
            raise WorkerJobError(CANCELLED, f"agent timeout: {exc}") from exc
        kind = exc.data.get("kind") if isinstance(exc.data, dict) else None
        # A usage-limit refusal: never retried (the nudge below would spend a
        # second call against an exhausted quota), and the message is passed
        # through *unwrapped* because Rust rebuilds it into
        # `AppError::RunnerBlocked`, whose Display is shown to the user
        # verbatim — prefixing it here would leak worker-internal wording
        # into the failure banner.
        if exc.code == AGENT_BLOCKED or kind == "agent_blocked":
            raise WorkerJobError(AGENT_BLOCKED, str(exc)) from exc
        if exc.code == -32000 and kind in _NO_RETRY_KINDS:
            raise WorkerJobError(WORKER_UNAVAILABLE, f"agent unavailable ({kind}): {exc}") from exc
        # `agent_json_parse` (-32603 — the agent's text wasn't even JSON) or
        # anything unexpected: treat identically to a schema mismatch so the
        # one-retry policy below covers it too.
        raise SchemaValidationError(str(exc)) from exc


def call_with_one_retry(
    label: str,
    system_prompt: str,
    prompt: str,
    timeout_ms: int,
    validate: Callable[[Any], dict[str, Any]],
) -> dict[str, Any]:
    try:
        raw = _agent_call(prompt, system_prompt, timeout_ms)
        return validate(raw)
    except SchemaValidationError as first_err:
        retry_prompt = prompt + RETRY_NUDGE.format(error=str(first_err))
        try:
            raw2 = _agent_call(retry_prompt, system_prompt, timeout_ms)
            return validate(raw2)
        except SchemaValidationError as second_err:
            raise WorkerJobError(
                RUNNER_SCHEMA_FAILURE,
                f"{label}: JSON schema failure after 1 retry: {second_err}",
            ) from second_err
