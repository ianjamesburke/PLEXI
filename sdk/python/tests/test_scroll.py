from __future__ import annotations

import warnings

import pytest

from plexi_sdk.widgets.scroll import ScrollState


# ---------------------------------------------------------------------------
# 1. Fresh state
# ---------------------------------------------------------------------------

def test_fresh_state_offset_is_zero():
    s = ScrollState(content_height=200.0, viewport_height=50.0)
    assert s.offset == 0.0


def test_fresh_state_max_offset():
    s = ScrollState(content_height=200.0, viewport_height=50.0)
    assert s.max_offset == 150.0


# ---------------------------------------------------------------------------
# 2. scroll_by positive — advances offset
# ---------------------------------------------------------------------------

def test_scroll_by_positive():
    s = ScrollState(content_height=200.0, viewport_height=50.0)
    s.scroll_by(30.0)
    assert s.offset == 30.0


# ---------------------------------------------------------------------------
# 3. scroll_by negative — retreats offset
# ---------------------------------------------------------------------------

def test_scroll_by_negative():
    s = ScrollState(content_height=200.0, viewport_height=50.0, offset=80.0)
    s.scroll_by(-20.0)
    assert s.offset == 60.0


# ---------------------------------------------------------------------------
# 4. Clamp at top — offset cannot go below 0
# ---------------------------------------------------------------------------

def test_clamp_at_top():
    s = ScrollState(content_height=200.0, viewport_height=50.0, offset=10.0)
    s.scroll_by(-50.0)
    assert s.offset == 0.0


def test_initial_offset_negative_clamped():
    s = ScrollState(content_height=200.0, viewport_height=50.0, offset=-99.0)
    assert s.offset == 0.0


# ---------------------------------------------------------------------------
# 5. Clamp at bottom — offset cannot exceed max_offset
# ---------------------------------------------------------------------------

def test_clamp_at_bottom():
    s = ScrollState(content_height=200.0, viewport_height=50.0)
    s.scroll_by(9999.0)
    assert s.offset == s.max_offset
    assert s.offset == 150.0


def test_initial_offset_exceeds_max_clamped():
    s = ScrollState(content_height=200.0, viewport_height=50.0, offset=999.0)
    assert s.offset == 150.0


# ---------------------------------------------------------------------------
# 6. Content smaller than viewport → max_offset = 0, scroll is no-op
# ---------------------------------------------------------------------------

def test_content_smaller_than_viewport():
    s = ScrollState(content_height=40.0, viewport_height=100.0)
    assert s.max_offset == 0.0
    assert s.offset == 0.0
    s.scroll_by(20.0)
    assert s.offset == 0.0


def test_content_equal_to_viewport():
    s = ScrollState(content_height=100.0, viewport_height=100.0)
    assert s.max_offset == 0.0
    assert s.offset == 0.0


# ---------------------------------------------------------------------------
# 7. ensure_visible — y above viewport scrolls up
# ---------------------------------------------------------------------------

def test_ensure_visible_above_scrolls_up():
    # Viewport shows [100, 150). y=80 is above.
    s = ScrollState(content_height=300.0, viewport_height=50.0, offset=100.0)
    s.ensure_visible(80.0)
    assert s.offset == 80.0


# ---------------------------------------------------------------------------
# 8. ensure_visible — y below viewport scrolls down
# ---------------------------------------------------------------------------

def test_ensure_visible_below_scrolls_down():
    # Viewport shows [0, 50). y=70 is below.
    s = ScrollState(content_height=300.0, viewport_height=50.0, offset=0.0)
    s.ensure_visible(70.0)
    # offset should place y at bottom edge: offset = y - viewport = 70 - 50 = 20
    assert s.offset == 20.0


# ---------------------------------------------------------------------------
# 9. ensure_visible — y inside viewport is no-op
# ---------------------------------------------------------------------------

def test_ensure_visible_inside_is_noop():
    # Viewport shows [50, 100). y=70 is inside.
    s = ScrollState(content_height=300.0, viewport_height=50.0, offset=50.0)
    s.ensure_visible(70.0)
    assert s.offset == 50.0


