"""V3 TextEdit example: state changes and submissions flow through ``update``."""

from plexi_sdk import state
from plexi_sdk.effects import SetState, SetTitle
from plexi_sdk.events import UiAction, UiValueChange
from plexi_sdk.ui import AppBar, Button, Column, FooterKeys, Label, TextEdit


def init(_size, _args):
    return [SetTitle("TextEdit Demo"), SetState({"draft": "", "submitted": ""})]


def update(event):
    if isinstance(event, UiValueChange) and event.handler_id == "draft":
        return [SetState({"draft": event.value})]
    if isinstance(event, UiAction) and event.handler_id in ("draft", "submit"):
        return [SetState({"submitted": state.get("draft", "")})]
    return []


def view():
    return Column([
        AppBar("TextEdit Demo"),
        TextEdit("draft", value=state.get("draft", ""), multiline=True,
                 placeholder="Type here..."),
        Button("Submit", on_click="submit", style="primary"),
        Label(state.get("submitted", "") or "No submission yet", tone="hint"),
        FooterKeys([("enter", "submit text")]),
    ], grow=True)
