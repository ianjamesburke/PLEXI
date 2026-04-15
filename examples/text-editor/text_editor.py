#!/usr/bin/env python3
"""
text_editor — Plexi external app

A pure-Python text editor that mirrors Plexi's in-process Rust editor's UX
via the JSON draw protocol. Proves cross-language composition through the SDK.

Features:
  - Open file from CLI arg:  python3 text_editor.py /path/to/file.py
  - Find (Cmd+F) / Replace (Cmd+R) with all-match highlighting
  - Line numbers gutter (Cmd+Shift+L)
  - Syntax highlighting via pygments (optional; falls back to plain)
  - Status bar (path, dirty, line:col, totals, lang, transient messages)
  - Goto line (Cmd+;)
  - Save (Cmd+S), Save As (Cmd+Shift+S, ~/ expansion, overwrite prompt)
  - Undo / redo (Cmd+Z / Cmd+Shift+Z) with 200-entry cap and 500ms coalesce
  - Word wrap toggle (Cmd+Shift+W), default on for plain/markdown, off for code
  - File-changed-on-disk detection on refocus
  - Auto-save for unnamed scratch buffers (30s / 100 keystrokes)
"""
from __future__ import annotations

import os
import pathlib
import sys
import time
from typing import Optional

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App, Emitter, RenderContext  # noqa: E402

# ---------------------------------------------------------------------------
# Optional pygments import
# ---------------------------------------------------------------------------

_HAS_PYGMENTS = False
lex = None  # type: ignore[assignment]
get_lexer_by_name = None  # type: ignore[assignment]
guess_lexer_for_filename = None  # type: ignore[assignment]
Token = None  # type: ignore[assignment]
ClassNotFound = Exception  # type: ignore[assignment,misc]
try:
    from pygments import lex  # type: ignore[no-redef]
    from pygments.lexers import get_lexer_by_name, guess_lexer_for_filename  # type: ignore[no-redef]
    from pygments.token import Token  # type: ignore[no-redef]
    from pygments.util import ClassNotFound  # type: ignore[no-redef]

    _HAS_PYGMENTS = True
except Exception:
    _HAS_PYGMENTS = False


# ---------------------------------------------------------------------------
# Theme — Catppuccin-inspired, matches Plexi's built-in palette
# ---------------------------------------------------------------------------

C = {
    "bg":          "#1e1e2e",
    "bg_sidebar":  "#181825",
    "bg_active":   "#313144",
    "bg_hover":    "#2a2a3c",
    "text":        "#cdd6f4",
    "text_dim":    "#6c7086",
    "text_section":"#585b70",
    "accent":      "#89b4fa",
    "border":      "#2a2a3c",
    "match":       "#3d3d55",
    "match_cur":   "#f9e2af",
    "dirty":       "#f38ba8",
    "ok":          "#a6e3a1",
    "sel":         "#45475a",
    # Syntax token colors (low-contrast)
    "syn_kw":      "#cba6f7",
    "syn_str":     "#a6e3a1",
    "syn_num":     "#fab387",
    "syn_com":     "#6c7086",
    "syn_fn":      "#89b4fa",
    "syn_type":    "#f9e2af",
    "syn_op":      "#94e2d5",
}


# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

CHAR_W = 7.4             # approximate monospace character width at size 13
LINE_H = 18.0            # editor line height
FONT_SIZE = 13.0
STATUS_H = 20.0
BAR_H = 24.0             # find/replace/goto/save-as bar height
GUTTER_PAD = 12.0

UNDO_CAP = 200
UNDO_COALESCE_MS = 500
AUTOSAVE_INTERVAL_S = 30.0
AUTOSAVE_KEY_THRESHOLD = 100
STATUS_MSG_TTL = 3.0
MAX_HIGHLIGHT_BYTES = 1_000_000


# ---------------------------------------------------------------------------
# Language detection
# ---------------------------------------------------------------------------

EXT_TO_LANG = {
    "py":   "python",
    "rs":   "rust",
    "js":   "javascript",
    "ts":   "typescript",
    "jsx":  "javascript",
    "tsx":  "typescript",
    "json": "json",
    "toml": "toml",
    "yaml": "yaml",
    "yml":  "yaml",
    "html": "html",
    "css":  "css",
    "sh":   "bash",
    "md":   "markdown",
    "txt":  "plain",
}

DEFAULT_WRAP_LANGS = {"plain", "markdown"}


def detect_language(path: Optional[str]) -> str:
    if not path:
        return "plain"
    ext = pathlib.Path(path).suffix.lstrip(".").lower()
    return EXT_TO_LANG.get(ext, "plain")


