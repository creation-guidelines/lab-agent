# AGENTS - how to work in this repository

Read [`kit/AGENT-INSTRUCTIONS.rst`](kit/AGENT-INSTRUCTIONS.rst) first; it applies here as to any project. This file is what's specific
to `lab-agent`.

## Setup

```
pip install -e . ruff
dev/gate.sh          # syntax, lint, every test (tests/, kit/guards/)
```

## Layout

| Path | What it's for |
|---|---|
| `lab_agent/` | the real package: `agent.py` (the loop), `jobs.py` (job discovery and headers), `runner.py` (running one job as a real process group), `writer.py` (the results tree), `gitio.py` (the only git commands used), `state.py` (what's already finished), `config.py` (settings) |
| `tests/` | one `test_*.py` per module above, run by `dev/run_tests.py` |
| `deploy/` | the systemd unit and install script for a lab machine |
| `kit/guards/` | a copy of the generic guard kit from `ideal-project-playbook`; not this project's style to enforce (excluded from lint), but its own tests still run |
| `docs/design.md` | how the three repositories (this one, a jobs repo, a results repo) fit together |

## Rules

- `lab_agent/` is the product; everything else exists to keep it correct. A change to the loop's behaviour needs a test in
  `tests/test_agent.py` that drives it end to end against real local git repositories (see that file for the pattern), not just a
  unit test of the piece that changed.
- The two repositories an agent talks to (jobs, results) are never the same repository, and the agent never writes to the one it
  reads from; `config.py` refuses to start if they match.
- A real process is stopped with SIGTERM first, SIGKILL only after a grace period, and always by process group
  (`os.killpg`), so a job's children are reached too. `tests/test_runner.py` proves this with real signals, not mocks.
- Push the results as one rolling commit (`gitio.push_results` amends, never creates a new commit): a job pushing its log every few
  seconds for hours must leave no history behind.
