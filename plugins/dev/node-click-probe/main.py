"""Node Click Probe - HostHarness node activation fixture (stints 0414, 1600).

Every action arrives through a node activation
(`AppRequest::ClickPaneNode`, `plexi pane click <pane_id> --node <ref>`), so a
test that only activates nodes proves the primitive end to end: read a
button's `ref` from `plexi pane state`, activate it, and observe the guest's
re-rendered `Text`.

The `s` key swaps the "Approve A" button for a "Delete B" button at the same
place in the tree, so a ref read before the swap must be refused after it.
"""

from plexi_sdk import state
from plexi_sdk.effects import SetState, SetTitle
from plexi_sdk.events import KeyEvent, UiAction
from plexi_sdk.ui import Button, Column, Text

INCREMENT = "increment"
APPROVE = "approve"
DELETE = "delete"


def init(_size, _args):
    return [
        SetTitle("Node Click Probe"),
        SetState({
            "count": state.get("count", 0),
            "approved": 0,
            "deleted": 0,
            "swapped": False,
        }),
    ]


def update(event):
    if isinstance(event, UiAction):
        field = {INCREMENT: "count", APPROVE: "approved", DELETE: "deleted"}.get(event.handler_id)
        if field is not None:
            return [SetState({field: state.get(field, 0) + 1})]
    if isinstance(event, KeyEvent) and event.pressed and event.key == "s":
        return [SetState({"swapped": not state.get("swapped", False)})]
    return []


def view():
    count = state.get("count", 0)
    decision = (
        Button("Delete B", DELETE)
        if state.get("swapped", False)
        else Button("Approve A", APPROVE)
    )
    return Column([
        Text(str(count), bold=True),
        Button("Increment", INCREMENT),
        decision,
        Text(f"approved:{state.get('approved', 0)} deleted:{state.get('deleted', 0)}"),
    ], grow=True)
