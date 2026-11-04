#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Tests for lab_agent/writer.py: the results-tree layout, log chunking and tail-only pushing, and pruning old jobs.
Plain asserts: `python3 tests/test_writer.py`."""
import json
import os
import sys
import tempfile
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
from lab_agent import writer  # noqa: E402
from lab_agent.jobs import Job  # noqa: E402


def job(name="0001-probe", mode="queue", timeout=300, depends_on=None):
    number, slug = name.split("-", 1)
    return Job(int(number), slug, path=f"/jobs/{name}.sh", mode=mode, timeout=timeout, depends_on=depends_on)


def test_top_status_is_written_as_one_json_object():
    d = tempfile.mkdtemp()
    writer.write_top_status(d, agent_version="0.1.0", running="0001-probe", last_heartbeat=123.0, done_count=4, host="lab1")
    data = json.load(open(os.path.join(d, "status.json")))
    assert data["running"] == "0001-probe" and data["done_count"] == 4 and data["host"] == "lab1"


def test_job_status_merges_fields_across_writes_instead_of_overwriting():
    d = tempfile.mkdtemp()
    j = job()
    writer.write_job_status(d, j, started_at=100.0, running=True)
    writer.write_job_status(d, j, finished_at=150.0, running=False, ok=True, exit_code=0)
    data = json.load(open(os.path.join(d, "jobs", j.name, "status.json")))
    assert data["started_at"] == 100.0 and data["finished_at"] == 150.0 and data["ok"] is True
    assert data["mode"] == "queue" and data["timeout"] == 300, "the job's own settings are always included"


def test_a_corrupt_existing_status_file_does_not_stop_the_next_write():
    d = tempfile.mkdtemp()
    j = job()
    path = os.path.join(d, "jobs", j.name, "status.json")
    os.makedirs(os.path.dirname(path))
    open(path, "w").write("{not json")
    writer.write_job_status(d, j, running=True)  # must not raise
    data = json.load(open(path))
    assert data["running"] is True


def test_a_short_log_is_one_chunk_with_no_omission_note():
    d = tempfile.mkdtemp()
    log = os.path.join(tempfile.mkdtemp(), "job.log")
    open(log, "w").write("hello\nworld\n")
    writer.sync_log(d, "0001-probe", log, chunk_bytes=1000, max_pushed_bytes=1000)
    chunks = sorted(os.listdir(os.path.join(d, "jobs", "0001-probe", "log")))
    assert chunks == ["0000.txt"]
    assert open(os.path.join(d, "jobs", "0001-probe", "log", "0000.txt")).read() == "hello\nworld\n"


def test_a_long_log_is_split_into_several_chunks_in_order():
    d = tempfile.mkdtemp()
    log = os.path.join(tempfile.mkdtemp(), "job.log")
    open(log, "w").write("x" * 2500)
    writer.sync_log(d, "0001-probe", log, chunk_bytes=1000, max_pushed_bytes=10000)
    log_dir = os.path.join(d, "jobs", "0001-probe", "log")
    chunks = sorted(os.listdir(log_dir))
    assert chunks == ["0000.txt", "0001.txt", "0002.txt"], chunks
    sizes = [os.path.getsize(os.path.join(log_dir, c)) for c in chunks]
    assert sizes == [1000, 1000, 500], sizes


def test_only_the_tail_of_a_log_beyond_the_limit_is_pushed_with_a_note():
    d = tempfile.mkdtemp()
    log = os.path.join(tempfile.mkdtemp(), "job.log")
    open(log, "w").write("a" * 500 + "b" * 500)  # 1000 bytes total
    writer.sync_log(d, "0001-probe", log, chunk_bytes=10000, max_pushed_bytes=500)
    content = open(os.path.join(d, "jobs", "0001-probe", "log", "0000.txt")).read()
    assert "500 earlier bytes omitted" in content
    assert content.endswith("b" * 500)
    assert "a" not in content.split("\n", 1)[1]  # none of the omitted "a" bytes leaked into the pushed part


def test_resyncing_a_shrunk_log_removes_the_stale_trailing_chunk_files():
    d = tempfile.mkdtemp()
    log = os.path.join(tempfile.mkdtemp(), "job.log")
    open(log, "w").write("x" * 2500)
    writer.sync_log(d, "0001-probe", log, chunk_bytes=1000, max_pushed_bytes=10000)
    assert len(os.listdir(os.path.join(d, "jobs", "0001-probe", "log"))) == 3
    # simulate pushing a shorter tail (as happens once a huge log exceeds max_pushed_bytes and only the tail is kept)
    writer.sync_log(d, "0001-probe", log, chunk_bytes=1000, max_pushed_bytes=600)
    remaining = sorted(os.listdir(os.path.join(d, "jobs", "0001-probe", "log")))
    assert remaining == ["0000.txt"], remaining


def test_prune_removes_only_the_oldest_jobs_beyond_the_limit():
    d = tempfile.mkdtemp()
    for name in ("0001-a", "0002-b", "0003-c", "0004-d"):
        os.makedirs(os.path.join(d, "jobs", name))
    writer.prune(d, keep_results=2, finished_order=["0001-a", "0002-b", "0003-c", "0004-d"])
    remaining = sorted(os.listdir(os.path.join(d, "jobs")))
    assert remaining == ["0003-c", "0004-d"], remaining


def test_prune_does_nothing_when_within_the_limit():
    d = tempfile.mkdtemp()
    os.makedirs(os.path.join(d, "jobs", "0001-a"))
    writer.prune(d, keep_results=50, finished_order=["0001-a"])
    assert os.listdir(os.path.join(d, "jobs")) == ["0001-a"]


TESTS = [v for k, v in sorted(globals().items()) if k.startswith("test_") and callable(v)]

if __name__ == "__main__":
    failed = 0
    for t in TESTS:
        try:
            t()
            print("ok  ", t.__name__)
        except Exception:
            failed += 1
            print("FAIL", t.__name__)
            traceback.print_exc()
    print(f"{len(TESTS) - failed}/{len(TESTS)} passed")
    sys.exit(1 if failed else 0)
