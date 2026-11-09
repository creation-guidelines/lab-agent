#!/usr/bin/env bash
# Build everything a release ships for one target from an already-built static binary: the
# .deb, the .rpm, the tarball and the checksums.
#
#   cargo build --release --locked --target x86_64-unknown-linux-musl
#   packaging/build-packages.sh x86_64-unknown-linux-musl
#
# Needs: cargo-deb, cargo-generate-rpm, GNU tar. Output goes to dist/.
set -euo pipefail

target="${1:?usage: $0 <rust target triple, e.g. x86_64-unknown-linux-musl>}"
cd "$(dirname "$0")/.."

binary="target/$target/release/lab-agent"
if [ ! -x "$binary" ]; then
  echo "missing $binary; build it first:" >&2
  echo "  cargo build --release --locked --target $target" >&2
  exit 1
fi
version="$("$binary" --version 2>/dev/null | awk '{print $2}')" || version=""
[ -n "$version" ] || { echo "cannot run $binary" >&2; exit 1; }

SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || date +%s)}"
export SOURCE_DATE_EPOCH

rm -rf dist
mkdir -p dist

cargo deb --no-build --package lab-agent --target "$target" --output dist/
cargo generate-rpm --package . --target "$target" --output dist/

name="lab-agent-v$version-$target"
stage="$(mktemp -d)"
mkdir -p "$stage/$name"
cp "$binary" "$stage/$name/lab-agent"
cp LICENSE README.md "$stage/$name/"
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$SOURCE_DATE_EPOCH" \
  -C "$stage" -czf "dist/$name.tar.gz" "$name"
rm -rf "$stage"

( cd dist && sha256sum ./* > SHA256SUMS )
echo "dist/ ready:"
ls -la dist/