# ---------------------------------------------------------------------------
# Undo stack
# ---------------------------------------------------------------------------


class UndoStack:
    """Coalescing undo stack. Each entry is a (content, cursor) snapshot."""

    def __init__(self, cap: int = UNDO_CAP):
        self._cap = cap
        self._undo: list[tuple[str, int]] = []
        self._redo: list[tuple[str, int]] = []
        self._last_push_ms: float = 0.0

    def clear(self):
        self._undo.clear()
        self._redo.clear()
        self._last_push_ms = 0.0

    def push(self, content: str, cursor: int, force: bool = False):
        now = time.monotonic() * 1000.0
        # Coalesce rapid consecutive pushes unless explicitly forced or crossed
        # a word boundary (caller passes force=True in that case).
        if (
            not force
            and self._undo
            and (now - self._last_push_ms) < UNDO_COALESCE_MS
        ):
            # Replace the most recent snapshot instead of pushing a new one.
            self._undo[-1] = (content, cursor)
        else:
            self._undo.append((content, cursor))
            if len(self._undo) > self._cap:
                self._undo.pop(0)
        self._last_push_ms = now
        self._redo.clear()

    def undo(self, current: str, cursor: int) -> Optional[tuple[str, int]]:
        if not self._undo:
            return None
        prev = self._undo.pop()
        self._redo.append((current, cursor))
        self._last_push_ms = 0.0
        return prev

    def redo(self, current: str, cursor: int) -> Optional[tuple[str, int]]:
        if not self._redo:
            return None
        nxt = self._redo.pop()
        self._undo.append((current, cursor))
        self._last_push_ms = 0.0
        return nxt


# ---------------------------------------------------------------------------
# Editor state
# ---------------------------------------------------------------------------


class EditorState:
    def __init__(self):
        self.content: str = ""
        self.saved_content: str = ""
        self.cursor: int = 0
        self.file_path: Optional[str] = None
        self.dirty: bool = False
        self.edit_mode: bool = True
        self.lang: str = "plain"
        self.word_wrap: bool = True
        self.word_wrap_user_set: bool = False
        self.show_line_numbers: bool = True

        # Scroll
        self.scroll_line: int = 0

        # Undo
        self.undo = UndoStack()

        # Find
        self.find_open: bool = False
        self.replace_open: bool = False
        self.find_query: str = ""
        self.replace_with: str = ""
        self.find_focus: str = "query"  # "query" | "replace"
        self.find_matches: list[tuple[int, int]] = []  # (start, end) byte offsets
        self.current_match: int = 0

        # Goto line
        self.goto_open: bool = False
        self.goto_input: str = ""

        # Save As
        self.save_as_open: bool = False
        self.save_as_input: str = ""
        self.save_as_overwrite: Optional[str] = None

        # Disk-changed prompt
        self.disk_mtime: Optional[float] = None
        self.disk_conflict_prompt: bool = False
        self.was_focused: bool = True

        # Status message
        self.status_msg: Optional[str] = None
        self.status_msg_at: float = 0.0

        # Autosave tracking
        self.autosave_last: float = time.monotonic()
        self.autosave_keys_since: int = 0

        # Cached line:col (1-indexed)
        self.cursor_line: int = 1
        self.cursor_col: int = 1


state = EditorState()
app = App(app_id="text-editor")


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _set_status(msg: str):
    state.status_msg = msg
    state.status_msg_at = time.monotonic()


def _update_line_col():
    before = state.content[: state.cursor]
    state.cursor_line = before.count("\n") + 1
    last_nl = before.rfind("\n")
    state.cursor_col = (state.cursor - last_nl) if last_nl >= 0 else (state.cursor + 1)


def _lines() -> list[str]:
    return state.content.split("\n")


def _line_start(line_idx: int) -> int:
    """Return cursor offset at start of line `line_idx` (0-indexed)."""
    lines = _lines()
    if line_idx <= 0:
        return 0
    if line_idx >= len(lines):
        return len(state.content)
    pos = 0
    for i in range(line_idx):
        pos += len(lines[i]) + 1  # +1 for the newline
    return pos


def _cursor_line_idx() -> int:
    return state.content[: state.cursor].count("\n")


def _cursor_col_idx() -> int:
    before = state.content[: state.cursor]
    last_nl = before.rfind("\n")
    return state.cursor - last_nl - 1 if last_nl >= 0 else state.cursor


def _push_undo(force: bool = False):
    state.undo.push(state.content, state.cursor, force=force)


def _mark_dirty():
    state.dirty = state.content != state.saved_content


