"""Tests for B2 UiNode component classes: Tabs, Grid, Toggle, ProgressBar."""

import pytest

from plexi_sdk.ui import (
    Accordion,
    ActionBar,
    Avatar,
    Banner,
    Breadcrumb,
    Button,
    Card,
    Checkbox,
    CodeBlock,
    Column,
    DateTimePicker,
    EmptyState,
    Grid,
    Icon,
    KeyValue,
    Modal,
    Pagination,
    Progress,
    ProgressBar,
    Radio,
    Section,
    Select,
    Skeleton,
    Slider,
    Switch,
    TabBar,
    Table,
    Tabs,
    Text,
    TextEdit,
    Toggle,
    Tooltip,
)


# ── helpers ────────────────────────────────────────────────────────────────


class _TextNode:
    """Minimal stub component with to_node()."""
    def __init__(self, text: str) -> None:
        self.text = text

    def to_node(self) -> dict:
        return {"type": "text", "text": self.text}


# ── Tabs ───────────────────────────────────────────────────────────────────


def test_tabs_to_node_returns_column() -> None:
    a = _TextNode("Panel A")
    b = _TextNode("Panel B")
    tabs = Tabs([("Tab 1", a), ("Tab 2", b)], active=0)
    node = tabs.to_node()
    assert isinstance(node, dict)
    assert node["type"] == "column"
    # Must have at least the tab bar and the active content
    assert len(node["children"]) == 2
    tab_bar = node["children"][0]
    assert tab_bar["type"] == "row"
    assert len(tab_bar["children"]) == 2


def test_tabs_active_tab_content_is_correct() -> None:
    a = _TextNode("Panel A")
    b = _TextNode("Panel B")
    tabs = Tabs([("Tab 1", a), ("Tab 2", b)], active=1)
    node = tabs.to_node()
    active_content = node["children"][1]
    assert active_content["text"] == "Panel B"


def test_tabs_empty_produces_column() -> None:
    node = Tabs([]).to_node()
    assert node["type"] == "column"
    # Only the tab bar; no content child
    assert node["children"][0]["type"] == "row"


def test_tabs_active_button_is_primary_style() -> None:
    a = _TextNode("A")
    b = _TextNode("B")
    tabs = Tabs([("First", a), ("Second", b)], active=0)
    node = tabs.to_node()
    tab_buttons = node["children"][0]["children"]
    assert tab_buttons[0]["type"] == "button"
    assert tab_buttons[0]["style"] == "primary"
    assert tab_buttons[1]["style"] == "secondary"


# ── Grid ───────────────────────────────────────────────────────────────────


def test_grid_2col_has_correct_rows() -> None:
    items = [_TextNode(str(i)) for i in range(4)]
    node = Grid(2, items).to_node()
    assert node["type"] == "column"
    # 4 items / 2 columns = 2 rows
    assert len(node["children"]) == 2
    # Each row is a row node with 2 children
    for row in node["children"]:
        assert row["type"] == "row"
        assert len(row["children"]) == 2


def test_grid_3col_odd_items() -> None:
    items = [_TextNode(str(i)) for i in range(5)]
    node = Grid(3, items).to_node()
    rows = node["children"]
    # 5 items / 3 columns = 2 rows (3 + 2)
    assert len(rows) == 2
    assert len(rows[0]["children"]) == 3
    assert len(rows[1]["children"]) == 2


def test_grid_gap_propagates() -> None:
    items = [_TextNode("x"), _TextNode("y")]
    node = Grid(2, items, gap=16.0).to_node()
    assert node["gap"] == 16.0
    assert node["children"][0]["gap"] == 16.0


def test_grid_single_column() -> None:
    items = [_TextNode("a"), _TextNode("b"), _TextNode("c")]
    node = Grid(1, items).to_node()
    assert len(node["children"]) == 3


# ── Toggle ─────────────────────────────────────────────────────────────────


def test_toggle_with_label_wraps_button_in_row() -> None:
    toggle = Toggle("dark_mode", value=True, label="Dark mode")
    node = toggle.to_node()
    assert isinstance(node, dict)
    assert node["type"] == "row"
    label_node, state_button = node["children"]
    assert label_node == {"type": "text", "text": "Dark mode"}
    assert state_button["on_click"] == "dark_mode"


