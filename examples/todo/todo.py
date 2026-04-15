from __future__ import annotations
"""
todo — Plexi app
Persistent todo list saved to ~/.plexi-alpha/todo.txt.

Controls:
  j / ↓            Move down
  k / ↑            Move up
  Space / Enter    Toggle done
  a                Add item (type at inline prompt, Enter to confirm, Esc to cancel)
  d                Delete selected item
  ⌘W               Close (host-handled)
"""

import os

from plexi_sdk import (
    App,
    THEME,
    BODY, CAPTION, HINT,
    PAD,
)


TODO_PATH = os.path.expanduser("~/.plexi-alpha/todo.txt")
ITEM_H = 28.0


# ── persistence ───────────────────────────────────────────────────────────────
def load_todos() -> list[dict]:
    """Load todos from file. Each line: `[x] text` or `[ ] text`."""
    todos: list[dict] = []
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
    except OSError:
        pass  # return empty list; error will surface on next save
    return todos


def save_todos(todos: list[dict]) -> None:
    try:
        os.makedirs(os.path.dirname(TODO_PATH), exist_ok=True)
        with open(TODO_PATH, "w", encoding="utf-8") as f:
            for t in todos:
                prefix = "[x] " if t["done"] else "[ ] "
                f.write(prefix + t["text"] + "\n")
    except OSError:
        pass  # non-fatal; data lives in memory


# ── state ─────────────────────────────────────────────────────────────────────
todos: list[dict] = load_todos()
cursor: int = 0

MODE_LIST = "list"
MODE_ADD = "add"
mode: str = MODE_LIST
add_buf: str = ""

app = App(app_id="todo")


# ── rendering ─────────────────────────────────────────────────────────────────
def _render_row(ctx, todo, i, x, y, w, is_sel):
    if is_sel and mode == MODE_LIST:
        ctx.rect(x + PAD / 2, y, w - PAD, ITEM_H, fill=THEME.highlight, radius=4.0)

    check = "✓" if todo["done"] else "○"
    check_color = THEME.green if todo["done"] else THEME.muted
    ctx.text(x + PAD, y + 6, check, size=CAPTION, color=check_color)

    text = todo["text"]
    if todo["done"]:
        text = "~~" + text + "~~"
        text_color = THEME.muted
    else:
        text_color = THEME.fg
    ctx.text(x + PAD + 24, y + 6, text, size=CAPTION, color=text_color)


@app.on_render
def render(ctx):
    ctx.rect(0, 0, ctx.width, ctx.height, fill=THEME.bg)

    count_done = sum(1 for t in todos if t["done"])
    ctx.header(f"Todo  ({count_done}/{len(todos)} done)")

    if not todos:
        ctx.empty_state("No todos yet", "Press a to add one.")
    else:
        ctx.scrollable_list(
            list_id="todos",
            items=todos,
            selected=cursor,
            row_height=ITEM_H,
            render_row=_render_row,
        )

    if mode == MODE_ADD:
        # Replace the status bar with an inline add prompt.
        bar_h = 30.0
        bar_y = ctx.height - bar_h
        ctx.rect(0, bar_y, ctx.width, bar_h, fill=THEME.surface)
        text_y = bar_y + (bar_h - HINT) / 2
        ctx.text(PAD, text_y, f"New item: {add_buf}_", size=HINT, color=THEME.yellow)
        ctx.text_right(
            ctx.width - PAD, text_y,
            "Enter  save    Esc  cancel",
            size=HINT, color=THEME.muted,
        )
    else:
        ctx.status_bar(
            [
                ("j/k", "navigate"),
                ("Space/Enter", "toggle"),
                ("a", "add"),
                ("d", "delete"),
                ("⌘W", "close"),
            ],
        )


# ── input ─────────────────────────────────────────────────────────────────────
@app.on_key
def on_key(key: str, _mods: dict, _emit):
    global cursor, mode, add_buf, todos

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
            cursor = min(cursor, len(todos) - 1) if todos else 0
            save_todos(todos)


app.run()
