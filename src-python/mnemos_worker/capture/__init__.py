"""Windows WASAPI audio capture (LLD-03 §4.2) — mic + loopback, running on a
dedicated OS thread inside the persistent worker, off the single-slot job
executor (HLD §9.2). macOS capture is the Swift sidecar instead
(`swift/mnemos-audio/`); nothing in this package runs there.
"""
