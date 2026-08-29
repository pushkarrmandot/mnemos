"""Speaker-bleed (cross-talk) detection for the two-channel transcript merge.

**The problem.** `transcribe_final` runs two independent Parakeet passes — one
over `mic.wav`, one over `system.wav` — and `merge_transcripts` labels each
turn purely by which file it came from ("You" for mic, "Them" for system).
That is exact when the two channels are acoustically independent, i.e. the
user is on headphones. On *speakers* it is not: the far side's voice leaves
the speakers, reaches the microphone, and gets transcribed a second time on
the mic channel — where it is then labeled "You". Measured on real recordings
the leaked copy arrives ~180-210ms later at roughly 0.2-0.4x the system
channel's amplitude.

Wrong attribution is worse than none here: this transcript feeds summary and
extraction, so a leaked turn labeled "You" becomes an action item the user
never agreed to. Hence this module.

**Why detection rather than cancellation.** Subtracting the leak with an
adaptive filter (classic AEC) would need a double-talk detector and would
have to track clock drift between two independent capture paths — and when it
misjudges, it eats the user's own speech, losing content permanently. We only
need the *label*, never clean audio, so this module reads the waveforms and
decides; it never modifies them. The worst case is a mislabeled line, not a
lost sentence. macOS Voice Processing I/O is enabled on the capture side too,
but it can only cancel audio the app itself renders — it has no reference for
another process's playback, so it removes little of this leakage in practice.

**How.** Per mic-sourced segment, over that segment's own time window:

  system silence — if the system channel is quiet, nothing was playing and
    the segment is genuine by construction. Settles most turns outright.
  amplitude ratio — leakage crosses a room and lands well below the system
    channel; a real speaker is inches from the mic and lands above it.
  envelope correlation — correlation of the two channels' short-time ENERGY
    envelopes, searched over plausible lags. Envelopes rather than raw
    waveforms because phase, loudspeaker distortion and clock drift between
    the two capture paths all break sample-level correlation while leaving
    the energy contour intact.
  text similarity — a system turn overlapping in time saying nearly the same
    words. One utterance cannot be spoken by two people at once, so the mic
    copy is a duplicate. Catches loud leakage the ratio test alone misses.

Validated against two real recordings (26 mic segments, hand-labeled): 25/26.
The single miss is a degenerate 13.5s Parakeet segment whose speech occupies
only the last moment, diluting its envelope correlation — and it misses
*conservatively*, keeping a leaked line rather than dropping a real one.
Every threshold below is deliberately biased that way: a stray duplicate is a
nuisance, a dropped sentence is unrecoverable.
"""

from __future__ import annotations

import wave
from dataclasses import dataclass
from difflib import SequenceMatcher
from pathlib import Path

import numpy as np

SAMPLE_RATE = 16000
#: 10ms energy-envelope frames.
_FRAME = 160
#: Leakage always *trails* the system channel; the negative side of the range
#: only covers segment-boundary jitter, not genuine anticipation.
_MIN_LAG_MS = -300
_MAX_LAG_MS = 600

#: Below this the system channel is effectively silent — nothing to leak.
SILENT_SYSTEM_RMS = 60.0
#: Leaked audio sits below the system channel; near-field speech does not.
MAX_BLEED_RATIO = 0.75
#: Envelope correlation that counts as "these channels move together".
MIN_BLEED_CORRELATION = 0.45
#: Weaker acoustic corroboration is enough when the text is near-identical.
MIN_WEAK_CORRELATION = 0.30
MIN_TEXT_SIMILARITY = 0.60
#: How far apart two turns may start and still be one utterance.
OVERLAP_TOLERANCE_MS = 1500
#: Windows shorter than this carry too little evidence to judge; kept as-is.
MIN_WINDOW_MS = 250


@dataclass(frozen=True)
class BleedVerdict:
    """Why a segment was judged leakage — carried into logs so a bad call can
    be diagnosed from a user's log without re-running the audio."""

    is_bleed: bool
    #: True when an overlapping system turn already carries this text, so the
    #: mic copy can be dropped outright. False means the content exists *only*
    #: on the mic channel and must be relabeled rather than deleted.
    has_duplicate: bool
    ratio: float
    correlation: float
    text_similarity: float


def read_wav_mono(path: str | Path) -> np.ndarray:
    """16-bit mono PCM as float64. Returns an empty array for an unreadable or
    empty file — a missing channel must degrade to "no detection", never raise
    into the transcription pipeline."""
    try:
        with wave.open(str(path)) as handle:
            frames = handle.readframes(handle.getnframes())
    except (OSError, wave.Error):
        return np.zeros(0)
    return np.frombuffer(frames, dtype=np.int16).astype(np.float64)


def _envelope(samples: np.ndarray) -> np.ndarray:
    frames = len(samples) // _FRAME
    if frames == 0:
        return np.zeros(0)
    return np.sqrt((samples[: frames * _FRAME].reshape(frames, _FRAME) ** 2).mean(axis=1))


