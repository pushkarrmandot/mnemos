"""Post-processing transcription phase (LLD-03 §6) — the `transcribing`
step of the eventual `process_conversation` job. This module owns only that
one phase: two full-file Parakeet passes (one per source WAV) merged by
timestamp into `transcript.json`, "You" (mic)/"Them" (system) labeled by
source file only (v1 tier — no diarization model, see LLD-03 §0b/HLD gaps).
Later steps (diarization, extraction, summary) belong to LLD-04/LLD-05 —
W10/W11 — and are not built here; no `process_conversation` *orchestrator*
exists yet either (that's whichever wave first chains steps a-h together).

Registered as the `transcribe_final` RPC method (LLD-03 §3.2): runs through
the job executor (not the capture/live-transcript fast path) because a
full-file Parakeet pass can take up to ~30s (HLD NFR budget) and must not
block Start/Stop Recording, but *can* be queued like any other job.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from mnemos_worker.audio_bleed import filter_bleed
from mnemos_worker import job_progress
from mnemos_worker.dispatch import method
from mnemos_worker.logging_config import get_logger
from mnemos_worker.models.transcription import ParakeetModel, Segment
from mnemos_worker.state_files import atomic_write_json

SCHEMA_VERSION = 1

log = get_logger(component="process-conversation")


def merge_transcripts(mic: list[Segment], system: list[Segment]) -> list[dict[str, Any]]:
    """Deterministic k-way merge on start timestamps (LLD-03 §6.2). Stable
    tiebreak: mic wins when starts collide. No cross-source de-duplication —
    diarization (LLD-04) is what collapses overlapping mic/system segments
    that are actually the same speaker."""
    tagged: list[tuple[Segment, str]] = [(s, "mic") for s in mic] + [(s, "system") for s in system]
    tagged.sort(key=lambda st: (st[0].ts_start_ms, 0 if st[1] == "mic" else 1))
    speaker_label = {"mic": "You", "system": "Them"}
    return [
        {
            "text": seg.text,
            "ts_start_ms": seg.ts_start_ms,
            "ts_end_ms": seg.ts_end_ms,
            "source": source,
            "speaker_label": speaker_label[source],
            # v1 has no diarization model (LLD-03 §0b) — the label above is
            # pure source-file attribution, not a Parakeet hint or
            # diarization output, so neither of LLD-03 §6.2's other
            # `speaker_label_source` values apply yet.
            "speaker_label_source": "source_file",
            "contact_id": None,
        }
        for seg, source in tagged
    ]


def _default_transcribe_file(path: str, on_progress=None) -> list[Segment]:
    return ParakeetModel.get().transcribe_file(path, on_progress)


def _file_progress(file_index: int, file_count: int):
    """Maps one file's `(processed, total)` samples onto overall progress
    across all files, so two sequential passes read as 0→100% once rather
    than 0→100% twice."""

    def report(processed: int, total: int) -> None:
        if total <= 0:
            return
        job_progress.report(
            "transcribe",
            (file_index + processed / total) / file_count,
            file_index=file_index,
            file_count=file_count,
        )

    return report


@method("transcribe_final")
def transcribe_final(params: dict[str, Any]) -> dict[str, Any]:
    conversation_id = params["conversation_id"]
    mic_path = params["mic_path"]
    system_path = params["system_path"]

    for path in (mic_path, system_path):
        if not Path(path).exists():
            raise FileNotFoundError(f"transcribe_final: missing source WAV: {path}")

    # DEVIATION from LLD-03 §3.2's request table (which lists only
    # `{conversation_id, mic_path, system_path}`, no explicit transcript
    # path): `transcript.json` is a sibling of `mic.wav` in the conversation
    # blob dir (LLD-01's `conversation_dir` groups all per-conversation
    # files together), so it's derived here rather than added as a new
    # request param.
    transcript_path = Path(mic_path).parent / "transcript.json"

    # Progress is reported per chunk so the request's deadline tracks
    # *silence* rather than total duration — a 33-minute recording must not
    # be indistinguishable from a wedged worker (see `job_progress`).
    mic_segments = _default_transcribe_file(mic_path, _file_progress(0, 2))
    system_segments = _default_transcribe_file(system_path, _file_progress(1, 2))
    # Explicit completion tick: the MLX backend skips its callback entirely
    # when the audio is shorter than one chunk, so short recordings would
    # otherwise report no progress at all and then jump straight to the
    # result. Also covers the remaining merge/bleed-filter work below.
    job_progress.report("transcribe", 1.0, file_index=2, file_count=2)

    turns = merge_transcripts(mic_segments, system_segments)

    # W17c: strip speaker bleed before anything downstream sees the transcript.
    # On speakers the far side's voice reaches the microphone and gets
    # transcribed a second time as "You" — and this file feeds summary and
    # extraction, so a leaked turn becomes an action item the user never
    # agreed to. Best-effort: a detection failure must never cost the user a
    # transcript that otherwise transcribed fine, so fall back to the
    # unfiltered merge rather than propagating.
    try:
        turns, dropped = filter_bleed(turns, mic_path, system_path)
        if dropped:
            log.info(
                "transcribe_final.bleed_filtered",
                conv=conversation_id,
                dropped=len(dropped),
                kept=len(turns),
            )
    except Exception as exc:  # noqa: BLE001 — quality feature, never fatal
        log.warning("transcribe_final.bleed_filter_failed", conv=conversation_id, error=str(exc))

    duration_ms = max((t["ts_end_ms"] for t in turns), default=0)

    transcript = {
        "schema_version": SCHEMA_VERSION,
        "conversation_id": conversation_id,
        "duration_ms": duration_ms,
        "turns": turns,
    }
    atomic_write_json(transcript_path, transcript)

    return {
        "transcript_path": str(transcript_path),
        "segment_count": len(turns),
        "duration_ms": duration_ms,
    }
