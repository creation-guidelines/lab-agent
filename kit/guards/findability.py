# SPDX-FileCopyrightText: 2026 grenudi
#
# SPDX-License-Identifier: GPL-3.0-or-later

"""The cold read, as a test: does the outline alone lead a stranger to the module that answers each question?

    findability.py OUTLINE QUESTIONS [--report] [--ratchet FILE]

The goal of a structure is that a person can learn what a program does, and where, by reading names and one sentence for each module.
Size limits only measure proxies for that. This asks the real question. Each question in the QUESTIONS file is written the way someone
new to the code would ask it. Every module is scored by how well its path and its purpose sentence match the question (a small
deterministic ranking: no model, no network), and the module that really answers it must be among the first few. A question that
fails shows a name or a sentence that does not say what its module is for.

QUESTIONS is JSON: ``{"top": 3, "questions": [{"question": "where is X told?", "answers": ["layer/path/to/module.py"]}]}``. To add a
question, add it with the module (or modules) that answer it: it is a regression test from then on. With --ratchet FILE the gaps are
a debt that may only shrink (see ratchet.py); without it, any answer outside the first ``top`` fails.

Ported from snapall's ``tests/findability.rs``. In snapall: 52 questions, every answer within the first three, about 37 first.
"""
import json
import math
import re
import sys

from outline import parse
from ratchet import RatchetError, check

STOP = set("""a an the of to in is it and or for on with what where how does do when which are be by as at from that this can i we you one its
if then not no who get gets there their them so than into about after before per while code program""".split())
EXTENSION_WORDS = {"py", "rs", "ts", "tsx", "js", "jsx", "go", "sh", "bash", "mod", "md", "rst", "init"}
IRREGULAR = {"taken": "take", "took": "take", "kept": "keep", "made": "make", "gone": "go", "ran": "run", "written": "write",
             "shown": "show", "drawn": "draw", "chosen": "choose", "held": "hold", "done": "do"}


def stem(word):
    word = IRREGULAR.get(word, word)
    out = word
    for suffix in ("ing", "ed", "es", "er", "s", "ly"):
        if len(word) > len(suffix) + 2 and word.endswith(suffix):
            out = word[: -len(suffix)]
            break
    # `delete` and `deleting` meet at `delet`, `take` and `taking` at `tak`
    if len(out) > 3 and out.endswith("e"):
        out = out[:-1]
    return out


def words(text):
    return [stem(w) for w in re.split(r"[^0-9A-Za-z]+", text.lower()) if w and w not in STOP and w not in EXTENSION_WORDS]


def prepare(modules):
    """(name, words of the path, words of the purpose) for every module, and how many modules each word names."""
    prepared = [(name, set(words(re.sub(r"[/_.\-]", " ", name))), set(words(purpose))) for name, purpose in modules]
    spread = {}
    for _, path, purpose in prepared:
        for word in path | purpose:
            spread[word] = spread.get(word, 0) + 1
    return prepared, spread


def rank(prepared, spread, question, answers):
    """The 1-based position of the best-ranked answer, and the three best names. ``None`` if no answer is found."""
    n = len(prepared)
    idf = lambda word: math.log((n + 1) / (spread.get(word, 0) + 1)) + 1.0
    wanted = set(words(question))
    scored = []
    for name, path, purpose in prepared:
        score = sum(2.0 * idf(w) if w in path else idf(w) if w in purpose else 0.0 for w in wanted)
        scored.append((-score, name))
    scored.sort()
    names = [name for _, name in scored]
    position = next((i + 1 for i, name in enumerate(names) if name in answers), None)
    return position, names[:3]


def run(outline_text, questions, ratchet_path=None, report=False, min_modules=1):
    modules = parse(outline_text)
    if len(modules) < min_modules:
        raise RatchetError(f"the outline has {len(modules)} modules: it has stopped being read")
    prepared, spread = prepare(modules)
    known = {name for name, _ in modules}
    top = questions["top"]
    gaps, ranks, lines = {}, [], []
    for item in questions["questions"]:
        question, answers = item["question"], item["answers"]
        for answer in answers:
            if answer not in known:
                raise RatchetError(f"`{answer}` (the answer to \"{question}\") is not in the outline")
        position, best = rank(prepared, spread, question, answers)
        ranks.append(position)
        gaps[question] = min(position if position is not None else 9999, 9999)
        lines.append(f"{'-' if position is None else position:>3}  {question}\n       found: {', '.join(best)}\n       wanted: {' | '.join(answers)}")
    first = sum(1 for r in ranks if r == 1)
    within = sum(1 for r in ranks if r is not None and r <= top)
    summary = f"findability: {first} of {len(ranks)} questions put the answer first; {within} within the first {top}"
    if report:
        print("\n".join(lines))
    if ratchet_path:
        check("Questions the outline does not answer within the first few modules (name the module for what it is for, in the words a stranger would use):",
              "findability_gaps", gaps, top, path=ratchet_path)
    else:
        failing = [q for q, g in gaps.items() if g > top]
        if failing:
            raise RatchetError(summary + "\nnot answered within the first " + str(top) + ":\n  " + "\n  ".join(failing))
    return summary


def main(argv):
    positional = [a for i, a in enumerate(argv) if not a.startswith("--") and (i == 0 or argv[i - 1] != "--ratchet")]
    if len(positional) < 2:
        sys.exit(__doc__)
    ratchet = argv[argv.index("--ratchet") + 1] if "--ratchet" in argv else None
    outline_text = open(positional[0], encoding="utf-8").read()
    questions = json.load(open(positional[1], encoding="utf-8"))
    try:
        print(run(outline_text, questions, ratchet, "--report" in argv))
    except RatchetError as error:
        print(error)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
