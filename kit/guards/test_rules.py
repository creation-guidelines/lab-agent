#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 grenudi
#
# SPDX-License-Identifier: GPL-3.0-or-later

"""Tests for the catalogue of guards in rules.py. Plain asserts, no framework: ``python3 kit/guards/test_rules.py``."""
import os
import sys
import tempfile
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import rules  # noqa: E402


def test_source_files_leave_out_tests_and_inline_test_modules_and_hidden_folders():
    root = tempfile.mkdtemp()
    for path, text in {"a.py": "x = 1\n", "tests/b.py": "x\n", "test_c.py": "x\n", "d.rs": "fn f() {}\n#[cfg(test)]\nmod t {}\n", ".hidden/e.py": "x\n",
                       "f.rs": "#![cfg(test)]\nfn t() {}\n", "g.txt": "x\n", "node_modules/h.js": "x\n"}.items():
        os.makedirs(os.path.dirname(os.path.join(root, path)), exist_ok=True)
        open(os.path.join(root, path), "w").write(text)
    found = rules.source_files([root])
    assert sorted(os.path.relpath(p, root) for p in found) == ["a.py", "d.rs"], sorted(found)
    assert found[os.path.join(root, "d.rs")] == "fn f() {}", "the inline test module is cut off"


def test_file_and_entry_file_sizes():
    files = {"src/a.py": "x\n" * 10, "src/__init__.py": "x\n" * 5, "src/mod.rs": "x\n" * 7}
    assert rules.files_too_long(files)["src/a.py"] == 11
    assert rules.entry_files_too_long(files) == {"src/__init__.py": 6, "src/mod.rs": 8}


def test_two_modules_with_one_purpose_are_counted_and_index_files_are_not():
    modules = [("a/x.py", "Reads the thing"), ("a/y.py", "Reads the thing"), ("a/z.py", "Writes it"), ("a/__init__.py", "Reads the thing"), ("a/w.py", "")]
    assert rules.shared_purposes(modules) == {"Reads the thing": 2, "Writes it": 1}


def test_a_module_with_no_purpose_sentence_is_found_and_index_files_are_not():
    modules = [("a/x.py", "Reads the thing"), ("a/y.py", ""), ("a/__init__.py", ""), ("a/mod.rs", "")]
    assert rules.purposeless_modules(modules) == {"a/y.py": 1}


def test_a_pattern_word_in_a_purpose_is_jargon_but_a_plain_word_is_not():
    modules = [("a.py", "An adapter for the disk"), ("b.py", "Asks the shell what is running"), ("c.py", "Takes a snapshot of every area"), ("d.py", "A transport layer"),
               ("e.py", "The port the server listens on")]
    assert sorted(rules.jargon_purposes(modules)) == ["a.py", "b.py", "e.py"], "a word inside another word ('transport') is not the word"


def test_what_only_some_files_may_use_is_counted_everywhere_else():
    files = {"ui/drawing.py": "import curses\ncurses.x()\n", "core/logic.py": "import curses\nx = curses.y\n", "core/other.py": "x = 1\n"}
    assert rules.uses_outside(files, r"/drawing\.", r"\bcurses\b") == {"core/logic.py": 2}


def test_a_plural_spelled_by_hand_is_found_in_any_language_and_ok_s_is_not_one():
    assert rules.hand_spelled_plurals({"a.rs": 'let s = if n == 1 { "" } else { "s" };'}) == {"a.rs": 1}
    assert rules.hand_spelled_plurals({"a.rs": 'let s = if n == 1 {\n    " was"\n} else {\n    "s were"\n};'}) == {"a.rs": 1}, "in any words, across lines"
    assert rules.hand_spelled_plurals({"a.ts": 'const s = n === 1 ? "" : "s";'}) == {"a.ts": 1}
    assert rules.hand_spelled_plurals({"a.py": 's = "" if n == 1 else "s"'}) == {"a.py": 1}
    assert rules.hand_spelled_plurals({"a.rs": 'format!("Delete {} snapshot(s) of home?", n)'}) == {"a.rs": 1}, "a plural left to the reader"
    assert rules.hand_spelled_plurals({"a.rs": "match x { Ok(s) => s, Err(e) => e }\nlet y = parse(s);"}) == {}, "(s) in code is not a plural"
    assert rules.hand_spelled_plurals({"a.rs": 'if n == 1 { do_it() } else { other() }'}) == {}, "an ordinary condition is not a plural"


def test_files_that_only_quote_the_idiom_can_be_exempted_by_name():
    files = {"rules.py": 'DOC = "Delete {} snapshot(s)"', "app.py": 'DOC = "Delete {} snapshot(s)"'}
    assert rules.hand_spelled_plurals(files) == {"rules.py": 1, "app.py": 1}
    assert rules.hand_spelled_plurals(files, skip=r"^rules\.py$") == {"app.py": 1}


def test_the_lexer_tells_a_word_in_code_from_a_word_in_a_string_or_a_comment():
    code = 'let config = "config";  // config\nfoo_config(x) /* config */ ConfigName\n# config\n"""config"""\nname = \'it\\\'s a config\''
    assert list(rules.identifiers(code)) == ["let", "config", "foo_config", "x", "ConfigName", "name"], list(rules.identifiers(code))


def test_a_retired_word_is_found_as_a_part_of_an_identifier_only_in_code():
    files = {"core/a.py": 'config = 1\narea_config = 2\nConfigName = 3\nconfigs = []\nx = "config"  # config\nreconfigure = 1\n', "adapter/b.py": "config = 1\n"}
    assert rules.retired_word_uses(files, "config", allowed=r"^adapter/") == {"core/a.py": 4}, "reconfigure is a different word"


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
