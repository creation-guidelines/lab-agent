# SPDX-License-Identifier: GPL-3.0-or-later
"""The main loop: pull jobs, run them one at a time in order, push progress, repeat. This is the whole point of the package; every other
module exists to be used from here.

    python3 -m lab_agent.agent [--config PATH]

Runs until killed (SIGTERM/SIGINT end it cleanly: the current job is left running and will be picked up on the next pull, see State). A
supervisor (systemd, see deploy/) should restart it on any other exit. One pass of the loop, roughly:

1. Pull the jobs repository.
2. If a job is running: check whether it finished, or whether its timeout has elapsed, or whether a *newer* job wants to preempt it; in
   any of those cases, end it and record the result. Otherwise, let it keep running.
3. If nothing is running: pick the next job the agent has not already finished, skipping (not failing) any whose dependency failed, and
   start it.
4. Push: the running job's log tail (at least every ``push_seconds``), and a heartbeat even when idle (every
   ``idle_heartbeat_seconds``) so a silent agent can be told apart from a silent lab.
5. Sleep ``poll_seconds`` and repeat.

A push failure (the network is down, GitHub is unreachable) is logged and retried next pass; it never stops a running job or drops its
output, which keeps accumulating in the local log either way.
"""
import logging
import os
import signal
import socket
import sys
import time

from . import __version__, gitio, jobs, state, writer
from . import config as config_module
from .runner import Runner

log = logging.getLogger("lab_agent")


