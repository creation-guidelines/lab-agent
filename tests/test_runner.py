#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Tests for lab_agent/runner.py: real processes, real process groups, real signals. Plain asserts: `python3 tests/test_runner.py`.
These are slower than the rest (a few seconds each, by design: they wait for real timeouts and real grace periods) but still a
few seconds total."""
import os
import signal
import sys
import tempfile
import time
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
from lab_agent.jobs import Job  # noqa: E402
from lab_agent.runner import Runner  # noqa: E402


def script(body):
    path = os.path.join(tempfile.mkdtemp(), "job.sh")
    open(path, "w").write(body)
    return path


def job(path, mode="queue", timeout=30):
    return Job(1, "test", path, mode, timeout, None)


def places():
    base = tempfile.mkdtemp()
    return os.path.join(base, "job.log"), os.path.join(base, "work")


def test_a_quick_job_runs_to_completion_and_poll_reports_its_real_exit_code():
    log, work = places()
    r = Runner(job(script("echo hello\nexit 7\n")), log, work, env=os.environ.copy())
    deadline = time.time() + 5
    code = None
    while time.time() < deadline and code is None:
        code = r.poll()
        time.sleep(0.05)
    assert code == 7, code
    assert "hello" in open(log).read()


def test_poll_returns_none_while_the_job_is_still_running():
    log, work = places()
    r = Runner(job(script("sleep 2\n")), log, work, env=os.environ.copy())
    assert r.poll() is None
    r.stop(grace_seconds=2)  # clean up: don't leave the sleep running past the test


def test_stdout_and_stderr_both_land_in_the_log_in_order_with_start_and_exit_markers():
    log, work = places()
    r = Runner(job(script("echo out-line\necho err-line >&2\n")), log, work, env=os.environ.copy())
    deadline = time.time() + 5
    while r.poll() is None and time.time() < deadline:
        time.sleep(0.05)
    text = open(log).read()
    assert "starting" in text and "out-line" in text and "err-line" in text and "exited with code 0" in text
    assert text.index("out-line") < text.index("exited with code")


def test_stop_sends_sigterm_and_a_well_behaved_job_exits_quickly_without_being_killed():
    log, work = places()
    r = Runner(job(script("trap 'echo got-term; exit 3' TERM\nsleep 30\n")), log, work, env=os.environ.copy())
    time.sleep(0.3)  # let the trap register
    t0 = time.time()
    code = r.stop(grace_seconds=5)
    elapsed = time.time() - t0
    assert code == 3, code
    assert elapsed < 4.5, f"should have exited promptly on SIGTERM, not waited out the grace period ({elapsed}s)"
    assert "got-term" in open(log).read()


def test_stop_sigkills_a_job_that_ignores_sigterm_after_the_grace_period():
    log, work = places()
    r = Runner(job(script("trap '' TERM\nsleep 30\n")), log, work, env=os.environ.copy())
    time.sleep(0.3)
    t0 = time.time()
    code = r.stop(grace_seconds=1)
    elapsed = time.time() - t0
    assert code == -signal.SIGKILL, code
    assert 1.0 <= elapsed < 3.0, f"should take roughly the grace period, not instant and not much longer: {elapsed}s"
    assert "SIGKILL" in open(log).read()


def test_stopping_an_already_finished_job_is_safe_and_returns_its_real_exit_code():
    log, work = places()
    r = Runner(job(script("exit 0\n")), log, work, env=os.environ.copy())
    deadline = time.time() + 5
    while r.poll() is None and time.time() < deadline:
        time.sleep(0.05)
    assert r.stop(grace_seconds=5) == 0


def test_stop_reaches_child_processes_spawned_by_the_job_not_just_the_shell():
    # the whole point of the process group: a job that backgrounds a child must not leave that child running after stop()
    log, work = places()
    marker = os.path.join(tempfile.mkdtemp(), "still-running")
    body = f"(trap '' TERM; while true; do sleep 0.1; done) &\ntouch {marker}\nwait\n"
    r = Runner(job(script(body)), log, work, env=os.environ.copy())
    deadline = time.time() + 5
    while not os.path.exists(marker) and time.time() < deadline:
        time.sleep(0.05)
    assert os.path.exists(marker), "the job never even started its child"
    r.stop(grace_seconds=1)
    time.sleep(0.3)
    # if the grandchild survived, its ppid would now be 1 (reparented) and it would still show up in ps; check via /proc scan
    import subprocess
    leftover = subprocess.run(["pgrep", "-f", "while true"], capture_output=True, text=True).stdout.strip()
    assert leftover == "", f"a child of the job is still running after stop(): pids {leftover}"


def test_the_job_receives_its_environment_including_job_specific_variables():
    log, work = places()
    env = dict(os.environ, JOB_NAME="0001-test", EXTRA_THING="banana")
    r = Runner(job(script('echo "name=$JOB_NAME extra=$EXTRA_THING"\n')), log, work, env=env)
    deadline = time.time() + 5
    while r.poll() is None and time.time() < deadline:
        time.sleep(0.05)
    assert "name=0001-test extra=banana" in open(log).read()


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
