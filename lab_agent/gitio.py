# SPDX-License-Identifier: GPL-3.0-or-later
"""The agent's only two ways of touching git: read a repository (clone it once, pull it after), and push a results tree as one rolling commit.

Authentication is whatever the ``remote`` URL itself carries - an ``https://x-access-token:TOKEN@github.com/...`` URL, built by the caller
(see agent.py), same as every other git operation in this project. These functions know nothing about how a remote is authenticated; they
only run git commands against whatever URL they are given, with a timeout (``config.git_timeout``) and a clean environment (no interactive
prompts: a stuck credential prompt must fail fast, not hang the agent), and raise ``GitError`` with the command's stderr on failure.
"""
import email.utils
import os
import subprocess


class GitError(Exception):
    """A git command failed or timed out; the message is what git said."""


def with_token(remote, token):
    """``remote`` with a GitHub token inserted as its credentials (``https://x-access-token:TOKEN@host/...``), if it is an
    ``https://`` URL and does not already carry one. Anything else - a local filesystem path (as the tests use), an
    already-authenticated URL, or no token at all - is returned unchanged, so the same remote string works in tests with no
    token and in production with one."""
    if token and remote.startswith("https://") and "@" not in remote.split("://", 1)[1].split("/", 1)[0]:
        return remote.replace("https://", f"https://x-access-token:{token}@", 1)
    return remote


def _run(args, cwd, timeout, extra_env=None):
    full_env = dict(os.environ, GIT_TERMINAL_PROMPT="0", GIT_ASKPASS="true", **(extra_env or {}))
    try:
        result = subprocess.run(["git", *args], cwd=cwd, env=full_env, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired as error:
        raise GitError(f"git {' '.join(args)} did not finish within {timeout}s") from error
    if result.returncode != 0:
        raise GitError(f"git {' '.join(args)} failed: {result.stderr.strip()[-2000:]}")
    return result.stdout


def ensure_clone(remote, branch, path, timeout):
    """Clone ``remote`` at ``branch`` into ``path`` if it is not there yet; otherwise leave it, so a long-lived checkout is reused across polls
    (``sync_jobs``/``sync_results`` below update it). Returns nothing; raises ``GitError`` on failure."""
    if os.path.isdir(os.path.join(path, ".git")):
        return
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    _run(["clone", "--branch", branch, "--single-branch", remote, path], cwd=None, timeout=timeout)


def sync_jobs(path, branch, timeout):
    """Fast-forward ``path`` to the remote's ``branch``, discarding any local changes first (the jobs clone is read-only from the agent's own
    side; anything local there would be a bug, not intent). Returns the new HEAD commit hash."""
    _run(["fetch", "--prune", "origin", branch], cwd=path, timeout=timeout)
    _run(["reset", "--hard", f"origin/{branch}"], cwd=path, timeout=timeout)
    _run(["clean", "-fdx"], cwd=path, timeout=timeout)
    return _run(["rev-parse", "HEAD"], cwd=path, timeout=timeout).strip()


def push_results(path, branch, message, timeout, author=("lab-agent", "lab-agent@localhost")):
    """Stage everything under ``path`` and *amend* it onto the single rolling commit on ``branch`` (never a new commit: a job
    pushing its log every few seconds must not leave hours of commit history behind), then force-push. Returns ``True`` if
    there was anything to commit, ``False`` if the working tree matched what was already committed (nothing is pushed in
    that case, so an idle agent does not spam the remote)."""
    # A real timestamp, not the word "now" (git's parser rejects that literal) - and --reset-author below, because
    # --amend otherwise keeps the ORIGINAL commit's author identity and date untouched even when GIT_AUTHOR_DATE is
    # set; only --reset-author actually applies it. Without both, a push made just now could show an author
    # timestamp hours or days old to anyone reading raw git log (the committer date alone did update, which is less
    # obviously wrong but still confusing next to a stale author date).
    now = email.utils.formatdate(localtime=True)
    env = {"GIT_AUTHOR_NAME": author[0], "GIT_AUTHOR_EMAIL": author[1], "GIT_AUTHOR_DATE": now,
          "GIT_COMMITTER_NAME": author[0], "GIT_COMMITTER_EMAIL": author[1], "GIT_COMMITTER_DATE": now}
    _run(["add", "-A"], cwd=path, timeout=timeout)
    status = _run(["status", "--porcelain"], cwd=path, timeout=timeout)
    if not status.strip():
        return False
    # amend, not a new commit: the whole point of a "rolling" results push is that the branch stays ONE commit, however many
    # times this runs, so a job pushing its log every 15 seconds for hours does not leave hours of history behind.
    _run(["commit", "--amend", "--reset-author", "-q", "-m", message], cwd=path, timeout=timeout, extra_env=env)
    _run(["push", "--force-with-lease", "origin", f"HEAD:{branch}"], cwd=path, timeout=timeout)
    return True


def reset_results_to_remote(path, branch, timeout):
    """Discard local commits and match the remote exactly: used once at start-up, in case a previous push half-completed (committed but not
    pushed) and the local history has since diverged from what another process or a manual edit put on the remote."""
    _run(["fetch", "origin", branch], cwd=path, timeout=timeout)
    _run(["reset", "--hard", f"origin/{branch}"], cwd=path, timeout=timeout)
