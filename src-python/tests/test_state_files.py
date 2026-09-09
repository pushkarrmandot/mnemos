"""Atomic JSON writes for the worker's crash-recovery state files. Rust only
reads these after a restart, so the guarantees that matter are: a reader
never sees a half-written file, and removing a file that never existed is a
no-op rather than an error.
"""

import json

from mnemos_worker.state_files import atomic_write_json, remove_if_exists


def test_atomic_write_json_creates_parent_directories(tmp_path):
    path = tmp_path / "nested" / "dir" / "current_job.json"
    atomic_write_json(path, {"job_id": "abc"})
    assert path.exists()
    assert json.loads(path.read_text()) == {"job_id": "abc"}


def test_atomic_write_json_leaves_no_tmp_file_behind(tmp_path):
    path = tmp_path / "pending_jobs.json"
    atomic_write_json(path, [{"job_id": "1"}])
    tmp = path.with_suffix(path.suffix + ".tmp")
    assert not tmp.exists()
    assert list(tmp_path.iterdir()) == [path]


def test_atomic_write_json_overwrites_existing_file_content(tmp_path):
    path = tmp_path / "current_job.json"
    atomic_write_json(path, {"job_id": "old"})
    atomic_write_json(path, {"job_id": "new"})
    assert json.loads(path.read_text()) == {"job_id": "new"}


def test_atomic_write_json_round_trips_various_payload_shapes(tmp_path):
    path = tmp_path / "pending_jobs.json"
    payload = [{"job_id": "1", "kind": "transcribe"}, {"job_id": "2", "kind": "extract"}]
    atomic_write_json(path, payload)
    assert json.loads(path.read_text()) == payload


def test_remove_if_exists_deletes_an_existing_file(tmp_path):
    path = tmp_path / "current_job.json"
    path.write_text("{}")
    remove_if_exists(path)
    assert not path.exists()


def test_remove_if_exists_is_a_noop_when_file_is_absent(tmp_path):
    path = tmp_path / "does_not_exist.json"
    # Must not raise FileNotFoundError.
    remove_if_exists(path)
    assert not path.exists()
