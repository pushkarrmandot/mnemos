import json

import pytest

from mnemos_worker.jobs import process_conversation
from mnemos_worker.jobs.process_conversation import merge_transcripts, transcribe_final
from mnemos_worker.models.transcription import Segment


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
