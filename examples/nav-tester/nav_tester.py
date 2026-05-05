#!/usr/bin/env python3
"""Nav Tester — POC app for the PGAP navigation stack (#392).

Demonstrates PushNav / PopNav / NavBack with a simple two-level UI:

  List view  (root)      — shows 5 named items.  Press Enter or click to open.
  Detail view (pushed)   — shows item detail.  Press Escape / Cmd+[ / back arrow
                           to go back.

The host shows a back arrow + view title in the pane chrome while the detail
view is active.  When the user navigates back, the host emits NavBack and the
app calls pop_nav() + returns to the list.
"""

from plexi_sdk import App, RenderContext, FG, MUTED, SURFACE, ACCENT, BG, BODY, CAPTION  # type: ignore[attr-defined]

# ── View identifiers ──────────────────────────────────────────────────────────
VIEW_LIST   = ""          # root — empty string means "no back target"
VIEW_DETAIL = "detail"

# Sample items for the list
ITEMS = [
    ("Mountains",   "Tall rocky things with snow on top."),
    ("Oceans",      "Very large and rather wet."),
    ("Forests",     "Full of trees. Birds optional."),
    ("Deserts",     "Surprisingly not empty."),
    ("Tundra",      "Cold, flat, and windswept."),
]


class NavTesterApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._view = VIEW_LIST
        self._selected = 0
        self._detail_index = 0

    # ── Navigation helpers ────────────────────────────────────────────────────

    def _open_detail(self, ctx: RenderContext, index: int) -> None:
        """Push the detail view onto the nav stack and switch views."""
        self._detail_index = index
        self._view = VIEW_DETAIL
        name = ITEMS[index][0]
        ctx.emit.push_nav(VIEW_DETAIL, name)

    def _go_back(self, ctx: RenderContext) -> None:
        """Return to the list view and pop the nav stack."""
        self._view = VIEW_LIST
        ctx.emit.pop_nav()

    # ── NavBack handler ───────────────────────────────────────────────────────

    def on_nav_back(self, ctx: RenderContext, view_id: str) -> None:
        """Host emitted NavBack (Cmd+[ or back arrow in chrome).

        ``view_id`` is the view we're going back *to*.  Since this app only
        has one level of depth, that's always the root (empty string).
        """
        if view_id == VIEW_LIST:
            self._go_back(ctx)

    # ── Key handler ───────────────────────────────────────────────────────────

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if self._view == VIEW_LIST:
            if key in ("up", "k"):
                self._selected = max(0, self._selected - 1)
            elif key in ("down", "j"):
                self._selected = min(len(ITEMS) - 1, self._selected + 1)
            elif key == "Enter":
                self._open_detail(ctx, self._selected)
        elif self._view == VIEW_DETAIL:
            # Escape in the detail view triggers back navigation.  The host
            # intercepts Escape when nav depth > 0 and sends NavBack instead of
            # closing the pane — so we handle it here for apps that want to
            # process it directly too.  NavBack is the canonical path; this is
            # just belt-and-suspenders for apps that don't want to wait.
            if key == "Escape":
                self._go_back(ctx)

    # ── Render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)

        if self._view == VIEW_LIST:
            self._render_list(ctx)
        else:
            self._render_detail(ctx)

    def _render_list(self, ctx: RenderContext) -> None:
        w, h = ctx.w, ctx.h
        pad = 16.0
        y = pad

        ctx.text(pad, y, "Navigation Stack Tester", size=16.0, color=FG, bold=True)
        y += 28.0
        ctx.text(pad, y, "Select an item and press Enter to open detail view.",
                 size=CAPTION, color=MUTED)
        y += 24.0

        row_h = 40.0
        for i, (name, _) in enumerate(ITEMS):
            row_y = y + i * row_h
            is_sel = i == self._selected
            bg = ACCENT if is_sel else SURFACE
            fg = FG
            ctx.rect(pad, row_y, w - pad * 2, row_h - 2, fill=bg, radius=4.0)
            ctx.text(pad + 12, row_y + 12, name, size=BODY, color=fg)

        footer_y = h - 28.0
        ctx.text(pad, footer_y, "↑/↓ select   Enter open",
                 size=CAPTION, color=MUTED)

    def _render_detail(self, ctx: RenderContext) -> None:
        w, h = ctx.w, ctx.h
        pad = 16.0
        y = pad

        name, desc = ITEMS[self._detail_index]

        ctx.text(pad, y, name, size=18.0, color=FG, bold=True)
        y += 32.0
        ctx.rect(pad, y, w - pad * 2, 1, fill=SURFACE, radius=0.0)
        y += 12.0
        ctx.text(pad, y, desc, size=BODY, color=FG)
        y += 24.0
        ctx.text(pad, y, "This is the detail view.  Use Cmd+[ or the back arrow to go back.",
                 size=CAPTION, color=MUTED)

        footer_y = h - 28.0
        ctx.text(pad, footer_y, "Cmd+[ or back arrow to return to list",
                 size=CAPTION, color=MUTED)


if __name__ == "__main__":
    NavTesterApp().run()
