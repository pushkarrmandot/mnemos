"""Validates the agent's extraction JSON against the extraction schema.

No `pydantic` dependency exists in `src-python/pyproject.toml` (the worker
skeleton only ships `structlog`/`numpy`), and adding one just for two small
schemas isn't worth a new dependency. Plain functions instead:
`validate_extraction_payload` / `validate_refresh_payload` raise
`SchemaValidationError` with a message suitable for a "reformat as strict
JSON" retry nudge, or return a normalized dict (unknown top-level fields
dropped, missing optional lists defaulted to `[]`) on success.
"""

from __future__ import annotations

from typing import Any


class SchemaValidationError(Exception):
    pass


def _require_str(obj: dict[str, Any], field: str, where: str) -> str:
    value = obj.get(field)
    if not isinstance(value, str) or not value.strip():
        raise SchemaValidationError(f"{where}.{field} must be a non-empty string")
    return value


def _require_str_allow_empty(obj: dict[str, Any], field: str, where: str) -> str:
    value = obj.get(field)
    if not isinstance(value, str):
        raise SchemaValidationError(f"{where}.{field} must be a string")
    return value


def _optional_str(obj: dict[str, Any], field: str) -> str | None:
    value = obj.get(field)
    if value is None:
        return None
    if not isinstance(value, str):
        raise SchemaValidationError(f"{field} must be a string or null")
    return value


def _optional_int(obj: dict[str, Any], field: str) -> int | None:
    value = obj.get(field)
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, int):
        raise SchemaValidationError(f"{field} must be an integer or null")
    return value


def _optional_bool(obj: dict[str, Any], field: str) -> bool:
    """Defaults to `False` rather than raising on a missing/null value — a
    model that forgets this one flag shouldn't fail the whole extraction."""
    value = obj.get(field)
    if value is None:
        return False
    if not isinstance(value, bool):
        raise SchemaValidationError(f"{field} must be a boolean")
    return value


def _list_of(obj: dict[str, Any], field: str, where: str) -> list[dict[str, Any]]:
    value = obj.get(field, [])
    if value is None:
        return []
    if not isinstance(value, list):
        raise SchemaValidationError(f"{where}.{field} must be a list")
    for item in value:
        if not isinstance(item, dict):
            raise SchemaValidationError(f"{where}.{field} entries must be objects")
    return value


def validate_extraction_payload(data: Any) -> dict[str, Any]:
    """Validates the per-conversation extraction schema. Raises
    `SchemaValidationError` on any violation."""
    if not isinstance(data, dict):
        raise SchemaValidationError("top-level response must be a JSON object")

    title = _require_str(data, "title", "extraction")
    summary_markdown = _require_str(data, "summary_markdown", "extraction")

    action_items = []
    for item in _list_of(data, "action_items", "extraction"):
        action_items.append(
            {
                "text": _require_str(item, "text", "action_items[]"),
                "assignee_hint": _optional_str(item, "assignee_hint"),
                "assignee_is_self": _optional_bool(item, "assignee_is_self"),
                "assignee_contact_id": _optional_str(item, "assignee_contact_id"),
                "due_hint": _optional_str(item, "due_hint"),
                "source_timestamp_ms": _optional_int(item, "source_timestamp_ms"),
            }
        )

    decisions = []
    for item in _list_of(data, "decisions", "extraction"):
        decisions.append(
            {
                "statement": _require_str(item, "statement", "decisions[]"),
                "decided_by_hint": _optional_str(item, "decided_by_hint"),
                "decided_by_is_self": _optional_bool(item, "decided_by_is_self"),
                "quote": _optional_str(item, "quote"),
                "source_timestamp_ms": _optional_int(item, "source_timestamp_ms"),
            }
        )

    open_questions = []
    for item in _list_of(data, "open_questions", "extraction"):
        open_questions.append(
            {
                "question": _require_str(item, "question", "open_questions[]"),
                "raised_by_hint": _optional_str(item, "raised_by_hint"),
                "raised_by_is_self": _optional_bool(item, "raised_by_is_self"),
                "source_timestamp_ms": _optional_int(item, "source_timestamp_ms"),
            }
        )

    bookmarks = []
    for item in _list_of(data, "bookmarks", "extraction"):
        bookmarks.append(
            {
                "text": _require_str(item, "text", "bookmarks[]"),
                "timestamp_ms": _optional_int(item, "timestamp_ms") or 0,
            }
        )

    return {
        "title": title,
        "summary_markdown": summary_markdown,
        "action_items": action_items,
        "decisions": decisions,
        "open_questions": open_questions,
        "bookmarks": bookmarks,
    }


def validate_refresh_payload(data: Any) -> dict[str, Any]:
    """Validates the project-memory refresh schema. The agent returns only
    these three fields — `last_refresh_at`/`last_refresh_runner` are stamped
    by the caller."""
    if not isinstance(data, dict):
        raise SchemaValidationError("top-level response must be a JSON object")

    overview_markdown = _require_str(data, "overview_markdown", "refresh")
    scope_drift_markdown = _require_str_allow_empty(data, "scope_drift_markdown", "refresh")
    supersessions = _list_of(data, "supersessions", "refresh")

    return {
        "overview_markdown": overview_markdown,
        "scope_drift_markdown": scope_drift_markdown,
        "supersessions": supersessions,
    }
