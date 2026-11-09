#!/usr/bin/env bash
# Check an assembled repository site before it is published: every signature on it verifies
# against the committed public key (and that key is the expected one), and what should be there
# is there. A site that fails this is never deployed.
#
# usage: check-site.sh SITEDIR EXPECTED-FINGERPRINT
set -euo pipefail
site="${1:?usage: $0 SITEDIR FINGERPRINT}"; expected="${2:?usage: $0 SITEDIR FINGERPRINT}"
here="$(cd "$(dirname "$0")" && pwd)"
export GNUPGHOME; GNUPGHOME="$(mktemp -d)"; chmod 700 "$GNUPGHOME"; trap 'rm -rf "$GNUPGHOME"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "  ok  $*"; }

gpg --batch --quiet --import "$here/lab-agent-repo.asc" 2>/dev/null
have="$(gpg --batch --list-keys --with-colons | awk -F: '/^fpr/{print $10; exit}')"
[ "$have" = "$expected" ] || fail "the committed public key is $have, expected $expected"
ok "the committed public key is the expected one"
cmp -s "$site/key.asc" "$here/lab-agent-repo.asc" || fail "the key the site serves is not the committed one"
ok "the site serves that key"

verify() { # verify SIGNATURE [FILE]
  local out; out="$(gpg --batch --verify "$@" 2>&1)" || fail "bad signature on $1: $out"
  echo "$out" | grep -q "${expected: -16}" || fail "$1 is signed by some other key: $out"
}
for repo in "$site"/arch/*/; do
  [ -d "$repo" ] || continue
  verify "$repo/lab-agent.db.sig" "$repo/lab-agent.db"
  for package in "$repo"/*.pkg.tar.zst; do verify "$package.sig" "$package"; done
  ok "pacman $(basename "$repo"): the database and every package verify"
done
echo "PASS: the site's signatures"
