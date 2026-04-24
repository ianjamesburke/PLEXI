#!/usr/bin/env python3
"""UI Playground — a reference app that renders every SDK v2 component.

Use this as a visual spec of what clean Plexi UIs look like. Resize the pane
to exercise the responsive behavior (Footer wraps, Heading truncates,
ScrollLog shows only what fits).

Keys:
  l — append a line to the scroll log (for watching it grow)
  c — clear the log
"""
from __future__ import annotations

import time

from plexi_sdk import App, RenderContext
from plexi_sdk.ui import (
    Column, Card, AppBar, Section, Divider,
    Heading, Label, KeyRow, ScrollLog, Spacer, Footer,
    ACCENT, GREEN, RED,
)


class UIPlaygroundApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._log: list[str] = [f"boot @ {time.strftime('%H:%M:%S')}"]
        ctx.status_summary("UI Playground — resize me")
        self.emit.info("UI Playground started")

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        k = key.lower()
        if k == "l":
            self._log.append(f"{time.strftime('%H:%M:%S')}  appended line #{len(self._log) + 1}")
            self.emit.schedule_render(after_ms=50)
        elif k == "c":
            self._log.clear()
            self.emit.schedule_render(after_ms=50)

    def on_render(self, ctx: RenderContext) -> None:
        ctx.render(Column([
            # ── Header ────────────────────────────────────────────────────
            AppBar(
                title="UI Playground", stacked top-to-bottom.",
            ),

            # ── Typography ────────────────────────────────────────────────
            Section("Typography"),
            Heading("Heading level 1 — biggest", level=1),
            Heading("Heading level 2 — medium", level=2),
            Heading("Heading level 3 — small", level=3),
            Label("Body text. The quick brown fox jumps over the lazy dog. "
                  "Long copy automatically wraps across up to three lines "
                  "and then truncates with an ellipsis if it still doesn't "
                  "fit inside the available width of this label.",
                  tone="body"),
            Label("Caption text — smaller, for secondary info.", tone="caption"),
            Label("Hint text — smallest, dimmed.", tone="hint"),

            # ── Cards + KeyRows ───────────────────────────────────────────
            Section("Card + KeyRow"),
            Card([
                KeyRow("l", "Append a log line"),
                KeyRow("c", "Clear the log"),
                KeyRow("⌘K", "Example chord shortcut"),
                KeyRow("⎋", "Escape — example single-glyph key"),
            ]),

            # ── Status colors ─────────────────────────────────────────────
            Section("Status colors via Label.color"),
            Card([
                Label("OK — all systems nominal.", tone="body",
                      color=GREEN, bold=True),
                Label("Working — in progress.", tone="body", color=ACCENT),
                Label("Error — needs attention.", tone="body", color=RED),
            ]),

            # ── Divider ───────────────────────────────────────────────────
            Divider(),

            # ── Scrolling log ─────────────────────────────────────────────
            Section("ScrollLog"),
            Label("Press `l` to append a line. The log shows newest on top "
                  "and only as many lines as fit in the available space.",
                  tone="caption"),
            ScrollLog(lines=self._log),

            # ── Footer pinned to bottom ───────────────────────────────────
            Spacer(grow=True),
            Footer(
                "Spacer(grow=True) pushes this footer to the bottom. "
                "Footer text wraps across up to two lines instead of "
                "clipping when the pane is narrow."
            ),
        ]))


UIPlaygroundApp().run()
