import threading
import pytest

import mnemos_worker.agent_call as agent_call
from mnemos_worker.errors import (
    AGENT_BLOCKED,
    CANCELLED,
    RUNNER_SCHEMA_FAILURE,
    WORKER_UNAVAILABLE,
    WorkerJobError,
)
from mnemos_worker.extraction_schema import SchemaValidationError
from mnemos_worker.rpc_client import ReverseRpcError


def _validator_requiring_field(field):
    def validate(raw):
        if field not in raw:
            raise SchemaValidationError(f"missing {field}")
        return raw

    return validate


def test_happy_path_no_retry(monkeypatch):
    calls = []

    def fake_call(method, params, timeout_s):
        calls.append(params)
        return {"ok": True}

    monkeypatch.setattr(agent_call, "call_reverse_rpc", fake_call)
    out = agent_call.call_with_one_retry(
        "extraction", "sys", "prompt", 30_000, _validator_requiring_field("ok")
    )
    assert out == {"ok": True}
    assert len(calls) == 1


def test_schema_failure_then_success_on_retry(monkeypatch):
    responses = iter([{"nope": True}, {"ok": True}])
    calls = []

    def fake_call(method, params, timeout_s):
        calls.append(params["prompt"])
        return next(responses)

    monkeypatch.setattr(agent_call, "call_reverse_rpc", fake_call)
    out = agent_call.call_with_one_retry(
        "extraction", "sys", "prompt", 30_000, _validator_requiring_field("ok")
    )
    assert out == {"ok": True}
    assert len(calls) == 2
    assert "not valid JSON" in calls[1]  # retry nudge appended


def test_schema_failure_twice_raises_runner_error(monkeypatch):
    def fake_call(method, params, timeout_s):
        return {"nope": True}

    monkeypatch.setattr(agent_call, "call_reverse_rpc", fake_call)
    with pytest.raises(WorkerJobError) as exc_info:
        agent_call.call_with_one_retry(
            "extraction", "sys", "prompt", 30_000, _validator_requiring_field("ok")
        )
    assert exc_info.value.code == RUNNER_SCHEMA_FAILURE
    assert "after 1 retry" in str(exc_info.value)


def test_timeout_never_retries(monkeypatch):
    calls = []

    def fake_call(method, params, timeout_s):
        calls.append(1)
        raise ReverseRpcError(-32020, "extraction timed out")

    monkeypatch.setattr(agent_call, "call_reverse_rpc", fake_call)
    with pytest.raises(WorkerJobError) as exc_info:
        agent_call.call_with_one_retry(
            "extraction", "sys", "prompt", 30_000, _validator_requiring_field("ok")
        )
    assert exc_info.value.code == CANCELLED
    assert len(calls) == 1


def test_cli_missing_never_retries(monkeypatch):
    calls = []

    def fake_call(method, params, timeout_s):
        calls.append(1)
        raise ReverseRpcError(-32000, "claude CLI not on PATH", {"kind": "cli_missing"})

    monkeypatch.setattr(agent_call, "call_reverse_rpc", fake_call)
    with pytest.raises(WorkerJobError) as exc_info:
        agent_call.call_with_one_retry(
            "extraction", "sys", "prompt", 30_000, _validator_requiring_field("ok")
        )
    assert exc_info.value.code == WORKER_UNAVAILABLE
    assert len(calls) == 1


def test_agent_json_parse_error_is_retried_once(monkeypatch):
    responses = iter(
        [
            ReverseRpcError(-32603, "agent_json_parse: expected value"),
            {"ok": True},
        ]
    )

    def fake_call(method, params, timeout_s):
        item = next(responses)
        if isinstance(item, Exception):
            raise item
        return item

    monkeypatch.setattr(agent_call, "call_reverse_rpc", fake_call)
    out = agent_call.call_with_one_retry(
        "extraction", "sys", "prompt", 30_000, _validator_requiring_field("ok")
    )
    assert out == {"ok": True}


def test_usage_limit_fails_fast_without_the_schema_retry_nudge(monkeypatch):
    """Regression: a provider usage-limit refusal used to fall through to
    `SchemaValidationError`, which retried the call with a "your JSON was
    malformed" nudge — spending a second call against an already-exhausted
    quota — and then reported the failure as `RUNNER_SCHEMA_FAILURE`, i.e.
    told the user the model produced bad output. It must fail immediately,
    once, with the message passed through untouched for the UI.
    """
    calls = []

    def fake_call(method, params, timeout_s):
        calls.append(params["prompt"])
        raise ReverseRpcError(
            AGENT_BLOCKED,
            "Claude usage limit reached. Your recording and transcript are saved.",
            {"kind": "agent_blocked", "resets_at": 1_700_000_000},
        )

    monkeypatch.setattr(agent_call, "call_reverse_rpc", fake_call)
    with pytest.raises(WorkerJobError) as exc_info:
        agent_call.call_with_one_retry(
            "extraction", "sys", "prompt", 30_000, _validator_requiring_field("ok")
        )

    assert exc_info.value.code == AGENT_BLOCKED
    assert exc_info.value.code != RUNNER_SCHEMA_FAILURE
    # Exactly one call — no retry burned against the exhausted quota.
    assert len(calls) == 1
    # Message survives verbatim: Rust rebuilds it into AppError::RunnerBlocked,
    # whose Display is rendered to the user.
    assert "usage limit reached" in str(exc_info.value)


