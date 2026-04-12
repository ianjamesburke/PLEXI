#!/usr/bin/env python3
from __future__ import annotations
"""
git-blame — Plexi app
Live git blame companion pane. Shows blame per line with age-colored commit hashes,
author names, and line content. Press Enter to view a diff, / to filter by author.

Controls:
  j / ArrowDown     Next line
  k / ArrowUp       Previous line
  Enter             Show diff for commit under cursor
  Esc               Close diff popup
  /                 Enter filter mode (type author name)
  r                 Reload blame
"""

import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Colors — Catppuccin Mocha
# ---------------------------------------------------------------------------

C = {
    "bg":      "#1e1e2e",
    "surface": "#313244",
    "header":  "#181825",
    "text":    "#cdd6f4",
    "subtext": "#6c7086",
    "accent":  "#89b4fa",   # recent commits (< 7 days)
    "overlay": "#45475a",
    "red":     "#f38ba8",
    "green":   "#a6e3a1",
}

HEADER_H = 40.0
ITEM_H = 22.0
CHAR_W = 7.5  # approximate monospace char width at size 12

# ---------------------------------------------------------------------------
# Data structures
# ---------------------------------------------------------------------------


class BlameEntry:
    __slots__ = ("hash", "short_hash", "author", "author_time", "summary", "line_no", "content")

    def __init__(self):
        self.hash: str = ""
        self.short_hash: str = ""
        self.author: str = ""
        self.author_time: int = 0
        self.summary: str = ""
        self.line_no: int = 0
        self.content: str = ""


# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------

blame_lines: list[BlameEntry] = []
filtered_lines: list[BlameEntry] = []
current_file: str = ""
branch_name: str = ""
error_msg: str = ""

cursor: int = 0
scroll_offset: int = 0

# Diff popup
diff_lines: list[str] = []
diff_scroll: int = 0
show_diff: bool = False

# Filter mode
filter_mode: bool = False
filter_text: str = ""

NOW = int(time.time())

# ---------------------------------------------------------------------------
# Git helpers
# ---------------------------------------------------------------------------


def git_branch() -> str:
    try:
        result = subprocess.run(
            ["git", "rev-parse", "--abbrev-ref", "HEAD"],
            capture_output=True, text=True, timeout=3,
        )
        return result.stdout.strip() if result.returncode == 0 else ""
    except Exception:
        return ""


def find_recent_tracked_file() -> str:
    """Return the most recently modified tracked file in cwd."""
    try:
        result = subprocess.run(
            ["git", "ls-files"],
            capture_output=True, text=True, timeout=5,
        )
        if result.returncode != 0 or not result.stdout.strip():
            return ""
        files = result.stdout.strip().splitlines()
        best = ""
        best_mtime = -1.0
        for f in files:
            try:
                mtime = os.path.getmtime(f)
                if mtime > best_mtime:
                    best_mtime = mtime
                    best = f
            except OSError:
                continue
        return best
    except Exception:
        return ""


def parse_blame(raw: str) -> list[BlameEntry]:
    """Parse git blame --porcelain output into BlameEntry list."""
    entries: list[BlameEntry] = []
    current: BlameEntry | None = None
    commit_cache: dict[str, BlameEntry] = {}

    for line in raw.splitlines():
        if not line:
            continue

        # Hunk header: 40-char hex hash followed by a space and line numbers
        if len(line) > 40 and line[40] == " " and all(c in "0123456789abcdef" for c in line[:40]):
            parts = line.split()
            if len(parts) >= 3:
                current = BlameEntry()
                current.hash = parts[0]
                current.short_hash = parts[0][:7]
                current.line_no = int(parts[2]) if len(parts) >= 3 else 0
                if parts[0] in commit_cache:
                    cached = commit_cache[parts[0]]
                    current.author = cached.author
                    current.author_time = cached.author_time
                    current.summary = cached.summary
                continue

        if current is None:
            continue

        if line.startswith("\t"):
            current.content = line[1:]
            commit_cache[current.hash] = current
            entries.append(current)
            current = None
        elif line.startswith("author "):
            current.author = line[7:]
        elif line.startswith("author-time "):
            try:
                current.author_time = int(line[12:])
            except ValueError:
                current.author_time = 0
        elif line.startswith("summary "):
            current.summary = line[8:]

    return entries


