"""End-to-end sanity tests for the headless snapshot infrastructure."""
from __future__ import annotations

import pytest
from plexi_sdk.testing import (
    assert_pixel,
    hex_to_rgba,
    render_draw_commands,
)


def test_render_rect_then_assert_pixel():
    cmds = [{"type": "rect", "x": 10, "y": 10, "w": 80, "h": 80,
             "fill": "#ff0000", "radius": 0}]
    png = render_draw_commands(cmds, 100, 100, background="#000000")

    # PNG header sanity.
    assert png[:4] == b"\x89PNG", "render_draw_commands must return a valid PNG"

    # Center of the rect (x=50, y=50) should be red.
    assert_pixel(png, 50, 50, "#ff0000")

    # Outside the rect (x=5, y=5) should be background black.
    assert_pixel(png, 5, 5, "#000000")


def test_hex_to_rgba_variants():
    assert hex_to_rgba("#ff0000") == (255, 0, 0, 255)
    assert hex_to_rgba("#0f0") == (0, 255, 0, 255)
    assert hex_to_rgba("#80808080") == (128, 128, 128, 128)


def test_assert_pixel_tolerance():
    """Tolerance allows small per-channel drift (e.g. alpha compositing)."""
    cmds = [{"type": "rect", "x": 0, "y": 0, "w": 50, "h": 50,
             "fill": "#ff0000", "radius": 0}]
    png = render_draw_commands(cmds, 100, 100, background="#000000")
    # With tolerance=4, (255, 0, 0, 255) ≈ "#ff0000" is fine.
    assert_pixel(png, 25, 25, "#ff0000", tolerance=4)


def test_assert_pixel_mismatch_raises():
    """assert_pixel raises AssertionError when the pixel is clearly wrong."""
    cmds = [{"type": "rect", "x": 0, "y": 0, "w": 100, "h": 100,
             "fill": "#0000ff", "radius": 0}]
    png = render_draw_commands(cmds, 100, 100, background="#000000")
    with pytest.raises(AssertionError, match="mismatch"):
        assert_pixel(png, 50, 50, "#ff0000", tolerance=4)


def test_render_multiple_commands():
    """Multiple rect commands accumulate correctly."""
    cmds = [
        {"type": "rect", "x": 0, "y": 0, "w": 50, "h": 100, "fill": "#00ff00", "radius": 0},
        {"type": "rect", "x": 50, "y": 0, "w": 50, "h": 100, "fill": "#0000ff", "radius": 0},
    ]
    png = render_draw_commands(cmds, 100, 100, background="#000000")
    assert_pixel(png, 25, 50, "#00ff00")   # left half: green
    assert_pixel(png, 75, 50, "#0000ff")   # right half: blue
