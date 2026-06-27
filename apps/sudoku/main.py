#!/usr/bin/env python3
"""Sudoku — Easy, Medium, and Hard difficulties."""

from __future__ import annotations

import random

import plexi_sdk as sdk
from plexi_sdk import dim, rgba, state, theme

TRANSPARENT = rgba(0, 0, 0, 0)
from plexi_sdk.effects import SetState, SetStatus, SetTimer, SetTitle
from plexi_sdk.events import KeyEvent, MouseEvent, Resize, TimerFired
from plexi_sdk.ui import (
    AppBar, Canvas, CanvasRect, CanvasText, Column, Divider,
    FooterKeys, HStack, Section, Sized, Text,
)

TIMER_ID = 1
SIDEBAR_W = 148
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

def init(_size, _args):
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
        return []

    if isinstance(event, MouseEvent) and event.pressed:
        return _mouse(d, event.region)

    if isinstance(event, KeyEvent) and event.pressed:
        return _key(d, event)

    return []

# ── Input handlers ─────────────────────────────────────────────────────────

def _mouse(d, region):
    if not region:
        return []
    if str(d.get("screen")) != "game":
        if region.startswith("diff-"):
            diff = DIFFICULTIES[int(region[5:])]
            return [SetState(_new_game(diff)), SetStatus(f"{diff.title()} — 00:00")]
        return []
    if region.startswith("num-"):
        return _enter_number(d, int(region[4:]))
    if region.startswith("cell-"):
        r, c = (int(v) for v in region[5:].split("-"))
        board = d.get("board", [[0] * 9] * 9)
        return [SetState({"sel_r": r, "sel_c": c, "sel_num": board[r][c]})]
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
    if screen != "menu":
        body = HStack([
            Canvas(_draw_grid(d), grow=True),
            Sized(width=SIDEBAR_W, child=_sidebar(d)),
        ], grow=True)
    else:
        body = Canvas(_draw_menu(d), grow=True)

    return Column([
        AppBar("Sudoku"),
        body,
        footer,
    ], padding=0, gap=0, grow=True)


# ── Menu drawing ───────────────────────────────────────────────────────────

def _draw_menu(d):
    w = sdk.canvas_width
    h = sdk.canvas_height
    diff_idx = int(d.get("diff_idx", 0))
    cmds = []

    title_y = h / 2 - 160.0
    cmds.append(CanvasText(w / 2, title_y, "SUDOKU", size=36.0, color=theme.fg, bold=True, align="center_center"))
    cmds.append(CanvasText(w / 2, title_y + 40.0, "Choose your difficulty", size=13.0, color=theme.muted, align="center_center"))

    bw, bh = 220.0, 64.0
    bx = (w - bw) / 2
    by_start = h / 2 - 80.0
    labels = [("Easy", f"{CLUES['easy']} clues"), ("Medium", f"{CLUES['medium']} clues"), ("Hard", f"{CLUES['hard']} clues")]
    tones = [theme.success, theme.warning, theme.danger]

    for i, (label, subtitle) in enumerate(labels):
        by = by_start + i * 80.0
        selected = i == diff_idx
        bg = theme.surface if selected else theme.bg
        border = tones[i] if selected else theme.highlight
        cmds.append(CanvasRect(bx, by, bw, bh, bg, radius=8.0,
                               border_color=border, border_width=2.0,
                               hit_region=f"diff-{i}"))
        text_color = tones[i] if selected else theme.fg
        cmds.append(CanvasText(bx + bw / 2, by + 22.0, label, size=17.0, color=text_color, bold=selected, align="center_center"))
        cmds.append(CanvasText(bx + bw / 2, by + 44.0, subtitle, size=11.0, color=theme.muted, align="center_center"))

    return cmds

# ── Game drawing ───────────────────────────────────────────────────────────

