#!/usr/bin/env python3
"""
navigator — Plexi app

Harpoon-style directory hotlist. Pin directories, jump to them by slot number
or by navigating the list. Persists to ~/.plexi-alpha/navigator.json.

Controls:
  j / ArrowDown    Move selection down
  k / ArrowUp      Move selection up
  1–9              Jump directly to slot N (and cd to it)
  Enter            cd to selected directory
  a                Add cwd to hotlist
  d / Delete       Remove selected entry
  r                Refresh / reload list from disk
"""

from __future__ import annotations

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App  # noqa: E402

# ---------------------------------------------------------------------------
# Persistence
# ---------------------------------------------------------------------------

PINS_FILE = os.path.expanduser("~/.plexi-alpha/navigator.json")


def _load_pins() -> list:
    """Load pinned directories from disk. Returns empty list on any error."""
    try:
        with open(PINS_FILE, "r", encoding="utf-8") as f:
            data = json.load(f)
        pins = data.get("pins", [])
        if not isinstance(pins, list):
            return []
        return [p for p in pins if isinstance(p, str)]
    except FileNotFoundError:
        return []
    except (json.JSONDecodeError, OSError) as e:
        sys.stderr.write(f"navigator: failed to load {PINS_FILE}: {e}\n")
        return []


def _save_pins(pins: list):
    """Persist pinned directories to disk. Fails loudly on error."""
    try:
        os.makedirs(os.path.dirname(PINS_FILE), exist_ok=True)
        with open(PINS_FILE, "w", encoding="utf-8") as f:
            json.dump({"pins": pins}, f, indent=2)
    except OSError as e:
        sys.stderr.write(f"navigator: failed to save {PINS_FILE}: {e}\n")


# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------

pins: list = _load_pins()
selected: int = 0


def _clamp_selected():
    global selected
    if not pins:
        selected = 0
    else:
        selected = max(0, min(selected, len(pins) - 1))


def _shorten_path(path: str) -> str:
    """Replace $HOME prefix with ~."""
    home = os.path.expanduser("~")
    if path == home:
        return "~"
    if path.startswith(home + os.sep):
        return "~" + path[len(home):]
    return path


def _add_cwd(emit):
    """Add the current working directory to pins if not already present."""
    global pins, selected
    cwd = os.getcwd()
    if cwd not in pins:
        pins.append(cwd)
        _save_pins(pins)
        selected = len(pins) - 1
        emit.info(f"navigator: pinned {cwd}")
    else:
        # Move selection to existing entry.
        selected = pins.index(cwd)


def _remove_selected():
    global pins, selected
    if not pins:
        return
    pins.pop(selected)
    _save_pins(pins)
    _clamp_selected()


def _cd_to(path: str, emit):
    """Emit a cd command for the linked terminal."""
    emit.cd(path)


# ---------------------------------------------------------------------------
# Theme — Catppuccin Mocha
# ---------------------------------------------------------------------------

C = {
    "bg":      "#1e1e2e",
    "header":  "#181825",
    "surface": "#313244",
    "sel_bg":  "#45475a",
    "text":    "#cdd6f4",
    "subtext": "#a6adc8",
    "muted":   "#6c7086",
    "accent":  "#89b4fa",
    "green":   "#a6e3a1",
    "red":     "#f38ba8",
    "missing": "#585b70",  # dimmed color for directories that no longer exist
}

HEADER_H = 48
FOOTER_H = 28
PADDING  = 16
ITEM_H   = 28

# ---------------------------------------------------------------------------
# Render
# ---------------------------------------------------------------------------

app = App(app_id="navigator")


