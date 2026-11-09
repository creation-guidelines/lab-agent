#!/usr/bin/env bash
# The README's repo-pacman snippet, run as written on a real system against the live repository:
# it installs the newest release, then a system update (pacman -Syu) leaves it there. The
# repository must already be published. The snippet is cut from the README at its marker.
#
# usage: repo-snippet.sh
# env:   SNIPPET_REF  git ref whose README to test (default: main)
set -euo pipefail
repo=creation-guidelines/lab-agent; ref="${SNIPPET_REF:-main}"
work="$(mktemp -d)"; cd "$work"
fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "  ok  $*"; }

releases_json() {
  if [ -n "${RELEASES_JSON_FILE:-}" ]; then cat "$RELEASES_JSON_FILE"
  else curl -fsS "https://api.github.com/repos/$repo/releases?per_page=100"; fi
}
newest_tag() {
  grep -o '"tag_name": *"lab-agent-v[0-9.]*"' | grep -o 'lab-agent-v[0-9.]*' | sed 's/^lab-agent-v//' \
    | sort -u -t. -k1,1nr -k2,2nr -k3,3nr | head -n1
}
latest="$(releases_json | newest_tag)"
echo "newest release: $latest"

for _ in $(seq 1 30); do curl -fsS -o /dev/null https://creation-guidelines.github.io/lab-agent/key.asc && break; sleep 10; done
curl -fsSL "https://raw.githubusercontent.com/$repo/$ref/README.md" -o README.md
snippet="$(awk -v id="repo-pacman" '
  $0 == "<!-- snippet:" id " -->" { want = 1; next }
  want && /^```/ { if (inblock) exit; inblock = 1; next }
  inblock { print }' README.md)"
[ -n "$snippet" ] || fail "no snippet marked repo-pacman in the README at $ref"

have() { lab-agent --version 2>/dev/null | awk '{print $2}' || true; }

out="$(bash -euo pipefail -c "$snippet" < <(yes) 2>&1)" || { echo "$out" | tail -25 >&2; fail "the snippet failed"; }
[ "$(have)" = "$latest" ] || { echo "$out" | tail -15 >&2; fail "after the snippet expected $latest, got '$(have)'"; }
ok "the snippet installs $latest from the repository"

pacman -Si lab-agent | grep -q "^Repository *: lab-agent" || fail "lab-agent does not come from the lab-agent repository"
pacman -Syu --noconfirm >/dev/null 2>&1
[ "$(have)" = "$latest" ] || fail "a system update changed the version: '$(have)'"
ok "a system update (pacman -Syu) leaves it at $latest, from the lab-agent repository"
echo "PASS: the README's pacman repository snippet"