def test_toggle_without_label_is_bare_button() -> None:
    node = Toggle("t", value=False).to_node()
    assert node["type"] == "button"
    assert node["on_click"] == "t"


def test_toggle_label_reflects_value() -> None:
    assert Toggle("t", value=True).to_node()["label"] == "on"
    assert Toggle("t", value=False).to_node()["label"] == "off"


# Clickable was removed entirely (stint: SDK widget-class audit) -- it
# composed onto a WIT "interactive" node type that decode_node_data
# (src/host/wasm_python.rs) has never had a decode arm for, so any live app
# using it crashed on render. See sdk/python/tests/test_ui_tree_widget_contract.py.


# ── ProgressBar ────────────────────────────────────────────────────────────


def test_progress_bar_to_node() -> None:
    node = ProgressBar(0.5, 1.0).to_node()
    assert node == {"type": "progress-bar", "value": 0.5, "max": 1.0}


def test_progress_bar_default_max() -> None:
    node = ProgressBar(0.5).to_node()
    assert node["max"] == 1.0


def test_progress_bar_zero_max_falls_back_to_one() -> None:
    node = ProgressBar(0.5, max_value=0.0).to_node()
    assert node["max"] == 1.0


def test_progress_bar_passes_raw_value_through() -> None:
    # The host clamps for rendering; the SDK just forwards value/max.
    node = ProgressBar(2.0, 1.0).to_node()
    assert node["value"] == 2.0


# ── TextEdit ───────────────────────────────────────────────────────────────


def test_textedit_is_component() -> None:
    from plexi_sdk.ui import Component
    te = TextEdit("x")
    assert isinstance(te, Component)


def test_textedit_to_node_fields() -> None:
    te = TextEdit("notes", placeholder="Write here", value="hello", multiline=True, max_length=500)
    assert te.to_node() == {
        "type": "TextInput",
        "value": "hello",
        "placeholder": "Write here",
        "on_change": "notes",
        "on_submit": "notes",
        "password": False,
        "autofocus": False,
    }


def test_textedit_nested_in_column_is_native() -> None:
    te = TextEdit("body", placeholder="Type...", multiline=True, height=120.0)
    col = Column([te])
    node = col.to_node()
    assert node is not None
    child_node = node["children"][0]
    assert child_node["type"] == "TextInput"
    assert child_node["on_change"] == "body"


def test_textedit_accepts_node_id_keyword() -> None:
    te = TextEdit(node_id="x", value="val")
    assert te.id == "x"
    assert te.to_node()["on_change"] == "x"


# ── ActionBar ─────────────────────────────────────────────────────────────


def test_action_bar_to_node_composes_row_of_buttons() -> None:
    bar = ActionBar(
        [
            Button("Save", "save", style="primary"),
            Button("Cancel", "cancel", style="ghost"),
        ]
    )

    node = bar.to_node()

    assert node["type"] == "row"
    assert node["children"] == [
        {
            "type": "button",
            "label": "Save",
            "on_click": "save",
            "style": "primary",
            "disabled": False,
        },
        {
            "type": "button",
            "label": "Cancel",
            "on_click": "cancel",
            "style": "ghost",
            "disabled": False,
        },
    ]


def test_action_bar_rejects_non_button_actions() -> None:
    with pytest.raises(TypeError, match="ActionBar actions\\[0\\] must be a Button"):
        ActionBar([TextEdit("not-a-button")])  # type: ignore[list-item]


# ── CPython-WASM decoder drift guard ────────────────────────────────────────
#
# Mirrors the `match kind { ... }` arms of `decode_node_data` in
# `src/host/wasm_python.rs`.
_DECODER_SUPPORTED_ROOT_TYPES = {
    "row", "column", "button", "text", "progress-bar", "badge",
    "divider", "spinner", "TextInput", "card", "Card", "DataTable",
    "data_table", "table", "Table",
}


