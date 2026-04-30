#!/usr/bin/env python3
"""Lifecycle Tester — triggers crash / hang / malformed JSON / clean exit.

POC for the observable app lifecycle pill (#316).

Keys:
  c — crash now (raise RuntimeError → red pill + traceback overlay)
  h — hang now  (infinite loop → red pill after host timeout)
  j — spam malformed JSON (protocol_error pill)
  x — exit cleanly (no pill)
"""

import sys
import time

from plexi_sdk import App, RenderContext
from plexi_sdk.ui import (
    Column, Card, Section, KeyRow, ScrollLog, AppBar, Spacer,
)


class LifecycleTesterApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._log: list[str] = []
        ctx.status_summary("Lifecycle Tester — ready")

    def on_render(self, ctx: RenderContext) -> None:
        ctx.render(Column([
            AppBar(title="Lifecycle Tester"),
            Section("Triggers"),
            Card([
                KeyRow("c", "Crash — raise RuntimeError (→ red pill + traceback)"),
                KeyRow("h", "Hang — infinite loop (→ red pill after host timeout)"),
                KeyRow("j", "Malformed JSON — spam invalid lines (→ protocol_error pill)"),
                KeyRow("x", "Clean exit — sys.exit(0) (no pill)"),
            ]),
            Section("Event log"),
            ScrollLog(lines=self._log, empty_text="no events yet"),
            Spacer(grow=True),
        ]))

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        if key == "c":
            self._log.append("→ raising RuntimeError…")
            raise RuntimeError("lifecycle-tester: intentional crash (key c)")

        elif key == "h":
            self._log.append("→ entering infinite loop (hang)…")
            while True:
                time.sleep(1)

        elif key == "j":
            self._log.append("→ spamming 10 invalid JSON lines…")
            for _ in range(10):
                sys.stdout.write("this is not json\n")
                sys.stdout.flush()
            self._log.append("  done — check for protocol_error pill")

        elif key == "x":
            self._log.append("→ sys.exit(0) — should exit cleanly, no pill")
            sys.exit(0)


LifecycleTesterApp().run()
