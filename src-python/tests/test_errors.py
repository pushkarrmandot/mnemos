from mnemos_worker.capture.errors import classify_wasapi_error


def test_device_invalidated_maps_to_mic_disconnected():
    exc = OSError("AUDCLNT_E_DEVICE_INVALIDATED: device gone")
    assert classify_wasapi_error(exc) == "mic_disconnected"


def test_disk_full_maps_to_disk_full():
    exc = OSError("ERROR_DISK_FULL")
    assert classify_wasapi_error(exc) == "disk_full"


def test_enospc_errno_maps_to_disk_full():
    exc = OSError(28, "No space left on device")
    assert classify_wasapi_error(exc) == "disk_full"


def test_unknown_error_defaults_to_encoder_error():
    exc = OSError("something else entirely")
    assert classify_wasapi_error(exc) == "encoder_error"
