#!/usr/bin/env python3
"""Todo — fs.read + fs.write + persistence example for PGAP v3."""
from __future__ import annotations

import json
import pathlib

from plexi_sdk import (
    App, RenderContext,
    FG, MUTED, SURFACE, BODY,
)
from plexi_sdk.ui import (
    Column, AppBar, Section, Label, Spacer, FooterKeys,
)

TODO_FILE = ".plexi/todos.json"


class TodoApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._items: list[dict] = []
        self._selected = 0
        self._adding = False
        self._input = ""
        # The pane group "cwd" lets the todo list follow the linked terminal.
        # Starts at workspace_root; PathChanged updates it when the terminal cds.
        self._cwd = pathlib.Path(self.workspace_root)
        self._load()

    def on_path_changed(self, ctx: RenderContext, cwd: str) -> None:
        if not cwd:
            return
        new_cwd = pathlib.Path(cwd)
        if new_cwd == self._cwd:
            return
        self._cwd = new_cwd
        self._selected = 0
        self._items = []
        self._load()
        self.emit.info(f"todo: cwd -> {cwd}")

    def _path(self) -> pathlib.Path:
        return self._cwd / TODO_FILE

    def _load(self) -> None:
        try:
            p = self._path()
            if p.exists():
                self._items = json.loads(p.read_text())
        except Exception as e:
            self.emit.error(f"todo load failed: {e}")

    def _save(self) -> None:
        try:
            p = self._path()
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(json.dumps(self._items, indent=2))
        except Exception as e:
            self.emit.error(f"todo save failed: {e}")

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if self._adding:
            if key == "Enter":
                if self._input.strip():
                    self._items.append({"text": self._input.strip(), "done": False})
                    self._save()
                self._input = ""
                self._adding = False
            elif key == "Escape":
                self._input = ""
                self._adding = False
            elif key == "Backspace":
                self._input = self._input[:-1]
            elif len(key) == 1:
                self._input += key
            return
        if key in ("ArrowUp", "k"):
            self._selected = max(0, self._selected - 1)
        elif key in ("ArrowDown", "j"):
            self._selected = min(len(self._items) - 1, self._selected + 1)
        elif key == " " and self._items:
            self._items[self._selected]["done"] ^= True
            self._save()
        elif key == "a":
            self._adding = True
        elif key == "d" and self._items:
            self._items.pop(self._selected)
            self._selected = min(self._selected, len(self._items) - 1)
            self._save()

    # ── Item list body — functional surface, primitive draws ───────────────
    class _Body:
        """Renders the todo list (or the add-item input bar) into whatever
        vertical space the Column hands us."""
        def __init__(self, app: "TodoApp") -> None:
            self._app = app

        def measure(self, avail_w: float) -> float:
            return 0.0

        def is_grow(self) -> bool:
            return True

        def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
            app = self._app
            items = [
                {"label": f"{'✓' if it['done'] else '○'} {it['text']}",
                 "secondary": None}
                for it in app._items
            ]
            if app._adding:
                # Add-item input bar occupies the bottom ~56 px of the body.
                bar_h = 56.0
                list_h = max(0.0, h - bar_h - 8.0)
                if items:
                    ctx.list(items, selected=app._selected, item_height=40.0,
                             x=x, y=y, w=w, h=list_h)
                bar_y = y + h - bar_h
                ctx.rect(x, bar_y, w, bar_h, fill=SURFACE, radius=4.0)
                ctx.text(x + 12, bar_y + 8, "New item:", size=13, color=MUTED)
                ctx.text(x + 12, bar_y + 28, app._input + "▌",
                         size=BODY, color=FG, monospace=True)
                return
            if items:
                ctx.list(items, selected=app._selected, item_height=40.0,
                         x=x, y=y, w=w, h=h)
            else:
                ctx.text(x, y + 12, "No items. Press 'a' to add.",
                         size=BODY, color=MUTED)

    def on_render(self, ctx: RenderContext) -> None:
        ctx.render(Column([
            AppBar(title="Todo")),
            Section("Current list"),
            self._Body(self),
            Spacer(grow=False),
            FooterKeys([
                (["↑", "↓"], "select"),
                ("space", "toggle"),
                ("a", "add"),
                ("d", "delete"),
            ]),
        ]))


if __name__ == "__main__":
    TodoApp().run()
