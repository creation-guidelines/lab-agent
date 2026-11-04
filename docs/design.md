# How this fits together

`lab-agent` runs on one dedicated machine, with hardware (today: an Android phone in fastboot, over USB) attached to it. It makes no
inbound connections, needs no port forwarding and no DDNS: everything it does is an outbound poll to GitHub.

## Three repositories, not one

- **This repository** (`lab-agent`): the agent's own code. Public, normal GitHub flow, tested by hosted CI on every pull request.
- **A jobs repository** (private, per lab): numbered shell scripts the agent runs, in order. Whoever is directing the work — a
  person, or an agent such as Claude — pushes a new job here. The lab-agent process only ever *reads* this repository: nothing
  structural stops the same token from writing to it, but the agent's own code never tries to, which `tests/test_agent.py`
  exercises end to end.
- **A results repository** (private, per lab): what the agent writes back — one rolling commit per push (so hours of a running job
  never turn into hours of commit history), a status file per job, and the job's log split into small chunks so a push only sends
  the tail that changed.

## The loop

1. The agent pulls the jobs repository every few seconds.
2. It runs the next job it hasn't already finished, as a real process in its own process group, so stopping it (on a timeout, or
   because a newer job asked to preempt it) reaches every child process the job spawned too — `fastboot`, `adb`, a background
   `dmesg -w`, not just the shell.
3. It pushes the running job's log and a status file at least every `push_seconds`, and a heartbeat even when idle, so a silent
   agent and a silent lab are never confused for each other.
4. A job's header (`# lab: mode=preempt timeout=900 depends_on=0042`) says whether it should wait for the current job or stop it,
   how long it may run, and what it depends on.

A crash of the agent process itself — not the job it was running — is recovered from on the next start: the interrupted job is
recorded as such (not silently re-run), and the loop continues.

## What this is *not* for

This is not a general CI system. It has no sandboxing: a job script runs as whatever user the agent runs as, with whatever access to
the hardware that user has. It exists because real hardware (a phone, in this case) cannot be virtualised onto a GitHub-hosted
runner, and because iterating against it needs a human in the loop checking results between runs, which this makes possible without
anyone copy-pasting logs back and forth.

## Getting a lab machine running

One command, from nothing: `LAB_AGENT_GITHUB_TOKEN=... bash deploy/connect.sh` creates the two repositories if they don't exist,
installs the agent into its own venv, installs and starts the per-user systemd service, and waits for a real heartbeat to land
in the results repository before calling itself done - not just that the service started, but that the whole loop actually
works. Safe to run again any time; every step checks whether its own work is already done. See the script's own header for what
it needs (a classic GitHub token with `repo` scope, typed into your own shell) and what it does with it (the token lives only in
one 600-permission file, read by the systemd unit; it is never written into `config.toml` or a git remote URL on disk).

See [`lab_agent/agent.py`](../lab_agent/agent.py) for the loop itself and [`kit/AGENT-INSTRUCTIONS.rst`](../kit/AGENT-INSTRUCTIONS.rst)
(from [ideal-project-playbook](https://github.com/creation-guidelines/ideal-project-playbook)) for the standards this project and the
work it enables both follow.
