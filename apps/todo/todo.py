#!/usr/bin/env python3
"""Todo — the "hello world that's useful." Simple list with persistence.

L1 Reference Implementation
============================
Patterns demonstrated:
  - Fully L1: Column + AppBar + Section + SelectList + FooterKeys + Label + Spacer
  - TextInput for inline item creation
  - JSON file persistence (.plexi/todos.json in workspace)
  - on_path_changed: reloads data when workspace context changes
  - on_escape for contextual mode exit (add mode vs. close)
  - Minimal state: list of dicts, selected index, adding flag
"""

import json
import pathlib

from plexi_sdk import App
from plexi_sdk.ui import (
    Column, AppBar, Section, Spacer, FooterKeys,
    SelectList, TextInput, Label,
)

TODO_FILE = ".plexi/todos.json"


class TodoApp(App):
    def on_init(self) -> None:
        self._items: list[dict] = []
        self._selected = 0
        self._adding = False
        self._cwd = pathlib.Path(self.workspace_root)
        self._list = SelectList([], selected_idx=0)
        self._input = TextInput("todo-add", placeholder="New item…", height=48.0)
        self._load()

    def on_path_changed(self, cwd: str) -> None:
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
        self._sync_list()

    def _save(self) -> None:
        try:
            p = self._path()
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(json.dumps(self._items, indent=2))
        except Exception as e:
            self.emit.error(f"todo save failed: {e}")

    def _sync_list(self) -> None:
        """Rebuild SelectList items from self._items and keep selection in bounds."""
        self._list.items = [
            {"name": f"{'✓' if it['done'] else '○'}  {it['text']}"}
            for it in self._items
        ]
        if self._items:
            self._selected = max(0, min(self._selected, len(self._items) - 1))
        else:
            self._selected = 0
        self._list.selected_idx = self._selected

    def on_escape(self, _ctx):
        if self._adding:
            self._adding = False
            return True
        return False

    def on_key(self, key: str, _mods: dict) -> None:
        if self._adding:
            return

        if key in ("up", "k", "ArrowUp"):
            self._list.handle_key("k")
            self._selected = self._list.selected_idx
        elif key in ("down", "j", "ArrowDown"):
            self._list.handle_key("j")
            self._selected = self._list.selected_idx
        elif key == " " and self._items:
            self._items[self._selected]["done"] ^= True
            self._save()
            self._sync_list()
        elif key == "a":
            self._adding = True
        elif key == "d" and self._items:
            self._items.pop(self._selected)
            self._selected = min(self._selected, max(0, len(self._items) - 1))
            self._save()
            self._sync_list()

    def on_render(self, ctx) -> None:
        if self._adding:
            body = Column([
                AppBar(title="Todo"),
                Section("Add item"),
                self._input,
                Spacer(grow=True),
                FooterKeys([
                    ("Enter", "save"),
                    ("Esc", "cancel"),
                ]),
            ])
            ctx.render(body)
            submitted = self._input.submitted
            if submitted is not None:
                text = submitted.strip()
                if text:
                    self._items.append({"text": text, "done": False})
                    self._save()
                    self._sync_list()
                self._adding = False
        else:
            if self._items:
                list_area = self._list
            else:
                list_area = Label("No items. Press 'a' to add.", tone="hint")  # type: ignore[assignment]
            ctx.render(Column([
                AppBar(title="Todo"),
                Section("Current list"),
                list_area,
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
