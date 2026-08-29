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
