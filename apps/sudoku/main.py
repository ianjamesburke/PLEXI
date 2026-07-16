#!/usr/bin/env python3
"""Sudoku — Easy, Medium, and Hard difficulties."""

from __future__ import annotations

import random

import plexi_sdk as sdk
from plexi_sdk import dim, state, theme
from plexi_sdk.effects import SetState, SetStatus, SetTimer, SetTitle
from plexi_sdk.events import KeyEvent, MouseEvent, Resize, TimerFired
from plexi_sdk.ui import (
    AppBar, Canvas, CanvasRect, CanvasText, Column,
    FooterKeys,
)

TIMER_ID = 1
SIDEBAR_W_MAX = 148
SIDEBAR_W_MIN = 100
CLUES = {"easy": 46, "medium": 34, "hard": 26}
DIFFICULTIES = ["easy", "medium", "hard"]
DIFF_TONE = {"easy": "success", "medium": "warning", "hard": "danger"}

# ── Sudoku generation ──────────────────────────────────────────────────────

def _fill(grid, pos):
    if pos == 81:
        return True
    r, c = divmod(pos, 9)
    nums = list(range(1, 10))
    random.shuffle(nums)
    for n in nums:
        if _ok(grid, r, c, n):
            grid[r][c] = n
            if _fill(grid, pos + 1):
                return True
            grid[r][c] = 0
    return False

