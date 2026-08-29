import mnemos_worker.agent_call as agent_call
from mnemos_worker.jobs.refresh_project_memory import refresh_project_memory


def test_refresh_happy_path_small_change(monkeypatch):
    def fake_call(method, params, timeout_s):
        return {
            "overview_markdown": "The team is redesigning a remote control.",
            "scope_drift_markdown": "Kickoff: universal remote at €25.",
            "supersessions": [],
        }

    monkeypatch.setattr(agent_call, "call_reverse_rpc", fake_call)
    result = refresh_project_memory(
        {
            "project_id": "p1",
            "current_memory": {
                "overview_markdown": "The team is redesigning a remote control device.",
                "scope_drift_markdown": "Kickoff: universal remote at €25.",
            },
            "new_extractions": [{"conv_id": "c1", "extraction": {"summary_markdown": "..."}}],
            "project_meta": {"name": "Remote Control"},
        }
    )
    assert result["significant_change"] is False
    assert result["diff_ratio"] > 0.20


def test_refresh_new_project_sentinel_never_significant(monkeypatch):
    def fake_call(method, params, timeout_s):
        assert "NEW PROJECT" in params["prompt"]
        return {
            "overview_markdown": "Brand new overview text about a totally new topic.",
            "scope_drift_markdown": "",
            "supersessions": [],
        }

    monkeypatch.setattr(agent_call, "call_reverse_rpc", fake_call)
    result = refresh_project_memory(
        {
            "project_id": "p1",
            "current_memory": None,
            "new_extractions": [],
            "project_meta": {"name": "New Project"},
        }
    )
    assert result["significant_change"] is False


def test_refresh_blows_away_most_content_flags_significant_change(monkeypatch):
    def fake_call(method, params, timeout_s):
        return {
            "overview_markdown": "x",
            "scope_drift_markdown": "",
            "supersessions": [],
        }

    monkeypatch.setattr(agent_call, "call_reverse_rpc", fake_call)
    result = refresh_project_memory(
        {
            "project_id": "p1",
            "current_memory": {
                "overview_markdown": (
                    "The team spent several weeks redesigning a universal remote "
                    "control targeted at a twenty-five euro retail price point, "
                    "covering button layout, battery life, and manufacturing cost."
                ),
                "scope_drift_markdown": "Kickoff on Aug 10 covered initial scope.",
            },
            "new_extractions": [],
            "project_meta": {"name": "Remote Control"},
        }
    )
    assert result["significant_change"] is True
    assert result["diff_ratio"] < 0.20
