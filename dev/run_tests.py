#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Run every test_*.py in this repository (tests/, kit/guards/), each as its own process, and summarise. One command for the gate
and for CI to depend on, instead of listing every test file by hand in two places.

    python3 dev/run_tests.py
"""
import glob
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def main():
    files = sorted(glob.glob(os.path.join(ROOT, "tests", "test_*.py"))) + \
        sorted(glob.glob(os.path.join(ROOT, "kit", "guards", "test_*.py")))
    failed = []
    for path in files:
        rel = os.path.relpath(path, ROOT)
        result = subprocess.run([sys.executable, path], cwd=ROOT, capture_output=True, text=True, timeout=120)
        tail = result.stdout.strip().split("\n")[-1] if result.stdout.strip() else "(no output)"
        print(f"{'ok  ' if result.returncode == 0 else 'FAIL'}  {rel:38} {tail}")
        if result.returncode != 0:
            failed.append(rel)
            print(result.stdout[-1500:])
            print(result.stderr[-1500:])
    print(f"\n{len(files) - len(failed)}/{len(files)} test files passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
