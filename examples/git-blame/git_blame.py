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
  Esc               Close diff popup (in popup), exit filter mode
  /                 Enter filter mode (type author name)
  r                 Reload blame
  ⌘W                Close app (list view)
"""

import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import (
    App,
    THEME,
    BODY, HINT, MONO_SMALL,
    PAD, HEADER_H,
)

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
    global blame_lines, filtered_lines, current_file, branch_name, error_msg, cursor, NOW

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
    global diff_lines, show_diff, diff_scroll
    diff_scroll = 0
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
    show_diff = True


# ---------------------------------------------------------------------------
# Age coloring
# ---------------------------------------------------------------------------

def age_color(author_time: int) -> str:
    age_secs = NOW - author_time
    if age_secs < 7 * 86400:
        return THEME.accent    # blue — recent
    if age_secs < 30 * 86400:
        return THEME.fg        # normal — medium
    return THEME.muted         # dimmed — old


def diff_line_color(line: str) -> str:
    if line.startswith("+") and not line.startswith("+++"):
        return THEME.green
    if line.startswith("-") and not line.startswith("---"):
        return THEME.red
    if line.startswith("@@"):
        return THEME.accent
    if line.startswith("commit ") or line.startswith("diff "):
        return THEME.fg
    return THEME.muted


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


# ---------------------------------------------------------------------------
# Rendering
# ---------------------------------------------------------------------------

ROW_H = 22.0


def _render_blame_row(ctx, entry, i, x, y, w, is_sel):
    if is_sel:
        ctx.rect(x, y, w, ROW_H, fill=THEME.highlight)

    # Short hash — age-colored
    ctx.text(
        x + PAD / 2, y + 4, entry.short_hash,
        size=MONO_SMALL, color=age_color(entry.author_time), monospace=True,
    )

    # Author first name (truncated to 10 chars)
    first_name = (entry.author.split()[0] if entry.author else "?")[:10]
    author_x = x + PAD / 2 + ctx.measure_text("abcdefgh ", MONO_SMALL, monospace=True)
    ctx.text(
        author_x, y + 4, first_name,
        size=MONO_SMALL, color=THEME.fg if is_sel else THEME.muted,
    )

    # Line content — fill remaining width
    content_x = author_x + ctx.measure_text("FirstName  ", MONO_SMALL, monospace=True)
    content_budget = max(0.0, (x + w) - content_x - PAD / 2)
    content = entry.content
    max_chars = max(10, int(content_budget / (MONO_SMALL * 0.60)))
    if len(content) > max_chars:
        content = content[:max_chars - 1] + "…"
    ctx.text(
        content_x, y + 4, content,
        size=MONO_SMALL, color=THEME.fg, monospace=True,
    )


@app.on_render
def render(ctx):
    ctx.rect(0, 0, ctx.width, ctx.height, fill=THEME.bg)

    # ── header ──────────────────────────────────────────────────────────────
    title = current_file or "git blame"
    subtitle = f"[{branch_name}]" if branch_name else None
    ctx.header(title, subtitle=subtitle)

    # ── error / empty state ─────────────────────────────────────────────────
    if error_msg and not blame_lines:
        ctx.empty_state("git blame unavailable", error_msg, icon_color=THEME.red)
        ctx.status_bar([("⌘W", "close")])
        return

    lines = filtered_lines

    if not lines:
        ctx.empty_state(
            "No matching lines",
            f"filter: {filter_text}" if filter_text else "Load a tracked file",
        )
    else:
        ctx.scrollable_list(
            list_id="blame",
            items=lines,
            selected=cursor,
            row_height=ROW_H,
            render_row=_render_blame_row,
            x=0,
            y=HEADER_H,
            w=ctx.width,
            h=ctx.height - HEADER_H - 30.0,
        )

    # ── filter bar (overlay just above status bar) ──────────────────────────
    if filter_mode:
        bar_h = 24.0
        bar_y = ctx.height - 30.0 - bar_h
        ctx.rect(0, bar_y, ctx.width, bar_h, fill=THEME.surface)
        ctx.text(
            PAD, bar_y + (bar_h - HINT) / 2,
            f"/ {filter_text}_",
            size=HINT, color=THEME.accent, monospace=True,
        )

    # ── status bar ──────────────────────────────────────────────────────────
    if filter_mode:
        ctx.status_bar([("type", "filter"), ("Enter/Esc", "done")])
    else:
        ctx.status_bar([
            ("j/k", "navigate"),
            ("Enter", "diff"),
            ("/", "filter"),
            ("r", "reload"),
            ("⌘W", "close"),
        ])

    # ── diff popup overlay ──────────────────────────────────────────────────
    if show_diff:
        _render_diff(ctx)


def _render_diff(ctx):
    pw = min(ctx.width - 32, 900.0)
    ph = ctx.height - 80
    px = (ctx.width - pw) / 2
    py = 40.0

    # Dim background
    ctx.rect(px, py, pw, ph, fill=THEME.bg, radius=6.0)
    ctx.rect(px, py, pw, 32, fill=THEME.surface, radius=6.0)
    ctx.text(px + PAD, py + (32 - BODY) / 2, "git show",
             size=BODY, color=THEME.accent, bold=True)
    ctx.text_right(px + pw - PAD, py + (32 - HINT) / 2,
                   "⌘W close", size=HINT, color=THEME.muted)

    inner_x = px + PAD
    inner_y = py + 36
    inner_w = pw - PAD * 2
    line_h = MONO_SMALL + 4

    # scrollable_text applies one color globally, but diff lines need
    # per-line coloring (+/-/@@). Draw manually with module-level scroll.
    global diff_scroll
    visible = max(1, int((ph - 36) / line_h))
    max_offset = max(0, len(diff_lines) - visible)
    diff_scroll = max(0, min(diff_scroll, max_offset))
    offset = diff_scroll

    max_chars = max(10, int(inner_w / (MONO_SMALL * 0.60)))
    for i in range(visible):
        idx = offset + i
        if idx >= len(diff_lines):
            break
        line = diff_lines[idx]
        display = line if len(line) <= max_chars else line[:max_chars - 1] + "…"
        ctx.text(
            inner_x, inner_y + i * line_h, display,
            size=MONO_SMALL, color=diff_line_color(line), monospace=True,
        )

    # Scrollbar
    if len(diff_lines) > visible:
        track_x = px + pw - PAD / 2 - 3
        track_h = line_h * visible
        thumb_h = max(24.0, track_h * visible / len(diff_lines))
        denom = max(1, len(diff_lines) - visible)
        thumb_y = inner_y + (track_h - thumb_h) * (offset / denom)
        ctx.rect(track_x, inner_y, 3, track_h, fill=THEME.surface, radius=2)
        ctx.rect(track_x, thumb_y, 3, thumb_h, fill=THEME.muted, radius=2)


@app.on_key
def on_key(key, mods, emit):
    global cursor, show_diff, diff_scroll
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