@app.on_render
def render(ctx):
    w = ctx.width
    h = ctx.height

    # Background
    ctx.rect(0, 0, w, h, fill=C["bg"])

    # Header
    n = len(pins)
    header_title = f"Navigator — {n} pin{'s' if n != 1 else ''}"
    ctx.rect(0, 0, w, HEADER_H, fill=C["header"])
    ctx.text(PADDING, 15, header_title, size=14, color=C["accent"], bold=True)
    hint_text = "[a] pin  [d] remove  [Enter] cd  [1–9] jump"
    hint_x = w - len(hint_text) * 6.5 - PADDING
    if hint_x > PADDING + len(header_title) * 8 + 16:
        ctx.text(hint_x, 17, hint_text, size=11, color=C["muted"])
    ctx.line(0, HEADER_H, w, HEADER_H, color=C["surface"], width=1.0)

    # Empty state
    if not pins:
        msg = "No pinned directories yet."
        hint = "Navigate to a directory in any terminal pane and press `a` to pin it."
        msg_y = HEADER_H + (h - HEADER_H - FOOTER_H) // 2 - 18
        ctx.text(PADDING, msg_y, msg, size=13, color=C["subtext"])
        # Wrap hint roughly
        max_chars = max(20, int((w - 2 * PADDING) / 7.5))
        hint_y = msg_y + 22
        word_buf = ""
        for word in hint.split():
            trial = (word_buf + " " + word).strip()
            if len(trial) > max_chars:
                ctx.text(PADDING, hint_y, word_buf.strip(), size=12, color=C["muted"])
                hint_y += 18
                word_buf = word
            else:
                word_buf = trial
        if word_buf.strip():
            ctx.text(PADDING, hint_y, word_buf.strip(), size=12, color=C["muted"])

        _draw_footer(ctx)
        return

    # List
    list_y = HEADER_H + 4
    list_h = h - HEADER_H - FOOTER_H - 4
    visible_rows = max(1, int(list_h / ITEM_H))

    # Scroll so selected stays visible.
    scroll = 0
    if selected >= visible_rows:
        scroll = selected - visible_rows + 1

    for idx in range(scroll, min(len(pins), scroll + visible_rows)):
        row = idx - scroll
        y = list_y + row * ITEM_H
        is_sel = (idx == selected)
        path = pins[idx]
        exists = os.path.isdir(path)

        if is_sel:
            ctx.rect(0, y, w, ITEM_H, fill=C["sel_bg"])

        # Slot number
        slot = idx + 1
        slot_text = str(slot) if slot <= 9 else " "
        slot_color = C["accent"] if is_sel else C["muted"]
        ctx.text(PADDING, y + 7, slot_text, size=12, color=slot_color, monospace=True)

        # Path — shortened, dimmed if missing
        short_path = _shorten_path(path)
        path_color = (C["text"] if is_sel else C["subtext"]) if exists else C["missing"]

        path_x = PADDING + 22
        # Truncate if too long
        max_chars = max(10, int((w - path_x - PADDING) / 7.5))
        display_path = short_path
        if len(display_path) > max_chars:
            display_path = "\u2026" + display_path[-(max_chars - 1):]

        ctx.text(path_x, y + 7, display_path, size=13, color=path_color, monospace=True)

        # Missing indicator
        if not exists:
            miss_text = "missing"
            miss_x = w - PADDING - len(miss_text) * 7 - 4
            if miss_x > path_x + len(display_path) * 7.5 + 8:
                ctx.text(miss_x, y + 8, miss_text, size=11, color=C["missing"])

    _draw_footer(ctx)


def _draw_footer(ctx):
    w = ctx.width
    h = ctx.height
    ctx.rect(0, h - FOOTER_H, w, FOOTER_H, fill=C["header"])
    ctx.line(0, h - FOOTER_H, w, h - FOOTER_H, color=C["surface"], width=1.0)
    footer_hint = "[j/k] navigate  [Enter] cd  [a] pin cwd  [d] remove  [r] reload"
    ctx.text(PADDING, h - FOOTER_H + 8, footer_hint, size=11, color=C["muted"])


# ---------------------------------------------------------------------------
# Input
# ---------------------------------------------------------------------------

@app.on_key
def on_key(key, _mods, emit):
    global selected, pins

    if key in ("j", "ArrowDown"):
        if pins:
            selected = min(selected + 1, len(pins) - 1)

    elif key in ("k", "ArrowUp"):
        if pins:
            selected = max(selected - 1, 0)

    elif key == "Enter":
        if pins and 0 <= selected < len(pins):
            _cd_to(pins[selected], emit)

    elif key == "a":
        _add_cwd(emit)

    elif key in ("d", "Delete"):
        _remove_selected()

    elif key == "r":
        pins = _load_pins()
        _clamp_selected()

    elif len(key) == 1 and key.isdigit() and key != "0":
        slot = int(key)
        idx = slot - 1
        if idx < len(pins):
            selected = idx
            _cd_to(pins[idx], emit)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

app.run()
