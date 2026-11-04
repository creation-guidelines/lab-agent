# SPDX-FileCopyrightText: 2026 grenudi
#
# SPDX-License-Identifier: GPL-3.0-or-later

"""The outline: every module of a program with the first sentence that says what it is for, which a stranger can read alone.

    outline.py [--layer NAME=DIR]... [--out FILE]

The outline is the cold read made repeatable. A module's purpose is the first sentence of its documentation: the module docstring
(Python), the leading ``//!`` lines (Rust), the leading ``/** */`` block (TypeScript, JavaScript), the comment above ``package`` (Go),
the first comment lines (shell). A module with none is listed with an empty purpose, and counted, because that is the first thing to
fix. The result is one line per module, ``layer/path: purpose``, which ``findability.py`` ranks against real questions.

With no --layer, each directory directly under src/ is a layer, or the whole tree is one.
"""
import ast
import os
import re
import sys

EXTENSIONS = ("py", "rs", "ts", "tsx", "js", "jsx", "go", "sh", "bash")
SKIP_FILES = {"__init__.py", "mod.rs", "lib.rs", "main.rs", "index.ts", "index.js", "__main__.py"}
SKIP_COMMENTS = ("SPDX", "Copyright", "Generalised from", "Generalized from", "shellcheck", "!")


def first_sentence(text):
    text = " ".join((text or "").split())
    match = re.match(r"(.+?[.!?])(?:\s|$)", text)
    sentence = match.group(1) if match else text
    return sentence.rstrip(".").strip()


def _python(text):
    try:
        return ast.get_docstring(ast.parse(text)) or ""
    except SyntaxError:
        return ""


def _rust(text):
    lines = []
    for line in text.split("\n"):
        stripped = line.strip()
        if stripped.startswith("//!"):
            lines.append(stripped[3:].strip())
        elif lines and stripped == "":
            break
        elif lines or stripped == "" or stripped.startswith(("//", "#![", "#[")):
            if lines:
                break
            continue
        else:
            break
    return "\n".join(lines)


def _block_comment(text):
    match = re.match(r"\s*(?:(?://[^\n]*\n|/\*(?!\*)[\s\S]*?\*/)\s*)*/\*\*([\s\S]*?)\*/", text)
    if not match:
        return ""
    return "\n".join(re.sub(r"^\s*\*\s?", "", line) for line in match.group(1).split("\n"))


def _go(text):
    lines = text.split("\n")
    at = next((i for i, line in enumerate(lines) if line.startswith("package ")), None)
    if at is None:
        return ""
    block = []
    i = at - 1
    while i >= 0 and lines[i].lstrip().startswith("//"):
        block.insert(0, lines[i].lstrip()[2:].strip())
        i -= 1
    return re.sub(r"^Package \w+\s*", "", " ".join(block))


def _shell(text):
    block = []
    for line in text.split("\n")[1:] if text.startswith("#!") else text.split("\n"):
        stripped = line.strip()
        if not stripped.startswith("#"):
            if block or stripped:
                break
            continue
        content = stripped.lstrip("#").strip()
        if not content or content.startswith(SKIP_COMMENTS):
            if block:
                break
            continue
        block.append(content)
    return " ".join(block)


HANDLERS = {"py": _python, "rs": _rust, "ts": _block_comment, "tsx": _block_comment, "js": _block_comment, "jsx": _block_comment,
            "go": _go, "sh": _shell, "bash": _shell}


def purpose(path, text):
    """The first sentence of what the module at ``path`` says it is for, or an empty string."""
    return first_sentence(HANDLERS[path.rsplit(".", 1)[-1]](text))


def build(layers):
    """``layers``: name -> directory. Returns a sorted list of (name, purpose), name being ``layer/relative/path``."""
    found = []
    for layer, root in layers.items():
        for directory, _, files in os.walk(root):
            inside = os.path.relpath(directory, root).split(os.sep)  # only the part below the root can be hidden
            if any((part.startswith(".") and part != ".") or part in ("node_modules", "target", "__pycache__") for part in inside):
                continue
            for file in files:
                if file in SKIP_FILES or file.rsplit(".", 1)[-1] not in EXTENSIONS:
                    continue
                path = os.path.join(directory, file)
                try:
                    text = open(path, encoding="utf-8").read()
                except (UnicodeDecodeError, OSError):
                    continue
                found.append((f"{layer}/{os.path.relpath(path, root)}", purpose(path, text)))
    return sorted(found)


def render(modules):
    return "\n".join(f"{name}: {purpose}" if purpose else f"{name}:" for name, purpose in modules) + "\n"


def parse(text):
    """The inverse of ``render``: a list of (name, purpose)."""
    out = []
    for line in text.split("\n"):
        if line.strip():
            name, _, rest = line.partition(":")
            out.append((name.strip(), rest.strip()))
    return out


def default_layers():
    if os.path.isdir("src"):
        found = {d: os.path.join("src", d) for d in sorted(os.listdir("src")) if os.path.isdir(os.path.join("src", d)) and not d.startswith(".")}
        return found or {"src": "src"}
    return {"all": "."}


def main(argv):
    values = lambda name: [argv[i + 1] for i, a in enumerate(argv) if a == f"--{name}" and i + 1 < len(argv)]
    layers = dict(item.split("=", 1) for item in values("layer")) or default_layers()
    modules = build(layers)
    text = render(modules)
    out = (values("out") or [None])[-1]
    if out:
        with open(out, "w", encoding="utf-8") as f:
            f.write(text)
    else:
        sys.stdout.write(text)
    missing = [name for name, p in modules if not p]
    print(f"{len(modules)} modules; {len(missing)} without a purpose sentence" + (f": {', '.join(missing[:5])}" if missing else ""), file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
