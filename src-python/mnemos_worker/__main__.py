"""Entrypoint: `python -m mnemos_worker --state-dir ~/Mnemos/state`.

Handles `handshake`, `health_check`, `shutdown`; capture + live-transcript
subscribe/unsubscribe on the fast path (never queued behind a job); and job
kinds registered via `@method` (`ping`, `transcribe_final`). Extraction
(W11) registers the same way — this loop does not change shape when it
lands.
"""

from __future__ import annotations

import argparse
import sys
import threading
from pathlib import Path

from mnemos_worker import PROTOCOL_VERSION
from mnemos_worker.capture.manager import CAPTURE_METHODS, CaptureManager
from mnemos_worker.dispatch import DISPATCH_TABLE
from mnemos_worker.heartbeat import Heartbeat
from mnemos_worker.job_executor import JobExecutor
from mnemos_worker.jobs.live_transcription import LIVE_TRANSCRIPTION_METHODS, LiveTranscriptionManager
from mnemos_worker.logging_config import configure, get_logger
from mnemos_worker.models.transcription import MODEL_METHODS, ParakeetModel
from mnemos_worker.protocol import FramingError, read_message, write_message
from mnemos_worker.rpc_client import ReverseRpcClient
from mnemos_worker.rpc_client import configure as configure_rpc_client

# Importing registers each handler into DISPATCH_TABLE (§method decorator).
# Each future job kind (extraction, ...) adds one import here.
from mnemos_worker.jobs import ping as _ping  # noqa: F401,E402
from mnemos_worker.jobs import process_conversation as _process_conversation  # noqa: F401,E402
from mnemos_worker.jobs import extract_memory as _extract_memory  # noqa: F401,E402
from mnemos_worker.jobs import refresh_project_memory as _refresh_project_memory  # noqa: F401,E402

