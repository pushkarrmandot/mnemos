"""Maps WASAPI failures to the shared `SidecarErrorKind` vocabulary so the
cross-platform `capture_event{kind:"error", error_kind:...}` shape never
leaks a raw Windows HRESULT to Rust.
"""

from __future__ import annotations


def classify_wasapi_error(exc: OSError) -> str:
    text = str(exc)
    if "AUDCLNT_E_DEVICE_INVALIDATED" in text:
        return "mic_disconnected"
    if "ERROR_DISK_FULL" in text or getattr(exc, "errno", None) == 28:  # ENOSPC
        return "disk_full"
    if "E_OUTOFMEMORY" in text:
        return "encoder_error"
    return "encoder_error"
