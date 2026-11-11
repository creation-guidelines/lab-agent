# lab-agent, redesigned (2026-11-12)

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