@pytest.mark.parametrize(
    "widget_factory",
    [
        lambda: Tabs([("Tab 1", _TextNode("Panel A")), ("Tab 2", _TextNode("Panel B"))], active=0),
        lambda: Tabs([]),
        lambda: Grid(2, [_TextNode("a"), _TextNode("b"), _TextNode("c")]),
        lambda: Grid(1, [_TextNode("a")]),
        lambda: Toggle("dark_mode", value=True),
        lambda: Toggle("dark_mode", value=True, label="Dark mode"),
        lambda: ProgressBar(0.5),
    ],
    ids=[
        "tabs", "tabs-empty", "grid", "grid-single-col",
        "toggle-no-label", "toggle-with-label", "progress_bar",
    ],
)
def test_v3_5_uinode_widgets_emit_decoder_supported_root_type(widget_factory) -> None:
    node = widget_factory().to_node()
    assert node["type"] in _DECODER_SUPPORTED_ROOT_TYPES, (
        f"{node['type']!r} has no CPython-WASM decode arm in "
        "src/host/wasm_python.rs decode_node_data — this widget will crash "
        "any live Python-WASM app that renders it"
    )


@pytest.mark.parametrize(
    "widget_factory",
    [
        lambda: Checkbox("c", label="I agree", checked=True),
        lambda: Checkbox("c"),
        lambda: Switch("s", label="Notifications", on=False),
        lambda: Switch("s"),
        lambda: Radio("r", options=["Low", "Medium", "High"], selected=1),
        lambda: Slider("sl", value=0.4),
        lambda: Select("se", options=["Red", "Green", "Blue"], selected=0, placeholder="Pick"),
        lambda: Select("se", options=["Red", "Green", "Blue"]),
        lambda: DateTimePicker("d", value="2026-01-01", mode="date"),
        lambda: TextEdit("te", value="hi"),
        lambda: Breadcrumb(items=["Home", "Apps", "Gallery"]),
        lambda: TabBar("tb", tabs=["Overview", "Details"], active=0),
        lambda: Pagination("p", page=0, total=5),
        lambda: Banner("Saved.", tone="success", title="Success"),
        lambda: Progress(value=0.5, label="Upload"),
        lambda: Progress(indeterminate=True, label="Syncing"),
        lambda: Skeleton(rows=3),
        lambda: KeyValue(rows=[("Region", "us-east-1")]),
        lambda: Table(columns=["Name"], rows=[["Ada"]]),
        lambda: CodeBlock(code="x = 1", language="python"),
        lambda: Card(children=[Text("hi")]),
        lambda: Avatar(label="Ian Burke"),
        lambda: Icon(name="gear"),
        lambda: Tooltip(text="hint", child=Text("hover me")),
        lambda: Accordion("a", title="Advanced", child=Text("hidden"), open=True),
        lambda: Accordion("a", title="Advanced", child=Text("hidden"), open=False),
        lambda: EmptyState(title="No results", description="Try again", icon="∅"),
        lambda: Modal("m", title="Confirm", child=Text("Are you sure?")),
        lambda: Section("Buttons"),
        lambda: ActionBar([Button("Save", "save")]),
    ],
    ids=[
        "checkbox-labeled", "checkbox-bare", "switch-labeled", "switch-bare",
        "radio", "slider", "select-with-placeholder", "select-bare",
        "date-time-picker", "text-edit", "breadcrumb", "tab-bar", "pagination",
        "banner", "progress", "progress-indeterminate", "skeleton", "key-value",
        "table", "code-block", "card", "avatar", "icon", "tooltip",
        "accordion-open", "accordion-closed", "empty-state", "modal", "section",
        "action-bar",
    ],
)
def test_stint_0407_widgets_emit_decoder_supported_root_type(widget_factory) -> None:
    node = widget_factory().to_node()
    assert node["type"] in _DECODER_SUPPORTED_ROOT_TYPES, (
        f"{node['type']!r} has no CPython-WASM decode arm in "
        "src/host/wasm_python.rs decode_node_data — this widget will crash "
        "any live Python-WASM app that renders it"
    )


# ── Stint 0816: Text / Card / Section / SelectList / DataTable / HTTP / State ──


def test_text_emits_color_truncate_and_align_and_validates_color() -> None:
    from plexi_sdk import theme
    from plexi_sdk.ui import Heading, Label, TEXT_COLORS

    for role in TEXT_COLORS:
        node = Text("status", color=role, truncate=True, align="center").to_node()
        assert node["color"] == role
        assert node["truncate"] is True
        assert node["align"] == "center"

    hex_node = Text("hex", color=theme.red, align="right").to_node()
    assert hex_node["color"] == theme.red
    assert hex_node["align"] == "right"

    hint_label = Label("subtle", tone="hint").to_node()
    assert hint_label["color"] == "muted"

    with pytest.raises(ValueError, match="Text color"):
        Text("bad", color="blue")
    with pytest.raises(ValueError, match="Label color"):
        Label("bad", color="blue")
    with pytest.raises(ValueError, match="Heading color"):
        Heading("bad", color="blue")


