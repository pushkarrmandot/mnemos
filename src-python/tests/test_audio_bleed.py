"""W17c speaker-bleed detection (`mnemos_worker.audio_bleed`).

Signals are synthesized rather than loaded from fixtures: leakage is defined
by its *relationship* between two channels (attenuated, delayed, correlated),
which is exactly what a generated pair can express precisely and a checked-in
WAV cannot without shipping megabytes.
"""

from __future__ import annotations

import wave
from pathlib import Path

import numpy as np
import pytest

from mnemos_worker.audio_bleed import (
    SAMPLE_RATE,
    classify_turn,
    filter_bleed,
    read_wav_mono,
)


def _speech_like(seconds: float, seed: int, amplitude: float = 8000.0) -> np.ndarray:
    """Noise shaped by a slow syllable-rate envelope — enough structure for
    envelope correlation to be meaningful, unlike flat white noise."""
    rng = np.random.default_rng(seed)
    n = int(seconds * SAMPLE_RATE)
    carrier = rng.normal(0, 1, n)
    syllables = np.abs(np.sin(2 * np.pi * 3.5 * np.arange(n) / SAMPLE_RATE))
    return carrier * syllables * amplitude


def _write(path: Path, samples: np.ndarray) -> None:
    with wave.open(str(path), "wb") as handle:
        handle.setnchannels(1)
        handle.setsampwidth(2)
        handle.setframerate(SAMPLE_RATE)
        handle.writeframes(np.clip(samples, -32768, 32767).astype(np.int16).tobytes())


def _turn(text: str, start_ms: int, end_ms: int, source: str) -> dict:
    return {
        "text": text,
        "ts_start_ms": start_ms,
        "ts_end_ms": end_ms,
        "source": source,
        "speaker_label": "You" if source == "mic" else "Them",
        "speaker_label_source": "source_file",
        "contact_id": None,
    }


def test_detects_attenuated_delayed_leakage():
    """The real signature: same audio, ~200ms later, ~0.3x amplitude."""
    system = _speech_like(4.0, seed=1)
    delay = int(0.2 * SAMPLE_RATE)
    mic = np.concatenate([np.zeros(delay), system[:-delay]]) * 0.3

    turn = _turn("the same words", 0, 4000, "mic")
    verdict = classify_turn(turn, [turn], mic, system)

    assert verdict.is_bleed
    assert verdict.ratio < 1.0
    assert verdict.correlation > 0.45


def test_near_field_speech_over_playing_audio_is_kept():
    """Double-talk: the user speaks *while* system audio plays. Their voice is
    uncorrelated and louder than any leak, so it must survive — this is the
    failure mode the thresholds are biased against."""
    system = _speech_like(4.0, seed=2)
    leak = np.concatenate([np.zeros(3200), system[:-3200]]) * 0.3
    mic = leak + _speech_like(4.0, seed=99, amplitude=20000.0)

    turn = _turn("something entirely different", 0, 4000, "mic")
    assert not classify_turn(turn, [turn], mic, system).is_bleed


def test_silent_system_channel_is_always_genuine():
    """Nothing playing means nothing can leak, however quiet the mic is."""
    system = np.zeros(4 * SAMPLE_RATE)
    mic = _speech_like(4.0, seed=3, amplitude=200.0)

    turn = _turn("quiet but real", 0, 4000, "mic")
    verdict = classify_turn(turn, [turn], mic, system)

    assert not verdict.is_bleed
    assert verdict.correlation == 0.0


def test_loud_leakage_caught_by_matching_text():
    """Leakage loud enough to pass the ratio gate is still caught when an
    overlapping system turn carries the same words."""
    system = _speech_like(4.0, seed=4)
    mic = np.concatenate([np.zeros(3200), system[:-3200]]) * 0.9

    mic_turn = _turn("kind of got down from there", 0, 4000, "mic")
    system_turn = _turn("kind of got down from there", 200, 4200, "system")
    verdict = classify_turn(mic_turn, [mic_turn, system_turn], mic, system)

    assert verdict.is_bleed
    assert verdict.has_duplicate


def test_duplicate_leak_is_dropped_and_unique_leak_is_relabeled(tmp_path):
    """The two policies that keep content from being lost: drop only when the
    words already exist on the correctly-labeled system turn; otherwise
    relabel, because the mic copy is the only copy."""
    system_audio = _speech_like(8.0, seed=5)
    delay = int(0.2 * SAMPLE_RATE)
    mic_audio = np.concatenate([np.zeros(delay), system_audio[:-delay]]) * 0.3

    _write(tmp_path / "mic.wav", mic_audio)
    _write(tmp_path / "system.wav", system_audio)

    duplicated = _turn("we built it for three years", 0, 4000, "mic")
    counterpart = _turn("we built it for three years", 100, 4100, "system")
    unique = _turn("it's a headset that you can wear", 4000, 8000, "mic")

    kept, dropped = filter_bleed(
        [duplicated, counterpart, unique], tmp_path / "mic.wav", tmp_path / "system.wav"
    )

    assert dropped == [duplicated]
    assert counterpart in kept
    relabeled = [t for t in kept if t["ts_start_ms"] == 4000]
    assert len(relabeled) == 1
    assert relabeled[0]["speaker_label"] == "Them"
    assert relabeled[0]["speaker_label_source"] == "cross_talk_corrected"
    assert relabeled[0]["text"] == unique["text"]


def test_system_turns_are_never_reclassified(tmp_path):
    """The mic is never routed into loopback capture, so a system turn cannot
    be leakage — it passes through untouched even when it looks correlated."""
    audio = _speech_like(4.0, seed=6)
    _write(tmp_path / "mic.wav", audio)
    _write(tmp_path / "system.wav", audio)

    system_turn = _turn("from the far side", 0, 4000, "system")
    kept, dropped = filter_bleed([system_turn], tmp_path / "mic.wav", tmp_path / "system.wav")

    assert dropped == []
    assert kept == [system_turn]


@pytest.mark.parametrize("missing", ["mic.wav", "system.wav"])
def test_unreadable_channel_degrades_to_no_filtering(tmp_path, missing):
    """A missing or unreadable channel must return the transcript untouched,
    never raise — the transcript itself is still perfectly usable."""
    _write(tmp_path / "mic.wav", _speech_like(2.0, seed=7))
    _write(tmp_path / "system.wav", _speech_like(2.0, seed=8))
    (tmp_path / missing).unlink()

    turns = [_turn("still here", 0, 2000, "mic")]
    kept, dropped = filter_bleed(turns, tmp_path / "mic.wav", tmp_path / "system.wav")

    assert kept == turns
    assert dropped == []
    assert len(read_wav_mono(tmp_path / missing)) == 0


def test_very_short_turn_is_left_alone():
    """Sub-250ms windows carry too little evidence to judge safely."""
    system = _speech_like(1.0, seed=9)
    mic = system * 0.3

    turn = _turn("yeah", 0, 100, "mic")
    assert not classify_turn(turn, [turn], mic, system).is_bleed
