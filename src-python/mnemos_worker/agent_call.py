"""Shared one-call-plus-one-retry policy (LLD-05 §4.5) for both memory jobs:
call `run_agent_extraction`, validate the JSON, and on a schema mismatch
retry exactly once with the validator's error appended as a nudge. A true
infra failure (agent timeout, CLI missing/not-logged-in/stream-corrupt)
never retries — it fails the job immediately with a code
`job_executor.py`/Rust's `map_json_rpc_error` can classify correctly.
"""

from __future__ import annotations

from typing import Any, Callable

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


def _agent_call(prompt: str, system_prompt: str, timeout_ms: int) -> Any:
    timeout_s = timeout_ms / 1000 + 10
    try:
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
