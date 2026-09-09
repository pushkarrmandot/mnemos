"""Per-conversation extraction — the `extracting` pipeline step. One
`run_agent_extraction` turn (plus at most one schema-retry turn) over a
transcript, returning the validated extraction payload as the job result.

The caller (Rust — `memory::extract_conversation`) is the one that persists
`extraction.json`/`summary.md`/the structured rows — the worker never
touches SQLite. This handler is deliberately file-path-agnostic: it takes
`transcript` as data in `params`, not a path to read itself. That keeps the
worker stateless with respect to the conversation blob directory (no
filesystem coupling to Rust's storage layout) and makes this handler
trivially unit-testable with an in-memory transcript, at the cost of the
caller having to load and pass the transcript in.
"""

from __future__ import annotations

from typing import Any

from mnemos_worker.agent_call import call_with_one_retry
from mnemos_worker.dispatch import method
from mnemos_worker.errors import VALIDATION, WorkerJobError
from mnemos_worker.extraction_schema import validate_extraction_payload
from mnemos_worker.prompts import EXTRACTION_SYSTEM_PROMPT, build_extraction_prompt

# Placeholder cap pending the agent runner exposing its real context window
# size. ~500k chars ≈ Claude Sonnet's ~200k token budget.
MAX_TRANSCRIPT_CHARS = 500_000

DEFAULT_TIMEOUT_MS = 30_000


def _transcript_body_len(transcript: dict[str, Any]) -> int:
    return sum(len(turn.get("text", "")) for turn in transcript.get("turns", []))


@method("extract_memory")
def extract_memory(params: dict[str, Any]) -> dict[str, Any]:
    transcript = params.get("transcript") or {}
    contacts = params.get("contacts") or []
    notes = params.get("notes")
    conversation_meta = params.get("conversation_meta") or {}
    timeout_ms = params.get("timeout_ms", DEFAULT_TIMEOUT_MS)

    if _transcript_body_len(transcript) > MAX_TRANSCRIPT_CHARS:
        raise WorkerJobError(VALIDATION, "extraction: transcript_length exceeds cap")

    prompt = build_extraction_prompt(transcript, contacts, notes, conversation_meta)
    return call_with_one_retry(
        "extraction",
        EXTRACTION_SYSTEM_PROMPT,
        prompt,
        timeout_ms,
        validate_extraction_payload,
    )
