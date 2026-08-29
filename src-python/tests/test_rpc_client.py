import threading
import time

import pytest

from mnemos_worker.rpc_client import ReverseRpcClient, ReverseRpcError


def test_call_blocks_until_resolved_from_another_thread():
    sent = []
    client = ReverseRpcClient(send=sent.append)

    def responder():
        while not sent:
            time.sleep(0.005)
        assert client.resolve(sent[0]["id"], {"answer": 42}, None)

    t = threading.Thread(target=responder)
    t.start()
    result = client.call("run_agent_extraction", {"prompt": "x"}, timeout_s=2)
    t.join()
    assert result == {"answer": 42}


def test_call_raises_reverse_rpc_error_on_error_reply():
    sent = []
    client = ReverseRpcClient(send=sent.append)

    def responder():
        while not sent:
            time.sleep(0.005)
        client.resolve(sent[0]["id"], None, {"code": -32020, "message": "timeout"})

    t = threading.Thread(target=responder)
    t.start()
    with pytest.raises(ReverseRpcError) as exc_info:
        client.call("run_agent_extraction", {}, timeout_s=2)
    t.join()
    assert exc_info.value.code == -32020


def test_call_times_out_without_a_reply():
    client = ReverseRpcClient(send=lambda payload: None)
    with pytest.raises(ReverseRpcError) as exc_info:
        client.call("run_agent_extraction", {}, timeout_s=0.05)
    assert exc_info.value.code == -32020


def test_resolve_unknown_id_returns_false():
    client = ReverseRpcClient(send=lambda payload: None)
    assert client.resolve("rpc-999", {}, None) is False


def test_request_ids_are_disjoint_from_forward_call_ids():
    sent = []
    client = ReverseRpcClient(send=sent.append)

    def responder():
        while not sent:
            time.sleep(0.005)
        client.resolve(sent[0]["id"], {}, None)

    t = threading.Thread(target=responder)
    t.start()
    client.call("run_agent_extraction", {}, timeout_s=2)
    t.join()
    assert sent[0]["id"].startswith("rpc-")
