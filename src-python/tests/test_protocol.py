import io
import threading

import pytest

from mnemos_worker.protocol import FramingError, encode_message, read_message, write_message


def test_round_trip():
    buf = io.BytesIO()
    write_message(buf, {"jsonrpc": "2.0", "id": "1", "method": "ping"}, threading.Lock())
    buf.seek(0)
    msg = read_message(buf)
    assert msg == {"jsonrpc": "2.0", "id": "1", "method": "ping"}


def test_eof_returns_none():
    buf = io.BytesIO(b"")
    assert read_message(buf) is None


def test_missing_content_length_raises():
    buf = io.BytesIO(b"X-Other: 1\r\n\r\n{}")
    with pytest.raises(FramingError):
        read_message(buf)


def test_truncated_body_raises():
    frame = encode_message({"hello": "world"})
    truncated = frame[:-2]
    buf = io.BytesIO(truncated)
    with pytest.raises(FramingError):
        read_message(buf)


def test_two_frames_do_not_interleave_with_shared_lock():
    buf = io.BytesIO()
    lock = threading.Lock()
    write_message(buf, {"a": 1}, lock)
    write_message(buf, {"b": 2}, lock)
    buf.seek(0)
    assert read_message(buf) == {"a": 1}
    assert read_message(buf) == {"b": 2}
