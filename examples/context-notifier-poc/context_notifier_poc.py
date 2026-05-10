#!/usr/bin/env python3
"""POC for #879: parks immediately, fires a notification every 5s.

Test procedure:
1. Open this app in context 1. Its pane shows "Parked — notifying every 5s".
2. Close the pane to park it.
3. Switch to a different context (e.g. context 3).
4. Within 5s, a notification badge should appear on context 1 — not context 3.

Before the fix, the badge always appeared on context 0.
"""

from plexi_sdk import App, RenderContext, PRIORITY_HIGH
from plexi_sdk.ui import Column, AppBar, Label, Spacer  # noqa: F401 (used in on_render)

TIMER_ID = "notify-tick"
INTERVAL_MS = 5_000


class ContextNotifierApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._count = 0
        ctx.set_timer(TIMER_ID, INTERVAL_MS)
        self.emit.info("context-notifier-poc started")

    def on_timer(self, ctx: RenderContext, timer_id: str) -> None:
        if timer_id != TIMER_ID:
            return
        self._count += 1
        ctx.notify(
            title="Context Notifier",
            body=f"#{self._count} — badge should be on the context where this app was parked",
            level="info",
            priority=PRIORITY_HIGH,
        )
        self.emit.info(f"context-notifier-poc fired notification #{self._count}")
        ctx.set_timer(TIMER_ID, INTERVAL_MS)
        self.emit.schedule_render(after_ms=100)

    def on_render(self, ctx: RenderContext) -> None:
        ctx.render(Column([
            AppBar(title="Context Notifier (POC)"),
            Spacer(),
            Label("Fires a notification every 5 seconds."),
            Label("Close this pane to park the app, then switch contexts."),
            Label(f"Notifications sent: {self._count}"),
            Spacer(),
        ]))


ContextNotifierApp().run()