def _ok(grid, r, c, n):
    if n in grid[r]:
        return False
    for i in range(9):
        if grid[i][c] == n:
            return False
    br, bc = (r // 3) * 3, (c // 3) * 3
    for dr in range(3):
        for dc in range(3):
            if grid[br + dr][bc + dc] == n:
                return False
    return True

def _new_game(difficulty):
    solution = [[0] * 9 for _ in range(9)]
    _fill(solution, 0)
    board = [row[:] for row in solution]
    given = [[True] * 9 for _ in range(9)]
    cells = list(range(81))
    random.shuffle(cells)
    to_remove = 81 - CLUES[difficulty]
    for pos in cells[:to_remove]:
        r, c = divmod(pos, 9)
        board[r][c] = 0
        given[r][c] = False
    return {
        "screen": "game",
        "difficulty": difficulty,
        "diff_idx": DIFFICULTIES.index(difficulty),
        "board": board,
        "solution": solution,
        "given": given,
        "sel_r": -1,
        "sel_c": -1,
        "errors": [[False] * 9 for _ in range(9)],
        "seconds": 0,
        "paused": False,
        "complete": False,
        "notes": [[[False] * 9 for _ in range(9)] for _ in range(9)],
        "notes_mode": False,
        "sel_num": 0,
    }

def _blank():
    return {
        "screen": "menu",
        "difficulty": "easy",
        "diff_idx": 0,
        "board": [[0] * 9 for _ in range(9)],
        "solution": [[0] * 9 for _ in range(9)],
        "given": [[False] * 9 for _ in range(9)],
        "sel_r": -1,
        "sel_c": -1,
        "errors": [[False] * 9 for _ in range(9)],
        "seconds": 0,
        "paused": False,
        "complete": False,
        "notes": [[[False] * 9 for _ in range(9)] for _ in range(9)],
        "notes_mode": False,
        "sel_num": 0,
    }

def _load():
    blank = _blank()
    d = {}
    for k, v in blank.items():
        d[k] = state.get(k, v)
    return d

# ── State helpers ──────────────────────────────────────────────────────────

def _errors(board, solution, given):
    out = [[False] * 9 for _ in range(9)]
    for r in range(9):
        for c in range(9):
            if not given[r][c] and board[r][c] != 0 and board[r][c] != solution[r][c]:
                out[r][c] = True
    return out

def _complete(board, _given, solution):
    for r in range(9):
        for c in range(9):
            if board[r][c] != solution[r][c]:
                return False
    return True

def _fmt(secs):
    m, s = divmod(int(secs), 60)
    return f"{m:02d}:{s:02d}"

# ── Grid layout ────────────────────────────────────────────────────────────

def _count_numbers(board):
    counts = [0] * 9
    for r in range(9):
        for c in range(9):
            v = board[r][c]
            if 1 <= v <= 9:
                counts[v - 1] += 1
    return counts

def _enter_number(d, num):
    sel_r = int(d.get("sel_r", -1))
    sel_c = int(d.get("sel_c", -1))
    # Always select this number for highlighting, even if no cell is active
    if sel_r < 0 or sel_c < 0:
        return [SetState({"sel_num": num})]
    given = d.get("given", [[True] * 9] * 9)
    if given[sel_r][sel_c]:
        return [SetState({"sel_num": num})]
    board = [list(row) for row in d.get("board", [[0] * 9] * 9)]
    solution = d.get("solution", [[0] * 9] * 9)
    notes = [[list(nc) for nc in row] for row in d.get("notes", [[[False]*9]*9]*9)]
    notes_mode = bool(d.get("notes_mode", False))
    if notes_mode:
        notes[sel_r][sel_c][num - 1] = not notes[sel_r][sel_c][num - 1]
        return [SetState({"notes": notes, "sel_num": num})]
    board[sel_r][sel_c] = num
    notes[sel_r][sel_c] = [False] * 9
    for i in range(9):
        notes[sel_r][i][num - 1] = False
        notes[i][sel_c][num - 1] = False
    br, bc = (sel_r // 3) * 3, (sel_c // 3) * 3
    for dr in range(3):
        for dc in range(3):
            notes[br + dr][bc + dc][num - 1] = False
    errs = _errors(board, solution, given)
    done = _complete(board, given, solution)
    new_state = {"board": board, "errors": errs, "complete": done, "notes": notes, "sel_num": num}
    if done:
        new_state["screen"] = "win"
        secs = int(d.get("seconds", 0))
        return [SetState(new_state), SetStatus(f"Solved in {_fmt(secs)}!")]
    return [SetState(new_state)]

# ── Lifecycle ─────────────────────────────────────────────────────────────

def init(size, _args):
    sdk.canvas_width, sdk.canvas_height = size
    sdk.pane_width, sdk.pane_height = size
    missing = {k: v for k, v in _blank().items() if state.get(k) is None}
    effects = [
        SetTitle("Sudoku"),
        SetStatus("Choose difficulty"),
        SetTimer(TIMER_ID, 1000, repeat=True),
    ]
    if missing:
        effects.append(SetState(missing))
    return effects

def update(event):
    d = _load()
    screen = str(d.get("screen", "menu"))

    if isinstance(event, TimerFired) and event.id == TIMER_ID:
        if screen == "game" and not d.get("paused") and not d.get("complete"):
            secs = int(d.get("seconds", 0)) + 1
            return [SetState({"seconds": secs}), SetStatus(_fmt(secs))]
        return []

    if isinstance(event, Resize):
        sdk.canvas_width = event.width
        sdk.canvas_height = event.height
        sdk.pane_width = event.width
        sdk.pane_height = event.height
        return []

    if isinstance(event, MouseEvent) and event.pressed:
        return _mouse(d, event.x, event.y)

    if isinstance(event, KeyEvent) and event.pressed:
        return _key(d, event)

    return []

# ── Input handlers ─────────────────────────────────────────────────────────

def _mouse(d, x, y):
    if str(d.get("screen")) != "game":
        w = sdk.canvas_width or 800.0
        h = sdk.canvas_height or 600.0
        for i, (bx, by, bw, bh) in enumerate(_diff_button_rects(w, h)):
            if bx <= x < bx + bw and by <= y < by + bh:
                diff = DIFFICULTIES[i]
                return [SetState(_new_game(diff)), SetStatus(f"{diff.title()} — 00:00")]
        return []

    sidebar_w, cell = _compute_layout()
    pane_w = sdk.canvas_width or sdk.pane_width or 800.0
    gw = cell * 9
    pair_w = gw + 12.0 + sidebar_w
    ox = max(8.0, (pane_w - pair_w) / 2.0)
    oy = 8.0
    sx = ox + gw + 12.0

    if ox <= x < ox + gw and oy <= y < oy + gw:
        r = max(0, min(8, int((y - oy) // cell)))
        c = max(0, min(8, int((x - ox) // cell)))
        board = d.get("board", [[0] * 9] * 9)
        return [SetState({"sel_r": r, "sel_c": c, "sel_num": board[r][c]})]

    _, num_rects = _draw_sidebar_canvas(d, sx, oy, sidebar_w, cell)
    for num, bx, by, bw, bh in num_rects:
        if bx <= x < bx + bw and by <= y < by + bh:
            return _enter_number(d, num)
    return []

def _key(d, event):
    key = event.key
    screen = str(d.get("screen", "menu"))

    if screen == "menu":
        idx = int(d.get("diff_idx", 0))
        if key in ("up", "k"):
            return [SetState({"diff_idx": (idx - 1) % 3})]
        if key in ("down", "j"):
            return [SetState({"diff_idx": (idx + 1) % 3})]
        if key in ("return", "space"):
            new = _new_game(DIFFICULTIES[idx])
            return [SetState(new), SetStatus(f"{DIFFICULTIES[idx].title()} — 00:00")]
        if key == "1":
            new = _new_game("easy")
            return [SetState(new), SetStatus("Easy — 00:00")]
        if key == "2":
            new = _new_game("medium")
            return [SetState(new), SetStatus("Medium — 00:00")]
        if key == "3":
            new = _new_game("hard")
            return [SetState(new), SetStatus("Hard — 00:00")]
        return []

    if screen in ("game", "win"):
        return _game_key(d, event)

    return []

def _game_key(d, event):
    key = event.key
    shift = event.modifiers.shift
    screen = str(d.get("screen", "game"))
    sel_r = int(d.get("sel_r", -1))
    sel_c = int(d.get("sel_c", -1))
    paused = bool(d.get("paused", False))
    notes_mode = bool(d.get("notes_mode", False))

    # Always-active keys
    if key == "m":
        blank = _blank()
        return [SetState(blank), SetStatus("Choose difficulty")]
    if key == "r":
        new = _new_game(str(d.get("difficulty", "easy")))
        return [SetState(new), SetStatus(f"{new['difficulty'].title()} — 00:00")]

    if screen == "win":
        if key in ("n", "m"):
            blank = _blank()
            return [SetState(blank), SetStatus("Choose difficulty")]
        return []

    if key == "p":
        return [SetState({"paused": not paused})]
    if key == "n":
        return [SetState({"notes_mode": not notes_mode})]

    if paused:
        return []

    # Navigation — shift+arrow jumps a full 3-cell box
    moves = {"up": (-1, 0), "k": (-1, 0), "down": (1, 0), "j": (1, 0),
              "left": (0, -1), "h": (0, -1), "right": (0, 1), "l": (0, 1)}
    if key in moves:
        dr, dc = moves[key]
        step = 3 if shift and key in ("up", "down", "left", "right") else 1
        nr = max(0, min(8, sel_r + dr * step)) if sel_r >= 0 else 0
        nc = max(0, min(8, sel_c + dc * step)) if sel_c >= 0 else 0
        if sel_r < 0:
            nr, nc = 4, 4
        return [SetState({"sel_r": nr, "sel_c": nc})]

    if sel_r < 0 or sel_c < 0:
        return []

    given = d.get("given", [[True] * 9] * 9)
    if given[sel_r][sel_c]:
        return []

    board = [list(row) for row in d.get("board", [[0] * 9] * 9)]
    solution = d.get("solution", [[0] * 9] * 9)
    if key in ("backspace", "delete"):
        board[sel_r][sel_c] = 0
        return [SetState({"board": board, "errors": _errors(board, solution, given), "sel_num": 0})]

    if key in "123456789" and len(key) == 1:
        return _enter_number(d, int(key))

    return []

# ── View ───────────────────────────────────────────────────────────────────

def _compute_layout():
    """Return (sidebar_w, cell) from current pane dimensions."""
    pane_w = sdk.pane_width or 800.0
    pane_h = sdk.pane_height or 600.0
    sidebar_w = max(SIDEBAR_W_MIN, min(SIDEBAR_W_MAX, int(pane_w * 0.18)))
    body_h = pane_h - 76.0  # minus AppBar + footer
    max_grid_w = pane_w - sidebar_w - 20.0
    cell = max(20.0, min((max_grid_w - 4.0) / 9.0, (body_h - 16.0) / 9.0))
    return sidebar_w, cell


def view():
    d = _load()
    screen = str(d.get("screen", "menu"))
    if screen == "menu":
        footer = FooterKeys([("↑↓", "select"), ("enter", "start"), ("1", "easy"), ("2", "medium"), ("3", "hard")])
    elif d.get("paused"):
        footer = FooterKeys([("p", "resume"), ("r", "restart"), ("m", "menu")])
    elif screen == "win":
        footer = FooterKeys([("r", "restart"), ("n", "new game"), ("m", "menu")])
    else:
        keys = [("1-9", "fill"), ("n", "notes"), ("p", "pause"), ("r", "restart"), ("m", "menu")]
        footer = FooterKeys(keys)
    screen = str(d.get("screen", "menu"))
    w, h = sdk.canvas_width, sdk.canvas_height
    if screen != "menu":
        # Single canvas for the entire game body: grid + sidebar drawn together.
        # This avoids HStack grow-distribution fighting with fixed-size children.
        body = Canvas(_draw_game(d), width=w, height=h, grow=True)
    else:
        body = Canvas(_draw_menu(d), width=w, height=h, grow=True)

    return Column([
        AppBar("Sudoku"),
        body,
        footer,
    ], padding=0, gap=0, grow=True)


# ── Shared drawing helpers ─────────────────────────────────────────────────

def _bordered_rect(x, y, w, h, fill, radius=0.0, border_color=None, border_width=0.0):
    """CanvasRect no longer takes border_color/border_width; draw the border
    as an outline rect behind an inset fill rect instead."""
    if not border_color or border_width <= 0:
        return [CanvasRect(x, y, w, h, fill, radius=radius)]
    return [
        CanvasRect(x, y, w, h, border_color, radius=radius),
        CanvasRect(x + border_width, y + border_width, w - 2 * border_width, h - 2 * border_width,
                   fill, radius=max(0.0, radius - border_width)),
    ]

# ── Menu drawing ───────────────────────────────────────────────────────────

def _diff_button_rects(w, h):
    bw, bh = 220.0, 64.0
    bx = (w - bw) / 2
    by_start = h / 2 - 80.0
    return [(bx, by_start + i * 80.0, bw, bh) for i in range(3)]

def _draw_menu(d):
    w = sdk.canvas_width
    h = sdk.canvas_height
    diff_idx = int(d.get("diff_idx", 0))
    cmds = []

    title_y = h / 2 - 160.0
    cmds.append(CanvasText(w / 2, title_y, "SUDOKU", size=36.0, color=theme.fg, bold=True, align="center_center"))
    cmds.append(CanvasText(w / 2, title_y + 40.0, "Choose your difficulty", size=13.0, color=theme.muted, align="center_center"))

    labels = [("Easy", f"{CLUES['easy']} clues"), ("Medium", f"{CLUES['medium']} clues"), ("Hard", f"{CLUES['hard']} clues")]
    tones = [theme.success, theme.warning, theme.danger]
    rects = _diff_button_rects(w, h)

    for i, (label, subtitle) in enumerate(labels):
        bx, by, bw, bh = rects[i]
        selected = i == diff_idx
        bg = theme.surface if selected else theme.bg
        border = tones[i] if selected else theme.highlight
        cmds.extend(_bordered_rect(bx, by, bw, bh, bg, radius=8.0, border_color=border, border_width=2.0))
        text_color = tones[i] if selected else theme.fg
        cmds.append(CanvasText(bx + bw / 2, by + 22.0, label, size=17.0, color=text_color, bold=selected, align="center_center"))
        cmds.append(CanvasText(bx + bw / 2, by + 44.0, subtitle, size=11.0, color=theme.muted, align="center_center"))

    return cmds

# ── Game drawing ───────────────────────────────────────────────────────────

def _draw_sidebar_canvas(d, sx, oy, sidebar_w, cell):
    """Sidebar content as canvas commands anchored at (sx, oy)."""
    difficulty = str(d.get("difficulty", "easy"))
    seconds = int(d.get("seconds", 0))
    notes_mode = bool(d.get("notes_mode", False))
    board = d.get("board", [[0] * 9] * 9)
    given = d.get("given", [[True] * 9] * 9)
    sel_num = int(d.get("sel_num", 0))
    counts = _count_numbers(board)
    filled = sum(1 for r in range(9) for c in range(9) if not given[r][c] and board[r][c] != 0)
    blanks = sum(1 for r in range(9) for c in range(9) if not given[r][c])
    d_color = getattr(theme, DIFF_TONE.get(difficulty, "accent"))

    cmds = []
    y = oy

    def section(label):
        nonlocal y
        y += 6
        cmds.append(CanvasText(sx, y, label.upper(), size=10.0, color=theme.muted, bold=True, align="left_top"))
        y += 13
        cmds.append(CanvasRect(sx, y, sidebar_w, 1.0, theme.highlight))
        y += 5

    section("DIFFICULTY")
    cmds.append(CanvasText(sx, y, difficulty.upper(), size=15.0, color=d_color, bold=True, align="left_top"))
    y += 22

    section("TIME")
    cmds.append(CanvasText(sx, y, _fmt(seconds), size=20.0, color=theme.fg, bold=True, align="left_top"))
    y += 28

    if notes_mode:
        cmds.append(CanvasText(sx, y, "NOTES ON", size=12.0, color=theme.warning, bold=True, align="left_top"))
        y += 20

    if blanks > 0:
        section("PROGRESS")
        cmds.append(CanvasText(sx, y, f"{filled}/{blanks}", size=13.0, color=theme.accent, align="left_top"))
        y += 20

    section("NUMBERS")
    gap = 5.0
    bw = bh = max(24.0, (sidebar_w - 2 * gap) / 3.0)
    num_rects = []
    for i in range(9):
        num = i + 1
        col_i = i % 3
        row_i = i // 3
        bx = sx + col_i * (bw + gap)
        by = y + row_i * (bh + gap)
        num_rects.append((num, bx, by, bw, bh))
        done_num = counts[num - 1] >= 9
        is_sel = num == sel_num and sel_num != 0
        if done_num:
            bg, text_col, border = theme.bg, theme.muted, theme.border
        elif is_sel:
            bg, text_col, border = theme.highlight, theme.accent, theme.accent
        else:
            bg, text_col, border = theme.surface, theme.fg, theme.highlight
        cmds.extend(_bordered_rect(bx, by, bw, bh, bg, radius=4.0, border_color=border, border_width=1.0))
        cmds.append(CanvasText(bx + bw / 2, by + bh / 2, str(num), size=15.0,
                               color=text_col, bold=not done_num, align="center_center"))
    return cmds, num_rects


def _draw_game(d):
    """Full game body: grid + sidebar on one canvas, sidebar flush right of grid."""
    sidebar_w, cell = _compute_layout()
    pane_w = sdk.canvas_width or sdk.pane_width or 800.0
    gw = cell * 9
    pair_w = gw + 12.0 + sidebar_w
    ox = max(8.0, (pane_w - pair_w) / 2.0)
    oy = 8.0
    sx = ox + gw + 12.0

    sidebar_cmds, _ = _draw_sidebar_canvas(d, sx, oy, sidebar_w, cell)
    return _draw_grid(d, cell, ox, oy) + sidebar_cmds


def _draw_grid(d, cell=None, ox=6.0, oy=8.0):
    """Canvas commands for the sudoku grid."""
    board = d.get("board", [[0] * 9] * 9)
    given = d.get("given", [[True] * 9] * 9)
    errors = d.get("errors", [[False] * 9] * 9)
    notes = d.get("notes", [[[False] * 9] * 9] * 9)
    sel_r = int(d.get("sel_r", -1))
    sel_c = int(d.get("sel_c", -1))
    paused = bool(d.get("paused", False))
    complete = bool(d.get("complete", False))
    screen = str(d.get("screen", "game"))
    seconds = int(d.get("seconds", 0))
    sel_num = int(d.get("sel_num", 0))

    if cell is None:
        _, cell = _compute_layout()
    gw = cell * 9
    gh = cell * 9
    cmds = []

    # Grid shadow/bg
    cmds.append(CanvasRect(ox - 3, oy - 3, gw + 6, gh + 6, theme.bg_darkest, radius=6.0))
    cmds.append(CanvasRect(ox, oy, gw, gh, theme.bg, radius=4.0))

    # Row + col cross highlight only — no box tint.
    if sel_r >= 0 and sel_c >= 0 and not paused and screen == "game":
        cmds.append(CanvasRect(ox, oy + sel_r * cell, gw, cell, theme.surface))
        cmds.append(CanvasRect(ox + sel_c * cell, oy, cell, gh, theme.surface))

    # Same-number highlight
    if sel_num != 0 and not paused:
        for r in range(9):
            for c in range(9):
                if board[r][c] == sel_num and not (r == sel_r and c == sel_c):
                    cmds.append(CanvasRect(ox + c * cell + 2, oy + r * cell + 2, cell - 4, cell - 4, theme.surface, radius=3.0))

    # Selected cell — highlight fill + muted border, clearly distinct from box tint
    if sel_r >= 0 and sel_c >= 0 and not paused and screen == "game":
        cmds.extend(_bordered_rect(
            ox + sel_c * cell + 1, oy + sel_r * cell + 1, cell - 2, cell - 2,
            theme.highlight, radius=3.0,
            border_color=theme.muted, border_width=2.0,
        ))

    # Cell content
    if not paused:
        for r in range(9):
            for c in range(9):
                cx = ox + c * cell + cell / 2
                cy = oy + r * cell + cell / 2
                val = board[r][c]
                if val != 0:
                    if given[r][c]:
                        col, bold = theme.fg, True
                    elif errors[r][c]:
                        col, bold = theme.danger, False
                    elif complete:
                        col, bold = theme.success, False
                    else:
                        col, bold = theme.accent, False
                    cmds.append(CanvasText(cx, cy, str(val), size=cell * 0.52, color=col, bold=bold, align="center_center"))
                else:
                    cell_notes = notes[r][c]
                    mini = cell / 3.2
                    for ni in range(9):
                        if cell_notes[ni]:
                            nr2, nc2 = divmod(ni, 3)
                            nx = ox + c * cell + nc2 * (cell / 3) + cell / 6
                            ny = oy + r * cell + nr2 * (cell / 3) + cell / 6
                            cmds.append(CanvasText(nx, ny, str(ni + 1), size=max(7.0, mini * 0.6), color=theme.muted, align="center_center"))

    # Grid lines — box boundaries use fg (2px), cell lines use muted (0.5px) so
    # they remain visible against the box and selected-cell fills.
    for i in range(10):
        box_line = i % 3 == 0
        lw = 2.0 if box_line else 0.5
        col = theme.fg if box_line else theme.muted
        cmds.append(CanvasRect(ox, oy + i * cell - lw / 2, gw, lw, col))
        cmds.append(CanvasRect(ox + i * cell - lw / 2, oy, lw, gh, col))


    # Overlay: paused
    cxg = ox + gw / 2
    cyg = oy + gh / 2
    scrim = dim(theme.bg_darkest, 238)
    if paused:
        cmds += [
            CanvasRect(cxg - 90, cyg - 32, 180, 64, scrim, radius=8.0),
            CanvasText(cxg, cyg, "PAUSED", size=22.0, color=theme.warning, bold=True, align="center_center"),
        ]

    # Overlay: win
    if screen == "win":
        cmds += [
            CanvasRect(cxg - 120, cyg - 50, 240, 100, scrim, radius=10.0),
            CanvasText(cxg, cyg - 18, "PUZZLE SOLVED!", size=20.0, color=theme.success, bold=True, align="center_center"),
            CanvasText(cxg, cyg + 12, _fmt(seconds), size=16.0, color=theme.fg, align="center_center"),
            CanvasText(cxg, cyg + 34, "press R for new game", size=11.0, color=theme.muted, align="center_center"),
        ]

    return cmds


