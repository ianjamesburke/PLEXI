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


def _fmt_param(arg: ast.arg, default: ast.expr | None) -> str:
    """Render one parameter with its annotation and default."""
    text = arg.arg
    if arg.annotation is not None:
        text += f": {ast.unparse(arg.annotation)}"
    if default is not None:
        sep = " = " if arg.annotation is not None else "="
        text += f"{sep}{_fmt_default(default)}"
    return text


def _params(args: ast.arguments) -> list[str]:
    """Render a function's parameters (annotations kept, 'self' omitted)."""
    params: list[str] = []

    # positional args
    n_defaults = len(args.defaults)
    n_args = len(args.args)
    for i, arg in enumerate(args.args):
        if arg.arg == "self":
            continue
        default_offset = i - (n_args - n_defaults)
        default = args.defaults[default_offset] if default_offset >= 0 else None
        params.append(_fmt_param(arg, default))

    # *args
    if args.vararg:
        params.append(f"*{args.vararg.arg}")

    # keyword-only args
    for kw, default in zip(args.kwonlyargs, args.kw_defaults):
        params.append(_fmt_param(kw, default))

    # **kwargs
    if args.kwarg:
        params.append(f"**{args.kwarg.arg}")

    return params


def _sig(node: ast.FunctionDef | ast.AsyncFunctionDef) -> str:
    """Return a signature string with annotations, omitting 'self'."""
    if _is_property(node):
        return node.name  # properties have no parens
    return f"{node.name}({', '.join(_params(node.args))})"


def _dedent(doc: str) -> str:
    return textwrap.dedent(doc).strip()


def _is_dataclass(cls: ast.ClassDef) -> bool:
    for deco in cls.decorator_list:
        target = deco.func if isinstance(deco, ast.Call) else deco
        if isinstance(target, ast.Name) and target.id == "dataclass":
            return True
        if isinstance(target, ast.Attribute) and target.attr == "dataclass":
            return True
    return False


def _fmt_field_default(node: ast.expr) -> str:
    """Format a dataclass field default; unwrap `field(...)` calls."""
    if isinstance(node, ast.Call) and (
        (isinstance(node.func, ast.Name) and node.func.id == "field")
        or (isinstance(node.func, ast.Attribute) and node.func.attr == "field")
    ):
        for kw in node.keywords:
            if kw.arg == "default" and kw.value is not None:
                return _fmt_default(kw.value)
            if kw.arg == "default_factory" and kw.value is not None:
                return f"{ast.unparse(kw.value)}()"
        return "..."
    return _fmt_default(node)


def _dataclass_params(cls: ast.ClassDef, class_map: dict[str, ast.ClassDef]) -> list[str]:
    """Synthesize the generated __init__ parameters for a dataclass.

    Follows dataclass semantics: base-class fields come first in declaration
    order; a redeclared field keeps its original position but takes the
    override's annotation/default. Bases are resolved within the same module
    (the SDK keeps each dataclass hierarchy in one file).
    """
    ordered: dict[str, str] = {}

    def collect(node: ast.ClassDef) -> None:
        for base in node.bases:
            if isinstance(base, ast.Name) and base.id in class_map:
                base_cls = class_map[base.id]
                if _is_dataclass(base_cls):
                    collect(base_cls)
        for stmt in node.body:
            if not (isinstance(stmt, ast.AnnAssign) and isinstance(stmt.target, ast.Name)):
                continue
            name = stmt.target.id
            annotation = ast.unparse(stmt.annotation)
            if name.startswith("_") or annotation.startswith("ClassVar"):
                continue
            rendered = f"{name}: {annotation}"
            if stmt.value is not None:
                rendered += f" = {_fmt_field_default(stmt.value)}"
            ordered[name] = rendered

    collect(cls)
    return list(ordered.values())


def _class_constructor_sig(
    cls: ast.ClassDef, class_map: dict[str, ast.ClassDef]
) -> str | None:
    """The class's construction signature, or None when it has no public one."""
    if _is_dataclass(cls):
        return f"{cls.name}({', '.join(_dataclass_params(cls, class_map))})"
    for method in _iter_methods(cls):
        if method.name == "__init__":
            return f"{cls.name}({', '.join(_params(method.args))})"
    return None


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


def render_class(
    cls: ast.ClassDef, expand: bool, class_map: dict[str, ast.ClassDef]
) -> list[str]:
    lines: list[str] = []
    doc = ast.get_docstring(cls)
    lines.append(f"### `{cls.name}`")
    constructor = _class_constructor_sig(cls, class_map)
    if constructor is not None:
        lines.append("")
        lines.append("```python")
        lines.append(constructor)
        lines.append("```")
    if doc:
        lines.append("")
        lines.append(_dedent(doc))

    if not expand:
        return lines

    methods: list[ast.FunctionDef | ast.AsyncFunctionDef] = [
        m for m in _iter_methods(cls)
        # __init__ is already covered by the constructor signature above.
        if _is_public(m.name) and m.name != "__init__" and m.name not in SKIP_METHODS
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

    class_map = {
        node.name: node for node in tree.body if isinstance(node, ast.ClassDef)
    }
    for node in _iter_top_level(tree):
        if isinstance(node, ast.ClassDef):
            if node.name.startswith("_"):
                continue
            expand = node.name in EXPAND_CLASSES
            lines.extend(render_class(node, expand=expand, class_map=class_map))
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
