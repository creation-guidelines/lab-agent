# lab-agent

A pull-based agent for a lab machine: it runs the scripts it finds in a git repository, one at a time, and reports progress back
through git. **Outbound connections only** — no port forwarding, no DDNS, nothing inbound.

To run it on a machine, one command - no system Python packages needed, it creates its own virtual environment:

```
LAB_AGENT_GITHUB_TOKEN=... bash deploy/connect.sh
```

Creates the jobs/results repositories if they don't exist, installs the agent, starts it, and proves a real heartbeat lands
before it calls itself done. See [`docs/design.md`](docs/design.md) for how the pieces fit together.

For development: clone, then `python3 -m venv .venv && .venv/bin/pip install -e .` (`dev/gate.sh` to check your work -
installs `ruff` into the same venv, not system-wide).

Built to drive real hardware iteration (phones, boards — anything that can't be virtualised onto a hosted CI runner) with an AI
agent making the changes between runs, but it has no opinion about what a job script actually does.

Follows [ideal-project-playbook](https://github.com/creation-guidelines/ideal-project-playbook); see
[`AGENTS.md`](AGENTS.md) and [`kit/AGENT-INSTRUCTIONS.rst`](kit/AGENT-INSTRUCTIONS.rst).
