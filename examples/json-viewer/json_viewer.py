from __future__ import annotations
"""
json-viewer — Plexi app

Collapsible tree viewer for JSON files. Built on the Phase 1 SDK components
layer (Theme, ctx.header, ctx.status_bar, ctx.scrollable_list,
ctx.empty_state) — no inline palette, no hand-rolled scroll clamp, no
custom status bar.

Launch via PLEXI_LAUNCH_PATH env var pointing to a .json file, or open any
.json file from Plexi's file picker / drop target.

Controls:
  j / ↓            Move down
  k / ↑            Move up
  Enter / Space    Expand / collapse node
  ⌘W               Close app (host-handled)
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import (
    App,
    THEME,
    BODY, CAPTION, HINT, MONO_SMALL,
    PAD, HEADER_H,
)

ITEM_H = 22.0
INDENT_W = 16.0

# ---------------------------------------------------------------------------
# Tree node
# ---------------------------------------------------------------------------


class Node:
    def __init__(self, key: str | None, value: object, depth: int = 0):
        self.key = key
        self.value = value
        self.depth = depth
        self.expanded: bool = depth < 2  # auto-expand first two levels
        self.children: list[Node] = []
        self._build_children()

    def _build_children(self):
        if isinstance(self.value, dict):
            for k, v in self.value.items():
                self.children.append(Node(str(k), v, self.depth + 1))
        elif isinstance(self.value, list):
            for i, v in enumerate(self.value):
                self.children.append(Node(f"[{i}]", v, self.depth + 1))

    @property
    def is_container(self) -> bool:
        return isinstance(self.value, (dict, list))

    def type_label(self) -> str:
        if isinstance(self.value, dict):
            return f"{{}} {len(self.value)} keys"
        if isinstance(self.value, list):
            return f"[] {len(self.value)} items"
        return ""

    def value_str(self) -> str:
        if isinstance(self.value, str):
            snippet = self.value[:60]
            suffix = "…" if len(self.value) > 60 else ""
            return f'"{snippet}{suffix}"'
        if self.value is None:
            return "null"
        if isinstance(self.value, bool):
            return "true" if self.value else "false"
        return str(self.value)

    def value_color(self) -> str:
        if isinstance(self.value, str):
            return THEME.green
        if self.value is None:
            return THEME.muted
        if isinstance(self.value, bool):
            return THEME.accent
        if isinstance(self.value, (int, float)):
            return THEME.yellow
        return THEME.fg


def flatten(node: Node, out: list[Node]):
    out.append(node)
    if node.is_container and node.expanded:
        for child in node.children:
            flatten(child, out)


# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------

root: Node | None = None
error_msg: str = ""
flat: list[Node] = []
cursor: int = 0


# ---------------------------------------------------------------------------
# Load
# ---------------------------------------------------------------------------


def load_file(path: str):
    global root, error_msg, flat, cursor
    try:
        with open(path, "r", encoding="utf-8") as f:
            data = json.load(f)
        root = Node(os.path.basename(path), data, depth=0)
        error_msg = ""
    except json.JSONDecodeError as exc:
        error_msg = f"JSON parse error: {exc}"
        root = None
    except OSError as exc:
        error_msg = f"Cannot open file: {exc}"
        root = None
    _rebuild_flat()
    cursor = 0


def _rebuild_flat():
    global flat
    flat = []
    if root:
        flatten(root, flat)


# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

app = App(app_id="json-viewer")

# Try PLEXI_LAUNCH_PATH at startup
_launch_path = os.environ.get("PLEXI_LAUNCH_PATH", "")
if _launch_path and os.path.isfile(_launch_path):
    load_file(_launch_path)


def _render_row(ctx, node, i, x, y, w, is_sel):
    if is_sel:
        ctx.rect(x, y, w, ITEM_H, fill=THEME.highlight)

    tx = x + PAD + node.depth * INDENT_W
    text_y = y + (ITEM_H - BODY) / 2 - 1

    # Expand/collapse indicator
    if node.is_container:
        arrow = "▼" if node.expanded else "▶"
        ctx.text(tx, text_y, arrow, size=CAPTION, color=THEME.muted)
        tx += 14

    # Key
    if node.key is not None:
        ctx.text(tx, text_y, node.key, size=BODY, color=THEME.accent, bold=True, monospace=True)
        tx += ctx.measure_text(node.key, BODY, monospace=True) + 4
        ctx.text(tx, text_y, ":", size=BODY, color=THEME.muted, monospace=True)
        tx += 10

    # Value / type label
    if node.is_container:
        ctx.text(tx, text_y, node.type_label(), size=BODY, color=THEME.muted, monospace=True)
    else:
        ctx.text(tx, text_y, node.value_str(), size=BODY, color=node.value_color(), monospace=True)


@app.on_render
def render(ctx):
    ctx.rect(0, 0, ctx.width, ctx.height, fill=THEME.bg)

    # ── error state ──────────────────────────────────────────────────────────
    if error_msg:
        ctx.header("JSON Viewer  ·  error")
        ctx.empty_state("Could not load file", error_msg, icon_color=THEME.red)
        ctx.status_bar([("⌘W", "close")])
        return

    # ── empty state ──────────────────────────────────────────────────────────
    if root is None:
        ctx.header("JSON Viewer")
        ctx.empty_state("No file loaded", "Drop a .json file here or open via the file picker.")
        # Drop target covers the body area so users can drag in a file.
        drop_y = HEADER_H + PAD
        drop_h = ctx.height - drop_y - 30.0 - PAD
        ctx.drop_target(
            "json", PAD, drop_y, ctx.width - PAD * 2, drop_h,
            accept=["json"], label="Drop JSON file",
        )
        ctx.status_bar([("⌘W", "close")])
        return

    # ── tree view ────────────────────────────────────────────────────────────
    title = f"JSON Viewer  ·  {root.key}"
    ctx.header(title)

    ctx.scrollable_list(
        list_id="tree",
        items=flat,
        selected=cursor,
        row_height=ITEM_H,
        render_row=_render_row,
    )

    ctx.status_bar(
        [
            ("j/k", "navigate"),
            ("Enter", "expand"),
            ("⌘W", "close"),
        ],
    )


# ---------------------------------------------------------------------------
# Input
# ---------------------------------------------------------------------------


@app.on_key
def on_key(key, mods, emit):
    global cursor

    if not flat:
        return

    if key in ("j", "ArrowDown"):
        cursor = min(cursor + 1, len(flat) - 1)
    elif key in ("k", "ArrowUp"):
        cursor = max(cursor - 1, 0)
    elif key in ("Enter", " "):
        if 0 <= cursor < len(flat):
            node = flat[cursor]
            if node.is_container:
                node.expanded = not node.expanded
                _rebuild_flat()
                cursor = min(cursor, len(flat) - 1)


@app.on_drop
def on_drop(target_id, paths, emit):
    global cursor
    if paths:
        load_file(paths[0])
        cursor = 0


app.run()
