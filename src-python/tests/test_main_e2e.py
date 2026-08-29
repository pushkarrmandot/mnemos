"""End-to-end: spawns the real `python -m mnemos_worker` subprocess and
round-trips handshake + ping over real stdio framing.
"""

from __future__ import annotations

import subprocess
import sys
import threading
from pathlib import Path

from mnemos_worker.protocol import read_message, write_message


def _spawn(tmp_path: Path) -> subprocess.Popen:
    return subprocess.Popen(
        [sys.executable, "-m", "mnemos_worker", "--state-dir", str(tmp_path)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        cwd=Path(__file__).resolve().parents[1],
    )


def test_handshake_and_ping_round_trip(tmp_path: Path):
    proc = _spawn(tmp_path)
    lock = threading.Lock()
    try:
        write_message(proc.stdin, {"jsonrpc": "2.0", "id": "1", "method": "handshake"}, lock)
        reply = read_message(proc.stdout)
        assert reply["id"] == "1"
        assert reply["result"]["protocol_version"] == 1

        write_message(proc.stdin, {"jsonrpc": "2.0", "id": "2", "method": "ping"}, lock)
        reply = read_message(proc.stdout)
        assert reply["id"] == "2"
        assert reply["result"]["pong"] is True
    finally:
        write_message(proc.stdin, {"jsonrpc": "2.0", "method": "shutdown"}, lock)
        proc.stdin.close()
        proc.wait(timeout=5)


def test_unknown_method_returns_method_not_found(tmp_path: Path):
    proc = _spawn(tmp_path)
    lock = threading.Lock()
    try:
        write_message(proc.stdin, {"jsonrpc": "2.0", "id": "9", "method": "nope"}, lock)
        reply = read_message(proc.stdout)
        assert reply["id"] == "9"
        assert reply["error"]["code"] == -32601
    finally:
        proc.stdin.close()
        proc.wait(timeout=5)
