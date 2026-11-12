# oco-agent: lab-agent, redesigned (2026-11-12)

The redesigned program is called **oco-agent** (decision 2026-11-12-oco-agent): built on ocoeco, born beside
lab-agent, which serves until the switch. Whoever drives it - a person or a machine - is *the driver*, and oco-agent treats them alike: it is the commons' system for coordinating work.

Written after the incident of 2026-11-11: a stall, then 102 finished jobs replayed after a reboot -
fifty at a time after a reconnect, on shared clones - while no alert could reach the author. No harm was
found, but the foundation that allowed it was laid when lab-agent was a one-off prototype: a job was a
script, "pending" was inferred from a missing result, nothing held a lease, nothing heartbeat, nothing
watched from outside, and jobs shared working copies.

## What the world has learned (the research behind this)

- **Durable execution** (Temporal and its peers): every step of a piece of work is recorded in an
  append-only event history; after a crash the work resumes from the last recorded step and never repeats
  a completed one. Long steps send heartbeats with their progress, so a lost worker is noticed within one
  heartbeat timeout.
- **Leases and fencing tokens** (Kleppmann, 2016; Patterns of Distributed Systems): a lease is a lock
  with a time limit - acquired, renewed, released, expired. A holder can wake after its lease ran out, so
  every action on the protected resource carries a token that only grows, and the resource refuses a
  stale one.
- **A dead man's switch** (Healthchecks and others): alert on absence, not failure - and keep the watcher
  somewhere other than the thing it watches.
- **Crash-only software** (Candea and Fox, 2003): a system that can be killed at any moment and recovers
  by reading its own record, the same way every time - so a reboot is never a special case.
- **Run once, by construction** (Kubernetes Jobs; idempotency keys): a finished job is never started
  again; an unstarted one expires after a deadline instead of waiting forever; a step that changes the
  world carries a key, so doing it twice finds it already done.

## The design, in ten points

1. **A job is a record with a journal**, not a script: submitted, claimed (with a lease and a fencing
   token), heartbeat (with the step reached), step done, finished (ok, failed, timed out, cancelled),
   parked. The journal is the truth; nothing is inferred from a missing file.
2. **Run once, by construction.** A job starts only from "submitted" with nothing after it; a new attempt
   is a new, explicit record. A job not claimed by its start-by deadline expires. The agent never starts
   a job older than its own start unless told to resume it, and connecting never resets the journal.
3. **The driver's lease** (the author's heartbeat): the session driving the lab - an agent like Claude,
   or the author - holds a lease renewed by heartbeats sent automatically by the client while it works.
   If the lease runs out, the agent starts nothing new, asks running jobs to stop at their next step,
   records them parked ("the driver went silent"), and waits to be told to resume.
4. **Each job's lease:** a running job heartbeats with its progress; one that stops is stopped after its
   heartbeat timeout and recorded "timed out at step N".
5. **Flows of steps:** a job is a named sequence of steps; each step that changes the world has an
   idempotency key (the branch, pull request or tag it makes) and is journalled when done. A resumed flow
   skips its finished steps - a rollout over 21 repositories resumes at the 13th, it does not restart.
6. **Isolation and resources:** each job works in its own git worktree, never a shared clone; a job
   declares what it needs (a repository, the phone, the network), and the agent runs side by side only
   what needs different things. The fencing token goes with every action on a shared resource.
7. **Crash-only:** killed at any moment - reboot, kill, power - the agent starts by reading the journal:
   a job whose lease ran out becomes "interrupted at step N" and is parked, never replayed.
8. **A voice that does not depend on the hands:** the agent's own heartbeat is watched from outside the
   machine - a scheduled GitHub workflow that alerts the author's urgent channel when the heartbeat goes
   stale - and the client can raise an alarm without the agent (a GitHub issue mentioning the author).
9. **Secrets out of sight:** git gets the token from a credential helper, never inside a URL, an argument
   or a child's environment; the token is rotated.
10. **Through ocoeco:** a finished flow records its glimpse (what it did, the machine time it took); the
    board is the backlog items in progress and the live flows; progress updates are composed from the
    journal. lab-agent depends on ocoeco, never the reverse.

All of it in Rust with argenv: the job's states are one enum whose transitions the compiler checks, and
every point above is proven by a chaos test - kill the agent mid-step, reboot, reconnect, cut the network,
silence the driver - each showing no replay, a park on silence, and a resume from the right step.

