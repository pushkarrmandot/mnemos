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
