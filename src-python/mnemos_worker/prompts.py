"""Prompt text for the two `run_agent_extraction` callers (LLD-05 §6). Plain
f-strings, not a Jinja2 template file — `src-python/pyproject.toml` has no
templating dependency and neither prompt needs more than string
interpolation + a loop, so pulling in `jinja2` for `extract.md.j2`/
`refresh_memory.md.j2` as LLD-05 §6 sketches wasn't worth it. LLD-05 itself
says it specifies the *contract*, not the exact wording — this module is the
"coding agent tunes it" implementation of that contract.
"""

from __future__ import annotations

from typing import Any

EXTRACTION_SYSTEM_PROMPT = """\
You are extracting structured notes from a meeting transcript.

Output ONLY a single JSON object — no prose, no markdown code fences, no \
explanation before or after it. The object must have exactly these keys:

{
  "summary_markdown": "# Overview\\n...\\n## Discussion\\n...\\n## Decisions\\n...\\n## Follow-ups\\n...\\n## Concerns\\n...",
  "action_items": [{"text": str, "assignee_hint": str|null, "due_hint": str|null, "source_timestamp_ms": int|null}],
  "decisions": [{"statement": str, "decided_by_hint": str|null, "quote": str|null, "source_timestamp_ms": int|null}],
  "open_questions": [{"question": str, "raised_by_hint": str|null, "source_timestamp_ms": int|null}],
  "bookmarks": []
}

Speaker labels in the transcript are channel labels, not people: "You" is \
the microphone (the user) and "Them" is everything the computer played \
(everyone else on the call, however many that is).

Filling in `assignee_hint`, `decided_by_hint`, and `raised_by_hint`:
- Use "You" when the person is the user. The known contacts list marks the \
user with `is_self: true` and gives their name — so if someone in the \
transcript addresses them by that name ("Priya, can you send that over"), \
the item is theirs and the hint is "You", even though the words came from \
the "Them" channel.
- Use a person's actual name when the transcript makes it clear who is meant.
- Use null when you cannot tell. Never write "Them" — it means "one of the \
other people, unknown", which is the same as not knowing, and it renders as \
a label that looks like information and carries none. Guessing is worse than \
null here: an unassigned item reads as needing an owner, while a wrongly \
assigned one reads as settled.

`source_timestamp_ms` is the `[123ms]` value prefixing the transcript line \
the item came from — milliseconds from the start of the recording. It is \
used to jump the user to that moment, so give the line where the thing was \
actually said, not where the topic was introduced. Use null if no line \
prefix applies.

`bookmarks` entries are `{"ts_ms": int, "label": str}`. Leave the list empty \
unless a moment is clearly worth flagging on its own.

Write `summary_markdown` in the transcript's own language. Omit a section \
entirely rather than padding it: a short conversation with no disagreement \
has no "Concerns", and inventing one to fill the heading is worse than the \
heading being absent.

This is a single turn: do not ask questions, do not use tools, just return \
the JSON object.\
"""

REFRESH_SYSTEM_PROMPT = """\
You are updating a project's running memory document from newly-processed \
conversations.

Output ONLY a single JSON object — no prose, no markdown code fences. The \
object must have exactly these keys:

{
  "overview_markdown": str,
  "scope_drift_markdown": str,
  "supersessions": [{"reversed_decision_id": str, "replaced_by_decision_id": str, "reason": str}]
}

`overview_markdown` is 2-3 sentences of PLAIN NARRATIVE PROSE: what this \
project is and where it stands. No headings. No bullet lists. No sections.

It must NOT contain "Discussion", "Decisions", "Follow-ups", "Concerns", \
"Open Questions", or "Action Items" sections. The per-conversation summaries \
you are given as input DO use those headings — that is the format of a \
single meeting's summary, NOT a template for this document. Do not copy \
their shape. Decisions and open questions render as their own structured \
lists beside this text, and action items belong to the user's own to-do \
surface, so repeating any of them here says everything twice.

`scope_drift_markdown` is likewise narrative prose: kickoff scope, what was \
added, what was cut and by whom, net position.

Rules (modify, do not rewrite):
0. The two format rules above OVERRIDE rule 1. If the CURRENT DOCUMENT \
violates them — for example its overview carries `## Decisions` or \
`## Action Items` sections, or runs far past three sentences — rewrite that \
field to comply, keeping only its factual content. Rule 1 protects the \
user's wording, not a malformed structure.
1. Treat the CURRENT DOCUMENT's markdown as ground truth — preserve its \
wording, structure, and headings.
2. Only change a section if a new conversation contains information that \
directly changes its factual content.
3. Never delete a structured item marked `added_manually: true` from your \
reasoning — it stays in storage regardless of what you write here.
4. When you detect a decision has been superseded, add an entry to \
`supersessions` — do not strike it through in the markdown; the UI renders \
that from the flag.
This is a single turn: do not ask questions, do not use tools, just return \
the JSON object.\
"""

