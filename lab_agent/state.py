# SPDX-License-Identifier: GPL-3.0-or-later
"""The agent's own memory of what it has done: which jobs are finished, which succeeded, and what is running right now.

Kept as one small JSON file (``config.state_path``), written after every change that matters (a job starts, a job ends, a heartbeat), so a
restart of the agent process - crash, reboot, a new version - reads it back and continues without re-running a finished job or losing the
fact that something was mid-flight. The file is written atomically (a temp file, then rename), so a crash mid-write never leaves it half
written; a half-written old file (from a crash before this was true, or a disk fault) is treated as empty, not fatal, and the agent logs
that it started over on an empty state.
"""
import json
import os
import time


class State:
    """``done``: job names that finished (``{name: {"ok": bool, "finished_at": float, "exit_code": int|None}}``).
    ``running``: the job in progress, or ``None`` (``{"name": str, "started_at": float, "pid": int|None}``)."""

    def __init__(self, path):
        self.path = path
        self.done = {}
        self.running = None
        self._load()

    def _load(self):
        try:
            with open(self.path, encoding="utf-8") as f:
                raw = json.load(f)
        except FileNotFoundError:
            return
        except (json.JSONDecodeError, OSError):
            return  # treated as empty: see the module docstring
        if isinstance(raw, dict):
            self.done = {k: v for k, v in raw.get("done", {}).items() if isinstance(v, dict)}
            running = raw.get("running")
            self.running = running if isinstance(running, dict) and "name" in running else None

    def _save(self):
        os.makedirs(os.path.dirname(self.path) or ".", exist_ok=True)
        tmp = f"{self.path}.tmp.{os.getpid()}"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump({"done": self.done, "running": self.running}, f, indent=2, sort_keys=True)
        os.replace(tmp, self.path)

    def job_started(self, name, pid):
        self.running = {"name": name, "started_at": time.time(), "pid": pid}
        self._save()

    def job_finished(self, name, ok, exit_code):
        self.done[name] = {"ok": ok, "finished_at": time.time(), "exit_code": exit_code}
        self.running = None
        self._save()

    def is_done(self, name):
        return name in self.done

    def done_names(self):
        return set(self.done)

    def ok_names(self):
        return {name for name, result in self.done.items() if result["ok"]}

    def resuming(self):
        """The job that was ``running`` when the agent last stopped, or ``None``. Its outcome is unknown (the process may have finished, failed,
        or still be running under a different pid after a crash of only the agent): the runner decides what to do with this, not this module."""
        return self.running
