#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Tests for lab_agent/gitio.py, against real local git repositories standing in for GitHub (a bare repo as the "remote", a working
clone as what the agent operates on). Plain asserts: `python3 tests/test_gitio.py`."""
import os
import subprocess
import sys
import tempfile
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
from lab_agent import gitio  # noqa: E402

ENV = dict(os.environ, GIT_AUTHOR_NAME="test", GIT_AUTHOR_EMAIL="t@example.com",
          GIT_COMMITTER_NAME="test", GIT_COMMITTER_EMAIL="t@example.com")


def sh(*args, cwd=None):
    result = subprocess.run(["git", *args], cwd=cwd, env=ENV, capture_output=True, text=True, timeout=10)
    assert result.returncode == 0, (args, result.stderr)
    return result.stdout


def bare_remote(initial_files=None, branch="main"):
    """A bare repo (the "remote") seeded with one commit, plus a scratch clone used only to create that commit."""
    remote = tempfile.mkdtemp()
    sh("init", "-q", "--bare", "-b", branch, cwd=remote)
    seed = tempfile.mkdtemp()
    sh("clone", "-q", remote, seed)
    for name, content in (initial_files or {"README": "hello\n"}).items():
        path = os.path.join(seed, name)
        os.makedirs(os.path.dirname(path) or seed, exist_ok=True)
        open(path, "w").write(content)
    sh("add", "-A", cwd=seed)
    sh("commit", "-q", "-m", "seed", cwd=seed)
    sh("push", "-q", "origin", branch, cwd=seed)
    return remote


def test_ensure_clone_creates_a_working_copy_and_is_a_no_op_the_second_time():
    remote = bare_remote()
    local = os.path.join(tempfile.mkdtemp(), "clone")
    gitio.ensure_clone(remote, "main", local, timeout=15)
    assert os.path.exists(os.path.join(local, "README"))
    # a second call must not fail or reset anything (it would be a bug for the agent's own in-progress work to vanish)
    open(os.path.join(local, "untracked.txt"), "w").write("still here?")
    gitio.ensure_clone(remote, "main", local, timeout=15)
    assert os.path.exists(os.path.join(local, "untracked.txt"))


def test_sync_jobs_fast_forwards_to_the_remote_and_discards_local_changes():
    remote = bare_remote({"jobs/0001-a.sh": "#!/bin/sh\n"})
    local = os.path.join(tempfile.mkdtemp(), "clone")
    gitio.ensure_clone(remote, "main", local, timeout=15)
    # a local change the agent never made (e.g. a stray file left by a crashed job) must not survive a sync
    open(os.path.join(local, "jobs", "stray.txt"), "w").write("should be wiped")
    sh("add", "-A", cwd=local)  # left staged but uncommitted, the worst case for `reset --hard` + `clean`

    sha = gitio.sync_jobs(local, "main", timeout=15)
    assert not os.path.exists(os.path.join(local, "jobs", "stray.txt"))
    assert sha == sh("rev-parse", "HEAD", cwd=local).strip()

    # now push a second job from a second clone and sync again: the first clone must pick it up
    seed = tempfile.mkdtemp()
    sh("clone", "-q", remote, seed)
    open(os.path.join(seed, "jobs", "0002-b.sh"), "w").write("#!/bin/sh\n")
    sh("add", "-A", cwd=seed)
    sh("commit", "-q", "-m", "second job", cwd=seed)
    sh("push", "-q", "origin", "main", cwd=seed)
    gitio.sync_jobs(local, "main", timeout=15)
    assert os.path.exists(os.path.join(local, "jobs", "0002-b.sh"))


def test_push_results_commits_and_pushes_and_is_idempotent_when_nothing_changed():
    remote = bare_remote({"status.json": "{}\n"}, branch="results")
    local = os.path.join(tempfile.mkdtemp(), "clone")
    gitio.ensure_clone(remote, "results", local, timeout=15)
    open(os.path.join(local, "status.json"), "w").write('{"running": null}\n')

    pushed = gitio.push_results(local, "results", "progress", timeout=15)
    assert pushed is True

    # a fresh clone of the remote must see the change
    check = tempfile.mkdtemp()
    sh("clone", "-q", "--branch", "results", remote, check)
    assert open(os.path.join(check, "status.json")).read() == '{"running": null}\n'

    # pushing again with no change in the tree must be a no-op (an idle heartbeat must not spam commits)
    pushed_again = gitio.push_results(local, "results", "heartbeat", timeout=15)
    assert pushed_again is False


def test_push_results_amends_the_same_commit_instead_of_piling_up_history():
    # even the remote's own seed commit gets absorbed on the first real push: the branch is always exactly one commit, so
    # pushing every few seconds for hours leaves no history behind at all
    remote = bare_remote({"status.json": "{}\n"}, branch="results")
    local = os.path.join(tempfile.mkdtemp(), "clone")
    gitio.ensure_clone(remote, "results", local, timeout=15)
    for i in range(3):
        open(os.path.join(local, "status.json"), "w").write(f'{{"n": {i}}}\n')
        gitio.push_results(local, "results", f"progress {i}", timeout=15)
    log = sh("log", "--oneline", cwd=local).strip().split("\n")
    assert len(log) == 1, f"the branch should always be exactly one commit: {log}"
    assert log[0].endswith("progress 2")

    check = tempfile.mkdtemp()
    sh("clone", "-q", "--branch", "results", remote, check)
    assert open(os.path.join(check, "status.json")).read() == '{"n": 2}\n'
    assert sh("log", "--oneline", cwd=check).strip().count("\n") == 0, "the remote itself must also be a single commit"


def test_reset_results_to_remote_discards_an_unpushed_local_commit():
    remote = bare_remote({"status.json": "{}\n"}, branch="results")
    local = os.path.join(tempfile.mkdtemp(), "clone")
    gitio.ensure_clone(remote, "results", local, timeout=15)
    open(os.path.join(local, "stray.json"), "w").write("{}")
    sh("add", "-A", cwd=local)
    sh("commit", "-q", "-m", "a commit that was never pushed", cwd=local)

    gitio.reset_results_to_remote(local, "results", timeout=15)
    assert not os.path.exists(os.path.join(local, "stray.json"))
    assert sh("log", "--oneline", cwd=local).strip().count("\n") == 0


def test_a_failed_git_command_raises_gitError_with_its_message():
    try:
        gitio.ensure_clone("/no/such/path/at/all", "main", os.path.join(tempfile.mkdtemp(), "x"), timeout=5)
    except gitio.GitError as error:
        assert "git clone" in str(error)
        return
    raise AssertionError("should have raised GitError")


def test_with_token_inserts_credentials_into_an_https_url():
    assert gitio.with_token("https://github.com/o/r.git", "abc123") == "https://x-access-token:abc123@github.com/o/r.git"


def test_with_token_leaves_a_url_unchanged_when_there_is_no_token_or_it_is_not_https():
    assert gitio.with_token("https://github.com/o/r.git", "") == "https://github.com/o/r.git"
    assert gitio.with_token("/local/path", "abc123") == "/local/path"


def test_with_token_does_not_double_insert_into_an_already_authenticated_url():
    already = "https://x-access-token:old@github.com/o/r.git"
    assert gitio.with_token(already, "new") == already


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
