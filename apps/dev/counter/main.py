"""V3 scaffold counter used by the first-frame scene."""

from plexi_sdk import state
from plexi_sdk.effects import SetState, SetTitle
from plexi_sdk.events import KeyEvent
from plexi_sdk.ui import AppBar, Column, FooterKeys, Text


def init(_size, _args):
    return [SetTitle("Counter"), SetState({"count": 0})]


def update(event):
    if isinstance(event, KeyEvent) and event.pressed and event.key in ("plus", "equals"):
        return [SetState({"count": state.get("count", 0) + 1})]
    return []


def view():
    return Column([
        AppBar("Counter"),
        Text(str(state.get("count", 0)), bold=True),
        FooterKeys([("+", "increment")]),
    ], grow=True)