def test_usage_limit_detected_by_data_kind_even_if_code_differs(monkeypatch):
    """`to_rpc_err` sets both the -32023 code and `kind: "agent_blocked"`;
    keying off either keeps this working if one side drifts."""

    def fake_call(method, params, timeout_s):
        raise ReverseRpcError(-32000, "out of usage", {"kind": "agent_blocked"})

    monkeypatch.setattr(agent_call, "call_reverse_rpc", fake_call)
    with pytest.raises(WorkerJobError) as exc_info:
        agent_call.call_with_one_retry(
            "extraction", "sys", "prompt", 30_000, _validator_requiring_field("ok")
        )
    assert exc_info.value.code == AGENT_BLOCKED


def test_progress_is_reported_while_the_agent_call_blocks(monkeypatch):
    """The regression that made this necessary: the host's request deadline
    bounds silence, so an agent call longer than that window was swept
    mid-flight and failed a job the agent went on to answer correctly. The
    job-executor thread is blocked inside `call_reverse_rpc` for the whole
    call, so the ticks have to come from somewhere else."""
    import threading

    from mnemos_worker import job_progress

    ticks: list[tuple[str, dict]] = []
    monkeypatch.setattr(agent_call, "_ALIVE_TICK_S", 0.01)
    job_progress.configure_progress_notifier(lambda method, params: ticks.append((method, params)))
    job_progress.set_current_request_id("job-1")

    def slow_call(method, params, timeout_s):
        # Stand-in for a gateway that returns nothing at all until it is done.
        threading.Event().wait(0.15)
        return {"ok": True}

    monkeypatch.setattr(agent_call, "call_reverse_rpc", slow_call)
    try:
        out = agent_call.call_with_one_retry(
            "extraction", "sys", "prompt", 30_000, _validator_requiring_field("ok")
        )
    finally:
        job_progress.set_current_request_id(None)
        job_progress.configure_progress_notifier(None)

    assert out == {"ok": True}
    assert ticks, "a blocked agent call must still report liveness"
    method, params = ticks[0]
    assert method == job_progress.TOPIC
    assert params["request_id"] == "job-1", "ticks must re-arm the right request"


def test_ticker_stops_once_the_call_returns(monkeypatch):
    """A thread left ticking after the call would keep a finished request's
    deadline alive and leak one thread per extraction."""
    from mnemos_worker import job_progress

    ticks: list[object] = []
    monkeypatch.setattr(agent_call, "_ALIVE_TICK_S", 0.01)
    job_progress.configure_progress_notifier(lambda method, params: ticks.append(params))
    job_progress.set_current_request_id("job-2")
    monkeypatch.setattr(agent_call, "call_reverse_rpc", lambda *a, **k: {"ok": True})
    try:
        agent_call.call_with_one_retry(
            "extraction", "sys", "prompt", 30_000, _validator_requiring_field("ok")
        )
        settled = len(ticks)
        threading.Event().wait(0.05)
        assert len(ticks) == settled, "no ticks may arrive after the call returned"
    finally:
        job_progress.set_current_request_id(None)
        job_progress.configure_progress_notifier(None)


def test_a_broken_notifier_cannot_fail_an_extraction(monkeypatch):
    """Liveness reporting is best-effort: it runs alongside work that is
    otherwise succeeding and must never be able to fail it."""
    from mnemos_worker import job_progress

    monkeypatch.setattr(agent_call, "_ALIVE_TICK_S", 0.01)

    attempts: list[object] = []

    def exploding(method, params):
        attempts.append(params)
        raise RuntimeError("notifier is broken")

    job_progress.configure_progress_notifier(exploding)
    job_progress.set_current_request_id("job-3")

    def slow_call(method, params, timeout_s):
        threading.Event().wait(0.2)
        return {"ok": True}

    monkeypatch.setattr(agent_call, "call_reverse_rpc", slow_call)
    try:
        out = agent_call.call_with_one_retry(
            "extraction", "sys", "prompt", 30_000, _validator_requiring_field("ok")
        )
    finally:
        job_progress.set_current_request_id(None)
        job_progress.configure_progress_notifier(None)

    assert out == {"ok": True}
    # The real property: the loop survives a raising notifier instead of dying
    # on the first tick. A ticker that stopped here would silently stop
    # re-arming the deadline and the sweep would fail the job anyway.
    assert len(attempts) > 1, "the ticker must keep reporting after a notifier raises"
