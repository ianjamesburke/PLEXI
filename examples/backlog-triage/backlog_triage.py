from __future__ import annotations
"""
backlog_triage.py — Keyboard-driven review of ~/.plexi/backlog/ notes.

Keys:
  j / Down    next note
  k / Up      previous note
  Enter       expand/collapse full text
  d           dismiss (delete) — moves to ~/.plexi/backlog/.dismissed/
  a           archive — moves to ~/Documents/archive/plexi-backlog/ (creates if missing)
  f           submit feedback (thumbs up / useful reminder)
  q / Esc     quit
"""

import pathlib
import shutil
from datetime import datetime
from plexi_sdk import App, load_manifest

# ── colour palette ───────────────────────────────────────────────────────────
BG        = "#1e1e2e"
SURFACE   = "#313244"
HIGHLIGHT = "#45475a"
ACCENT    = "#89b4fa"
MUTED     = "#6c7086"
FG        = "#cdd6f4"
RED       = "#f38ba8"
GREEN     = "#a6e3a1"
YELLOW    = "#f9e2af"

# ── locate backlog dirs ───────────────────────────────────────────────────────
def _backlog_dirs() -> list[pathlib.Path]:
    """Return existing backlog directories, alpha-first."""
    candidates = [
        pathlib.Path.home() / ".plexi-alpha" / "backlog",
        pathlib.Path.home() / ".plexi-beta" / "backlog",
        pathlib.Path.home() / ".plexi" / "backlog",
    ]
    return [p for p in candidates if p.exists()]


def _load_notes() -> list[dict]:
    """Load all .md notes from backlog dirs, newest first."""
    notes = []
    seen: set[str] = set()
    for d in _backlog_dirs():
        for f in sorted(d.glob("*.md"), key=lambda p: p.stat().st_mtime, reverse=True):
            if f.name not in seen:
                seen.add(f.name)
                try:
                    text = f.read_text(errors="replace").strip()
                    notes.append({
                        "path": f,
                        "name": f.name,
                        "text": text,
                        "preview": text.splitlines()[0][:120] if text else "(empty)",
                        "mtime": datetime.fromtimestamp(f.stat().st_mtime).strftime("%b %d %H:%M"),
                    })
                except OSError:
                    pass
    return notes


# ── state ─────────────────────────────────────────────────────────────────────
notes: list[dict] = _load_notes()
selected: int = 0
expanded: bool = False
status_msg: str = ""
status_color: str = GREEN

manifest = load_manifest(__file__)
APP_VERSION = manifest.get("app", {}).get("version", "0.1.0")

app = App("backlog-triage")


def _clamp(idx: int) -> int:
    if not notes:
        return 0
    return max(0, min(len(notes) - 1, idx))


def _dismiss(idx: int) -> str:
    note = notes[idx]
    dst_dir = note["path"].parent / ".dismissed"
    try:
        dst_dir.mkdir(exist_ok=True)
        shutil.move(str(note["path"]), str(dst_dir / note["name"]))
        return f"Dismissed: {note['name']}"
    except OSError as e:
        return f"Error: {e}"


def _archive(idx: int) -> str:
    note = notes[idx]
    archive_dir = pathlib.Path.home() / "Documents" / "archive" / "plexi-backlog"
    try:
        archive_dir.mkdir(parents=True, exist_ok=True)
        dst = archive_dir / note["name"]
        shutil.move(str(note["path"]), str(dst))
        return f"Archived to {archive_dir.name}/{note['name']}"
    except OSError as e:
        return f"Error: {e}"


