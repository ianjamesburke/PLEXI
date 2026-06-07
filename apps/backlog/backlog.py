#!/usr/bin/env python3
"""Backlog viewer — browse, preview, open, archive, delete, and add backlog items.

hjkl navigation. e edits inline. Enter opens in default app. a archives.
d deletes (confirm). n creates a new item via inline TextEdit. r refreshes.
/ searches.
Shows items from two sources merged by mtime:
  - workspace backlog: <workspace>/.plexi/backlog/
  - channel backlog:   $PLEXI_CONFIG_DIR/backlog/ (quick notes from host ⌘0)
Items from the channel backlog are prefixed with [ch] in the list.

Pass -g/--global to show only channel-level items: plexi open backlog -g

L1 Reference Implementation
============================
Patterns demonstrated:
  - Column layout with AppBar + growable body + FooterKeys
  - SelectList for navigable item lists
  - TextEdit for inline item creation and editing (component_tree overlay)
  - Two-pane split via custom Component (escape hatch for HSplit)
  - Filesystem persistence (read/write/delete backlog .md files)
  - CLI argument parsing via Arg descriptor
  - Multi-source data merging (workspace + channel backlog dirs)
  - Search/filter mode with live refiltering
  - Confirm-delete overlay pattern
  - on_component_event for TextEdit change/submit handling
  - ctx.theme colors throughout (no hardcoded colors)
"""

import os
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '../../sdk/python'))
from plexi_sdk import App, Arg, TextEdit
from plexi_sdk.ui import (
    AppBar, Column, Component, Footer, FooterKeys,
    SelectList,
    SPACE_MD, TEXT_HINT, TEXT_BODY, TEXT_CAPTION,
)


def _detect_channel_backlog_dir() -> "Path | None":
    """Return the channel-level backlog dir, or None if none exists."""
    env = os.environ.get("PLEXI_CONFIG_DIR")
    if env:
        p = Path(env) / "backlog"
        return p if p.is_dir() else None
    candidates = [
        Path.home() / d / "backlog"
        for d in (".plexi-alpha", ".plexi-beta", ".plexi")
    ]
    existing: list = []
    for p in candidates:
        try:
            if p.is_dir():
                existing.append((p.stat().st_mtime, p))
        except OSError:
            pass
    return max(existing)[1] if existing else None


LIST_FRAC = 0.38   # fraction of width for the item list


class _TwoPaneBody(Component):
    """Raw-draw component for the split list+preview body.

    The SDK has no HSplit container, so the two-pane layout is implemented
    here as a Component that uses ctx primitives directly. This lets the
    Column lay it out correctly (grow=True fills the slack between AppBar
    and FooterKeys) while keeping all draw logic encapsulated.
    """

    def __init__(self, app: "BacklogApp") -> None:
        self._app = app

    def is_grow(self) -> bool:
        return True

    def measure(self, _avail_w: float) -> float:
        return 0.0  # grows to fill

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        app = self._app
        list_w = w * LIST_FRAC

        if app.global_only:
            has_any_dir = app.channel_dir is not None and app.channel_dir.is_dir()
        else:
            has_any_dir = app.backlog_dir.is_dir() or (
                app.channel_dir is not None and app.channel_dir.is_dir()
            )

        if not has_any_dir:
            app.emit.info("backlog: no dirs found — showing empty-state guide")
            ctx.text(x + 12, y + 12, "No notes yet.", size=TEXT_BODY, color=ctx.theme.fg)
            if app.global_only:
                ctx.text(x + 12, y + 34,
                         "No channel-level backlog found. Press ⌘0 to create a Quick Note.",
                         size=TEXT_HINT, color=ctx.theme.muted, max_width=w - 24)
            else:
                ctx.text(x + 12, y + 34,
                         "Press ⌘0 to open Quick Note, then press Enter twice to send to your backlog.",
                         size=TEXT_HINT, color=ctx.theme.muted, max_width=w - 24)
            return

        # Vertical divider between list and preview
        ctx.rect(x + list_w, y, 1.0, h, ctx.theme.highlight)

        # ── Item list ────────────────────────────────────────────────────────────
        if not app.filtered:
            msg = "No results" if app.search_query else "Backlog is empty"
            ctx.text(x + 12, y + 12, msg, size=TEXT_CAPTION, color=ctx.theme.muted)
        else:
            app._item_list.render(ctx, x, y, list_w, h)

        # ── Preview ──────────────────────────────────────────────────────────────
        px = x + list_w + 14
        pw = w - (list_w + 14) - 10
        if app.preview_path and app.preview_text is not None:
            ctx.text(px, y - SPACE_MD, app.preview_path.name,
                     size=TEXT_HINT, color=ctx.theme.accent, bold=True, max_width=pw)
            lines = app.preview_text.splitlines()
            for li, line in enumerate(lines):
                ly = y + li * 15.0
                if ly > y + h - 4:
                    break
                color = ctx.theme.fg if not line.startswith("#") else ctx.theme.accent
                ctx.text(px, ly, line, size=TEXT_HINT, color=color, monospace=True,
                         max_width=pw)


