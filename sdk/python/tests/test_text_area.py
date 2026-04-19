"""Snapshot tests for TextArea widget.

Tests are split into two categories:
  - DrawCommand assertions: deterministic, font-independent. These verify
    that the widget emits correctly positioned rects/text dicts.
  - Pixel assertions: end-to-end, invokes plexi_render. Used sparingly
    for one smoke test to prove the full path works.
"""
from __future__ import annotations

import os
from pathlib import Path

import pytest

from plexi_sdk.widgets.text_buffer import TextBuffer
from plexi_sdk.widgets.text_area import TextArea, TextAreaTheme
from plexi_sdk.testing import render_draw_commands, assert_pixel, save_snapshot


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _default_theme() -> TextAreaTheme:
    return TextAreaTheme(
        background="#1e1e2e",
        foreground="#cdd6f4",
        cursor="#cdd6f4",
        selection="#45475a",
        line_height=20.0,
        font_size=14.0,
        char_width=8.4,
        padding=8.0,
    )


def _make_area(text: str = "", theme: TextAreaTheme | None = None) -> TextArea:
    return TextArea(TextBuffer(text), theme=theme or _default_theme())


def _cursor_rect(cmds: list[dict]) -> dict | None:
    """Return the first 2-px-wide rect (the cursor command), or None."""
    for cmd in cmds:
        if cmd.get("type") == "rect" and cmd.get("w") == 2.0:
            return cmd
    return None


def _selection_rects(cmds: list[dict], theme: TextAreaTheme) -> list[dict]:
    """Return rects whose fill matches theme.selection (not background, not cursor)."""
    return [
        c for c in cmds
        if c.get("type") == "rect"
        and c.get("fill") == theme.selection
    ]


# ---------------------------------------------------------------------------
# 1. Background fills the viewport
# ---------------------------------------------------------------------------

def test_background_fills_viewport():
    area = _make_area("")
    cmds = area.render(0, 0, 200, 100)

    bg = [c for c in cmds if c.get("type") == "rect" and c.get("fill") == area.theme.background]
    assert len(bg) >= 1, "No background rect found"
    bg0 = bg[0]
    assert bg0["x"] == 0 and bg0["y"] == 0
    assert bg0["w"] == 200 and bg0["h"] == 100


# ---------------------------------------------------------------------------
# 2. Cursor at origin
# ---------------------------------------------------------------------------

def test_cursor_visible_at_origin():
    t = _default_theme()
    area = _make_area("hello\nworld", t)
    # cursor defaults to (0, 0)
    assert area.buffer.cursor.row == 0
    assert area.buffer.cursor.col == 0

    cmds = area.render(0, 0, 200, 80)
    cur = _cursor_rect(cmds)
    assert cur is not None, "No cursor rect found"

    expected_x = t.padding + 0 * t.char_width  # col 0
    expected_y = t.padding + 0 * t.line_height  # row 0
    assert cur["x"] == pytest.approx(expected_x), f"cursor x: {cur['x']} != {expected_x}"
    assert cur["y"] == pytest.approx(expected_y), f"cursor y: {cur['y']} != {expected_y}"


# ---------------------------------------------------------------------------
# 3. Cursor at (row=1, col=2)
# ---------------------------------------------------------------------------

def test_cursor_at_1_2():
    t = _default_theme()
    area = _make_area("hello\nworld", t)
    area.buffer.set_cursor(1, 2)

    cmds = area.render(0, 0, 200, 80)
    cur = _cursor_rect(cmds)
    assert cur is not None, "No cursor rect found"

    expected_x = t.padding + 2 * t.char_width
    expected_y = t.padding + 1 * t.line_height
    assert cur["x"] == pytest.approx(expected_x), f"cursor x: {cur['x']} != {expected_x}"
    assert cur["y"] == pytest.approx(expected_y), f"cursor y: {cur['y']} != {expected_y}"


# ---------------------------------------------------------------------------
# 4. Pixel test: cursor appears in rendered image
# ---------------------------------------------------------------------------

