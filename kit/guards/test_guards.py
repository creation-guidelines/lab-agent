#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 grenudi
#
# SPDX-License-Identifier: GPL-3.0-or-later

"""Tests for the ratchet, the outline and the findability ranking. Plain asserts, no framework: ``python3 kit/guards/test_guards.py``."""
import json
import os
import sys
import tempfile
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import findability  # noqa: E402
import outline  # noqa: E402
import ratchet  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(HERE))


def tmp():
    return tempfile.mkdtemp()


def debts(path, data):
    with open(path, "w", encoding="utf-8") as f:
        json.dump(data, f)


def fails(fn, *needles):
    try:
        fn()
    except ratchet.RatchetError as error:
        for needle in needles:
            assert needle in str(error), f"{needle!r} is not in the failure: {error}"
        return str(error)
    raise AssertionError("it should have failed and passed")


# ---- the ratchet -------------------------------------------------------------------------------------------------------------------

def test_a_file_within_the_limit_passes_and_a_missing_record_is_empty():
    ratchet.check("rule", "long", {"a": 10, "b": 300}, 300, path=os.path.join(tmp(), "none.json"), update=False)


def test_a_new_offender_fails():
    fails(lambda: ratchet.check("Files too long:", "long", {"a": 301}, 300, path=os.path.join(tmp(), "r.json"), update=False),
          "Files too long:", "`a` is 301 (the most allowed is 300)")


def test_a_recorded_debt_passes_and_growth_fails():
    path = os.path.join(tmp(), "r.json")
    debts(path, {"long": {"a": 400}})
    ratchet.check("rule", "long", {"a": 400}, 300, path=path, update=False)
    fails(lambda: ratchet.check("rule", "long", {"a": 401}, 300, path=path, update=False), "`a` is 401 (the most allowed is 400)")


def test_an_improvement_must_be_written_down():
    path = os.path.join(tmp(), "r.json")
    debts(path, {"long": {"a": 400}})
    fails(lambda: ratchet.check("rule", "long", {"a": 350}, 300, path=path, update=False), "`a` shrank to 350: lower its entry from 400")


def test_updating_lowers_a_debt_and_the_lowered_record_then_passes_without_updating():
    path = os.path.join(tmp(), "r.json")
    debts(path, {"long": {"a": 400, "b": 500}, "other": {"x": 1}})
    ratchet.check("rule", "long", {"a": 350, "b": 500}, 300, path=path, update=True)
    assert json.load(open(path)) == {"long": {"a": 350, "b": 500}, "other": {"x": 1}}, "only the named list changes"
    ratchet.check("rule", "long", {"a": 350, "b": 500}, 300, path=path, update=False)


def test_updating_never_raises_a_debt_and_never_adds_one():
    path = os.path.join(tmp(), "r.json")
    debts(path, {"long": {"a": 400}})
    fails(lambda: ratchet.check("rule", "long", {"a": 450}, 300, path=path, update=True), "`a` is 450")
    fails(lambda: ratchet.check("rule", "long", {"a": 400, "new": 999}, 300, path=path, update=True), "`new` is 999")
    assert json.load(open(path)) == {"long": {"a": 400}}, "a failing update writes nothing"


def test_a_vanished_file_and_a_file_back_within_the_limit_are_taken_off_the_list():
    path = os.path.join(tmp(), "r.json")
    debts(path, {"long": {"gone": 400, "fixed": 400, "still": 400}})
    fails(lambda: ratchet.check("rule", "long", {"fixed": 100, "still": 400}, 300, path=path, update=False),
          "`gone` is on the debt list but no longer exists", "`fixed` is 100, within the limit now")
    ratchet.check("rule", "long", {"fixed": 100, "still": 400}, 300, path=path, update=True)
    assert json.load(open(path)) == {"long": {"still": 400}}


def test_the_failure_says_how_to_record_paid_debts():
    message = fails(lambda: ratchet.check("r", "long", {"a": 999}, 300, path=os.path.join(tmp(), "x.json"), update=False))
    assert "UPDATE_RATCHETS=1" in message and "over the limit of 300 now" in message


# ---- the outline ---------------------------------------------------------------------------------------------------------------------

def test_the_first_sentence_is_the_purpose():
    assert outline.first_sentence("Does a thing. And then more.") == "Does a thing"
    assert outline.first_sentence("  Spread\n over   lines.\n\nSecond.") == "Spread over lines"
    assert outline.first_sentence("No full stop") == "No full stop"
    assert outline.first_sentence("") == "" and outline.first_sentence(None) == ""


def test_each_language_gives_its_purpose():
    assert outline.purpose("a.py", '# licence\n"""Reads the thing. More.\n\nDetail."""\nimport os\n') == "Reads the thing"
    assert outline.purpose("a.py", "import os\n") == ""
    assert outline.purpose("a.rs", "// licence\n\n#![allow(x)]\n//! Puts a restore point back. The rest.\n//!\n//! More.\n\nuse std;\n") == "Puts a restore point back"
    assert outline.purpose("a.rs", "use std;\n//! too late\n") == ""
    assert outline.purpose("a.ts", "/* licence */\n/**\n * Draws the header.\n * More.\n */\nexport const x = 1;\n") == "Draws the header"
    assert outline.purpose("a.go", "// Copyright\n\n// Package store keeps the records. More.\npackage store\n") == "keeps the records"
    assert outline.purpose("a.sh", "#!/bin/bash\n# SPDX-License-Identifier: X\n#\n# Everything that has to pass before shipping. More.\n#\nset -u\n") == "Everything that has to pass before shipping"
    assert outline.purpose("a.sh", "#!/bin/bash\nset -u\n") == ""


