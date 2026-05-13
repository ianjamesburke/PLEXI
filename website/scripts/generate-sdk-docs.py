#!/usr/bin/env python3
"""Generate Astro-compatible markdown docs from Python SDK source files.

Uses the ast module to extract classes, methods, and docstrings. Generates
markdown files in website/src/content/docs/ with Astro frontmatter.

Set SDK_SOURCE env var to override SDK path. Defaults to ../../sdk/python/plexi_sdk
"""

import ast
import os
import sys
from pathlib import Path
from typing import Optional

VERIFIED_VERSION = "3.6.54"


def get_sdk_source() -> Path:
    env_path = os.environ.get("SDK_SOURCE")
    if env_path:
        return Path(env_path)
    script_dir = Path(__file__).parent
    return script_dir.parent.parent / "sdk" / "python" / "plexi_sdk"


class DocExtractor:
    """Extract docstrings and signatures from Python source."""

    def __init__(self, file_path: Path):
        self.file_path = file_path
        with open(file_path, encoding="utf-8") as f:
            self.source = f.read()
        self.tree = ast.parse(self.source)

    def get_module_docstring(self) -> Optional[str]:
        return ast.get_docstring(self.tree)

    def extract_classes(self, include_private: bool = False) -> list[dict]:
        classes = []
        for node in self.tree.body:
            if isinstance(node, ast.ClassDef):
                if not include_private and node.name.startswith("_"):
                    continue
                classes.append(self._extract_class(node))
        return classes

    def _extract_class(self, node: ast.ClassDef) -> dict:
        return {
            "name": node.name,
            "docstring": ast.get_docstring(node),
            "methods": self._extract_methods(node),
        }

    def _extract_methods(self, class_node: ast.ClassDef) -> list[dict]:
        methods = []
        for node in class_node.body:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                if node.name.startswith("_"):
                    continue
                is_prop = any(
                    (isinstance(d, ast.Name) and d.id == "property")
                    or (isinstance(d, ast.Attribute) and d.attr == "setter")
                    for d in node.decorator_list
                )
                if is_prop:
                    continue
                methods.append(self._extract_method(node))
        return methods

    def _extract_method(self, node: "ast.FunctionDef | ast.AsyncFunctionDef") -> dict:
        is_async = isinstance(node, ast.AsyncFunctionDef)
        sig = self._signature(node)
        return {
            "name": node.name,
            "async": is_async,
            "signature": sig,
            "docstring": ast.get_docstring(node),
        }

    def _signature(self, node: "ast.FunctionDef | ast.AsyncFunctionDef") -> str:
        """Build method signature preserving type hints alongside defaults."""
        all_args = node.args.args
        n_defaults = len(node.args.defaults)
        n_args = len(all_args)

        parts: list[str] = []
        for i, arg in enumerate(all_args):
            if arg.arg == "self":
                continue

            s = arg.arg
            if arg.annotation:
                s += f": {ast.unparse(arg.annotation)}"

            default_idx = i - (n_args - n_defaults)
            if default_idx >= 0:
                default_str = ast.unparse(node.args.defaults[default_idx])
                s += f" = {default_str}"

            parts.append(s)

        if node.args.vararg:
            v = f"*{node.args.vararg.arg}"
            if node.args.vararg.annotation:
                v += f": {ast.unparse(node.args.vararg.annotation)}"
            parts.append(v)
        if node.args.kwarg:
            kw = f"**{node.args.kwarg.arg}"
            if node.args.kwarg.annotation:
                kw += f": {ast.unparse(node.args.kwarg.annotation)}"
            parts.append(kw)

        ret = ""
        if node.returns:
            ret = f" -> {ast.unparse(node.returns)}"

        prefix = "async def" if isinstance(node, ast.AsyncFunctionDef) else "def"
        return f"{prefix} {node.name}({', '.join(parts)}){ret}"


def _clean_docstring(doc: str) -> str:
    """Convert rST artifacts to markdown-friendly equivalents."""
    lines = doc.split("\n")
    cleaned = []
    for line in lines:
        if line.rstrip().endswith("::"):
            line = line.rstrip()[:-1]
        cleaned.append(line)
    return "\n".join(cleaned)


def fmt_methods(methods: list[dict]) -> str:
    out = ""
    for m in methods:
        out += f"### `{m['name']}`\n\n"
        out += f"```python\n{m['signature']}\n```\n\n"
        if m["docstring"]:
            out += f"{_clean_docstring(m['docstring'])}\n\n"
        out += "---\n\n"
    return out


def generate_sdk_overview(sdk_source: Path) -> str:
    init_file = sdk_source / "__init__.py"
    extractor = DocExtractor(init_file)
    module_doc = extractor.get_module_docstring() or ""

    return f'''---
title: "SDK Overview"
description: "Python SDK for building Plexi pane apps"
verified_version: "{VERIFIED_VERSION}"
---

{module_doc}
'''


