"""Single-slot job queue (BACKEND_STANDARDS §2 "Job queue" —
`ThreadPoolExecutor(max_workers=1)`, models are not thread-safe). Every
accepted job is parked in `pending_jobs.json` immediately, moved to
`current_job.json` when it starts executing, and removed from both once it
completes — this is the state a crashed worker leaves behind for the Rust
supervisor to replay (LLD-02 §6).
"""

from __future__ import annotations

import queue
import threading
from pathlib import Path
from typing import Any, Callable

from mnemos_worker import job_progress
from mnemos_worker.dispatch import DISPATCH_TABLE
from mnemos_worker.errors import WorkerJobError
from mnemos_worker.state_files import atomic_write_json, remove_if_exists

OnResponse = Callable[[str, dict[str, Any] | None, dict[str, Any] | None], None]


class JobExecutor:
    def __init__(self, state_dir: Path, on_response: OnResponse, logger: Any) -> None:
        self._state_dir = state_dir
        self._on_response = on_response
        self._log = logger
        self._pending: list[dict[str, Any]] = []
        self._pending_lock = threading.Lock()
        self._queue: queue.Queue[dict[str, Any] | None] = queue.Queue()
        # Idempotency cache (LLD-02 §6): re-issuing a job this process already
        # completed is a no-op returning the cached result. Only meaningful
        # within one worker lifetime — a full restart naturally re-runs
        # replayed jobs, which is safe because `ping` has no side effects.
        self._completed_cache: dict[tuple[str, str], dict[str, Any]] = {}
        self._current_done = threading.Event()
        self._current_done.set()
        self._thread = threading.Thread(target=self._run, name="job-executor", daemon=True)

    def start(self) -> None:
        self._thread.start()

    def submit(self, request_id: str, kind: str, params: dict[str, Any]) -> None:
        job_key = (kind, str(params.get("job_id", request_id)))
        cached = self._completed_cache.get(job_key)
        if cached is not None:
            self._on_response(request_id, cached, None)
            return

        job = {"id": request_id, "kind": kind, "params": params, "job_key": list(job_key)}
        with self._pending_lock:
            self._pending.append(job)
            self._persist_pending_locked()
        self._current_done.clear()
        self._queue.put(job)

    def drain(self, timeout_s: float) -> None:
        """Waits for any in-flight job to finish (shutdown grace period)."""
        self._current_done.wait(timeout_s)

    def stop(self) -> None:
        self._queue.put(None)

    def _persist_pending_locked(self) -> None:
        atomic_write_json(self._state_dir / "pending_jobs.json", {"jobs": self._pending})

    def _run(self) -> None:
        while True:
            job = self._queue.get()
            if job is None:
                return
            with self._pending_lock:
                self._pending = [j for j in self._pending if j["id"] != job["id"]]
                self._persist_pending_locked()
            atomic_write_json(self._state_dir / "current_job.json", job)

            result: dict[str, Any] | None = None
            error: dict[str, Any] | None = None
            # Lets a long-running handler emit progress attributed to *this*
            # request without every handler signature growing a notify
            # parameter — see `job_progress`. Cleared in `finally` so a
            # handler that raises can't leave a stale id attributed to the
            # next job.
            job_progress.set_current_request_id(job["id"])
            try:
                handler = DISPATCH_TABLE[job["kind"]]
                result = handler(job["params"])
            except WorkerJobError as exc:
                self._log.error("job.failed", kind=job["kind"], code=exc.code, error=str(exc))
                error = {"code": exc.code, "message": str(exc)}
            except Exception as exc:  # noqa: BLE001 — becomes a JSON-RPC error, never crashes the loop
                self._log.error("job.failed", kind=job["kind"], error=str(exc))
                error = {"code": -32000, "message": str(exc)}
            finally:
                job_progress.set_current_request_id(None)

            remove_if_exists(self._state_dir / "current_job.json")
            if error is None:
                self._completed_cache[tuple(job["job_key"])] = result or {}
            self._current_done.set()
            self._on_response(job["id"], result, error)