class BacklogApp(App):
    global_only: Arg[bool] = Arg("-g", "--global", dest="global_only", type=bool, default=False)

    def on_init(self) -> None:
        self.backlog_dir = Path(self.workspace_root) / ".plexi" / "backlog"
        self.channel_dir = _detect_channel_backlog_dir()
        self.items: list = []
        self._source: dict = {}
        self.filtered: list = []
        self.selected = 0
        self.search_query = ""
        self.in_search = False
        self.preview_text = ""
        self.preview_path = None
        self.confirm_delete = False
        self.in_add = False
        self._add_value = ""
        self.in_edit = False
        self._edit_value = ""
        self._edit_path: "Path | None" = None
        self.status = ""

        # Stateful widgets — created once here, stable across renders
        self._item_list = SelectList([])
        self._body = _TwoPaneBody(self)

        self._load()
        self.emit.info(
            f"BacklogApp ready — workspace: {self.backlog_dir}, channel: {self.channel_dir}, global_only={self.global_only}"
        )

    # ── Data ────────────────────────────────────────────────────────────────────

    def _load(self) -> None:
        self._source = {}
        all_files: list = []

        def _collect(directory: "Path | None", source: str) -> None:
            if directory is None or not directory.is_dir():
                return
            for f in directory.iterdir():
                if f.is_file() and not f.name.startswith(".") and f.suffix == ".md":
                    if f not in self._source:
                        all_files.append(f)
                        self._source[f] = source

        if not self.global_only:
            _collect(self.backlog_dir, "ws")
        _collect(self.channel_dir, "channel")
        all_files.sort(key=lambda f: f.stat().st_mtime, reverse=True)
        self.items = all_files
        self._refilter()

    def _refilter(self) -> None:
        q = self.search_query.lower()
        self.filtered = [
            f for f in self.items
            if not q or q in f.stem.lower() or q in f.name.lower()
        ]
        self.selected = min(self.selected, max(0, len(self.filtered) - 1))
        self._item_list.items = [
            {"name": f"[ch] {p.stem}" if self._source.get(p) == "channel" else p.stem}
            for p in self.filtered
        ]
        self._item_list.selected_idx = self.selected
        self._cache_preview()

    def _cache_preview(self) -> None:
        if not self.filtered or self.selected >= len(self.filtered):
            self.preview_text = ""
            self.preview_path = None
            return
        path = self.filtered[self.selected]
        if path == self.preview_path:
            return
        self.preview_path = path
        try:
            self.preview_text = path.read_text(errors="replace")
        except OSError as e:
            self.preview_text = f"Error: {e}"

    # ── Actions ─────────────────────────────────────────────────────────────────

    def _open(self) -> None:
        if not self.filtered:
            return
        path = self.filtered[self.selected]
        subprocess.Popen(["open", str(path)])
        self.status = f"Opened {path.name}"

    def _archive(self) -> None:
        if not self.filtered:
            return
        path = self.filtered[self.selected]
        archived_dir = path.parent / "archived"
        try:
            archived_dir.mkdir(parents=True, exist_ok=True)
            dest = archived_dir / path.name
            n = 2
            while dest.exists():
                dest = archived_dir / f"{path.stem}-{n}{path.suffix}"
                n += 1
            shutil.move(str(path), str(dest))
            self.status = f"Archived {path.name}"
        except OSError as e:
            self.status = f"Error: {e}"
        self.selected = max(0, self.selected - 1)
        self._load()

    def _create_item(self, title: str) -> None:
        """Create a new backlog item from a TextEdit submission."""
        title = title.strip()
        if not title:
            self.in_add = False
            return
        safe = "".join(c if c.isalnum() or c in " -_" else "-" for c in title)
        safe = safe.strip().replace(" ", "-").lower() or "untitled"
        if self.global_only:
            if not self.channel_dir:
                self.status = "Error: no channel-level backlog directory found"
                self.in_add = False
                return
            target_dir = self.channel_dir
        else:
            target_dir = self.backlog_dir
        target_dir.mkdir(parents=True, exist_ok=True)
        path = target_dir / f"{safe}.md"
        n = 2
        while path.exists():
            path = self.backlog_dir / f"{safe}-{n}.md"
            n += 1
        try:
            path.write_text(f"# {title}\n")
            self.status = f"Created {path.name}"
        except OSError as e:
            self.status = f"Error: {e}"
        self.in_add = False
        self._load()

    def _delete(self) -> None:
        if not self.filtered:
            return
        path = self.filtered[self.selected]
        try:
            path.unlink()
            self.status = f"Deleted {path.name}"
        except OSError as e:
            self.status = f"Error: {e}"
        self.confirm_delete = False
        self.selected = max(0, self.selected - 1)
        self._load()

    def _start_edit(self) -> None:
        """Enter inline edit mode for the selected item."""
        if not self.filtered:
            return
        path = self.filtered[self.selected]
        self._edit_path = path
        self._edit_value = path.stem
        self.in_edit = True
        self.emit.info(f"backlog: editing {path.name}")

    # ── Component events (TextEdit) ────────────────────────────────────────────

    def on_component_event(self, node_id, event_type, payload):
        value = (payload or {}).get("value", "")
        if node_id == "backlog-add":
            if event_type == "change":
                self._add_value = value
            elif event_type == "submit":
                self.emit.info(f"backlog: add submit: {value!r}")
                self._create_item(value)
                self._add_value = ""
        elif node_id == "backlog-edit":
            if event_type == "change":
                self._edit_value = value
            elif event_type == "submit":
                self.emit.info(f"backlog: edit submit: {value!r}")
                self._rename_item(value)

    def _rename_item(self, new_title: str) -> None:
        """Rename the file for the item being edited."""
        new_title = new_title.strip()
        if not new_title or self._edit_path is None:
            self.in_edit = False
            self._edit_path = None
            return
        old_path = self._edit_path
        safe = "".join(c if c.isalnum() or c in " -_" else "-" for c in new_title)
        safe = safe.strip().replace(" ", "-").lower() or "untitled"
        new_path = old_path.parent / f"{safe}.md"
        if new_path != old_path:
            n = 2
            while new_path.exists():
                new_path = old_path.parent / f"{safe}-{n}.md"
                n += 1
            try:
                # Rewrite file with new title heading, then rename
                content = old_path.read_text(errors="replace")
                lines = content.splitlines(keepends=True)
                if lines and lines[0].startswith("# "):
                    lines[0] = f"# {new_title}\n"
                else:
                    lines.insert(0, f"# {new_title}\n")
                old_path.write_text("".join(lines))
                old_path.rename(new_path)
                self.status = f"Renamed to {new_path.name}"
            except OSError as e:
                self.status = f"Error: {e}"
        else:
            # Same filename, just update the heading
            try:
                content = old_path.read_text(errors="replace")
                lines = content.splitlines(keepends=True)
                if lines and lines[0].startswith("# "):
                    lines[0] = f"# {new_title}\n"
                else:
                    lines.insert(0, f"# {new_title}\n")
                old_path.write_text("".join(lines))
                self.status = f"Updated {old_path.name}"
            except OSError as e:
                self.status = f"Error: {e}"
        self.in_edit = False
        self._edit_path = None
        self._load()

    # ── Render ───────────────────────────────────────────────────────────────────

    def on_render(self, ctx) -> None:
        w, h = ctx.w, ctx.h
        title = "backlog (global)" if self.global_only else "backlog"
        item_count = len(self.filtered)
        count_str = f"{item_count} item{'s' if item_count != 1 else ''}"

        # Footer content depends on mode
        if self.in_search:
            footer_widget: Component = Footer(
                f"/ {self.search_query}▌",  # block cursor
                color=ctx.theme.accent,
            )
        elif self.status:
            footer_widget = Footer(self.status, color=ctx.theme.success)
        else:
            footer_widget = FooterKeys([
                ("j/k", "navigate"),
                ("Enter", "open"),
                ("e", "edit"),
                ("n", "new"),
                ("a", "archive"),
                ("d", "delete"),
                ("/", "search"),
                ("r", "refresh"),
            ])

        ctx.render(Column([
            AppBar(title, subtitle=count_str),
            self._body,
            footer_widget,
        ], padding=0.0, padding_top=0, gap=0.0))

        # ── Add-item overlay (TextEdit) ──────────────────────────────────────────
        if self.in_add:
            ox, oy, ow, oh = w / 2 - 200, h / 2 - 36, 400, 86
            ctx.rect(ox, oy, ow, oh, ctx.theme.surface, radius=6.0)
            ctx.rect(ox, oy, ow, 1, ctx.theme.highlight)
            ctx.text(ox + 16, oy + 12, "New backlog item",
                     size=TEXT_CAPTION, color=ctx.theme.accent, bold=True)
            ctx.render_tree(TextEdit(
                "backlog-add",
                placeholder="Title (Enter to create, Esc to cancel)",
                value=self._add_value,
            ).to_node())

        # ── Edit-item overlay (TextEdit) ─────────────────────────────────────────
        if self.in_edit and self._edit_path is not None:
            ox, oy, ow, oh = w / 2 - 200, h / 2 - 36, 400, 86
            ctx.rect(ox, oy, ow, oh, ctx.theme.surface, radius=6.0)
            ctx.rect(ox, oy, ow, 1, ctx.theme.highlight)
            ctx.text(ox + 16, oy + 12, "Edit item title",
                     size=TEXT_CAPTION, color=ctx.theme.accent, bold=True)
            ctx.render_tree(TextEdit(
                "backlog-edit",
                placeholder="New title (Enter to save, Esc to cancel)",
                value=self._edit_value,
            ).to_node())

        # ── Delete confirm overlay ───────────────────────────────────────────────
        if self.confirm_delete and self.filtered:
            name = self.filtered[self.selected].name
            ox, oy, ow, oh = w / 2 - 170, h / 2 - 36, 340, 72
            ctx.rect(ox, oy, ow, oh, ctx.theme.surface, radius=6.0)
            ctx.rect(ox, oy, ow, 1, ctx.theme.highlight)
            ctx.text(ox + 16, oy + 14, f"Delete '{name}'?",
                     size=TEXT_BODY, color=ctx.theme.danger, bold=True)
            ctx.text(ox + 16, oy + 36, "Enter → confirm    Esc → cancel",
                     size=TEXT_HINT, color=ctx.theme.muted)

    # ── Keys ─────────────────────────────────────────────────────────────────────

    def on_escape(self):
        self.status = ""
        if self.in_add:
            self.in_add = False
            self._add_value = ""
            return True
        if self.in_edit:
            self.in_edit = False
            self._edit_value = ""
            self._edit_path = None
            return True
        if self.confirm_delete:
            self.confirm_delete = False
            return True
        if self.in_search:
            self.in_search = False
            self.search_query = ""
            self._refilter()
            return True
        return False

    def on_key(self, key: str, _mods: dict) -> None:
        self.status = ""   # clear on any key

        # ── Add-item mode (host owns the TextEdit buffer) ────────────────────────
        if self.in_add:
            return

        # ── Edit-item mode (host owns the TextEdit buffer) ───────────────────────
        if self.in_edit:
            return

        # ── Confirm-delete mode ──────────────────────────────────────────────────
        if self.confirm_delete:
            if key == "return":
                self._delete()
            elif key == "d":
                self.confirm_delete = False
            return

        # ── Search mode ──────────────────────────────────────────────────────────
        if self.in_search:
            if key == "backspace":
                self.search_query = self.search_query[:-1]
                self._refilter()
            elif key == "return":
                self.in_search = False
            elif len(key) == 1:
                self.search_query += key
                self._refilter()
            return

        # ── Normal mode ──────────────────────────────────────────────────────────
        if key in ("j", "down", "k", "up"):
            self._item_list.handle_key(key)
            self.selected = self._item_list.selected_idx
            self._cache_preview()
        elif key == "return":
            self._open()
        elif key == "e":
            self._start_edit()
        elif key == "a":
            self._archive()
        elif key == "d" and self.filtered:
            self.confirm_delete = True
        elif key == "n":
            self.in_add = True
            self._add_value = ""
        elif key == "r":
            self._load()
            self.status = "Refreshed"
        elif key == "/":
            self.in_search = True
            self.search_query = ""
            self._refilter()


if __name__ == "__main__":
    BacklogApp().run()
