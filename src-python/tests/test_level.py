import numpy as np

from mnemos_worker.capture.level import FLOOR_DB, rms_dbfs


def test_silence_is_floor_db():
    silence = (b"\x00\x00") * 1600
    assert rms_dbfs(silence) == FLOOR_DB


def test_full_scale_square_wave_is_near_zero_db():
    samples = np.full(1600, 32767, dtype="<i2")
    db = rms_dbfs(samples.tobytes())
    assert -1.0 < db <= 0.0


def test_empty_buffer_is_floor_db():
    assert rms_dbfs(b"") == FLOOR_DB
