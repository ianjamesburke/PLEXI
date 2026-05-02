#!/usr/bin/env python3
"""Input Inspector — POC for issue #331.

Two-panel layout:
  Left  — interactive zone; click, drag, and type here to generate events.
  Right — scrollable event log showing every key, mouse, and scroll event.
"""
from __future__ import annotations

import os
import sys
import time
from collections import deque

_sdk_path = os.path.join(os.path.dirname(__file__), "..", "..", "sdk", "python")
if os.path.isdir(_sdk_path):
    sys.path.insert(0, _sdk_path)

from plexi_sdk import App, RenderContext, BG, SURFACE, HIGHLIGHT, FG, MUTED, ACCENT
from plexi_sdk import BODY, CAPTION, PAD

MAX_EVENTS = 200
MOVE_LOG_MIN_INTERVAL = 1.0 / 30.0
ROW_H = 26.0
HEADER_H = 44.0
FOOTER_H = 32.0
DIVIDER_X_FRAC = 0.4   # left panel takes 40% of width
SCROLL_ID = "event-log"

# Colour-code by event category
CAT_COLOR = {
    "key":        "#cba6f7",   # mauve
    "click":      "#89dceb",   # sky
    "mouse_down": "#f38ba8",   # red
    "mouse_up":   "#a6e3a1",   # green
    "mouse_move": "#fab387",   # peach
    "scroll":     "#f9e2af",   # yellow
}
DEFAULT_COLOR = MUTED


def _ts() -> str:
    return time.strftime("%H:%M:%S")


class InputInspectorApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._events: deque[tuple[str, str, str]] = deque(maxlen=MAX_EVENTS)
        self._scroll_y = 0.0
        self._mouse_pos: tuple[float, float] = (0.0, 0.0)
        self._held_buttons: list[str] = []
        self._last_key = ""
        self._last_move_log_at = 0.0
        self._last_move_buttons: tuple[str, ...] = ()
        # Enable continuous mouse-move delivery
        ctx.set_mouse_tracking(True)

    # ── event handlers ──────────────────────────────────────────────────────

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        mod_parts = [k for k, v in mods.items() if v and (k != "shift" or len(key) > 1)]
        label = "+".join(mod_parts + [key]) if mod_parts else key
        self._last_key = label
        self._push("key", f"key  {label!r}")

    def on_click(self, ctx: RenderContext, x: float, y: float, button: str) -> None:
        self._push("click", f"click  {button}  ({x:.0f}, {y:.0f})")

    def on_mouse_down(self, ctx: RenderContext, x: float, y: float, button: str) -> None:
        if button not in self._held_buttons:
            self._held_buttons.append(button)
        self._push("mouse_down", f"down   {button}  ({x:.0f}, {y:.0f})")

    def on_mouse_up(self, ctx: RenderContext, x: float, y: float, button: str) -> None:
        self._held_buttons = [b for b in self._held_buttons if b != button]
        self._push("mouse_up", f"up     {button}  ({x:.0f}, {y:.0f})")

    def on_mouse_move(self, ctx: RenderContext, x: float, y: float, buttons: list) -> None:
        self._mouse_pos = (x, y)
        now = time.monotonic()
        button_state = tuple(buttons)
        if (
            now - self._last_move_log_at >= MOVE_LOG_MIN_INTERVAL
            or button_state != self._last_move_buttons
        ):
            held = ", ".join(buttons) if buttons else "—"
            self._push("mouse_move", f"move   ({x:.0f}, {y:.0f})  held={held}")
            self._last_move_log_at = now
            self._last_move_buttons = button_state

    def on_scroll(self, ctx: RenderContext, id: str, offset_y: float) -> None:
        if id == SCROLL_ID:
            self._scroll_y = offset_y
        self._push("scroll", f"scroll  id={id!r}  offset={offset_y:.1f}")

    # ── helpers ─────────────────────────────────────────────────────────────

    def _push(self, cat: str, msg: str) -> None:
        if cat in ("mouse_move", "scroll") and self._events and self._events[0][1] == cat:
            self._events[0] = (_ts(), cat, msg)
        else:
            self._events.appendleft((_ts(), cat, msg))

    # ── render ───────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)
        div_x = ctx.w * DIVIDER_X_FRAC

        self._draw_left(ctx, div_x)
        self._draw_right(ctx, div_x)

        # Divider line
        ctx.line(div_x, 0, div_x, ctx.h, color=HIGHLIGHT, width=1.0)

    def _draw_left(self, ctx: RenderContext, div_x: float) -> None:
        # Panel background
        ctx.rect(0, 0, div_x, ctx.h, fill=SURFACE)

        # Title
        ctx.text(div_x / 2, 22, "Interactive Zone",
                 size=BODY, color=FG, align="center")
        ctx.line(0, HEADER_H - 1, div_x, HEADER_H - 1, color=HIGHLIGHT, width=0.5)

        cx = div_x / 2
        base_y = HEADER_H + 24.0

        # Mouse position
        mx, my = self._mouse_pos
        ctx.text(cx, base_y, f"mouse  ({mx:.0f}, {my:.0f})",
                 size=CAPTION, color=MUTED, align="center")

        # Held buttons
        held_label = "  ".join(self._held_buttons) if self._held_buttons else "—"
        ctx.text(cx, base_y + 22, f"held  {held_label}",
                 size=CAPTION, color=ACCENT, align="center", monospace=True)

        # Last key
        ctx.text(cx, base_y + 48, f"last key  {self._last_key or '—'}",
                 size=CAPTION, color=CAT_COLOR["key"], align="center", monospace=True)

        # Big crosshair dot at mouse pos (clamped to left panel)
        dot_x = min(mx, div_x - 8)
        dot_y = my
        if dot_y > HEADER_H:
            ctx.circle(dot_x, dot_y, 5.0, fill=ACCENT)

        # Instruction hint at bottom
        hint_y = ctx.h - FOOTER_H / 2
        ctx.text(cx, hint_y,
                 "click · drag · type · scroll",
                 size=CAPTION, color=MUTED, align="center")

    def _draw_right(self, ctx: RenderContext, div_x: float) -> None:
        panel_w = ctx.w - div_x

        # Header
        ctx.rect(div_x, 0, panel_w, HEADER_H, fill=SURFACE)
        count = len(self._events)
        ctx.text(div_x + PAD, 22, f"Event Log  ({count})",
                 size=BODY, color=FG, align="left_center")
        ctx.text(ctx.w - PAD, 22, "newest first",
                 size=CAPTION, color=MUTED, align="right_center")
        ctx.line(div_x, HEADER_H - 1, ctx.w, HEADER_H - 1, color=HIGHLIGHT, width=0.5)

        viewport_y = HEADER_H
        viewport_h = ctx.h - HEADER_H
        content_h = max(viewport_h + ROW_H, count * ROW_H)

        ctx.begin_scroll(SCROLL_ID, 0, viewport_y, ctx.w, viewport_h,
                         content_height=content_h)

        first = max(0, int(self._scroll_y / ROW_H))
        last = min(count, int((self._scroll_y + viewport_h) / ROW_H) + 2)
        events = list(self._events)

        for i in range(first, last):
            if i >= len(events):
                break
            ts, cat, msg = events[i]
            row_y = viewport_y + i * ROW_H - self._scroll_y
            bg = SURFACE if i % 2 == 0 else BG
            ctx.rect(div_x, row_y, panel_w, ROW_H, fill=bg)

            color = CAT_COLOR.get(cat, DEFAULT_COLOR)
            ctx.text_row(
                div_x + PAD, row_y + ROW_H / 2,
                items=[
                    {"text": ts, "color": MUTED, "size": CAPTION, "monospace": True},
                    {"text": msg, "color": color, "size": CAPTION, "monospace": True},
                ],
                gap=12.0,
                align="left_center",
            )

            ctx.line(div_x, row_y + ROW_H - 1, ctx.w, row_y + ROW_H - 1,
                     color=HIGHLIGHT, width=0.5)

        ctx.end_scroll()


if __name__ == "__main__":
    InputInspectorApp().run()