def load_blame(filepath: str):
    global blame_lines, filtered_lines, current_file, branch_name, error_msg, cursor, scroll_offset, NOW

    NOW = int(time.time())
    current_file = filepath
    branch_name = git_branch()
    error_msg = ""

    try:
        result = subprocess.run(
            ["git", "blame", "--porcelain", filepath],
            capture_output=True, text=True, timeout=10,
        )
        if result.returncode != 0:
            error_msg = result.stderr.strip() or f"git blame failed for {filepath}"
            blame_lines = []
            filtered_lines = []
            return
        blame_lines = parse_blame(result.stdout)
        apply_filter()
    except Exception as e:
        error_msg = f"Error running git blame: {e}"
        blame_lines = []
        filtered_lines = []


def apply_filter():
    global filtered_lines, cursor
    if filter_text:
        q = filter_text.lower()
        filtered_lines = [e for e in blame_lines if q in e.author.lower()]
    else:
        filtered_lines = blame_lines
    cursor = min(cursor, max(0, len(filtered_lines) - 1))


def load_diff(commit_hash: str):
    global diff_lines, diff_scroll, show_diff
    try:
        result = subprocess.run(
            ["git", "show", "--color=never", commit_hash],
            capture_output=True, text=True, timeout=10,
        )
        diff_lines = result.stdout.splitlines() if result.returncode == 0 else [
            result.stderr.strip() or "git show failed"
        ]
    except Exception as e:
        diff_lines = [f"Error: {e}"]
    diff_scroll = 0
    show_diff = True


# ---------------------------------------------------------------------------
# Age coloring
# ---------------------------------------------------------------------------

def age_color(author_time: int) -> str:
    age_secs = NOW - author_time
    if age_secs < 7 * 86400:
        return C["accent"]    # blue — recent
    if age_secs < 30 * 86400:
        return C["text"]      # normal — medium
    return C["subtext"]       # dimmed — old


def diff_line_color(line: str) -> str:
    if line.startswith("+") and not line.startswith("+++"):
        return C["green"]
    if line.startswith("-") and not line.startswith("---"):
        return C["red"]
    if line.startswith("@@"):
        return C["accent"]
    if line.startswith("commit ") or line.startswith("diff "):
        return C["text"]
    return C["subtext"]


# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

app = App(app_id="git-blame")

# Initial load
_initial_file = find_recent_tracked_file()
if _initial_file:
    load_blame(_initial_file)
else:
    error_msg = "No git repo found or no tracked files."


@app.on_render
def render(ctx):
    global scroll_offset

    ctx.rect(0, 0, ctx.width, ctx.height, fill=C["bg"])

    if error_msg and not blame_lines:
        _render_header(ctx)
        ctx.text(16, HEADER_H + 20, error_msg, size=13, color=C["subtext"])
        return

    lines = filtered_lines
    visible_rows = max(1, int((ctx.height - HEADER_H) / ITEM_H))

    # Clamp scroll so cursor stays visible
    if cursor < scroll_offset:
        scroll_offset = cursor
    elif cursor >= scroll_offset + visible_rows:
        scroll_offset = cursor - visible_rows + 1

    # Column layout
    hash_col_w = 8 * CHAR_W    # "abc1234 "
    author_col_w = 11 * CHAR_W # "FirstName  "
    content_x = hash_col_w + author_col_w + 8
    content_max_chars = max(10, int((ctx.width - content_x - 8) / CHAR_W))

    for i in range(scroll_offset, min(scroll_offset + visible_rows, len(lines))):
        entry = lines[i]
        row_index = i - scroll_offset
        y = HEADER_H + row_index * ITEM_H
        is_cursor = (i == cursor)

        if is_cursor:
            ctx.rect(0, y, ctx.width, ITEM_H, fill=C["surface"])

        # Short hash — age colored
        ctx.text(8, y + 4, entry.short_hash, size=12,
                 color=age_color(entry.author_time), monospace=True)

        # Author first name (truncated to 10 chars)
        first_name = (entry.author.split()[0] if entry.author else "?")[:10]
        ctx.text(hash_col_w + 8, y + 4, first_name, size=12,
                 color=C["text"] if is_cursor else C["subtext"])

        # Line content (truncated to fit)
        content = entry.content
        if len(content) > content_max_chars:
            content = content[:content_max_chars - 1] + "…"
        ctx.text(content_x, y + 4, content, size=12,
                 color=C["text"], monospace=True)

    _render_header(ctx)

    # Filter bar at bottom
    if filter_mode:
        bar_y = ctx.height - 28
        ctx.rect(0, bar_y, ctx.width, 28, fill=C["surface"])
        ctx.text(12, bar_y + 6, f"/ {filter_text}_", size=12,
                 color=C["accent"], monospace=True)

    if show_diff:
        _render_diff(ctx)


