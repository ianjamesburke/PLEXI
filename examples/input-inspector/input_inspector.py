#!/usr/bin/env python3
"""Input Inspector — POC for issue #331.

Two-panel layout:
  Left  — interactive zone; click, drag, and type here to generate events.
  Right — scrollable event log showing every key, mouse, and scroll event.

MIDI support requires the `midi.in` capability (declared in manifest.toml).
Network MIDI / RTP-MIDI is out of scope — only IAC Driver and USB/hardware
endpoints are supported by the CoreMIDI layer.
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
from plexi_sdk import BODY, CAPTION, PAD, CapabilityDeniedError
from plexi_sdk import midi as midi_sdk

MAX_EVENTS = 200
MOVE_LOG_MIN_INTERVAL = 1.0 / 30.0
SCROLL_LOG_MIN_INTERVAL = 1.0 / 20.0
ROW_H = 26.0
HEADER_H = 44.0
FOOTER_H = 32.0
DIVIDER_X_FRAC = 0.4   # left panel takes 40% of width
INPUT_SCROLL_ID = "interactive-zone-scroll"
MIDI_PIPE_ID = "inspector-midi-in"

# Colour-code by event category
CAT_COLOR = {
    "key":        "#cba6f7",   # mauve
    "click":      "#89dceb",   # sky
    "mouse_down": "#f38ba8",   # red
    "mouse_up":   "#a6e3a1",   # green
    "mouse_move": "#fab387",   # peach
    "scroll":     "#f9e2af",   # yellow
    "midi":       "#94e2d5",   # teal
}
DEFAULT_COLOR = MUTED


def _ts() -> str:
    return time.strftime("%H:%M:%S")


class InputInspectorApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._events: deque[tuple[str, str, str]] = deque(maxlen=MAX_EVENTS)
        self._input_scroll_offset = 0.0
        self._mouse_pos: tuple[float, float] = (0.0, 0.0)
        self._held_buttons: list[str] = []
        self._last_key = ""
        self._last_move_log_at = 0.0
        self._last_scroll_log_at = 0.0
        self._last_move_buttons: tuple[str, ...] = ()
        self._show_inputs_page = False
        self._enabled_categories = {
            "key": True,
            "click": True,
            "mouse_down": True,
            "mouse_up": True,
            "mouse_move": True,
            "scroll": True,
            "midi": True,
        }
        # MIDI state
        self._midi_inputs: list = []     # list of MidiPortInfo
        self._midi_pipe = None           # Pipe | None
        self._midi_open_port_id: str = ""
        self._midi_error: str = ""
        # Enable continuous mouse-move delivery
        ctx.set_mouse_tracking(True)
        ctx.info("input-inspector: on_init")

    # ── event handlers ──────────────────────────────────────────────────────

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        key_lower = key.lower()
        if key_lower == "i":
            self._show_inputs_page = not self._show_inputs_page
            return
        if key_lower == "m":
            self._refresh_midi_ports()
            return
        if key_lower == "x" and self._midi_open_port_id:
            self._close_midi_input()
            return
        # Digit keys on inputs page open a MIDI port by index.
        if self._show_inputs_page and key_lower in "0123456789":
            idx = int(key_lower)
            if idx < len(self._midi_inputs):
                port = self._midi_inputs[idx]
                if self._midi_open_port_id == port.id:
                    self._close_midi_input()
                else:
                    self._open_midi_input(port.id)
            return
        if key_lower in ("1", "2", "3", "4", "5", "6", "7"):
            mapping = {
                "1": "key",
                "2": "click",
                "3": "mouse_down",
                "4": "mouse_up",
                "5": "mouse_move",
                "6": "scroll",
                "7": "midi",
            }
            cat = mapping[key_lower]
            self._enabled_categories[cat] = not self._enabled_categories[cat]
            return

        mod_parts = [k for k, v in mods.items() if v and (k != "shift" or len(key) > 1)]
        label = "+".join(mod_parts + [key]) if mod_parts else key
        self._last_key = label
        self._push("key", f"key  {label!r}")

    def on_midi_input_opened(
        self, pipe_id: str, port_id: str, port_name: str
    ) -> None:
        self.emit.info(f"midi.input opened: port_id={port_id} port_name={port_name!r}")
        self._midi_open_port_id = port_id
        self._midi_error = ""

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
        if id != INPUT_SCROLL_ID:
            return
        delta = offset_y - self._input_scroll_offset
        self._input_scroll_offset = offset_y
        if abs(delta) < 0.01:
            return
        now = time.monotonic()
        if now - self._last_scroll_log_at >= SCROLL_LOG_MIN_INTERVAL:
            direction = "down" if delta > 0 else "up"
            self._push("scroll", f"scroll  zone=interactive  dir={direction}  delta={delta:.1f}")
            self._last_scroll_log_at = now

    # ── MIDI helpers ─────────────────────────────────────────────────────────

    def _refresh_midi_ports(self) -> None:
        try:
            result = self.emit.run_sync(self.emit.list_midi_devices())
            self._midi_inputs = result.inputs
            self._midi_error = ""
        except CapabilityDeniedError as e:
            self._midi_error = f"capability denied: {e}"
        except Exception as e:
            self._midi_error = f"list_midi_devices error: {e}"

    def _open_midi_input(self, port_id: str) -> None:
        if self._midi_open_port_id:
            self._close_midi_input()
        try:
            self._midi_pipe = self.emit.open_midi_input(port_id, MIDI_PIPE_ID)
            # _midi_open_port_id is set in on_midi_input_opened
        except CapabilityDeniedError as e:
            self._midi_error = f"capability denied: {e}"

    def _close_midi_input(self) -> None:
        if not self._midi_open_port_id:
            return
        self.emit.close_midi_input(self._midi_open_port_id)
        self._midi_open_port_id = ""
        self._midi_pipe = None

    def _drain_midi_pipe(self) -> None:
        if self._midi_pipe is None:
            return
        while True:
            frame = self._midi_pipe.read_frame()
            if frame is None:
                break
            self._push("midi", f"midi  {midi_sdk.describe(frame)}")

    # ── helpers ─────────────────────────────────────────────────────────────

    def _push(self, cat: str, msg: str) -> None:
        if not self._enabled_categories.get(cat, True):
            return
        if cat in ("mouse_move", "scroll") and self._events and self._events[0][1] == cat:
            self._events[0] = (_ts(), cat, msg)
        else:
            self._events.appendleft((_ts(), cat, msg))

    # ── render ───────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        self._drain_midi_pipe()
        ctx.clear(BG)
        if self._show_inputs_page:
            self._draw_inputs_page(ctx)
            return
        div_x = ctx.w * DIVIDER_X_FRAC

        self._draw_left(ctx, div_x)
        self._draw_right(ctx, div_x)

        # Divider line
        ctx.line(div_x, 0, div_x, ctx.h, color=HIGHLIGHT, width=1.0)

    def _draw_left(self, ctx: RenderContext, div_x: float) -> None:
        # Panel background
        ctx.rect(0, 0, div_x, ctx.h, fill=SURFACE)
        # Register scroll capture region in the interactive zone only.
        zone_y = HEADER_H
        zone_h = max(1.0, ctx.h - HEADER_H)
        ctx.begin_scroll(
            INPUT_SCROLL_ID,
            0,
            zone_y,
            div_x,
            zone_h,
            content_height=zone_h + 6000.0,
        )

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

        # MIDI status badge
        if self._midi_open_port_id:
            ctx.text(cx, base_y + 74, f"midi  {self._midi_open_port_id}",
                     size=CAPTION, color=CAT_COLOR["midi"], align="center", monospace=True)

        # Big crosshair dot at mouse pos (clamped to left panel)
        dot_x = min(mx, div_x - 8)
        dot_y = my
        if dot_y > HEADER_H:
            ctx.circle(dot_x, dot_y, 5.0, fill=ACCENT)

        # Instruction hint at bottom
        hint_y = ctx.h - FOOTER_H / 2
        ctx.text(cx, hint_y,
                 "click · drag · type · scroll · i:inputs · m:midi",
                 size=CAPTION, color=MUTED, align="center")
        ctx.end_scroll()

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

        events = list(self._events)
        viewport_y = HEADER_H
        viewport_h = ctx.h - HEADER_H
        visible_rows = max(1, int(viewport_h / ROW_H))
        display_count = min(count, visible_rows)

        for i in range(display_count):
            ts, cat, msg = events[i]
            row_y = viewport_y + i * ROW_H
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

    def _draw_inputs_page(self, ctx: RenderContext) -> None:
        ctx.rect(0, 0, ctx.w, ctx.h, fill=SURFACE)
        ctx.text(PAD, 22, "Input Sources", size=BODY, color=FG, align="left_center")
        ctx.text(
            ctx.w - PAD,
            22,
            "i:back  1-7:toggle",
            size=CAPTION,
            color=MUTED,
            align="right_center",
            monospace=True,
        )
        ctx.line(0, HEADER_H - 1, ctx.w, HEADER_H - 1, color=HIGHLIGHT, width=0.5)

        rows = [
            ("1", "keyboard", "key"),
            ("2", "mouse click", "click"),
            ("3", "mouse down", "mouse_down"),
            ("4", "mouse up", "mouse_up"),
            ("5", "mouse move", "mouse_move"),
            ("6", "scroll events", "scroll"),
            ("7", "midi events", "midi"),
        ]
        y = HEADER_H + 16.0
        for key, label, cat in rows:
            enabled = self._enabled_categories.get(cat, True)
            status = "ON " if enabled else "OFF"
            color = ACCENT if enabled else MUTED
            ctx.text_row(
                PAD,
                y,
                items=[
                    {"text": f"[{key}]", "color": MUTED, "size": CAPTION, "monospace": True},
                    {"text": f"{label:<14}", "color": FG, "size": CAPTION, "monospace": True},
                    {"text": status, "color": color, "size": CAPTION, "monospace": True},
                ],
                gap=12.0,
                align="left_top",
            )
            y += 24.0

        # MIDI port section
        y += 12.0
        ctx.line(PAD, y, ctx.w - PAD, y, color=HIGHLIGHT, width=0.5)
        y += 12.0
        ctx.text(PAD, y, "MIDI Inputs", size=CAPTION, color=FG, align="left_top")
        hint = "m:refresh"
        if self._midi_open_port_id:
            hint += "  x:close"
        ctx.text(ctx.w - PAD, y, hint, size=CAPTION, color=MUTED, align="right_top", monospace=True)
        y += 20.0

        if self._midi_error:
            ctx.text(PAD, y, self._midi_error, size=CAPTION, color="#f38ba8", align="left_top",
                     max_width=ctx.w - PAD * 2)
            y += 20.0
        elif not self._midi_inputs:
            ctx.text(PAD, y, "press m to enumerate ports",
                     size=CAPTION, color=MUTED, align="left_top")
            y += 20.0
        else:
            for idx, port in enumerate(self._midi_inputs):
                is_open = port.id == self._midi_open_port_id
                badge = "● OPEN" if is_open else f"[{idx}] open"
                badge_color = CAT_COLOR["midi"] if is_open else MUTED
                ctx.text_row(
                    PAD,
                    y,
                    items=[
                        {"text": f"{idx}:", "color": MUTED, "size": CAPTION, "monospace": True},
                        {"text": port.name, "color": FG, "size": CAPTION, "monospace": True},
                        {"text": badge, "color": badge_color, "size": CAPTION, "monospace": True},
                    ],
                    gap=10.0,
                    align="left_top",
                )
                y += 22.0

        y += 8.0
        ctx.text(
            PAD,
            y,
            "Note: Network MIDI (RTP-MIDI) is not supported.",
            size=CAPTION,
            color=MUTED,
            align="left_top",
            max_width=ctx.w - PAD * 2,
        )


if __name__ == "__main__":
    InputInspectorApp().run()
