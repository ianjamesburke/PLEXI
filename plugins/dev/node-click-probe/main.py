"""Node Click Probe - HostHarness node-targeted click fixture (stint 0414).

A single `Button` mutates state entirely through a node-targeted click
(`AppRequest::ClickPaneNode`, `plexi pane click <pane_id> --node <node_id>`)
— no keyboard input — so a test that only exercises node-targeted clicks
proves the primitive end to end: resolve the button's `node_id` from
`plexi pane state`, click it, and observe the guest's re-rendered `Text`.
"""

from plexi_sdk import state
from plexi_sdk.effects import SetState, SetTitle
from plexi_sdk.events import UiAction
from plexi_sdk.ui import Button, Column, Text

INCREMENT = "increment"


def init(_size, _args):
    return [
        SetTitle("Node Click Probe"),
        SetState({"count": state.get("count", 0)}),
    ]


def update(event):
    if isinstance(event, UiAction) and event.handler_id == INCREMENT:
        return [SetState({"count": state.get("count", 0) + 1})]
    return []


def view():
    count = state.get("count", 0)
    return Column([
        Text(str(count), bold=True),
        Button("Increment", INCREMENT),
    ], grow=True)
