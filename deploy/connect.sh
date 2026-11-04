#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The one command to set up a lab machine from nothing: creates the two private repositories (jobs, results) if they don't
# exist, installs lab-agent into its own venv, installs and starts the user systemd service, then waits and checks that a
# real heartbeat actually reached the results repository before calling itself done. Safe to run again at any point (every
# step checks whether its own work is already done).
#
#   LAB_AGENT_GITHUB_TOKEN=ghp_xxx bash deploy/connect.sh
#
# The token is typed into YOUR OWN shell, never pasted anywhere else. It is used here to create the repositories, and it is
# what the running agent itself uses too (over plain HTTPS, like every other git operation in this project): it ends up in
# ~/.config/lab-agent/token, a file only you can read (chmod 600), referenced by the systemd unit - never embedded in
# config.toml or in a git remote URL on disk. A classic PAT with the "repo" scope is enough (github.com/settings/tokens).
#
# Optional settings, as environment variables:
#   LAB_AGENT_REPO_OWNER    where to create the repos (default: the account the token belongs to)
#   LAB_AGENT_JOBS_REPO     default "lab-agent-jobs"
#   LAB_AGENT_RESULTS_REPO  default "lab-agent-results"
#   LAB_AGENT_SOURCE        where to install the agent itself from (default: this repository's own origin remote)
set -eu

TOKEN="${LAB_AGENT_GITHUB_TOKEN:?set LAB_AGENT_GITHUB_TOKEN in your own shell, not in a chat - see the top of this script}"
JOBS_REPO="${LAB_AGENT_JOBS_REPO:-lab-agent-jobs}"
RESULTS_REPO="${LAB_AGENT_RESULTS_REPO:-lab-agent-results}"
HOME_CFG="$HOME/.config/lab-agent"
HOME_VENV="$HOME/.local/share/lab-agent/venv"
HOME_SYSTEMD="$HOME/.config/systemd/user"

say() { printf '%s %s\n' "$(date +%H:%M:%S)" "$1"; }
api() { curl -sS -H "Authorization: Bearer $TOKEN" -H "Accept: application/vnd.github+json" "$@"; }

# ---- preflight: fail with a clear, actionable message instead of halfway through ----------------------------------------------
missing=""
for tool in python3 git curl systemctl loginctl; do command -v "$tool" >/dev/null 2>&1 || missing="$missing $tool"; done
if [ -n "$missing" ]; then
  echo "connect.sh: missing:$missing" >&2
  echo "  on Debian/Ubuntu:  sudo apt install python3 python3-venv git curl" >&2
  exit 1
fi
python3 -c 'import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)' || { echo "connect.sh: need Python 3.11 or newer"; exit 1; }
for tool in fastboot adb; do
  command -v "$tool" >/dev/null 2>&1 || echo "note: '$tool' is not on PATH yet - job scripts that use it will fail until it is (Debian/Ubuntu: sudo apt install android-sdk-platform-tools-common android-tools-fastboot android-tools-adb)"
done

# ---- who, and where -------------------------------------------------------------------------------------------------------------
OWNER="${LAB_AGENT_REPO_OWNER:-$(api https://api.github.com/user | python3 -c 'import json,sys; print(json.load(sys.stdin)["login"])')}"
say "repository owner: $OWNER"

# ---- repositories: created private and idempotently (a 200 from the API means it already exists, which is success) -----------
create_repo_if_missing() {
  name="$1"
  code=$(api -o /dev/null -w '%{http_code}' "https://api.github.com/repos/$OWNER/$name")
  if [ "$code" = "200" ]; then
    say "$name already exists"
    return
  fi
  say "creating $name (private)"
  out=$(api -X POST "https://api.github.com/user/repos" -d "{\"name\":\"$name\",\"private\":true,\"auto_init\":true}")
  echo "$out" | python3 -c 'import json,sys; d=json.load(sys.stdin); sys.exit(0 if "id" in d else 1)' \
    || { echo "connect.sh: could not create $name: $out" >&2; exit 1; }
}
create_repo_if_missing "$JOBS_REPO"
create_repo_if_missing "$RESULTS_REPO"

# a branch named "results" must exist before anything can push to it: create it from the repo's default branch if it is not there
ensure_branch() {
  repo="$1"; branch="$2"
  code=$(api -o /dev/null -w '%{http_code}' "https://api.github.com/repos/$OWNER/$repo/branches/$branch")
  [ "$code" = "200" ] && return
  default=$(api "https://api.github.com/repos/$OWNER/$repo" | python3 -c 'import json,sys; print(json.load(sys.stdin)["default_branch"])')
  sha=$(api "https://api.github.com/repos/$OWNER/$repo/git/ref/heads/$default" | python3 -c 'import json,sys; print(json.load(sys.stdin)["object"]["sha"])')
  say "creating branch $branch on $repo (from $default)"
  api -X POST "https://api.github.com/repos/$OWNER/$repo/git/refs" -d "{\"ref\":\"refs/heads/$branch\",\"sha\":\"$sha\"}" >/dev/null
}
ensure_branch "$RESULTS_REPO" "results"