# ---------------------------------------------------------------------------
# 10. ensure_visible — margin applied correctly on both sides
# ---------------------------------------------------------------------------

def test_ensure_visible_margin_top():
    # Viewport [100, 150). y=105, margin=10 → y < offset + margin (100+10=110)
    # → scroll up so y = offset + margin → offset = y - margin = 105 - 10 = 95
    s = ScrollState(content_height=300.0, viewport_height=50.0, offset=100.0)
    s.ensure_visible(105.0, margin=10.0)
    assert s.offset == 95.0


def test_ensure_visible_margin_bottom():
    # Viewport [0, 50). y=42, margin=10 → y > offset + viewport - margin (0+50-10=40)
    # → scroll down so y = offset + viewport - margin → offset = y - viewport + margin = 42 - 50 + 10 = 2
    s = ScrollState(content_height=300.0, viewport_height=50.0, offset=0.0)
    s.ensure_visible(42.0, margin=10.0)
    assert s.offset == 2.0


# ---------------------------------------------------------------------------
# 11. Shrinking content_height re-clamps offset
# ---------------------------------------------------------------------------

def test_shrink_content_reclamps_offset():
    s = ScrollState(content_height=300.0, viewport_height=50.0, offset=200.0)
    assert s.offset == 200.0
    s.content_height = 100.0  # new max_offset = 50
    assert s.max_offset == 50.0
    assert s.offset == 50.0


def test_shrink_content_below_viewport_zeroes_offset():
    s = ScrollState(content_height=300.0, viewport_height=50.0, offset=200.0)
    s.content_height = 30.0  # smaller than viewport → max_offset = 0
    assert s.offset == 0.0


# ---------------------------------------------------------------------------
# 12. Zero viewport_height is handled
# ---------------------------------------------------------------------------

def test_zero_viewport_height():
    s = ScrollState(content_height=100.0, viewport_height=0.0)
    # max_offset = content_height - 0 = 100
    assert s.max_offset == 100.0
    s.scroll_by(50.0)
    assert s.offset == 50.0


def test_zero_viewport_clamp():
    s = ScrollState(content_height=100.0, viewport_height=0.0)
    s.scroll_by(9999.0)
    assert s.offset == 100.0


# ---------------------------------------------------------------------------
# 13. Negative dimension input — clamped with warning (not ValueError)
# ---------------------------------------------------------------------------

def test_negative_content_height_clamped():
    with warnings.catch_warnings(record=True) as w:
        warnings.simplefilter("always")
        s = ScrollState(content_height=-10.0, viewport_height=50.0)
        assert len(w) == 1
        assert "content_height" in str(w[0].message)
    assert s.content_height == 0.0
    assert s.max_offset == 0.0


def test_negative_viewport_height_clamped():
    with warnings.catch_warnings(record=True) as w:
        warnings.simplefilter("always")
        s = ScrollState(content_height=100.0, viewport_height=-5.0)
        assert len(w) == 1
        assert "viewport_height" in str(w[0].message)
    assert s.viewport_height == 0.0


# ---------------------------------------------------------------------------
# 14. scroll_to clamps correctly
# ---------------------------------------------------------------------------

def test_scroll_to_within_range():
    s = ScrollState(content_height=200.0, viewport_height=50.0)
    s.scroll_to(75.0)
    assert s.offset == 75.0


def test_scroll_to_above_max_clamps():
    s = ScrollState(content_height=200.0, viewport_height=50.0)
    s.scroll_to(9999.0)
    assert s.offset == 150.0


def test_scroll_to_negative_clamps():
    s = ScrollState(content_height=200.0, viewport_height=50.0)
    s.scroll_to(-1.0)
    assert s.offset == 0.0


# ---------------------------------------------------------------------------
# 15. Grow viewport_height shrinks max_offset and re-clamps
# ---------------------------------------------------------------------------

def test_grow_viewport_reclamps_offset():
    s = ScrollState(content_height=200.0, viewport_height=50.0, offset=140.0)
    s.viewport_height = 100.0  # new max_offset = 100
    assert s.max_offset == 100.0
    assert s.offset == 100.0