def generate_app_docs(sdk_source: Path) -> str:
    extractor = DocExtractor(sdk_source / "_app.py")
    classes = extractor.extract_classes()
    app_class = next((c for c in classes if c["name"] == "App"), None)
    if not app_class:
        return _empty_page("App", "No App class found.")

    docstring = app_class["docstring"] or ""

    return f'''---
title: "App"
description: "Base class for Plexi pane apps with lifecycle hooks"
verified_version: "{VERIFIED_VERSION}"
---

# App

{docstring}

## Methods

{fmt_methods(app_class["methods"])}'''


def generate_render_context_docs(sdk_source: Path) -> str:
    extractor = DocExtractor(sdk_source / "_render_context.py")
    classes = extractor.extract_classes()
    ctx_class = next((c for c in classes if c["name"] == "RenderContext"), None)
    if not ctx_class:
        return _empty_page("RenderContext", "No RenderContext class found.")

    docstring = ctx_class["docstring"] or ""

    return f'''---
title: "RenderContext"
description: "Drawing API for rendering pane content"
verified_version: "{VERIFIED_VERSION}"
---

# RenderContext

{docstring}

## Properties

| Property | Type | Description |
|----------|------|-------------|
| `x` | `float` | Pane origin x (usually 0) |
| `y` | `float` | Pane origin y (usually 0) |
| `w` | `float` | Pane width in logical pixels |
| `h` | `float` | Pane height in logical pixels |
| `width` | `float` | Alias for `w` |
| `height` | `float` | Alias for `h` |
| `frame_id` | `int` | Monotonically increasing render counter |
| `elapsed` | `float` | Seconds since previous on_render (0.0 on first frame) |
| `workspace_root` | `str` | Absolute path to the workspace root |
| `capabilities` | `list[str]` | Granted capability strings |
| `feature_flags` | `list[str]` | Enabled feature flag strings |
| `emit` | `Emitter` | Emitter instance for out-of-frame commands |

## Methods

{fmt_methods(ctx_class["methods"])}'''


def generate_emitter_docs(sdk_source: Path) -> str:
    extractor = DocExtractor(sdk_source / "_emitter.py")
    classes = extractor.extract_classes()
    emitter_class = next((c for c in classes if c["name"] == "Emitter"), None)
    if not emitter_class:
        return _empty_page("Emitter", "No Emitter class found.")

    docstring = emitter_class["docstring"] or ""

    return f'''---
title: "Emitter"
description: "Host commands, notifications, HTTP, secrets, and device APIs"
verified_version: "{VERIFIED_VERSION}"
---

# Emitter

{docstring}

Available as `self.emit` on App or `ctx.emit` on RenderContext. All methods are thread-safe.

## Methods

{fmt_methods(emitter_class["methods"])}'''


def generate_widgets_docs(sdk_source: Path) -> str:
    widgets_dir = sdk_source / "widgets"
    widget_files = [
        ("Button & ButtonStyle", "button.py"),
        ("KeyMap", "keymap.py"),
        ("ListView", "list_view.py"),
        ("ScrollState", "scroll.py"),
        ("TextArea & TextAreaTheme", "text_area.py"),
        ("TextBuffer, Cursor & Selection", "text_buffer.py"),
    ]

    sections = ""
    for _, filename in widget_files:
        filepath = widgets_dir / filename
        if not filepath.exists():
            continue
        extractor = DocExtractor(filepath)
        classes = extractor.extract_classes()
        for cls in classes:
            sections += f"## {cls['name']}\n\n"
            if cls["docstring"]:
                sections += f"{_clean_docstring(cls['docstring'])}\n\n"
            if cls["methods"]:
                sections += fmt_methods(cls["methods"])

    return f'''---
title: "Widgets"
description: "Reusable UI components for Plexi apps"
verified_version: "{VERIFIED_VERSION}"
---

# Widgets

Import widgets from `plexi_sdk.widgets`:

```python
from plexi_sdk.widgets import (
    Button, ButtonStyle,
    KeyMap,
    ListView,
    ScrollState,
    TextArea, TextAreaTheme,
    TextBuffer, Cursor, Selection,
    emit_text_input,
)
```

{sections}'''


def _empty_page(title: str, msg: str) -> str:
    return f'''---
title: "{title}"
description: ""
verified_version: "{VERIFIED_VERSION}"
---

# {title}

{msg}
'''


def main() -> None:
    sdk_source = get_sdk_source()
    if not sdk_source.exists():
        print(f"Error: SDK source not found at {sdk_source}", file=sys.stderr)
        sys.exit(1)

    docs_dir = Path(__file__).parent.parent / "src" / "content" / "docs"
    docs_dir.mkdir(parents=True, exist_ok=True)

    pages = {
        "sdk.md": generate_sdk_overview(sdk_source),
        "sdk-app.md": generate_app_docs(sdk_source),
        "sdk-render-context.md": generate_render_context_docs(sdk_source),
        "sdk-emitter.md": generate_emitter_docs(sdk_source),
        "sdk-widgets.md": generate_widgets_docs(sdk_source),
    }

    for filename, content in pages.items():
        path = docs_dir / filename
        path.write_text(content)
        print(f"Generated {path}")

    print(f"\nSDK docs generated in {docs_dir}")


if __name__ == "__main__":
    main()