def _insert(s: str, coalesce: bool = True):
    _push_undo(force=not coalesce)
    state.content = state.content[: state.cursor] + s + state.content[state.cursor :]
    state.cursor += len(s)
    _mark_dirty()
    _recompute_matches()
    state.autosave_keys_since += 1


def _delete_back():
    if state.cursor == 0:
        return
    _push_undo(force=True)
    state.content = state.content[: state.cursor - 1] + state.content[state.cursor :]
    state.cursor -= 1
    _mark_dirty()
    _recompute_matches()
    state.autosave_keys_since += 1


def _delete_forward():
    if state.cursor >= len(state.content):
        return
    _push_undo(force=True)
    state.content = state.content[: state.cursor] + state.content[state.cursor + 1 :]
    _mark_dirty()
    _recompute_matches()
    state.autosave_keys_since += 1


def _expand(path: str) -> str:
    return os.path.expanduser(path)


def _config_dir() -> pathlib.Path:
    """Mirror Plexi's build-specific config dir resolution (alpha/beta/stable)."""
    argv0 = os.environ.get("PLEXI_BINARY_NAME", "")
    if "alpha" in argv0.lower():
        name = ".plexi-alpha"
    elif "beta" in argv0.lower():
        name = ".plexi-beta"
    else:
        name = ".plexi-alpha"  # default to alpha — this app lives under alpha-first dev
    return pathlib.Path.home() / name


def _scratch_dir() -> pathlib.Path:
    return _config_dir() / "scratch"


def _read_mtime(path: str) -> Optional[float]:
    try:
        return os.path.getmtime(path)
    except OSError:
        return None


def load_file(path: str):
    path = _expand(path)
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            content = f.read()
    except OSError as e:
        _set_status(f"Open failed: {e}")
        return
    state.content = content
    state.saved_content = content
    state.file_path = path
    state.cursor = 0
    state.dirty = False
    state.lang = detect_language(path)
    state.edit_mode = False
    if not state.word_wrap_user_set:
        state.word_wrap = state.lang in DEFAULT_WRAP_LANGS
    state.disk_mtime = _read_mtime(path)
    state.undo.clear()
    state.find_matches = []
    state.current_match = 0
    _set_status(f"Opened {os.path.basename(path)}")


def save_file():
    """Save to existing path. If no path, save to a scratch file."""
    path = state.file_path
    if not path:
        scratch = _scratch_dir()
        try:
            scratch.mkdir(parents=True, exist_ok=True)
        except OSError as e:
            _set_status(f"Save failed: {e}")
            return
        ts = int(time.time())
        path = str(scratch / f"scratch-{ts}.txt")
        state.file_path = path
    try:
        with open(path, "w", encoding="utf-8") as f:
            f.write(state.content)
    except OSError as e:
        _set_status(f"Save failed: {e}")
        return
    state.saved_content = state.content
    state.dirty = False
    state.disk_mtime = _read_mtime(path)
    state.lang = detect_language(path)
    _set_status(f"Saved {os.path.basename(path)}")


def save_as(target: str):
    target = _expand(target)
    if not target:
        _set_status("Save As cancelled: empty path")
        return
    if os.path.exists(target) and state.save_as_overwrite != target:
        state.save_as_overwrite = target
        _set_status(f"Overwrite {os.path.basename(target)}? Enter=yes, Esc=no")
        return
    try:
        parent = os.path.dirname(target)
        if parent:
            os.makedirs(parent, exist_ok=True)
        with open(target, "w", encoding="utf-8") as f:
            f.write(state.content)
    except OSError as e:
        _set_status(f"Save failed: {e}")
        return
    state.file_path = target
    state.saved_content = state.content
    state.dirty = False
    state.lang = detect_language(target)
    state.disk_mtime = _read_mtime(target)
    state.save_as_open = False
    state.save_as_input = ""
    state.save_as_overwrite = None
    _set_status(f"Saved {os.path.basename(target)}")


def reload_from_disk():
    if not state.file_path:
        return
    try:
        with open(state.file_path, "r", encoding="utf-8", errors="replace") as f:
            state.content = f.read()
    except OSError as e:
        _set_status(f"Reload failed: {e}")
        return
    state.saved_content = state.content
    state.dirty = False
    state.disk_mtime = _read_mtime(state.file_path)
    state.undo.clear()
    _set_status("Reloaded from disk")


