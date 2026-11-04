#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 grenudi
#
# SPDX-License-Identifier: GPL-3.0-or-later

"""The architecture guards of a project, wired up: this repository's own, and the one to copy.

    python3 kit/guards/architecture.py            run every guard (exit 1 and the offenders on a failure)
    UPDATE_RATCHETS=1 python3 kit/guards/architecture.py     record debts that have been paid

To use it in another project, copy this file next to ``outline.py``, ``ratchet.py`` and ``rules.py``, change the CONFIG block, and add the rules
you want from ``rules.py``; start with the ones the code already satisfies, and give the ones it cannot yet satisfy a recorded debt (write the first
debts into ``tests/ratchets.json`` by hand, then let ``UPDATE_RATCHETS=1`` only ever lower them). Each guard is one ``check`` call: a rule, the name
of its debts in the file, what the code has now, and the limit.
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import findability  # noqa: E402
import outline  # noqa: E402
import rules  # noqa: E402
from ratchet import RatchetError, check  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(HERE))

# ---- CONFIG: change this for your project -------------------------------------------------------------------------------------------------
LAYERS = {"lab_agent": "lab_agent"}   # name -> folder, as the outline lists them: this project's real code, not the copied kit itself
RATCHETS = "tests/ratchets.json"                                                  # the debts, as data that may only shrink
QUESTIONS = "kit/guards/self-questions.json"                                              # real questions a newcomer would ask, and the module that answers each
MAX_FILE_LINES = 300
MAX_ENTRY_FILE_LINES = 120
PLURAL_RULE_FILES = r"^kit/guards/(rules|architecture)\.py$"   # the files that define the plural rule quote what it forbids
RETIRED_WORDS = {}                              # word -> a regular expression for the paths that may still use it (their adapter); e.g. {"config": r"^adapter/"}
ONLY_IN = []                                    # (allowed-path regex, forbidden pattern, what it is): purity by role; e.g. (r"/drawing", r"\bcurses\b", "the drawing library")
# --------------------------------------------------------------------------------------------------------------------------------------------


def run():
    os.chdir(ROOT)
    files = rules.source_files(LAYERS.values())
    modules = outline.build(LAYERS)
    ratchet = os.path.join(ROOT, RATCHETS)
    check(f"Files longer than {MAX_FILE_LINES} lines (a file is short enough to read whole; split it by purpose, or record the debt):",
          "files_too_long", rules.files_too_long(files), MAX_FILE_LINES, path=ratchet)
    check(f"Entry files longer than {MAX_ENTRY_FILE_LINES} lines (an entry file only says what is in the folder):",
          "entry_files_too_long", rules.entry_files_too_long(files), MAX_ENTRY_FILE_LINES, path=ratchet)
    check("Modules with no first sentence saying what they are for (write one: the outline and the findability test are built from it):",
          "purposeless_modules", rules.purposeless_modules(modules), 0, path=ratchet)
    check("Modules that share a purpose sentence (two modules cannot be for the same thing; say what each is for):",
          "shared_purposes", rules.shared_purposes(modules), 1, path=ratchet)
    check("Modules whose first sentence uses a pattern word (say what it is for, not which pattern it is):",
          "jargon_purposes", rules.jargon_purposes(modules), 0, path=ratchet)
    check("Plurals spelled by hand (`if n == 1` in any words, or `thing(s)`): say it with one helper:",
          "hand_spelled_plurals", rules.hand_spelled_plurals(files, skip=PLURAL_RULE_FILES), 0, path=ratchet)
    for word, allowed in RETIRED_WORDS.items():
        check(f"Identifiers that still use the retired word `{word}` outside the code that owns it:",
              f"retired_{word}", rules.retired_word_uses(files, word, allowed), 0, path=ratchet)
    for i, (allowed, forbidden, what) in enumerate(ONLY_IN):
        check(f"Files that use {what} but are not allowed to:", f"only_in_{i}", rules.uses_outside(files, allowed, forbidden), 0, path=ratchet)
    questions = os.path.join(ROOT, QUESTIONS)
    asked = ""
    if os.path.exists(questions):
        import json
        spec = json.load(open(questions, encoding="utf-8"))
        if spec["questions"]:
            asked = "; " + findability.run(outline.render(modules), spec, ratchet_path=ratchet)
            if len(modules) < 10:
                asked += f" (only {len(modules)} modules: with so few, any answer is near the top, so this cannot fail yet; it starts to mean something around 10 to 15)"
    return f"architecture: {len(files)} files, {len(modules)} modules: every guard holds{asked}"


if __name__ == "__main__":
    try:
        print(run())
    except RatchetError as error:
        print(error)
        sys.exit(1)
