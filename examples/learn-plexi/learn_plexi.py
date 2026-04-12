#!/usr/bin/env python3
from __future__ import annotations
"""
learn-plexi — Plexi app
Gamified interactive tutorial that teaches new users the core Plexi keybindings.

Controls:
  Enter        Advance on welcome screen
  Any key      Acknowledge and advance on levels 3, 4, 5
  r            Restart from the done screen
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Catppuccin Mocha palette
# ---------------------------------------------------------------------------
C = {
    "bg":      "#1e1e2e",
    "surface": "#313244",
    "text":    "#cdd6f4",
    "subtext": "#9399b2",
    "muted":   "#6c7086",
    "accent":  "#89b4fa",
    "green":   "#a6e3a1",
    "yellow":  "#f9e2af",
    "peach":   "#fab387",
    "header":  "#181825",
}

# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------
TOTAL_LEVELS = 6

current_level = 0   # 0 = welcome, 1 = hjkl, 2 = new context, 3 = split, 4 = close, 5 = done

# Level 2 (hjkl) pane-size tracking
_last_size: tuple[float, float] = (0.0, 0.0)
_left_pane: bool = False    # True once size has shrunk (focus left)
_returned: bool = False     # True once focus has returned (size grew back)

# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------
app = App(app_id="learn-plexi")


def reset_state():
    global current_level, _last_size, _left_pane, _returned
    current_level = 0
    _last_size = (0.0, 0.0)
    _left_pane = False
    _returned = False


# ---------------------------------------------------------------------------
# Drawing helpers
# ---------------------------------------------------------------------------

def draw_background(ctx):
    ctx.rect(0, 0, ctx.width, ctx.height, fill=C["bg"])


def draw_progress_bar(ctx, completed: int, total: int):
    """Render a segmented progress bar near the bottom of the pane."""
    bar_y = ctx.height - 44
    bar_h = 6
    margin = 20
    gap = 6
    total_gap = gap * (total - 1)
    seg_w = (ctx.width - margin * 2 - total_gap) / total

    for i in range(total):
        x = margin + i * (seg_w + gap)
        fill = C["accent"] if i < completed else C["surface"]
        ctx.rect(x, bar_y, seg_w, bar_h, fill=fill, radius=3.0)


def draw_footer(ctx, level_display: int, total: int):
    label = f"learn-plexi  •  level {level_display}/{total}"
    ctx.text(ctx.width / 2 - 90, ctx.height - 26, label,
             size=11, color=C["muted"], monospace=True)


def draw_level_header(ctx, level_num: int, title: str):
    """Level number pill + title."""
    ctx.rect(20, 16, ctx.width - 40, 44, fill=C["header"], radius=8.0)
    level_str = f"Level {level_num}"
    ctx.text(28, 26, level_str, size=11, color=C["accent"], bold=True)
    ctx.text(28 + 72, 24, title, size=15, color=C["text"], bold=True)


def draw_instruction(ctx, lines: list[str], y_start: float = 120):
    """Draw multi-line instruction text centered-ish."""
    line_h = 26
    for i, line in enumerate(lines):
        ctx.text(28, y_start + i * line_h, line, size=14, color=C["text"])


def draw_hint(ctx, hint: str, y: float | None = None):
    if y is None:
        y = ctx.height - 76
    ctx.text(28, y, hint, size=12, color=C["subtext"])


# ---------------------------------------------------------------------------
# Level renderers
# ---------------------------------------------------------------------------

def render_welcome(ctx):
    draw_background(ctx)

    # Big logo area
    logo_y = ctx.height * 0.25
    ctx.text(ctx.width / 2 - 52, logo_y, "PLEXI", size=48, color=C["accent"], bold=True)
    ctx.text(ctx.width / 2 - 72, logo_y + 62, "terminal multiplexer", size=14, color=C["subtext"])

    # Prompt
    ctx.text(ctx.width / 2 - 88, logo_y + 110, "Press  Enter  to begin", size=16, color=C["text"])

    draw_progress_bar(ctx, 0, TOTAL_LEVELS)
    draw_footer(ctx, 0, TOTAL_LEVELS)


def render_hjkl(ctx):
    global _last_size, _left_pane, _returned

    draw_background(ctx)
    draw_level_header(ctx, 1, "Navigate panes")

    w, h = ctx.width, ctx.height
    size_now = (w, h)

    # Track pane size changes to detect focus leaving/returning
    if _last_size != (0.0, 0.0):
        prev_area = _last_size[0] * _last_size[1]
        curr_area = size_now[0] * size_now[1]
        if not _left_pane and curr_area < prev_area * 0.9:
            _left_pane = True
        elif _left_pane and not _returned and curr_area > prev_area * 1.1:
            _returned = True
    _last_size = size_now

    lines = [
        "Use  h j k l  to move focus between panes.",
        "",
        "Move focus away from this pane and back.",
        "Complete this action to unlock the next level.",
    ]
    draw_instruction(ctx, lines)

    # Keyboard-style key boxes
    keys = [("h", "← left"), ("j", "↓ down"), ("k", "↑ up"), ("l", "→ right")]
    box_w, box_h = 52, 44
    gap = 12
    total_w = len(keys) * box_w + (len(keys) - 1) * gap
    start_x = (w - total_w) / 2
    key_y = h * 0.52

    for i, (key_char, label) in enumerate(keys):
        kx = start_x + i * (box_w + gap)
        ctx.rect(kx, key_y, box_w, box_h, fill=C["surface"], radius=6.0)
        ctx.text(kx + box_w / 2 - 6, key_y + 10, key_char,
                 size=18, color=C["accent"], bold=True, monospace=True)
        ctx.text(kx + box_w / 2 - 24, key_y + box_h + 6, label,
                 size=10, color=C["muted"])

    # Status indicator
    if _returned:
        status = "Great job! Moving to the next level..."
        status_color = C["green"]
    elif _left_pane:
        status = "Good — now come back to this pane!"
        status_color = C["yellow"]
    else:
        status = "Start by moving focus away with h, j, k, or l"
        status_color = C["subtext"]

    ctx.text(28, h * 0.80, status, size=13, color=status_color)

    draw_progress_bar(ctx, 1, TOTAL_LEVELS)
    draw_footer(ctx, 1, TOTAL_LEVELS)


def render_new_context(ctx):
    draw_background(ctx)
    draw_level_header(ctx, 2, "New context")

    lines = [
        "Press  Cmd+T  to open a new context.",
        "(Like a new tab in your browser.)",
        "",
        "Come back here when you're done,",
        "then press any key to continue.",
    ]
    draw_instruction(ctx, lines)

    # Visual shortcut display
    btn_y = ctx.height * 0.62
    ctx.rect(ctx.width / 2 - 60, btn_y, 120, 38, fill=C["surface"], radius=8.0)
    ctx.text(ctx.width / 2 - 36, btn_y + 11, "Cmd + T",
             size=14, color=C["accent"], bold=True, monospace=True)

    draw_hint(ctx, "Press any key to continue after opening a context")
    draw_progress_bar(ctx, 2, TOTAL_LEVELS)
    draw_footer(ctx, 2, TOTAL_LEVELS)


def render_split(ctx):
    draw_background(ctx)
    draw_level_header(ctx, 3, "Split a pane")

    lines = [
        "Press  Cmd+D  to split this pane vertically.",
        "",
        "Splitting lets you view multiple things side-by-side",
        "in the same context.",
    ]
    draw_instruction(ctx, lines)

    # Split illustration
    ill_y = ctx.height * 0.58
    ill_w = 180
    ill_h = 70
    ill_x = ctx.width / 2 - ill_w / 2
    ctx.rect(ill_x, ill_y, ill_w, ill_h, fill=C["surface"], radius=6.0)
    mid = ill_x + ill_w / 2
    ctx.line(mid, ill_y + 8, mid, ill_y + ill_h - 8,
             color=C["accent"], width=2.0)
    ctx.text(ill_x + 16, ill_y + ill_h / 2 - 8, "pane", size=11, color=C["muted"])
    ctx.text(mid + 16, ill_y + ill_h / 2 - 8, "pane", size=11, color=C["muted"])

    draw_hint(ctx, "Press any key to continue after splitting")
    draw_progress_bar(ctx, 3, TOTAL_LEVELS)
    draw_footer(ctx, 3, TOTAL_LEVELS)


def render_close(ctx):
    draw_background(ctx)
    draw_level_header(ctx, 4, "Close a pane")

    lines = [
        "Press  Cmd+W  to close a pane.",
        "",
        "Use this to tidy up when you're done",
        "with a split or context.",
    ]
    draw_instruction(ctx, lines)

    btn_y = ctx.height * 0.62
    ctx.rect(ctx.width / 2 - 66, btn_y, 132, 38, fill=C["surface"], radius=8.0)
    ctx.text(ctx.width / 2 - 40, btn_y + 11, "Cmd + W",
             size=14, color=C["peach"], bold=True, monospace=True)

    draw_hint(ctx, "Press any key to continue after closing a pane")
    draw_progress_bar(ctx, 4, TOTAL_LEVELS)
    draw_footer(ctx, 4, TOTAL_LEVELS)


def render_done(ctx):
    draw_background(ctx)

    w, h = ctx.width, ctx.height

    # Celebration header
    ctx.rect(20, 16, w - 40, 52, fill=C["header"], radius=8.0)
    ctx.text(w / 2 - 100, 28, "You know Plexi!", size=20, color=C["green"], bold=True)

    # Reference card
    card_y = 90
    card_x = 28
    col2_x = w / 2 + 8
    card_w = w / 2 - 36

    ctx.text(card_x, card_y, "Keybinding reference", size=13, color=C["accent"], bold=True)

    shortcuts = [
        ("h / j / k / l", "Move focus left / down / up / right"),
        ("Cmd+T",          "Open new context"),
        ("Cmd+D",          "Split pane vertically"),
        ("Cmd+W",          "Close pane"),
    ]

    row_h = 28
    for i, (key, desc) in enumerate(shortcuts):
        ry = card_y + 24 + i * row_h
        ctx.rect(card_x, ry, card_w, row_h - 4, fill=C["surface"], radius=4.0)
        ctx.text(card_x + 10, ry + 8, key,
                 size=12, color=C["yellow"], bold=True, monospace=True)
        ctx.text(col2_x, ry + 8, desc, size=12, color=C["text"])

    # Restart hint
    restart_y = card_y + 24 + len(shortcuts) * row_h + 16
    ctx.text(card_x, restart_y, "Press  r  to restart the tutorial",
             size=13, color=C["subtext"])

    draw_progress_bar(ctx, TOTAL_LEVELS, TOTAL_LEVELS)
    draw_footer(ctx, TOTAL_LEVELS, TOTAL_LEVELS)


# ---------------------------------------------------------------------------
# Event handlers
# ---------------------------------------------------------------------------

LEVEL_RENDERERS = [
    render_welcome,
    render_hjkl,
    render_new_context,
    render_split,
    render_close,
    render_done,
]


@app.on_render
def render(ctx):
    renderer = LEVEL_RENDERERS[current_level]
    renderer(ctx)


@app.on_key
def on_key(key: str, mods: dict, emit):
    global current_level, _left_pane, _returned, _last_size

    # Level 0 — welcome: only Enter advances
    if current_level == 0:
        if key == "Enter":
            current_level = 1
        return

    # Level 1 — hjkl: advance only after leaving and returning
    if current_level == 1:
        # Any key press after _returned triggers advance
        if _returned:
            current_level = 2
            # Reset hjkl tracking for potential replay
            _left_pane = False
            _returned = False
            _last_size = (0.0, 0.0)
        return

    # Levels 2, 3, 4 — acknowledgment: any key advances
    if current_level in (2, 3, 4):
        current_level += 1
        return

    # Level 5 — done: r restarts
    if current_level == 5:
        if key == "r":
            reset_state()
        return


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

app.run()