def test_pixel_cursor_appears():
    t = _default_theme()
    area = _make_area("hello", t)
    # cursor at (0, 0) — top-left of text area

    cmds = area.render(0, 0, 200, 60)

    png = render_draw_commands(cmds, 200, 60, background=t.background)
    assert png[:4] == b"\x89PNG", "Expected valid PNG"

    # Cursor should be at (padding, padding + line_height/2).
    # Use a point slightly inside the 2px-wide cursor rect.
    cursor_px_x = int(t.padding)   # cursor left edge
    cursor_px_y = int(t.padding + t.line_height / 2)  # vertically centered in line

    # The cursor colour is the same as foreground (#cdd6f4 = 205, 214, 244).
    # Use a moderate tolerance since the renderer may anti-alias.
    assert_pixel(png, cursor_px_x, cursor_px_y, t.cursor, tolerance=20)


# ---------------------------------------------------------------------------
# 5. Selection on a single line
# ---------------------------------------------------------------------------

def test_selection_single_line():
    t = _default_theme()
    area = _make_area("hello", t)
    area.buffer.set_cursor(0, 1)
    area.buffer.select_to(0, 4)   # selects "ell"

    cmds = area.render(0, 0, 200, 60)
    sel = _selection_rects(cmds, t)

    assert len(sel) == 1, f"Expected 1 selection rect, got {len(sel)}: {sel}"
    r = sel[0]

    expected_x = t.padding + 1 * t.char_width      # starts at col 1
    expected_w = 3 * t.char_width                   # spans cols 1..4 (3 chars)
    assert r["x"] == pytest.approx(expected_x, abs=0.1), f"sel x: {r['x']} != {expected_x}"
    assert r["w"] == pytest.approx(expected_w, abs=0.1), f"sel w: {r['w']} != {expected_w}"


# ---------------------------------------------------------------------------
# 6. Selection spanning three lines
# ---------------------------------------------------------------------------

def test_selection_spans_three_lines():
    t = _default_theme()
    area = _make_area("abc\ndef\nghi", t)
    area.buffer.set_cursor(0, 1)
    area.buffer.select_to(2, 2)   # from (0,1) to (2,2)

    cmds = area.render(0, 0, 300, 120)
    sel = _selection_rects(cmds, t)

    assert len(sel) == 3, f"Expected 3 selection rects, got {len(sel)}: {sel}"

    # Sort by y to get consistent ordering.
    sel_sorted = sorted(sel, key=lambda r: r["y"])

    # First rect: starts at col 1 of row 0.
    first = sel_sorted[0]
    assert first["x"] == pytest.approx(t.padding + 1 * t.char_width, abs=0.1), \
        f"first sel x: {first['x']}"

    # Last rect: starts at col 0 (beginning of line).
    last = sel_sorted[2]
    assert last["x"] == pytest.approx(t.padding + 0 * t.char_width, abs=0.1), \
        f"last sel x: {last['x']}"


# ---------------------------------------------------------------------------
# 7. Snapshot file smoke test
# ---------------------------------------------------------------------------

def test_snapshot_file():
    t = _default_theme()
    area = _make_area("hello\nworld", t)
    area.buffer.set_cursor(0, 0)

    cmds = area.render(0, 0, 200, 80)
    png = render_draw_commands(cmds, 200, 80, background=t.background)
    assert png[:4] == b"\x89PNG"

    snapshot_path = Path(__file__).parent / "snapshots" / "text_area_hello.png"
    save_snapshot(png, snapshot_path)
    assert snapshot_path.exists(), "Snapshot file was not written"

    # Re-render and assert three characteristic pixels.
    # 1. Background in top-right corner (outside text/cursor).
    #    x=180, y=4 — well to the right, above text.
    assert_pixel(png, 180, 4, t.background, tolerance=10)

    # 2. Background at far right, below last line of text.
    #    y=70 is below "world" (which ends at padding + 2*line_height = 8+40=48).
    assert_pixel(png, 180, 70, t.background, tolerance=10)

    # 3. Cursor pixel (same as test_pixel_cursor_appears).
    cursor_px_x = int(t.padding)
    cursor_px_y = int(t.padding + t.line_height / 2)
    assert_pixel(png, cursor_px_x, cursor_px_y, t.cursor, tolerance=20)
