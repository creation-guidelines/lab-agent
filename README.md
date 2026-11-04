# lab-agent

A pull-based agent for a lab machine: it runs the scripts it finds in a git repository, one at a time, and reports progress back
through git. **Outbound connections only** — no port forwarding, no DDNS, nothing inbound.

```
pip install git+https://github.com/creation-guidelines/lab-agent.git
```

or clone and `pip install -e .` for development (`dev/gate.sh` to check your work). See [`docs/design.md`](docs/design.md) for how
the pieces fit together. To actually run it on a machine: `LAB_AGENT_GITHUB_TOKEN=... bash deploy/connect.sh` - one command,
creates everything it needs, proves a real heartbeat lands before it calls itself done.

Built to drive real hardware iteration (phones, boards — anything that can't be virtualised onto a hosted CI runner) with an AI
agent making the changes between runs, but it has no opinion about what a job script actually does.

Follows [ideal-project-playbook](https://github.com/creation-guidelines/ideal-project-playbook); see
[`AGENTS.md`](AGENTS.md) and [`kit/AGENT-INSTRUCTIONS.rst`](kit/AGENT-INSTRUCTIONS.rst).
