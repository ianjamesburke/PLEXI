#!/usr/bin/env python3
"""Todo — fs.read + fs.write + persistence example for PGAP v3."""
from __future__ import annotations

import sys
import os

import json
import pathlib
from plexi_sdk import App, RenderContext, BG, FG, MUTED, ACCENT, SURFACE, GREEN, RED, BODY, CAPTION, HINT

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

    def on_render(self, ctx: RenderContext) -> None:
        ctx.rect(0, 0, ctx.w, ctx.h, fill=BG)
        ctx.rect(0, 0, ctx.w, 44, fill=SURFACE)
        ctx.text(16, 14, "Todo", size=18.0, color=ACCENT, bold=True)
        ctx.text(72, 18, str(self._cwd), size=CAPTION, color=MUTED, monospace=True)
        items = []
        for it in self._items:
            check = "✓" if it["done"] else "○"
            items.append({"label": f"{check} {it['text']}", "secondary": None})
        if items:
            ctx.list(items, selected=self._selected, item_height=40.0,
                     y=52, h=max(0, ctx.h - 112))
        else:
            ctx.text(16, 80, "No items. Press 'a' to add.", size=BODY, color=MUTED)
        # Input bar
        bar_y = ctx.h - 56
        if self._adding:
            ctx.rect(0, bar_y, ctx.w, 56, fill=SURFACE)
            ctx.text(16, bar_y + 8, "New item:", size=CAPTION, color=MUTED)
            ctx.text(16, bar_y + 26, self._input + "▌", size=BODY, color=FG, monospace=True)
        else:
            ctx.text(16, ctx.h - 24, "↑↓ select · Space toggle · a add · d delete", size=HINT, color=MUTED)


if __name__ == "__main__":
    TodoApp().run()