SHUTDOWN_GRACE_S = 30.0


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(prog="mnemos_worker")
    parser.add_argument(
        "--state-dir",
        type=Path,
        default=Path.home() / "Mnemos" / "state",
        help="Directory for pending_jobs.json / current_job.json",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    args.state_dir.mkdir(parents=True, exist_ok=True)

    configure()
    log = get_logger(component="mnemos-worker")

    stream_in = sys.stdin.buffer
    stream_out = sys.stdout.buffer
    write_lock = threading.Lock()

    def send(payload: dict) -> None:
        write_message(stream_out, payload, write_lock)

    def notify(method: str, params: dict) -> None:
        send({"jsonrpc": "2.0", "method": method, "params": params})

    def send_response(request_id: str, result: dict | None, error: dict | None) -> None:
        msg: dict = {"jsonrpc": "2.0", "id": request_id}
        if error is not None:
            msg["error"] = error
        else:
            msg["result"] = result if result is not None else {}
        send(msg)

    executor = JobExecutor(args.state_dir, on_response=send_response, logger=log)
    executor.start()

    # LLD-05 §4.2/§5.2 — lets a job-executor thread (`extract_memory`,
    # `refresh_project_memory`) call back into Rust via `run_agent_extraction`
    # and block for the reply; this read loop resolves that reply below.
    rpc_client = ReverseRpcClient(send)
    configure_rpc_client(rpc_client)

    heartbeat = Heartbeat(stream_out, write_lock)
    heartbeat.start()

    # Windows-only capture (LLD-03 §4.2): runs on its own dedicated thread,
    # off the job executor entirely (HLD §9.2), so a slow post-processing
    # job can never stall Start/Stop Recording. macOS uses the Swift
    # sidecar instead (`ipc::swift` on the Rust side) — nothing here runs
    # there, but the manager is harmless to construct on any OS since
    # `pyaudiowpatch` is only imported lazily inside the real stream
    # factory, never at import time.
    capture = CaptureManager(notify=notify)
    live_transcription = LiveTranscriptionManager(notify=notify)

    # Parakeet TDT 0.6B loaded once here, off the request path, so the first
    # live-transcription tick or `transcribe_final` call after Start isn't
    # also paying model-load latency (LLD-03 §7). Best-effort — see
    # `ParakeetModel.warm_up`'s docstring on why a load failure doesn't stop
    # the worker from starting. Backgrounded (W9 2026-08-22): real weight
    # loading takes far longer than a failed-import warm-up did, and this
    # call sits before the message loop below — synchronously here, it was
    # blocking the `handshake` response (and every other RPC) until the
    # model finished loading, well past callers' handshake timeouts. A
    # caller that races ahead of warm-up just pays the load latency on its
    # first real transcribe call instead (`ParakeetModel.get()` blocks on
    # the same `_executor.submit(...).result()` either way).
    threading.Thread(
        target=ParakeetModel.warm_up, kwargs={"notify": notify}, name="parakeet-warmup", daemon=True
    ).start()

    log.info("worker.started", protocol_version=PROTOCOL_VERSION, state_dir=str(args.state_dir))

    shutting_down = False
    while not shutting_down:
        try:
            msg = read_message(stream_in)
        except FramingError as exc:
            log.error("worker.framing_error", error=str(exc))
            break

        if msg is None:
            log.info("worker.stdin_eof")
            break

        method_name = msg.get("method")
        request_id = msg.get("id")

        if method_name is None:
            # No "method" + an "id" is a reply to one of *our* outbound
            # `run_agent_extraction` calls (LLD-05 §4.2) — the worker never
            # issues any other kind of forward request, so this is the only
            # thing an id-but-no-method frame can be.
            if request_id is not None and ("result" in msg or "error" in msg):
                if not rpc_client.resolve(str(request_id), msg.get("result"), msg.get("error")):
                    log.warning("worker.unmatched_rpc_reply", id=request_id)
            else:
                log.warning("worker.malformed_message", message=msg)
            continue

        if request_id is None:
            # Notification.
            if method_name == "shutdown":
                log.info("worker.shutdown_requested")
                shutting_down = True
            else:
                log.warning("worker.unknown_notification", method=method_name)
            continue

        # Request.
        if method_name == "handshake":
            send_response(request_id, {"protocol_version": PROTOCOL_VERSION}, None)
        elif method_name == "health_check":
            # Answered directly on the read loop, never queued behind a job —
            # otherwise a busy job would make the worker look "stuck" to
            # Rust's 500ms budget even though it's fine (BACKEND §2).
            send_response(request_id, {"ok": True}, None)
        elif method_name in CAPTURE_METHODS:
            # Answered directly, same reasoning as health_check above — a
            # slow post-processing job must never delay Start/Stop
            # Recording (HLD §9.2). Capture itself runs on its own thread,
            # not the job executor.
            try:
                result = CAPTURE_METHODS[method_name](capture, msg.get("params") or {})
                send_response(request_id, result, None)
            except Exception as exc:  # noqa: BLE001 — becomes a JSON-RPC error, not a crash
                send_response(request_id, None, {"code": -32000, "message": str(exc)})
        elif method_name in LIVE_TRANSCRIPTION_METHODS:
            # Same fast-path reasoning: subscribe/unsubscribe are Ack-only
            # (LLD-03 §3.2) and must not queue behind a slow transcribe_final
            # job — the live thread they start/stop runs independently.
            try:
                result = LIVE_TRANSCRIPTION_METHODS[method_name](live_transcription, msg.get("params") or {})
                send_response(request_id, result, None)
            except Exception as exc:  # noqa: BLE001 — becomes a JSON-RPC error, not a crash
                send_response(request_id, None, {"code": -32000, "message": str(exc)})
        elif method_name in MODEL_METHODS:
            # Fast-path: a status poll from onboarding must return instantly
            # even while a slow `transcribe_final` job is queued.
            try:
                result = MODEL_METHODS[method_name](msg.get("params") or {})
                send_response(request_id, result, None)
            except Exception as exc:  # noqa: BLE001 — becomes a JSON-RPC error, not a crash
                send_response(request_id, None, {"code": -32000, "message": str(exc)})
        elif method_name in DISPATCH_TABLE:
            executor.submit(request_id, method_name, msg.get("params") or {})
        else:
            send_response(
                request_id, None, {"code": -32601, "message": f"method not found: {method_name}"}
            )

    live_transcription.stop_all(timeout=10.0)
    executor.drain(SHUTDOWN_GRACE_S)
    heartbeat.stop()
    executor.stop()
    log.info("worker.exiting")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
