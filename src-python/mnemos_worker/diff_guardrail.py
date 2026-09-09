"""Diff guardrail: `SequenceMatcher` ratio < 0.20 (>80% content
churn) between old and new `project_memory.json` markdown flags
`significant_change` so the caller can surface the amber "changed
significantly" card and keep the pre-write snapshot as the revert target.
"""

from __future__ import annotations

import difflib
import re

SIGNIFICANT_CHANGE_THRESHOLD = 0.20


def _normalize(text: str | None) -> str:
    return re.sub(r"\s+", " ", text or "").strip()


def significant_change_ratio(old_text: str | None, new_text: str | None) -> tuple[float, bool]:
    """Returns `(ratio, significant_change)`. `old_text is None` (never
    refreshed before — the `# NEW PROJECT` sentinel) is never a
    "significant change" — there's nothing to protect yet."""
    if old_text is None:
        return 1.0, False
    old_n, new_n = _normalize(old_text), _normalize(new_text)
    ratio = difflib.SequenceMatcher(None, old_n, new_n).ratio()
    return ratio, ratio < SIGNIFICANT_CHANGE_THRESHOLD
