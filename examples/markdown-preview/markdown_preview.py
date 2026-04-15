#!/usr/bin/env python3
from __future__ import annotations
"""
markdown-preview — Plexi app
Live preview of a Markdown file. Polls mtime every second and re-renders on change.

Supported patterns:
  # / ## / ###   ATX headers (bold accent, scaled size)
  **bold**        bold text
  `code`          monospace green inline code
  ```…```         fenced code blocks (surface-colored background)
  - / * bullet    bullet points with • prefix
  blank lines     vertical spacing

Controls:
  j / ↓   Scroll down
  k / ↑   Scroll up
  r       Reload file now
"""

import os
import re
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

_launch_path = os.environ.get("PLEXI_LAUNCH_PATH", "")
if _launch_path and os.path.isfile(_launch_path):
    FILE_PATH: str = os.path.abspath(_launch_path)
else:
    FILE_PATH = os.path.join(os.getcwd(), "README.md")

# ---------------------------------------------------------------------------
# Colors — Catppuccin Mocha
# ---------------------------------------------------------------------------

C = {
    "bg":       "#1e1e2e",
    "surface":  "#313244",
    "surface1": "#45475a",
    "text":     "#cdd6f4",
    "subtext":  "#6c7086",
    "accent":   "#89b4fa",
    "green":    "#a6e3a1",
    "peach":    "#fab387",
    "header":   "#181825",
}

PADDING    = 16
LINE_H     = 20
HEADER_H   = 40

# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------

_lines:       list[dict] = []   # parsed render lines
_scroll:      int = 0
_last_mtime:  float = 0.0
_last_poll:   float = 0.0
_error_msg:   str = ""

# ---------------------------------------------------------------------------
# Markdown parser — produces a list of "render line" dicts
# ---------------------------------------------------------------------------

_BOLD_RE  = re.compile(r"\*\*(.+?)\*\*")
_CODE_RE  = re.compile(r"`([^`]+)`")

_FENCED_OPEN  = re.compile(r"^```")
_ATX_H1       = re.compile(r"^#\s+(.*)")
_ATX_H2       = re.compile(r"^##\s+(.*)")
_ATX_H3       = re.compile(r"^###\s+(.*)")
_BULLET       = re.compile(r"^[-*]\s+(.*)")


def _parse_inline(text: str) -> list[dict]:
    """Split text into spans: {text, bold, mono}."""
    spans: list[dict] = []
    i = 0
    while i < len(text):
        # Try bold
        m = _BOLD_RE.search(text, i)
        c = _CODE_RE.search(text, i)
        # pick the earlier match
        first = None
        if m and c:
            first = m if m.start() <= c.start() else c
        elif m:
            first = m
        elif c:
            first = c

        if first is None:
            spans.append({"text": text[i:], "bold": False, "mono": False})
            break

        if first.start() > i:
            spans.append({"text": text[i:first.start()], "bold": False, "mono": False})

        if first is m:
            spans.append({"text": first.group(1), "bold": True, "mono": False})
        else:
            spans.append({"text": first.group(1), "bold": False, "mono": True})
        i = first.end()

    return spans


def _parse(content: str) -> list[dict]:
    """
    Returns a flat list of render-line dicts:
      {kind: "h1"|"h2"|"h3"|"bullet"|"code_line"|"para"|"blank"|"fence_start"|"fence_end",
       spans: [...], raw: str}
    """
    result: list[dict] = []
    in_fence = False

    for raw_line in content.splitlines():
        line = raw_line

        if in_fence:
            if _FENCED_OPEN.match(line):
                in_fence = False
                result.append({"kind": "fence_end", "spans": [], "raw": line})
            else:
                result.append({"kind": "code_line", "spans": [{"text": line, "bold": False, "mono": True}], "raw": line})
            continue

        if _FENCED_OPEN.match(line):
            in_fence = True
            result.append({"kind": "fence_start", "spans": [], "raw": line})
            continue

        if not line.strip():
            result.append({"kind": "blank", "spans": [], "raw": ""})
            continue

        m = _ATX_H3.match(line)
        if m:
            result.append({"kind": "h3", "spans": _parse_inline(m.group(1)), "raw": line})
            continue

        m = _ATX_H2.match(line)
        if m:
            result.append({"kind": "h2", "spans": _parse_inline(m.group(1)), "raw": line})
            continue

        m = _ATX_H1.match(line)
        if m:
            result.append({"kind": "h1", "spans": _parse_inline(m.group(1)), "raw": line})
            continue

        m = _BULLET.match(line)
        if m:
            result.append({"kind": "bullet", "spans": _parse_inline("• " + m.group(1)), "raw": line})
            continue

        result.append({"kind": "para", "spans": _parse_inline(line), "raw": line})

    return result


def _reload():
    global _lines, _error_msg, _last_mtime
    try:
        mtime = os.path.getmtime(FILE_PATH)
        with open(FILE_PATH, "r", encoding="utf-8") as f:
            content = f.read()
        _lines = _parse(content)
        _last_mtime = mtime
        _error_msg = ""
    except FileNotFoundError:
        _error_msg = f"File not found: {FILE_PATH}"
        _lines = []
    except Exception as e:
        _error_msg = f"Error reading file: {e}"
        _lines = []


