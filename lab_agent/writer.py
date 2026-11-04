# SPDX-License-Identifier: GPL-3.0-or-later
"""What the agent writes into the results tree, so the other side (a person, or an agent reading through git) can see what happened.

Layout, under the results clone's working directory:

    status.json                    one object: what the agent is doing right now (see write_top_status)
    jobs/<name>/status.json        one object: everything known about that job (see write_job_status)
    jobs/<name>/log/0000.txt ...   the job's console output, split into chunks of ``chunk_bytes``

Only the *tail* of a very long log is pushed (``max_pushed_bytes``): the agent's own machine keeps the whole thing
(``config.local_logs/<name>.log``), so nothing is lost, but a runaway job cannot make every push larger than the last.
Old job directories beyond ``keep_results`` are deleted from the results tree (not from the machine) by ``prune``, oldest first,
so the pushed repository does not grow without bound.
"""
import json
import os
import shutil
import time


def _atomic_write_json(path, data):
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    tmp = f"{path}.tmp.{os.getpid()}"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=2, sort_keys=True)
        f.write("\n")
    os.replace(tmp, path)


def write_top_status(results_dir, *, agent_version, running, last_heartbeat, done_count, host):
    """The single file that answers "what is the agent doing right now": the job name it is running (or ``None``), when it last
    showed signs of life, how many jobs are finished overall, and which machine this is (so results from more than one lab are
    never confused if the repo is ever shared)."""
    _atomic_write_json(os.path.join(results_dir, "status.json"), {
        "agent_version": agent_version, "running": running, "last_heartbeat": last_heartbeat,
        "done_count": done_count, "host": host, "updated_at": time.time(),
    })


def write_job_note(results_dir, name, **fields):
    """Merge ``fields`` into ``jobs/<name>/status.json`` by job name alone (no ``Job`` object needed: used when the agent
    knows only a name, such as a job from a previous run whose script may no longer even exist in the jobs repository)."""
    path = os.path.join(results_dir, "jobs", name, "status.json")
    current = {}
    if os.path.exists(path):
        try:
            with open(path, encoding="utf-8") as f:
                current = json.load(f)
        except (json.JSONDecodeError, OSError):
            current = {}
    current.update(name=name, **fields)
    current["updated_at"] = time.time()
    _atomic_write_json(path, current)


def write_job_status(results_dir, job, **fields):
    """Merge ``fields`` into ``jobs/<job.name>/status.json`` (creating it on first write). Always includes the job's own
    settings (mode, timeout, depends_on) so the file is self-describing without needing the job script alongside it."""
    write_job_note(results_dir, job.name, mode=job.mode, timeout=job.timeout, depends_on=job.depends_on, **fields)


def sync_log(results_dir, job_name, full_log_path, chunk_bytes, max_pushed_bytes):
    """Copy the tail of ``full_log_path`` (at most ``max_pushed_bytes``) into ``jobs/<job_name>/log/`` as fixed-size chunk files,
    removing any chunk files left over from a previous, longer version of the log. If the tail is shorter than the whole file,
    the first chunk is prefixed with a note saying how much was left off, so a reader is never silently missing the start."""
    log_dir = os.path.join(results_dir, "jobs", job_name, "log")
    os.makedirs(log_dir, exist_ok=True)
    try:
        size = os.path.getsize(full_log_path)
    except OSError:
        size = 0
    omitted = max(0, size - max_pushed_bytes)
    with open(full_log_path, "rb") as f:
        if omitted:
            f.seek(omitted)
        data = f.read(max_pushed_bytes)
    if omitted:
        data = f"... ({omitted} earlier bytes omitted; the full log is kept on the lab machine)\n".encode() + data
    chunks = [data[i:i + chunk_bytes] for i in range(0, len(data), chunk_bytes)] or [b""]
    for index, chunk in enumerate(chunks):
        with open(os.path.join(log_dir, f"{index:04d}.txt"), "wb") as f:
            f.write(chunk)
    for leftover in sorted(os.listdir(log_dir)):
        if leftover.endswith(".txt") and int(leftover[:4]) >= len(chunks):
            os.remove(os.path.join(log_dir, leftover))


def prune(results_dir, keep_results, finished_order):
    """Delete the results-tree directories of the oldest finished jobs beyond ``keep_results``, by the order they finished in
    (``finished_order``, oldest first). The agent's own machine keeps every job's full log and state regardless; this only
    trims what gets pushed and kept in the results repository's working tree."""
    to_remove = finished_order[:max(0, len(finished_order) - keep_results)]
    for name in to_remove:
        path = os.path.join(results_dir, "jobs", name)
        if os.path.isdir(path):
            shutil.rmtree(path, ignore_errors=True)
