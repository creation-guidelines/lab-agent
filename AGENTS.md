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