def check_disk_conflict():
    """Compare disk mtime to last-known; prompt or silent-reload."""
    if not state.file_path or state.disk_mtime is None:
        return
    mtime = _read_mtime(state.file_path)
    if mtime is None or mtime == state.disk_mtime:
        return
    if not state.dirty:
        reload_from_disk()
    else:
        state.disk_conflict_prompt = True


def maybe_autosave():
    """Scratch-only periodic autosave."""
    if state.file_path:
        return  # named files never auto-save
    if not state.content:
        return
    now = time.monotonic()
    if (
        now - state.autosave_last < AUTOSAVE_INTERVAL_S
        and state.autosave_keys_since < AUTOSAVE_KEY_THRESHOLD
    ):
        return
    scratch = _scratch_dir()
    try:
        scratch.mkdir(parents=True, exist_ok=True)
    except OSError:
        return
    ts = int(time.time())
    target = scratch / f"autosave-{ts}.txt"
    try:
        with open(target, "w", encoding="utf-8") as f:
            f.write(state.content)
        state.autosave_last = now
        state.autosave_keys_since = 0
    except OSError:
        pass


# ---------------------------------------------------------------------------
# Find / replace
# ---------------------------------------------------------------------------


def _recompute_matches():
    """Recalculate all find matches and pick the 'current' match below cursor."""
    if not state.find_open or not state.find_query:
        state.find_matches = []
        state.current_match = 0
        return
    q = state.find_query
    matches: list[tuple[int, int]] = []
    start = 0
    while True:
        idx = state.content.find(q, start)
        if idx < 0:
            break
        matches.append((idx, idx + len(q)))
        start = idx + max(1, len(q))
    state.find_matches = matches
    state.current_match = 0
    for i, (s, _e) in enumerate(matches):
        if s >= state.cursor:
            state.current_match = i
            break


def find_next(delta: int):
    if not state.find_matches:
        return
    state.current_match = (state.current_match + delta) % len(state.find_matches)
    s, _e = state.find_matches[state.current_match]
    state.cursor = s
    _ensure_cursor_visible()


def replace_current():
    if not state.find_matches or not state.find_open:
        return
    s, e = state.find_matches[state.current_match]
    _push_undo(force=True)
    state.content = state.content[:s] + state.replace_with + state.content[e:]
    state.cursor = s + len(state.replace_with)
    _mark_dirty()
    _recompute_matches()
    _set_status("Replaced 1")


def replace_all():
    if not state.find_matches or not state.find_open:
        return
    _push_undo(force=True)
    n = state.content.count(state.find_query)
    state.content = state.content.replace(state.find_query, state.replace_with)
    # Best-effort cursor adjustment: clamp to content length
    state.cursor = min(state.cursor, len(state.content))
    _mark_dirty()
    _recompute_matches()
    _set_status(f"Replaced {n} match{'es' if n != 1 else ''}")


# ---------------------------------------------------------------------------
# Cursor movement + scroll
# ---------------------------------------------------------------------------


