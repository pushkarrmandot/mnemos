"""The only real job kind this wave ships. Real job handlers (transcription,
extraction) land in W7/W11 — this exists to prove the job-queue + park/replay
machinery end-to-end (§6 of LLD-02), not to do real work.
"""

from __future__ import annotations

import time
from typing import Any

from mnemos_worker.dispatch import method


@method("ping")
def ping(params: dict[str, Any]) -> dict[str, Any]:
    delay_ms = params.get("delay_ms", 0)
    if delay_ms:
        time.sleep(delay_ms / 1000)
    return {"pong": True}