RETRY_NUDGE = (
    "\n\nYour last response was not valid JSON matching the required schema. "
    "Error: {error}. Respond ONLY with the corrected JSON object. "
    "No prose, no markdown fences."
)


def _render_transcript(transcript: dict[str, Any]) -> str:
    turns = transcript.get("turns", [])
    lines = []
    for turn in turns:
        label = turn.get("speaker_label", "?")
        text = turn.get("text", "")
        ts = turn.get("ts_start_ms")
        prefix = f"[{ts}ms] " if ts is not None else ""
        lines.append(f"{prefix}{label}: {text}")
    return "\n".join(lines)


def _render_contacts(contacts: list[dict[str, Any]]) -> str:
    """Renders the contact list with the self-entry called out in words.

    ``contacts`` is a raw list of dicts, so ``is_self`` would otherwise reach
    the model only as one key among several inside a Python repr. It is the
    single most consequential field in the prompt — it is what lets the model
    attribute "Priya, can you send that over" to the user — so it gets a
    sentence of its own rather than trusting the model to notice a flag.
    """
    lines = []
    for contact in contacts:
        name = contact.get("display_name") or contact.get("first_name") or "?"
        if contact.get("is_self"):
            lines.append(
                f'- {name} — THIS IS THE USER. Attribute their items to "You".'
            )
        else:
            lines.append(f"- {name}")
    return "\n".join(lines)


def build_extraction_prompt(
    transcript: dict[str, Any],
    contacts: list[dict[str, Any]],
    notes: str | None,
    conversation_meta: dict[str, Any],
) -> str:
    transcript_body = _render_transcript(transcript)
    parts = [
        f"Conversation metadata: {conversation_meta}",
    ]
    if contacts:
        parts.append(f"Known contacts:\n{_render_contacts(contacts)}")
    if notes:
        parts.append(f"User notes: {notes}")
    parts.append("Transcript:")
    parts.append(transcript_body)
    return "\n\n".join(parts)


def build_refresh_prompt(
    current_memory: dict[str, Any] | None,
    new_extractions: list[dict[str, Any]],
    project_meta: dict[str, Any],
) -> str:
    parts = [f"Project: {project_meta.get('name')}"]
    user = project_meta.get("user")
    if user:
        name = user.get("display_name") or user.get("first_name")
        if name:
            # Without this the model has no subject for the project's own
            # story and either writes around the user in the passive voice or
            # invents a name for them.
            parts.append(
                f"The user whose project this is: {name}. Refer to them as "
                f'"you" in the prose, never by name and never in the third person.'
            )
    if current_memory is None:
        parts.append(
            "CURRENT DOCUMENT: # NEW PROJECT (no prior memory document exists yet)."
        )
    else:
        parts.append(
            "CURRENT DOCUMENT overview_markdown:\n"
            f"{current_memory.get('overview_markdown', '')}\n\n"
            "CURRENT DOCUMENT scope_drift_markdown:\n"
            f"{current_memory.get('scope_drift_markdown', '')}"
        )
    parts.append(f"New conversations since last refresh ({len(new_extractions)}):")
    for entry in new_extractions:
        parts.append(f"- {entry}")
    return "\n\n".join(parts)