## How it is built: the canon's ports and adapters, the playbook's names

The canon's default architecture is ports and adapters (hexagonal): the core talks to the world only through
ports - conversations it defines in its own words - and adapters plug technologies into them; in-memory
adapters make tests trivial. The ideal-project playbook adds what makes it good:

- **Names explain the program.** Areas are named for what lab-agent is for, never for patterns: taking work,
  doing it, keeping the journal, holding leases, resuming, speaking (progress and alarms), listening (the
  author's messages), watching (from outside). A module's first sentence says what it is for, in plain words.
- **Tool words live only in the adapter that drives them.** GitHub, git, ntfy, systemd, a commit, a push
  appear in their adapters and nowhere above; the core asks the question it means ("is there new work?",
  "has the driver gone silent?") and the adapter owns the how.
- **The boundary is checked, not hoped for.** The core is its own crate with no input or output at all, so
  the compiler enforces it; the playbook's guards run on every change: purity by role, vocabulary (a tool
  word above its adapter fails), one place, and findability (a cold reader finds the right area for a real
  question).

Today's adapters: work arrives and the journal travels through git and GitHub; the voice and the author's
messages go through ntfy; the machine's service is systemd. Replacing one - git with Redis, ntfy with
another channel - is a new adapter passing the same checks the in-memory one passes; the core does not
change.

## The journal is data: tad tables, queried with SQL

The journal is not a heap of files in a repository: it is declared tad tables - jobs, their events, steps,
leases, the author's messages, waits - checked like any tad data and queried with real SQL (tad's query
command reads the declared tables through DuckDB, as tad's original research chose; a change goes through tad's checks before it is
kept). Where the tables live is itself an adapter: a folder moved by git today, Redis or a database later.
And every finished piece of work becomes a glimpse in ocoeco's shared language, so what the hands did is
intelligible to the whole commons.

## Why not adopt a durable-execution engine

Temporal, Restate and their peers solve this in general, but each needs a server to run, secure and keep
up; lab-agent's constraints are one machine, no inbound port, and git as the transport and the durable
record it already has. So lab-agent takes their proven ideas - the journal, leases, heartbeats, idempotent
steps - into a small core of its own, and records the reason.

## The stages, each proven before the next

1. **Safety first** - close the incident's paths: run-once and the start-by deadline, the agent's start
   as a watermark, connect never resetting the journal; the driver's lease and park-on-silence; the
   outside watcher; the client's alarm.
2. **The journal and flows** - jobs as journalled flows of idempotent steps; resume from a step.
3. **Isolation and resources** - worktrees per job; side by side by resource; fencing tokens.
4. **Through ocoeco** - glimpses from flows; the board as a view; updates composed from the journal.
5. **Secrets** - the credential helper; the token rotated.

## The switch (2026-11-12)

oco-agent takes over the running of jobs from lab-agent's daemon step by step, each step with a way back.
What does **not** move yet: lab-agent's daemon also carries the author's voice - a notification when a job
ends, the board pushed to the phone every few minutes - so it keeps running until oco-agent has a voice of
its own; it is simply no longer asked to run jobs. lab-agent's client (board, inbox, notify, ask) stays.

1. **oco-agent serves beside lab-agent** - a service of its own on the lab (`oco-agent/service/install`),
   listening and speaking to the jobs repository, its work under `oco-agent/jobs`, apart from lab-agent's
   `jobs/`. Each agent does one job at a time; a driver uses one agent at a time, so they do not work in
   the same clones together. *Way back: `systemctl --user disable --now oco-agent`.*
2. **oco-agent's client reaches drivers** - shipped in lab-agent's release (the same binaries and
   packages), so a driver anywhere installs it as it installs lab-agent. *Way back: none needed.*
3. **Drivers move** - first the AI driving the lab: it asks through oco-agent (`--via`), and falls back to
   lab-agent, still running, if anything is wrong. *Way back: ask through lab-agent again.*
4. **The watcher probes oco-agent** - so the outside watch tests the agent that does the work; the shadow
   keeps judging lab-agent, which should start nothing. *Way back: reinstall the earlier workflow.*
5. **lab-agent's daemon stops running jobs** - once oco-agent has carried the work for a while without the
   shadow or the watcher speaking up, and has a voice of its own. Then the repository takes oco-agent's name.
