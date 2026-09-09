"""One job kind among several (see the `jobs` package for transcription,
extraction, and memory-refresh handlers) — this one exists to prove the
job-queue + park/replay machinery end-to-end, not to do real work.
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
