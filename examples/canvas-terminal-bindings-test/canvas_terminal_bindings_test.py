#!/usr/bin/env python3
"""Canvas Terminal Bindings Demo — POC for #78.

Exercises all five v3.5 Canvas Terminal Binding Primitives over a single
linked terminal pane. The app opens its terminal at startup; each on-screen
button dispatches one primitive against the linked terminal.

Buttons (click or press the matching number):
  1. Run `ls`        → run_in_linked_terminal
  2. Insert /tmp     → insert_path_token
  3. Preview rm      → request_command_preview (modal-style readout)
  4. Open ~/Desktop  → open_artifact (open_in_pane)
  5. Reveal ~/       → open_artifact (reveal_in_finder)

Capability: `terminal.bindings`.
"""
from __future__ import annotations

import os

from plexi_sdk import (
    App,
    RenderContext,
    BG,
    FG,
    ACCENT,
    MUTED,
    SURFACE,
    HIGHLIGHT,
    CapabilityDeniedError,
)


BUTTONS = [
    ("1", "Run `ls` in linked terminal"),
    ("2", "Insert /tmp path token"),
    ("3", "Preview `rm -rf .git`"),
    ("4", "Open ~/Desktop in pane"),
    ("5", "Reveal ~/ in Finder"),
]


class CanvasTerminalBindingsTestApp(App):
    async def on_init(self, ctx: RenderContext) -> None:
        self._terminal_pane_id: int = 0
        self._last_action: str = "(no action yet)"
        self._preview_text: str = ""
        # Await the linked terminal — suspends only this coroutine, keeping the
        # event loop free to process host events. If the manifest doesn't
        # declare `terminal.bindings` this raises CapabilityDeniedError.
        try:
            self._terminal_pane_id = await self.emit.request_linked_terminal(
                cwd=None,
                label="bindings demo",
            )
            ctx.status_summary(
                f"Bindings Demo — terminal {self._terminal_pane_id}"
            )
            self.emit.info(
                f"canvas-terminal-bindings-test: linked to pane "
                f"{self._terminal_pane_id}"
            )
        except CapabilityDeniedError as e:
            ctx.status_summary("ERROR: terminal.bindings capability missing")
            self.emit.error(str(e))

    def on_render(self, ctx: RenderContext) -> None:
        w, h = ctx.w, ctx.h

        ctx.rect(0, 0, w, h, BG)

        # Header.
        ctx.text(
            x=24, y=24,
            text="canvas terminal bindings demo",
            size=22.0, color=FG, bold=True,
        )
        # Subtitle: the linked terminal pane id (or error).
        if self._terminal_pane_id == 0:
            subtitle = "no linked terminal — manifest missing terminal.bindings"
            color = "#ef4444"
        else:
            subtitle = f"linked terminal pane #{self._terminal_pane_id}  ·  click a button or press 1–5"
            color = MUTED
        ctx.text(x=24, y=58, text=subtitle, size=12.0, color=color)

        # Buttons.
        btn_x = 24
        btn_y = 96
        btn_w = max(w - 48, 240)
        btn_h = 44
        gap = 10
        for i, (key, label) in enumerate(BUTTONS):
            y = btn_y + i * (btn_h + gap)
            ctx.rect(btn_x, y, btn_w, btn_h, SURFACE, radius=6.0)
            ctx.text(
                x=btn_x + 14, y=y + 22,
                text=f"[{key}]",
                size=14.0, color=ACCENT, monospace=True, bold=True,
                align="left_center",
            )
            ctx.text(
                x=btn_x + 60, y=y + 22,
                text=label,
                size=14.0, color=FG, align="left_center",
                max_width=btn_w - 80, elide=True,
            )

        # Status panel.
        status_y = btn_y + len(BUTTONS) * (btn_h + gap) + 12
        ctx.rect(btn_x, status_y, btn_w, 84, HIGHLIGHT, radius=6.0)
        ctx.text(
            x=btn_x + 14, y=status_y + 16,
            text="last action",
            size=11.0, color=MUTED, bold=True,
        )
        ctx.text(
            x=btn_x + 14, y=status_y + 36,
            text=self._last_action,
            size=13.0, color=FG, monospace=True,
            max_width=btn_w - 28, elide=True,
        )
        if self._preview_text:
            ctx.text(
                x=btn_x + 14, y=status_y + 60,
                text=self._preview_text,
                size=12.0, color=ACCENT, monospace=True,
                max_width=btn_w - 28, elide=True,
            )

    async def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if any(mods.values()):
            return
        if key == "1":
            self._run_ls()
        elif key == "2":
            self._insert_tmp()
        elif key == "3":
            await self._preview_rm()
        elif key == "4":
            self._open_desktop()
        elif key == "5":
            self._reveal_home()
        self.emit.schedule_render(after_ms=16)

    async def on_click(
        self,
        ctx: RenderContext,
        x: float,
        y: float,
        button: str,
    ) -> None:
        # Same hit regions as on_render.
        btn_y = 96
        btn_h = 44
        gap = 10
        idx = int((y - btn_y) // (btn_h + gap))
        if 0 <= idx < len(BUTTONS) and 24 <= x:
            sync_handlers = [self._run_ls, self._insert_tmp, None,
                             self._open_desktop, self._reveal_home]
            if idx == 2:
                await self._preview_rm()
            elif sync_handlers[idx]:
                sync_handlers[idx]()
        self.emit.schedule_render(after_ms=16)

    # ── Primitive dispatchers ──────────────────────────────────────────

    def _need_terminal(self) -> bool:
        if self._terminal_pane_id == 0:
            self._last_action = "no linked terminal — capability denied"
            return False
        return True

    def _run_ls(self) -> None:
        if not self._need_terminal():
            return
        self.emit.run_in_linked_terminal(self._terminal_pane_id, "ls", echo=True)
        self._last_action = "run_in_linked_terminal('ls', echo=True)"
        self._preview_text = ""

    def _insert_tmp(self) -> None:
        if not self._need_terminal():
            return
        self.emit.insert_path_token(
            self._terminal_pane_id, "/tmp", mode="append",
        )
        self._last_action = "insert_path_token('/tmp', mode='append')"
        self._preview_text = ""

    async def _preview_rm(self) -> None:
        if not self._need_terminal():
            return
        cmd, cwd = await self.emit.request_command_preview(
            self._terminal_pane_id, "rm -rf .git",
        )
        self._last_action = "request_command_preview('rm -rf .git')"
        self._preview_text = f"would run: {cmd}  in: {cwd or '?'}"

    def _open_desktop(self) -> None:
        path = os.path.expanduser("~/Desktop")
        self.emit.open_artifact(path, mode="open_in_pane")
        self._last_action = f"open_artifact({path!r}, 'open_in_pane')"
        self._preview_text = ""

    def _reveal_home(self) -> None:
        path = os.path.expanduser("~")
        self.emit.open_artifact(path, mode="reveal_in_finder")
        self._last_action = f"open_artifact({path!r}, 'reveal_in_finder')"
        self._preview_text = ""


if __name__ == "__main__":
    CanvasTerminalBindingsTestApp().run()
