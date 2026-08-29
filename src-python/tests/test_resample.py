import numpy as np

from mnemos_worker.capture.resample import resample_to_16k_mono


def test_downsamples_48khz_stereo_to_16khz_mono_length():
    duration_s = 1.0
    n = int(48000 * duration_s)
    stereo = np.zeros((n, 2), dtype="<i2")
    stereo[:, 0] = 1000
    stereo[:, 1] = -1000
    pcm = stereo.tobytes()

    out = resample_to_16k_mono(pcm, sample_rate=48000, channels=2)
    out_samples = np.frombuffer(out, dtype="<i2")

    # ~16000 samples for 1s of audio at the target rate.
    assert abs(len(out_samples) - 16000) <= 2
    # Stereo (1000, -1000) downmixes to ~0.
    assert abs(int(out_samples[100])) < 50


def test_passthrough_when_already_16khz_mono():
    samples = np.array([100, -100, 200, -200], dtype="<i2")
    out = resample_to_16k_mono(samples.tobytes(), sample_rate=16000, channels=1)
    assert np.frombuffer(out, dtype="<i2").tolist() == samples.tolist()


def test_empty_input_is_empty_output():
    assert resample_to_16k_mono(b"", sample_rate=48000, channels=2) == b""
