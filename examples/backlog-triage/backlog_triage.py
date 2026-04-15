from __future__ import annotations
"""
backlog_triage.py — Keyboard-driven review of ~/.plexi-alpha/backlog/ notes.

Reference implementation for the Phase 1 SDK components layer (Theme,
ctx.header, ctx.status_bar, ctx.scrollable_list, ctx.empty_state,
ctx.wrap_text, safe_move). All layout is composed from SDK primitives — no
inline palette constants, no hand-rolled scroll clamp, no custom status bar.

Keys:
  j / Down    next note
  k / Up      previous note
  Enter       open full-pane detail view (Enter or Esc to return)
  a           archive — move to backlog/archived/
  t           trash   — move to backlog/trash/
  f           thumbs-up feedback ("useful reminder")
  Esc         close app (host-handled, list view only)
"""

import pathlib
from datetime import datetime

from plexi_sdk import (
    App, load_manifest, safe_move,
    THEME,
    BODY, CAPTION, MONO_BODY,
    PAD, HEADER_H,
)


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
        skip_names = {"archived", "trash"}
        for f in sorted(d.glob("*.md"), key=lambda p: p.stat().st_mtime, reverse=True):
            if f.name in seen or any(part in skip_names for part in f.relative_to(d).parts[:-1]):
                continue
            seen.add(f.name)
            try:
                text = f.read_text(errors="replace").strip()
                lines = text.splitlines()
                first = lines[0] if lines else "(empty)"
                rest = " ".join(l.strip() for l in lines[1:] if l.strip())
                notes.append({
                    "path": f,
                    "name": f.name,
                    "text": text,
                    "preview": first[:180],
                    "rest": rest[:180],
                    "mtime": datetime.fromtimestamp(f.stat().st_mtime).strftime("%b %d %H:%M"),
                })
            except OSError:
                pass
    return notes


# ── state ─────────────────────────────────────────────────────────────────────
notes: list[dict] = _load_notes()
selected: int = 0
expanded: bool = False           # full-pane detail view toggle
detail_scroll: int = 0           # scroll offset within the detail view
status_msg: str = ""
status_color: str | None = None  # None → default muted

manifest = load_manifest(__file__)
APP_VERSION = manifest.get("app", {}).get("version", "0.2.0")

app = App("backlog-triage")


def _clamp(idx: int) -> int:
    if not notes:
        return 0
    return max(0, min(len(notes) - 1, idx))


# ── rendering ─────────────────────────────────────────────────────────────────
def _render_row(ctx, note, i, x, y, w, is_sel):
    row_h = 52.0
    bg = THEME.highlight if is_sel else THEME.bg
    ctx.rect(x + PAD / 2, y, w - PAD, row_h, fill=bg, radius=6)

    dot_color = THEME.accent if is_sel else THEME.muted
    ctx.rect(x + PAD, y + (row_h - 6) / 2, 6, 6, fill=dot_color, radius=3)

    text_x = x + PAD + 20
    preview = note["preview"]
    ctx.text(
        text_x, y + 6,
        preview[:120],
        size=BODY,
        color=THEME.fg if is_sel else THEME.muted,
        bold=is_sel,
    )
    sub = note.get("rest") or "(single line)"
    ctx.text(
        text_x, y + 6 + BODY + 4,
        sub[:140],
        size=CAPTION,
        color=THEME.muted,
    )


@app.on_render
def render(ctx):
    global detail_scroll

    ctx.rect(0, 0, ctx.width, ctx.height, fill=THEME.bg)

    # ── detail view ──────────────────────────────────────────────────────────
    if expanded and notes:
        note = notes[selected]
        ctx.header(f"{note['name']}  ({selected + 1}/{len(notes)})")

        wrapped = ctx.wrap_text(
            note["text"],
            max_width_px=ctx.width - PAD * 2,
            size=MONO_BODY,
            monospace=True,
        )
        detail_scroll = ctx.scrollable_text(
            text_id="detail",
            lines=wrapped,
            scroll_offset=detail_scroll,
            line_height=MONO_BODY + 4,
            size=MONO_BODY,
            monospace=True,
        )

        ctx.status_bar(
            [("j/k", "scroll"), ("Enter/Esc", "back"), ("⌘W", "close")],
        )
        return

    # ── empty state ──────────────────────────────────────────────────────────
    if not notes:
        ctx.header("Backlog Triage  ·  0 notes")
        ctx.empty_state("No backlog notes", "You're clear!")
        ctx.status_bar([("⌘W", "close")])
        return

    # ── list view ────────────────────────────────────────────────────────────
    ctx.header(f"Backlog Triage  ·  {len(notes)} note{'s' if len(notes) != 1 else ''}")

    ctx.scrollable_list(
        list_id="notes",
        items=notes,
        selected=selected,
        row_height=56.0,
        render_row=_render_row,
    )

    ctx.status_bar(
        [
            ("j/k", "navigate"),
            ("Enter", "read"),
            ("a", "archive"),
            ("t", "trash"),
            ("⌘W", "close"),
        ],
        status_msg=status_msg or None,
        status_color=status_color,
    )


# ── input ─────────────────────────────────────────────────────────────────────
@app.on_key
def on_key(key, mods, emit):
    global selected, expanded, detail_scroll, notes, status_msg, status_color

    # Clear transient status on any input.
    status_msg = ""
    status_color = None

    # Detail view input.
    if expanded:
        if key in ("j", "ArrowDown"):
            detail_scroll += 1
        elif key in ("k", "ArrowUp"):
            detail_scroll = max(0, detail_scroll - 1)
        elif key in ("Enter", "Escape"):
            expanded = False
            detail_scroll = 0
        return

    # List view input.
    if key in ("j", "ArrowDown"):
        selected = _clamp(selected + 1)
    elif key in ("k", "ArrowUp"):
        selected = _clamp(selected - 1)
    elif key == "Enter" and notes:
        expanded = True
        detail_scroll = 0
    elif key == "a" and notes:
        note = notes[selected]
        status_msg = safe_move(note["path"], note["path"].parent / "archived")
        status_color = THEME.green
        notes = _load_notes()
        selected = _clamp(selected)
    elif key == "t" and notes:
        note = notes[selected]
        status_msg = safe_move(note["path"], note["path"].parent / "trash")
        status_color = THEME.yellow
        notes = _load_notes()
        selected = _clamp(selected)
    elif key == "f" and notes:
        note = notes[selected]
        emit.submit_feedback(
            f"useful backlog note: {note['preview'][:60]}",
            rating=5,
            category="backlog",
        )
        status_msg = "Feedback recorded"
        status_color = THEME.green


app.run()
