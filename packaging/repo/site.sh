#!/usr/bin/env bash
# The repository's front page and the files beside the package directory. The instructions on the
# page are the README's own repo-pacman snippet, cut out by its marker, so the two cannot differ.
#
# usage: site.sh SITEDIR README.md VERSION
set -euo pipefail
site="${1:?usage: $0 SITEDIR README.md VERSION}"; readme="${2:?}"; version="${3:?}"
here="$(cd "$(dirname "$0")" && pwd)"
fingerprint="$(gpg --show-keys --with-colons "$here/lab-agent-repo.asc" | awk -F: '/^fpr/{print $10; exit}')"

snippet() { awk -v id="$1" '
  $0 == "<!-- snippet:" id " -->" { want = 1; next }
  want && /^```/ { if (inblock) exit; inblock = 1; next }
  inblock { print }' "$readme"; }
escape() { sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'; }

mkdir -p "$site"
cp "$here/lab-agent-repo.asc" "$site/key.asc"
touch "$site/.nojekyll"
code="$(snippet repo-pacman)"
[ -n "$code" ] || { echo "the README has no snippet marked repo-pacman" >&2; exit 1; }
cat > "$site/index.html" <<HTML
<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>lab-agent packages</title>
<style>body{font:16px/1.5 system-ui,sans-serif;max-width:46rem;margin:2rem auto;padding:0 1rem}pre{background:#f4f4f4;padding:.8rem;overflow:auto;border-radius:6px}code{font-family:ui-monospace,monospace}@media(prefers-color-scheme:dark){body{background:#161616;color:#ddd}pre{background:#242424}a{color:#8cf}}</style>
</head><body>
<h1>lab-agent packages</h1>
<p>A pacman repository for <a href="https://github.com/creation-guidelines/lab-agent">lab-agent</a>, so your package manager updates it with the rest of your system. Latest release: <b>${version}</b>. Packages are the release's own files, unchanged; the repository's metadata is signed.</p>
<p>Signing key fingerprint, to compare with the one committed in the repo at <code>packaging/repo/FINGERPRINT.txt</code>:<br><code>${fingerprint}</code> (<a href="key.asc">key.asc</a>)</p>
<h2>Arch, CachyOS, Manjaro, ...</h2>
<pre>$(printf '%s\n' "$code" | escape)</pre>
</body></html>
HTML
echo "site written to $site (version $version, key $fingerprint)"
