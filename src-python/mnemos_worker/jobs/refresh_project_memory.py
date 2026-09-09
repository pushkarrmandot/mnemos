"""Project-memory refresh. One `run_agent_extraction` turn (plus at most one
schema-retry turn) that asks the agent to *modify* the current
`project_memory.json` document, then computes the diff guardrail here
(Python owns `difflib.SequenceMatcher` — no Rust-side equivalent was worth
adding for one bool).

Known gap: `prior_decisions`/`prior_open_questions` are NOT threaded into
this prompt in v1 — passing them requires a project-scoped list method on
`StorageService` (`list_project_decisions`/`list_project_open_questions`)
that doesn't exist yet. Supersession detection has less context as a
result; the refresh pipeline, the modify-not-rewrite contract, and the diff
guardrail are otherwise built to spec.

Same file-path-agnostic design as `extract_memory.py`: the caller (Rust —
`memory::refresh_project`) reads `project_memory.json`/`extraction.json`
files and passes their contents as data, and owns the snapshot-before-
overwrite + prune-history + final atomic write.
"""

from __future__ import annotations

from typing import Any

from mnemos_worker.agent_call import call_with_one_retry
from mnemos_worker.diff_guardrail import significant_change_ratio
from mnemos_worker.dispatch import method
from mnemos_worker.extraction_schema import validate_refresh_payload
from mnemos_worker.prompts import REFRESH_SYSTEM_PROMPT, build_refresh_prompt

DEFAULT_TIMEOUT_MS = 60_000


@method("refresh_project_memory")
def refresh_project_memory(params: dict[str, Any]) -> dict[str, Any]:
    current_memory = params.get("current_memory")
    new_extractions = params.get("new_extractions") or []
    project_meta = params.get("project_meta") or {}
    timeout_ms = params.get("timeout_ms", DEFAULT_TIMEOUT_MS)

    prompt = build_refresh_prompt(current_memory, new_extractions, project_meta)
    payload = call_with_one_retry(
        "refresh",
        REFRESH_SYSTEM_PROMPT,
        prompt,
        timeout_ms,
        validate_refresh_payload,
    )

    old_combined = None
    if current_memory is not None:
        old_combined = (current_memory.get("overview_markdown", "") or "") + "\n" + (
            current_memory.get("scope_drift_markdown", "") or ""
        )
    new_combined = payload["overview_markdown"] + "\n" + payload["scope_drift_markdown"]
    ratio, significant_change = significant_change_ratio(old_combined, new_combined)

    payload["significant_change"] = significant_change
    payload["diff_ratio"] = ratio
    return payload
