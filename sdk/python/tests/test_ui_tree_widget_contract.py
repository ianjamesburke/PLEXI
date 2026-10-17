"""Guardrail: every widget `plexi_sdk.ui` exports must produce a UiNode tree
the host's `decode_node_data` (`src/host/wasm_python.rs`) can actually decode.

This test enumerates *every* declarative-tree widget class currently
exported and asserts its `to_node()` output survives real encoding via
`_adapter._encode_uitree` -- the exact function the CPython-WASM bridge
calls from `view()` -- with every node's "type" landing in the same set
`decode_node_data` accepts. A future widget that returns `None` or an
undecodable type fails here instead of crashing a live app's render pass.
"""
from __future__ import annotations

import pytest

from plexi_sdk import ui
from plexi_sdk._adapter import _encode_uitree


# Every literal "type" string `decode_node_data` in `src/host/wasm_python.rs`
# matches (its match arms accept both PascalCase and lower-case/kebab
# aliases). Keep in sync with that match by hand.
ACCEPTED_NODE_TYPES = frozenset({
    "Empty",
    "Text", "text", "label",
    "AppBar", "app_bar", "app-bar",
    "FooterKeys", "footer_keys", "footer-keys",
    "Pinned", "pinned",
    "Spinner", "spinner",
    "Column", "column",
    "DataTable", "data_table", "table", "Table",
    "Button", "button",
    "TextInput", "text_input", "text-input",
    "Row", "row",
    "Divider", "divider",
    "Space", "spacer",
    "ProgressBar", "progress_bar", "progress-bar",
    "Badge", "badge",
    "ListView", "list_view", "list-view",
    "Scroll", "scroll",
    "Padding", "padding",
    "Card", "card",
    "Canvas", "canvas",
})

# Token/constant names in `ui.__all__` -- not classes, nothing to instantiate.
_TOKENS = {
    "SPACE_XS", "SPACE_SM", "SPACE_MD", "SPACE_LG", "SPACE_XL",
    "TEXT_HINT", "TEXT_CAPTION", "TEXT_BODY", "TEXT_HEADING",
    "TEXT_TITLE", "TEXT_TITLE_XL",
    "RADIUS_SM", "RADIUS_MD", "RADIUS_LG", "RADIUS_BADGE",
    "BadgeColor", "BADGE_COLORS",
    "TextColor", "TEXT_COLORS",
    "CANVAS_WIRE_DECIMALS",
}

# Exported symbols that are not themselves tree-node widgets -- each has a
# documented reason they can't/shouldn't go through `to_node()`.
_NOT_A_WIDGET = {
    "Component",  # abstract base; never instantiated directly, to_node() is None by design
    "ensure_visible",  # scroll-math helper function
    "move_cursor",  # list-cursor helper function
    "CanvasRect", "CanvasCircle", "CanvasLine", "CanvasText",
    # ^ Canvas draw commands -- to_command(), not to_node(); only meaningful
    # nested inside a Canvas(commands=[...]) node, never as a tree node.
}

