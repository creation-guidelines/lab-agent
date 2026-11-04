# SPDX-License-Identifier: GPL-3.0-or-later
"""Job scripts in the jobs repository: finding them, ordering them, reading their header.

A job is a shell script at ``<jobs_dir>/NNNN-what-it-does.sh`` (four digits, then a hyphen, then a short name). Jobs run in filename order, each
exactly once. A job is never edited after it is written: a later job is a new, higher-numbered file. The agent's own record of which jobs it has
already run (see state.py) is what makes a restart not re-run a finished job.

The header is plain ``#`` comment lines at the top of the file, each ``key = value``::

    #!/bin/sh
    # lab: mode=preempt timeout=900 depends_on=0042

mode
    ``queue`` (the default): wait for the current job to finish.
    ``preempt``: stop the current job first (see runner.py for how).
depends_on
    a job name or number that must have finished successfully before this one is eligible to run; otherwise it is skipped, not failed, and the
    agent explains why in its log, so the gap is visible rather than silent.
timeout
    seconds before the job is stopped; defaults to ``config.default_timeout`` and is clamped to ``config.max_timeout``.
"""
import os
import re
import shlex

NAME = re.compile(r"^(\d{4})-([a-z0-9][a-z0-9_-]*)\.sh$")
HEADER_LINE = re.compile(r"^#\s*lab:\s*(.*)$")
KNOWN_KEYS = {"mode", "timeout", "depends_on"}
MODES = {"queue", "preempt"}


class JobError(Exception):
    """A job file's name or header cannot be understood; it is skipped, not run."""


class Job:
    """One job file: its identity, header, and path. Two jobs are equal, and hash equal, by name alone."""

    def __init__(self, number, slug, path, mode, timeout, depends_on):
        self.number, self.slug, self.path = number, slug, path
        self.mode, self.timeout, self.depends_on = mode, timeout, depends_on

    @property
    def name(self):
        return f"{self.number:04d}-{self.slug}"

    def __eq__(self, other):
        return isinstance(other, Job) and self.name == other.name

    def __hash__(self):
        return hash(self.name)

    def __repr__(self):
        return f"Job({self.name!r}, mode={self.mode!r})"


def parse_header(text, default_timeout, max_timeout):
    """The ``# lab: ...`` line's settings, as ``(mode, timeout, depends_on)``, defaulted and validated."""
    settings = {}
    for line in text.split("\n"):
        match = HEADER_LINE.match(line.strip())
        if not match:
            if line.strip() and not line.strip().startswith("#"):
                break  # the header is only at the top of the file, before any real content
            continue
        try:
            pairs = shlex.split(match.group(1))
        except ValueError as error:
            raise JobError(f"the lab: header is not valid ({error})") from error
        for pair in pairs:
            if "=" not in pair:
                raise JobError(f"the lab: header has {pair!r}, which is not key=value")
            key, _, value = pair.partition("=")
            if key not in KNOWN_KEYS:
                raise JobError(f"the lab: header names {key!r}; known: {', '.join(sorted(KNOWN_KEYS))}")
            settings[key] = value
    mode = settings.get("mode", "queue")
    if mode not in MODES:
        raise JobError(f"mode={mode!r} is not one of {', '.join(sorted(MODES))}")
    timeout = settings.get("timeout")
    if timeout is not None:
        if not timeout.isdigit() or int(timeout) <= 0:
            raise JobError(f"timeout={timeout!r} is not a positive whole number of seconds")
        timeout = min(int(timeout), max_timeout)
    else:
        timeout = default_timeout
    return mode, timeout, settings.get("depends_on")


def discover(jobs_dir, default_timeout, max_timeout):
    """Every job in ``jobs_dir``, in filename order. A file that does not match ``NAME`` is ignored (it is not a job). A job whose header cannot
    be parsed is reported through ``errors`` (``{filename: message}``) and left out, so one bad file cannot stop every other job from running."""
    found, errors = [], {}
    if not os.path.isdir(jobs_dir):
        return found, errors
    for filename in sorted(os.listdir(jobs_dir)):
        match = NAME.match(filename)
        if not match:
            continue
        path = os.path.join(jobs_dir, filename)
        try:
            with open(path, encoding="utf-8") as f:
                mode, timeout, depends_on = parse_header(f.read(4096), default_timeout, max_timeout)
            found.append(Job(int(match.group(1)), match.group(2), path, mode, timeout, depends_on))
        except (JobError, OSError, UnicodeDecodeError) as error:
            errors[filename] = str(error)
    return found, errors


def next_job(jobs, done):
    """The earliest job not in ``done`` (a set of job names already finished, successfully or not). ``None`` when there is none."""
    for job in jobs:
        if job.name not in done:
            return job
    return None


def ready(job, done, ok):
    """Whether ``job``'s dependency, if any, has finished (in ``done``) and succeeded (in ``ok``, a set of job names that exited zero).
    A dependency is matched by its number or its full name, so ``depends_on=42`` and ``depends_on=0042-boot-image`` both work."""
    if not job.depends_on:
        return True, None
    for candidate in done:
        number = candidate.split("-", 1)[0]
        if candidate == job.depends_on or number == job.depends_on or number.lstrip("0") == job.depends_on.lstrip("0"):
            return candidate in ok, candidate
    return False, None
