# SPDX-License-Identifier: GPL-3.0-or-later
"""Running one job's script as a real, killable process group, with its output going straight to a log file.

A job runs as ``/bin/sh <path to the job file>`` (never by relying on the file's own executable bit, which a fresh git checkout
may not preserve), in its own process group (``start_new_session=True``), so that stopping it - on a timeout or a preemption -
can reach every child process it spawned (fastboot, adb, a background `dmesg -w`, ...), not just the shell itself.
"""
import contextlib
import os
import signal
import subprocess
import time


class Runner:
    """One running (or finished) job. ``start`` is called once; ``poll``, ``elapsed`` and ``stop`` may be called repeatedly."""

    def __init__(self, job, log_path, workdir, env):
        self.job = job
        self.log_path = log_path
        self._start_time = None
        self._exit_code = None
        self._log_file = None
        os.makedirs(workdir, exist_ok=True)
        os.makedirs(os.path.dirname(log_path) or ".", exist_ok=True)
        self._log_file = open(log_path, "ab", buffering=0)  # kept open for the job's lifetime; closed explicitly in _close_log
        self._log_file.write(f"--- lab-agent: starting {job.name} (mode={job.mode}, timeout={job.timeout}s) ---\n".encode())
        self._process = subprocess.Popen(
            ["/bin/sh", job.path], cwd=workdir, env=env,
            stdout=self._log_file, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
            start_new_session=True,
        )
        self._start_time = time.time()

    def elapsed(self):
        return time.time() - self._start_time

    def poll(self):
        """``None`` while running; the exit code once the process has ended. Reaps the process exactly once."""
        if self._exit_code is not None:
            return self._exit_code
        code = self._process.poll()
        if code is not None:
            self._exit_code = code
            self._close_log()
        return code

    def stop(self, grace_seconds, reason="stopped"):
        """Ask the process group to end (SIGTERM), wait up to ``grace_seconds``, then force it (SIGKILL) if it has not. Safe to
        call on a process that has already finished. Returns the eventual exit code (negative when it had to be killed, by
        the usual POSIX convention: ``-signal.SIGKILL``)."""
        if self._exit_code is not None:
            return self._exit_code
        if self._log_file and not self._log_file.closed:
            self._log_file.write(f"--- lab-agent: {reason}, sending SIGTERM ---\n".encode())
        with contextlib.suppress(ProcessLookupError):
            os.killpg(self._process.pid, signal.SIGTERM)
        deadline = time.time() + grace_seconds
        while time.time() < deadline:
            if self._process.poll() is not None:
                break
            time.sleep(0.2)
        else:
            if self._log_file and not self._log_file.closed:
                self._log_file.write(b"--- lab-agent: still running after the grace period, sending SIGKILL ---\n")
            with contextlib.suppress(ProcessLookupError):
                os.killpg(self._process.pid, signal.SIGKILL)
            self._process.wait(timeout=10)
        self._exit_code = self._process.returncode
        self._close_log()
        return self._exit_code

    def _close_log(self):
        if self._log_file and not self._log_file.closed:
            self._log_file.write(f"--- lab-agent: exited with code {self._exit_code} ---\n".encode())
            self._log_file.close()
