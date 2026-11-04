#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Tests for lab_agent/jobs.py: header parsing, discovery order, and dependency readiness. Plain asserts: `python3 tests/test_jobs.py`."""
import os
import sys
import tempfile
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
from lab_agent import jobs  # noqa: E402


def write(dir_, name, body):
    with open(os.path.join(dir_, name), "w") as f:
        f.write(body)


def test_a_plain_job_defaults_to_queue_mode_and_the_default_timeout():
    mode, timeout, dep = jobs.parse_header("#!/bin/sh\necho hi\n", default_timeout=300, max_timeout=9999)
    assert mode == "queue" and timeout == 300 and dep is None


def test_header_settings_are_read_and_timeout_is_clamped_to_the_maximum():
    mode, timeout, dep = jobs.parse_header("#!/bin/sh\n# lab: mode=preempt timeout=99999 depends_on=0012\necho hi\n", default_timeout=300, max_timeout=600)
    assert mode == "preempt" and timeout == 600 and dep == "0012"


def test_an_unknown_key_or_bad_mode_or_bad_timeout_is_a_job_error():
    for bad in ("# lab: nope=1\n", "# lab: mode=sideways\n", "# lab: timeout=abc\n", "# lab: timeout=-5\n", "# lab: justaword\n"):
        try:
            jobs.parse_header("#!/bin/sh\n" + bad, 300, 9999)
        except jobs.JobError:
            continue
        raise AssertionError(f"should have been a JobError: {bad!r}")


def test_the_header_is_only_read_from_the_top_before_real_content():
    # a "# lab:" line appearing after the script has started is not the header: it is just a comment in the body
    mode, _, _ = jobs.parse_header("#!/bin/sh\necho hi\n# lab: mode=preempt\n", 300, 9999)
    assert mode == "queue"


def test_discover_orders_by_number_and_ignores_files_that_do_not_match_the_pattern():
    d = tempfile.mkdtemp()
    write(d, "0002-second.sh", "#!/bin/sh\n")
    write(d, "0001-first.sh", "#!/bin/sh\n")
    write(d, "not-a-job.txt", "ignored")
    write(d, "readme.sh", "no number, ignored")
    found, errors = jobs.discover(d, 300, 9999)
    assert [j.name for j in found] == ["0001-first", "0002-second"], [j.name for j in found]
    assert errors == {}


def test_a_job_with_an_unparseable_header_is_reported_and_left_out_without_stopping_the_others():
    d = tempfile.mkdtemp()
    write(d, "0001-good.sh", "#!/bin/sh\n")
    write(d, "0002-bad.sh", "#!/bin/sh\n# lab: mode=sideways\n")
    found, errors = jobs.discover(d, 300, 9999)
    assert [j.name for j in found] == ["0001-good"]
    assert "0002-bad.sh" in errors and "sideways" in errors["0002-bad.sh"]


def test_next_job_skips_what_is_already_done():
    d = tempfile.mkdtemp()
    write(d, "0001-a.sh", "#!/bin/sh\n")
    write(d, "0002-b.sh", "#!/bin/sh\n")
    found, _ = jobs.discover(d, 300, 9999)
    assert jobs.next_job(found, done=set()).name == "0001-a"
    assert jobs.next_job(found, done={"0001-a"}).name == "0002-b"
    assert jobs.next_job(found, done={"0001-a", "0002-b"}) is None


def test_a_job_with_no_dependency_is_always_ready():
    d = tempfile.mkdtemp()
    write(d, "0001-a.sh", "#!/bin/sh\n")
    job = jobs.discover(d, 300, 9999)[0][0]
    is_ready, dep = jobs.ready(job, done=set(), ok=set())
    assert is_ready and dep is None


def test_a_job_waits_for_its_dependency_by_number_or_full_name():
    d = tempfile.mkdtemp()
    write(d, "0005-child.sh", "#!/bin/sh\n# lab: depends_on=0002\n")
    job = jobs.discover(d, 300, 9999)[0][0]
    assert jobs.ready(job, done=set(), ok=set()) == (False, None)
    assert jobs.ready(job, done={"0002-parent"}, ok=set()) == (False, "0002-parent")
    assert jobs.ready(job, done={"0002-parent"}, ok={"0002-parent"}) == (True, "0002-parent")


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
