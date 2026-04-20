#!/usr/bin/env python3
"""Quick Note — first-party note composer for PGAP v3.

Opens to blank composer. Enter saves note (first line = title).
Cmd+K or F1 opens note browser. Saves to <workspace_root>/.plexi/notes/<timestamp>.md.
Posts a notification on save (surfaces in notification palette).
"""
from __future__ import annotations

import sys
import os

import pathlib
import time
from datetime import datetime
from plexi_sdk import App, RenderContext, BG, FG, MUTED, ACCENT, SURFACE, HIGHLIGHT, GREEN, BODY, CAPTION, HINT

NOTES_DIR = ".plexi/notes"


class QuickNoteApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._mode = "compose"   # compose | browse
        self._lines: list[str] = [""]
        self._cursor_line = 0
        self._notes: list[pathlib.Path] = []
        self._selected = 0
        self._preview: str = ""
        self._status = ""

    def _notes_dir(self) -> pathlib.Path:
        return pathlib.Path(self.workspace_root) / NOTES_DIR

    def _load_notes(self) -> None:
        try:
            d = self._notes_dir()
            if d.exists():
                self._notes = sorted(d.glob("*.md"), reverse=True)
            else:
                self._notes = []
        except Exception as e:
            self.emit.error(f"quick-note list failed: {e}")

    def _save(self) -> None:
        text = "\n".join(self._lines).strip()
        if not text:
            return
        first_line = self._lines[0].strip() or "Untitled"
        try:
            d = self._notes_dir()
            d.mkdir(parents=True, exist_ok=True)
            ts = datetime.now().strftime("%Y%m%d_%H%M%S")
            fname = d / f"{ts}.md"
            fname.write_text(text)
            self._status = f"Saved: {fname.name}"
            self.emit.notify(title="Note saved", body=first_line, level="info")
            # Reset composer
            self._lines = [""]
            self._cursor_line = 0
        except Exception as e:
            self.emit.error(f"quick-note save failed: {e}")
            self._status = f"Error: {e}"

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        cmd = mods.get("cmd", False)
        if self._mode == "compose":
            if cmd and key == "k":
                self._mode = "browse"
                self._load_notes()
                return
            if key == "Enter" and cmd:
                self._save()
                return
            if key == "Enter":
                # Insert newline
                self._lines.insert(self._cursor_line + 1, "")
                self._cursor_line += 1
            elif key == "Backspace":
                if self._lines[self._cursor_line]:
                    self._lines[self._cursor_line] = self._lines[self._cursor_line][:-1]
                elif self._cursor_line > 0:
                    self._lines.pop(self._cursor_line)
                    self._cursor_line -= 1
            elif key == "ArrowUp":
                self._cursor_line = max(0, self._cursor_line - 1)
            elif key == "ArrowDown":
                self._cursor_line = min(len(self._lines) - 1,
                                        self._cursor_line + 1)
            elif len(key) == 1:
                self._lines[self._cursor_line] += key
        elif self._mode == "browse":
            if key == "Escape":
                self._mode = "compose"
            elif key in ("ArrowUp", "k"):
                self._selected = max(0, self._selected - 1)
                self._load_preview()
            elif key in ("ArrowDown", "j"):
                self._selected = min(len(self._notes) - 1, self._selected + 1)
                self._load_preview()
            elif key == "Enter" and self._notes:
                # Load note into composer
                try:
                    text = self._notes[self._selected].read_text()
                    self._lines = text.split("\n")
                    self._cursor_line = len(self._lines) - 1
                    self._mode = "compose"
                except Exception as e:
                    self.emit.error(f"quick-note open failed: {e}")

    def _load_preview(self) -> None:
        if not self._notes:
            self._preview = ""
            return
        try:
            self._preview = self._notes[self._selected].read_text()[:400]
        except Exception:
            self._preview = ""

    def on_render(self, ctx: RenderContext) -> None:
        ctx.rect(0, 0, ctx.w, ctx.h, fill=BG)
        ctx.rect(0, 0, ctx.w, 44, fill=SURFACE)
        ctx.text(16, 14, "Quick Note", size=18.0, color=ACCENT, bold=True)
        if self._status:
            ctx.text(ctx.w - 300, 14, self._status, size=12.0, color=GREEN)

        if self._mode == "compose":
            y = 60.0
            for i, line in enumerate(self._lines):
                color = FG if i == 0 else MUTED if i > 0 else FG
                size = 18.0 if i == 0 else BODY
                cursor = "▌" if i == self._cursor_line else ""
                ctx.text(16, y, line + cursor, size=size, color=color)
                y += size + 6
                if y > ctx.h - 60:
                    break
            ctx.text(16, ctx.h - 24,
                     "Cmd+Enter save · Cmd+K browse notes", size=HINT, color=MUTED)
        else:
            # Browse mode
            items = [{"label": p.stem, "secondary": None} for p in self._notes]
            if items:
                ctx.list(items, selected=self._selected, item_height=40.0)
            else:
                ctx.text(16, 80, "No notes yet.", size=BODY, color=MUTED)
            # Preview panel (right side if wide enough)
            if ctx.w > 600 and self._preview:
                px = ctx.w * 0.5
                ctx.rect(px, 44, ctx.w - px, ctx.h - 44, fill=SURFACE)
                py = 60.0
                for l in self._preview.split("\n")[:int((ctx.h - 80) / 18)]:
                    ctx.text(px + 12, py, l, size=CAPTION, color=FG)
                    py += 18
            ctx.text(16, ctx.h - 24,
                     "↑↓ navigate · Enter open · Esc back", size=HINT, color=MUTED)


if __name__ == "__main__":
    QuickNoteApp().run()
