"""Prove printable keys arrive as V3 ``KeyEvent`` values."""

from plexi_sdk import state
from plexi_sdk.effects import SetState, SetTitle
from plexi_sdk.events import KeyEvent
from plexi_sdk.ui import AppBar, Column, FooterKeys, Text


def init(_size, _args):
    return [SetTitle("KeyMap Probe"), SetState({"last_action": "none"})]


def update(event):
    if isinstance(event, KeyEvent) and event.pressed and event.key == "z":
        action = "ctrl-z" if event.modifiers.ctrl else "bare-z"
        return [SetState({"last_action": action})]
    return []


def view():
    return Column([
        AppBar("KeyMap Probe"),
        Text(f"last_action={state.get('last_action', 'none')}", bold=True),
        FooterKeys([("z", "record printable key")]),
    ], grow=True)
