"""`job_progress.report` — the best-effort progress-tick emitter that feeds
Rust's deadline-extension logic. Configuration is module-global state, so
every test resets it via `monkeypatch` rather than mutating the shared
globals directly, keeping tests isolated from each other and from anything
else in the suite that might configure the real notifier.
"""

import pytest

from mnemos_worker import job_progress


@pytest.fixture(autouse=True)
def _reset_globals(monkeypatch):
    monkeypatch.setattr(job_progress, "_notify", None)
    monkeypatch.setattr(job_progress, "_current_request_id", None)


def test_report_is_a_noop_when_no_notifier_configured():
    job_progress.set_current_request_id("req-1")
    # Must not raise even though `_notify` is None.
    job_progress.report("transcribe", 0.5)


def test_report_is_a_noop_when_no_request_id_set():
    calls = []
    job_progress.configure_progress_notifier(lambda topic, payload: calls.append((topic, payload)))
    job_progress.report("transcribe", 0.5)
    assert calls == []


def test_report_sends_topic_and_payload_when_fully_configured():
    calls = []
    job_progress.configure_progress_notifier(lambda topic, payload: calls.append((topic, payload)))
    job_progress.set_current_request_id("req-42")

    job_progress.report("transcribe", 0.5, chunk=3)

    assert len(calls) == 1
    topic, payload = calls[0]
    assert topic == job_progress.TOPIC
    assert payload["request_id"] == "req-42"
    assert payload["kind"] == "transcribe"
    assert payload["fraction"] == 0.5
    assert payload["chunk"] == 3


@pytest.mark.parametrize(
    "raw,expected",
    [
        (-0.3, 0.0),
        (0.0, 0.0),
        (0.7, 0.7),
        (1.0, 1.0),
        (1.4, 1.0),
    ],
)
def test_fraction_is_clamped_to_zero_one(raw, expected):
    calls = []
    job_progress.configure_progress_notifier(lambda topic, payload: calls.append(payload))
    job_progress.set_current_request_id("req-1")

    job_progress.report("transcribe", raw)

    assert calls[0]["fraction"] == expected


def test_report_swallows_exceptions_raised_by_the_notifier():
    def bad_notify(topic, payload):
        raise RuntimeError("boom")

    job_progress.configure_progress_notifier(bad_notify)
    job_progress.set_current_request_id("req-1")

    # Must not raise: progress reporting is telemetry, never load-bearing.
    job_progress.report("transcribe", 0.5)


def test_set_current_request_id_can_clear_back_to_none():
    calls = []
    job_progress.configure_progress_notifier(lambda topic, payload: calls.append(payload))
    job_progress.set_current_request_id("req-1")
    job_progress.set_current_request_id(None)

    job_progress.report("transcribe", 0.5)

    assert calls == []
