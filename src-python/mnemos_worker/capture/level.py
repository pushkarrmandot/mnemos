"""RMS-in-dBFS level metering over a 16-bit PCM buffer (LLD-03 §4.2:
"Level events are RMS over the last ~100ms window converted to dBFS")."""

from __future__ import annotations

import math

import numpy as np

FLOOR_DB = -96.0


def rms_dbfs(pcm: bytes) -> float:
    if len(pcm) < 2:
        return FLOOR_DB
    samples = np.frombuffer(pcm, dtype="<i2").astype(np.float64) / 32768.0
    if samples.size == 0:
        return FLOOR_DB
    rms = math.sqrt(float(np.mean(samples**2)))
    if rms <= 0:
        return FLOOR_DB
    return 20.0 * math.log10(rms)
