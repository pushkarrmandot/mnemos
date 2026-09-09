import json
import wave

import pytest

from mnemos_worker.jobs import process_conversation
from mnemos_worker.jobs.process_conversation import merge_transcripts, transcribe_final
from mnemos_worker.models.transcription import Segment


def _write_wav(path, frames: int) -> None:
    """A real, well-formed WAV — matches what a capture backend that
    delivered zero buffers actually writes on disk (header, no samples),
    not an arbitrary invalid file."""
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(16000)
        if frames:
            w.writeframes(b"\x00\x00" * frames)


def test_merge_transcripts_orders_by_start_ts_mic_wins_ties():
    mic = [Segment(text="mic-a", ts_start_ms=1000, ts_end_ms=1500)]
    system = [
        Segment(text="sys-a", ts_start_ms=1000, ts_end_ms=1600),
        Segment(text="sys-b", ts_start_ms=500, ts_end_ms=900),
    ]
    turns = merge_transcripts(mic, system)

    assert [t["text"] for t in turns] == ["sys-b", "mic-a", "sys-a"]
    assert turns[1]["source"] == "mic"
    assert turns[1]["speaker_label"] == "You"
    assert turns[2]["source"] == "system"
    assert turns[2]["speaker_label"] == "Them"
    assert all(t["speaker_label_source"] == "source_file" for t in turns)
    assert all(t["contact_id"] is None for t in turns)


def test_merge_transcripts_empty_inputs():
    assert merge_transcripts([], []) == []


def test_transcribe_final_writes_transcript_json(tmp_path, monkeypatch):
    mic_path = tmp_path / "mic.wav"
    system_path = tmp_path / "system.wav"
    mic_path.write_bytes(b"\x00")
    system_path.write_bytes(b"\x00")

    def fake_transcribe_file(path: str, on_progress=None):
        if on_progress is not None:
            on_progress(1, 1)
        if path == str(mic_path):
            return [Segment(text="hi there", ts_start_ms=0, ts_end_ms=1000)]
        return [Segment(text="hello back", ts_start_ms=500, ts_end_ms=2000)]

    monkeypatch.setattr(process_conversation, "_default_transcribe_file", fake_transcribe_file)

    result = transcribe_final(
        {
            "conversation_id": "conv-1",
            "mic_path": str(mic_path),
            "system_path": str(system_path),
        }
    )

    transcript_path = tmp_path / "transcript.json"
    assert result["transcript_path"] == str(transcript_path)
    assert result["segment_count"] == 2
    assert result["duration_ms"] == 2000

    on_disk = json.loads(transcript_path.read_text())
    assert on_disk["schema_version"] == 1
    assert on_disk["conversation_id"] == "conv-1"
    assert on_disk["duration_ms"] == 2000
    assert [t["text"] for t in on_disk["turns"]] == ["hi there", "hello back"]
    assert on_disk["turns"][0]["source"] == "mic"
    assert on_disk["turns"][0]["speaker_label"] == "You"
    assert on_disk["turns"][1]["source"] == "system"
    assert on_disk["turns"][1]["speaker_label"] == "Them"


def test_transcribe_final_raises_if_source_wav_missing(tmp_path):
    mic_path = tmp_path / "mic.wav"
    mic_path.write_bytes(b"\x00")
    missing_system = tmp_path / "system.wav"

    with pytest.raises(FileNotFoundError):
        transcribe_final(
            {
                "conversation_id": "conv-1",
                "mic_path": str(mic_path),
                "system_path": str(missing_system),
            }
        )


def test_has_audio_frames_true_for_real_audio(tmp_path):
    path = tmp_path / "mic.wav"
    _write_wav(path, frames=1000)
    assert process_conversation._has_audio_frames(str(path)) is True


def test_has_audio_frames_false_for_header_only_wav(tmp_path):
    """The exact shape of the bug: a capture backend that delivered zero
    buffers for a whole recording still writes a well-formed WAV header, so
    the file exists and opens fine — it just has no frames."""
    path = tmp_path / "system.wav"
    _write_wav(path, frames=0)
    assert process_conversation._has_audio_frames(str(path)) is False


def test_has_audio_frames_false_for_unparseable_file(tmp_path):
    path = tmp_path / "system.wav"
    path.write_bytes(b"not a wav")
    assert process_conversation._has_audio_frames(str(path)) is False


def test_default_transcribe_file_skips_the_model_for_empty_audio(tmp_path, monkeypatch):
    path = tmp_path / "system.wav"
    _write_wav(path, frames=0)

    def fail_if_called(*_args, **_kwargs):
        raise AssertionError("the model must not be invoked on an empty stream")

    monkeypatch.setattr(
        process_conversation.ParakeetModel, "get", staticmethod(lambda: type(
            "M", (), {"transcribe_file": fail_if_called}
        )())
    )

    assert process_conversation._default_transcribe_file(str(path)) == []


def test_transcribe_final_survives_one_empty_stream(tmp_path, monkeypatch):
    """Reproduces a real production failure: a ~1-minute recording where the
    system-audio backend delivered zero buffers (see the Core Audio process
    tap's startup probe) produced a well-formed, zero-frame `system.wav`
    alongside a normal `mic.wav`. Before this fix, handing the empty file to
    Parakeet raised `[as_strided] Negative dimensions not allowed`, which
    aborted the whole job — discarding the mic transcript too, even though
    it had transcribed correctly — and left the user stuck on a permanent
    "storage error" retry loop because `transcript.json` was never written.

    One empty stream must degrade to "half the transcript," never to
    "no transcript."
    """
    mic_path = tmp_path / "mic.wav"
    system_path = tmp_path / "system.wav"
    _write_wav(mic_path, frames=16000 * 41)  # ~41s, matching the field case
    _write_wav(system_path, frames=0)

    def fake_transcribe_file(path: str, on_progress=None):
        if path == str(system_path):
            raise AssertionError("must not reach the model for an empty stream")
        if on_progress is not None:
            on_progress(1, 1)
        return [Segment(text="hi there", ts_start_ms=0, ts_end_ms=1000)]

    monkeypatch.setattr(process_conversation, "_default_transcribe_file", lambda path, on_progress=None: (
        [] if path == str(system_path) else fake_transcribe_file(path, on_progress)
    ))

    result = transcribe_final(
        {
            "conversation_id": "conv-1",
            "mic_path": str(mic_path),
            "system_path": str(system_path),
        }
    )

    assert result["segment_count"] == 1
    on_disk = json.loads((tmp_path / "transcript.json").read_text())
    assert [t["text"] for t in on_disk["turns"]] == ["hi there"]
    assert on_disk["turns"][0]["source"] == "mic"
