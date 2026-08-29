"""Per-conversation extraction (LLD-05 §4) — the `extracting` pipeline step.
One `run_agent_extraction` turn (plus at most one schema-retry turn) over a
transcript, returning the validated LLD-05 §4.3 payload as the job result.

The caller (Rust — `memory::extract_conversation`) is the one that persists
`extraction.json`/`summary.md`/the structured rows, per LLD-05 §4.4's "worker
does not touch SQLite" and this wave's design of keeping this handler
file-path-agnostic (it receives `transcript` as data, not a path — see
`product_docs/lld/LLD_05_MEMORY_SYSTEM.md`'s "Implementation status" for why
that's a deliberate deviation from the LLD's "worker reads the file itself"
sketch).
"""

from __future__ import annotations

from typing import Any

from mnemos_worker.agent_call import call_with_one_retry
from mnemos_worker.dispatch import method
from mnemos_worker.errors import VALIDATION, WorkerJobError
from mnemos_worker.extraction_schema import validate_extraction_payload
from mnemos_worker.prompts import EXTRACTION_SYSTEM_PROMPT, build_extraction_prompt

# LLD-05 §4.2 — placeholder cap pending LLD-07 exposing the runner's real
# context window (§10 Q3). ~500k chars ≈ Claude Sonnet's ~200k token budget.
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
