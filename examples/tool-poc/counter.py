#!/usr/bin/env python3
from __future__ import annotations

# Counter app — exposes `increment` and `reset` as AI-callable tools (#516).
#
# Demo: open this pane alongside chat-poc. Ask the AI "increment the counter
# 3 times" or "reset the counter". The host broker routes the tool calls here
# via PlexiEvent::ToolCall; the SDK dispatches to the registered handlers and
# replies with DrawCommand::ToolResult.
#
# This app does NOT require ai.query — it is a tool provider, not a consumer.

import sys
sys.path.insert(0, "../..")

from plexi_sdk import App, RenderContext


class CounterApp(App):
    def __init__(self) -> None:
        super().__init__()
        self._count: int = 0

        # Register tools using the @app.tool decorator.
        # expose_tools is called automatically on registration.

        @self.tool(
            "increment",
            description="Increment the counter by n (default 1).",
            schema={
                "type": "object",
                "properties": {
                    "n": {
                        "type": "integer",
                        "description": "Amount to increment by (default 1).",
                    }
                },
            },
        )
        def handle_increment(args: dict) -> dict:
            n = int(args.get("n") or 1)
            self._count += n
            self.emit.info(f"tool: increment by {n} → count={self._count}")
            return {"count": self._count, "delta": n}

        @self.tool(
            "reset",
            description="Reset the counter to zero.",
            schema={"type": "object", "properties": {}},
        )
        def handle_reset(args: dict) -> dict:  # noqa: ARG001
            self._count = 0
            self.emit.info("tool: reset → count=0")
            return {"count": 0}

    def on_render(self, ctx: RenderContext) -> None:
        bg = "#1e1e2e"
        fg = "#cdd6f4"
        accent = "#89b4fa"
        dim = "#6c7086"

        ctx.clear(bg)

        # Title
        ctx.text(ctx.w / 2 - 80, 30, "Counter (Tool POC)", size=18.0, color=fg, bold=True)

        # Count display
        ctx.text(ctx.w / 2 - 60, 100, f"Count: {self._count}", size=32.0, color=accent, bold=True)

        # Instructions
        ctx.text(20, ctx.h - 100, "Open chat-poc in another pane.", size=13.0, color=dim)
        ctx.text(20, ctx.h - 78, 'Ask: "increment the counter 3 times"', size=13.0, color=dim)
        ctx.text(20, ctx.h - 56, 'or: "reset the counter"', size=13.0, color=dim)
        ctx.text(20, ctx.h - 30, "Keys: [i]/[+] increment  [r] reset", size=13.0, color=dim)

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if key in ("i", "+"):
            self._count += 1
        elif key == "r":
            self._count = 0


if __name__ == "__main__":
    CounterApp().run()
