#!/usr/bin/env python3
"""Backlog viewer — browse, preview, open, archive, and delete .plexi/backlog items.

hjkl navigation. e/Enter opens in default app. a archives. d deletes (confirm).
r refreshes. / searches. Items are markdown files inside <workspace>/.plexi/backlog/.
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '../../sdk/python'))
from plexi_sdk import App, RenderContext, BG, SURFACE, HIGHLIGHT, ACCENT, MUTED, FG, RED, GREEN

ROW_H     = 22.0
LIST_FRAC = 0.38   # fraction of width for the item list


class BacklogApp(App):

    def on_init(self, ctx: RenderContext) -> None:
        self.backlog_dir = Path(ctx.workspace_root) / ".plexi" / "backlog"
        self.archived_dir = self.backlog_dir / "archived"
        self.items: list = []         # list[Path]
        self.filtered: list = []
        self.selected = 0
        self.search_query = ""
        self.in_search = False
        self.preview_text = ""
        self.preview_path = None
        self.confirm_delete = False
        self.status = ""
        self._load()

    # ── Data ────────────────────────────────────────────────────────────────────

    def _load(self) -> None:
        self.items = []
        if not self.backlog_dir.exists():
            self.filtered = []
            return
        files = [
            f for f in self.backlog_dir.iterdir()
            if f.is_file() and not f.name.startswith(".")
        ]
        files.sort(key=lambda f: f.stat().st_mtime, reverse=True)
        self.items = files
        self._refilter()

    def _refilter(self) -> None:
        q = self.search_query.lower()
        self.filtered = [
            f for f in self.items
            if not q or q in f.stem.lower() or q in f.name.lower()
        ]
        self.selected = min(self.selected, max(0, len(self.filtered) - 1))
        self._cache_preview()

    def _cache_preview(self) -> None:
        if not self.filtered or self.selected >= len(self.filtered):
            self.preview_text = ""
            self.preview_path = None
            return
        path = self.filtered[self.selected]
        if path == self.preview_path:
            return
        self.preview_path = path
        try:
            self.preview_text = path.read_text(errors="replace")
        except OSError as e:
            self.preview_text = f"Error: {e}"

    # ── Actions ─────────────────────────────────────────────────────────────────

    def _open(self) -> None:
        if not self.filtered:
            return
        path = self.filtered[self.selected]
        subprocess.Popen(["open", str(path)])
        self.status = f"Opened {path.name}"

    def _archive(self) -> None:
        if not self.filtered:
            return
        path = self.filtered[self.selected]
        self.archived_dir.mkdir(parents=True, exist_ok=True)
        dest = self.archived_dir / path.name
        try:
            shutil.move(str(path), str(dest))
            self.status = f"Archived {path.name}"
        except OSError as e:
            self.status = f"Error: {e}"
        self.selected = max(0, self.selected - 1)
        self._load()

    def _delete(self) -> None:
        if not self.filtered:
            return
        path = self.filtered[self.selected]
        try:
            path.unlink()
            self.status = f"Deleted {path.name}"
        except OSError as e:
            self.status = f"Error: {e}"
        self.confirm_delete = False
        self.selected = max(0, self.selected - 1)
        self._load()

    # ── Render ───────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        w, h = ctx.w, ctx.h
        ctx.clear(BG)

        list_w = w * LIST_FRAC
        top = 32.0
        bottom_bar_h = 26.0
        list_h = h - top - bottom_bar_h

        # Header
        ctx.text(12, 10, ".plexi/backlog", size=12, color=ACCENT, bold=True)
        item_count = len(self.filtered)
        count_label = f"{item_count} item{'s' if item_count != 1 else ''}"
        ctx.text(list_w - 80, 10, count_label, size=11, color=MUTED)

        if not self.backlog_dir.exists():
            ctx.text(12, top + 12, "No backlog directory found.", size=13, color=FG)
            ctx.text(12, top + 34, f"mkdir -p {self.backlog_dir}", size=11, color=MUTED, monospace=True)
            return

        # Divider
        ctx.line(list_w, top - 4, list_w, h - bottom_bar_h, color=HIGHLIGHT, width=1.0)

        # ── Item list ────────────────────────────────────────────────────────────
        visible_start = max(0, self.selected - int(list_h / ROW_H) + 4)
        for i, path in enumerate(self.filtered):
            if i < visible_start:
                continue
            row_y = top + (i - visible_start) * ROW_H
            if row_y + ROW_H > h - bottom_bar_h:
                break
            is_sel = i == self.selected
            if is_sel:
                ctx.rect(1, row_y, list_w - 2, ROW_H, HIGHLIGHT, radius=3.0)
            name = path.stem
            color = FG if is_sel else MUTED
            ctx.text(12, row_y + 4, name, size=12, color=color)

        if not self.filtered:
            msg = "No results" if self.search_query else "Backlog is empty"
            ctx.text(12, top + 12, msg, size=12, color=MUTED)

        # ── Preview ──────────────────────────────────────────────────────────────
        px = list_w + 14
        pw = w - px - 10
        if self.preview_path and self.preview_text is not None:
            ctx.text(px, top - 18, self.preview_path.name,
                     size=11, color=ACCENT, bold=True)
            lines = self.preview_text.splitlines()
            for li, line in enumerate(lines):
                ly = top + li * 15.0
                if ly > h - bottom_bar_h - 4:
                    break
                # Dim markdown headings differently
                color = FG if not line.startswith("#") else ACCENT
                ctx.text(px, ly, line[:int(pw / 7)], size=11, color=color, monospace=True)

        # ── Bottom bar ───────────────────────────────────────────────────────────
        bar_y = h - bottom_bar_h
        ctx.rect(0, bar_y, w, bottom_bar_h, SURFACE)

        if self.in_search:
            ctx.text(12, bar_y + 7,
                     f"/ {self.search_query}\u258c",   # block cursor
                     size=12, color=ACCENT, monospace=True)
        elif self.status:
            ctx.text(12, bar_y + 7, self.status, size=11, color=GREEN)
        else:
            ctx.text(12, bar_y + 7,
                     "j/k  navigate    e  open    a  archive    d  delete    /  search    r  refresh",
                     size=10, color=MUTED)

        # ── Delete confirm overlay ────────────────────────────────────────────────
        if self.confirm_delete and self.filtered:
            name = self.filtered[self.selected].name
            ox, oy, ow, oh = w / 2 - 170, h / 2 - 36, 340, 72
            ctx.rect(ox, oy, ow, oh, SURFACE, radius=6.0)
            ctx.rect(ox, oy, ow, 1, HIGHLIGHT)
            ctx.text(ox + 16, oy + 14, f"Delete  \u2018{name}\u2019?",
                     size=13, color=RED, bold=True)
            ctx.text(ox + 16, oy + 36, "Enter \u2192 confirm    Esc \u2192 cancel",
                     size=11, color=MUTED)

    # ── Keys ─────────────────────────────────────────────────────────────────────

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        self.status = ""   # clear on any key

        # ── Confirm-delete mode ──────────────────────────────────────────────────
        if self.confirm_delete:
            if key == "Enter":
                self._delete()
            elif key in ("Escape", "d"):
                self.confirm_delete = False
            return

        # ── Search mode ──────────────────────────────────────────────────────────
        if self.in_search:
            if key == "Escape":
                self.in_search = False
                self.search_query = ""
                self._refilter()
            elif key == "Backspace":
                self.search_query = self.search_query[:-1]
                self._refilter()
            elif key == "Enter":
                self.in_search = False
            elif len(key) == 1:   # single char from Text event; ignores "Slash" etc.
                self.search_query += key
                self._refilter()
            return

        # ── Normal mode ──────────────────────────────────────────────────────────
        n = len(self.filtered)

        if key in ("j", "ArrowDown"):
            self.selected = min(self.selected + 1, max(0, n - 1))
            self._cache_preview()
        elif key in ("k", "ArrowUp"):
            self.selected = max(0, self.selected - 1)
            self._cache_preview()
        elif key in ("e", "Enter"):
            self._open()
        elif key == "a":
            self._archive()
        elif key == "d" and self.filtered:
            self.confirm_delete = True
        elif key == "r":
            self._load()
            self.status = "Refreshed"
        elif key == "/":             # Text event sends "/" for the slash key
            self.in_search = True
            self.search_query = ""
            self._refilter()


if __name__ == "__main__":
    BacklogApp().run()
