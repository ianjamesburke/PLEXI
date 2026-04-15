#!/usr/bin/env python3
from __future__ import annotations
"""
diff-viewer — Plexi app
Renders `git diff HEAD` with syntax highlighting.

Controls:
  j / ↓   Scroll down
  k / ↑   Scroll up
  r       Re-run diff
"""

import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Colors — Catppuccin Mocha
# ---------------------------------------------------------------------------

C = {
    "bg":      "#1e1e2e",
    "surface": "#313244",
    "text":    "#cdd6f4",
    "subtext": "#6c7086",
    "accent":  "#89b4fa",
    "green":   "#a6e3a1",
    "red":     "#f38ba8",
    "peach":   "#fab387",
    "header":  "#181825",
}

PADDING  = 16
LINE_H   = 17
HEADER_H = 40

# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------

diff_lines: list[dict] = []   # {text: str, color: str}
scroll:     int = 0
error_msg:  str = ""

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _color_for_line(line: str) -> str:
    if line.startswith("+") and not line.startswith("+++"):
        return C["green"]
    if line.startswith("-") and not line.startswith("---"):
        return C["red"]
    if line.startswith("@@"):
        return C["accent"]
    if line.startswith("diff ") or line.startswith("index "):
        return C["subtext"]
    return C["text"]


def _run_diff():
    global diff_lines, error_msg, scroll
    try:
        result = subprocess.run(
            ["git", "diff", "HEAD"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        if result.returncode != 0 and result.stderr:
            stderr = result.stderr.strip()
            if "not a git repository" in stderr.lower():
                error_msg = "Not a git repository."
                diff_lines = []
                return
            error_msg = f"git error: {stderr[:120]}"
            diff_lines = []
            return

        raw = result.stdout
        error_msg = ""
        if not raw.strip():
            diff_lines = [{"text": "No changes (working tree is clean).", "color": C["subtext"]}]
        else:
            diff_lines = [
                {"text": line, "color": _color_for_line(line)}
                for line in raw.splitlines()
            ]
        scroll = 0
    except FileNotFoundError:
        error_msg = "git not found in PATH."
        diff_lines = []
    except subprocess.TimeoutExpired:
        error_msg = "git diff timed out."
        diff_lines = []
    except Exception as e:
        error_msg = f"Unexpected error: {e}"
        diff_lines = []


# Initial run
_run_diff()

# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

app = App()


@app.on_render
def render(ctx):
    global scroll

    w = ctx.width
    h = ctx.height

    ctx.rect(0, 0, w, h, fill=C["bg"])

    # Header
    ctx.rect(0, 0, w, HEADER_H, fill=C["header"])
    ctx.text(PADDING, 12, "Diff Viewer — git diff HEAD", size=13, color=C["accent"], bold=True)
    hint = "j/k=scroll  r=refresh"
    ctx.text(w - len(hint) * 7.2 - PADDING, 14, hint, size=11, color=C["subtext"])
    ctx.line(0, HEADER_H, w, HEADER_H, color=C["surface"], width=1.0)

    if error_msg:
        ctx.text(PADDING, HEADER_H + 20, error_msg, size=13, color=C["peach"])
        return

    # Body
    body_h = h - HEADER_H - PADDING
    visible = max(1, int(body_h / LINE_H))
    max_scroll = max(0, len(diff_lines) - visible)
    clamped = max(0, min(scroll, max_scroll))
    if clamped != scroll:
        scroll = clamped

    for i, entry in enumerate(diff_lines[scroll: scroll + visible]):
        y = HEADER_H + PADDING + i * LINE_H
        text = entry["text"]
        color = entry["color"]
        # Truncate to approximate visible width
        max_chars = int((w - PADDING * 2) / 7.0)
        if len(text) > max_chars:
            text = text[:max_chars - 1] + "…"
        ctx.text(PADDING, y, text, size=12, color=color, monospace=True)

    # Scroll indicator
    if len(diff_lines) > visible:
        pct = int(scroll / max(1, max_scroll) * 100)
        label = f"{pct}%  {scroll + 1}/{len(diff_lines)}"
        ctx.text(w - PADDING - len(label) * 7, h - 12, label, size=11, color=C["subtext"])


@app.on_key
def on_key(key, _mods, _emit):
    global scroll
    if key in ("j", "ArrowDown"):
        scroll += 1
    elif key in ("k", "ArrowUp"):
        scroll = max(0, scroll - 1)
    elif key == "r":
        _run_diff()


app.run()
