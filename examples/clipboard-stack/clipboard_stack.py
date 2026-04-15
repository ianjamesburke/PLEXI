#!/usr/bin/env python3
from __future__ import annotations
"""
clipboard_stack — Plexi app
Persistent multi-slot clipboard history with fuzzy search and pinned entries.

Controls:
  j/k or arrows     Navigate list
  Enter / y         Copy selected entry back to clipboard
  d                 Delete selected entry
  p                 Pin / unpin entry
  /                 Enter search mode
  Esc               Exit search mode
"""

import json
import os
import platform
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Colors — Catppuccin Mocha
# ---------------------------------------------------------------------------

C = {
    "bg":       "#1e1e2e",
    "surface":  "#313244",
    "text":     "#cdd6f4",
    "subtext":  "#6c7086",
    "accent":   "#89b4fa",
    "pinned":   "#cba6f7",
    "confirm":  "#a6e3a1",
    "border":   "#45475a",
}

IS_MACOS = platform.system() == "Darwin"
MAX_ENTRIES = 50
ROW_H = 36.0
HEADER_H = 36.0
SEARCH_H = 40.0
CONFIRM_DURATION = 1.2  # seconds

DATA_PATH = os.path.expanduser("~/.plexi-alpha/clipboard_stack.json")

# ---------------------------------------------------------------------------
# Type detection
# ---------------------------------------------------------------------------

def detect_type(text: str) -> str:
    s = text.strip()
    if s.startswith("http://") or s.startswith("https://"):
        return "url"
    if (s.startswith("/") and " " not in s) or s.startswith("~/"):
        return "path"
    return "text"

TYPE_ICON = {"text": "📋", "url": "🔗", "path": "📁"}

# ---------------------------------------------------------------------------
# Relative time
# ---------------------------------------------------------------------------

def rel_time(ts: float) -> str:
    delta = int(time.time() - ts)
    if delta < 60:
        return f"{delta}s ago"
    if delta < 3600:
        return f"{delta // 60}m ago"
    if delta < 86400:
        return f"{delta // 3600}h ago"
    return f"{delta // 86400}d ago"

# ---------------------------------------------------------------------------
# Persistence
# ---------------------------------------------------------------------------

def load_stack() -> list[dict]:
    try:
        with open(DATA_PATH) as f:
            data = json.load(f)
        if isinstance(data, list):
            return data
    except (OSError, json.JSONDecodeError):
        pass
    return []

def save_stack(stack: list[dict]) -> None:
    try:
        os.makedirs(os.path.dirname(DATA_PATH), exist_ok=True)
        with open(DATA_PATH, "w") as f:
            json.dump(stack, f, indent=2)
    except OSError as e:
        pass  # non-fatal; log skipped to avoid spam

# ---------------------------------------------------------------------------
# Clipboard I/O
# ---------------------------------------------------------------------------

def read_clipboard() -> str | None:
    if not IS_MACOS:
        return None
    try:
        result = subprocess.run(["pbpaste"], capture_output=True, text=True, timeout=1)
        return result.stdout
    except Exception:
        return None

def write_clipboard(text: str) -> None:
    if not IS_MACOS:
        return
    try:
        subprocess.run(["pbcopy"], input=text, text=True, timeout=1)
    except Exception:
        pass

# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------

stack: list[dict] = load_stack()
selected: int = 0
scroll_offset: int = 0
search_mode: bool = False
search_query: str = ""
last_clipboard: str = read_clipboard() or ""
confirm_until: float = 0.0
confirm_text: str = ""

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def filtered_entries() -> list[dict]:
    """Pinned entries first, then rest. Filter by search_query if set."""
    q = search_query.lower()
    pinned = [e for e in stack if e.get("pinned")]
    rest = [e for e in stack if not e.get("pinned")]
    combined = pinned + rest
    if not q:
        return combined
    return [e for e in combined if q in e.get("content", "").lower()]

def clamp_selection(entries: list[dict]) -> None:
    global selected, scroll_offset
    if not entries:
        selected = 0
        scroll_offset = 0
        return
    selected = max(0, min(selected, len(entries) - 1))

def push_entry(content: str) -> None:
    # Avoid exact duplicate at top of stack (respecting pinned order)
    unpinned = [e for e in stack if not e.get("pinned")]
    if unpinned and unpinned[0].get("content") == content:
        return
    # Also skip if it already exists at top of full stack (would just be a dup)
    if stack and stack[0].get("content") == content and not stack[0].get("pinned"):
        return
    entry = {
        "content": content,
        "ts": time.time(),
        "kind": detect_type(content),
        "pinned": False,
    }
    # Insert after any pinned entries at front
    insert_at = sum(1 for e in stack if e.get("pinned"))
    stack.insert(insert_at, entry)
    # Trim unpinned to keep total under MAX_ENTRIES (never trim pinned)
    unpinned_entries = [e for e in stack if not e.get("pinned")]
    pinned_entries = [e for e in stack if e.get("pinned")]
    if len(unpinned_entries) > MAX_ENTRIES:
        unpinned_entries = unpinned_entries[:MAX_ENTRIES]
    stack.clear()
    stack.extend(pinned_entries + unpinned_entries)
    save_stack(stack)

def confirm(msg: str) -> None:
    global confirm_until, confirm_text
    confirm_until = time.monotonic() + CONFIRM_DURATION
    confirm_text = msg

# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

app = App(app_id="clipboard-stack")