@app.on_render
def render(ctx):
    global selected, expanded, status_msg, status_color

    ctx.rect(0, 0, ctx.width, ctx.height, fill=BG)

    # ── header ────────────────────────────────────────────────────────────────
    ctx.rect(0, 0, ctx.width, 36, fill=SURFACE)
    title = f"Backlog Triage  ·  {len(notes)} note{'s' if len(notes) != 1 else ''}"
    ctx.text(16, 10, title, size=14, color=ACCENT, bold=True)
    ctx.text(ctx.width - 200, 10, "j/k navigate  d dismiss  a archive", size=11, color=MUTED)

    if not notes:
        ctx.text(ctx.width / 2 - 80, ctx.height / 2, "No backlog notes — you're clear!", size=15, color=GREEN)
        return

    # ── note list ─────────────────────────────────────────────────────────────
    list_top = 44.0
    row_h = 52.0
    visible = max(1, int((ctx.height - list_top - 80) / row_h))
    scroll_off = max(0, selected - visible + 1)

    for i in range(visible):
        ni = scroll_off + i
        if ni >= len(notes):
            break
        note = notes[ni]
        y = list_top + i * row_h
        is_sel = ni == selected

        bg = HIGHLIGHT if is_sel else BG
        ctx.rect(0, y, ctx.width, row_h - 2, fill=bg, radius=4)

        dot_color = ACCENT if is_sel else MUTED
        ctx.rect(10, y + 19, 6, 6, fill=dot_color, radius=3)

        preview = note["preview"]
        if len(preview) > 80:
            preview = preview[:77] + "…"
        ctx.text(26, y + 8, preview, size=13, color=FG if is_sel else MUTED, bold=is_sel)
        ctx.text(26, y + 28, f"{note['mtime']}  ·  {note['name']}", size=10, color=MUTED)

        # scroll bar hint
        if len(notes) > visible:
            bar_h = ctx.height - list_top - 80
            thumb = max(20, bar_h * visible / len(notes))
            thumb_y = list_top + bar_h * scroll_off / len(notes)
            ctx.rect(ctx.width - 5, list_top, 3, bar_h, fill=SURFACE)
            ctx.rect(ctx.width - 5, thumb_y, 3, thumb, fill=MUTED, radius=2)

    # ── expanded note ─────────────────────────────────────────────────────────
    if expanded and notes:
        note = notes[selected]
        panel_y = list_top + visible * row_h + 8
        panel_h = ctx.height - panel_y - 56
        if panel_h > 40:
            ctx.rect(0, panel_y, ctx.width, panel_h, fill=SURFACE, radius=6)
            lines = note["text"].splitlines()
            ty = panel_y + 10
            line_h = 16
            for line in lines:
                if ty + line_h > panel_y + panel_h - 4:
                    ctx.text(16, ty, "…", size=11, color=MUTED)
                    break
                ctx.text(16, ty, line[:int((ctx.width - 32) / 7)], size=12, color=FG, monospace=True)
                ty += line_h

    # ── status bar ────────────────────────────────────────────────────────────
    ctx.rect(0, ctx.height - 40, ctx.width, 40, fill=SURFACE)
    hint = "[Enter] expand  [d] dismiss  [a] archive  [f] feedback  [q] quit"
    ctx.text(16, ctx.height - 27, status_msg or hint, size=11, color=status_color if status_msg else MUTED)
    ctx.text(ctx.width - 120, ctx.height - 27, f"v{APP_VERSION}", size=10, color=MUTED)


@app.on_key
def on_key(key, mods, emit):
    global selected, expanded, notes, status_msg, status_color

    status_msg = ""
    status_color = GREEN

    if key in ("j", "ArrowDown"):
        selected = _clamp(selected + 1)
        expanded = False
    elif key in ("k", "ArrowUp"):
        selected = _clamp(selected - 1)
        expanded = False
    elif key == "Enter":
        expanded = not expanded
    elif key == "d" and notes:
        msg = _dismiss(selected)
        notes = _load_notes()
        selected = _clamp(selected)
        expanded = False
        status_msg = msg
        status_color = YELLOW
    elif key == "a" and notes:
        msg = _archive(selected)
        notes = _load_notes()
        selected = _clamp(selected)
        expanded = False
        status_msg = msg
        status_color = GREEN
    elif key == "f" and notes:
        note = notes[selected]
        emit.submit_feedback(
            f"useful backlog note: {note['preview'][:60]}",
            rating=5,
            category="backlog",
        )
        status_msg = "Feedback recorded!"
        status_color = GREEN
    elif key in ("q", "Escape"):
        emit.run_in_terminal("")  # trigger close via companion terminal


app.run()
