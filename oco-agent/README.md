# oco-agent

The commons' system for coordinating work, growing here beside lab-agent, which serves until oco-agent is
proven and takes over (then this repository takes its name). The design, and why: [`../DESIGN.md`](../DESIGN.md).

- [`core/`](core/) - the rules, with nothing in them that touches the world: the journal (what happened to
  each job, the only truth about it), leases (who may act, and until when), deciding (what the agent does
  next), and the measure (the checks every journal must pass, wherever it is kept - every way the old agent
  was broken on 2026-11-11, on purpose).
- [`journal-in-tad/`](journal-in-tad/) - the journal kept in tad tables: a folder you can read and diff, reshape
  with text tools, and ask in SQL. Held to the same measure as the journal kept in memory.
- [`replay/`](replay/) - what oco-agent would have done: an old agent's log, answered start by start. Run over the
  log of 2026-11-11, it is the proof that the incident cannot happen here.
- [`command-line/`](command-line/) - the program a driver types: `oco-agent beat | submit | release | standing | next | run`,
  its command line declared once with argenv. `run` does the work through the processes on this machine:
  a job is claimed before it starts, every process it starts carries its mark and ends with it.
- [`github/`](github/) - oco-agent on GitHub, the only part that knows it: work asked for as files under
  `oco-agent/jobs` in a repository (a top line, read only at the top, says who asked, when, and the limits),
  drivers' beats as its Actions variables. A driver asks with `--via OWNER/REPO`; the runner takes in with
  `run --listen OWNER/REPO`. What happened is spoken back to the same repository's `spoken` branch -
  the journal, and each job's log - so a driver anywhere can `standing`, `log` and `wait` with `--via`.
- [`shadow/`](shadow/) - the shadow run: every ten minutes a timer on the lab feeds lab-agent's newest log lines
  through oco-agent's rules and alerts urgently if lab-agent started work oco-agent would have refused. It
  decides; it never acts. Installed with `oco-agent/shadow/install`.
- [`watcher/`](watcher/) - the watcher outside the machine: `.github/workflows/watch-the-lab.yml` in this public
  repository probes oco-agent through the jobs repository every half hour and alerts the urgent channel
  itself if the lab has not answered for 45 minutes.
- [`service/`](service/) - oco-agent as a service on the lab, beside lab-agent: `oco-agent/service/install OWNER/REPO`.
  It keeps the lab awake while it serves. A job never restarts the agent that runs it - that cuts the job off,
  and the new run parks it; asked from inside a job, the installer leaves the restart to systemd, ten seconds on. The switch, step by step with a way back at each: `../DESIGN.md`.
- **Saying** - drivers say things to one another: `oco-agent say TEXT --to NAME [--urgent]`, `oco-agent inbox
  [--after N]`. What is said is a tad table beside the journal. A driver with a channel on this machine
  (`~/.config/oco-agent/channels.toml`, private: ntfy topics) has it carried to their phone, urgent words on
  the urgent topic; what they answer on their own topic is heard as something they said to everyone; and how
  each of their own jobs ended is said to them once. Only `command-line/src/ntfy.rs` knows ntfy.
