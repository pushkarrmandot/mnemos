import pytest

from mnemos_worker.extraction_schema import (
    SchemaValidationError,
    validate_extraction_payload,
    validate_refresh_payload,
)


def test_missing_summary_markdown_rejected():
    with pytest.raises(SchemaValidationError):
        validate_extraction_payload({"action_items": []})


def test_extra_top_level_fields_silently_dropped():
    out = validate_extraction_payload({"summary_markdown": "# Overview", "unexpected_field": 123})
    assert "unexpected_field" not in out
    assert out["summary_markdown"] == "# Overview"


def test_bookmarks_optional():
    out = validate_extraction_payload({"summary_markdown": "# Overview"})
    assert out["bookmarks"] == []


def test_empty_lists_valid():
    out = validate_extraction_payload(
        {
            "summary_markdown": "# Overview",
            "action_items": [],
            "decisions": [],
            "open_questions": [],
        }
    )
    assert out["action_items"] == []
    assert out["decisions"] == []
    assert out["open_questions"] == []


def test_source_timestamp_ms_accepts_null():
    out = validate_extraction_payload(
        {
            "summary_markdown": "# Overview",
            "action_items": [{"text": "Send spec", "source_timestamp_ms": None}],
        }
    )
    assert out["action_items"][0]["source_timestamp_ms"] is None


def test_non_dict_top_level_rejected():
    with pytest.raises(SchemaValidationError):
        validate_extraction_payload(["not", "an", "object"])


def test_action_item_missing_required_text_rejected():
    with pytest.raises(SchemaValidationError):
        validate_extraction_payload(
            {"summary_markdown": "# Overview", "action_items": [{"assignee_hint": "David"}]}
        )


def test_refresh_payload_happy_path():
    out = validate_refresh_payload(
        {
            "overview_markdown": "The team is redesigning a remote.",
            "scope_drift_markdown": "Kickoff...",
            "supersessions": [],
        }
    )
    assert out["overview_markdown"].startswith("The team")
    assert out["supersessions"] == []


def test_refresh_payload_missing_field_rejected():
    with pytest.raises(SchemaValidationError):
        validate_refresh_payload({"overview_markdown": "x"})
