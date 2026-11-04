# SPDX-FileCopyrightText: 2026 grenudi
#
# SPDX-License-Identifier: GPL-3.0-or-later

"""A ratchet: debts that are recorded as data and may only shrink.

A rule that cannot be met today is not a reason to have no rule. The ratchet records, per offender, how bad it is now; a test then
fails when any offender gets worse, when a new offender appears, and when an offender has improved and the record was not updated, so
that progress cannot slip back and what remains is a list anyone can read.

    from ratchet import check
    check("Files over 300 lines:", "files_too_long", {"src/a.py": 412, "src/b.py": 90}, limit=300)

Ported from snapall's ``tests/common/mod.rs``. The debts live in a JSON file (``tests/ratchets.json`` by default), one object per list
name. Run with ``UPDATE_RATCHETS=1`` and every debt that has shrunk, or gone, is lowered or removed in the file; nothing is ever raised
and nothing new is added, so growth and new offenders still fail.
"""
import json
import os


class RatchetError(AssertionError):
    """The code is further from a rule than the record says, or the record is behind the code."""


def read_debts(path):
    if not os.path.exists(path):
        return {}
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def check(rule, list_name, observed, limit, path="tests/ratchets.json", update=None):
    """``observed``: name -> size, what the code has now. ``limit``: what is allowed without a reason. ``list_name`` names the
    debts in the file: the offenders over the limit today, with their size. ``update`` overrides the UPDATE_RATCHETS variable."""
    paying = bool(os.environ.get("UPDATE_RATCHETS")) if update is None else update
    all_debts = read_debts(path)
    debt = dict(all_debts.get(list_name, {}))
    problems = []
    for name, size in observed.items():
        most = debt.get(name, limit)
        if size > most:
            problems.append(f"`{name}` is {size} (the most allowed is {most})")
    remaining = {}
    for listed, most in debt.items():
        size = observed.get(listed)
        if size is None:
            if not paying:
                problems.append(f"`{listed}` is on the debt list but no longer exists: take it off")
        elif size <= limit:
            if not paying:
                problems.append(f"`{listed}` is {size}, within the limit now: take it off the list, so it cannot grow back")
        elif size < most:
            if paying:
                remaining[listed] = size
            else:
                problems.append(f"`{listed}` shrank to {size}: lower its entry from {most}, so it cannot grow back")
        else:
            remaining[listed] = size
    if paying and not problems and remaining != debt:
        all_debts[list_name] = remaining
        os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            json.dump(all_debts, f, indent=2, sort_keys=True)
            f.write("\n")
    if problems:
        over = [f'"{name}": {size},' for name, size in sorted(observed.items()) if size > limit]
        raise RatchetError(
            f"{rule}\n  " + "\n  ".join(problems)
            + f"\nover the limit of {limit} now:\n  " + "\n  ".join(over)
            + "\nto record paid debts: UPDATE_RATCHETS=1 and run the test again")
