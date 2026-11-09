# lab-agent

A pull-based job runner for hardware-in-the-loop work: it polls a git repo for job scripts,
runs them on whatever machine it's installed on, and reports results through git. Built for
flashing, booting, and testing real hardware from a chat session or a CI pipeline, without
copy-pasting shell commands back and forth.

Project-agnostic: the same binary works for any hardware setup, not one specific device. Point
it at a pair of repos (one for jobs, one for results) and it's ready.

## Install

Add the repository once, then updates come with the rest of your system:

<!-- snippet:repo-pacman -->
```sh
curl -fsSL https://creation-guidelines.github.io/lab-agent/key.asc -o /tmp/lab-agent-repo.asc
sudo pacman-key --add /tmp/lab-agent-repo.asc
sudo pacman-key --lsign-key B345BA33340C75FB327B22CCDD5D5430E6BA4289
printf '[lab-agent]\nServer = https://creation-guidelines.github.io/lab-agent/arch/$arch\nSigLevel = Required\n' | sudo tee -a /etc/pacman.conf
sudo pacman -Sy lab-agent
```

Or build it yourself:

```sh
cargo install --locked --git https://github.com/creation-guidelines/lab-agent
```

## Use

One command sets everything up:

```sh
lab-agent connect
```

It asks for the GitHub owner and project name (used to name the jobs/results repos), a token,
how many jobs may run at once, and the poll interval. Then it:

- creates the jobs/results repos if they don't already exist,
- grants itself one `NOPASSWD` sudoers line (on its own binary, nothing else — see
  [Running things as root](#running-things-as-root)),
- installs and starts a systemd user service.

From there, push a numbered shell script to the jobs repo's `jobs/` directory and it runs:

```sh
jobs/0001-probe-usb.sh
```

A job script can carry a header comment that controls how it runs:

```sh
#!/bin/sh
# lab: mode=preempt timeout=300
echo "hello from real hardware"
```

- `mode=queue` (default) — runs after everything already queued.
- `mode=preempt` — kills whatever is currently running (the whole process group, not just the
  top-level script) and jumps the queue. For "stop what you're doing, do this now."
- `timeout=N` — seconds before the job's whole process group is stopped: SIGTERM, then 10 seconds
  to clean up (`trap ... TERM`), then SIGKILL. Enforced on the group, not just the immediate
  child, so a hung subprocess (e.g. a `fastboot reboot` that never returns) can't wedge the agent
  the way it would a naive `Child::kill()`.

Results land in the results repo, one directory per job:

```
jobs/0001-probe-usb/status.json
jobs/0001-probe-usb/log/0000.txt
```

Watch it live:

```sh
journalctl --user -u lab-agent -f    # raw logs
lab-agent monitor                     # a TUI listing job statuses
```

## For agents and scripts

A client needs no config file, only the environment (never a flag, so nothing lands in shell
history): `LAB_AGENT_TOKEN`, `LAB_AGENT_OWNER`, optionally `LAB_AGENT_JOBS_REPO` /
`LAB_AGENT_RESULTS_REPO`.

```sh
lab-agent submit --wait --name build < build.sh      # run on the lab machine; output streams back
lab-agent submit --wait --json build.sh              # ... as JSON, with the log in "log"
lab-agent jobs -n 10 --json                          # recent jobs
```

The job's exit status is the command's exit status (`124` timeout, `125` cancelled, `75` still
running). The full contract - ids, JSON shapes, exit statuses, rules - is in
[AGENTS.md](AGENTS.md#the-agent-api-using-lab-agent-from-another-program).

## Watching jobs

`lab-agent monitor` is a live table of jobs; `lab-agent jobs [-n N] [--json]` prints the same once.
On the machine that runs the agent they read its own records (instant, no network, works when
GitHub is unreachable); anywhere else they read the results repo on GitHub, and `--remote` forces
that on the machine too. A `running` job that has outlived its limit is shown as `stale`.

## Running things as root

Job scripts that need root don't get a sudoers line per command. `connect` grants exactly one,
on the `lab-agent` binary itself:

```sh
sudo lab-agent root-exec -- mount -o ro /dev/loop0 /mnt
```

This is the only privileged path there is. No per-command sudoers edits, ever.

## Development

```sh
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

See [AGENTS.md](AGENTS.md) for how this repo is organized and the conventions it follows.

## License

MIT
