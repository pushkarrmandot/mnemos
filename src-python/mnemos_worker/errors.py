"""`WorkerJobError` lets a job handler pick its own JSON-RPC error code
instead of `job_executor.py`'s generic `-32000` catch-all, so the Rust side's
`map_json_rpc_error` (LLD-02 §4.4) can classify the failure instead of every
job error looking like an opaque `Internal`. Codes reused here are ones
`map_json_rpc_error` already understands (`-32001` WorkerUnavailable,
`-32010` Validation, `-32020` Cancelled) plus `-32022`, added this wave for
"the agent's JSON still fails schema validation after one retry" (LLD-05
§4.5) -> `AppError::Runner`.
"""

from __future__ import annotations


class WorkerJobError(Exception):
    def __init__(self, code: int, message: str) -> None:
        super().__init__(message)
        self.code = code


RUNNER_SCHEMA_FAILURE = -32022
WORKER_UNAVAILABLE = -32001
VALIDATION = -32010
CANCELLED = -32020
# The agent refused for a provider-side usage limit. Distinct from
# WORKER_UNAVAILABLE so Rust's `map_json_rpc_error` can rebuild an
# `AppError::RunnerBlocked` (recoverable, user-facing message preserved)
# rather than a generic "worker unavailable; retry after 0ms".
AGENT_BLOCKED = -32023