def test_card_and_section_and_datatable_and_select_list_wire_encoding() -> None:
    from plexi_sdk._adapter import _encode_uitree
    from plexi_sdk.events import KeyEvent
    from plexi_sdk.ui import DataTable, Label, SelectList, move_cursor

    card_tree = _encode_uitree(Card([Text("inside card")], padding=16.0))
    root_card = card_tree["nodes"][card_tree["root"]]["data"]
    assert root_card["type"] == "Card"
    assert root_card["card"] is True
    assert root_card["top"] == 16.0

    sec_with_children = Section("Group", [Text("one"), Text("two")]).to_node()
    assert sec_with_children is not None
    assert len(sec_with_children["children"]) == 4

    sec_varargs = Section("Group", Text("one"), Text("two")).to_node()
    assert sec_varargs is not None
    assert len(sec_varargs["children"]) == 4

    dt = DataTable(
        columns=["ID", "Name"],
        rows=[["1", "Alpha"], ["2", "Beta"]],
        selected=1,
        on_select="pick_row",
    )
    dt_tree = _encode_uitree(dt)
    dt_root = dt_tree["nodes"][dt_tree["root"]]["data"]
    assert dt_root["type"] == "DataTable"
    assert dt_root["columns"] == ["ID", "Name"]
    assert dt_root["rows"] == [["1", "Alpha"], ["2", "Beta"]]
    assert dt_root["selected"] == 1
    assert dt_root["on_select"] == "pick_row"

    sl = SelectList(["Alpha", Label("Beta"), {"name": "Gamma"}], selected=2, on_select="pick_item")
    sl_tree = _encode_uitree(sl)
    sl_root = sl_tree["nodes"][sl_tree["root"]]["data"]
    assert sl_root["type"] == "ListView"
    assert sl_root["selected"] == 2
    assert sl_root["on_select"] == "pick_item"
    assert len(sl_root["items"]) == 3

    assert move_cursor(KeyEvent("j"), 3, 0) == 1
    assert move_cursor(KeyEvent("down"), 3, 2) == 2
    assert move_cursor(KeyEvent("k"), 3, 2) == 1
    assert move_cursor(KeyEvent("G"), 3, 0) == 2
    assert move_cursor(KeyEvent("g"), 3, 2) == 0
    assert SelectList.move_cursor("up", 3, 1) == 0
    assert SelectList.move_cursor("x", 3, 1) is None


def test_http_fetch_params_and_http_response_helpers_and_state_helpers() -> None:
    import json
    import plexi_sdk as sdk
    from plexi_sdk import _v3_state, state
    from plexi_sdk._adapter import _decode_event, _encode_effect
    from plexi_sdk.effects import HttpFetch, PersistState, SetState
    from plexi_sdk.events import HttpResponse

    fetch = HttpFetch("https://api.example.test/search", params={"q": "plexi", "limit": 5}, tag="search")
    assert fetch.resolved_url() == "https://api.example.test/search?q=plexi&limit=5"
    encoded = _encode_effect(fetch)
    assert encoded["url"] == "https://api.example.test/search?q=plexi&limit=5"
    assert encoded["tag"] == "search"

    resp: HttpResponse = _decode_event({
        "type": "HttpResponse",
        "status": 201,
        "headers": [["content-type", "application/json"]],
        "body": list(b'{"ok": true}'),
        "error": None,
        "request_id": 7,
        "tag": "search",
        "truncated": False,
    })
    assert resp.status == 201
    assert resp.headers == [("content-type", "application/json")]
    assert resp.text() == '{"ok": true}'
    assert resp.json() == {"ok": True}
    assert resp.request_id == 7
    assert resp.tag == "search"

    _v3_state._state = sdk.StateSnapshot({"a": 10}, {"a": json.dumps(10).encode()})
    _v3_state._in_view = False
    loaded = state.load({"a": 1, "b": [1, 2]})
    assert loaded == {"a": 10, "b": [1, 2]}
    assert state.patch(a=20) == SetState({"a": 20})
    assert state.patch({"b": [3]}, persist=True) == PersistState({"b": [3]})

