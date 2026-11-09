#!/usr/bin/env bash
# Build the static release binary so that the same source gives the same bytes wherever it is
# built.
#
#   packaging/build-binary.sh x86_64-unknown-linux-musl
#
# Build and release workflows both go through this script, so the binary that is tested is the
# binary that ships.
set -euo pipefail

target="${1:?usage: $0 <rust target triple, e.g. x86_64-unknown-linux-musl>}"
cd "$(dirname "$0")/.."

root="$PWD"
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
sysroot="$(rustc --print sysroot)"
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$root=/build --remap-path-prefix=$cargo_home=/cargo --remap-path-prefix=$sysroot=/rust"
export CARGO_INCREMENTAL=0

cargo build --release --locked --target "$target"
echo "built target/$target/release/lab-agent with $(rustc -V)"
