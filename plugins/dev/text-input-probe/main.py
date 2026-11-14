"""Text Input Probe - HostHarness TextInput typing fixture (stint 0456).

A single `TextInput` plus observer `Text` rows. Proves the end-to-end typing
contract: keystrokes into a *focused* declarative TextInput reach the host
TextEdit (never the app's raw KeyEvent path), `on_change` round-trips the
draft, Enter fires `on_submit`, and an unfocused pane still routes keys to
the app as `KeyEvent`s. The `keys:` counter is the tell: it must stay frozen
while the field is focused and typing happens.
"""

from plexi_sdk import state
from plexi_sdk.effects import SetState, SetTitle
from plexi_sdk.events import KeyEvent, UiAction, UiValueChange
from plexi_sdk.ui import Column, Text, TextInput

DRAFT_INPUT = "draft-input"
SUBMIT = "submit"


def init(_size, _args):
    return [
        SetTitle("Text Input Probe"),
        SetState({
            "draft": state.get("draft", ""),
            "submitted": state.get("submitted", ""),
            "keys": state.get("keys", 0),
        }),
    ]


def update(event):
    if isinstance(event, UiValueChange) and event.handler_id == DRAFT_INPUT:
        return [SetState({"draft": event.value})]
    if isinstance(event, UiAction) and event.handler_id == SUBMIT:
        return [SetState({"submitted": state.get("draft", ""), "draft": ""})]
    if isinstance(event, KeyEvent) and event.pressed:
        return [SetState({"keys": state.get("keys", 0) + 1})]
    return []


def view():
    return Column([
        Text(f"draft:{state.get('draft', '')}"),
        Text(f"submitted:{state.get('submitted', '')}"),
        Text(f"keys:{state.get('keys', 0)}"),
        TextInput(
            DRAFT_INPUT,
            value=state.get("draft", ""),
            placeholder="Type here",
            on_submit=SUBMIT,
        ),
    ], grow=True)
