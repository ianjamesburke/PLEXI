#!/usr/bin/env python3
"""Backlog viewer — browse, preview, open, archive, delete, and add backlog items.

hjkl navigation. e/Enter opens in default app. a archives. d deletes (confirm).
n creates a new item via host TextInput (issue #283). r refreshes. / searches.
Shows items from two sources merged by mtime:
  - workspace backlog: <workspace>/.plexi/backlog/
  - channel backlog:   $PLEXI_CONFIG_DIR/backlog/ (quick notes from host ⌘0)
Items from the channel backlog are prefixed with [ch] in the list.
"""

import os
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '../../sdk/python'))
from plexi_sdk import App, RenderContext, BG, SURFACE, HIGHLIGHT, ACCENT, MUTED, FG, RED, GREEN
from plexi_sdk.ui import SelectList

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
# Chrome — where the list/preview body starts and ends vertically. These
# replace the fixed header/footer bars from v1 chrome. They're tuned so the
# list sits below Plexi's own pane chrome and above the key-hint footer.
TOP       = 32.0
BOTTOM_BAR_H = 26.0


class BacklogApp(App):

    def on_init(self, ctx: RenderContext) -> None:
        self.backlog_dir = Path(ctx.workspace_root) / ".plexi" / "backlog"
        self.channel_dir = _detect_channel_backlog_dir()
        self.items: list = []         # list[Path]
        self._source: dict = {}       # Path -> "ws" | "channel"
        self.filtered: list = []
        self.selected = 0
        self.search_query = ""
        self.in_search = False
        self.preview_text = ""
        self.preview_path = None
        self.confirm_delete = False
        self.in_add = False        # showcase: host-owned TextInput entry mode
        self.status = ""
        self._item_list = SelectList([])
        self._load()
        self.emit.info(
            f"BacklogApp ready — workspace: {self.backlog_dir}, channel: {self.channel_dir}"
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
        """Create a new backlog item from a TextInput submission.

        `title` becomes the filename stem (sanitised); the file body is
        a single-line markdown header so the item shows up in the
        preview panel immediately.
        """
        title = title.strip()
        if not title:
            self.in_add = False
            return
        # Conservative filename sanitisation — mirror the convention
        # users follow when creating backlog notes by hand.
        safe = "".join(c if c.isalnum() or c in " -_" else "-" for c in title)
        safe = safe.strip().replace(" ", "-").lower() or "untitled"
        self.backlog_dir.mkdir(parents=True, exist_ok=True)
        path = self.backlog_dir / f"{safe}.md"
        # If the slug collides, append a numeric suffix until free.
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

    # ── Render ───────────────────────────────────────────────────────────────────
    #
    # SDK v2 is a vertical stack; the two-pane list+preview layout doesn't map
    # onto it without building a HSplit container. Keep the body as primitive
    # draws; only the bottom status/key-hint bar is migrated chrome.

    def on_render(self, ctx: RenderContext) -> None:
        w, h = ctx.w, ctx.h
        ctx.clear(BG)

        list_w = w * LIST_FRAC
        list_h = h - TOP - BOTTOM_BAR_H

        # Header
        ctx.text(12, 10, "backlog", size=12, color=ACCENT, bold=True,
                 max_width=list_w - 100)
        item_count = len(self.filtered)
        count_label = f"{item_count} item{'s' if item_count != 1 else ''}"
        ctx.text(list_w - 80, 10, count_label, size=11, color=MUTED,
                 max_width=80)

        has_any_dir = self.backlog_dir.is_dir() or (
            self.channel_dir is not None and self.channel_dir.is_dir()
        )
        if not has_any_dir:
            self.emit.info("backlog: no dirs found — showing empty-state guide")
            ctx.text(12, TOP + 12, "No notes yet.", size=13, color=FG)
            ctx.text(12, TOP + 34,
                     "Press ⌘0 to open Quick Note, then press Enter twice to send to your backlog.",
                     size=11, color=MUTED, max_width=w - 24)
            return

        # Divider
        ctx.line(list_w, TOP - 4, list_w, h - BOTTOM_BAR_H, color=HIGHLIGHT, width=1.0)

        # ── Item list ────────────────────────────────────────────────────────────
        if not self.filtered:
            msg = "No results" if self.search_query else "Backlog is empty"
            ctx.text(12, TOP + 12, msg, size=12, color=MUTED)
        else:
            self._item_list.render(ctx, 0, TOP, list_w, list_h)

        # ── Preview ──────────────────────────────────────────────────────────────
        px = list_w + 14
        pw = w - px - 10
        if self.preview_path and self.preview_text is not None:
            ctx.text(px, TOP - 18, self.preview_path.name,
                     size=11, color=ACCENT, bold=True, max_width=pw)
            lines = self.preview_text.splitlines()
            for li, line in enumerate(lines):
                ly = TOP + li * 15.0
                if ly > h - BOTTOM_BAR_H - 4:
                    break
                # Dim markdown headings differently
                color = FG if not line.startswith("#") else ACCENT
                ctx.text(px, ly, line, size=11, color=color, monospace=True,
                         max_width=pw)

        # ── Bottom bar ───────────────────────────────────────────────────────────
        bar_y = h - BOTTOM_BAR_H
        ctx.rect(0, bar_y, w, BOTTOM_BAR_H, SURFACE)

        if self.in_search:
            ctx.text(12, bar_y + 7,
                     f"/ {self.search_query}▌",   # block cursor
                     size=12, color=ACCENT, monospace=True)
        elif self.status:
            ctx.text(12, bar_y + 7, self.status, size=11, color=GREEN)
        else:
            ctx.text(12, bar_y + 7,
                     "j/k navigate  e open  n new  a archive  d delete  / search  r refresh",
                     size=10, color=MUTED)

        # ── Add-item overlay (host-owned TextInput) ─────────────────────────────
        # Issue #283 showcase migration: a real callsite for the new
        # single-line submit-only primitive. The host owns the buffer
        # entirely — `text_input` returns the value once on submit, then
        # `None` on subsequent frames.
        if self.in_add:
            ox, oy, ow, oh = w / 2 - 200, h / 2 - 36, 400, 86
            ctx.rect(ox, oy, ow, oh, SURFACE, radius=6.0)
            ctx.rect(ox, oy, ow, 1, HIGHLIGHT)
            ctx.text(ox + 16, oy + 12, "New backlog item",
                     size=12, color=ACCENT, bold=True)
            submitted = ctx.text_input(
                "backlog-new",
                x=ox + 16, y=oy + 36, w=ow - 32,
                placeholder="Title (Enter to create, Esc to cancel)",
            )
            if submitted is not None:
                self._create_item(submitted)

        # ── Delete confirm overlay ────────────────────────────────────────────────
        if self.confirm_delete and self.filtered:
            name = self.filtered[self.selected].name
            ox, oy, ow, oh = w / 2 - 170, h / 2 - 36, 340, 72
            ctx.rect(ox, oy, ow, oh, SURFACE, radius=6.0)
            ctx.rect(ox, oy, ow, 1, HIGHLIGHT)
            ctx.text(ox + 16, oy + 14, f"Delete  ‘{name}’?",
                     size=13, color=RED, bold=True)
            ctx.text(ox + 16, oy + 36, "Enter → confirm    Esc → cancel",
                     size=11, color=MUTED)

    # ── Keys ─────────────────────────────────────────────────────────────────────

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        self.status = ""   # clear on any key

        # ── Add-item mode (host owns the buffer) ─────────────────────────────────
        # The TextInput widget eats characters; the app only handles the
        # exit shortcut. Submission is delivered via PlexiEvent::TextSubmitted
        # and consumed in `on_render`'s `ctx.text_input(...)` call.
        if self.in_add:
            if key == "escape":
                self.in_add = False
            return

        # ── Confirm-delete mode ──────────────────────────────────────────────────
        if self.confirm_delete:
            if key == "return":
                self._delete()
            elif key in ("escape", "d"):
                self.confirm_delete = False
            return

        # ── Search mode ──────────────────────────────────────────────────────────
        if self.in_search:
            if key == "escape":
                self.in_search = False
                self.search_query = ""
                self._refilter()
            elif key == "backspace":
                self.search_query = self.search_query[:-1]
                self._refilter()
            elif key == "return":
                self.in_search = False
            elif len(key) == 1:   # single char from Text event; ignores "Slash" etc.
                self.search_query += key
                self._refilter()
            return

        # ── Normal mode ──────────────────────────────────────────────────────────
        if key in ("j", "down", "k", "up"):
            self._item_list.handle_key(key)
            self.selected = self._item_list.selected_idx
            self._cache_preview()
        elif key in ("e", "return"):
            self._open()
        elif key == "a":
            self._archive()
        elif key == "d" and self.filtered:
            self.confirm_delete = True
        elif key == "n":
            # Showcase: open the host TextInput overlay for a new item.
            self.in_add = True
        elif key == "r":
            self._load()
            self.status = "Refreshed"
        elif key == "/":             # Text event sends "/" for the slash key
            self.in_search = True
            self.search_query = ""
            self._refilter()


if __name__ == "__main__":
    BacklogApp().run()