class Agent:
    def __init__(self, cfg):
        self.cfg = cfg
        self.state = state.State(cfg.state_path)
        self.runner = None
        self.current_job = None
        self._last_push = 0.0
        self._last_heartbeat = 0.0
        self._stopping = False
        # read once, used to build the authenticated remote URLs: never written to disk, never passed to a job as an
        # environment variable (job scripts get the rest of os.environ, see _start, but not this one explicitly removed there)
        self._token = os.environ.get("LAB_AGENT_GITHUB_TOKEN", "")
        self._jobs_url = gitio.with_token(cfg.jobs_remote, self._token)
        self._results_url = gitio.with_token(cfg.results_remote, self._token)

    def setup(self):
        if not self._token and (self.cfg.jobs_remote.startswith("https://github.com") or self.cfg.results_remote.startswith("https://github.com")):
            raise SystemExit("lab-agent: set LAB_AGENT_GITHUB_TOKEN (a token with access to the jobs and results repositories)")
        os.makedirs(self.cfg.home, exist_ok=True)
        os.makedirs(self.cfg.local_logs, exist_ok=True)
        os.makedirs(self.cfg.workdir, exist_ok=True)
        gitio.ensure_clone(self._jobs_url, self.cfg.jobs_branch, self.cfg.jobs_clone, self.cfg.git_timeout)
        gitio.ensure_clone(self._results_url, self.cfg.results_branch, self.cfg.results_clone, self.cfg.git_timeout)
        gitio.reset_results_to_remote(self.cfg.results_clone, self.cfg.results_branch, self.cfg.git_timeout)
        resuming = self.state.resuming()
        if resuming:
            log.warning("a job (%s) was running when the agent last stopped; recording it as interrupted", resuming["name"])
            self.state.job_finished(resuming["name"], ok=False, exit_code=None)
            note = ("the agent restarted while this job was running; its process, if still alive, "
                   "was not supervised and may have kept running or been reaped by the system")
            self._write_job_outcome_note(resuming["name"], note)

    def _write_job_outcome_note(self, name, note):
        writer.write_job_note(self.cfg.results_clone, name, ok=False, finished_at=time.time(), exit_code=None,
                              running=False, note=note)

    def request_stop(self):
        self._stopping = True

    def run_forever(self):
        self.setup()
        log.info("lab-agent started; jobs=%s results=%s", self.cfg.jobs_remote, self.cfg.results_remote)
        while not self._stopping:
            try:
                self._pass()
            except gitio.GitError as error:
                log.warning("git problem this pass, will retry next pass: %s", error)
            except Exception:
                log.exception("unexpected error this pass; continuing (the running job, if any, is untouched)")
            time.sleep(self.cfg.poll_seconds)
        if self.runner:
            log.info("stopping: leaving %s running for the next agent start to find", self.current_job.name)

    def _pass(self):
        gitio.sync_jobs(self.cfg.jobs_clone, self.cfg.jobs_branch, self.cfg.git_timeout)
        jobs_dir = os.path.join(self.cfg.jobs_clone, self.cfg.jobs_dir)
        found, errors = jobs.discover(jobs_dir, self.cfg.default_timeout, self.cfg.max_timeout)
        for filename, message in errors.items():
            log.warning("skipping unreadable job %s: %s", filename, message)

        if self.runner:
            self._check_running(found)
        if not self.runner:
            self._maybe_start_next(found)

        now = time.time()
        due_push = self.runner and now - self._last_push >= self.cfg.push_seconds
        due_heartbeat = not self.runner and now - self._last_heartbeat >= self.cfg.idle_heartbeat_seconds
        if due_push or due_heartbeat or self._stopping:
            self._push(now)

    def _check_running(self, found):
        job, runner = self.current_job, self.runner
        code = runner.poll()
        if code is not None:
            self._finish(job, ok=(code == 0), exit_code=code)
            return
        preemptor = next((j for j in found if j.mode == "preempt" and j.name != job.name and not self.state.is_done(j.name)
                          and (j.number > job.number or j.name > job.name)), None)
        if preemptor:
            log.info("preempting %s for %s", job.name, preemptor.name)
            code = runner.stop(self.cfg.grace_seconds, reason=f"preempted by {preemptor.name}")
            self._finish(job, ok=False, exit_code=code, note=f"preempted by {preemptor.name}")
            return
        if runner.elapsed() > job.timeout:
            log.warning("%s exceeded its timeout of %ss, stopping it", job.name, job.timeout)
            code = runner.stop(self.cfg.grace_seconds, reason="timed out")
            self._finish(job, ok=False, exit_code=code, note="timed out")

    def _finish(self, job, ok, exit_code, note=None):
        self.state.job_finished(job.name, ok, exit_code)
        fields = {"finished_at": time.time(), "running": False, "ok": ok, "exit_code": exit_code}
        if note:
            fields["note"] = note
        writer.write_job_status(self.cfg.results_clone, job, **fields)
        self._sync_current_log(job)
        self.runner = None
        self.current_job = None
        log.info("%s finished: %s (exit %s)%s", job.name, "ok" if ok else "FAILED", exit_code, f" ({note})" if note else "")

    def _maybe_start_next(self, found):
        done = self.state.done_names()
        ok = self.state.ok_names()
        for job in found:
            if job.name in done:
                continue
            is_ready, dep_name = jobs.ready(job, done, ok)
            if job.depends_on and not is_ready:
                if dep_name:  # the dependency finished, but failed: this job is skipped, not blocked forever
                    self.state.job_finished(job.name, ok=False, exit_code=None)
                    writer.write_job_status(self.cfg.results_clone, job, ok=False, exit_code=None, finished_at=time.time(),
                                            running=False, note=f"skipped: depends_on={job.depends_on}, which did not succeed")
                    log.info("skipping %s: its dependency %s did not succeed", job.name, dep_name)
                    continue
                else:
                    continue  # waiting for the dependency to even finish
            self._start(job)
            return

    def _start(self, job):
        log.info("starting %s (mode=%s, timeout=%ss)", job.name, job.mode, job.timeout)
        log_path = os.path.join(self.cfg.local_logs, f"{job.name}.log")
        workdir = os.path.join(self.cfg.workdir, job.name)
        env = dict(os.environ, JOB_NAME=job.name, JOB_WORKDIR=workdir,
                   JOB_RESULTS_DIR=os.path.join(self.cfg.results_clone, "jobs", job.name), **self.cfg.extra_env)
        os.makedirs(env["JOB_RESULTS_DIR"], exist_ok=True)
        self.runner = Runner(job, log_path, workdir, env)
        self.current_job = job
        self.state.job_started(job.name, self.runner._process.pid)
        writer.write_job_status(self.cfg.results_clone, job, started_at=time.time(), running=True)

    def _sync_current_log(self, job):
        log_path = os.path.join(self.cfg.local_logs, f"{job.name}.log")
        if os.path.exists(log_path):
            writer.sync_log(self.cfg.results_clone, job.name, log_path, self.cfg.log_chunk_bytes, self.cfg.max_pushed_log_bytes)

    def _push(self, now):
        if self.current_job:
            self._sync_current_log(self.current_job)
        order = sorted(self.state.done, key=lambda n: self.state.done[n]["finished_at"])
        writer.prune(self.cfg.results_clone, self.cfg.keep_results, order)
        writer.write_top_status(self.cfg.results_clone, agent_version=__version__,
                                 running=self.current_job.name if self.current_job else None,
                                 last_heartbeat=now, done_count=len(self.state.done), host=socket.gethostname())
        message = f"progress: {self.current_job.name}" if self.current_job else "heartbeat: idle"
        try:
            pushed = gitio.push_results(self.cfg.results_clone, self.cfg.results_branch, message, self.cfg.git_timeout)
            if pushed:
                self._last_push = now
            self._last_heartbeat = now
        except gitio.GitError as error:
            log.warning("could not push results, will retry next pass: %s", error)


def main(argv=None):
    import argparse
    parser = argparse.ArgumentParser(prog="lab-agent")
    parser.add_argument("--config", default=config_module.DEFAULT_PATH)
    args = parser.parse_args(argv)
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
    try:
        cfg = config_module.load(args.config)
    except config_module.ConfigError as error:
        sys.exit(f"lab-agent: {error}")
    agent = Agent(cfg)
    for sig in (signal.SIGTERM, signal.SIGINT):
        signal.signal(sig, lambda *_: agent.request_stop())
    agent.run_forever()


if __name__ == "__main__":
    main()
