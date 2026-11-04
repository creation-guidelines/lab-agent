#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Tests for lab_agent/state.py: atomic save/load, and surviving a corrupt or missing file. Plain asserts: `python3 tests/test_state.py`."""
import json
import os
import sys
import tempfile
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
from lab_agent.state import State  # noqa: E402


def path():
    return os.path.join(tempfile.mkdtemp(), "state.json")


def test_a_missing_file_starts_empty():
    s = State(path())
    assert s.done == {} and s.running is None and s.resuming() is None


def test_a_corrupt_file_is_treated_as_empty_not_fatal():
    p = path()
    with open(p, "w") as f:
        f.write("{not valid json")
    s = State(p)
    assert s.done == {} and s.running is None


def test_job_started_then_finished_round_trips_through_a_fresh_load():
    p = path()
    s = State(p)
    s.job_started("0001-a", pid=1234)
    assert s.resuming() == {"name": "0001-a", "started_at": s.running["started_at"], "pid": 1234}
    s.job_finished("0001-a", ok=True, exit_code=0)
    assert s.running is None and s.is_done("0001-a") and "0001-a" in s.ok_names()

    reloaded = State(p)
    assert reloaded.is_done("0001-a") and "0001-a" in reloaded.ok_names() and reloaded.resuming() is None


def test_a_failed_job_is_done_but_not_ok():
    s = State(path())
    s.job_finished("0001-a", ok=False, exit_code=1)
    assert s.is_done("0001-a") and "0001-a" not in s.ok_names() and s.done_names() == {"0001-a"}


def test_the_file_is_written_atomically_no_tmp_file_left_behind():
    p = path()
    s = State(p)
    s.job_started("0001-a", pid=1)
    s.job_finished("0001-a", ok=True, exit_code=0)
    directory = os.path.dirname(p)
    assert os.listdir(directory) == ["state.json"], os.listdir(directory)
    with open(p) as f:
        json.load(f)  # must be valid, complete JSON, not a half-written temp file


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
