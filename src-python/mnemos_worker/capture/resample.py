"""Downmix + linear-interpolation resample to 16kHz mono 16-bit PCM
("Resample is done in Python via numpy linear interpolation. PROVISIONAL —
if quality is not acceptable, swap in soxr"). Kept as a free function,
independent of any live audio stream, so it's directly unit-testable with
synthetic PCM.
"""

from __future__ import annotations

import numpy as np

TARGET_SAMPLE_RATE = 16000


def resample_to_16k_mono(pcm: bytes, sample_rate: int, channels: int) -> bytes:
    if len(pcm) == 0:
        return b""

    samples = np.frombuffer(pcm, dtype="<i2").astype(np.float64)
    if channels > 1:
        samples = samples.reshape(-1, channels).mean(axis=1)

    if sample_rate == TARGET_SAMPLE_RATE:
        resampled = samples
    else:
        duration_s = len(samples) / sample_rate
        target_n = max(1, round(duration_s * TARGET_SAMPLE_RATE))
        src_x = np.linspace(0.0, duration_s, num=len(samples), endpoint=False)
        dst_x = np.linspace(0.0, duration_s, num=target_n, endpoint=False)
        resampled = np.interp(dst_x, src_x, samples)

    clipped = np.clip(resampled, -32768, 32767).astype("<i2")
    return clipped.tobytes()
