#!/usr/bin/env python3
"""
todo — Plexi app
Persistent todo list saved to ~/.plexi-alpha/todo.txt.

Controls:
  j / ↓    Move down
  k / ↑    Move up
  Space / Enter    Toggle done
  a        Add item (type at inline prompt, Enter to confirm, Esc to cancel)
  d        Delete selected item
  q        Quit
"""
from __future__ import annotations

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
    "red":     "#f38ba8",
    "yellow":  "#f9e2af",
    "header":  "#181825",
    "sel":     "#313244",
    "done":    "#585b70",
}

PADDING    = 16
HEADER_H   = 48
ITEM_H     = 28
FOOTER_H   = 36

TODO_PATH  = os.path.expanduser("~/.plexi-alpha/todo.txt")

# ---------------------------------------------------------------------------
# Persistence
# ---------------------------------------------------------------------------

def load_todos() -> list[dict]:
    """Load todos from file. Each line: `[x] text` or `[ ] text`."""
    todos = []
    try:
        os.makedirs(os.path.dirname(TODO_PATH), exist_ok=True)
        if not os.path.exists(TODO_PATH):
            return todos
        with open(TODO_PATH, "r", encoding="utf-8") as f:
            for line in f:
                line = line.rstrip("\n")
                if not line:
                    continue
                done = line.startswith("[x] ")
                text = line[4:] if line.startswith(("[x] ", "[ ] ")) else line
                todos.append({"text": text, "done": done})
    except OSError as exc:
        pass  # return empty list; error will surface on next save
    return todos


def save_todos(todos: list[dict]):
    try:
        os.makedirs(os.path.dirname(TODO_PATH), exist_ok=True)
        with open(TODO_PATH, "w", encoding="utf-8") as f:
            for t in todos:
                prefix = "[x] " if t["done"] else "[ ] "
                f.write(prefix + t["text"] + "\n")
    except OSError as exc:
        pass  # non-fatal; data lives in memory


# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------

todos: list[dict] = load_todos()
cursor: int = 0
scroll: int = 0

MODE_LIST  = "list"
MODE_ADD   = "add"
mode: str  = MODE_LIST
add_buf: str = ""

# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

app = App(app_id="todo")


@app.on_render
def render(ctx):
    global scroll, cursor

    w = ctx.width
    h = ctx.height

    ctx.rect(0, 0, w, h, fill=C["bg"])

    # Header
    ctx.rect(0, 0, w, HEADER_H, fill=C["header"])
    count_done = sum(1 for t in todos if t["done"])
    ctx.text(PADDING, 14, f"Todo  ({count_done}/{len(todos)} done)", size=14, color=C["accent"], bold=True)
    hint = "a=add  d=delete  Space=toggle"
    ctx.text(w - len(hint) * 7.2 - PADDING, 16, hint, size=10, color=C["subtext"])
    ctx.line(0, HEADER_H, w, HEADER_H, color=C["surface"], width=1.0)

    body_y = HEADER_H + PADDING
    body_h = h - body_y - FOOTER_H

    # Visible rows
    visible = max(1, int(body_h / ITEM_H))

    # Clamp scroll to keep cursor visible
    if cursor < scroll:
        scroll = cursor
    elif cursor >= scroll + visible:
        scroll = cursor - visible + 1
    scroll = max(0, min(scroll, max(0, len(todos) - visible)))

    if not todos:
        ctx.text(PADDING, body_y + 10, "No todos yet. Press a to add one.", size=13, color=C["subtext"])
    else:
        for i, todo in enumerate(todos[scroll:scroll + visible]):
            row_idx = scroll + i
            ry = body_y + i * ITEM_H

            # Selection highlight
            if row_idx == cursor and mode == MODE_LIST:
                ctx.rect(0, ry, w, ITEM_H, fill=C["sel"], radius=4.0)

            # Checkbox
            check = "✓" if todo["done"] else "○"
            check_color = C["green"] if todo["done"] else C["subtext"]
            ctx.text(PADDING, ry + 6, check, size=13, color=check_color)

            # Item text
            text = todo["text"]
            text_color = C["done"] if todo["done"] else C["text"]
            # Strikethrough visual: prepend ~~ prefix for done items
            if todo["done"]:
                text = "~~" + text + "~~"
            ctx.text(PADDING + 24, ry + 6, text, size=13, color=text_color)

    # Footer: add prompt or status bar
    footer_y = h - FOOTER_H
    ctx.rect(0, footer_y, w, FOOTER_H, fill=C["header"])
    ctx.line(0, footer_y, w, footer_y, color=C["surface"], width=1.0)

    if mode == MODE_ADD:
        prompt = f"New item: {add_buf}_"
        ctx.text(PADDING, footer_y + 10, prompt, size=13, color=C["yellow"])
        ctx.text(w - 120, footer_y + 10, "Enter=save  Esc=cancel", size=10, color=C["subtext"])
    else:
        ctx.text(PADDING, footer_y + 10, TODO_PATH, size=10, color=C["subtext"])


@app.on_key
def on_key(key: str, _mods: dict, _emit):
    global cursor, scroll, mode, add_buf, todos

    if mode == MODE_ADD:
        if key == "Escape":
            mode = MODE_LIST
            add_buf = ""
        elif key == "Enter":
            text = add_buf.strip()
            if text:
                todos.append({"text": text, "done": False})
                save_todos(todos)
                cursor = len(todos) - 1
            mode = MODE_LIST
            add_buf = ""
        elif key == "Backspace":
            add_buf = add_buf[:-1]
        elif len(key) == 1 and key.isprintable():
            add_buf += key
        return

    # MODE_LIST
    if key in ("j", "ArrowDown"):
        cursor = min(cursor + 1, len(todos) - 1) if todos else 0
    elif key in ("k", "ArrowUp"):
        cursor = max(cursor - 1, 0)
    elif key in (" ", "Enter"):
        if todos and 0 <= cursor < len(todos):
            todos[cursor]["done"] = not todos[cursor]["done"]
            save_todos(todos)
    elif key == "a":
        mode = MODE_ADD
        add_buf = ""
    elif key == "d":
        if todos and 0 <= cursor < len(todos):
            todos.pop(cursor)
            cursor = min(cursor, len(todos) - 1)
            save_todos(todos)
    elif key == "q":
        pass  # Plexi handles pane close


app.run()
