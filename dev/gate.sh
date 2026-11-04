#!/bin/bash
# SPDX-FileCopyrightText: 2026 grenudi
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Generalised from snapall's dev/gate.sh: the step mechanism is the part worth keeping; the steps are your project's.
#
# Every check that has to pass before a change is pushed, in one command with a compact result.
#
#   gate.sh          the fast gate, a minute or two: the checks that need nothing special. Run it before every push.
#   gate.sh --full   adds what needs a fixture or root (golden output, the whole suite on the real thing). CI runs all of it on
#                    every pull request, so use this to reproduce a CI failure, not as a habit: push and let ship.py wait.
#
# Each step writes its output to /tmp/gate/NAME.log and prints one line; a failing step prints the end of its log. The exit code is
# 0 only if every step passed.
set -u
# the repository root, wherever this script is kept (dev/, kit/scripts/, ...): the gate runs from the root
here=$(git -C "$(dirname "$0")" rev-parse --show-toplevel 2>/dev/null) || { echo "gate.sh: not inside a git repository"; exit 2; }
cd "$here" || exit 2
mkdir -p /tmp/gate
failed=0

# Build output is the one thing that fills a disk, and it is only a cache: when little is free, drop what is yours and keep the
# dependencies, which are slow to rebuild. Replace this with your build tool's clean command.
free_gb=$(df -BG --output=avail "$here" | tail -n 1 | tr -dc '0-9')
if [ "${free_gb:-99}" -lt 4 ]; then
  echo "note: ${free_gb} GB free: cleaning this workspace's build output"
  # cargo clean -p yourcrate >/dev/null 2>&1      # or: rm -rf build/ dist/ .pytest_cache/ node_modules/.cache/
fi

step() {
  local name=$1; shift
  local started=$SECONDS
  if "$@" > "/tmp/gate/$name.log" 2>&1; then
    printf 'ok    %-26s %3ss\n' "$name" $((SECONDS - started))
  else
    printf 'FAIL  %-26s %3ss  (/tmp/gate/%s.log)\n' "$name" $((SECONDS - started)) "$name"
    tail -n 14 "/tmp/gate/$name.log" | cut -c1-160 | sed 's/^/        /'
    failed=1
  fi
}

# ---- the fast gate ------------------------------------------------------------------------------------------------------------
step syntax find . -name '*.py' -not -path './.git/*' -exec python3 -m py_compile {} +
step lint   python3 -m ruff check .
step tests  python3 dev/run_tests.py

# ---- the full gate: what needs a fixture, a service or root ------------------------------------------------------------------
if [ "${1:-}" = "--full" ]; then
  # step golden ./kit/scripts/golden-output.sh check ./path/to/program commands.txt
  :
fi

[ "$failed" -eq 0 ] && echo "gate: all passed" || echo "gate: FAILED"
exit "$failed"
