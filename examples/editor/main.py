#!/usr/bin/env python3
"""Editor — minimal native text editor PGAP app.

Opens via `plexi open editor [file]`. Full-page TextArea + TextBuffer.
Cmd+S to save. Cmd+W (close) is handled by the host.
"""
import asyncio
import sys
from pathlib import Path

from plexi_sdk import App, RenderContext, BG, FG, MUTED, SURFACE, ACCENT, CAPTION, HINT, PAD
from plexi_sdk.widgets.text_area import TextArea, TextAreaTheme
from plexi_sdk.widgets.text_buffer import TextBuffer

HEADER_H = 36.0
FOOTER_H = 24.0


class EditorApp(App):
    async def on_init(self, ctx: RenderContext) -> None:
        path_arg = sys.argv[1].strip() if len(sys.argv) > 1 else None
        self._path: Path | None = Path(path_arg).expanduser().resolve() if path_arg else None
        self._dirty = False
        self._status = ""

        initial_text = ""
        if self._path and self._path.exists():
            try:
                initial_text = await asyncio.to_thread(self._path.read_text, encoding="utf-8")
            except Exception as e:
                self._status = f"Error reading file: {e}"
                ctx.warn(f"editor: failed to read {self._path}: {e}")

        self._buffer = TextBuffer(initial_text)
        self._area = TextArea(
            self._buffer,
            theme=TextAreaTheme(
                background=BG,
                foreground=FG,
                cursor=ACCENT,
                selection="#45475a",
                line_height=20.0,
                font_size=14.0,
                char_width=8.4,
                padding=PAD,
                monospace=True,
            ),
        )
        ctx.info(f"editor: init path={self._path}")

    def _title(self) -> str:
        name = self._path.name if self._path else "untitled"
        return f"{name}{'*' if self._dirty else ''}"

    async def _save(self, ctx: RenderContext) -> None:
        if not self._path:
            self._status = "no file — pass a path: plexi open editor <path>"
            ctx.warn("editor: save attempted with no path")
            return
        try:
            text = self._buffer.text()
            await asyncio.to_thread(self._path.write_text, text, encoding="utf-8")
            self._dirty = False
            self._status = f"Saved {self._path.name}"
            ctx.info(f"editor: saved path={self._path}")
        except Exception as e:
            self._status = f"Save failed: {e}"
            ctx.error(f"editor: save failed path={self._path}: {e}")

    async def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        cmd = mods.get("cmd", False)
        shift = mods.get("shift", False)
        ctrl = mods.get("ctrl", False)

        if cmd and key == "s":
            await self._save(ctx)
            return

        # Cmd+W is consumed by the host (close_pane binding) before reaching the app.
        # All other cmd-combos (cmd+c, cmd+v, etc.) are also host-handled.
        if cmd:
            return

        self._area.on_key(key, shift=shift, ctrl=ctrl)
        _mutating = {"backspace", "delete", "enter", "return"}
        if len(key) == 1 or key in _mutating:
            self._dirty = True
        self._status = ""

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)

        # ── Header ───────────────────────────────────────────────────
        ctx.rect(0, 0, ctx.w, HEADER_H, fill=SURFACE, radius=0)
        title = self._title()
        ctx.text(PAD, (HEADER_H - CAPTION) / 2, title, size=CAPTION, color=FG, bold=True)
        hint = "⌘S save  ⌘W close"
        ctx.text(ctx.w - PAD, (HEADER_H - HINT) / 2, hint, size=HINT, color=MUTED, align="right")

        # ── Editor body ──────────────────────────────────────────────
        body_y = HEADER_H
        body_h = max(0.0, ctx.h - HEADER_H - FOOTER_H)
        for cmd_dict in self._area.render(0, body_y, ctx.w, body_h):
            ctx._queue(cmd_dict)

        # ── Footer ───────────────────────────────────────────────────
        footer_y = ctx.h - FOOTER_H
        ctx.rect(0, footer_y, ctx.w, FOOTER_H, fill=SURFACE, radius=0)
        status_text = self._status or (str(self._path) if self._path else "untitled — no file path")
        ctx.text(PAD, footer_y + (FOOTER_H - HINT) / 2, status_text, size=HINT, color=MUTED,
                 max_width=ctx.w - PAD * 2, elide=True)


EditorApp().run()
