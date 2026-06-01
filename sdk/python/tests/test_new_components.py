"""Tests for B2 UiNode component classes: Tabs, Grid, Toggle, Clickable, ProgressBar."""

from plexi_sdk.ui import Tabs, Grid, Toggle, Clickable, ProgressBar


# ── helpers ────────────────────────────────────────────────────────────────


class _TextNode:
    """Minimal stub component with to_node()."""
    def __init__(self, text: str) -> None:
        self.text = text

    def to_node(self) -> dict:
        return {"type": "text", "text": self.text}


# ── Tabs ───────────────────────────────────────────────────────────────────


def test_tabs_to_node_returns_stack() -> None:
    a = _TextNode("Panel A")
    b = _TextNode("Panel B")
    tabs = Tabs([("Tab 1", a), ("Tab 2", b)], active=0)
    node = tabs.to_node()
    assert isinstance(node, dict)
    assert node["type"] == "stack"
    assert node["direction"] == "vertical"
    # Must have at least the tab bar and the active content
    assert len(node["children"]) == 2
    tab_bar = node["children"][0]
    assert tab_bar["type"] == "stack"
    assert tab_bar["direction"] == "horizontal"
    assert len(tab_bar["children"]) == 2


def test_tabs_active_tab_content_is_correct() -> None:
    a = _TextNode("Panel A")
    b = _TextNode("Panel B")
    tabs = Tabs([("Tab 1", a), ("Tab 2", b)], active=1)
    node = tabs.to_node()
    active_content = node["children"][1]
    assert active_content["text"] == "Panel B"


def test_tabs_empty_produces_stack() -> None:
    node = Tabs([]).to_node()
    assert node["type"] == "stack"
    # Only the tab bar; no content child
    assert node["children"][0]["type"] == "stack"


def test_tabs_active_button_is_bold() -> None:
    a = _TextNode("A")
    b = _TextNode("B")
    tabs = Tabs([("First", a), ("Second", b)], active=0)
    node = tabs.to_node()
    tab_buttons = node["children"][0]["children"]
    assert tab_buttons[0]["child"]["bold"] is True
    assert tab_buttons[1]["child"]["bold"] is False


# ── Grid ───────────────────────────────────────────────────────────────────


def test_grid_2col_has_correct_rows() -> None:
    items = [_TextNode(str(i)) for i in range(4)]
    node = Grid(2, items).to_node()
    assert node["type"] == "stack"
    assert node["direction"] == "vertical"
    # 4 items / 2 columns = 2 rows
    assert len(node["children"]) == 2
    # Each row is a horizontal stack with 2 children
    for row in node["children"]:
        assert row["type"] == "stack"
        assert row["direction"] == "horizontal"
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


def test_toggle_to_node_has_node_id() -> None:
    toggle = Toggle("dark_mode", value=True, label="Dark mode")
    node = toggle.to_node()
    assert isinstance(node, dict)
    assert node.get("node_id") == "dark_mode"


def test_toggle_on_click_is_true() -> None:
    node = Toggle("t", value=False).to_node()
    assert node["on_click"] is True


def test_toggle_has_l0_fallback() -> None:
    node = Toggle("t", value=True, label="X").to_node()
    assert "_l0" in node
    l0 = node["_l0"]
    assert l0["type"] == "interactive"
    assert l0["node_id"] == "t"


def test_toggle_type_is_interactive() -> None:
    node = Toggle("t", value=False).to_node()
    assert node["type"] == "interactive"


# ── Clickable ──────────────────────────────────────────────────────────────


def test_clickable_wraps_in_interactive() -> None:
    child = _TextNode("click me")
    node = Clickable("btn", child).to_node()
    assert isinstance(node, dict)
    assert node["type"] == "interactive"
    assert node["node_id"] == "btn"


def test_clickable_child_is_nested() -> None:
    child = _TextNode("hello")
    node = Clickable("x", child).to_node()
    assert node["child"] == {"type": "text", "text": "hello"}


def test_clickable_on_click_default_true() -> None:
    node = Clickable("x", _TextNode("y")).to_node()
    assert node["on_click"] is True


def test_clickable_on_click_false() -> None:
    node = Clickable("x", _TextNode("y"), on_click=False).to_node()
    assert node["on_click"] is False


def test_clickable_on_hover_is_false() -> None:
    node = Clickable("x", _TextNode("y")).to_node()
    assert node["on_hover"] is False


# ── ProgressBar ────────────────────────────────────────────────────────────


def test_progress_bar_to_node() -> None:
    node = ProgressBar(0.5, 1.0).to_node()
    assert isinstance(node, dict)
    assert node["type"] == "stack"


def test_progress_bar_direction_horizontal() -> None:
    node = ProgressBar(0.5).to_node()
    assert node["direction"] == "horizontal"


def test_progress_bar_full_has_one_child() -> None:
    node = ProgressBar(1.0, 1.0).to_node()
    # 100% filled: empty ratio is 0, so only the filled child
    assert len(node["children"]) == 1
    assert node["children"][0]["type"] == "text"


def test_progress_bar_zero_has_two_children() -> None:
    # 0% filled still produces filled (at least 1 char) + empty
    node = ProgressBar(0.0, 1.0).to_node()
    assert len(node["children"]) == 2


def test_progress_bar_clamps_above_max() -> None:
    node = ProgressBar(2.0, 1.0).to_node()
    assert len(node["children"]) == 1  # 100%, no empty portion


def test_progress_bar_custom_color() -> None:
    node = ProgressBar(0.5, color="danger").to_node()
    filled = node["children"][0]
    assert filled["color"] == "danger"


def test_progress_bar_default_color() -> None:
    node = ProgressBar(0.5).to_node()
    filled = node["children"][0]
    assert filled["color"] == "accent"
