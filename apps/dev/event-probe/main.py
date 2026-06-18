#!/usr/bin/env python3
"""Event Probe — POC companion for host event subscriptions (stint 0214).

Declares one stream (``probe.tick``) and emits a deterministic event on a
button press or the ``e`` key. Used to manually validate both agent-facing
transports:

  - CLI:  ``plexi events subscribe event-probe probe.tick`` (NDJSON stream)
  - MCP:  the host event MCP server's ``subscribe_and_wait`` tool

Each tick increments a counter and emits::

    {"event": "probe.tick", "summary": "Probe tick N",
     "resource_id": "probe-session", "revision_after": "tick-N",
     "payload": {"count": N}}
"""
from plexi_sdk import App
from plexi_sdk.ui import AppBar, ButtonRow, Column, FooterKeys, Label, Spacer

STREAM = "probe.tick"
RESOURCE = "probe-session"


class EventProbe(App):
    def on_init(self) -> None:
        self.count: int = self.state.get("count", 0)
        # Declare the stream before emitting on it — the host rejects events
        # on undeclared streams.
        self.emit.declare_event_streams([
            {
                "name": STREAM,
                "schema": {
                    "type": "object",
                    "properties": {"count": {"type": "integer"}},
                    "required": ["count"],
                },
                "description": "Fires once per probe tick with the running count.",
            }
        ])
        self.emit.info(f"event-probe ready (count={self.count})")

    def view(self):
        return Column([
            AppBar(title="Event Probe"),
            Label(f"ticks emitted: {self.count}", bold=True),
            Label(f"stream: {STREAM}  ·  resource: {RESOURCE}"),
            Spacer(grow=True),
            ButtonRow("emit", "Emit tick (e)"),
            FooterKeys(shortcuts=[("e", "emit tick")]),
        ])

    def emit_tick(self) -> None:
        self.count += 1
        self.emit.emit_event(
            event=STREAM,
            actor="user",
            summary=f"Probe tick {self.count}",
            resource_id=RESOURCE,
            resource_scope="document",
            revision_after=f"tick-{self.count}",
            payload={"count": self.count},
            suggested_trigger="conversation",
        )
        self.emit.info(f"emitted {STREAM} count={self.count}")
        self.state.save({"count": self.count})
        self.emit.schedule_render()

    def on_component_event(self, node_id: str, event_type: str, payload: dict) -> None:
        if node_id == "emit" and event_type == "click":
            self.emit_tick()

    def on_key(self, key: str, mods: dict) -> None:
        if key == "e":
            self.emit_tick()


EventProbe().run()
