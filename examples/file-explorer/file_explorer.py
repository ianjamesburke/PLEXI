#!/usr/bin/env python3
"""File Explorer — directory browser that tracks the terminal's cwd via PGAP PathChanged."""
from __future__ import annotations

import sys

import pathlib
from plexi_sdk import App, RenderContext, BG, MUTED, ACCENT, SURFACE, HINT

HEADER_H = 44.0
STATUS_H = 28.0


def _list_dir(path: pathlib.Path) -> list[tuple[str, bool]]:
    """Return sorted (name, is_dir) pairs for path, dirs first."""
    try:
        entries = list(path.iterdir())
    except PermissionError:
        return [("(permission denied)", False)]
    dirs = sorted((e.name, True) for e in entries if e.is_dir() and not e.name.startswith("."))
    hidden_dirs = sorted((e.name, True) for e in entries if e.is_dir() and e.name.startswith("."))
    files = sorted((e.name, False) for e in entries if e.is_file() and not e.name.startswith("."))
    hidden_files = sorted((e.name, False) for e in entries if e.is_file() and e.name.startswith("."))
    return dirs + hidden_dirs + files + hidden_files


class FileExplorer(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._cwd = pathlib.Path(self.workspace_root)
        self._entries: list[tuple[str, bool]] = []  # (name, is_dir)
        self._selected = 0
        self._status = ""
        self._refresh()

    def on_path_changed(self, ctx: RenderContext, cwd: str) -> None:  # noqa: ARG002
        """Terminal changed directory — follow it."""
        new = pathlib.Path(cwd)
        if new != self._cwd:
            self._cwd = new
            self._selected = 0
            self._refresh()

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:  # noqa: ARG002
        if key in ("ArrowUp", "k"):
            self._selected = max(0, self._selected - 1)
        elif key in ("ArrowDown", "j"):
            self._selected = min(len(self._entries) - 1, self._selected + 1)
        elif key == "Enter":
            self._open_selected()
        elif key == "Backspace" or (key == "h" and not mods.get("cmd")):
            self._go_parent()
        elif key == "r":
            self._refresh()

    def _open_selected(self) -> None:
        if not self._entries:
            return
        name, is_dir = self._entries[self._selected]
        target = self._cwd / name
        if is_dir:
            self._cwd = target
            self._selected = 0
            self._refresh()
            self.emit.cd_to(str(self._cwd))
        else:
            self._status = f"{name}"

    def _go_parent(self) -> None:
        parent = self._cwd.parent
        if parent != self._cwd:
            self._cwd = parent
            self._selected = 0
            self._refresh()
            self.emit.cd_to(str(self._cwd))

    def _refresh(self) -> None:
        self._entries = _list_dir(self._cwd)
        self.emit.status_summary(str(self._cwd))

    def on_render(self, ctx: RenderContext) -> None:
        ctx.rect(0, 0, ctx.w, ctx.h, fill=BG)

        # Header: current path
        ctx.rect(0, 0, ctx.w, HEADER_H, fill=SURFACE)
        ctx.text(16, 10, "Files", size=14.0, color=MUTED)
        path_str = str(self._cwd)
        # Truncate long paths from the left
        max_chars = int((ctx.w - 80) / 8)
        if len(path_str) > max_chars:
            path_str = "…" + path_str[-(max_chars - 1):]
        ctx.text(64, 10, path_str, size=14.0, color=ACCENT, bold=True, monospace=True)

        if self._status:
            ctx.text(16, HEADER_H - 16, self._status, size=HINT, color=MUTED)

        # File list
        items = []
        for name, is_dir in self._entries:
            label = ("/" + name) if is_dir else name
            items.append({"label": label, "secondary": None, "is_dir": is_dir})

        list_top = HEADER_H
        list_h = ctx.h - HEADER_H - STATUS_H
        ctx.rect(0, list_top, ctx.w, list_h, fill=BG)
        ctx.list(items, selected=self._selected, item_height=24.0,
                 y=list_top, h=list_h)

        # Footer
        ctx.rect(0, ctx.h - STATUS_H, ctx.w, STATUS_H, fill=SURFACE)
        ctx.text(16, ctx.h - STATUS_H + 8, "↑↓ navigate · Enter open · Backspace up · r refresh",
                 size=HINT, color=MUTED)


if __name__ == "__main__":
    FileExplorer().run()
