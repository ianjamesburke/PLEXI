"""Key Event Probe - HostHarness KeyPane fixture (stint 0426).

A `+`/`-`/`r` keyboard-only counter — no click/pointer input — so a test
that only exercises `AppRequest::KeyPane` (`plexi pane key <pane_id> <key>`)
proves KeyEvent dispatch reaches a real WASM Python pane end to end: press
"+" via the production key-injection path and observe the guest's
re-rendered `Text`.
"""

from plexi_sdk import state
from plexi_sdk.effects import SetState, SetTitle
from plexi_sdk.events import KeyEvent
from plexi_sdk.ui import AppBar, Column, FooterKeys, Text


def init(size, args):
    return [
        SetTitle("Key Event Probe"),
        SetState({"count": 0}),
    ]


def update(event):
    if isinstance(event, KeyEvent) and event.pressed:
        if event.key in ("equals", "plus"):
            return [SetState({"count": state.get("count", 0) + 1})]
        if event.key == "minus":
            return [SetState({"count": state.get("count", 0) - 1})]
        if event.key == "r":
            return [SetState({"count": 0})]
    return []


def view():
    count = state.get("count", 0)
    return Column([
        AppBar("Key Event Probe"),
        Text(str(count), bold=True),
        FooterKeys([
            ("+", "increment"),
            ("-", "decrement"),
            ("r", "reset"),
        ]),
    ], grow=True)