def _render_header(ctx):
    ctx.rect(0, 0, ctx.width, HEADER_H, fill=C["header"])
    title = current_file or "git blame"
    if branch_name:
        title = f"{title}  [{branch_name}]"
    ctx.text(12, 11, title, size=13, color=C["accent"], bold=True)
    hint = "j/k  Enter=diff  /=filter  r=reload"
    ctx.text(ctx.width - len(hint) * 6.5 - 8, 11, hint, size=11, color=C["subtext"])


def _render_diff(ctx):
    pw = min(ctx.width - 32, 900.0)
    ph = ctx.height - 80
    px = (ctx.width - pw) / 2
    py = 40.0

    ctx.rect(px, py, pw, ph, fill=C["bg"], radius=6.0)
    ctx.rect(px, py, pw, 32, fill=C["header"], radius=6.0)
    ctx.text(px + 12, py + 8, "git show", size=13, color=C["accent"], bold=True)
    ctx.text(px + pw - 100, py + 8, "Esc to close", size=11, color=C["subtext"])

    inner_y = py + 36
    inner_h = ph - 36
    visible = max(1, int(inner_h / ITEM_H))
    max_chars = max(10, int((pw - 24) / CHAR_W))

    for i in range(diff_scroll, min(diff_scroll + visible, len(diff_lines))):
        line = diff_lines[i]
        row_y = inner_y + (i - diff_scroll) * ITEM_H
        display = line if len(line) <= max_chars else line[:max_chars - 1] + "…"
        ctx.text(px + 12, row_y + 3, display, size=12,
                 color=diff_line_color(line), monospace=True)


@app.on_key
def on_key(key, mods, emit):
    global cursor, scroll_offset, show_diff, diff_scroll
    global filter_mode, filter_text

    if show_diff:
        if key == "Escape":
            show_diff = False
        elif key in ("j", "ArrowDown"):
            diff_scroll = min(diff_scroll + 1, max(0, len(diff_lines) - 1))
        elif key in ("k", "ArrowUp"):
            diff_scroll = max(diff_scroll - 1, 0)
        return

    if filter_mode:
        if key in ("Escape", "Enter"):
            filter_mode = False
        elif key == "Backspace":
            filter_text = filter_text[:-1]
            apply_filter()
        elif len(key) == 1:
            filter_text += key
            apply_filter()
        return

    lines = filtered_lines
    if key in ("j", "ArrowDown"):
        cursor = min(cursor + 1, max(0, len(lines) - 1))
    elif key in ("k", "ArrowUp"):
        cursor = max(cursor - 1, 0)
    elif key == "Enter" and lines:
        load_diff(lines[cursor].hash)
    elif key == "/":
        filter_mode = True
        filter_text = ""
        apply_filter()
    elif key == "r":
        if current_file:
            load_blame(current_file)
        cursor = 0
        scroll_offset = 0


@app.on_command
def on_command(text, emit):
    """Accept a file path as command argument to switch blame target."""
    path = text.strip()
    if not path:
        return
    if os.path.isfile(path):
        load_blame(path)
    else:
        abs_path = os.path.join(os.getcwd(), path)
        if os.path.isfile(abs_path):
            load_blame(abs_path)


app.run()