def _visible_line_count(height: float) -> int:
    content_h = height - STATUS_H
    if state.find_open:
        content_h -= BAR_H
        if state.replace_open:
            content_h -= BAR_H
    if state.goto_open:
        content_h -= BAR_H
    if state.save_as_open:
        content_h -= BAR_H
    return max(1, int(content_h // LINE_H))


def _ensure_cursor_visible():
    cl = _cursor_line_idx()
    visible = _visible_line_count(state.__dict__.get("_height", 600))
    if cl < state.scroll_line:
        state.scroll_line = cl
    elif cl >= state.scroll_line + visible:
        state.scroll_line = cl - visible + 1


def move_cursor_line(delta: int):
    cur_line = _cursor_line_idx()
    cur_col = _cursor_col_idx()
    target = max(0, min(len(_lines()) - 1, cur_line + delta))
    line_text = _lines()[target]
    new_col = min(cur_col, len(line_text))
    state.cursor = _line_start(target) + new_col
    _ensure_cursor_visible()


def move_cursor_char(delta: int):
    state.cursor = max(0, min(len(state.content), state.cursor + delta))
    _ensure_cursor_visible()


def move_cursor_line_edge(to_start: bool):
    line = _cursor_line_idx()
    if to_start:
        state.cursor = _line_start(line)
    else:
        lines = _lines()
        state.cursor = _line_start(line) + len(lines[line])
    _ensure_cursor_visible()


def goto_line(n: int):
    n = max(1, n)
    lines = _lines()
    n = min(n, len(lines))
    state.cursor = _line_start(n - 1)
    _ensure_cursor_visible()


# ---------------------------------------------------------------------------
# Syntax highlighting
# ---------------------------------------------------------------------------


def _pygments_color(tok) -> str:
    """Map a pygments token type to one of our palette tokens."""
    name = str(tok)
    if "Comment" in name:
        return C["syn_com"]
    if "String" in name:
        return C["syn_str"]
    if "Number" in name:
        return C["syn_num"]
    if "Keyword" in name:
        return C["syn_kw"]
    if "Name.Function" in name or "Name.Class" in name:
        return C["syn_fn"]
    if "Name.Builtin" in name or "Name.Tag" in name:
        return C["syn_type"]
    if "Operator" in name:
        return C["syn_op"]
    return C["text"]


def highlight_line(line: str, lang: str) -> list[tuple[str, str]]:
    """Return a list of (text_span, color) tuples for a single line."""
    if not _HAS_PYGMENTS or lang == "plain" or not line:
        return [(line, C["text"])]
    if len(state.content) > MAX_HIGHLIGHT_BYTES:
        return [(line, C["text"])]
    assert get_lexer_by_name is not None and guess_lexer_for_filename is not None and lex is not None
    try:
        try:
            lexer = get_lexer_by_name(lang)
        except ClassNotFound:
            try:
                lexer = guess_lexer_for_filename(state.file_path or "a.txt", line)
            except ClassNotFound:
                return [(line, C["text"])]
        spans: list[tuple[str, str]] = []
        for tok, text in lex(line, lexer):  # type: ignore[arg-type]
            if not text:
                continue
            color = _pygments_color(tok)
            # Strip trailing newline pygments may add
            if text.endswith("\n"):
                text = text[:-1]
            if text:
                spans.append((text, color))
        if not spans:
            return [(line, C["text"])]
        return spans
    except Exception:
        return [(line, C["text"])]


# ---------------------------------------------------------------------------
# Rendering
# ---------------------------------------------------------------------------


@app.on_render
def render(ctx: RenderContext):
    state.__dict__["_height"] = ctx.height

    # On focus regain (tracked cheaply — we consider every render frame as focus)
    # check for disk conflict.
    if not state.was_focused:
        state.was_focused = True
        check_disk_conflict()

    _update_line_col()
    maybe_autosave()

    # Expire transient status
    if state.status_msg and (time.monotonic() - state.status_msg_at) > STATUS_MSG_TTL:
        state.status_msg = None

    w, h = ctx.width, ctx.height

    # Background
    ctx.rect(0, 0, w, h, fill=C["bg"])

    # Top bars (find / replace, goto, save-as)
    y = 0.0
    if state.find_open:
        _render_find_bar(ctx, y)
        y += BAR_H
        if state.replace_open:
            _render_replace_bar(ctx, y)
            y += BAR_H
    if state.goto_open:
        _render_goto_bar(ctx, y)
        y += BAR_H
    if state.save_as_open:
        _render_save_as_bar(ctx, y)
        y += BAR_H

    # Editor body
    editor_top = y
    editor_bottom = h - STATUS_H
    _render_editor(ctx, editor_top, editor_bottom)

    # Disk conflict overlay
    if state.disk_conflict_prompt:
        _render_disk_conflict(ctx)

    # Status bar
    _render_status_bar(ctx, h - STATUS_H, w)


def _render_find_bar(ctx: RenderContext, y: float):
    ctx.rect(0, y, ctx.width, BAR_H, fill=C["bg_sidebar"])
    ctx.text(8, y + 5, "Find:", size=11, color=C["text_dim"], monospace=True)
    q = state.find_query + ("|" if state.find_focus == "query" else "")
    ctx.text(50, y + 5, q, size=11, color=C["text"], monospace=True)
    count = len(state.find_matches)
    if count:
        info = f"{state.current_match + 1}/{count}"
    else:
        info = "0/0"
    ctx.text(ctx.width - 100, y + 5, info, size=11, color=C["text_dim"], monospace=True)
    ctx.text(ctx.width - 50, y + 5, "Esc", size=11, color=C["text_dim"], monospace=True)
    ctx.rect(0, y + BAR_H - 1, ctx.width, 1, fill=C["border"])


def _render_replace_bar(ctx: RenderContext, y: float):
    ctx.rect(0, y, ctx.width, BAR_H, fill=C["bg_sidebar"])
    ctx.text(8, y + 5, "Repl:", size=11, color=C["text_dim"], monospace=True)
    r = state.replace_with + ("|" if state.find_focus == "replace" else "")
    ctx.text(50, y + 5, r, size=11, color=C["text"], monospace=True)
    hint = "Enter=one Shift+Enter=all"
    ctx.text(ctx.width - len(hint) * 7 - 8, y + 5, hint,
             size=11, color=C["text_dim"], monospace=True)
    ctx.rect(0, y + BAR_H - 1, ctx.width, 1, fill=C["border"])


def _render_goto_bar(ctx: RenderContext, y: float):
    ctx.rect(0, y, ctx.width, BAR_H, fill=C["bg_sidebar"])
    ctx.text(8, y + 5, "Go to line:", size=11, color=C["text_dim"], monospace=True)
    ctx.text(95, y + 5, state.goto_input + "|",
             size=11, color=C["text"], monospace=True)
    ctx.rect(0, y + BAR_H - 1, ctx.width, 1, fill=C["border"])


def _render_save_as_bar(ctx: RenderContext, y: float):
    ctx.rect(0, y, ctx.width, BAR_H, fill=C["bg_sidebar"])
    if state.save_as_overwrite:
        prompt = f"Overwrite {os.path.basename(state.save_as_overwrite)}? Enter=yes Esc=no"
        ctx.text(8, y + 5, prompt, size=11, color=C["dirty"], monospace=True)
    else:
        ctx.text(8, y + 5, "Save As:", size=11, color=C["text_dim"], monospace=True)
        ctx.text(75, y + 5, state.save_as_input + "|",
                 size=11, color=C["text"], monospace=True)
    ctx.rect(0, y + BAR_H - 1, ctx.width, 1, fill=C["border"])


def _render_editor(ctx: RenderContext, top: float, bottom: float):
    lines = _lines()
    total = len(lines)

    # Gutter width
    gutter_w = 0.0
    if state.show_line_numbers:
        digits = max(2, len(str(total)))
        gutter_w = digits * CHAR_W + GUTTER_PAD

    # Body background
    ctx.rect(gutter_w, top, ctx.width - gutter_w, bottom - top, fill=C["bg"])

    # Gutter background
    if gutter_w > 0:
        ctx.rect(0, top, gutter_w, bottom - top, fill=C["bg_sidebar"])

    # Visible slice
    visible = max(1, int((bottom - top) // LINE_H))
    state.scroll_line = max(0, min(state.scroll_line, max(0, total - 1)))
    first = state.scroll_line
    last = min(total, first + visible)

    # Highlight current match rectangles (global — draw all, with 'current' emphasized)
    match_set: dict[int, list[tuple[int, int, bool]]] = {}
    for i, (s, e) in enumerate(state.find_matches):
        line_before = state.content[:s].count("\n")
        line_start_off = _line_start(line_before)
        col_s = s - line_start_off
        col_e = e - line_start_off
        match_set.setdefault(line_before, []).append(
            (col_s, col_e, i == state.current_match)
        )

    cursor_line = _cursor_line_idx()
    cursor_col = _cursor_col_idx()

    for vi, line_idx in enumerate(range(first, last)):
        y = top + vi * LINE_H
        line_text = lines[line_idx]

        # Line-number gutter
        if gutter_w > 0:
            num = str(line_idx + 1)
            num_color = C["text_dim"] if line_idx != cursor_line else C["text"]
            ctx.text(
                gutter_w - len(num) * CHAR_W - 6, y + 3, num,
                size=FONT_SIZE, color=num_color, monospace=True,
            )

        # Current-line highlight
        if line_idx == cursor_line:
            ctx.rect(gutter_w, y, ctx.width - gutter_w, LINE_H,
                     fill=C["bg_hover"])

        # Match highlights on this line
        for col_s, col_e, is_cur in match_set.get(line_idx, []):
            mx = gutter_w + col_s * CHAR_W
            mw = max(1.0, (col_e - col_s) * CHAR_W)
            fill = C["match_cur"] if is_cur else C["match"]
            ctx.rect(mx, y, mw, LINE_H, fill=fill)

        # Highlighted text spans
        spans = highlight_line(line_text, state.lang)
        x = gutter_w + 4
        for span_text, color in spans:
            if x > ctx.width:
                break
            ctx.text(x, y + 3, span_text, size=FONT_SIZE, color=color, monospace=True)
            x += len(span_text) * CHAR_W

        # Cursor
        if line_idx == cursor_line and state.edit_mode:
            cx = gutter_w + 4 + cursor_col * CHAR_W
            ctx.rect(cx, y + 2, 1.5, LINE_H - 4, fill=C["accent"])


def _render_disk_conflict(ctx: RenderContext):
    w, h = ctx.width, ctx.height
    panel_w = 360.0
    panel_h = 90.0
    px = (w - panel_w) / 2
    py = (h - panel_h) / 2
    ctx.rect(px, py, panel_w, panel_h, fill=C["bg_active"], radius=6.0)
    ctx.text(px + 12, py + 12, "File changed on disk",
             size=13, color=C["accent"], bold=True)
    ctx.text(px + 12, py + 34, "R = Reload   K = Keep mine",
             size=11, color=C["text_dim"], monospace=True)


def _render_status_bar(ctx: RenderContext, y: float, w: float):
    ctx.rect(0, y, w, STATUS_H, fill=C["bg_sidebar"])
    ctx.rect(0, y, w, 1, fill=C["border"])

    # Left: path + dirty
    left_path = state.file_path or "[scratch]"
    if len(left_path) > 40:
        left_path = "..." + left_path[-37:]
    dot = " \u25cf" if state.dirty else ""
    left = f"{left_path}{dot}"
    ctx.text(8, y + 4, left, size=11,
             color=C["dirty"] if state.dirty else C["text"], monospace=True)

    # Right: line:col total_lines bytes lang
    total_lines = state.content.count("\n") + 1
    right = (
        f"{state.cursor_line}:{state.cursor_col}  "
        f"L{total_lines}  "
        f"{len(state.content)}b  "
        f"{state.lang}"
    )
    ctx.text(w - len(right) * 7 - 8, y + 4, right,
             size=11, color=C["text_dim"], monospace=True)

    # Center: transient status message
    if state.status_msg:
        msg = state.status_msg
        ctx.text(w / 2 - len(msg) * 3.5, y + 4, msg,
                 size=11, color=C["ok"], monospace=True)


# ---------------------------------------------------------------------------
# Input routing
# ---------------------------------------------------------------------------


def _close_all_bars():
    state.find_open = False
    state.replace_open = False
    state.goto_open = False
    state.save_as_open = False
    state.save_as_overwrite = None
    state.find_matches = []


@app.on_key
def on_key(key: str, mods: dict, emit: Emitter):
    cmd = bool(mods.get("command"))
    shift = bool(mods.get("shift"))
    alt = bool(mods.get("alt"))

    # -----------------------------------------------------------------------
    # Command shortcuts — must be checked before the input bars swallow keys
    # -----------------------------------------------------------------------
    if cmd and not alt:
        if key in ("f", "F"):
            state.find_open = not state.find_open
            state.find_focus = "query"
            if state.find_open:
                _recompute_matches()
            else:
                state.replace_open = False
                state.find_matches = []
            return
        if key in ("r", "R") and not shift:
            state.find_open = True
            state.replace_open = True
            state.find_focus = "replace"
            _recompute_matches()
            return
        if key in ("s", "S") and not shift:
            save_file()
            return
        if key in ("s", "S") and shift:
            state.save_as_open = True
            state.save_as_input = state.file_path or ""
            state.save_as_overwrite = None
            return
        if key in ("z", "Z") and not shift:
            res = state.undo.undo(state.content, state.cursor)
            if res:
                state.content, state.cursor = res
                _mark_dirty()
                _recompute_matches()
            return
        if key in ("z", "Z") and shift:
            res = state.undo.redo(state.content, state.cursor)
            if res:
                state.content, state.cursor = res
                _mark_dirty()
                _recompute_matches()
            return
        if key in ("l", "L") and shift:
            state.show_line_numbers = not state.show_line_numbers
            return
        if key in ("w", "W") and shift:
            state.word_wrap = not state.word_wrap
            state.word_wrap_user_set = True
            return
        if key == ";":
            state.goto_open = not state.goto_open
            state.goto_input = ""
            return

    # -----------------------------------------------------------------------
    # Disk-conflict prompt
    # -----------------------------------------------------------------------
    if state.disk_conflict_prompt:
        if key in ("r", "R"):
            state.disk_conflict_prompt = False
            reload_from_disk()
            return
        if key in ("k", "K", "Escape"):
            state.disk_conflict_prompt = False
            state.disk_mtime = _read_mtime(state.file_path or "")
            return
        return

    # -----------------------------------------------------------------------
    # Goto-line bar
    # -----------------------------------------------------------------------
    if state.goto_open:
        if key == "Escape":
            state.goto_open = False
            state.goto_input = ""
            return
        if key == "Enter":
            try:
                goto_line(int(state.goto_input))
                _set_status(f"Jumped to line {state.goto_input}")
            except ValueError:
                _set_status("Invalid line")
            state.goto_open = False
            state.goto_input = ""
            return
        if key == "Backspace":
            state.goto_input = state.goto_input[:-1]
            return
        if len(key) == 1 and key.isdigit():
            state.goto_input += key
        return

    # -----------------------------------------------------------------------
    # Save As bar
    # -----------------------------------------------------------------------
    if state.save_as_open:
        if key == "Escape":
            state.save_as_open = False
            state.save_as_input = ""
            state.save_as_overwrite = None
            return
        if key == "Enter":
            if state.save_as_overwrite:
                save_as(state.save_as_overwrite)
            else:
                save_as(state.save_as_input)
            return
        if key == "Backspace":
            state.save_as_input = state.save_as_input[:-1]
            state.save_as_overwrite = None
            return
        if len(key) == 1 and key.isprintable():
            state.save_as_input += key
            state.save_as_overwrite = None
        return

    # -----------------------------------------------------------------------
    # Find / Replace bar
    # -----------------------------------------------------------------------
    if state.find_open:
        if key == "Escape":
            _close_all_bars()
            return
        if key == "Tab":
            if state.replace_open:
                state.find_focus = "replace" if state.find_focus == "query" else "query"
            return
        if key == "Enter":
            if state.find_focus == "replace":
                if shift:
                    replace_all()
                else:
                    replace_current()
            else:
                find_next(-1 if shift else 1)
            return
        if key == "Backspace":
            if state.find_focus == "query":
                state.find_query = state.find_query[:-1]
                _recompute_matches()
            else:
                state.replace_with = state.replace_with[:-1]
            return
        if len(key) == 1 and key.isprintable():
            if state.find_focus == "query":
                state.find_query += key
                _recompute_matches()
            else:
                state.replace_with += key
            return
        return

    # -----------------------------------------------------------------------
    # Editor body
    # -----------------------------------------------------------------------
    if key == "ArrowLeft":
        move_cursor_char(-1); return
    if key == "ArrowRight":
        move_cursor_char(1); return
    if key == "ArrowUp":
        move_cursor_line(-1); return
    if key == "ArrowDown":
        move_cursor_line(1); return
    if key == "Home":
        move_cursor_line_edge(True); return
    if key == "End":
        move_cursor_line_edge(False); return
    if key == "PageUp":
        move_cursor_line(-_visible_line_count(state.__dict__.get("_height", 600)))
        return
    if key == "PageDown":
        move_cursor_line(_visible_line_count(state.__dict__.get("_height", 600)))
        return
    if key == "Backspace":
        _delete_back(); return
    if key == "Delete":
        _delete_forward(); return
    if key == "Enter":
        _insert("\n", coalesce=False); return
    if key == "Tab":
        _insert("    "); return
    if len(key) == 1 and key.isprintable():
        # Break coalescing on whitespace or word boundary
        coalesce = key not in (" ", "\t")
        _insert(key, coalesce=coalesce)
        return


@app.on_command
def on_command(text: str, emit: Emitter):
    """Support palette commands: 'open <path>', 'save', 'save-as <path>'."""
    text = text.strip()
    if not text:
        return
    parts = text.split(maxsplit=1)
    cmd = parts[0].lower()
    arg = parts[1] if len(parts) > 1 else ""
    if cmd == "open" and arg:
        load_file(arg)
    elif cmd == "save":
        save_file()
    elif cmd == "save-as" and arg:
        save_as(arg)


@app.on_resize
def on_resize(width: float, height: float):
    _ensure_cursor_visible()


@app.on_click
def on_click(x: float, y: float, button: str, emit: Emitter):
    # Basic click-to-position: find the line and column under the click.
    lines = _lines()
    total = len(lines)
    # Compute body top the same way the renderer does.
    body_top = 0.0
    if state.find_open:
        body_top += BAR_H
        if state.replace_open:
            body_top += BAR_H
    if state.goto_open:
        body_top += BAR_H
    if state.save_as_open:
        body_top += BAR_H

    gutter_w = 0.0
    if state.show_line_numbers:
        digits = max(2, len(str(total)))
        gutter_w = digits * CHAR_W + GUTTER_PAD

    if y < body_top or y > state.__dict__.get("_height", 600) - STATUS_H:
        return
    line_idx = state.scroll_line + int((y - body_top) // LINE_H)
    line_idx = max(0, min(total - 1, line_idx))
    col_x = max(0.0, x - gutter_w - 4)
    col = int(round(col_x / CHAR_W))
    col = max(0, min(col, len(lines[line_idx])))
    state.cursor = _line_start(line_idx) + col


# ---------------------------------------------------------------------------
# Initial file load from argv
# ---------------------------------------------------------------------------

if len(sys.argv) > 1:
    load_file(sys.argv[1])


if __name__ == "__main__":
    app.run()
