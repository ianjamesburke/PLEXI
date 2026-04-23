#!/usr/bin/env python3
"""Notification Tester — proof-of-concept playground for the notification
system, rebuilt on SDK v2 declarative UI.

Press:
  m — fire a plain message notification
  c — fire a choice notification (blocks for response)
  i — fire an input notification (blocks for text)
  b — queue a burst (3 messages back-to-back, so you can cycle with Cmd+]/Cmd+[)
  r — fire a required message (cannot be dismissed with Esc)
"""
from __future__ import annotations

import threading
import time

from plexi_sdk import App, RenderContext
from plexi_sdk.ui import (
    Column, Card, Header, Section,
    KeyRow, ScrollLog, Spacer, Footer,
)


class NotificationTesterApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._log: list[str] = []
        self._burst_counter = 0
        ctx.status_summary("Notification Tester — ready")
        self.emit.info("Notification Tester started")

    def _append(self, line: str) -> None:
        ts = time.strftime("%H:%M:%S")
        self._log.append(f"{ts}  {line}")
        self.emit.schedule_render(after_ms=50)

    # ── Fire helpers — blocking calls run on a background thread so the
    #    render/input thread stays responsive while the modal is up.
    def _fire_message(self) -> None:
        self.emit.notify(
            title="Plain message",
            body="This is a message-kind notification. Enter or Space to acknowledge.",
            level="info",
        )
        self._append("sent: message (fire-and-forget)")

    def _fire_required_message(self) -> None:
        def runner():
            result = self.emit.notify_and_wait(
                title="Required message",
                body="You cannot dismiss this with Esc. Press Enter to acknowledge.",
                level="warn",
            )
            self._append(f"required message → {result}")
        threading.Thread(target=runner, daemon=True).start()
        self._append("sent: required message")

    def _fire_choice(self) -> None:
        def runner():
            options = [
                {"label": "Yes", "value": "yes", "shortcut": "y"},
                {"label": "No", "value": "no", "shortcut": "n"},
                {"label": "Maybe", "value": "maybe", "shortcut": "m"},
            ]
            result = self.emit.notify_choice(
                title="How's your focus right now?",
                body="Pick one. Use y/n/m or arrow keys + Enter, or 1-3 to direct-select.",
                options=options,
                level="info",
            )
            self._append(f"choice → {result}")
        threading.Thread(target=runner, daemon=True).start()
        self._append("sent: choice")

    def _fire_input(self) -> None:
        def runner():
            result = self.emit.notify_input(
                title="What are you working on?",
                prompt="Type a short description…",
                body="Enter for newline, Cmd+Enter to submit, Esc to cancel.",
                level="info",
            )
            if result == "__cancel__":
                self._append("input → cancelled")
            else:
                self._append(f"input → {result!r}")
        threading.Thread(target=runner, daemon=True).start()
        self._append("sent: input")

    def _fire_burst(self) -> None:
        self._burst_counter += 1
        tag = self._burst_counter
        for i in range(1, 4):
            self.emit.notify(
                title=f"Burst {tag} — message {i}/3",
                body="Use ⌘] / ⌘[ to cycle without acknowledging. "
                     "Enter acknowledges and moves on.",
                level="info",
            )
        self._append(f"sent: burst {tag} (3 messages)")

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        k = key.lower()
        if k == "m":
            self._fire_message()
        elif k == "c":
            self._fire_choice()
        elif k == "i":
            self._fire_input()
        elif k == "b":
            self._fire_burst()
        elif k == "r":
            self._fire_required_message()

    def on_render(self, ctx: RenderContext) -> None:
        ctx.render(Column([
            Header(
                title="Notification Tester",
                subtitle="Fire each Notify kind on demand",
            ),
            Card([
                KeyRow("m", "Message (fire-and-forget)"),
                KeyRow("c", "Choice (blocks for pick)"),
                KeyRow("i", "Input (blocks for text)"),
                KeyRow("b", "Burst — 3 messages at once"),
                KeyRow("r", "Required message (no Esc)"),
            ]),
            Section("Event log"),
            ScrollLog(lines=self._log, empty_text="no events yet"),
            Spacer(grow=True),
            Footer(
                "Modal opens automatically on send. "
                "⌘⇧A re-opens. ⌘] / ⌘[ cycle the queue."
            ),
        ]))


NotificationTesterApp().run()
