# AGENTS.md

Instructions for an AI agent working in this repository.

## What this is

`lab-agent` is a pull-based job runner for hardware-in-the-loop work. It polls a jobs repo
(a plain git repo with numbered `.sh` scripts under `jobs/`), runs each script on whatever
machine the agent is installed on, and pushes status + logs to a results repo. It exists so an
agent (or a person) can drive real hardware — flash a device, run a build, probe a USB port —
by pushing a file to git, instead of needing a live shell session on that machine.

It is **project-agnostic**: one binary, reusable for any hardware setup. A specific project
(e.g. mainlining a phone's kernel) configures its own jobs/results repo pair via `lab-agent
connect` and is otherwise unrelated to this repo.

## Architecture, briefly

- `src/contract.rs` — the single argenv declaration of every subcommand and input; parsing, linting, `--help`, completions and the man page all derive from it.
- `src/agent.rs` — the agent API (`submit`, `wait`, `logs`); its exit statuses and JSON shapes are a contract, see below.
- `src/completions.rs` — a thin adapter over argenv's generator; nothing is written by hand.
- `src/main.rs` — CLI entry point. Subcommand selection (`connect`/`run`/`monitor`/`root-exec`)
  is plain `argv[1]` dispatch; argenv's `contract!` macro declares the shared flags (currently
  just `--verbose`), not a subcommand tree.
- `src/connect.rs` — one-time interactive setup: prompts, repo creation, the one sudoers grant,
  the systemd unit.
- `src/daemon.rs` — the poll loop (`run` subcommand). Polling and job *execution* are
  deliberately decoupled: a hung job blocks only itself, never new-job pickup. This is the fix
  for the one real failure the previous (Python) implementation had — an orphaned child process
  outliving its parent script wedged the whole agent because nothing else was watching for new
  work while it waited.
- `src/job.rs` — runs one job script in its own process group (`setsid`) with a hard-enforced
  timeout: on expiry, `SIGKILL` goes to the *group*, not just the top-level process. If you touch
  timeout handling, keep it this way — a plain `Child::kill()` does not kill grandchildren.
- `src/github.rs` — a small blocking GitHub REST client (`ureq`, not `reqwest`+`tokio`): read/
  write one file, list a directory, check/create a repo. Nothing more. Keep it that way; this
  project does not need an async runtime or a generated SDK.
- `src/monitor.rs` — the `monitor` TUI (ratatui), reads the results repo.
- `packaging/` — release artifacts: `.deb`/`.rpm`/tarball (`build-binary.sh`,
  `build-packages.sh`), the Arch `PKGBUILD`, and `packaging/repo/` — the signed pacman
  repository (GPG key `packaging/repo/lab-agent-repo.asc`, fingerprint in
  `packaging/repo/FINGERPRINT.txt`). The private half of that key is the `REPO_SIGNING_KEY`
  GitHub Actions secret; nothing else has it, and the jobs that use it (`with-key.sh`,
  `build-arch.sh`) never build or run the project's own code.

## The agent API: using lab-agent from another program

If you are an AI agent (or a CI job, or a script) and a lab machine runs `lab-agent run`, you do
not need a shell on it. Push a script, wait for it, read its output. Everything goes through the
jobs and results repos on GitHub, so nothing listens on a port and the machine can sit behind NAT.

**Configure a client with the environment - never with flags** (a flag lands in shell history and
process listings):

| variable | meaning |
|---|---|
| `LAB_AGENT_TOKEN` | GitHub token with `repo` scope |
| `LAB_AGENT_OWNER` | user or org that owns the repos |
| `LAB_AGENT_JOBS_REPO` | default `lab-agent-jobs` |
| `LAB_AGENT_RESULTS_REPO` | default `lab-agent-results` |
| `LAB_AGENT_API_URL` | GitHub Enterprise API root; default `https://api.github.com` |

On the lab machine itself, `lab-agent connect` writes a config file instead and none of these are
needed.

**The verbs** (all of them are declared in `src/contract.rs`; `lab-agent --help` lists them):

```sh
# run a script on the lab machine and wait: its output streams to stdout, the status is the exit code
lab-agent submit --wait --name kernel-build < build.sh

# ... or as JSON: one object at the end, with the whole log in "log"
lab-agent submit --wait --json --timeout 1800 build.sh

# asynchronous: prints the job id; read it later
ID=$(lab-agent submit --name probe probe.sh)
lab-agent wait "$ID" --within 600       # streams output; exit 75 if it is still running after 600s
lab-agent logs "$ID"                    # the output so far (a running job's log is pushed every ~8s)
lab-agent jobs -n 20 --json             # recent jobs: name, status, exit_code, started_at, finished_at
```

**A job id** is `<unix-seconds>-<name>`; ids sort in run order, and the job's script is
`jobs/<id>.sh` in the jobs repo. A script may start with a header, which `submit` writes for you
from `--mode` / `--timeout`:

```sh
# lab: mode=queue timeout=300
```

- `queue` (default): runs when there is room under the agent's `max_parallel`.
- `preempt`: cancels the jobs running at that moment, then runs first.
- `timeout`: seconds; when it expires the job's **whole process group** gets SIGTERM, then 10
  seconds to clean up (`trap 'umount ...; rm ...' TERM`), then SIGKILL for whatever is left - so a
  stuck child cannot hold the agent, and a job that mounted something can put it back. Preemption
  stops a job the same way. A job that mounts or attaches anything should install such a trap.

**Exit statuses** of `wait` and `submit --wait`:

| status | meaning |
|---|---|
| `0` | the job succeeded |
| the job's own status | it failed (`1` if it reported none) |
| `75` | `--within` elapsed; the job may still finish (also: never picked up - is the agent running?) |
| `124` | the job hit its own `timeout` (as `timeout(1)`) |
| `125` | the job was cancelled (preempted) or cut off by an agent restart (`interrupted`) |
| `69` | GitHub's hourly request budget is used up; nothing is lost - the job still runs, run `wait` again later |

**`--json` shapes.** `submit` (no `--wait`): `{"name": ID, "file": "jobs/ID.sh"}`. `wait` and
`submit --wait`: `{"name", "status", "exit_code", "started_at", "finished_at", "elapsed_secs",
"exit", "log"}`, where `status` is one of `running`, `ok`, `failed`, `timed_out`, `preempted`,
`interrupted`, `exit` is the status the command exits with, and times are Unix seconds.
`jobs --json`: a list of `{"name", "status", "exit_code", "started_at", "finished_at",
"timeout_secs", "stale"}`. **`stale: true` means the row says `running` but is not a live job**: it
has outlived its time limit plus two minutes with no result recorded (the agent died, or could not
push). Treat it as unknown, never as running; `lab-agent jobs` and `monitor` show it as `stale`.

**Rules a caller should know**

- A job's outcome is written to disk the instant it is known and pushed afterwards, and pushed
  again on every poll until GitHub has it: a failed push or a spent budget delays a result, it does
  not lose it, and a finished job is not left saying `running`.
- A job with a result is never run again, whatever happens to the agent. To run something again,
  submit it again (a new id).
- Root is `sudo -n lab-agent root-exec -- COMMAND ARGS...` inside the script. There is no other
  path, and `connect` granted that one line once.
- Scripts live in a git repo and logs in another: **do not put secrets in a script or print one**.
- A log longer than 900 KB keeps its tail (GitHub's contents API serves files up to 1 MB).
- **The budget is GitHub's, per user.** 5000 REST requests an hour, shared by *every* token of the
  same user, so a busy session elsewhere can starve the agent (and the agent's own polling counts).
  The daemon's idle polling costs nothing (conditional requests: a 'not modified' answer is free);
  when the budget runs out it pauses until it returns and waits for it before giving up on a
  result, so a finished job's output is not lost. `lab-agent` exits `69` and says when it returns.
- **On the machine that runs the agent, `monitor` and `jobs` read the agent's own records** (a
  `<job>.status.json` beside each log in `~/.local/state/lab-agent/`): instant, no token, no
  network, and they work while GitHub is refusing requests. Everywhere else (an agent's sandbox, a
  laptop) they read GitHub; `--remote` forces that on the machine too.
- Leaving `lab-agent monitor` open all day costs almost nothing: every re-read is conditional and a
  finished job never changes. (Until 0.1.10 it re-read every file every 3 seconds, ~37,000
  requests an hour, and spent the whole budget in minutes.)
- Polling is cheap: reads are conditional (ETag), and a "not modified" answer does not count
  against GitHub's rate limit. Writes are one commit each; do not submit in a tight loop.
- Prefer `submit --wait --json` for one-shot work and `submit` then `wait` for long work, so a
  dropped connection does not lose the job: ids are durable and `wait` can be run again.
- Heavy work (builds, anything that wants the machine's cores, anything that touches hardware)
  belongs on the lab machine, not in your own sandbox.

## Notifications (ntfy)

`lab-agent notify-setup` (also offered at the end of `connect`) gives each user **their own two
channels** on [ntfy](https://ntfy.sh): `info` for progress, `urgent` for "you are needed". Their
names are 120-bit random strings made on that user's machine and kept only in `notify.toml`
(mode 0600) beside the config - they are the secret, so never put one in a flag, a log, a script
or a repo. Nobody shares a channel: two users of lab-agent never see each other's messages.

- On the phone, subscribe by scanning the QR codes `notify-setup` prints (or opening its `ntfy://`
  links on the phone): the app's `+` button suggests a new random name, and an `https://` link opens
  the web app. Urgent ringing through Do Not Disturb is an Android setting: Settings > Apps > ntfy >
  Notifications > "Max priority" > override Do Not Disturb.
- `lab-agent notify "text"` / `lab-agent notify --urgent "text"` sends from the machine it runs on.
- `lab-agent submit --notify job.sh` (or `notify=yes` in the `# lab:` header) makes the agent tell
  the phone how the job ended: info if it succeeded, urgent if not, with its last lines.
- **An agent working remotely does not need the secret**: it submits a job that runs
  `lab-agent notify ...`, or submits with `--notify`; the lab machine sends.
- A sender with no `notify.toml` (CI, a sandbox) can use `LAB_AGENT_NTFY_INFO_TOPIC`,
  `LAB_AGENT_NTFY_URGENT_TOPIC`, `LAB_AGENT_NTFY_SERVER`, `LAB_AGENT_NTFY_TOKEN`.
- On the public ntfy.sh server, "private" means unguessable, not encrypted. For encryption or
  access control, use your own ntfy server or one with access tokens; setup asks for both.

## Talking while working: the board and the inbox

So the user can follow a long session, and speak into it, without interrupting it:

- **The board** (`board.json` in the jobs repo) says what is being done, a sentence per task:
  `lab-agent board set ID "what is being done"`, then `wait ID` (waiting for the user), `pause ID`,
  `resume ID`, `done ID`. `lab-agent board` alone, in a terminal, is a live view with a color per state
  (on the agent's machine it reads a local copy: no network); `board show` prints it once. Done
  tasks stay a day.
- **The control board** (`lab-agent board` in a terminal) is one live view of the whole system: tasks
  by state, the jobs, the conversation (the user's messages, read ticks, the agent's replies) and a
  reply box (Enter sends to the inbox). Its layout follows the window: side by side when wide,
  a 2x2 grid when square (tiling), stacked when tall (a phone).
- **Desktop popups**: every notification also pops up on the machine's desktop (notify-send: KDE,
  GNOME, XFCE, mako, dunst; without a notification service a zenity/kdialog window), info as
  normal, urgent as critical (stays until dismissed). Read receipts do not pop up. `desktop = false`
  in notify.toml turns it off. Send the heads-up (what to expect, what to check) *before* the
  thing happens: the popup is instant even when the phone's ntfy app is late.
- **Sent messages show at once** on the board: sending..., sent, then the filed line; a failed one
  stays in red with the reason.
- **Delivery delays are the phone's**, not the sender's: Android holds ntfy messages back unless each
  subscription has Instant delivery on and ntfy is Unrestricted in battery settings.
  The machine running the agent sends the open tasks to the user's info channel every 5 minutes
  (`lab-agent board every MINUTES` changes it), says once when all are done, and costs the
  writer nothing but one small write per change.
- **The inbox** is a third ntfy channel, made by `notify-setup`, that the user writes to from the
  ntfy app, or from the machine with `lab-agent inbox "text"`. The machine files new messages in
  `inbox.json` (results repo). `lab-agent inbox` shows
  the unread ones; `--ack` marks them read (the user gets a receipt), `--reply "text"` sends a short
  reply to the info channel.

**Working agreement for an agent in a long session:** set a board task when a block of work starts
and mark it done when it ends; run `lab-agent inbox` at every natural pause (between steps, never in
the middle of one); act on what it says, `--ack` it, and `--reply` briefly when an answer helps.
A message is the user's word, but anything destructive or surprising is confirmed in the chat first:
the inbox topic is a secret, not a signature.

## Conventions this repo follows

Part of the `creation-guidelines` org; follows
[ideal-project-playbook](https://github.com/creation-guidelines/ideal-project-playbook).
Specifically:

- Conventional commits (`feat:`, `fix:`, `chore:`, ...) — `release-plz` and `cliff.toml` depend
  on this for the changelog.
- One iteration, one branch, one PR. Don't push directly to `main`.
- `cargo fmt`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test` all
  need to pass before a PR merges — `ci.yml` enforces this, but run it locally first; it's
  cheaper than waiting on CI.
- Rust by default for new tooling in this org (not Python) — a statically-typed compiler catches
  what a dynamically-typed one lets through to runtime.
- Prefer existing, well-tested tools and crates over hand-rolled ones. This project's CI and
  packaging were deliberately copied from `grenudi/snapall` (another Rust CLI in the same
  pattern) wherever the pattern was generic, rather than written from scratch — do the same
  before adding new machinery here.
- Conserve tokens/effort on ad-hoc work: when a task is going to repeat, automate it once instead
  of re-deriving it by hand each time.

## Things to not break

- The GitHub client stays synchronous (`ureq`), no `tokio`. If a genuine need for concurrency
  beyond the daemon's own job-parallelism threads comes up, prefer `std::thread`, not an async
  runtime.
- `root-exec` is the *only* privileged path. Do not add per-command sudoers entries anywhere —
  if a job needs root, it calls `sudo lab-agent root-exec -- <command>`.
- The timeout kill must stay group-wide (`killpg`, not a direct `Child::kill()`). This is the
  actual bug this rewrite fixes; regressing it brings back the original failure mode.
- The signing key's private half must never be committed, logged, or printed. It lives only as
  the `REPO_SIGNING_KEY` secret.