def _best_envelope_correlation(mic: np.ndarray, system: np.ndarray) -> float:
    """Peak normalized correlation between two energy envelopes across the
    plausible lag range. The lag itself is not returned: it varies with output
    device and drifts within a recording, so only the peak is meaningful."""
    if len(mic) < 4 or len(system) < 4:
        return 0.0
    mic = mic - mic.mean()
    system = system - system.mean()
    if np.linalg.norm(mic) == 0 or np.linalg.norm(system) == 0:
        return 0.0

    best = 0.0
    for lag in range(_MIN_LAG_MS // 10, _MAX_LAG_MS // 10 + 1):
        if lag >= 0:
            left, right = mic[lag:], system[: len(system) - lag] if lag else system
        else:
            left, right = mic[: len(mic) + lag], system[-lag:]
        width = min(len(left), len(right))
        if width < 4:
            continue
        left, right = left[:width], right[:width]
        denominator = np.linalg.norm(left) * np.linalg.norm(right)
        if denominator == 0:
            continue
        best = max(best, float((left * right).sum() / denominator))
    return best


def _best_text_similarity(turn: dict, turns: list[dict]) -> float:
    best = 0.0
    for other in turns:
        if other.get("source") != "system":
            continue
        if abs(other["ts_start_ms"] - turn["ts_start_ms"]) > OVERLAP_TOLERANCE_MS:
            continue
        best = max(
            best,
            SequenceMatcher(None, turn["text"].lower(), other["text"].lower()).ratio(),
        )
    return best


def _slice(samples: np.ndarray, start_ms: int, end_ms: int) -> np.ndarray:
    start = max(0, int(start_ms / 1000 * SAMPLE_RATE))
    end = min(len(samples), int(end_ms / 1000 * SAMPLE_RATE))
    return samples[start:end] if end > start else np.zeros(0)


def classify_turn(
    turn: dict,
    turns: list[dict],
    mic: np.ndarray,
    system: np.ndarray,
) -> BleedVerdict:
    """Judge one mic-sourced turn. Callers pass system-sourced turns straight
    through — the system channel cannot contain microphone leakage (the mic is
    never routed back into the loopback capture)."""
    none = BleedVerdict(False, False, 0.0, 0.0, 0.0)
    if turn["ts_end_ms"] - turn["ts_start_ms"] < MIN_WINDOW_MS:
        return none

    mic_window = _slice(mic, turn["ts_start_ms"], turn["ts_end_ms"])
    system_window = _slice(system, turn["ts_start_ms"], turn["ts_end_ms"])
    if len(mic_window) == 0 or len(system_window) == 0:
        return none

    system_rms = float(np.sqrt((system_window**2).mean()))
    if system_rms < SILENT_SYSTEM_RMS:
        return none

    mic_rms = float(np.sqrt((mic_window**2).mean()))
    ratio = mic_rms / system_rms
    correlation = _best_envelope_correlation(_envelope(mic_window), _envelope(system_window))
    similarity = _best_text_similarity(turn, turns)

    acoustic = ratio < MAX_BLEED_RATIO and correlation > MIN_BLEED_CORRELATION
    textual = similarity >= MIN_TEXT_SIMILARITY and correlation > MIN_WEAK_CORRELATION
    return BleedVerdict(
        is_bleed=acoustic or textual,
        has_duplicate=similarity >= MIN_TEXT_SIMILARITY,
        ratio=ratio,
        correlation=correlation,
        text_similarity=similarity,
    )


def filter_bleed(
    turns: list[dict],
    mic_path: str | Path,
    system_path: str | Path,
) -> tuple[list[dict], list[dict]]:
    """Applies bleed detection to an already-merged turn list.

    Returns `(turns, dropped)`. A leaked turn whose words already appear on the
    system channel is **dropped** (the content survives on the correctly
    labeled "Them" turn). A leaked turn with no such counterpart is
    **relabeled** to "Them" instead — the system pass segmented it differently
    or missed it, so deleting it would destroy content that exists nowhere
    else. Either way no audio is touched and nothing is silently lost.
    """
    mic = read_wav_mono(mic_path)
    system = read_wav_mono(system_path)
    if len(mic) == 0 or len(system) == 0:
        return turns, []

    kept: list[dict] = []
    dropped: list[dict] = []
    for turn in turns:
        if turn.get("source") != "mic":
            kept.append(turn)
            continue

        verdict = classify_turn(turn, turns, mic, system)
        if not verdict.is_bleed:
            kept.append(turn)
            continue

        if verdict.has_duplicate:
            dropped.append(turn)
            continue

        kept.append(
            {
                **turn,
                "speaker_label": "Them",
                # Distinguishable from plain "source_file" so a reader (or a
                # later diarization pass) can tell this label was corrected
                # rather than taken at face value from the channel.
                "speaker_label_source": "cross_talk_corrected",
            }
        )
    return kept, dropped
