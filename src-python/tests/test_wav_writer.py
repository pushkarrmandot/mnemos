import wave

from mnemos_worker.capture.wav_writer import ChunkedWavWriter


def test_flush_produces_a_readable_wav(tmp_path):
    path = tmp_path / "mic.wav"
    writer = ChunkedWavWriter(str(path), sample_rate=16000, channels=1, sample_width=2)
    pcm = (b"\x01\x00" * 1600) * 3  # 300ms of fake int16 samples
    writer.append(pcm)
    flushed = writer.flush()
    assert flushed == len(pcm)

    with wave.open(str(path), "rb") as w:
        assert w.getframerate() == 16000
        assert w.getnchannels() == 1
        assert w.getsampwidth() == 2
        assert w.getnframes() == len(pcm) // 2


def test_header_is_valid_even_without_close_crash_simulation(tmp_path):
    path = tmp_path / "mic.wav"
    writer = ChunkedWavWriter(str(path))
    writer.append(b"\x02\x00" * 100)
    writer.flush()
    # Simulate a crash: never call close(). The header on disk must still
    # declare the real data size (LLD-03 §4.1's crash-safety property).
    with wave.open(str(path), "rb") as w:
        assert w.getnframes() == 100


def test_close_flushes_pending_bytes():
    import tempfile

    with tempfile.TemporaryDirectory() as d:
        path = f"{d}/mic.wav"
        writer = ChunkedWavWriter(path)
        writer.append(b"\x03\x00" * 50)
        writer.close()
        with wave.open(path, "rb") as w:
            assert w.getnframes() == 50


def test_empty_flush_is_a_noop(tmp_path):
    writer = ChunkedWavWriter(str(tmp_path / "mic.wav"))
    assert writer.flush() == 0
    writer.close()
