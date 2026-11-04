#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""End-to-end tests for lab_agent/agent.py: the whole loop, against real local git repositories standing in for GitHub (see
test_gitio.py for why). No background thread and no sleep-based polling loop is used here: the test calls ``agent._pass()``
itself, as many times as it needs, which is exactly the one method ``run_forever`` calls in a loop - so these tests exercise the
real code path, just driven by hand instead of by a timer. Plain asserts: `python3 tests/test_agent.py` (a few seconds each)."""
import json
import os
import subprocess
import sys
import tempfile
import time
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
from lab_agent.agent import Agent  # noqa: E402
from lab_agent.config import Config  # noqa: E402

ENV = dict(os.environ, GIT_AUTHOR_NAME="test", GIT_AUTHOR_EMAIL="t@example.com",
          GIT_COMMITTER_NAME="test", GIT_COMMITTER_EMAIL="t@example.com")


def sh(*args, cwd=None):
    result = subprocess.run(["git", *args], cwd=cwd, env=ENV, capture_output=True, text=True, timeout=10)
    assert result.returncode == 0, (args, result.stderr)
    return result.stdout


def bare_remote(branch, seed_files):
    remote = tempfile.mkdtemp()
    sh("init", "-q", "--bare", "-b", branch, cwd=remote)
    seed = tempfile.mkdtemp()
    sh("clone", "-q", remote, seed)
    for name, content in seed_files.items():
        path = os.path.join(seed, name)
        os.makedirs(os.path.dirname(path) or seed, exist_ok=True)
        open(path, "w").write(content)
    sh("add", "-A", cwd=seed)
    sh("commit", "-q", "-m", "seed", cwd=seed)
    sh("push", "-q", "origin", branch, cwd=seed)
    return remote


class Lab:
    """One test's worth of fake GitHub: a jobs remote and a results remote, plus a scratch clone of the jobs remote for pushing
    new job files (standing in for "the agent reads what was pushed"), and a fresh config+agent pointed at both."""

    def __init__(self, **cfg_overrides):
        self.jobs_remote = bare_remote("main", {"jobs/.keep": ""})
        self.results_remote = bare_remote("results", {"README.md": "results\n"})
        self.pusher = tempfile.mkdtemp()
        sh("clone", "-q", self.jobs_remote, self.pusher)
        home = tempfile.mkdtemp()
        self.cfg = Config(jobs_remote=self.jobs_remote, results_remote=self.results_remote,
                          home=home, poll_seconds=0.01, push_seconds=0.01, idle_heartbeat_seconds=0.01,
                          grace_seconds=1, default_timeout=30, max_timeout=600, git_timeout=15, **cfg_overrides)
        self.agent = Agent(self.cfg)
        self.agent.setup()

    def push_job(self, name, body, mode=None, timeout=None, depends_on=None):
        header = []
        if mode:
            header.append(f"mode={mode}")
        if timeout:
            header.append(f"timeout={timeout}")
        if depends_on:
            header.append(f"depends_on={depends_on}")
        text = "#!/bin/sh\n" + (f"# lab: {' '.join(header)}\n" if header else "") + body
        path = os.path.join(self.pusher, "jobs", f"{name}.sh")
        open(path, "w").write(text)
        sh("add", "-A", cwd=self.pusher)
        sh("commit", "-q", "-m", name, cwd=self.pusher)
        sh("push", "-q", "origin", "main", cwd=self.pusher)

    def run_until(self, predicate, timeout=10, step=0.1):
        deadline = time.time() + timeout
        while time.time() < deadline:
            self.agent._pass()
            if predicate(self.agent):
                return
            time.sleep(step)
        raise AssertionError(f"condition not met within {timeout}s")

    def results_status(self, job_name=None):
        check = tempfile.mkdtemp()
        sh("clone", "-q", "--branch", "results", self.results_remote, check)
        path = os.path.join(check, "jobs", job_name, "status.json") if job_name else os.path.join(check, "status.json")
        if not os.path.exists(path):
            return None
        return json.load(open(path))

    def results_log_text(self, job_name):
        check = tempfile.mkdtemp()
        sh("clone", "-q", "--branch", "results", self.results_remote, check)
        log_dir = os.path.join(check, "jobs", job_name, "log")
        if not os.path.isdir(log_dir):
            return ""
        return "".join(open(os.path.join(log_dir, f)).read() for f in sorted(os.listdir(log_dir)))


def test_a_single_job_runs_and_its_log_and_status_are_pushed():
    lab = Lab()
    lab.push_job("0001-hello", "echo hello-from-the-job\nexit 0\n")
    lab.run_until(lambda a: a.state.is_done("0001-hello"))

    status = lab.results_status("0001-hello")
    assert status["ok"] is True and status["exit_code"] == 0, status
    assert "hello-from-the-job" in lab.results_log_text("0001-hello")
    top = lab.results_status()
    assert top["done_count"] == 1


def test_queue_mode_waits_for_the_current_job_before_starting_the_next():
    lab = Lab()
    lab.push_job("0001-first", "sleep 1\necho first-done\n")
    lab.run_until(lambda a: a.current_job is not None and a.current_job.name == "0001-first")
    lab.push_job("0002-second", "echo second-ran\n")  # default mode=queue
    lab.agent._pass()
    assert lab.agent.current_job.name == "0001-first", "queue mode must not interrupt the job already running"
    lab.run_until(lambda a: a.state.is_done("0002-second"), timeout=15)
    status1, status2 = lab.results_status("0001-first"), lab.results_status("0002-second")
    assert status1["ok"] and status2["ok"]
    assert status1["finished_at"] < status2["started_at"], "the second job must only start after the first finished"


def test_preempt_mode_stops_the_current_job_and_runs_the_newer_one():
    lab = Lab()
    lab.push_job("0001-long", "trap 'echo got-term' TERM\nsleep 30\n")
    lab.run_until(lambda a: a.current_job is not None and a.current_job.name == "0001-long")
    time.sleep(0.3)  # let the trap register before we try to stop it
    lab.push_job("0002-urgent", "echo urgent-ran\n", mode="preempt")
    lab.run_until(lambda a: a.state.is_done("0002-urgent"), timeout=10)

    status1 = lab.results_status("0001-long")
    assert status1["ok"] is False and "preempt" in status1.get("note", ""), status1
    assert "got-term" in lab.results_log_text("0001-long"), "a preempted job should still get a clean SIGTERM, not just be killed"
    status2 = lab.results_status("0002-urgent")
    assert status2["ok"] is True


def test_a_job_that_exceeds_its_timeout_is_stopped_and_marked_failed():
    lab = Lab()
    lab.push_job("0001-stuck", "sleep 30\n", timeout=1)
    lab.run_until(lambda a: a.state.is_done("0001-stuck"), timeout=10)
    status = lab.results_status("0001-stuck")
    assert status["ok"] is False and "timed out" in status.get("note", ""), status


def test_a_job_is_skipped_without_running_when_its_dependency_failed():
    lab = Lab()
    lab.push_job("0001-fails", "exit 1\n")
    lab.push_job("0002-depends", "echo should-not-run\n", depends_on="0001")
    lab.run_until(lambda a: a.state.is_done("0002-depends"), timeout=10)
    status = lab.results_status("0002-depends")
    assert status["ok"] is False and "skipped" in status.get("note", "") and "0001" in status["note"], status
    assert "should-not-run" not in lab.results_log_text("0002-depends")


def test_resuming_after_a_restart_marks_the_interrupted_job_and_moves_on():
    lab = Lab()
    lab.push_job("0001-interrupted", "sleep 20\n")
    lab.run_until(lambda a: a.current_job is not None and a.current_job.name == "0001-interrupted")
    lab.push_job("0002-after", "echo after-restart\n")

    # simulate the agent process dying without a clean stop: build a brand-new Agent on the SAME on-disk state and home.
    # the orphaned sleep from the first agent is still alive underneath, same as it would be after a real crash.
    second_agent = Agent(lab.cfg)
    second_agent.setup()
    assert second_agent.state.is_done("0001-interrupted")
    status = lab.results_status("0001-interrupted")
    assert status is None or status.get("ok") is not True  # setup() writes its own note directly; either is acceptable here

    lab.agent = second_agent  # so run_until drives the new one
    lab.run_until(lambda a: a.state.is_done("0002-after"), timeout=10)
    assert lab.results_status("0002-after")["ok"] is True
    subprocess.run(["pkill", "-f", "0001-interrupted"], capture_output=True)  # clean up the orphan


def test_setup_refuses_to_start_against_a_real_github_remote_with_no_token():
    import contextlib
    import io

    cfg = Config(jobs_remote="https://github.com/example/jobs.git", results_remote="https://github.com/example/results.git",
                home=tempfile.mkdtemp())
    agent = Agent(cfg)
    os.environ.pop("LAB_AGENT_GITHUB_TOKEN", None)
    try:
        with contextlib.redirect_stderr(io.StringIO()):
            agent.setup()
    except SystemExit as error:
        assert "LAB_AGENT_GITHUB_TOKEN" in str(error)
        return
    raise AssertionError("should have refused to start")


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
