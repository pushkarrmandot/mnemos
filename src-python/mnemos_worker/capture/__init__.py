"""Windows WASAPI audio capture — mic + loopback, running on a dedicated OS
thread inside the persistent worker, off the single-slot job executor.
macOS capture is the Swift sidecar instead (`swift/mnemos-audio/`); nothing
in this package runs there.
"""
