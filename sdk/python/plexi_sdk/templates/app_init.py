from plexi_sdk import state
from plexi_sdk.effects import SetState, SetTitle
from plexi_sdk.events import KeyEvent
from plexi_sdk.ui import AppBar, Column, FooterKeys, Text


def init(size, args):
    return [
        SetTitle("__DISPLAY_NAME__"),
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
        AppBar("__DISPLAY_NAME__"),
        Text(str(count), bold=True),
        FooterKeys([
            ("+", "increment"),
            ("-", "decrement"),
            ("r", "reset"),
        ]),
    ], grow=True)
