import json
import time
from pathlib import Path

import pytest

from mnemos_worker.dispatch import DISPATCH_TABLE, method
from mnemos_worker.job_executor import JobExecutor


@pytest.fixture(autouse=True)
def _register_test_job():
    calls = []

    @method("test_job")
    def _handler(params):
        calls.append(params)
        if params.get("delay_ms"):
            time.sleep(params["delay_ms"] / 1000)
        if params.get("fail"):
            raise ValueError("boom")
        return {"echo": params}

    yield calls
    del DISPATCH_TABLE["test_job"]


class DummyLogger:
    def error(self, *a, **k):
        pass

    def info(self, *a, **k):
        pass


def test_submit_parks_then_completes(tmp_path: Path):
    responses = []
    executor = JobExecutor(tmp_path, on_response=lambda *a: responses.append(a), logger=DummyLogger())
    executor.start()

    executor.submit("req-1", "test_job", {"x": 1})
    executor.drain(2.0)

    assert responses == [("req-1", {"echo": {"x": 1}}, None)]
    assert not (tmp_path / "current_job.json").exists()
    pending = json.loads((tmp_path / "pending_jobs.json").read_text())
    assert pending["jobs"] == []


def test_slow_job_is_visible_in_current_job_json(tmp_path: Path):
    responses = []
    executor = JobExecutor(tmp_path, on_response=lambda *a: responses.append(a), logger=DummyLogger())
    executor.start()

    executor.submit("req-2", "test_job", {"delay_ms": 200})
    time.sleep(0.05)

    current = json.loads((tmp_path / "current_job.json").read_text())
    assert current["id"] == "req-2"

    executor.drain(2.0)
    assert responses[0][0] == "req-2"


def test_pending_jobs_json_written_before_execution(tmp_path: Path):
    responses = []
    executor = JobExecutor(tmp_path, on_response=lambda *a: responses.append(a), logger=DummyLogger())
    # Do not start the worker thread yet — job must sit parked.
    executor.submit("req-3", "test_job", {"x": 1})

    pending = json.loads((tmp_path / "pending_jobs.json").read_text())
    assert [j["id"] for j in pending["jobs"]] == ["req-3"]

    executor.start()
    executor.drain(2.0)
    assert responses[0][0] == "req-3"


def test_failed_job_returns_error_and_clears_current(tmp_path: Path):
    responses = []
    executor = JobExecutor(tmp_path, on_response=lambda *a: responses.append(a), logger=DummyLogger())
    executor.start()

    executor.submit("req-4", "test_job", {"fail": True})
    executor.drain(2.0)

    request_id, result, error = responses[0]
    assert request_id == "req-4"
    assert result is None
    assert error["code"] == -32000
    assert not (tmp_path / "current_job.json").exists()


def test_idempotent_replay_returns_cached_result(tmp_path: Path):
    responses = []
    executor = JobExecutor(tmp_path, on_response=lambda *a: responses.append(a), logger=DummyLogger())
    executor.start()

    executor.submit("req-5", "test_job", {"job_id": "shared-key", "x": 1})
    executor.drain(2.0)
    # A second submission carrying the same job_id (a Rust-side replay after
    # restart re-issuing the same logical job) must not re-execute.
    executor.submit("req-6", "test_job", {"job_id": "shared-key", "x": 1})

    assert len(responses) == 2
    assert responses[1][0] == "req-6"
    assert responses[1][1] == responses[0][1]
