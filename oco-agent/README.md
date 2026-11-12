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
- [`command-line/`](command-line/) - the program a driver types: `oco-agent beat | submit | release | standing | next`,
  its command line declared once with argenv. Today it speaks a driver's side; it does not yet run jobs.
- [`shadow/`](shadow/) - the shadow run: every ten minutes a timer on the lab feeds lab-agent's newest log lines
  through oco-agent's rules and alerts urgently if lab-agent started work oco-agent would have refused. It
  decides; it never acts. Installed with `oco-agent/shadow/install`.
- [`watcher/`](watcher/) - the watcher outside the machine: a workflow in the jobs repository asks the lab a tiny
  probe every half hour and alerts the urgent channel itself if the lab has not answered for 45 minutes -
  an alert path that does not go through the lab. Installed with `oco-agent/watcher/install OWNER/JOBS-REPO`.
