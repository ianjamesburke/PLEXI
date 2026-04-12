#!/usr/bin/env python3
from __future__ import annotations
"""
learn-plexi — self-contained simulated UI tutorial.

Draws a miniature mock of the Plexi interface on-canvas and animates each
lesson inside it. No real pane events needed — everything is driven by
_frame and user keystrokes.

Controls per stage:
  Stage 0  — Enter to begin
  Stage 1  — Enter to continue
  Stage 2  — h/j/k/l to shift demo focus, then Enter to continue
  Stage 3  — D to trigger split animation, then Enter to continue
  Stage 4  — T to trigger tab animation, then Enter to continue
  Stage 5  — W to trigger close animation, then Enter to continue
  Stage 6  — R to restart, Q/Escape to quit
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Catppuccin Mocha palette
# ---------------------------------------------------------------------------
C = {
    "bg":       "#1e1e2e",
    "mantle":   "#181825",
    "surface0": "#313244",
    "surface1": "#45475a",
    "overlay":  "#6c7086",
    "subtext":  "#9399b2",
    "text":     "#cdd6f4",
    "accent":   "#89b4fa",
    "green":    "#a6e3a1",
    "yellow":   "#f9e2af",
    "peach":    "#fab387",
    "red":      "#f38ba8",
    "header":   "#181825",
}

ANIM_FRAMES = 20   # frames for a single animation transition

# ---------------------------------------------------------------------------
# Global state
# ---------------------------------------------------------------------------
_stage: int = 0
_frame: int = 0
_anim_start: int = 0

_hjkl_pressed: bool = False
_demo_focus: int = 1        # 0=left, 1=center, 2=right  (stage 2)
_split_done: bool = False   # stage 3 — split anim triggered
_tab_done: bool = False     # stage 4 — tab anim triggered
_close_done: bool = False   # stage 5 — close anim triggered

# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------
app = App(app_id="learn-plexi")


def reset_state():
    global _stage, _frame, _anim_start
    global _hjkl_pressed, _demo_focus, _split_done, _tab_done, _close_done
    _stage = 0
    _frame = 0
    _anim_start = 0
    _hjkl_pressed = False
    _demo_focus = 1
    _split_done = False
    _tab_done = False
    _close_done = False


def anim_t() -> float:
    """0→1 progress for current animation, clamped."""
    raw = (_frame - _anim_start) / ANIM_FRAMES
    if raw < 0.0:
        return 0.0
    if raw > 1.0:
        return 1.0
    return raw


# ---------------------------------------------------------------------------
# Drawing helpers
# ---------------------------------------------------------------------------

def bg(ctx):
    ctx.rect(0, 0, ctx.width, ctx.height, fill=C["bg"])


def footer_hint(ctx, msg: str):
    """Dim hint line at bottom."""
    ctx.text(ctx.width / 2 - len(msg) * 3.5, ctx.height - 22,
             msg, size=12, color=C["overlay"])


def tip_bubble(ctx, x: float, y: float, w: float, lines: list):
    """Rounded tip box."""
    lh = 20
    h = lh * len(lines) + 16
    ctx.rect(x, y, w, h, fill=C["mantle"], radius=6.0)
    for i, line in enumerate(lines):
        ctx.text(x + 10, y + 8 + i * lh, line, size=12, color=C["text"])


def key_box(ctx, x: float, y: float, label: str, highlighted: bool = False):
    """24x24 key cap."""
    fill = C["accent"] if highlighted else C["surface0"]
    lcolor = C["mantle"] if highlighted else C["text"]
    ctx.rect(x, y, 24, 24, fill=fill, radius=4.0)
    ctx.text(x + 7, y + 5, label, size=12, color=lcolor, bold=True, monospace=True)


def fake_terminal_lines(ctx, x: float, y: float, w: float, n: int = 4):
    """Draw muted bars to suggest terminal text content."""
    lh = 11
    lens = [int(w * 0.6), int(w * 0.45), int(w * 0.7), int(w * 0.35)]
    for i in range(min(n, len(lens))):
        bar_w = lens[i]
        ctx.rect(x + 6, y + 6 + i * lh, bar_w, 5, fill=C["surface1"], radius=2.0)


def mock_pane(ctx, x: float, y: float, w: float, h: float,
              focused: bool = False, title: str = ""):
    """Draw a fake terminal pane rect."""
    if w < 2 or h < 2:
        return
    border_color = C["accent"] if focused else C["overlay"]
    border_w = 2.0 if focused else 1.0
    fill = C["surface0"] if focused else C["mantle"]

    ctx.rect(x, y, w, h, fill=fill, radius=4.0)
    # border via four rects
    ctx.rect(x, y, w, border_w, fill=border_color)
    ctx.rect(x, y + h - border_w, w, border_w, fill=border_color)
    ctx.rect(x, y, border_w, h, fill=border_color)
    ctx.rect(x + w - border_w, y, border_w, h, fill=border_color)

    # title bar
    if title:
        ctx.rect(x + border_w, y + border_w, w - border_w * 2, 16, fill=C["surface1"])
        ctx.text(x + 6, y + 3, title, size=9, color=C["subtext"])

    # fake content lines
    content_y = y + (20 if title else 8)
    if h > 40:
        fake_terminal_lines(ctx, x, content_y, w - 12, n=3)

    # blinking cursor on focused pane
    if focused and (_frame // 30) % 2 == 0 and h > 55:
        ctx.rect(x + 8, content_y + 42, 7, 11, fill=C["accent"], radius=1.0)


def mock_toolbar(ctx, x: float, y: float, w: float, tabs: list, active: int = 0):
    """Draw a fake context tab bar."""
    ctx.rect(x, y, w, 24, fill=C["mantle"], radius=0.0)
    tx = x + 8
    for i, tab in enumerate(tabs):
        tab_w = len(tab) * 7 + 16
        fill = C["surface0"] if i == active else C["mantle"]
        tc = C["text"] if i == active else C["subtext"]
        ctx.rect(tx, y + 2, tab_w, 20, fill=fill, radius=4.0)
        ctx.text(tx + 8, y + 6, tab, size=10, color=tc)
        tx += tab_w + 4


# ---------------------------------------------------------------------------
# Stage renderers
# ---------------------------------------------------------------------------

def render_stage0(ctx):
    """Welcome screen."""
    bg(ctx)
    w, h = ctx.width, ctx.height

    # PLEXI spelled out in blocks
    letters = "PLEXI"
    block_w, block_h = 48, 56
    gap = 12
    total_letter_w = len(letters) * block_w + (len(letters) - 1) * gap
    start_x = (w - total_letter_w) / 2
    logo_y = h * 0.22

    for i, ch in enumerate(letters):
        bx = start_x + i * (block_w + gap)
        ctx.rect(bx, logo_y, block_w, block_h, fill=C["accent"], radius=6.0)
        ctx.text(bx + 12, logo_y + 10, ch, size=32, color=C["mantle"], bold=True)

    ctx.text(w / 2 - 130, logo_y + block_h + 18,
             "A terminal multiplexer. Let's learn the basics.",
             size=14, color=C["subtext"])

    ctx.text(w / 2 - 80, logo_y + block_h + 52,
             "Press Enter to begin \u2192",
             size=13, color=C["text"])


def render_stage1(ctx):
    """The Pane — single large fake pane with label and tip."""
    bg(ctx)
    w, h = ctx.width, ctx.height

    pane_x = w * 0.12
    pane_y = h * 0.14
    pane_w = w * 0.76
    pane_h = h * 0.50

    mock_pane(ctx, pane_x, pane_y, pane_w, pane_h, focused=True, title="bash")

    # Arrow + label below pane
    label_y = pane_y + pane_h + 10
    ctx.line(w / 2, pane_y + pane_h, w / 2, label_y, color=C["accent"], width=1.5)
    ctx.text(w / 2 - 52, label_y + 4, "This is a pane",
             size=13, color=C["accent"], bold=True)

    tip_bubble(ctx, pane_x, pane_y + pane_h + 34, pane_w,
               ["Plexi splits your screen into panes.",
                "Each pane is an independent terminal."])

    footer_hint(ctx, "Press Enter to continue")


def render_stage2(ctx):
    """Focus — 3 panes, hjkl moves the highlight."""
    bg(ctx)
    w, h = ctx.width, ctx.height

    pane_gap = 8
    pane_y = h * 0.14
    pane_h = h * 0.44
    total_w = w * 0.82
    pane_w = (total_w - 2 * pane_gap) / 3
    start_x = (w - total_w) / 2

    for i in range(3):
        px = start_x + i * (pane_w + pane_gap)
        mock_pane(ctx, px, pane_y, pane_w, pane_h, focused=(i == _demo_focus))

    # hjkl key row
    keys = [("h", "\u2190"), ("j", "\u2193"), ("k", "\u2191"), ("l", "\u2192")]
    key_row_w = len(keys) * 24 + (len(keys) - 1) * 8
    key_x = (w - key_row_w) / 2
    key_y = pane_y + pane_h + 16

    for i, (k, arrow) in enumerate(keys):
        kx = key_x + i * 32
        key_box(ctx, kx, key_y, k, highlighted=_hjkl_pressed)
        ctx.text(kx + 2, key_y + 28, arrow, size=9, color=C["subtext"])

    tip_bubble(ctx, start_x, key_y + 46, total_w,
               ["Focus moves between panes.",
                "Try pressing h, j, k, l."])

    if _hjkl_pressed:
        ctx.text(w / 2 - 100, key_y + 96,
                 "Good! Press Enter to continue \u2192", size=13, color=C["green"])
    else:
        footer_hint(ctx, "Press h/j/k/l to move focus \u2014 then Enter")


def render_stage3(ctx):
    """Split — animate one pane splitting into two."""
    bg(ctx)
    w, h = ctx.width, ctx.height

    pane_y = h * 0.16
    pane_h = h * 0.48
    pane_x = w * 0.12
    full_w = w * 0.76

    t = anim_t() if _split_done else 0.0
    # ease-out
    t_e = 1.0 - (1.0 - t) * (1.0 - t)

    gap = 8
    split_w = (full_w - gap) / 2

    if t_e < 0.01:
        mock_pane(ctx, pane_x, pane_y, full_w, pane_h, focused=True, title="bash")
    else:
        left_w = full_w * (1 - t_e) + split_w * t_e
        right_x = pane_x + left_w + gap
        right_w = full_w - left_w - gap
        mock_pane(ctx, pane_x, pane_y, left_w, pane_h, focused=True, title="bash")
        if right_w > 4:
            mock_pane(ctx, right_x, pane_y, right_w, pane_h,
                      focused=False, title="bash")

    tip_bubble(ctx, pane_x, pane_y + pane_h + 12, full_w,
               ["Cmd+D splits the focused pane vertically."])

    if _split_done and t >= 1.0:
        ctx.text(w / 2 - 110, pane_y + pane_h + 56,
                 "Split complete! Press Enter to continue \u2192",
                 size=13, color=C["green"])
    elif _split_done:
        footer_hint(ctx, "Animating split\u2026")
    else:
        footer_hint(ctx, "Press D to see a split \u2014 then Enter")


def render_stage4(ctx):
    """New Context — toolbar with animated second tab appearing."""
    bg(ctx)
    w, h = ctx.width, ctx.height

    pane_y = h * 0.24
    pane_h = h * 0.46
    pane_x = w * 0.12
    pane_w = w * 0.76

    tabs = ["Context 1"]
    active_tab = 0
    if _tab_done:
        tabs = ["Context 1", "Context 2"]
        if anim_t() >= 1.0:
            active_tab = 1

    mock_toolbar(ctx, pane_x, pane_y - 24, pane_w, tabs, active=active_tab)
    mock_pane(ctx, pane_x, pane_y, pane_w, pane_h, focused=True, title="bash")

    tip_bubble(ctx, pane_x, pane_y + pane_h + 12, pane_w,
               ["Cmd+T opens a new context \u2014 like a workspace tab."])

    if _tab_done and anim_t() >= 1.0:
        ctx.text(w / 2 - 110, pane_y + pane_h + 56,
                 "New context added! Press Enter to continue \u2192",
                 size=13, color=C["green"])
    elif _tab_done:
        footer_hint(ctx, "Animating\u2026")
    else:
        footer_hint(ctx, "Press T to add a context \u2014 then Enter")


def render_stage5(ctx):
    """Close — animate right pane shrinking to nothing."""
    bg(ctx)
    w, h = ctx.width, ctx.height

    pane_y = h * 0.16
    pane_h = h * 0.48
    pane_x = w * 0.12
    full_w = w * 0.76
    gap = 8
    base_split_w = (full_w - gap) / 2

    t = anim_t() if _close_done else 0.0
    t_e = 1.0 - (1.0 - t) * (1.0 - t)

    right_w = base_split_w * (1.0 - t_e)
    left_w = full_w - gap - right_w if right_w > 1 else full_w

    mock_pane(ctx, pane_x, pane_y, left_w, pane_h, focused=True, title="bash")
    if right_w > 2:
        mock_pane(ctx, pane_x + left_w + gap, pane_y, right_w, pane_h,
                  focused=False, title="bash")

    tip_bubble(ctx, pane_x, pane_y + pane_h + 12, full_w,
               ["Cmd+W closes the focused pane."])

    if _close_done and t >= 1.0:
        ctx.text(w / 2 - 100, pane_y + pane_h + 56,
                 "Pane closed! Press Enter to continue \u2192",
                 size=13, color=C["green"])
    elif _close_done:
        footer_hint(ctx, "Animating\u2026")
    else:
        footer_hint(ctx, "Press W to close a pane \u2014 then Enter")


def render_stage6(ctx):
    """Reference card — full-screen keybinding summary."""
    bg(ctx)
    w, h = ctx.width, ctx.height

    ctx.rect(20, 16, w - 40, 46, fill=C["mantle"], radius=8.0)
    ctx.text(w / 2 - 80, 28, "You know Plexi!", size=20, color=C["green"], bold=True)

    learned = [
        ("h / j / k / l", "Move focus left / down / up / right"),
        ("Cmd+D",          "Split pane vertically"),
        ("Cmd+T",          "Open new context"),
        ("Cmd+W",          "Close pane"),
    ]
    more = [
        ("Cmd+P",   "Open app launcher"),
        ("/",       "Search / filter"),
        ("Escape",  "Exit current mode"),
        ("Cmd+K",   "Clear terminal"),
    ]

    col_w = (w - 60) / 2
    row_h = 30
    card_y = 80

    ctx.text(28, card_y, "What you learned", size=12, color=C["accent"], bold=True)
    for i, (key, desc) in enumerate(learned):
        ry = card_y + 22 + i * row_h
        ctx.rect(28, ry, col_w - 8, row_h - 4, fill=C["surface0"], radius=4.0)
        ctx.text(36, ry + 8, key, size=11, color=C["yellow"], bold=True, monospace=True)
        ctx.text(36 + 100, ry + 8, desc, size=11, color=C["text"])

    rx = 28 + col_w + 4
    ctx.text(rx, card_y, "More useful keys", size=12, color=C["accent"], bold=True)
    for i, (key, desc) in enumerate(more):
        ry = card_y + 22 + i * row_h
        ctx.rect(rx, ry, col_w - 8, row_h - 4, fill=C["surface0"], radius=4.0)
        ctx.text(rx + 8, ry + 8, key, size=11, color=C["yellow"], bold=True, monospace=True)
        ctx.text(rx + 8 + 80, ry + 8, desc, size=11, color=C["text"])

    restart_y = card_y + 22 + max(len(learned), len(more)) * row_h + 12
    ctx.text(28, restart_y,
             "Press R to restart  \u2022  Q or Escape to dismiss",
             size=12, color=C["subtext"])


STAGE_RENDERERS = [
    render_stage0,
    render_stage1,
    render_stage2,
    render_stage3,
    render_stage4,
    render_stage5,
    render_stage6,
]

# ---------------------------------------------------------------------------
# Event handlers
# ---------------------------------------------------------------------------

@app.on_render
def render(ctx):
    global _frame
    _frame += 1
    idx = min(_stage, len(STAGE_RENDERERS) - 1)
    STAGE_RENDERERS[idx](ctx)


@app.on_key
def on_key(key: str, mods: dict, emit):
    global _stage, _anim_start
    global _hjkl_pressed, _demo_focus
    global _split_done, _tab_done, _close_done

    # Stage 0 — welcome
    if _stage == 0:
        if key == "Enter":
            _stage = 1
        return

    # Stage 1 — the pane
    if _stage == 1:
        if key == "Enter":
            _stage = 2
        return

    # Stage 2 — hjkl navigation
    if _stage == 2:
        if key in ("h", "j", "k", "l"):
            _hjkl_pressed = True
            if key == "h":
                _demo_focus = max(0, _demo_focus - 1)
            elif key == "j":
                _demo_focus = min(2, _demo_focus + 1)
            elif key == "k":
                _demo_focus = max(0, _demo_focus - 1)
            elif key == "l":
                _demo_focus = min(2, _demo_focus + 1)
            return
        if key == "Enter" and _hjkl_pressed:
            _stage = 3
            _anim_start = _frame
        return

    # Stage 3 — split
    if _stage == 3:
        if key in ("d", "D") and not _split_done:
            _split_done = True
            _anim_start = _frame
            return
        if key == "Enter" and _split_done:
            _stage = 4
            _anim_start = _frame
        return

    # Stage 4 — new context
    if _stage == 4:
        if key in ("t", "T") and not _tab_done:
            _tab_done = True
            _anim_start = _frame
            return
        if key == "Enter" and _tab_done:
            _stage = 5
            _anim_start = _frame
        return

    # Stage 5 — close
    if _stage == 5:
        if key in ("w", "W") and not _close_done:
            _close_done = True
            _anim_start = _frame
            return
        if key == "Enter" and _close_done:
            _stage = 6
        return

    # Stage 6 — reference card
    if _stage == 6:
        if key in ("r", "R"):
            reset_state()
        return


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

app.run()
