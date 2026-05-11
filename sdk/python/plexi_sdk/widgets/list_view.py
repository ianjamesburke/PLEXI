"""ListView — stateful scrollable list widget.

Create once in on_init, call render(items) each frame inside Column([...]).
handle_key / hit_test / set_selected give full keyboard and pointer control.
"""

from __future__ import annotations


class _ListViewRenderer:
    """Internal — returned by ListView.render(); bridges Component protocol."""

    def __init__(self, owner: "ListView", items: list) -> None:
        self._owner = owner
        self._items = items

    # -- Component protocol ---------------------------------------------------

    def measure(self, _avail_w: float) -> float:
        return 0.0  # grow

    def is_grow(self) -> bool:
        return True

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        self._owner._render_items(ctx, x, y, w, h, self._items)

    # Let _render_clipped work (used by Column layout engine)
    def _render_clipped(self, ctx, x: float, y: float, w: float, h: float) -> None:
        ctx.push_clip(x, y, w, h)
        try:
            self.render(ctx, x, y, w, h)
        finally:
            ctx.pop_clip()


class ListView:
    """Stateful scrollable list widget with fixed item height.

    Usage::

        class MyApp(App):
            def on_init(self, ctx):
                self._list = ListView(item_height=48.0)

            def on_render(self, ctx):
                ctx.render(Column([
                    AppBar("Title"),
                    self._list.render([
                        ListItem(title=items[i], selected=i == self._list.selected_index)
                        for i in range(len(items))
                    ]),
                ]))

            def on_key(self, ctx, key, mods):
                if self._list.handle_key(key):
                    self.emit.schedule_render(after_ms=16)

            def on_click(self, ctx, x, y, button):
                idx = self._list.hit_test(y)
                if idx is not None:
                    self._list.set_selected(idx)
    """

    def __init__(self, item_height: float = 48.0) -> None:
        self._item_height = item_height
        self._selected: int = 0
        self._scroll_offset: float = 0.0
        self._viewport_y: float = 0.0
        self._viewport_h: float = 0.0
        self._item_count: int = 0

    # -- Public API -----------------------------------------------------------

    @property
    def selected_index(self) -> int:
        return self._selected

    def set_selected(self, idx: int) -> None:
        if self._item_count > 0:
            self._selected = max(0, min(idx, self._item_count - 1))
        else:
            self._selected = 0
        self._scroll_to_selected()

    def handle_key(self, key: str) -> bool:
        """Move selection for j/k/arrows. Returns True if the key was consumed."""
        if self._item_count == 0:
            return False
        if key in ("j", "ArrowDown", "down"):
            self._selected = min(self._selected + 1, self._item_count - 1)
        elif key in ("k", "ArrowUp", "up"):
            self._selected = max(self._selected - 1, 0)
        else:
            return False
        self._scroll_to_selected()
        return True

    def hit_test(self, y: float) -> int | None:
        """Return item index at screen-coord y (from last render), or None."""
        if self._viewport_h == 0.0 or self._item_count == 0:
            return None
        local_y = y - self._viewport_y + self._scroll_offset
        if local_y < 0.0:
            return None
        idx = int(local_y / self._item_height)
        if 0 <= idx < self._item_count:
            return idx
        return None

    def render(self, items: list) -> "_ListViewRenderer":
        """Return a renderable Component containing the given items.

        Call inside Column([...]) on every frame — the returned object is
        lightweight and safe to recreate each render.
        """
        return _ListViewRenderer(self, items)

    # -- Internal -------------------------------------------------------------

    def _total_h(self) -> float:
        return self._item_count * self._item_height

    def _clamp_scroll(self) -> None:
        max_scroll = max(0.0, self._total_h() - self._viewport_h)
        self._scroll_offset = max(0.0, min(self._scroll_offset, max_scroll))

    def _scroll_to_selected(self) -> None:
        item_top = self._selected * self._item_height
        item_bot = item_top + self._item_height
        if item_top < self._scroll_offset:
            self._scroll_offset = item_top
        elif item_bot > self._scroll_offset + self._viewport_h:
            self._scroll_offset = item_bot - self._viewport_h
        self._clamp_scroll()

    def _render_items(
        self, ctx, x: float, y: float, w: float, h: float, items: list
    ) -> None:
        self._viewport_y = y
        self._viewport_h = h
        self._item_count = len(items)
        self._clamp_scroll()

        ctx.push_clip(x, y, w, h)
        try:
            cursor_y = y - self._scroll_offset
            for item in items:
                ib = cursor_y + self._item_height
                if ib > y and cursor_y < y + h:
                    item.render(ctx, x, cursor_y, w, self._item_height)
                cursor_y += self._item_height
        finally:
            ctx.pop_clip()

        # Scrollbar indicator when content overflows
        total_h = self._total_h()
        if total_h > h and h > 0:
            try:
                from plexi_sdk.ui import HIGHLIGHT, MUTED
            except ImportError:
                return
            sb_w = 3.0
            thumb_h = max(16.0, h * (h / total_h))
            thumb_y = y + (self._scroll_offset / total_h) * h
            thumb_y = min(thumb_y, y + h - thumb_h)
            bar_x = x + w - sb_w
            ctx.rect(bar_x, y, sb_w, h, HIGHLIGHT)
            ctx.rect(bar_x, thumb_y, sb_w, thumb_h, MUTED)
