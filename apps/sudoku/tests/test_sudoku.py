"""Sudoku hit-testing unit tests.

Covers the pure coordinate -> target mapping used by the MouseEvent handler
(stint 0397/0394) — no host, no subprocess, no canvas rendering required.
"""

from __future__ import annotations

import os
import sys

sys.path.insert(
    0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python")
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import main as sudoku  # noqa: E402


def test_diff_button_hit_at_each_center():
    w, h = 800.0, 600.0
    rects = sudoku._diff_button_rects(w, h)
    assert len(rects) == 3
    for i, (bx, by, bw, bh) in enumerate(rects):
        indexed = [(j, rx, ry, rw, rh) for j, (rx, ry, rw, rh) in enumerate(rects)]
        cx, cy = bx + bw / 2, by + bh / 2
        assert sudoku._hit_rect(cx, cy, indexed) == i


def test_diff_button_miss_outside_all_buttons():
    w, h = 800.0, 600.0
    rects = [(i, bx, by, bw, bh) for i, (bx, by, bw, bh) in enumerate(sudoku._diff_button_rects(w, h))]
    assert sudoku._hit_rect(0.0, 0.0, rects) is None
    assert sudoku._hit_rect(w, h, rects) is None


def test_num_button_hit_maps_to_number():
    d = sudoku._new_game("easy")
    sidebar_w, cell = 120.0, 40.0
    _, num_rects = sudoku._draw_sidebar_canvas(d, 500.0, 8.0, sidebar_w, cell)
    assert len(num_rects) == 9
    for num, bx, by, bw, bh in num_rects:
        cx, cy = bx + bw / 2, by + bh / 2
        assert sudoku._hit_rect(cx, cy, num_rects) == num


def test_num_button_miss_between_buttons_returns_none():
    d = sudoku._new_game("easy")
    sidebar_w, cell = 120.0, 40.0
    _, num_rects = sudoku._draw_sidebar_canvas(d, 500.0, 8.0, sidebar_w, cell)
    assert sudoku._hit_rect(-1000.0, -1000.0, num_rects) is None


def test_cell_hit_maps_to_selected_cell():
    ox, oy, cell = 10.0, 8.0, 40.0
    for r in range(9):
        for c in range(9):
            cx = ox + c * cell + cell / 2
            cy = oy + r * cell + cell / 2
            assert sudoku._hit_cell(cx, cy, ox, oy, cell) == (r, c)


def test_cell_hit_boundary_is_linear_not_coincidental():
    """Check several points near cell boundaries, not just centers, to prove
    the row/col math is a real division, not a fluke that only lines up at
    one sampled point per cell."""
    ox, oy, cell = 10.0, 8.0, 40.0

    # Last pixel inside cell (2, 2) still maps to (2, 2).
    assert sudoku._hit_cell(ox + 2 * cell + cell - 0.01, oy + 2 * cell + cell - 0.01, ox, oy, cell) == (2, 2)
    # First pixel of the next cell over maps to (2, 3).
    assert sudoku._hit_cell(ox + 3 * cell, oy + 2 * cell, ox, oy, cell) == (2, 3)
    # First pixel of the next row down maps to (3, 2).
    assert sudoku._hit_cell(ox + 2 * cell, oy + 3 * cell, ox, oy, cell) == (3, 2)
    # Top-left corner of the grid maps to (0, 0).
    assert sudoku._hit_cell(ox, oy, ox, oy, cell) == (0, 0)
    # Last pixel of the whole grid maps to (8, 8).
    assert sudoku._hit_cell(ox + 9 * cell - 0.01, oy + 9 * cell - 0.01, ox, oy, cell) == (8, 8)


def test_cell_hit_outside_grid_is_none():
    ox, oy, cell = 10.0, 8.0, 40.0
    gw = cell * 9
    assert sudoku._hit_cell(ox - 1.0, oy, ox, oy, cell) is None
    assert sudoku._hit_cell(ox, oy - 1.0, ox, oy, cell) is None
    assert sudoku._hit_cell(ox + gw, oy, ox, oy, cell) is None
    assert sudoku._hit_cell(ox, oy + gw, ox, oy, cell) is None


def test_mouse_menu_click_starts_game_with_clicked_difficulty():
    sudoku._canvas_width = 800.0
    sudoku._canvas_height = 600.0
    d = sudoku._blank()
    bx, by, bw, bh = sudoku._diff_button_rects(800.0, 600.0)[1]
    effects = sudoku._mouse(d, bx + bw / 2, by + bh / 2)
    assert len(effects) == 2
    new_state = effects[0].data
    assert new_state["screen"] == "game"
    assert new_state["difficulty"] == "medium"


def test_mouse_game_click_selects_cell():
    sudoku._canvas_width = 800.0
    sudoku._canvas_height = 600.0
    d = sudoku._new_game("easy")
    sidebar_w, cell = sudoku._compute_layout()
    pane_w = sudoku._canvas_width or 800.0
    gw = cell * 9
    pair_w = gw + 12.0 + sidebar_w
    ox = max(8.0, (pane_w - pair_w) / 2.0)
    oy = 8.0
    effects = sudoku._mouse(d, ox + cell / 2, oy + cell / 2)
    assert len(effects) == 1
    assert effects[0].data == {"sel_r": 0, "sel_c": 0, "sel_num": d["board"][0][0]}
