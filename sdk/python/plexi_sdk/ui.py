"""Plexi SDK v2 — declarative UI primitives.

A component tree that lays itself out and emits low-level `DrawCommand`s.
Apps describe *what* the screen should look like; the SDK handles *where*.

Design goals:
  - Hard to make ugly UI. Defaults do the right thing.
  - Compose: a `Card` can hold `KeyRow`s, a `Column` can hold `Card`s.
  - Responsive: components truncate, wrap, or scroll instead of clipping.
  - Escape hatch: apps that need pixel control still have `ctx.rect` /
    `ctx.text` from the lower-level API.

Usage:
    from plexi_sdk import App, RenderContext
    from plexi_sdk.ui import Column, Header, Card, KeyRow, Section, Spacer, Footer

    def on_render(self, ctx: RenderContext) -> None:
        ctx.render(Column([
            Header("My App", "Short subtitle"),
            Card([
                KeyRow("m", "Message"),
                KeyRow("c", "Choice"),
            ]),
            Section("Events"),
            Spacer(grow=True),
            Footer("Status line"),
        ]))

## Component measurement

Each component reports a `measure(avail_w) -> height` used in a single
top-to-bottom pass. `Spacer(grow=True)` reports 0 and is expanded in a
second pass to consume whatever slack is left. When the pane is smaller
than the total fixed-height content, grow spacers collapse to 0 and
content at the bottom may not render — keep the total intentionally
below the minimum pane size, or use `ScrollLog` for variable content.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import List, Optional, Union

# ── Style tokens ──────────────────────────────────────────────────────────
# Keep these in sync with Rust's src/style.rs. Adding a token here without
# a matching Rust constant is fine (pure Python), but overlap should match.

# Spacing (pixels). 4-based scale.
SPACE_XS = 4.0
SPACE_SM = 8.0
SPACE_MD = 12.0
SPACE_LG = 16.0
SPACE_XL = 24.0

# Typography (pt).
TEXT_HINT = 11.0
TEXT_CAPTION = 12.0
TEXT_BODY = 14.0
TEXT_HEADING = 16.0
TEXT_TITLE = 20.0
TEXT_TITLE_XL = 28.0

# Radii.
RADIUS_SM = 4.0
RADIUS_MD = 8.0
RADIUS_LG = 12.0

# Palette — matches the Python-side constants from plexi_sdk/__init__.py.
# Re-exported here so UI code doesn't have to import both.
BG = "#1e1e2e"
SURFACE = "#313244"
HIGHLIGHT = "#45475a"
ACCENT = "#89b4fa"
MUTED = "#6c7086"
FG = "#cdd6f4"
RED = "#f38ba8"
GREEN = "#a6e3a1"
YELLOW = "#f9e2af"

# ── Utilities ──────────────────────────────────────────────────────────────


def _wrap_to_width(text: str, avail_px: float, font_size: float,
                   mono: bool = False, max_lines: int = 3) -> List[str]:
    """Word-wrap `text` into up to `max_lines` lines. Final line gets an
    ellipsis if content was truncated.

    Uses approximate character-width ratios (0.60 mono, 0.55 proportional)
    for layout arithmetic only. Actual clip/elision at render time is handled
    by the host via `max_width` on `ctx.text()`.
    """
    if avail_px <= 0 or not text:
        return []
    char_w = font_size * (0.60 if mono else 0.55)
    max_chars = max(1, int(avail_px / char_w))

    words = text.split()
    lines: List[str] = []
    current = ""
    for word in words:
        candidate = word if not current else f"{current} {word}"
        if len(candidate) <= max_chars:
            current = candidate
            continue
        if current:
            lines.append(current)
            current = word
        else:
            # Single word longer than line — hard-break.
            lines.append(word[: max_chars - 1] + "…")
            current = ""
        if len(lines) >= max_lines:
            break
    if current and len(lines) < max_lines:
        lines.append(current)

    if len(lines) == max_lines and (
        sum(len(l) for l in lines) + len(lines) - 1 < len(text)
    ):
        last = lines[-1]
        if not last.endswith("…"):
            if len(last) >= max_chars:
                last = last[: max_chars - 1] + "…"
            else:
                last = last + "…"
            lines[-1] = last
    return lines


# ── Component base ─────────────────────────────────────────────────────────


class Component:
    """Base class. Subclasses implement `measure` and `render`."""

    def measure(self, avail_w: float) -> float:
        """Return pixel height this component needs within `avail_w`."""
        return 0.0

    def is_grow(self) -> bool:
        """True if the component grows to fill remaining space."""
        return False

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        """Emit draw commands. Implementations should stay within (x, y, w, h)."""
        raise NotImplementedError

    def _render_clipped(self, ctx, x: float, y: float, w: float, h: float) -> None:
        """Render this component clipped to its allocated rect.

        Container components (Column, Card) call this instead of `render` when
        descending into children so each child's draws are bounded to its rect.
        The PushClip/PopClip pair is emitted unconditionally; the host intersects
        the new rect with the current clip stack top (only ever tightens).
        """
        ctx.push_clip(x, y, w, h)
        try:
            self.render(ctx, x, y, w, h)
        finally:
            ctx.pop_clip()


# ── Leaf components ────────────────────────────────────────────────────────


@dataclass
class Heading(Component):
    """Title-ish text. level 1 = TEXT_TITLE_XL, 2 = TEXT_TITLE, 3 = TEXT_HEADING.

    `ctx.text(x, y, ...)` treats `y` as the TOP of the text box (host renders
    with egui::Align2::LEFT_TOP). A Heading with font size `fs` occupies rows
    `y` .. `y + fs` exactly — no added descent, no baseline offset.
    """
    text: str
    level: int = 1
    color: str = FG
    bold: bool = True

    def _font_size(self) -> float:
        return {
            1: TEXT_TITLE_XL,
            2: TEXT_TITLE,
            3: TEXT_HEADING,
        }.get(self.level, TEXT_TITLE)

    def measure(self, avail_w: float) -> float:
        return self._font_size()

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        fs = self._font_size()
        ctx.text(x, y, self.text, size=fs, color=self.color, bold=self.bold,
                 max_width=w, elide=True)


@dataclass
class Label(Component):
    """Body/caption/hint text. Wraps up to `max_lines`, then truncates.

    Line height = font_size + `LINE_LEADING`; lines stack top-to-bottom with
    the first line's top at the component's `y`.
    """
    text: str
    tone: str = "body"  # "body" | "caption" | "hint"
    color: Optional[str] = None
    bold: bool = False
    max_lines: int = 3

    LINE_LEADING = 4.0

    def _font_size(self) -> float:
        return {
            "body": TEXT_BODY,
            "caption": TEXT_CAPTION,
            "hint": TEXT_HINT,
        }.get(self.tone, TEXT_BODY)

    def _color(self) -> str:
        if self.color:
            return self.color
        return {
            "body": FG,
            "caption": FG,
            "hint": MUTED,
        }.get(self.tone, FG)

    def _lines(self, avail_w: float) -> List[str]:
        return _wrap_to_width(self.text, avail_w, self._font_size(),
                              max_lines=self.max_lines)

    def _line_h(self) -> float:
        return self._font_size() + self.LINE_LEADING

    def measure(self, avail_w: float) -> float:
        lines = self._lines(avail_w)
        if not lines:
            return 0.0
        # n lines = n-1 leadings + n font heights; equivalently n*line_h - leading.
        return len(lines) * self._line_h() - self.LINE_LEADING

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        fs = self._font_size()
        color = self._color()
        line_h = self._line_h()
        for i, line in enumerate(self._lines(w)):
            ctx.text(x, y + i * line_h, line,
                     size=fs, color=color, bold=self.bold)


@dataclass
class Spacer(Component):
    """Fixed or flex gap. `grow=True` expands to consume remaining space."""
    size: float = SPACE_MD
    grow: bool = False

    def is_grow(self) -> bool:
        return self.grow

    def measure(self, avail_w: float) -> float:
        return 0.0 if self.grow else self.size

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        return


@dataclass
class Divider(Component):
    """A horizontal 1px rule."""
    color: str = HIGHLIGHT
    margin_top: float = SPACE_SM
    margin_bottom: float = SPACE_SM

    def measure(self, avail_w: float) -> float:
        return 1.0 + self.margin_top + self.margin_bottom

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        ctx.rect(x, y + self.margin_top, w, 1.0, self.color)


@dataclass
class AppBar(Component):
    """Thin top-of-pane app bar — single-line title, vertically centred
    in a fixed band, 1px divider at the bottom edge.

    Replaces the old `Header` two-line block. Apps that need subtitle /
    metadata text put it in a separate `Label(text, tone="hint")` row
    below the AppBar — keeps the app bar minimal and predictable.

    Future: an optional `actions` slot for right-aligned menu buttons
    (e.g. settings, refresh) so apps can grow their own toolbar without
    each rolling its own layout.
    """
    title: str
    accent: str = FG

    TITLE_SIZE = 16.0   # TEXT_HEADING — app-bar scale, not billboard
    BAND_H = 36.0       # thin band; native-toolbar feel
    DIVIDER_H = 1.0

    def measure(self, avail_w: float) -> float:
        return self.BAND_H + self.DIVIDER_H

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        # Opaque BG backdrop so content scrolled under the AppBar doesn't
        # bleed through (defensive; the host clip stack makes overdraw
        # structurally impossible, but the backdrop keeps chrome visually
        # solid regardless of rendering order).
        ctx.rect(x, y, w, h, BG)
        # Vertically centre the title in BAND_H. Empirical -1px nudge to
        # compensate for proportional-font descent bias at 16pt.
        text_y = y + (self.BAND_H - self.TITLE_SIZE) / 2.0 - 1.0
        ctx.text(x, text_y, self.title,
                 size=self.TITLE_SIZE, color=self.accent, bold=True,
                 max_width=w, elide=True)
        ctx.rect(x, y + self.BAND_H, w, self.DIVIDER_H, HIGHLIGHT)


@dataclass
class Section(Component):
    """Section divider with a small uppercase label sitting above the rule.

    Vertical stack: SPACE_SM padding, label (TEXT_HINT), SPACE_XS, divider,
    SPACE_SM padding.
    """
    title: str

    def measure(self, avail_w: float) -> float:
        return SPACE_SM + TEXT_HINT + SPACE_XS + 1.0 + SPACE_SM

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        label_y = y + SPACE_SM
        ctx.text(x, label_y, self.title.upper(),
                 size=TEXT_HINT, color=MUTED, bold=True,
                 max_width=w, elide=True)
        line_y = label_y + TEXT_HINT + SPACE_XS
        ctx.rect(x, line_y, w, 1.0, HIGHLIGHT)


@dataclass
class KeyRow(Component):
    """A keycap chip (or a chord of chips) followed by a description, left-aligned.

    Emits DrawCommand::KeyChipRow — the host measures each chip with real font
    metrics and flows them left-to-right. No Python-side width math.

    `key` accepts a single string (e.g. `"m"`) or a list (e.g. `["⌘", "K"]`).
    """
    key: Union[str, List[str]]
    description: str

    HEIGHT = 28.0
    CHIP_PAD_V = 1.0  # used for measure() height only

    def _keys(self) -> List[str]:
        if isinstance(self.key, list):
            return self.key
        return [self.key]

    def measure(self, avail_w: float) -> float:
        return self.HEIGHT

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        # Vertical offset so the chip row is centred within HEIGHT.
        chip_h = TEXT_HINT + self.CHIP_PAD_V * 2
        chip_y = y + (self.HEIGHT - chip_h) / 2.0
        ctx.key_chip_row(x=x, y=chip_y, keys=self._keys(),
                         description=self.description, font_size=TEXT_HINT)


@dataclass
class ScrollLog(Component):
    """Bounded text log. Shows the most recent lines that fit in the available
    space; older lines are hidden. Lines are rendered newest-at-top."""
    lines: List[str]
    line_size: float = TEXT_CAPTION
    empty_text: str = "no events yet"
    max_pixel_height: Optional[float] = None
    _assigned_h: float = field(default=0.0, repr=False)

    def is_grow(self) -> bool:
        # ScrollLog takes what it's given — typically follows a Spacer(grow=True).
        # Marking it grow=False means it won't expand past its content unless
        # explicitly sized. See `flex=True` variant below if we ever want that.
        return False

    def measure(self, avail_w: float) -> float:
        if not self.lines:
            return self.line_size + 6.0
        line_h = self.line_size + 4.0
        content_h = len(self.lines) * line_h
        if self.max_pixel_height is not None:
            return min(content_h, self.max_pixel_height)
        return content_h

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        if not self.lines:
            ctx.text(x, y, self.empty_text,
                     size=self.line_size, color=MUTED)
            return
        line_h = self.line_size + 4.0
        visible = max(1, int(h / line_h))
        recent = list(reversed(self.lines[-visible:]))
        for i, line in enumerate(recent):
            ctx.text(x, y + i * line_h, line,
                     size=self.line_size, color=FG, monospace=True,
                     max_width=w, elide=True)


@dataclass
class Scrollable(Component):
    """A clip-bounded vertically-scrollable container.

    Renders its `child` component clipped to the allocated rect. If the child
    is taller than the available height the excess is hidden and a thin
    scrollbar indicator is drawn on the right edge.

    Scroll offset is persisted on the instance, so the `Scrollable` must be
    stable across renders — create it once in `on_init` (or as a class
    attribute), not inside `on_render`.

    Keyboard scroll: j/k or arrow-down/up keys update `scroll_offset`.
    Apps drive this by calling `handle_key(key)` from their `on_key` handler.

    # IMPL-NOTE: Wheel/touch event plumbing is deferred. The PGAP protocol
    # doesn't yet carry scroll-wheel events from the host to the app. Once
    # PlexiEvent::Scroll is wired through (a follow-up to #314), Scrollable
    # can subscribe to it here with no breaking changes. For now, keyboard
    # scroll (j/k) is the v1 input source. Apps that need wheel scroll today
    # should use the host-managed DrawCommand::List primitive instead.
    """
    child: Component
    scroll_offset: float = field(default=0.0, repr=False)
    # How many pixels j/k advances per keypress.
    key_step: float = 20.0

    # Width of the scrollbar indicator drawn when content overflows.
    _SCROLLBAR_W: float = field(default=3.0, init=False, repr=False)
    # Stored child height from last measure (used for scrollbar sizing).
    _child_h: float = field(default=0.0, repr=False)
    # Stored allocated height from last render (used for scroll clamping).
    _avail_h: float = field(default=0.0, repr=False)

    def measure(self, avail_w: float) -> float:
        """Scrollable reports 0 so it grows to consume available space."""
        return 0.0

    def is_grow(self) -> bool:
        return True

    def _clamp_offset(self, avail_h: float) -> None:
        max_offset = max(0.0, self._child_h - avail_h)
        self.scroll_offset = max(0.0, min(self.scroll_offset, max_offset))

    def handle_key(self, key: str) -> bool:
        """Update scroll_offset for j/k/ArrowDown/ArrowUp keys.

        Returns True if the key was consumed. Call from the app's on_key handler:

            if self._scrollable.handle_key(key):
                return  # consumed
        """
        if key in ("j", "ArrowDown", "down"):
            self.scroll_offset += self.key_step
            self._clamp_offset(self._avail_h)
            return True
        if key in ("k", "ArrowUp", "up"):
            self.scroll_offset = max(0.0, self.scroll_offset - self.key_step)
            return True
        return False

    def ensure_visible(self, top: float, bottom: float, margin: float = 0.0) -> None:
        """See `ensure_visible(...)` free function. Wrapper for Scrollable's
        own offset + cached viewport height. Use this from inside an app
        that's wrapped its content in a Scrollable instance:

            self._sel_idx = min(self._sel_idx + 1, len(items) - 1)
            self._scrollable.ensure_visible(self._sel_idx * ROW_H,
                                            self._sel_idx * ROW_H + ROW_H)
        """
        if self._avail_h <= 0:
            return
        self.scroll_offset = ensure_visible(
            self.scroll_offset, self._avail_h, top, bottom, margin=margin
        )
        self._clamp_offset(self._avail_h)

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        self._avail_h = h
        # Measure child at our width (less scrollbar gutter).
        content_w = w - self._SCROLLBAR_W - 2.0
        self._child_h = self.child.measure(content_w)
        self._clamp_offset(h)

        # Clip to our allocated rect, then render child offset upward.
        ctx.push_clip(x, y, w, h)
        try:
            child_y = y - self.scroll_offset
            self.child.render(ctx, x, child_y, content_w, self._child_h)
        finally:
            ctx.pop_clip()

        # Scrollbar indicator (only when content overflows).
        if self._child_h > h and h > 0:
            track_h = h
            thumb_ratio = h / self._child_h
            thumb_h = max(16.0, track_h * thumb_ratio)
            thumb_y = y + (self.scroll_offset / self._child_h) * track_h
            # Clamp thumb to track
            thumb_y = min(thumb_y, y + track_h - thumb_h)
            bar_x = x + w - self._SCROLLBAR_W
            ctx.rect(bar_x, y, self._SCROLLBAR_W, track_h, HIGHLIGHT)
            ctx.rect(bar_x, thumb_y, self._SCROLLBAR_W, thumb_h, MUTED)


def ensure_visible(scroll_offset: float, viewport_h: float,
                   top: float, bottom: float, margin: float = 0.0) -> float:
    """Solve 'selection follows scroll' in one call. Returns the new offset.

    The pattern: the user is navigating items with j/k. The cursor moves
    freely while it stays inside the visible viewport; the moment it would
    go off the top or bottom edge, the viewport scrolls just enough to
    keep it visible. Identical to every native list widget.

    Apps call this from their nav handler after mutating their
    selected_index — works whether the app uses a `Scrollable` component
    or hand-rolls its own scroll offset (commit-graph's clip-and-offset
    style):

        new_sel = min(self._sel + 1, len(items) - 1)
        item_top = new_sel * ROW_H
        self._scroll_offset = ensure_visible(
            self._scroll_offset, viewport_h,
            top=item_top, bottom=item_top + ROW_H,
        )
        self._sel = new_sel

    Args:
        scroll_offset: current scroll offset in the child's local space.
        viewport_h:    visible height of the viewport.
        top, bottom:   the item's top/bottom edges in the child's local space.
        margin:        scrolloff equivalent. Set to one row-height to keep
                       a row of breathing room above/below the cursor.

    Returns:
        The new scroll_offset that keeps `[top, bottom]` visible. Identical
        to the input if the cursor was already in view.
    """
    if viewport_h <= 0:
        return scroll_offset
    cursor_top = scroll_offset + margin
    cursor_bottom = scroll_offset + viewport_h - margin
    if top < cursor_top:
        return max(0.0, top - margin)
    if bottom > cursor_bottom:
        return bottom - viewport_h + margin
    return scroll_offset


@dataclass
class Footer(Component):
    """Small caption row. Wraps instead of clipping. The parent `Column`
    provides the outer bottom padding, so no extra padding is needed here."""
    text: str
    color: str = MUTED
    max_lines: int = 2

    TOP_GAP = SPACE_MD
    LINE_H = TEXT_HINT + 5.0

    def _lines(self, avail_w: float) -> List[str]:
        return _wrap_to_width(self.text, avail_w, TEXT_HINT,
                              max_lines=self.max_lines)

    def measure(self, avail_w: float) -> float:
        lines = self._lines(avail_w)
        count = max(1, len(lines))
        return self.TOP_GAP + 1.0 + self.TOP_GAP + count * self.LINE_H

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        line_y = y + self.TOP_GAP
        ctx.rect(x, line_y, w, 1.0, HIGHLIGHT)
        text_y = line_y + 1.0 + self.TOP_GAP
        for i, line in enumerate(self._lines(w)):
            ctx.text(x, text_y + i * self.LINE_H, line,
                     size=TEXT_HINT, color=self.color)


@dataclass
class FooterKeys(Component):
    """Footer row that renders keyboard shortcuts as key chips + descriptions.

    Each shortcut is a ``(key_or_keys, description)`` tuple — the same shape
    as ``KeyRow``.  Chips are rendered inline (horizontal flow) separated by a
    small gap, identical in style to ``KeyRow`` but packed tightly so many
    shortcuts fit on one line.

    Example::

        FooterKeys([
            ("j", "down"),
            ("k", "up"),
            (["g", "G"], "ends"),
            ("?", "help"),
        ])

    ``key_or_keys`` may be a single string or a list of strings (chord).
    Lists are joined with ``/`` as a single chip label so they stay compact
    in the footer context.
    """
    shortcuts: List[tuple]  # list of (key_or_keys, description)

    # TOP_GAP reduced from SPACE_MD (12px) to SPACE_SM (8px) — trimmer chrome.
    TOP_GAP = SPACE_SM
    CHIP_H = TEXT_HINT + 2.0 * 1.0   # TEXT_HINT + 2*CHIP_PAD_V
    # Single-row height. The host wraps the row to multiple lines when
    # `max_width` can't fit everything; very narrow panes may render past
    # this measurement. Apps wanting exact bounded footers should put
    # FooterKeys in a fixed-height region or constrain the shortcut count.
    ROW_H = CHIP_H + 2.0  # reduced from +4.0 — tighter without cramping chips

    def measure(self, avail_w: float) -> float:
        return self.TOP_GAP + 1.0 + self.TOP_GAP + self.ROW_H

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        # Opaque BG backdrop so any content scrolled behind the footer doesn't
        # bleed through the divider line.
        ctx.rect(x, y, w, h, BG)
        line_y = y + self.TOP_GAP
        ctx.rect(x, line_y, w, 1.0, HIGHLIGHT)

        chip_row_y = line_y + 1.0 + self.TOP_GAP

        # Single host-measured shortcuts row — host owns ALL geometry:
        # chip widths from real font metrics, inter-group flow, and
        # multi-line wrap when `max_width` is exceeded. SDK does no
        # width math, no truncation, no overlap. This is the whole
        # point of the host-measured layout primitives (#312).
        ctx.shortcuts(
            x=x,
            y=chip_row_y,
            max_width=w,
            pairs=list(self.shortcuts),
            font_size=TEXT_HINT,
        )


# ── Badge primitive ────────────────────────────────────────────────────────


def badge(
    ctx,
    x: float,
    y_center: float,
    label: str,
    fill: str = ACCENT,
    fg: str = BG,
    font_size: float = TEXT_HINT,
    radius: float = RADIUS_MD,
) -> None:
    """Render a host-measured pill badge centred on ``y_center``.

    The host measures the label with real egui font metrics, sizes the pill
    (text_w + padding), and centres the text — no Python width math.

    Args:
        ctx:       A ``RenderContext`` instance.
        x:         Left edge of the badge.
        y_center:  Vertical centre of the badge (e.g. the commit-node ``cy``).
        label:     Text to display inside the pill.
        fill:      Pill background colour.
        fg:        Text colour (default ``BG`` — dark text on light pill).
        font_size: Label pt size (default ``TEXT_HINT``).
        radius:    Corner radius. Use ``RADIUS_SM`` (4 px) for tag chips,
                   ``RADIUS_MD`` (8 px, default) for branch badges.
    """
    ctx.badge(x=x, y_center=y_center, label=label,
              fill=fill, fg=fg, font_size=font_size, radius=radius)


# ── Loading pill (suspense indicator) ──────────────────────────────────────
#
# A small chip that apps overlay on top of stale content while a refresh
# is in flight. The point: don't full-swap the pane to a spinner card on
# every refresh — keep the existing UI mounted, surface a localised
# loading indicator only over the region that's being refreshed.
#
# Pattern in the calling app:
#     1. Track `_fetching: bool` separately from `_mode`.
#     2. On first-ever fetch, show `_render_loading()` (full-pane spinner).
#     3. On every subsequent fetch, set `_fetching = True` and re-render —
#        the existing _render_ready stays up; loading_pill renders on top.
#     4. When the fetch completes: set `_fetching = False`, update data,
#        re-render. Pill disappears, content updates in place.
#
# This is the SDK-level equivalent of React Suspense with stale-while-
# revalidate: the boundary stays mounted with current content; the only
# visual signal is a small pill instead of a destructive remount.

import time as _ct_time  # `time` collides with some example apps' imports


def loading_pill(ctx, x: float, y: float, label: str = "Fetching…") -> float:
    """Render a small spinner+label pill at (x, y). Returns rendered width.

    The pill uses host-measured `badge()` rendering (so widths are
    correct), with a wall-clock-driven Braille spinner glyph that ticks
    at 8 fps regardless of how often `loading_pill` is called.

    Pattern: position this in the top-right of the region being
    refreshed. While `_fetching` is true, render it on top of the stale
    content. When the fetch completes, just stop calling it.

    Args:
        ctx:   RenderContext.
        x, y:  Top-left of the pill (NOT y-centre — easier to anchor).
        label: Text shown after the spinner glyph.
    """
    spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
    idx = int(_ct_time.monotonic() * 8) % len(spinner)
    text = f"{spinner[idx]}  {label}"
    # Use the host-measured badge; subtle styling (surface fill, muted fg).
    # Pill is anchored top-left here; convert to y_center for badge().
    ctx.badge(x=x, y_center=y + 9.0, label=text,
              fill=HIGHLIGHT, fg=FG, font_size=TEXT_HINT,
              radius=RADIUS_SM)
    # Approx width — not measured here because we don't need it for
    # placement (callers anchor by top-right of the parent region).
    return len(text) * TEXT_HINT * 0.62 + 16.0


# ── Container components ───────────────────────────────────────────────────


@dataclass
class Card(Component):
    """Surface-colored container with inner padding. Stacks its children
    vertically with a configurable gap. A 1px border in HIGHLIGHT separates
    it from the pane background — essential when SURFACE and BG are close
    in brightness."""
    children: List[Component]
    padding: float = SPACE_LG
    gap: float = SPACE_XS
    background: str = SURFACE
    border: Optional[str] = HIGHLIGHT  # set to None for a borderless card
    radius: float = RADIUS_MD

    def _inner_w(self, outer_w: float) -> float:
        return outer_w - 2 * self.padding

    def measure(self, avail_w: float) -> float:
        inner_w = self._inner_w(avail_w)
        if not self.children:
            return 2 * self.padding
        child_heights = [c.measure(inner_w) for c in self.children]
        total = sum(child_heights) + self.gap * max(0, len(self.children) - 1)
        return total + 2 * self.padding

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        ctx.rect(x, y, w, h, self.background, radius=self.radius)
        if self.border:
            # Top + bottom + left + right 1px strokes. Drawn as four thin
            # rects because `ctx.rect` doesn't support a separate stroke.
            ctx.rect(x, y, w, 1.0, self.border)
            ctx.rect(x, y + h - 1.0, w, 1.0, self.border)
            ctx.rect(x, y, 1.0, h, self.border)
            ctx.rect(x + w - 1.0, y, 1.0, h, self.border)
        inner_x = x + self.padding
        inner_y = y + self.padding
        inner_w = w - 2 * self.padding
        cursor = inner_y
        for i, child in enumerate(self.children):
            ch = child.measure(inner_w)
            child._render_clipped(ctx, inner_x, cursor, inner_w, ch)
            cursor += ch
            if i < len(self.children) - 1:
                cursor += self.gap


@dataclass
class Column(Component):
    """The root container. Stacks children vertically. Handles grow spacers:
    measures fixed-height children first, then distributes leftover space to
    any `Spacer(grow=True)` descendants at the top level.

    Padding defaults to `SPACE_XL` (24px) on the sides and bottom, and
    `SPACE_SM` (8px) on the top. A top-of-pane `Header` carries its own
    visual weight via TEXT_TITLE_XL and its own bottom rhythm (gap +
    divider), so anything above a few px reads as "the title is dropped"
    rather than anchored. The other three sides stay at 24px where content
    *does* need breathing room.

    Override either with `padding=` (all sides) or `padding_top=` (top only).
    """
    children: List[Component]
    padding: float = SPACE_XL
    padding_top: Optional[float] = None
    gap: float = SPACE_MD

    @property
    def _pad_top(self) -> float:
        return self.padding_top if self.padding_top is not None else SPACE_SM

    def measure(self, avail_w: float) -> float:
        inner_w = avail_w - 2 * self.padding
        total = 0.0
        for i, c in enumerate(self.children):
            total += c.measure(inner_w)
            if i < len(self.children) - 1:
                total += self.gap
        return total + self._pad_top + self.padding

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        inner_x = x + self.padding
        inner_y = y + self._pad_top
        inner_w = w - 2 * self.padding
        inner_h = h - self._pad_top - self.padding

        heights = [c.measure(inner_w) for c in self.children]
        gap_total = self.gap * max(0, len(self.children) - 1)
        fixed_used = sum(heights) + gap_total

        grow_indices = [i for i, c in enumerate(self.children) if c.is_grow()]
        slack = max(0.0, inner_h - fixed_used)
        if grow_indices and slack > 0:
            share = slack / len(grow_indices)
            for i in grow_indices:
                heights[i] += share

        cursor = inner_y
        for i, child in enumerate(self.children):
            ch = heights[i]
            if cursor + ch > inner_y + inner_h:
                # Clamp to remaining space; prevents overdraw on a too-small pane.
                ch = max(0.0, inner_y + inner_h - cursor)
                if ch <= 0:
                    break
            child._render_clipped(ctx, inner_x, cursor, inner_w, ch)
            cursor += ch
            if i < len(self.children) - 1:
                cursor += self.gap


# ── Public render entry point ──────────────────────────────────────────────


def render_tree(ctx, root: Component, fill: str = BG) -> None:
    """Clear the pane to `fill`, then render `root` into the full pane rect.

    Apps normally call `ctx.render(root)` instead, which calls this.
    """
    ctx.clear(fill)
    root.render(ctx, 0.0, 0.0, ctx.w, ctx.h)


__all__ = [
    # tokens
    "SPACE_XS", "SPACE_SM", "SPACE_MD", "SPACE_LG", "SPACE_XL",
    "TEXT_HINT", "TEXT_CAPTION", "TEXT_BODY", "TEXT_HEADING",
    "TEXT_TITLE", "TEXT_TITLE_XL",
    "RADIUS_SM", "RADIUS_MD", "RADIUS_LG",
    "BG", "SURFACE", "HIGHLIGHT", "ACCENT", "MUTED", "FG",
    "RED", "GREEN", "YELLOW",
    # components
    "Component", "Column", "Card",
    "AppBar", "Section", "KeyRow", "Heading", "Label",
    "Spacer", "Divider", "ScrollLog", "Scrollable", "Footer", "FooterKeys",
    # badge primitive
    "badge",
    # scroll helpers
    "ensure_visible",
    # entry
    "render_tree",
]
