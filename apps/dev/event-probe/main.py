#!/usr/bin/env python3
"""Event Probe — event-bus fixture and POC (stints 0214, 0511).

Declares one stream (``probe.tick``) and emits a deterministic event on the
``e`` key or the Emit button. Used to validate every event transport:

  - CLI:    ``plexi events subscribe event-probe probe.tick`` (NDJSON stream)
  - Scenes: ``expect = { event_stream = "probe.tick", ... }``
  - Rust:   ``HostHarness::wait_for_app_event``

Each tick increments a counter and emits::

    {"event": "probe.tick", "summary": "Probe tick N",
     "resource_id": "probe-session", "revision_after": "tick-N",
     "payload": {"count": N}}
"""

from __future__ import annotations

import json

from plexi_sdk import effects, events, state

STREAM = "probe.tick"
RESOURCE = "probe-session"


def init(_size, _args) -> list:
    # Declare the stream before emitting on it — the host rejects events on
    # undeclared streams.
    return [
        effects.SetTitle("Event Probe"),
        effects.SetState({"count": 0}),
        effects.DeclareEventStreams([
            effects.EventStreamDecl(
                name=STREAM,
                schema_json=json.dumps({
                    "type": "object",
                    "properties": {"count": {"type": "integer"}},
                    "required": ["count"],
                }),
                description="Fires once per probe tick with the running count.",
            )
        ]),
    ]


def _emit_tick() -> list:
    count = state.get("count", 0) + 1
    return [
        effects.SetState({"count": count}),
        effects.EmitEvent(
            event=STREAM,
            actor="user",
            summary=f"Probe tick {count}",
            resource_id=RESOURCE,
            resource_scope="document",
            revision_after=f"tick-{count}",
            payload_json=json.dumps({"count": count}),
        ),
        effects.SetStatus(f"emitted tick {count}"),
    ]


def update(event) -> list:
    # "enter" included so scene `key` steps (named-key delivery) can drive it.
    if isinstance(event, events.KeyEvent) and event.pressed and event.key in ("e", "enter"):
        return _emit_tick()
    if isinstance(event, events.UiAction) and event.handler_id == "emit":
        return _emit_tick()
    return []


def view():
    count = state.get("count", 0)
    return {
        "type": "column",
        "children": [
            {"type": "app_bar", "title": "Event Probe"},
            {"type": "text", "text": f"ticks emitted: {count}", "bold": True},
            {"type": "text", "text": f"stream: {STREAM}  ·  resource: {RESOURCE}"},
            {"type": "button", "label": "Emit tick (e)", "on_click": "emit"},
        ],
    }
