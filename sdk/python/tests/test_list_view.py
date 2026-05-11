"""Tests for ListView stateful widget."""

from unittest.mock import MagicMock

from plexi_sdk.widgets.list_view import ListView


class _FakeItem:
    """Minimal Component-like object for testing."""

    def measure(self, _avail_w: float) -> float:
        return 0.0

    def is_grow(self) -> bool:
        return False

    def render(self, ctx, x, y, w, h) -> None:
        ctx.rect(x, y, w, h, "bg")


def _make_items(n: int) -> list:
    return [_FakeItem() for _ in range(n)]


def _do_render(lv: ListView, items: list, x=0.0, y=10.0, w=200.0, h=300.0) -> None:
    """Run a render pass so viewport state is recorded in lv."""
    ctx = MagicMock()
    component = lv.render(items)
    component.render(ctx, x, y, w, h)


# -- Key navigation -----------------------------------------------------------

def test_j_moves_selection_down() -> None:
    lv = ListView(item_height=48.0)
    items = _make_items(5)
    _do_render(lv, items)
    consumed = lv.handle_key("j")
    assert consumed is True
    assert lv.selected_index == 1


def test_k_moves_selection_up() -> None:
    lv = ListView(item_height=48.0)
    items = _make_items(5)
    _do_render(lv, items)
    lv.set_selected(3)
    lv.handle_key("k")
    assert lv.selected_index == 2


def test_arrow_keys_consumed() -> None:
    lv = ListView(item_height=48.0)
    _do_render(lv, _make_items(5))
    assert lv.handle_key("ArrowDown") is True
    assert lv.handle_key("ArrowUp") is True
    assert lv.selected_index == 0  # down then up = back to 0


def test_unrecognised_key_not_consumed() -> None:
    lv = ListView(item_height=48.0)
    _do_render(lv, _make_items(3))
    assert lv.handle_key("Return") is False


def test_clamps_at_top_boundary() -> None:
    lv = ListView(item_height=48.0)
    _do_render(lv, _make_items(3))
    lv.handle_key("k")  # already at 0
    assert lv.selected_index == 0


def test_clamps_at_bottom_boundary() -> None:
    lv = ListView(item_height=48.0)
    _do_render(lv, _make_items(3))
    lv.set_selected(2)
    lv.handle_key("j")  # already at last
    assert lv.selected_index == 2


def test_empty_list_keys_not_consumed() -> None:
    lv = ListView(item_height=48.0)
    _do_render(lv, [])
    assert lv.handle_key("j") is False
    assert lv.handle_key("k") is False


# -- Scroll clamping ----------------------------------------------------------

def test_scroll_advances_when_selection_passes_viewport_bottom() -> None:
    # viewport h=100, item_height=48 → 2 items visible
    lv = ListView(item_height=48.0)
    items = _make_items(10)
    _do_render(lv, items, h=100.0)
    # Press j twice — item 2 bottom is at 144 > 100
    lv.handle_key("j")
    lv.handle_key("j")
    assert lv._scroll_offset > 0.0


def test_scroll_retreats_when_selection_passes_viewport_top() -> None:
    lv = ListView(item_height=48.0)
    items = _make_items(10)
    _do_render(lv, items, h=100.0)
    lv.set_selected(5)
    prev_scroll = lv._scroll_offset
    lv.set_selected(0)
    assert lv._scroll_offset < prev_scroll


def test_scroll_clamps_to_zero() -> None:
    lv = ListView(item_height=48.0)
    items = _make_items(3)
    _do_render(lv, items, h=300.0)  # viewport larger than content
    assert lv._scroll_offset == 0.0


def test_scroll_clamps_on_viewport_resize() -> None:
    lv = ListView(item_height=48.0)
    items = _make_items(10)
    # Small viewport — scroll to bottom
    _do_render(lv, items, h=100.0)
    lv.set_selected(9)
    assert lv._scroll_offset > 0.0
    # Render with a larger viewport — scroll should be clamped down
    _do_render(lv, items, h=600.0)  # bigger than total content 480px
    assert lv._scroll_offset == 0.0


# -- Hit testing --------------------------------------------------------------

def test_hit_test_first_item() -> None:
    lv = ListView(item_height=48.0)
    items = _make_items(5)
    # viewport starts at y=10
    _do_render(lv, items, y=10.0, h=500.0)
    # click at y=15 → inside first item (y=10 to y=58)
    assert lv.hit_test(15.0) == 0


def test_hit_test_second_item() -> None:
    lv = ListView(item_height=48.0)
    items = _make_items(5)
    _do_render(lv, items, y=0.0, h=500.0)
    # click at y=60 → second item (48..96)
    assert lv.hit_test(60.0) == 1


def test_hit_test_above_viewport_returns_none() -> None:
    lv = ListView(item_height=48.0)
    _do_render(lv, _make_items(3), y=50.0, h=300.0)
    assert lv.hit_test(10.0) is None  # above y=50


def test_hit_test_below_items_returns_none() -> None:
    lv = ListView(item_height=48.0)
    items = _make_items(3)  # total 144px
    _do_render(lv, items, y=0.0, h=500.0)
    assert lv.hit_test(200.0) is None  # beyond item 3 bottom


def test_hit_test_with_scroll_offset() -> None:
    lv = ListView(item_height=48.0)
    items = _make_items(10)
    _do_render(lv, items, y=0.0, h=100.0)
    # Scroll down by one item
    lv._scroll_offset = 48.0
    # click at y=10 now maps to content_y=58 → index 1
    assert lv.hit_test(10.0) == 1


def test_hit_test_before_render_returns_none() -> None:
    lv = ListView(item_height=48.0)
    assert lv.hit_test(50.0) is None


# -- set_selected -------------------------------------------------------------

def test_set_selected_clamps_negative() -> None:
    lv = ListView()
    _do_render(lv, _make_items(5))
    lv.set_selected(-1)
    assert lv.selected_index == 0


def test_set_selected_clamps_past_end() -> None:
    lv = ListView()
    _do_render(lv, _make_items(5))
    lv.set_selected(100)
    assert lv.selected_index == 4


def test_set_selected_noop_on_empty() -> None:
    lv = ListView()
    _do_render(lv, [])
    lv.set_selected(3)
    assert lv.selected_index == 0


# -- render returns a Component-protocol object --------------------------------

def test_render_returns_grow_component() -> None:
    lv = ListView()
    comp = lv.render(_make_items(3))
    assert comp.is_grow() is True
    assert comp.measure(200.0) == 0.0


def test_render_calls_item_render_for_visible_items() -> None:
    lv = ListView(item_height=48.0)
    items = [MagicMock() for _ in range(5)]
    ctx = MagicMock()
    ctx.push_clip = MagicMock()
    ctx.pop_clip = MagicMock()
    comp = lv.render(items)
    comp.render(ctx, 0.0, 0.0, 200.0, 200.0)  # viewport shows ~4 items
    # All 5 items fit in 240px content vs 200px viewport — 4 items visible
    visible_calls = sum(1 for item in items if item.render.called)
    assert visible_calls >= 4
