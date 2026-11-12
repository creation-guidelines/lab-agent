# oco-agent

The commons' system for coordinating work, growing here beside lab-agent, which serves until oco-agent is
proven and takes over (then this repository takes its name). The design, and why: [`../DESIGN.md`](../DESIGN.md).

- [`core/`](core/) - the rules, with nothing in them that touches the world: the journal (what happened to
  each job, the only truth about it), leases (who may act, and until when), deciding (what the agent does
  next). `core/tests/chaos.rs` tries every way the old agent was broken on 2026-11-11, on purpose.
