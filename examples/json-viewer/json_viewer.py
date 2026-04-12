#!/usr/bin/env python3
"""
json-viewer — Plexi app
Collapsible tree viewer for JSON files.

Launch via PLEXI_LAUNCH_PATH env var pointing to a .json file,
or open any .json file from Plexi's file picker.

Controls:
  j / ↓    Move down
  k / ↑    Move up
  Enter / Space    Expand / collapse node
  q        Quit (close app)
"""
from __future__ import annotations

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Catppuccin Mocha
# ---------------------------------------------------------------------------

C = {
    "bg":      "#1e1e2e",
    "surface": "#313244",
    "text":    "#cdd6f4",
    "subtext": "#6c7086",
    "accent":  "#89b4fa",
    "green":   "#a6e3a1",
    "yellow":  "#f9e2af",
    "red":     "#f38ba8",
    "mauve":   "#cba6f7",
    "peach":   "#fab387",
    "header":  "#181825",
    "sel":     "#313244",
}

PADDING  = 16
HEADER_H = 48
ITEM_H   = 20
INDENT_W = 16

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

    @property
    def child_count(self) -> int:
        return len(self.children)

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
            return C["green"]
        if self.value is None:
            return C["subtext"]
        if isinstance(self.value, bool):
            return C["mauve"]
        if isinstance(self.value, (int, float)):
            return C["yellow"]
        return C["text"]


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
scroll: int = 0

# ---------------------------------------------------------------------------
# Load
# ---------------------------------------------------------------------------

def load_file(path: str):
    global root, error_msg, flat, cursor, scroll
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


@app.on_render
def render(ctx):
    global scroll, cursor

    w = ctx.width
    h = ctx.height

    ctx.rect(0, 0, w, h, fill=C["bg"])

    # Header
    ctx.rect(0, 0, w, HEADER_H, fill=C["header"])
    title = "JSON Viewer"
    if root:
        title += f"  —  {root.key}"
    ctx.text(PADDING, 14, title, size=14, color=C["accent"], bold=True)
    hint = "j/k=nav  Space=expand  q=quit"
    ctx.text(w - len(hint) * 7.2 - PADDING, 16, hint, size=10, color=C["subtext"])
    ctx.line(0, HEADER_H, w, HEADER_H, color=C["surface"], width=1.0)

    body_y = HEADER_H + PADDING
    body_h = h - body_y - PADDING

    if error_msg:
        ctx.text(PADDING, body_y, error_msg, size=13, color=C["red"])
        return

    if root is None:
        ctx.text(PADDING, body_y, "Drop a JSON file here or open via file picker.", size=13, color=C["subtext"])
        # Drop target hint
        ctx.rect(PADDING, body_y + 30, w - PADDING * 2, 80, fill=C["surface"], radius=8.0)
        ctx.text(w / 2 - 80, body_y + 62, "Drop .json file here", size=13, color=C["subtext"])
        ctx.drop_target("json", PADDING, body_y + 30, w - PADDING * 2, 80,
                        accept=["json"], label="Drop JSON file")
        return

    # Visible rows
    visible = max(1, int(body_h / ITEM_H))

    # Clamp scroll to keep cursor visible
    if cursor < scroll:
        scroll = cursor
    elif cursor >= scroll + visible:
        scroll = cursor - visible + 1
    scroll = max(0, min(scroll, max(0, len(flat) - visible)))

    for i, node in enumerate(flat[scroll:scroll + visible]):
        row_idx = scroll + i
        ry = body_y + i * ITEM_H

        # Selection highlight
        if row_idx == cursor:
            ctx.rect(0, ry, w, ITEM_H, fill=C["sel"])

        x = PADDING + node.depth * INDENT_W

        # Expand/collapse indicator
        if node.is_container:
            arrow = "▼" if node.expanded else "▶"
            ctx.text(x, ry + 2, arrow, size=11, color=C["subtext"])
            x += 14

        # Key
        if node.key is not None:
            ctx.text(x, ry + 2, node.key, size=12, color=C["accent"], bold=True)
            x += len(node.key) * 7.5 + 4
            ctx.text(x, ry + 2, ": ", size=12, color=C["subtext"])
            x += 10

        # Value / type label
        if node.is_container:
            ctx.text(x, ry + 2, node.type_label(), size=12, color=C["subtext"])
        else:
            ctx.text(x, ry + 2, node.value_str(), size=12, color=node.value_color())

    # Scroll indicator
    if len(flat) > visible:
        pct = scroll / max(1, len(flat) - visible)
        bar_h = max(20, int(body_h * visible / len(flat)))
        bar_y = body_y + int((body_h - bar_h) * pct)
        ctx.rect(w - 4, bar_y, 3, bar_h, fill=C["overlay"], radius=1.5)


@app.on_key
def on_key(key: str, _mods: dict, _emit):
    global cursor, scroll

    if key in ("j", "ArrowDown"):
        cursor = min(cursor + 1, len(flat) - 1)
    elif key in ("k", "ArrowUp"):
        cursor = max(cursor - 1, 0)
    elif key in ("Enter", " "):
        if flat and 0 <= cursor < len(flat):
            node = flat[cursor]
            if node.is_container:
                node.expanded = not node.expanded
                _rebuild_flat()
                # Clamp cursor after collapse
                cursor = min(cursor, len(flat) - 1)
    elif key == "q":
        pass  # Plexi handles quit via the pane close button; q is a no-op here


@app.on_drop
def on_drop(target_id: str, paths: list, _emit):
    if paths:
        load_file(paths[0])
        global cursor, scroll
        cursor = 0
        scroll = 0


app.run()