# Initial load
_reload()

# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

app = App()


@app.on_render
def render(ctx):
    global _scroll, _last_poll, _last_mtime

    # Poll mtime every second
    now = time.monotonic()
    if now - _last_poll >= 1.0:
        _last_poll = now
        try:
            mtime = os.path.getmtime(FILE_PATH)
            if mtime != _last_mtime:
                _reload()
        except OSError:
            pass

    w = ctx.width
    h = ctx.height

    # Background
    ctx.rect(0, 0, w, h, fill=C["bg"])

    # Header
    ctx.rect(0, 0, w, HEADER_H, fill=C["header"])
    fname = os.path.basename(FILE_PATH)
    ctx.text(PADDING, 12, fname, size=13, color=C["accent"], bold=True)
    hint = "j/k=scroll  r=reload"
    ctx.text(w - len(hint) * 7.2 - PADDING, 14, hint, size=11, color=C["subtext"])
    ctx.line(0, HEADER_H, w, HEADER_H, color=C["surface"], width=1.0)

    if _error_msg:
        ctx.text(PADDING, HEADER_H + 20, _error_msg, size=13, color=C["peach"])
        return

    # Render lines
    body_h = h - HEADER_H - PADDING
    visible = max(1, int(body_h / LINE_H))

    # Clamp scroll
    max_scroll = max(0, len(_lines) - visible)
    clamped = max(0, min(_scroll, max_scroll))
    if clamped != _scroll:
        _scroll = clamped

    y = HEADER_H + PADDING
    in_fence = False
    fence_x = PADDING
    fence_rects: list[tuple[int, int]] = []   # (y_start, y_end) of each fence block

    # First pass: identify fence regions for background rects
    fi = 0
    fs: float | None = None
    for rline in _lines:
        if rline["kind"] == "fence_start":
            fs = fi
        elif rline["kind"] == "fence_end" and fs is not None:
            fence_rects.append((fs, fi))
            fs = None
        fi += 1

    visible_lines = _lines[_scroll: _scroll + visible + 2]

    # Draw fence backgrounds first
    yi = HEADER_H + PADDING
    abs_i = _scroll
    fence_bg_drawn: set[int] = set()
    for idx, rline in enumerate(_lines[_scroll:_scroll + visible + 2]):
        real_idx = _scroll + idx
        for (fb, fe) in fence_rects:
            if fb <= real_idx <= fe and fb not in fence_bg_drawn:
                # find how many lines in this block are visible
                start_y = HEADER_H + PADDING + idx * LINE_H
                block_len = fe - fb + 1
                ctx.rect(
                    PADDING - 4, start_y - 2,
                    w - PADDING * 2 + 8, block_len * LINE_H + 4,
                    fill=C["surface"], radius=4.0,
                )
                fence_bg_drawn.add(fb)

    # Draw text
    for idx, rline in enumerate(visible_lines):
        ly = HEADER_H + PADDING + idx * LINE_H
        kind = rline["kind"]

        if kind == "blank":
            continue
        if kind in ("fence_start", "fence_end"):
            continue

        if kind == "h1":
            x = PADDING
            for sp in rline["spans"]:
                ctx.text(x, ly - 2, sp["text"], size=20, color=C["accent"], bold=True)
                x += len(sp["text"]) * 11.5
        elif kind == "h2":
            x = PADDING
            for sp in rline["spans"]:
                ctx.text(x, ly - 1, sp["text"], size=16, color=C["accent"], bold=True)
                x += len(sp["text"]) * 9.2
        elif kind == "h3":
            x = PADDING
            for sp in rline["spans"]:
                ctx.text(x, ly, sp["text"], size=14, color=C["peach"], bold=True)
                x += len(sp["text"]) * 8.0
        elif kind == "code_line":
            x = PADDING + 4
            for sp in rline["spans"]:
                ctx.text(x, ly, sp["text"], size=12, color=C["green"], monospace=True)
        elif kind == "bullet":
            x = PADDING
            for sp in rline["spans"]:
                color = C["text"]
                ctx.text(x, ly, sp["text"], size=13, color=color, bold=sp["bold"],
                         monospace=sp["mono"])
                x += len(sp["text"]) * (7.2 if not sp["mono"] else 7.0)
        else:  # para
            x = PADDING
            for sp in rline["spans"]:
                color = C["green"] if sp["mono"] else C["text"]
                ctx.text(x, ly, sp["text"], size=13, color=color,
                         bold=sp["bold"], monospace=sp["mono"])
                x += len(sp["text"]) * (7.0 if sp["mono"] else 7.2)

    # Scroll %
    if len(_lines) > visible:
        pct = int(_scroll / max(1, max_scroll) * 100)
        label = f"{pct}%"
        ctx.text(w - PADDING - len(label) * 7, h - 12, label, size=11, color=C["subtext"])


@app.on_key
def on_key(key, _mods, _emit):
    global _scroll
    if key in ("j", "ArrowDown"):
        _scroll += 1
    elif key in ("k", "ArrowUp"):
        _scroll = max(0, _scroll - 1)
    elif key == "r":
        _reload()
        _scroll = 0


app.run()
