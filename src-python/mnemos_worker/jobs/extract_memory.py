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

# Extraction has to cover process start, the model round-trip, and structured
# output over the whole transcript. 30s covered that on a fast, warm, direct
# connection and nothing else: on a corporate machine whose `claude` proxies
# through an internal gateway, a 164-segment transcript timed out three times
# in a row at exactly 30.0s, while interactive chat on the same binary worked
# fine. The budget was the problem, not the runner.
#
# Scaled by transcript size, because one deadline for a five-minute standup
# and a ninety-minute workshop cannot be right for both. Generous on purpose:
# the cost of waiting too long is a slow summary, and the cost of being too
# strict is no summary at all plus a failure the user has to act on.
BASE_TIMEOUT_MS = 180_000
# ~1s per 1k transcript characters, on top of the base.
TIMEOUT_MS_PER_1K_CHARS = 1_000
MAX_TIMEOUT_MS = 900_000


def _timeout_for(transcript: dict[str, Any]) -> int:
    scaled = BASE_TIMEOUT_MS + (_transcript_body_len(transcript) // 1_000) * TIMEOUT_MS_PER_1K_CHARS
    return min(scaled, MAX_TIMEOUT_MS)


def _transcript_body_len(transcript: dict[str, Any]) -> int:
    return sum(len(turn.get("text", "")) for turn in transcript.get("turns", []))


@method("extract_memory")
def extract_memory(params: dict[str, Any]) -> dict[str, Any]:
    transcript = params.get("transcript") or {}
    contacts = params.get("contacts") or []
    notes = params.get("notes")
    conversation_meta = params.get("conversation_meta") or {}
    timeout_ms = params.get("timeout_ms") or _timeout_for(transcript)

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
