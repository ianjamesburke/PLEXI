"""Guardrail: every widget `plexi_sdk.ui` exports must produce a UiNode tree
the host's `decode_node_data` (`src/host/wasm_python.rs`) can actually decode.

An SDK audit found a whole class of exported widgets that crashed or
silently no-op'd in the only shipped rendering mode -- the v3 declarative
tree (`Markdown`/`Clickable` emitted node types with no host decode arm;
`Footer`'s own node likewise; `ButtonRow` omitted a required field;
`InfoTable`/`Row`/`FormField`/`KeyRow`/`ScrollLog`/`ChatBubble`/`ListItem`/
`Sized` had no `to_node()` at all, so `render_tree()` raised
`TypeError: X.to_node() returned None`; `SelectList` silently dropped
`leading`/`trailing`; `Divider(color=...)` silently dropped `color`). Each
is now either fixed or removed from `plexi_sdk.ui.__all__` -- see the
docstring of each affected class in `ui.py` for the reasoning.

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
# aliases). Keep in sync with that match by hand -- same convention as
# `_DECODER_SUPPORTED_ROOT_TYPES` in `test_new_components.py` and
# `BADGE_COLORS` mirroring `decode_badge_color` in `ui.py`. Deliberately not
# derived from the SDK's own output, or this guard would be tautological.
ACCEPTED_NODE_TYPES = frozenset({
    "Empty",
    "Text", "text", "label",
    "AppBar", "app_bar", "app-bar",
    "FooterKeys", "footer_keys", "footer-keys",
    "Pinned", "pinned",
    "Spinner", "spinner",
    "Column", "column",
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
    "Canvas", "canvas",
})

# Token/constant names in `ui.__all__` -- not classes, nothing to instantiate.
_TOKENS = {
    "SPACE_XS", "SPACE_SM", "SPACE_MD", "SPACE_LG", "SPACE_XL",
    "TEXT_HINT", "TEXT_CAPTION", "TEXT_BODY", "TEXT_HEADING",
    "TEXT_TITLE", "TEXT_TITLE_XL",
    "RADIUS_SM", "RADIUS_MD", "RADIUS_LG", "RADIUS_BADGE",
    "BG", "FG", "ACCENT", "SURFACE", "HIGHLIGHT", "MUTED", "GREEN", "RED", "YELLOW",
    "BadgeColor", "BADGE_COLORS",
}

# Exported symbols that are not themselves tree-node widgets -- each has a
# documented reason they can't/shouldn't go through `to_node()`.
_NOT_A_WIDGET = {
    "Component",  # abstract base; never instantiated directly, to_node() is None by design
    "badge",  # canvas-mode drawing helper function, not a Component
    "ensure_visible",  # scroll-math helper function
    "render_tree",  # the render entry point itself, not a node
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
    "ActionBar": lambda: ui.ActionBar([]),
    "AppBar": lambda: ui.AppBar("Title"),
    "Section": lambda: ui.Section("Title"),
    "Heading": lambda: ui.Heading("Title"),
    "Label": lambda: ui.Label("Body"),
    "Spacer": lambda: ui.Spacer(),
    "Divider": lambda: ui.Divider(),
    "Badge": lambda: ui.Badge("tag"),
    "Canvas": lambda: ui.Canvas(),
    "Scrollable": lambda: ui.Scrollable(ui.Label("content")),
    "FooterKeys": lambda: ui.FooterKeys([("k", "desc")]),
    "TextInput": lambda: ui.TextInput("field-id"),
    "TextEdit": lambda: ui.TextEdit("field-id"),
    "SelectList": lambda: ui.SelectList(
        [{"name": "one", "leading": "*", "trailing": ">"}]
    ),
    "Pending": lambda: ui.Pending(
        active=True, child=ui.Label("content"), placeholder=ui.Skeleton(rows=3)
    ),
    "ButtonRow": lambda: ui.ButtonRow("btn-id", "Click"),
    "Tabs": lambda: ui.Tabs([("Tab", ui.Label("content"))]),
    "Grid": lambda: ui.Grid(2, [ui.Label("a"), ui.Label("b")]),
    "Toggle": lambda: ui.Toggle("toggle-id", value=True, label="Enabled"),
    "ProgressBar": lambda: ui.ProgressBar(0.5),
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
            f"(the exact path render_tree() drives at runtime): {exc}"
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


def test_button_row_emits_required_on_click() -> None:
    """Regression: decode_node_data's Button arm requires "on_click"
    (required_string) -- ButtonRow used to omit it entirely."""
    node = ui.ButtonRow("action-id", "Go").to_node()
    assert node["on_click"] == "action-id"


def test_pending_renders_placeholder_or_child_by_active() -> None:
    child = ui.Label("content")
    placeholder = ui.Skeleton(rows=3)
    assert ui.Pending(active=True, child=child, placeholder=placeholder).to_node() == (
        placeholder.to_node()
    )
    assert ui.Pending(active=False, child=child, placeholder=placeholder).to_node() == (
        child.to_node()
    )
