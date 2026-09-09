"""`Heartbeat` emits `{"method": "heartbeat"}` frames on a fixed interval and
stops cleanly. These tests use a tiny `interval_s` so the thread ticks a few
times within a normal test timeout, and read frames back with the real
`protocol.read_message` so the assertions pin the actual wire format, not an
internal representation of it.
"""

import io
import threading
import time

from mnemos_worker.heartbeat import Heartbeat
from mnemos_worker.protocol import read_message


def test_heartbeat_emits_frames_on_stream_at_the_configured_interval():
    stream = io.BytesIO()
    hb = Heartbeat(stream=stream, interval_s=0.02)
    hb.start()
    time.sleep(0.09)
    hb.stop()
    hb._thread.join(timeout=1)

    stream.seek(0)
    messages = []
    while True:
        msg = read_message(stream)
        if msg is None:
            break
        messages.append(msg)

    assert len(messages) >= 2
    for msg in messages:
        assert msg == {"jsonrpc": "2.0", "method": "heartbeat"}


def test_stop_prevents_further_ticks():
    stream = io.BytesIO()
    hb = Heartbeat(stream=stream, interval_s=0.02)
    hb.start()
    time.sleep(0.05)
    hb.stop()
    hb._thread.join(timeout=1)

    stream.seek(0, io.SEEK_END)
    size_after_stop = stream.tell()
    time.sleep(0.06)
    assert stream.tell() == size_after_stop, "no frames should be written after stop()"


def test_stop_before_first_interval_elapses_emits_nothing():
    stream = io.BytesIO()
    hb = Heartbeat(stream=stream, interval_s=10.0)
    hb.start()
    hb.stop()
    hb._thread.join(timeout=1)
    assert stream.getvalue() == b""


def test_writes_are_serialized_through_the_shared_lock():
    # A lock passed by the caller must be the one actually used for writes,
    # since production wiring shares one lock across the heartbeat thread
    # and the main writer thread to avoid interleaved frames.
    stream = io.BytesIO()
    lock = threading.Lock()
    hb = Heartbeat(stream=stream, lock=lock, interval_s=0.02)
    assert hb._lock is lock

    hb.start()
    time.sleep(0.05)
    hb.stop()
    hb._thread.join(timeout=1)
    # If the lock were ignored, this would still likely pass on a single
    # writer thread; the real guarantee is `hb._lock is lock` above plus no
    # exception here (writes actually happened under contention-free use).
    assert stream.tell() > 0
