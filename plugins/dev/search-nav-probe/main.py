"""Search Nav Probe - ArrowUp/ArrowDown-through-focused-TextInput fixture (stint 0729 follow-up).

A single autofocus `TextInput` plus a static result list and two observer
rows: `selected:` (the list cursor) and `keys:` (a counter of every KeyEvent
the app has seen). Proves the `dispatch_app_key_events` contract while a
declarative TextInput holds focus: bare ArrowUp/ArrowDown presses still
reach the app's KeyEvent path and move `selected`, while a typed letter
lands in the host TextEdit's `on_change` draft and never bumps `keys`.
"""

from plexi_sdk import state
from plexi_sdk.effects import SetState, SetTitle
from plexi_sdk.events import KeyEvent, UiValueChange
from plexi_sdk.ui import Column, FooterKeys, SelectList, Text, TextInput

RESULTS = ["Alpha", "Bravo", "Charlie"]
QUERY_INPUT = "query"


def init(_size, _args):
    return [
        SetTitle("Search Nav Probe"),
        SetState({
            "query": state.get("query", ""),
            "selected": state.get("selected", 0),
            "keys": state.get("keys", 0),
        }),
    ]


def update(event):
    if isinstance(event, UiValueChange) and event.handler_id == QUERY_INPUT:
        return [SetState({"query": event.value})]
    if isinstance(event, KeyEvent) and event.pressed:
        selected = state.get("selected", 0)
        if event.key in ("down", "ArrowDown"):
            selected = _clamp(selected + 1)
        elif event.key in ("up", "ArrowUp"):
            selected = _clamp(selected - 1)
        return [SetState({"keys": state.get("keys", 0) + 1, "selected": selected})]
    return []


def _clamp(selected: int) -> int:
    return max(0, min(selected, len(RESULTS) - 1))


def view():
    return Column([
        Text(f"query:{state.get('query', '')}"),
        Text(f"selected:{state.get('selected', 0)}"),
        Text(f"keys:{state.get('keys', 0)}"),
        SelectList(
            [{"name": name} for name in RESULTS], selected_idx=state.get("selected", 0)
        ),
        TextInput(
            QUERY_INPUT,
            value=state.get("query", ""),
            on_change=QUERY_INPUT,
            placeholder="Search",
            autofocus=True,
        ),
        FooterKeys([("up/down", "select result")]),
    ], grow=True)
