"""Tests for actionable SDK error messages (#1203).

Each test verifies that a common misuse pattern raises a clear, actionable
error instead of a cryptic AttributeError or TypeError deep in the call stack.
"""

import json
import warnings
from unittest.mock import MagicMock

import pytest

from plexi_sdk.ui import (
    Column, Card, Scrollable, Label, Heading, Component, FooterKeys,
    Spacer, ChatBubble, SelectList, InfoTable, ListRow, LeadingBadge,
    RowChip, Tabs, Grid, render_tree,
)
from plexi_sdk._render_context import RenderContext
from plexi_sdk._types import TextCommand, VALID_TEXT_ALIGN
from plexi_sdk.widgets.list_view import ListView
from plexi_sdk.ui import ListItem


# ── Helpers ───────────────────────────────────────────────────────────────────


def _make_ctx() -> RenderContext:
    app = MagicMock()
    app._mx = 0.0
    app._my = 0.0
    app._btn_active_until = {}
    return RenderContext(
        frame_id=0,
        rect={"x": 0.0, "y": 0.0, "w": 400.0, "h": 300.0},
        workspace_root="",
        capabilities=[],
        feature_flags=[],
        app=app,
        elapsed=0.0,
        clicks=[],
    )


# ── ctx.render() misuse ──────────────────────────────────────────────────────


def test_ctx_render_rejects_none():
    ctx = _make_ctx()
    with pytest.raises(TypeError, match="received None"):
        ctx.render(None)


def test_ctx_render_rejects_string():
    ctx = _make_ctx()
    with pytest.raises(TypeError, match="received a string"):
        ctx.render("hello")


def test_ctx_render_rejects_list():
    ctx = _make_ctx()
    with pytest.raises(TypeError, match="received a list"):
        ctx.render([Label(text="a"), Label(text="b")])


def test_ctx_render_rejects_tuple():
    ctx = _make_ctx()
    with pytest.raises(TypeError, match="received a tuple"):
        ctx.render((Label(text="a"),))


# ── render_tree() rejects non-Component ───────────────────────────────────────


def test_render_tree_rejects_non_component():
    ctx = _make_ctx()
    with pytest.raises(TypeError, match="expected a Component"):
        render_tree(ctx, "not a component")


def test_render_tree_rejects_dict():
    ctx = _make_ctx()
    with pytest.raises(TypeError, match="expected a Component"):
        render_tree(ctx, {"type": "text"})


# ── ctx.rect() / ctx.text() type validation ──────────────────────────────────


def test_ctx_rect_rejects_non_string_fill():
    ctx = _make_ctx()
    with pytest.raises(TypeError, match="fill must be a hex color string"):
        ctx.rect(0, 0, 100, 100, 0xFF0000)


def test_ctx_text_rejects_non_string_text():
    ctx = _make_ctx()
    with pytest.raises(TypeError, match="text must be a string"):
        ctx.text(0, 0, 42, size=14.0, color="#cdd6f4")


def test_ctx_text_rejects_non_string_color():
    ctx = _make_ctx()
    with pytest.raises(TypeError, match="color must be a hex color string"):
        ctx.text(0, 0, "hello", size=14.0, color=0xFF0000)


def test_ctx_text_rejects_non_number_size():
    ctx = _make_ctx()
    with pytest.raises(TypeError, match="size must be a number"):
        ctx.text(0, 0, "hello", size="big", color="#cdd6f4")


# ── Text align vocabulary (#2198) ─────────────────────────────────────────────


def test_ctx_text_rejects_legacy_align():
    ctx = _make_ctx()
    with pytest.raises(ValueError, match="left_top"):
        ctx.text(0, 0, "hello", size=14.0, color="#cdd6f4", align="top_left")


def test_ctx_text_accepts_all_nine_aligns():
    ctx = _make_ctx()
    for align in VALID_TEXT_ALIGN:
        ctx.text(0, 0, "hello", size=14.0, color="#cdd6f4", align=align)


def test_text_command_default_align_is_canonical():
    cmd = TextCommand(x=0, y=0, text="hi", size=14.0, color="#cdd6f4")
    assert cmd.align in VALID_TEXT_ALIGN


def test_text_command_rejects_legacy_align():
    with pytest.raises(ValueError, match="align must be one of"):
        TextCommand(x=0, y=0, text="hi", size=14.0, color="#cdd6f4", align="top_left")


def test_text_command_accepts_center_center():
    cmd = TextCommand(x=0, y=0, text="hi", size=14.0, color="#cdd6f4",
                      align="center_center")
    assert cmd.align == "center_center"


# ── ctx.render_tree() validation ─────────────────────────────────────────────


def test_ctx_render_tree_rejects_none():
    ctx = _make_ctx()
    with pytest.raises(TypeError, match="received None"):
        ctx.render_tree(None)


# ── Heading validation ────────────────────────────────────────────────────────


def test_heading_rejects_non_string_text():
    with pytest.raises(TypeError, match="text must be a string"):
        Heading(text=42)


def test_heading_rejects_invalid_level():
    with pytest.raises(ValueError, match="level must be 1, 2, or 3"):
        Heading(text="Title", level=5)


