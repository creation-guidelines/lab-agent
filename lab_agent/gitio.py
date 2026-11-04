# SPDX-License-Identifier: GPL-3.0-or-later
"""The agent's only two ways of touching git: read a repository (clone it once, pull it after), and push a results tree as one rolling commit.

Every git call runs with a timeout (``config.git_timeout``) and a clean environment (no interactive prompts: a stuck credential prompt must
fail fast, not hang the agent), and failures raise ``GitError`` with the command's stderr rather than letting a non-zero exit pass silently.
"""
import os
import subprocess


class GitError(Exception):
    """A git command failed or timed out; the message is what git said."""


def _run(args, cwd, timeout, env=None):
    full_env = dict(os.environ, GIT_TERMINAL_PROMPT="0", GIT_ASKPASS="true")
    full_env.update(env or {})
    try:
        result = subprocess.run(["git", *args], cwd=cwd, env=full_env, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired as error:
        raise GitError(f"git {' '.join(args)} did not finish within {timeout}s") from error
    if result.returncode != 0:
        raise GitError(f"git {' '.join(args)} failed: {result.stderr.strip()[-2000:]}")
    return result.stdout


def ensure_clone(remote, branch, path, timeout, ssh_key=None):
    """Clone ``remote`` at ``branch`` into ``path`` if it is not there yet; otherwise leave it, so a long-lived checkout is reused across polls
    (``sync_jobs``/``sync_results`` below update it). Returns nothing; raises ``GitError`` on failure."""
    env = {"GIT_SSH_COMMAND": f"ssh -i {ssh_key} -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new"} if ssh_key else None
    if os.path.isdir(os.path.join(path, ".git")):
        return
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    _run(["clone", "--branch", branch, "--single-branch", remote, path], cwd=None, timeout=timeout, env=env)


def sync_jobs(path, branch, timeout, ssh_key=None):
    """Fast-forward ``path`` to the remote's ``branch``, discarding any local changes first (the jobs clone is read-only from the agent's own
    side; anything local there would be a bug, not intent). Returns the new HEAD commit hash."""
    env = {"GIT_SSH_COMMAND": f"ssh -i {ssh_key} -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new"} if ssh_key else None
    _run(["fetch", "--prune", "origin", branch], cwd=path, timeout=timeout, env=env)
    _run(["reset", "--hard", f"origin/{branch}"], cwd=path, timeout=timeout, env=env)
    _run(["clean", "-fdx"], cwd=path, timeout=timeout, env=env)
    return _run(["rev-parse", "HEAD"], cwd=path, timeout=timeout).strip()


def push_results(path, branch, message, timeout, ssh_key=None, author=("lab-agent", "lab-agent@localhost")):
    """Stage everything under ``path`` and *amend* it onto the single rolling commit on ``branch`` (never a new commit: a job
    pushing its log every few seconds must not leave hours of commit history behind), then force-push. Returns ``True`` if
    there was anything to commit, ``False`` if the working tree matched what was already committed (nothing is pushed in
    that case, so an idle agent does not spam the remote)."""
    env = {"GIT_SSH_COMMAND": f"ssh -i {ssh_key} -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new"} if ssh_key else None
    env = dict(env or {}, GIT_AUTHOR_NAME=author[0], GIT_AUTHOR_EMAIL=author[1], GIT_COMMITTER_NAME=author[0], GIT_COMMITTER_EMAIL=author[1])
    _run(["add", "-A"], cwd=path, timeout=timeout, env=env)
    status = _run(["status", "--porcelain"], cwd=path, timeout=timeout, env=env)
    if not status.strip():
        return False
    # amend, not a new commit: the whole point of a "rolling" results push is that the branch stays ONE commit, however many
    # times this runs, so a job pushing its log every 15 seconds for hours does not leave hours of history behind.
    _run(["commit", "--amend", "-q", "-m", message], cwd=path, timeout=timeout, env=env)
    _run(["push", "--force-with-lease", "origin", f"HEAD:{branch}"], cwd=path, timeout=timeout, env=env)
    return True


def reset_results_to_remote(path, branch, timeout, ssh_key=None):
    """Discard local commits and match the remote exactly: used once at start-up, in case a previous push half-completed (committed but not
    pushed) and the local history has since diverged from what another process or a manual edit put on the remote."""
    env = {"GIT_SSH_COMMAND": f"ssh -i {ssh_key} -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new"} if ssh_key else None
    _run(["fetch", "origin", branch], cwd=path, timeout=timeout, env=env)
    _run(["reset", "--hard", f"origin/{branch}"], cwd=path, timeout=timeout, env=env)
