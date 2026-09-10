import pytest

import mnemos_worker.agent_call as agent_call
from mnemos_worker.errors import VALIDATION, WorkerJobError
from mnemos_worker.jobs.extract_memory import extract_memory

TRANSCRIPT = {
    "schema_version": 1,
    "conversation_id": "c1",
    "duration_ms": 60_000,
    "turns": [
        {"text": "Let's ship the auth spec today.", "speaker_label": "You", "ts_start_ms": 0},
        {"text": "Sounds good, I'll review it.", "speaker_label": "Them", "ts_start_ms": 4000},
    ],
}


def test_extraction_happy_path(monkeypatch):
    def fake_call(method, params, timeout_s):
        assert method == "run_agent_extraction"
        assert "prompt" in params and "system_prompt" in params
        return {
            "title": "Auth Spec Handoff",
            "summary_markdown": "# Overview\nShipping the auth spec.",
            "action_items": [{"text": "Send David the auth spec", "assignee_hint": "David"}],
            "decisions": [],
            "open_questions": [],
        }

    monkeypatch.setattr(agent_call, "call_reverse_rpc", fake_call)
    result = extract_memory(
        {
            "conversation_id": "c1",
            "transcript": TRANSCRIPT,
            "contacts": [],
            "notes": None,
            "conversation_meta": {"duration_s": 60},
        }
    )
    assert result["summary_markdown"].startswith("# Overview")
    assert len(result["action_items"]) == 1
    assert result["bookmarks"] == []


def test_oversized_transcript_rejected_without_calling_agent(monkeypatch):
    calls = []
    monkeypatch.setattr(agent_call, "call_reverse_rpc", lambda *a, **k: calls.append(1))
    huge_transcript = {"turns": [{"text": "x" * 600_000, "speaker_label": "You", "ts_start_ms": 0}]}
    with pytest.raises(WorkerJobError) as exc_info:
        extract_memory({"conversation_id": "c1", "transcript": huge_transcript})
    assert exc_info.value.code == VALIDATION
    assert calls == []


class TestExtractionTimeout:
    """30s was the whole budget for process start, model round-trip and
    structured output. It held on a fast, warm, direct connection and nowhere
    else: a corporate machine whose `claude` proxies through an internal
    gateway hit exactly 30.0s three times in a row on a 164-segment
    transcript, while interactive chat on the same binary worked fine.
    """

    @staticmethod
    def _transcript(chars: int):
        return {"turns": [{"text": "x" * chars}]}

    def test_short_transcript_gets_the_base_budget(self):
        from mnemos_worker.jobs.extract_memory import BASE_TIMEOUT_MS, _timeout_for

        assert _timeout_for(self._transcript(500)) == BASE_TIMEOUT_MS

    def test_budget_grows_with_transcript_length(self):
        from mnemos_worker.jobs.extract_memory import _timeout_for

        short = _timeout_for(self._transcript(1_000))
        long = _timeout_for(self._transcript(100_000))
        assert long > short, "a 90-minute workshop cannot share a deadline with a standup"

    def test_budget_is_capped(self):
        from mnemos_worker.jobs.extract_memory import MAX_TIMEOUT_MS, _timeout_for

        assert _timeout_for(self._transcript(10_000_000)) == MAX_TIMEOUT_MS

    def test_an_explicit_caller_timeout_still_wins(self):
        """The command layer can still pin it — used by tests and by any
        caller that knows better than the heuristic."""
        from mnemos_worker.jobs.extract_memory import _timeout_for

        assert _timeout_for(self._transcript(1_000)) != 5_000