def _draw_grid(d):
    """Canvas commands for the sudoku grid only (no sidebar)."""
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

    # sdk.canvas_width/height reflect full pane size, not HStack allocation.
    # Use canvas_width minus sidebar for horizontal sizing, and a fixed
    # top margin for vertical so the grid never overflows its allocation.
    grid_w = sdk.canvas_width - SIDEBAR_W - 8
    TOP_PAD = 24.0
    cell = min((grid_w - 24) / 9, (sdk.canvas_height * 0.75 - TOP_PAD) / 9)
    ox = (grid_w - cell * 9) / 2
    oy = TOP_PAD
    gw = cell * 9
    gh = cell * 9
    cmds = []

    # Grid shadow/bg
    cmds.append(CanvasRect(ox - 3, oy - 3, gw + 6, gh + 6, theme.bg_darkest, radius=6.0))
    cmds.append(CanvasRect(ox, oy, gw, gh, theme.bg, radius=4.0))

    # Per-cell hit regions
    for r in range(9):
        for c in range(9):
            cmds.append(CanvasRect(ox + c * cell, oy + r * cell, cell, cell,
                                   TRANSPARENT, hit_region=f"cell-{r}-{c}"))

    # Highlight row, col, box of selected cell
    if sel_r >= 0 and sel_c >= 0 and not paused and screen == "game":
        cmds.append(CanvasRect(ox, oy + sel_r * cell, gw, cell, theme.border))
        cmds.append(CanvasRect(ox + sel_c * cell, oy, cell, gh, theme.border))
        br, bc = (sel_r // 3) * 3, (sel_c // 3) * 3
        cmds.append(CanvasRect(ox + bc * cell, oy + br * cell, cell * 3, cell * 3, theme.border))

    # Same-number highlight
    if sel_num != 0 and not paused:
        for r in range(9):
            for c in range(9):
                if board[r][c] == sel_num and not (r == sel_r and c == sel_c):
                    cmds.append(CanvasRect(ox + c * cell + 2, oy + r * cell + 2, cell - 4, cell - 4, theme.surface, radius=3.0))

    # Selected cell highlight
    if sel_r >= 0 and sel_c >= 0 and not paused and screen == "game":
        cmds.append(CanvasRect(ox + sel_c * cell + 1, oy + sel_r * cell + 1, cell - 2, cell - 2, theme.highlight, radius=3.0))

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

    # Grid lines
    for i in range(10):
        box_line = i % 3 == 0
        lw = 2.0 if box_line else 0.5
        col = theme.fg if box_line else theme.highlight
        cmds.append(CanvasRect(ox, oy + i * cell - lw / 2, gw, lw, col))
        cmds.append(CanvasRect(ox + i * cell - lw / 2, oy, lw, gh, col))

    # Box outline on top
    if sel_r >= 0 and sel_c >= 0 and not paused and screen == "game":
        br, bc = (sel_r // 3) * 3, (sel_c // 3) * 3
        bs = cell * 3
        cmds.append(CanvasRect(ox + bc * cell, oy + br * cell, bs, bs, TRANSPARENT,
                               border_color=theme.muted, border_width=2.0))

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


def _numpad_canvas(d):
    """3x3 numpad as a small canvas with hit regions."""
    board = d.get("board", [[0] * 9] * 9)
    sel_num = int(d.get("sel_num", 0))
    counts = _count_numbers(board)
    bw, bh, gap = 38.0, 38.0, 6.0
    cmds = []
    for i in range(9):
        num = i + 1
        col = i % 3
        row = i // 3
        bx = col * (bw + gap)
        by = row * (bh + gap)
        done_num = counts[num - 1] >= 9
        is_sel = num == sel_num and sel_num != 0
        if done_num:
            bg, text_col, border = theme.bg, theme.muted, theme.border
        elif is_sel:
            bg, text_col, border = theme.highlight, theme.accent, theme.accent
        else:
            bg, text_col, border = theme.surface, theme.fg, theme.highlight
        cmds.append(CanvasRect(bx, by, bw, bh, bg, radius=4.0,
                               border_color=border, border_width=1.0,
                               hit_region=f"num-{num}"))
        cmds.append(CanvasText(bx + bw / 2, by + bh / 2, str(num), size=15.0,
                               color=text_col, bold=not done_num, align="center_center"))
    total_w = 3 * bw + 2 * gap
    total_h = 3 * bh + 2 * gap
    return Canvas(cmds, width=total_w, height=total_h)


def _sidebar(d):
    """L1 Column sidebar: difficulty, timer, progress, numpad."""
    difficulty = str(d.get("difficulty", "easy"))
    seconds = int(d.get("seconds", 0))
    notes_mode = bool(d.get("notes_mode", False))
    board = d.get("board", [[0] * 9] * 9)
    given = d.get("given", [[True] * 9] * 9)

    filled = sum(1 for r in range(9) for c in range(9) if not given[r][c] and board[r][c] != 0)
    blanks = sum(1 for r in range(9) for c in range(9) if not given[r][c])
    d_color = getattr(theme, DIFF_TONE.get(difficulty, "accent"))

    children = [
        Section("DIFFICULTY"),
        Text(difficulty.upper(), color=d_color, bold=True, size=15),
        Divider(),
        Section("TIME"),
        Text(_fmt(seconds), bold=True, size=20),
    ]
    if notes_mode:
        children += [Divider(), Text("NOTES ON", color=theme.warning, bold=True)]
    if blanks > 0:
        children += [
            Divider(),
            Section("PROGRESS"),
            Text(f"{filled}/{blanks}", color=theme.accent, size=13),
        ]
    children += [
        Divider(),
        Section("NUMBERS"),
        _numpad_canvas(d),
    ]
    return Column(children, padding=12, gap=6)
