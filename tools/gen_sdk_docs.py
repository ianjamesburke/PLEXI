#!/usr/bin/env python3
"""Generate website/src/content/docs/sdk.md from SDK Python docstrings.

Walks sdk/python/plexi_sdk/ using the ast module (no import side-effects),
extracts module / class / function docstrings, and emits structured markdown.

Run via: just gen-sdk-docs
"""
from __future__ import annotations

import ast
import sys
import textwrap
from pathlib import Path
from typing import Iterator

REPO_ROOT = Path(__file__).resolve().parent.parent
SDK_ROOT = REPO_ROOT / "sdk" / "python" / "plexi_sdk"
OUTPUT_PATH = REPO_ROOT / "website" / "src" / "content" / "docs" / "sdk.md"

# Files processed in section order. Each entry is (path-relative-to-SDK_ROOT, section-title).
# Private helper files are included selectively.
SECTIONS: list[tuple[str, str]] = [
    ("__init__.py",        "Overview"),
    ("effects.py",         "Effects"),
    ("events.py",          "Events"),
    ("_v3_state.py",       "State and Logging"),
    ("ui.py",              "UI Components"),
    ("testing.py",         "Testing"),
    ("_types.py",          "Types"),
    ("_theme.py",          "Theme"),
    ("_constants.py",      "Constants"),
    ("_protocol.py",       "Protocol Types"),
]

# Classes whose public methods are expanded with individual subsections.
EXPAND_CLASSES = {
    "Theme", "AppPalette", "StateProxy", "LogProxy",
}

# Methods to skip even on expanded classes (internal plumbing).
SKIP_METHODS = {"__init_subclass__", "__class_getitem__"}


def _is_public(name: str) -> bool:
    return not name.startswith("_") or name in ("__init__",)


def _is_property(node: ast.FunctionDef | ast.AsyncFunctionDef) -> bool:
    for deco in node.decorator_list:
        if isinstance(deco, ast.Name) and deco.id == "property":
            return True
    return False


def _fmt_default(node: ast.expr) -> str:
    """Format a default value, collapsing sentinel names to '...'."""
    text = ast.unparse(node)
    # Collapse internal sentinels and verbose expressions.
    if isinstance(node, ast.Name) and node.id.startswith("_"):
        return "..."
    return text


def _sig(node: ast.FunctionDef | ast.AsyncFunctionDef) -> str:
    """Return a compact signature string, omitting 'self' and type annotations."""
    if _is_property(node):
        return node.name  # properties have no parens

    args = node.args
    params: list[str] = []

    # positional args
    n_defaults = len(args.defaults)
    n_args = len(args.args)
    for i, arg in enumerate(args.args):
        if arg.arg == "self":
            continue
        default_offset = i - (n_args - n_defaults)
        if default_offset >= 0:
            default_node = args.defaults[default_offset]
            params.append(f"{arg.arg}={_fmt_default(default_node)}")
        else:
            params.append(arg.arg)

    # *args
    if args.vararg:
        params.append(f"*{args.vararg.arg}")

    # keyword-only args
    for kw, default in zip(args.kwonlyargs, args.kw_defaults):
        if default is not None:
            params.append(f"{kw.arg}={_fmt_default(default)}")
        else:
            params.append(kw.arg)

    # **kwargs
    if args.kwarg:
        params.append(f"**{args.kwarg.arg}")

    return f"{node.name}({', '.join(params)})"


def _dedent(doc: str) -> str:
    return textwrap.dedent(doc).strip()


def _iter_top_level(
    tree: ast.Module,
) -> Iterator[ast.ClassDef | ast.FunctionDef | ast.AsyncFunctionDef]:
    for node in tree.body:
        if isinstance(node, (ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)):
            yield node


def _iter_methods(
    cls: ast.ClassDef,
) -> Iterator[ast.FunctionDef | ast.AsyncFunctionDef]:
    for node in cls.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            yield node


def render_class(cls: ast.ClassDef, expand: bool) -> list[str]:
    lines: list[str] = []
    doc = ast.get_docstring(cls)
    lines.append(f"### `{cls.name}`")
    if doc:
        lines.append("")
        lines.append(_dedent(doc))

    if not expand:
        return lines

    methods: list[ast.FunctionDef | ast.AsyncFunctionDef] = [
        m for m in _iter_methods(cls)
        if _is_public(m.name) and m.name not in SKIP_METHODS
    ]
    if not methods:
        return lines

    lines.append("")
    for m in methods:
        mdoc = ast.get_docstring(m)
        if _is_property(m):
            prefix = "property "
        elif isinstance(m, ast.AsyncFunctionDef):
            prefix = "async "
        else:
            prefix = ""
        lines.append(f"#### `{prefix}{_sig(m)}`")
        if mdoc:
            lines.append("")
            lines.append(_dedent(mdoc))
        lines.append("")

    return lines


def render_function(fn: ast.FunctionDef | ast.AsyncFunctionDef) -> list[str]:
    lines: list[str] = []
    doc = ast.get_docstring(fn)
    prefix = "async " if isinstance(fn, ast.AsyncFunctionDef) else ""
    lines.append(f"### `{prefix}{_sig(fn)}`")
    if doc:
        lines.append("")
        lines.append(_dedent(doc))
    return lines


def process_file(path: Path, section_title: str) -> list[str]:
    src = path.read_text(encoding="utf-8")
    tree = ast.parse(src, filename=str(path))

    lines: list[str] = []
    lines.append(f"## {section_title}")
    lines.append("")

    mod_doc = ast.get_docstring(tree)
    if mod_doc:
        lines.append(_dedent(mod_doc))
        lines.append("")

    for node in _iter_top_level(tree):
        if isinstance(node, ast.ClassDef):
            if node.name.startswith("_"):
                continue
            expand = node.name in EXPAND_CLASSES
            lines.extend(render_class(node, expand=expand))
            lines.append("")
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            if not _is_public(node.name):
                continue
            lines.extend(render_function(node))
            lines.append("")

    return lines


FRONTMATTER = """\
---
title: Python SDK
description: Reference for the Plexi Python SDK v3 native app API.
order: 6
---
"""


def generate() -> str:
    out: list[str] = []
    out.append(FRONTMATTER.rstrip())
    out.append("<!-- Generated by tools/gen_sdk_docs.py — do not edit by hand. -->")
    out.append("<!-- Run `just gen-sdk-docs` to regenerate. -->")
    out.append("")

    for rel_path, title in SECTIONS:
        path = SDK_ROOT / rel_path
        if not path.exists():
            print(f"WARNING: {path} not found, skipping.", file=sys.stderr)
            continue
        section_lines = process_file(path, title)
        out.extend(section_lines)

    # Trim trailing blank lines, end with single newline.
    text = "\n".join(out).rstrip() + "\n"
    return text


def main() -> None:
    if "--stdout" in sys.argv:
        sys.stdout.write(generate())
        return
    content = generate()
    OUTPUT_PATH.write_text(content, encoding="utf-8")
    print(f"Written: {OUTPUT_PATH.relative_to(REPO_ROOT)}")


if __name__ == "__main__":
    main()
