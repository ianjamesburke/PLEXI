#!/usr/bin/env python3
"""Quick Note — first-party note composer for PGAP v3.

Opens to blank composer. Enter saves note (first line = title).
Cmd+K or F1 opens note browser. Saves to <workspace_root>/.plexi/notes/<timestamp>.md.
Posts a notification on save (surfaces in notification palette).
"""
from __future__ import annotations

import pathlib
from datetime import datetime

from plexi_sdk import (
    App, RenderContext,
    FG, MUTED, SURFACE, GREEN, BODY, CAPTION, HINT,
    PRIORITY_LOW,
)

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
            self.emit.notify(title="Note saved", body=first_line, level="info", priority=PRIORITY_LOW)
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

    # ── Render ─────────────────────────────────────────────────────────────
    # SDK v2 gives us the top Header + bottom Footer. The editor body (compose
    # lines with cursor, or browse list + side preview) is a custom functional
    # surface and stays on primitive draws. A _BodyGap component reserves the
    # vertical space between Header and Footer; its render paints the body.

    class _Body:
        """Escape hatch: paints the compose/browse body inside the Column's
        remaining space. `is_grow=True` so Column gives us everything left
        over after Header and Footer are measured."""
        def __init__(self, app: "QuickNoteApp") -> None:
            self._app = app

        def measure(self, avail_w: float) -> float:
            return 0.0

        def is_grow(self) -> bool:
            return True

        def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
            self._app._render_body(ctx, x, y, w, h)

    def _render_body(self, ctx, x: float, y: float, w: float, h: float) -> None:
        if self._mode == "compose":
            cy = y
            for i, line in enumerate(self._lines):
                color = FG if i == 0 else (MUTED if i > 0 else FG)
                size = 18.0 if i == 0 else BODY
                cursor = "▌" if i == self._cursor_line else ""
                ctx.text(x, cy, line + cursor, size=size, color=color)
                cy += size + 6
                if cy > y + h - 4:
                    break
        else:
            # Browse mode — list on the left, optional preview on the right.
            items = [{"label": p.stem, "secondary": None} for p in self._notes]
            if items:
                ctx.list(items, selected=self._selected, item_height=40.0,
                         x=x, y=y, w=w, h=h)
            else:
                ctx.text(x, y + 16, "No notes yet.", size=BODY, color=MUTED)
            # Preview panel (right side if wide enough)
            if w > 520 and self._preview:
                px = x + w * 0.5
                ctx.rect(px, y, w - (px - x), h, fill=SURFACE)
                py = y + 12
                for ln in self._preview.split("\n")[: int((h - 16) / 18)]:
                    ctx.text(px + 12, py, ln, size=CAPTION, color=FG)
                    py += 18

    def _shortcut_footer(self):
        from plexi_sdk.ui import FooterKeys
        if self._mode == "compose":
            return FooterKeys([
                (["⌘", "↵"], "save"),
                (["⌘", "k"], "browse notes"),
            ])
        return FooterKeys([
            (["↑", "↓"], "navigate"),
            ("↵", "open"),
            ("esc", "back"),
        ])

    def on_render(self, ctx: RenderContext) -> None:
        from plexi_sdk.ui import Column, Header, Spacer, Footer

        subtitle = "Compose" if self._mode == "compose" else "Browse saved notes"
        children = [
            Header(title="Quick Note", subtitle=subtitle),
            self._Body(self),
            Spacer(size=HINT),
        ]
        if self._status:
            children.append(Footer(self._status, color=GREEN))
        children.append(self._shortcut_footer())
        ctx.render(Column(children))


if __name__ == "__main__":
    QuickNoteApp().run()