# name -> zero-argument factory producing a minimally-valid instance.
# Every name in `ui.__all__` must appear here, in `_NOT_A_WIDGET`, or in
# `_TOKENS` -- `test_every_export_is_classified` fails loudly otherwise, so
# a newly exported widget can't slip past this guardrail untested.
_FACTORIES = {
    "Column": lambda: ui.Column([]),
    "Card": lambda: ui.Card([]),
    "HStack": lambda: ui.HStack([]),
    "Row": lambda: ui.Row([]),
    "Actions": lambda: ui.Actions([]),
    "ActionBar": lambda: ui.ActionBar([]),
    "AppBar": lambda: ui.AppBar("Title"),
    "Section": lambda: ui.Section("Title"),
    "Heading": lambda: ui.Heading("Title"),
    "Label": lambda: ui.Label("Body"),
    "Text": lambda: ui.Text("Body"),
    "Spacer": lambda: ui.Spacer(),
    "Divider": lambda: ui.Divider(),
    "Badge": lambda: ui.Badge("tag"),
    "Banner": lambda: ui.Banner("Saved", tone="success"),
    "Button": lambda: ui.Button("Click", "btn-id"),
    "Checkbox": lambda: ui.Checkbox("chk-id", label="Agree"),
    "Toggle": lambda: ui.Toggle("toggle-id", value=True, label="Enabled"),
    "Switch": lambda: ui.Switch("switch-id", on=False, label="Enabled"),
    "Radio": lambda: ui.Radio("radio-id", options=["A", "B"]),
    "Slider": lambda: ui.Slider("slider-id", value=0.5),
    "Select": lambda: ui.Select("select-id", options=["A", "B"], selected=0),
    "DateTimePicker": lambda: ui.DateTimePicker("dt-id", value="2026-01-01"),
    "FormField": lambda: ui.FormField("field-id", "Label"),
    "TextInput": lambda: ui.TextInput("field-id"),
    "TextEdit": lambda: ui.TextEdit("field-id"),
    "SelectList": lambda: ui.SelectList(
        [{"name": "one", "leading": "*", "trailing": ">"}]
    ),
    "Breadcrumb": lambda: ui.Breadcrumb(["Home", "Sub"]),
    "Tabs": lambda: ui.Tabs([("Tab", ui.Label("content"))]),
    "TabBar": lambda: ui.TabBar("tb", tabs=["One", "Two"], active=0),
    "Pagination": lambda: ui.Pagination("page-id", page=0, total=3),
    "DataTable": lambda: ui.DataTable(columns=["Col"], rows=[["Val"]]),
    "Table": lambda: ui.Table(columns=["Col"], rows=[["Val"]]),
    "Grid": lambda: ui.Grid(2, [ui.Label("a"), ui.Label("b")]),
    "ProgressBar": lambda: ui.ProgressBar(0.5),
    "Progress": lambda: ui.Progress(value=0.5, label="Loading"),
    "Spinner": lambda: ui.Spinner("Loading"),
    "Skeleton": lambda: ui.Skeleton(rows=2),
    "Pending": lambda: ui.Pending(
        active=True, child=ui.Label("content"), placeholder=ui.Skeleton(rows=3)
    ),
    "KeyValue": lambda: ui.KeyValue([("Key", "Val")]),
    "CodeBlock": lambda: ui.CodeBlock("x = 1", language="python"),
    "Avatar": lambda: ui.Avatar("Ian Burke"),
    "Icon": lambda: ui.Icon("gear"),
    "Tooltip": lambda: ui.Tooltip("hint", ui.Label("target")),
    "Accordion": lambda: ui.Accordion("acc-id", "More", ui.Label("body"), open=True),
    "EmptyState": lambda: ui.EmptyState("Empty", description="Nothing here"),
    "Modal": lambda: ui.Modal("modal-id", "Confirm", ui.Label("Sure?")),
    "Scrollable": lambda: ui.Scrollable(ui.Label("content")),
    "FooterKeys": lambda: ui.FooterKeys([("k", "desc")]),
    "Canvas": lambda: ui.Canvas(),
}


def test_every_export_is_classified() -> None:
    known = _TOKENS | _NOT_A_WIDGET | set(_FACTORIES)
    missing = [name for name in ui.__all__ if name not in known]
    assert not missing, (
        f"plexi_sdk.ui.__all__ exports {missing!r} which this guardrail "
        "does not classify. Add a factory to _FACTORIES in "
        "sdk/python/tests/test_ui_tree_widget_contract.py, or a documented "
        "skip to _NOT_A_WIDGET/_TOKENS."
    )
    stale = [name for name in _FACTORIES if name not in ui.__all__]
    assert not stale, (
        f"_FACTORIES references {stale!r} which are no longer in "
        "ui.__all__ -- remove the stale entries."
    )


def _node_types(tree: dict) -> list[str]:
    return [node["data"]["type"] for node in tree["nodes"]]


@pytest.mark.parametrize("name", sorted(_FACTORIES))
def test_widget_produces_host_decodable_tree(name: str) -> None:
    widget = _FACTORIES[name]()
    try:
        tree = _encode_uitree(widget)
    except Exception as exc:  # noqa: BLE001 -- want the widget name in the failure
        pytest.fail(
            f"{name}.to_node() could not be encoded into a UiNode tree "
            f"(the exact path a live render drives at runtime): {exc}"
        )
    types = _node_types(tree)
    bad = [t for t in types if t not in ACCEPTED_NODE_TYPES]
    assert not bad, (
        f"{name} produced node type(s) {bad!r} not decodable by the host's "
        f"decode_node_data (src/host/wasm_python.rs). Full tree types: "
        f"{types!r}"
    )


def test_select_list_leading_trailing_reach_the_wire() -> None:
    """Regression for the leading/trailing drop: _adapter.py's select_list
    branch must actually emit the leading/trailing text, not just accept it
    without erroring."""
    tree = _encode_uitree(
        ui.SelectList([{"name": "Item", "leading": "*", "trailing": "chevron"}])
    )
    texts = {
        node["data"].get("text")
        for node in tree["nodes"]
        if node["data"].get("type") == "text"
    }
    assert "*" in texts
    assert "chevron" in texts


def test_divider_color_is_not_emitted_on_the_wire() -> None:
    """Regression: the WIT `divider` variant is a bare unit with no fields;
    a color in the wire dict would silently be ignored by the host, so
    to_node() must not emit one at all."""
    node = ui.Divider(color="#ff0000").to_node()
    assert node == {"type": "divider"}


def test_pending_renders_placeholder_or_child_by_active() -> None:
    child = ui.Label("content")
    placeholder = ui.Skeleton(rows=3)
    assert ui.Pending(active=True, child=child, placeholder=placeholder).to_node() == (
        placeholder.to_node()
    )
    assert ui.Pending(active=False, child=child, placeholder=placeholder).to_node() == (
        child.to_node()
    )