@app.on_render
def render(ctx):
    global last_clipboard, selected, scroll_offset

    # Poll clipboard
    if IS_MACOS:
        current = read_clipboard()
        if current is not None and current != last_clipboard and current.strip():
            last_clipboard = current
            push_entry(current)

    entries = filtered_entries()
    clamp_selection(entries)

    w = ctx.width
    h = ctx.height

    # Background
    ctx.rect(0, 0, w, h, fill=C["bg"])

    if not IS_MACOS:
        ctx.text(20, h / 2 - 10, "Clipboard Stack requires macOS (pbpaste/pbcopy).",
                 size=14, color=C["subtext"])
        return

    # Header
    ctx.rect(0, 0, w, HEADER_H, fill=C["surface"])
    ctx.text(12, 10, "📋 Clipboard Stack", size=14, color=C["text"], bold=True)
    count_label = f"{len(stack)} entries"
    ctx.text(w - len(count_label) * 7.5 - 12, 10, count_label, size=12, color=C["subtext"])

    # List area
    list_top = HEADER_H
    list_bottom = h - (SEARCH_H if search_mode else 0)
    list_h = list_bottom - list_top
    visible_rows = max(1, int(list_h // ROW_H))

    # Ensure selected is visible
    if selected < scroll_offset:
        scroll_offset = selected
    elif selected >= scroll_offset + visible_rows:
        scroll_offset = selected - visible_rows + 1

    if not entries:
        msg = "No matches." if search_query else "Nothing copied yet."
        ctx.text(12, list_top + 20, msg, size=13, color=C["subtext"])
    else:
        for vi in range(visible_rows):
            ei = scroll_offset + vi
            if ei >= len(entries):
                break
            entry = entries[ei]
            ry = list_top + vi * ROW_H
            is_selected = (ei == selected)
            is_pinned = entry.get("pinned", False)

            # Row background
            if is_selected:
                ctx.rect(0, ry, w, ROW_H, fill=C["surface"])
                # Left accent bar
                accent_color = C["pinned"] if is_pinned else C["accent"]
                ctx.rect(0, ry, 4, ROW_H, fill=accent_color)

            # Index
            idx_label = f"{ei + 1:2d}"
            idx_color = C["pinned"] if is_pinned else C["subtext"]
            ctx.text(10, ry + 10, idx_label, size=11, color=idx_color, monospace=True)

            # Pin icon
            if is_pinned:
                ctx.text(34, ry + 10, "📌", size=11, color=C["pinned"])

            # Type icon
            icon = TYPE_ICON.get(entry.get("kind", "text"), "📋")
            ctx.text(52, ry + 10, icon, size=11, color=C["subtext"])

            # Content preview — truncate to available width
            content = entry.get("content", "")
            first_line = content.split("\n")[0]
            max_chars = max(10, int((w - 150) / 7.5))
            if len(first_line) > max_chars:
                first_line = first_line[:max_chars - 1] + "…"
            text_color = C["text"] if is_selected else C["text"]
            ctx.text(70, ry + 10, first_line, size=12, color=text_color, monospace=True)

            # Timestamp
            ts_label = rel_time(entry.get("ts", time.time()))
            ctx.text(w - len(ts_label) * 7 - 10, ry + 10, ts_label, size=11, color=C["subtext"])

            # Row separator
            if not is_selected:
                ctx.line(0, ry + ROW_H - 1, w, ry + ROW_H - 1, color=C["border"], width=0.5)

    # Search bar
    if search_mode:
        sb_y = h - SEARCH_H
        ctx.rect(0, sb_y, w, SEARCH_H, fill=C["surface"])
        ctx.text(12, sb_y + 12, "/", size=14, color=C["accent"], bold=True)
        query_display = search_query + "█"
        ctx.text(28, sb_y + 12, query_display, size=13, color=C["text"], monospace=True)

    # Confirmation flash
    if time.monotonic() < confirm_until:
        ctx.rect(w / 2 - 120, h / 2 - 20, 240, 40, fill=C["surface"], radius=8.0)
        ctx.text(w / 2 - len(confirm_text) * 5, h / 2 - 8, confirm_text,
                 size=13, color=C["confirm"], bold=True)

    # Help hint at bottom (when not searching)
    if not search_mode:
        hint = "j/k navigate  Enter copy  d del  p pin  / search"
        ctx.text(12, h - 18, hint, size=10, color=C["subtext"])


@app.on_key
def on_key(key, _mods, _emit):
    global selected, scroll_offset, search_mode, search_query

    entries = filtered_entries()

    if search_mode:
        if key == "Escape":
            search_mode = False
            search_query = ""
            selected = 0
        elif key == "Backspace":
            search_query = search_query[:-1]
            selected = 0
        elif len(key) == 1:
            search_query += key
            selected = 0
        return

    # Normal mode
    if key in ("j", "ArrowDown"):
        selected = min(selected + 1, max(0, len(entries) - 1))
    elif key in ("k", "ArrowUp"):
        selected = max(selected - 1, 0)
    elif key == "slash" or key == "/":
        search_mode = True
        search_query = ""
        selected = 0
    elif key in ("Enter", "y"):
        if entries:
            entry = entries[selected]
            write_clipboard(entry["content"])
            confirm("Copied to clipboard!")
    elif key == "d":
        if entries:
            entry = entries[selected]
            if entry in stack:
                stack.remove(entry)
                save_stack(stack)
            clamp_selection(filtered_entries())
    elif key == "p":
        if entries:
            entry = entries[selected]
            if entry in stack:
                idx = stack.index(entry)
                stack[idx]["pinned"] = not stack[idx].get("pinned", False)
                # Re-sort: pinned to front
                pinned = [e for e in stack if e.get("pinned")]
                unpinned = [e for e in stack if not e.get("pinned")]
                stack.clear()
                stack.extend(pinned + unpinned)
                save_stack(stack)


app.run()