def test_heading_accepts_valid_levels():
    for level in (1, 2, 3):
        h = Heading(text="ok", level=level)
        assert h.level == level


# ── Label validation ─────────────────────────────────────────────────────────


def test_label_rejects_non_string_text():
    with pytest.raises(TypeError, match="text must be a string"):
        Label(text=123)


def test_label_rejects_invalid_tone():
    with pytest.raises(ValueError, match="tone must be"):
        Label(text="ok", tone="large")


def test_label_accepts_valid_tones():
    for tone in ("body", "caption", "hint"):
        lbl = Label(text="ok", tone=tone)
        assert lbl.tone == tone


# ── ChatBubble validation ────────────────────────────────────────────────────


def test_chatbubble_rejects_invalid_role():
    with pytest.raises(ValueError, match="role must be"):
        ChatBubble(text="hi", role="system")


def test_chatbubble_accepts_valid_roles():
    for role in ("user", "assistant", "error"):
        bubble = ChatBubble(text="hi", role=role)
        assert bubble.role == role


# ── FooterKeys validation ────────────────────────────────────────────────────


def test_footerkeys_rejects_non_tuple_shortcut():
    with pytest.raises(TypeError, match="must be a .* tuple"):
        FooterKeys(shortcuts=["j"])


def test_footerkeys_rejects_wrong_length_tuple():
    with pytest.raises(TypeError, match="must be a .* tuple"):
        FooterKeys(shortcuts=[("j", "down", "extra")])


def test_footerkeys_accepts_valid_shortcuts():
    fk = FooterKeys(shortcuts=[("j", "down"), (["g", "G"], "ends")])
    assert len(fk.shortcuts) == 2


# ── SelectList validation ────────────────────────────────────────────────────


def test_selectlist_rejects_non_list():
    with pytest.raises(TypeError, match="must be a list of dicts"):
        SelectList(items="not a list")


def test_selectlist_rejects_non_dict_item():
    with pytest.raises(TypeError, match="must be a dict"):
        SelectList(items=["string item"])


def test_selectlist_rejects_missing_name_key():
    with pytest.raises(ValueError, match="missing required 'name' key"):
        SelectList(items=[{"title": "wrong key"}])


def test_selectlist_accepts_valid_items():
    sl = SelectList(items=[{"name": "Item 1"}, {"name": "Item 2", "description": "desc"}])
    assert len(sl.items) == 2


# ── InfoTable validation ─────────────────────────────────────────────────────


def test_infotable_rejects_non_tuple_row():
    with pytest.raises(TypeError, match="must be a .* tuple"):
        InfoTable(rows=["not a tuple"])


def test_infotable_rejects_wrong_length_row():
    with pytest.raises(TypeError, match="must be a .* tuple"):
        InfoTable(rows=[("key", "val", "extra")])


def test_infotable_accepts_valid_rows():
    it = InfoTable(rows=[("key", "value")])
    assert len(it.rows) == 1


# ── ListRow validation ───────────────────────────────────────────────────────


def test_listrow_rejects_wrong_leading_type():
    with pytest.raises(TypeError, match="leading must be"):
        ListRow(id="x", primary="text", leading="not_a_badge")


def test_listrow_rejects_wrong_chip_type():
    with pytest.raises(TypeError, match="chips.*must be a RowChip"):
        ListRow(id="x", primary="text", chips=["not a chip"])


def test_listrow_accepts_valid_leading():
    lr = ListRow(id="x", primary="text", leading=LeadingBadge("label"))
    assert lr.leading is not None


# ── Tabs validation ──────────────────────────────────────────────────────────


def test_tabs_rejects_non_tuple_entry():
    with pytest.raises(TypeError, match="must be a .* tuple"):
        Tabs(tabs=["not a tuple"])


def test_tabs_rejects_non_string_label():
    node = MagicMock()
    node.to_node.return_value = {"type": "text", "text": "x"}
    with pytest.raises(TypeError, match="label must be a str"):
        Tabs(tabs=[(42, node)])


def test_tabs_accepts_valid_entries():
    node = MagicMock()
    node.to_node.return_value = {"type": "text", "text": "x"}
    t = Tabs(tabs=[("Tab1", node)])
    assert len(t.tabs) == 1


# ── Grid validation ──────────────────────────────────────────────────────────


def test_grid_rejects_non_has_to_node_child():
    with pytest.raises(TypeError, match="must implement to_node"):
        Grid(columns=2, children=[42, "not a node"])


def test_grid_accepts_dict_children():
    g = Grid(columns=2, children=[{"type": "text"}])
    assert len(g.children) == 1


# ── ListView.render() validation ─────────────────────────────────────────────


def test_listview_render_rejects_non_renderable_items():
    lv = ListView(item_height=40.0)
    with pytest.raises(TypeError, match="must have a render"):
        lv.render(["not a component"])


def test_listview_render_accepts_component_items():
    lv = ListView(item_height=40.0)
    item = ListItem(title="Test")
    renderer = lv.render([item])
    assert isinstance(renderer, Component)