def test_the_outline_lists_every_module_and_counts_those_without_a_purpose():
    root = tmp()
    os.makedirs(os.path.join(root, "pkg", "inner"))
    open(os.path.join(root, "pkg", "a.py"), "w").write('"""Does a."""\n')
    open(os.path.join(root, "pkg", "inner", "b.py"), "w").write("x = 1\n")
    open(os.path.join(root, "pkg", "__init__.py"), "w").write('"""never listed"""\n')
    open(os.path.join(root, "pkg", "notes.txt"), "w").write("not source")
    modules = outline.build({"core": os.path.join(root, "pkg")})
    assert modules == [("core/a.py", "Does a"), ("core/inner/b.py", "")], modules
    assert outline.parse(outline.render(modules)) == modules, "render and parse are inverse"


def test_a_hidden_directory_is_skipped_but_a_root_given_as_dots_is_not():
    root = tmp()
    os.makedirs(os.path.join(root, ".git"))
    os.makedirs(os.path.join(root, "node_modules"))
    open(os.path.join(root, ".git", "h.py"), "w").write('"""hidden."""\n')
    open(os.path.join(root, "node_modules", "n.py"), "w").write('"""vendored."""\n')
    open(os.path.join(root, "a.py"), "w").write('"""kept."""\n')
    assert outline.build({"x": root}) == [("x/a.py", "kept")]
    cwd = os.getcwd()
    os.chdir(root)
    try:
        # regression: a root of "." or "../x" has dot components that are not hidden directories
        assert outline.build({"x": "."}) == [("x/a.py", "kept")]
        os.makedirs("sub")
        os.chdir("sub")
        assert outline.build({"x": ".."}) == [("x/a.py", "kept")]
    finally:
        os.chdir(cwd)


# ---- findability -----------------------------------------------------------------------------------------------------------------------

def test_words_are_stemmed_so_that_the_forms_of_a_word_meet():
    s = findability.stem
    assert s("delete") == s("deleting") == s("deleted") == "delet"
    assert s("take") == s("taking") == s("taken") == s("took")
    assert s("keep") == s("kept") and s("write") == s("written")
    assert findability.words("Where is the restore point put back?") == ["restor", "point", "put", "back"]


OUTLINE = """core/going_back/putting_back.py: Puts a restore point back, area by area, and says what each area's outcome means
core/taking/snapshots.py: Takes a snapshot of every protected area in one action
core/keeping/limits.py: How many snapshots an area keeps, and what the next cleanup would remove
window/tabs/health.py: The health tab: whether protection is working and how much room it takes
binary/render/points.py: Lists the restore points
"""


def question(text, *answers):
    return {"question": text, "answers": list(answers)}


def test_a_question_finds_the_module_that_answers_it():
    ok = findability.run(OUTLINE, {"top": 1, "questions": [
        question("where is a restore point put back?", "core/going_back/putting_back.py"),
        question("what does the next cleanup remove?", "core/keeping/limits.py"),
        question("how much room does protection take?", "window/tabs/health.py")]})
    assert ok == "findability: 3 of 3 questions put the answer first; 3 within the first 1", ok


def test_an_answer_that_is_not_in_the_outline_is_an_error_in_the_questions():
    fails(lambda: findability.run(OUTLINE, {"top": 3, "questions": [question("anything", "core/nope.py")]}), "`core/nope.py`", "is not in the outline")


def test_a_question_whose_answer_is_not_near_the_top_fails_and_says_which():
    message = fails(lambda: findability.run(OUTLINE, {"top": 1, "questions": [question("how do I take a snapshot?", "binary/render/points.py")]}),
                    "not answered within the first 1", "how do I take a snapshot?")
    assert "0 within" in message or "within the first 1" in message


def test_with_a_ratchet_the_gaps_are_a_debt_that_may_only_shrink():
    path = os.path.join(tmp(), "r.json")
    questions = {"top": 1, "questions": [question("how do I take a snapshot?", "binary/render/points.py")]}
    fails(lambda: findability.run(OUTLINE, questions, ratchet_path=path), "how do I take a snapshot?")
    os.environ["UPDATE_RATCHETS"] = "1"
    try:
        fails(lambda: findability.run(OUTLINE, questions, ratchet_path=path))  # an update never adds a new debt
    finally:
        del os.environ["UPDATE_RATCHETS"]
    debts(path, {"findability_gaps": {"how do I take a snapshot?": 4}})
    findability.run(OUTLINE, questions, ratchet_path=path)  # now it is a recorded debt: passes, and must not grow


def test_an_outline_that_is_too_small_has_stopped_being_read():
    fails(lambda: findability.run(OUTLINE, {"top": 3, "questions": []}, min_modules=100), "has stopped being read")


def test_the_repository_answers_questions_about_its_own_code():
    """The dogfood: the outline of this repository's real package (lab_agent/, not the copied kit itself), and real questions about it."""
    modules = outline.build({"lab_agent": os.path.join(ROOT, "lab_agent")})
    assert all(purpose for _, purpose in modules), [name for name, p in modules if not p]
    questions = json.load(open(os.path.join(HERE, "self-questions.json"), encoding="utf-8"))
    print("    " + findability.run(outline.render(modules), questions, min_modules=5))


TESTS = [v for k, v in sorted(globals().items()) if k.startswith("test_") and callable(v)]

if __name__ == "__main__":
    failed = 0
    for t in TESTS:
        try:
            t()
            print("ok  ", t.__name__)
        except Exception:
            failed += 1
            print("FAIL", t.__name__)
            traceback.print_exc()
    print(f"{len(TESTS) - failed}/{len(TESTS)} passed")
    sys.exit(1 if failed else 0)