# make sure the jobs repository has the jobs/ directory the agent looks in, so the very first poll finds a real (empty) one
ensure_jobs_dir() {
  code=$(api -o /dev/null -w '%{http_code}' "https://api.github.com/repos/$OWNER/$JOBS_REPO/contents/jobs/.keep")
  [ "$code" = "200" ] && return
  say "seeding $JOBS_REPO's jobs/ directory"
  api -X PUT "https://api.github.com/repos/$OWNER/$JOBS_REPO/contents/jobs/.keep" \
    -d '{"message":"chore: the jobs directory","content":""}' >/dev/null
}
ensure_jobs_dir

# ---- the agent itself, in its own venv: does not touch system Python at all ----------------------------------------------------
SOURCE="${LAB_AGENT_SOURCE:-git+https://github.com/creation-guidelines/lab-agent.git}"
if [ ! -x "$HOME_VENV/bin/lab-agent" ]; then
  say "creating the venv and installing lab-agent"
  python3 -m venv "$HOME_VENV"
  "$HOME_VENV/bin/pip" install -q --upgrade pip
  "$HOME_VENV/bin/pip" install -q "$SOURCE"
else
  say "upgrading lab-agent in its existing venv"
  "$HOME_VENV/bin/pip" install -q --upgrade "$SOURCE"
fi

# ---- a phone attached over USB right now: used only to fill in PHONE_SERIAL automatically when there is exactly one ------------
phone_extra_env=""
if command -v fastboot >/dev/null 2>&1; then
  serials=$(fastboot devices 2>/dev/null | awk '{print $1}')
  count=$(echo "$serials" | grep -c . || true)
  if [ "$count" = "1" ]; then
    phone_extra_env="
[extra_env]
PHONE_SERIAL = \"$serials\""
    say "found one phone in fastboot (serial $serials): added as PHONE_SERIAL"
  elif [ "$count" -gt 1 ]; then
    say "more than one device in fastboot: edit $HOME_CFG/config.toml's [extra_env] yourself"
  fi
fi

# ---- config.toml and the token file: the token lives ONLY in its own 600 file, never in config.toml or a git remote URL -------
mkdir -p "$HOME_CFG"
chmod 700 "$HOME_CFG"
printf 'LAB_AGENT_GITHUB_TOKEN=%s\n' "$TOKEN" > "$HOME_CFG/env"
chmod 600 "$HOME_CFG/env"

cat > "$HOME_CFG/config.toml" <<EOF
# Written by connect.sh. Re-running connect.sh overwrites this file; anything you need to keep belongs in the job scripts
# themselves, not here. The token is in ./env (chmod 600), referenced by the systemd unit, never here.
[jobs]
remote = "https://github.com/$OWNER/$JOBS_REPO.git"

[results]
remote = "https://github.com/$OWNER/$RESULTS_REPO.git"
$phone_extra_env
EOF

# ---- the user service: no root for this part --------------------------------------------------------------------------------
HERE="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
mkdir -p "$HOME_SYSTEMD"
cp "$HERE/lab-agent-user.service" "$HOME_SYSTEMD/lab-agent.service"
systemctl --user daemon-reload
systemctl --user enable --now lab-agent

if ! loginctl show-user "$(whoami)" -p Linger 2>/dev/null | grep -q "Linger=yes"; then
  echo
  echo "ONE THING NEEDS ROOT, ONCE, so the service keeps running after you log out:"
  echo "    sudo loginctl enable-linger $(whoami)"
  echo
fi

udev_rule="/etc/udev/rules.d/51-android.rules"
if [ ! -f "$udev_rule" ]; then
  echo "to use fastboot/adb without root, also once:"
  echo "    sudo cp \"$HERE/51-android.rules\" $udev_rule && sudo udevadm control --reload-rules"
  echo
fi

# ---- prove it, don't just claim it: wait for systemd to say it's up, then for a real heartbeat to land in the results repo ----
say "waiting for the service to start"
for _ in $(seq 1 15); do
  systemctl --user is-active --quiet lab-agent && break
  sleep 1
done
if ! systemctl --user is-active --quiet lab-agent; then
  echo "connect.sh: the service did not start; see: journalctl --user -u lab-agent -n 50" >&2
  exit 1
fi

say "waiting for the first heartbeat to reach $RESULTS_REPO (up to 90s)"
# the GitHub API, not raw.githubusercontent.com: that is a CDN that can lag minutes behind a push you just made, which would
# report "not ready" on a setup that had, in fact, already worked.
for _ in $(seq 1 18); do
  code=$(api -o /dev/null -w '%{http_code}' "https://api.github.com/repos/$OWNER/$RESULTS_REPO/contents/status.json?ref=results")
  if [ "$code" = "200" ]; then
    say "connected: $OWNER/$JOBS_REPO -> this machine -> $OWNER/$RESULTS_REPO"
    echo
    echo "ready. To give it work: push a file named jobs/0001-whatever.sh to $OWNER/$JOBS_REPO's main branch."
    echo "Watch it:   journalctl --user -u lab-agent -f"
    echo "Results:    https://github.com/$OWNER/$RESULTS_REPO/tree/results"
    exit 0
  fi
  sleep 5
done
echo "connect.sh: the service is running but no heartbeat has reached $RESULTS_REPO yet; see: journalctl --user -u lab-agent -n 50" >&2
exit 1
