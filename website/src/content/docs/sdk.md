---
title: "SDK Overview"
description: "Python SDK v2 for building Plexi pane apps"
verified_version: "0.0.669"
---

Python SDK v2 for Plexi PGAP apps.

Normal apps implement ``view()`` and return a component tree:

    from plexi_sdk import App
    from plexi_sdk.ui import AppBar, Column, FooterKeys, Label, Spacer

    class CounterApp(App):
        def on_init(self) -> None:
            self.count = self.state.get("count", 0)

        def view(self):
            return Column([
                AppBar("Counter"),
                Spacer(grow=True),
                Label(str(self.count), bold=True),
                Spacer(grow=True),
                FooterKeys([("+", "increment"), ("-", "decrement")]),
            ])

        def on_key(self, key: str, mods: dict) -> None:
            if key in ("plus", "equals"):
                self.count += 1
            elif key == "minus":
                self.count -= 1
            self.state.save({"count": self.count})
            self.emit.schedule_render()

    CounterApp().run()

Use ``on_render(ctx)`` only for games, animations, realtime visualizations, or
other pixel-control apps. Never override both ``view()`` and ``on_render(ctx)``.

Host-brokered actions live on ``self.emit``:

    await self.emit.http_get(url)       # requires net.http
    await self.emit.secret_get("KEY")   # requires secrets.get
    await self.emit.ai_query("low", system, messages)  # requires ai.query

Capabilities gate PGAP host APIs. Python apps are native subprocesses, not a
process sandbox.
