"""Focus Visibility Probe - shows this pane's live focus and visibility state.

The host sends PaneShown / FocusGained / FocusLost / PaneHidden as the pane is
shown, focused, unfocused and hidden (stint 1180). Switch panes, contexts or
zoom and watch the state and the event history change.
"""

from plexi_sdk import state
from plexi_sdk.effects import SetState, SetTitle
from plexi_sdk.events import FocusGained, FocusLost, PaneHidden, PaneShown
from plexi_sdk.ui import AppBar, Column, Text

_EVENTS = {
    PaneShown: ("visible", True, "shown"),
    PaneHidden: ("visible", False, "hidden"),
    FocusGained: ("focused", True, "focus gained"),
    FocusLost: ("focused", False, "focus lost"),
}
_HISTORY = 8


def init(size, args):
    return [
        SetTitle("Focus Visibility Probe"),
        SetState({"visible": False, "focused": False, "history": []}),
    ]


def update(event):
    entry = _EVENTS.get(type(event))
    if entry is None:
        return []
    key, value, label = entry
    history = (state.get("history", []) + [label])[-_HISTORY:]
    return [SetState({key: value, "history": history})]


def view():
    focused = state.get("focused", False)
    visible = state.get("visible", False)
    return Column([
        AppBar("Focus Visibility Probe"),
        Text("focused" if focused else "not focused", bold=True),
        Text("visible" if visible else "hidden"),
        Text("history: " + (", ".join(state.get("history", [])) or "none")),
    ], grow=True)
