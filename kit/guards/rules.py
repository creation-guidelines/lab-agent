# SPDX-FileCopyrightText: 2026 grenudi
#
# SPDX-License-Identifier: GPL-3.0-or-later

"""The catalogue of guards, as functions: each reads source files and returns ``{name: number}``, which is exactly what ``ratchet.check`` takes.

    files_too_long(files)                      lines per file                                  (a file is short enough to read whole)
    entry_files_too_long(files)                lines per index file                            (an entry file only says what is in the folder)
    purposeless_modules(modules)               modules whose first sentence says nothing        (a stranger must be able to say what each is for)
    shared_purposes(modules)                   how many modules share a purpose sentence      (two modules cannot be for the same thing)
    jargon_purposes(modules)                   modules whose purpose uses a pattern word       (say what it is for, not what pattern it is)
    uses_outside(files, allowed, forbidden)    matches of a forbidden pattern outside the files allowed to have it   (purity by role)
    hand_spelled_plurals(files, skip=None)     plurals spelled by hand                         (a helper says it, once)
    retired_word_uses(files, word, allowed)    identifiers that still contain a retired word  (checked on tokens, not text)

``files`` is ``{path: text}`` (``source_files`` reads it from folders; test files and inline test modules are left out), ``modules`` is the
outline's ``[(name, purpose)]``. A function that returns an empty dict has nothing to report; give the result to ``ratchet.check`` with a limit
(300 lines, 1 shared purpose, 0 for the rest) and the debts that cannot be paid yet become data that may only shrink. Ported from the tests in snapall's
``architecture.rs``; the lexer is a small one for C-like and Python sources, which is enough to tell a word in code from a word in a string.
"""
import os
import re

from outline import EXTENSIONS, SKIP_FILES

ENTRY_FILES = {"mod.rs", "lib.rs", "main.rs", "__init__.py", "index.ts", "index.js", "index.tsx"}
TESTS = ("/tests/", "/test/", "/__tests__/", "test_", "_test.", ".test.", ".spec.", "tests.rs")
STOP_AT = "#[cfg(test)]"
JARGON = re.compile(r"(adapter|imperative|\bshell\b|hexagon|composition root|functional core|driving|\bport\b)", re.I)


def source_files(roots, extensions=EXTENSIONS, tests=TESTS, stop_at=STOP_AT):
    """``{path: production text}`` for every source file under ``roots``."""
    found = {}
    for root in roots:
        for directory, _, names in os.walk(root):
            inside = os.path.relpath(directory, root).split(os.sep)
            if any((p.startswith(".") and p != ".") or p in ("node_modules", "target", "__pycache__") for p in inside):
                continue
            for name in names:
                path = os.path.join(directory, name)
                if name.rsplit(".", 1)[-1] not in extensions or any(t in "/" + path for t in tests):
                    continue
                try:
                    text = open(path, encoding="utf-8").read()
                except (UnicodeDecodeError, OSError):
                    continue
                if stop_at and stop_at.replace("#[", "#![") in text:
                    continue
                found[path] = text.split("\n" + stop_at)[0] if stop_at else text
    return found


def files_too_long(files):
    return {path: len(text.split("\n")) for path, text in files.items()}


def entry_files_too_long(files):
    return {path: len(text.split("\n")) for path, text in files.items() if os.path.basename(path) in ENTRY_FILES}


def purposeless_modules(modules):
    """Modules with no documentation sentence at all: the first thing to fix, because the outline and the findability test are built from it."""
    return {name: 1 for name, purpose in modules if not purpose and os.path.basename(name) not in SKIP_FILES}


def shared_purposes(modules):
    count = {}
    for name, purpose in modules:
        if purpose and os.path.basename(name) not in SKIP_FILES:
            count[purpose] = count.get(purpose, 0) + 1
    return count


def jargon_purposes(modules):
    return {name: 1 for name, purpose in modules if purpose and JARGON.search(purpose)}


def uses_outside(files, allowed, forbidden):
    """``allowed`` and ``forbidden`` are regular expressions: the files whose path matches ``allowed`` may use what ``forbidden`` matches; elsewhere it is counted."""
    allowed, forbidden = re.compile(allowed), re.compile(forbidden)
    return {path: n for path, text in files.items() if not allowed.search(path) and (n := len(forbidden.findall(text)))}


PLURAL_IDIOMS = [
    re.compile(r"[=!]==?\s*1\s*\{\s*\"[^\"]*\"\s*\}\s*else\s*\{\s*\"[^\"]*\""),            # Rust: if n == 1 { "" } else { "s" }
    re.compile(r"[=!]==?\s*1\s*\?\s*[\"'][^\"']*[\"']\s*:\s*[\"']"),                        # JS, TS, Java, C: n === 1 ? "" : "s"
    re.compile(r"[\"'][^\"']*[\"']\s+if\s+\w+\s*[=!]=\s*1\s+else\s+[\"']"),                 # Python: "" if n == 1 else "s"
]


def hand_spelled_plurals(files, skip=None):
    """The idiom in any words, and ``snapshot(s)`` inside a string, which leaves the plural to the reader. ``skip`` is a regular expression for
    the files that only *quote* the idiom (the ones that define this rule): their exemption is visible where it is configured."""
    found = {}
    skip = re.compile(skip) if skip else None
    for path, text in files.items():
        if skip and skip.search(path):
            continue
        flat = " ".join(text.split())
        n = sum(len(idiom.findall(flat)) for idiom in PLURAL_IDIOMS)
        for hack in ("(s)", "(es)"):
            at = flat.find(hack)
            while at != -1:
                quotes = flat[:at].count('"') - flat[:at].count('\\"')
                if quotes % 2 == 1 and flat[:at][-1:].isalpha():  # inside a string, after a letter: ``Ok(s)`` in code is not one
                    n += 1
                at = flat.find(hack, at + 1)
        if n:
            found[path] = n
    return found


def identifiers(text, line_comment=("//", "#")):
    """The identifiers of ``text`` that are code: not inside a string, a character or a comment."""
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if text.startswith(line_comment, i):
            i = text.find("\n", i)
            i = n if i == -1 else i
        elif text.startswith("/*", i):
            i = text.find("*/", i + 2)
            i = n if i == -1 else i + 2
        elif c in "\"'":
            quote = c * 3 if text.startswith(c * 3, i) else c
            i += len(quote)
            while i < n and not text.startswith(quote, i):
                i += 2 if text[i] == "\\" else 1
            i += len(quote)
        elif c.isalpha() or c == "_":
            j = i
            while j < n and (text[j].isalnum() or text[j] == "_"):
                j += 1
            yield text[i:j]
            i = j
        else:
            i += 1


def _parts(identifier):
    return [p.lower() for p in re.findall(r"[A-Z]?[a-z0-9]+|[A-Z]+(?![a-z])", identifier.replace("_", " "))]


def retired_word_uses(files, word, allowed=None):
    """Identifiers that have ``word`` as one of their parts (``config``, ``area_config``, ``ConfigName``), outside the files matching ``allowed``."""
    allowed = re.compile(allowed) if allowed else None
    found = {}
    for path, text in files.items():
        if allowed and allowed.search(path):
            continue
        n = sum(1 for name in identifiers(text) if word.lower() in _parts(name) or word.lower() + "s" in _parts(name))
        if n:
            found[path] = n
    return found
