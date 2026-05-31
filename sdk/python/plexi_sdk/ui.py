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
# Badge-specific radius — between tag-chip (4) and full-stadium (8). At
# TEXT_HINT size the pill height is ~17 px; RADIUS_MD makes it 94% of
# max-oval (cliché). 6.0 gives visible corners while staying clearly rounded.
# Keep in sync with src/style.rs RADIUS_BADGE and _render_context.py badge().
RADIUS_BADGE = 6.0

# Live host theme — populated from the Init payload (light/dark + user overrides).
# Components read theme.<role> at render time so they track the active theme.
from ._theme import theme

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


def _markdown_measure_lines(text: str, avail_px: float, font_size: float,
                            max_lines: int) -> int:
    """Conservative line estimate for host-rendered markdown blocks.

    The real markdown renderer lives in Rust/egui, so the Python SDK cannot
    know exact glyph metrics or list indentation. This estimator intentionally
    errs tall: chat bubbles should have a little breathing room, not clip the
    final markdown row outside the bubble.
    """
    if avail_px <= 0 or not text:
        return 0

    char_w = font_size * 0.55
    base_chars = max(1, int(avail_px / char_w))
    total = 0

    for raw in text.splitlines() or [text]:
        line = raw.strip()
        if not line:
            total += 1
            continue

        # CommonMark lists/code blocks reserve horizontal space for markers
        # and indentation; reduce the wrapping budget to match that shape.
        budget = base_chars
        if line.startswith(("- ", "* ", "+ ")) or line[:3].isdigit():
            budget = max(1, budget - 4)
            line = line[2:].strip() if not line[:3].isdigit() else line
        elif line.startswith(("```", "    ")):
            budget = max(1, int(budget * 0.82))

        # Markdown headings/lists get extra vertical rhythm in egui_commonmark.
        block_extra = 1 if line.startswith(("#", "- ", "* ", "+ ", "```")) else 0
        wrapped = _wrap_to_width(line, avail_px=budget * char_w,
                                 font_size=font_size, max_lines=max_lines)
        total += max(1, len(wrapped)) + block_extra
        if total >= max_lines:
            return max_lines

    return min(total, max_lines)


# ── Component base ─────────────────────────────────────────────────────────


class Component:
    """Base class. Subclasses implement `measure` and `render`."""

    def measure(self, _avail_w: float) -> float:
        """Return pixel height this component needs within `avail_w`."""
        return 0.0

    def is_grow(self) -> bool:
        """True if the component grows to fill remaining space."""
        return False

    def render(self, _ctx, _x: float, _y: float, _w: float, _h: float) -> None:
        """Emit draw commands. Implementations should stay within (x, y, w, h)."""
        raise NotImplementedError

    def render_into(self, ctx, x: float, y: float, w: float) -> float:
        """Measure, render, and return the consumed height.

        Enables flow layout without manual y-coordinate arithmetic::

            y = 0.0
            y += appbar.render_into(ctx, 0, y, ctx.w)
            y += section.render_into(ctx, 0, y, ctx.w)
        """
        h = self.measure(w)
        self.render(ctx, x, y, w, h)
        return h

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
    `y` .. `y + fs` plus descender padding.
    """
    text: str
    level: int = 1
    color: "str | None" = None
    bold: bool = True

    DESCENDER_PAD = 3.0

    def _font_size(self) -> float:
        return {
            1: TEXT_TITLE_XL,
            2: TEXT_TITLE,
            3: TEXT_HEADING,
        }.get(self.level, TEXT_TITLE)

    def measure(self, _avail_w: float) -> float:
        return self._font_size() + self.DESCENDER_PAD

    def render(self, ctx, x: float, y: float, w: float, _h: float) -> None:
        fs = self._font_size()
        ctx.text(x, y, self.text, size=fs, color=self.color or theme.fg, bold=self.bold,
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
    # Extra space below the last line's baseline to avoid clipping
    # descenders (g, y, p, q). Roughly 20% of body font size.
    DESCENDER_PAD = 3.0

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
            "body": theme.fg,
            "caption": theme.fg,
            "hint": theme.muted,
        }.get(self.tone, theme.fg)

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
        # Add descender padding so the last line's descenders aren't clipped.
        return len(lines) * self._line_h() - self.LINE_LEADING + self.DESCENDER_PAD

    def render(self, ctx, x: float, y: float, w: float, _h: float) -> None:
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

    def measure(self, _avail_w: float) -> float:
        return 0.0 if self.grow else self.size

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        return


@dataclass
class Divider(Component):
    """A horizontal 1px rule."""
    color: "str | None" = None
    margin_top: float = SPACE_SM
    margin_bottom: float = SPACE_SM

    def measure(self, avail_w: float) -> float:
        return 1.0 + self.margin_top + self.margin_bottom

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        ctx.rect(x, y + self.margin_top, w, 1.0, self.color or theme.highlight)


@dataclass
class AppBar(Component):
    """Thin top-of-pane app bar with optional subtitle.

    Single-line (title only): fixed BAND_H band, title vertically centred.
    Two-line (title + subtitle): taller BAND_H_DOUBLE band, title/subtitle
    stacked and centred together.
    """
    title: str
    subtitle: Optional[str] = None
    accent: Optional[str] = None  # default: theme.fg (resolved at render time)

    TITLE_SIZE = 16.0
    SUBTITLE_SIZE = TEXT_HINT
    BAND_H = 34.0
    # Two-line band = SPACE_SM top + title + SPACE_XS + subtitle + SPACE_SM
    # bottom = 8 + 16 + 4 + 12 + 8 = 48. Snug, symmetric padding.
    BAND_H_DOUBLE = 48.0
    DIVIDER_H = 1.0

    def _band(self) -> float:
        return self.BAND_H_DOUBLE if self.subtitle else self.BAND_H

    def measure(self, avail_w: float) -> float:
        return self._band() + self.DIVIDER_H

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        ctx.rect(x, y, w, h, theme.bg)
        accent = self.accent or theme.fg
        band = self._band()
        text_x = x + SPACE_MD
        text_w = w - 2 * SPACE_MD
        if self.subtitle:
            block_h = self.TITLE_SIZE + SPACE_XS + self.SUBTITLE_SIZE
            title_y = y + (band - block_h) / 2.0
            sub_y = title_y + self.TITLE_SIZE + SPACE_XS
            ctx.text(text_x, title_y, self.title,
                     size=self.TITLE_SIZE, color=accent, bold=True,
                     max_width=text_w, elide=True)
            ctx.text(text_x, sub_y, self.subtitle,
                     size=self.SUBTITLE_SIZE, color=theme.muted,
                     max_width=text_w, elide=True)
        else:
            text_y = y + (band - self.TITLE_SIZE) / 2.0
            ctx.text(text_x, text_y, self.title,
                     size=self.TITLE_SIZE, color=accent, bold=True,
                     max_width=text_w, elide=True)
        ctx.rect(x, y + band, w, self.DIVIDER_H, theme.highlight)


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
                 size=TEXT_HINT, color=theme.muted, bold=True,
                 max_width=w, elide=True)
        line_y = label_y + TEXT_HINT + SPACE_XS
        ctx.rect(x, line_y, w, 1.0, theme.highlight)


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
                     size=self.line_size, color=theme.muted)
            return
        line_h = self.line_size + 4.0
        visible = max(1, int(h / line_h))
        recent = list(reversed(self.lines[-visible:]))
        for i, line in enumerate(recent):
            ctx.text(x, y + i * line_h, line,
                     size=self.line_size, color=theme.fg, monospace=True,
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

    Mouse-wheel scroll: call `handle_scroll(delta_y)` from the app's
    `on_scroll_delta` handler (receives `PlexiEvent::Scroll` from the host).
    """
    child: Component
    scroll_offset: float = field(default=0.0, repr=False)
    # How many pixels j/k advances per keypress.
    key_step: float = 20.0

    def __post_init__(self):
        if not isinstance(self.child, Component):
            raise TypeError(
                f"Scrollable child must subclass Component, got {type(self.child).__name__}. "
                "Ad-hoc widget classes missing _render_clipped will crash at render time."
            )

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

    def handle_scroll(self, delta_y: float) -> bool:
        """Update scroll_offset from a mouse-wheel delta.

        Returns True (always consumes the event). Call from the app's
        on_scroll_delta handler:

            def on_scroll_delta(self, _ctx, delta_y):
                self._scrollable.handle_scroll(delta_y)
                self.emit.schedule_render()

        `delta_y` is positive when scrolling up (matches egui's
        smooth_scroll_delta convention). The offset is clamped to
        [0, content_height - viewport_height].
        """
        self.scroll_offset = max(0.0, self.scroll_offset - delta_y)
        self._clamp_offset(self._avail_h)
        return True

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
        ctx.register_scroll_consumer(self)
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
            ctx.rect(bar_x, y, self._SCROLLBAR_W, track_h, theme.highlight)
            ctx.rect(bar_x, thumb_y, self._SCROLLBAR_W, thumb_h, theme.muted)


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
    color: "str | None" = None
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
        ctx.rect(x, line_y, w, 1.0, theme.highlight)
        text_y = line_y + 1.0 + self.TOP_GAP
        for i, line in enumerate(self._lines(w)):
            ctx.text(x, text_y + i * self.LINE_H, line,
                     size=TEXT_HINT, color=self.color or theme.muted)


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
    # Draw the separating rule above the chip row. True for true footers
    # (bottom of a pane). Set False when this row sits directly under an
    # AppBar, whose own bottom divider already separates it — otherwise the
    # two rules stack with dead space between them.
    divider: bool = True

    # TOP_GAP reduced from SPACE_MD (12px) to SPACE_SM (8px) — trimmer chrome.
    TOP_GAP = SPACE_SM
    CHIP_H = TEXT_HINT + 2.0 * 1.0   # TEXT_HINT + 2*CHIP_PAD_V
    # Single-row height. The host wraps the row to multiple lines when
    # `max_width` can't fit everything; very narrow panes may render past
    # this measurement. Apps wanting exact bounded footers should put
    # FooterKeys in a fixed-height region or constrain the shortcut count.
    ROW_H = CHIP_H + 2.0  # reduced from +4.0 — tighter without cramping chips

    def measure(self, avail_w: float) -> float:
        if not self.divider:
            # Symmetric padding above and below the chip row.
            return self.TOP_GAP + self.ROW_H + self.TOP_GAP
        return self.TOP_GAP + 1.0 + self.TOP_GAP + self.ROW_H + self.TOP_GAP

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        # Opaque BG backdrop so any content scrolled behind the footer doesn't
        # bleed through the divider line.
        ctx.rect(x, y, w, h, theme.bg)
        if self.divider:
            line_y = y + self.TOP_GAP
            ctx.rect(x, line_y, w, 1.0, theme.highlight)
            chip_row_y = line_y + 1.0 + self.TOP_GAP + (self.ROW_H - self.CHIP_H) / 2.0
        else:
            # Center the chip row vertically within its symmetric band.
            chip_row_y = y + self.TOP_GAP + (self.ROW_H - self.CHIP_H) / 2.0

        # Single host-measured shortcuts row — host owns ALL geometry:
        # chip widths from real font metrics, inter-group flow, and
        # multi-line wrap when `max_width` is exceeded. SDK does no
        # width math, no truncation, no overlap. This is the whole
        # point of the host-measured layout primitives (#312).
        ctx.shortcuts(
            x=x + SPACE_MD,
            y=chip_row_y,
            max_width=w - 2 * SPACE_MD,
            pairs=list(self.shortcuts),
            font_size=TEXT_HINT,
        )


# ── Composite list components ──────────────────────────────────────────────


@dataclass
class ListItem(Component):
    """Single or double-line list item with optional leading icon and trailing text.

    Replaces the manual ``ctx.rect`` + y-offset pattern for list rows.
    All vertical centering is handled internally — no ``align=`` juggling or
    ``h * 0.38`` / ``h * 0.72`` magic numbers needed.

    Example::

        ListItem(
            title=cmd["name"],
            subtitle=cmd.get("description"),
            trailing="›",
            selected=(i == self._sel),
        )
    """
    title: str
    subtitle: Optional[str] = None
    leading: Optional[str] = None   # icon character or short label
    trailing: Optional[str] = None  # chevron, badge text
    selected: bool = False
    background: Optional[str] = None  # default: theme.surface (or theme.highlight when selected)
    radius: float = RADIUS_MD

    HEIGHT_SINGLE = 36.0
    HEIGHT_DOUBLE = 48.0
    _LEAD_SLOT = SPACE_XL    # fixed slot width for leading icon
    _TRAIL_SLOT = SPACE_LG   # fixed slot width for trailing text
    _PAD_H = SPACE_MD        # inner horizontal padding

    def _h(self) -> float:
        return self.HEIGHT_DOUBLE if self.subtitle else self.HEIGHT_SINGLE

    def measure(self, avail_w: float) -> float:
        return self._h()

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        bg = theme.highlight if self.selected else (self.background or theme.surface)
        ctx.rect(x, y, w, h, bg, radius=self.radius)

        inner_x = x + self._PAD_H
        inner_w = w - self._PAD_H * 2

        if self.leading:
            ctx.text(inner_x, y + h / 2.0, self.leading,
                     size=TEXT_BODY, color=theme.muted, align="left_center")
            inner_x += self._LEAD_SLOT
            inner_w -= self._LEAD_SLOT

        if self.trailing:
            ctx.text(x + w - self._PAD_H, y + h / 2.0, self.trailing,
                     size=TEXT_HINT, color=theme.muted, align="right_center")
            inner_w -= self._TRAIL_SLOT

        title_color = theme.accent if self.selected else theme.fg
        if self.subtitle:
            ctx.text(inner_x, y + h * 0.35, self.title,
                     size=TEXT_BODY, color=title_color, bold=True,
                     align="left_center", max_width=inner_w, elide=True)
            ctx.text(inner_x, y + h * 0.70, self.subtitle,
                     size=TEXT_HINT, color=theme.muted,
                     align="left_center", max_width=inner_w, elide=True)
        else:
            ctx.text(inner_x, y + h / 2.0, self.title,
                     size=TEXT_BODY, color=title_color, bold=True,
                     align="left_center", max_width=inner_w, elide=True)


@dataclass
class Row(Component):
    """Horizontal row: optional leading icon, main label, optional trailing text.

    Vertically centres all items automatically. Use instead of paired
    ``ctx.text(x, y + h/2, ..., align="left_center")`` calls when building
    info rows with an icon, label, and badge or chevron.

    Example::

        Row(label="Workspace", leading="⚡", trailing=f"{count}")
    """
    label: str
    leading: Optional[str] = None           # icon / short text, left slot
    trailing: Optional[str] = None          # badge / chevron, right slot
    font_size: float = TEXT_BODY
    color: "str | None" = None
    leading_color: Optional[str] = None     # default: theme.muted
    trailing_color: Optional[str] = None    # default: theme.muted
    height: Optional[float] = None          # default: font_size + SPACE_MD
    bold: bool = False

    _LEAD_SLOT = SPACE_XL
    _TRAIL_SLOT = SPACE_XL

    def _h(self) -> float:
        return self.height if self.height is not None else self.font_size + SPACE_MD

    def measure(self, avail_w: float) -> float:
        return self._h()

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        yc = y + h / 2.0
        inner_x = x
        inner_w = w

        if self.leading:
            ctx.text(inner_x, yc, self.leading,
                     size=self.font_size,
                     color=self.leading_color or theme.muted,
                     align="left_center")
            inner_x += self._LEAD_SLOT
            inner_w -= self._LEAD_SLOT

        if self.trailing:
            ctx.text(x + w, yc, self.trailing,
                     size=self.font_size,
                     color=self.trailing_color or theme.muted,
                     align="right_center")
            inner_w -= self._TRAIL_SLOT

        ctx.text(inner_x, yc, self.label,
                 size=self.font_size, color=self.color or theme.fg, bold=self.bold,
                 align="left_center", max_width=inner_w, elide=True)


# ── Badge primitive ────────────────────────────────────────────────────────


def badge(
    ctx,
    x: float,
    y_center: float,
    label: str,
    fill: "str | None" = None,
    fg: "str | None" = None,
    font_size: float = TEXT_HINT,
    radius: float = RADIUS_BADGE,
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
        fg:        Text colour (default: theme bg — dark text on light pill).
        font_size: Label pt size (default ``TEXT_HINT``).
        radius:    Corner radius. Use ``RADIUS_SM`` (4 px) for tag chips,
                   ``RADIUS_BADGE`` (6 px, default) for rounded badges without
                   the perfect-stadium look of ``RADIUS_MD`` (8 px).
    """
    ctx.badge(x=x, y_center=y_center, label=label,
              fill=fill or theme.accent, fg=fg or theme.bg,
              font_size=font_size, radius=radius)


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
              fill=theme.highlight, fg=theme.fg, font_size=TEXT_HINT,
              radius=RADIUS_SM)
    # Approx width — not measured here because we don't need it for
    # placement (callers anchor by top-right of the parent region).
    return len(text) * TEXT_HINT * 0.62 + 16.0


# ── Container components ───────────────────────────────────────────────────


@dataclass
class Card(Component):
    """Surface-colored container with inner padding. Stacks its children
    vertically with a configurable gap. A 1px border in `theme.highlight` separates
    it from the pane background — essential when surface and bg are close
    in brightness."""
    children: List[Component]
    padding: float = SPACE_LG
    gap: float = SPACE_XS
    background: "str | None" = None
    border: Optional[str] = "__theme__"
    radius: float = RADIUS_MD

    def __post_init__(self):
        self.children = list(self.children)
        for child in self.children:
            if not isinstance(child, Component):
                raise TypeError(
                    f"Card children must subclass Component, got {type(child).__name__}. "
                    "Ad-hoc widget classes missing _render_clipped will crash at render time."
                )

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
        ctx.rect(x, y, w, h, self.background or theme.surface, radius=self.radius)
        border_color = theme.highlight if self.border == "__theme__" else self.border
        if border_color:
            # Top + bottom + left + right 1px strokes. Drawn as four thin
            # rects because `ctx.rect` doesn't support a separate stroke.
            ctx.rect(x, y, w, 1.0, border_color)
            ctx.rect(x, y + h - 1.0, w, 1.0, border_color)
            ctx.rect(x, y, 1.0, h, border_color)
            ctx.rect(x + w - 1.0, y, 1.0, h, border_color)
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
class TextInput(Component):
    """Layout-aware text input. Place inside a Column like any other child.

    After ``ctx.render(column)``, read ``.submitted`` to get the text the user
    submitted (pressed Enter), or ``None`` if nothing was submitted this frame.

    Create once (in ``on_init``) and update ``placeholder`` as needed — the
    instance is stable across renders so the host can track focus state.

    When ``multiline=True``, Shift+Enter inserts a newline and Enter submits.
    """
    id: str
    placeholder: str = ""
    height: float = 48.0
    multiline: bool = False

    _submitted: Optional[str] = field(default=None, init=False, repr=False)

    def measure(self, avail_w: float) -> float:
        return self.height

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        self._submitted = ctx.text_input(self.id, x=x, y=y, w=w,
                                         placeholder=self.placeholder, h=h,
                                         multiline=self.multiline)

    @property
    def submitted(self) -> Optional[str]:
        """Text submitted this frame (user pressed Enter), else None."""
        return self._submitted


@dataclass
class ChatBubble(Component):
    """A chat message bubble with left/right alignment and colored background.

    ``align="right"`` for user messages (accent bg), ``"left"`` for
    assistant messages (surface bg). Error messages use ``role="error"``.
    """
    text: str
    role: str = "assistant"
    max_lines: int = 50

    LINE_LEADING = 5.0
    DESCENDER_PAD = 5.0
    BUBBLE_PAD = SPACE_MD
    BUBBLE_MAX_FRAC = 0.78
    BUBBLE_MIN_W = 38.0

    def _font_size(self) -> float:
        return TEXT_BODY

    def _bubble_colors(self) -> tuple:
        if self.role == "user":
            return (theme.accent, theme.bg)
        if self.role == "error":
            # Error background derived from theme.danger with darkening — no direct theme role.
            return ("#45171e", theme.danger)
        return (theme.surface, theme.fg)

    def _plain_text(self) -> str:
        text = self.text
        for marker in ("**", "__", "`"):
            text = text.replace(marker, "")
        return text

    def _natural_text_w(self) -> float:
        char_w = self._font_size() * 0.55
        lines = self._plain_text().splitlines() or [self._plain_text()]
        longest = max((len(line.strip()) for line in lines), default=0)
        return longest * char_w

    def _bubble_w(self, avail_w: float) -> float:
        max_w = max(self.BUBBLE_MIN_W, avail_w * self.BUBBLE_MAX_FRAC)
        natural_w = self._natural_text_w() + 2 * self.BUBBLE_PAD
        return min(avail_w, max(self.BUBBLE_MIN_W, min(max_w, natural_w)))

    def _text_w(self, avail_w: float) -> float:
        return self._bubble_w(avail_w) - 2 * self.BUBBLE_PAD

    def _lines(self, avail_w: float) -> List[str]:
        return _wrap_to_width(self.text, self._text_w(avail_w),
                              self._font_size(), max_lines=self.max_lines)

    def _line_h(self) -> float:
        return self._font_size() + self.LINE_LEADING

    def measure(self, avail_w: float) -> float:
        text_w = self._text_w(avail_w)
        line_count = _markdown_measure_lines(
            self.text, text_w, self._font_size(), self.max_lines
        )
        if line_count <= 0:
            return 0.0
        text_h = line_count * self._line_h() - self.LINE_LEADING + self.DESCENDER_PAD
        # egui_commonmark adds small margins around paragraphs and list blocks.
        return text_h + 2 * self.BUBBLE_PAD + SPACE_XS

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        bg, fg = self._bubble_colors()
        bubble_w = self._bubble_w(w)
        bx = x + w - bubble_w if self.role == "user" else x
        ctx.rect(bx, y, bubble_w, h, fill=bg, radius=RADIUS_LG)
        fs = self._font_size()
        text_x = bx + self.BUBBLE_PAD
        text_y = y + self.BUBBLE_PAD
        text_w = bubble_w - 2 * self.BUBBLE_PAD
        ctx.markdown(text_x, text_y, text_w, self.text, base_size=fs, color=fg)


class SelectList(Component):
    """Keyboard-navigable scrollable list. Stateful — create in on_init, not on_render.

    items: list of dicts with keys: name (str), description (str, optional),
           leading (str, optional), trailing (str, optional)
    selected_idx: currently highlighted row index

    Call handle_key(key) from on_key. Call hit_index(click_y) from on_click.
    """

    def __init__(self, items: List[dict], selected_idx: int = 0) -> None:
        self.items = items
        self.selected_idx = selected_idx
        self._scroll_px: float = 0.0
        self._viewport_h: float = 0.0
        self._rendered_rects: List[tuple] = []  # (y_top, y_bot, idx) populated each render

    def is_grow(self) -> bool:
        return True

    def measure(self, avail_w: float) -> float:
        return 0.0  # is_grow — allocated by parent

    def _item_h(self, i: int) -> float:
        item = self.items[i]
        return ListItem.HEIGHT_DOUBLE if item.get("description") else ListItem.HEIGHT_SINGLE

    def _item_top(self, idx: int) -> float:
        """Item's y position in content-local space (before scroll)."""
        y = 0.0
        for i in range(idx):
            y += self._item_h(i) + SPACE_XS
        return y

    def _total_content_h(self) -> float:
        if not self.items:
            return 0.0
        return sum(self._item_h(i) for i in range(len(self.items))) + SPACE_XS * (len(self.items) - 1)

    def _clamp_scroll(self) -> None:
        max_scroll = max(0.0, self._total_content_h() - self._viewport_h)
        self._scroll_px = max(0.0, min(self._scroll_px, max_scroll))

    def handle_key(self, key: str) -> bool:
        """Update selection and scroll for j/k/arrows. Returns True if consumed."""
        total = len(self.items)
        if total == 0:
            return False
        if key in ("j", "ArrowDown", "down"):
            self.selected_idx = min(self.selected_idx + 1, total - 1)
        elif key in ("k", "ArrowUp", "up"):
            self.selected_idx = max(self.selected_idx - 1, 0)
        else:
            return False
        # Scroll to keep selected item visible
        item_top = self._item_top(self.selected_idx)
        item_bot = item_top + self._item_h(self.selected_idx)
        self._scroll_px = ensure_visible(self._scroll_px, self._viewport_h, item_top, item_bot)
        self._clamp_scroll()
        return True

    def hit_index(self, click_y: float) -> Optional[int]:
        """Return item index at click_y (screen coords from last render), or None."""
        for (yt, yb, idx) in self._rendered_rects:
            if yt <= click_y < yb:
                return idx
        return None

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        self._viewport_h = h
        self._rendered_rects = []
        self._clamp_scroll()

        if not self.items:
            ctx.text(x + w / 2, y + h / 2, "No items",
                     size=TEXT_HINT, color=theme.muted, align="center")
            return

        ctx.push_clip(x, y, w, h)
        try:
            cursor_y = y - self._scroll_px
            for i, item in enumerate(self.items):
                ih = self._item_h(i)
                yt = cursor_y
                yb = cursor_y + ih
                if yb > y and yt < y + h:
                    self._rendered_rects.append((yt, yb, i))
                    li = ListItem(
                        title=item["name"],
                        subtitle=item.get("description") or None,
                        leading=item.get("leading"),
                        trailing=item.get("trailing"),
                        selected=(i == self.selected_idx),
                    )
                    li.render(ctx, x, cursor_y, w, ih)
                cursor_y += ih + SPACE_XS
        finally:
            ctx.pop_clip()

        # Scrollbar indicator when content overflows
        total_h = self._total_content_h()
        if total_h > h and h > 0:
            sb_w = 3.0
            thumb_ratio = h / total_h
            thumb_h = max(16.0, h * thumb_ratio)
            thumb_y = y + (self._scroll_px / total_h) * h
            thumb_y = min(thumb_y, y + h - thumb_h)
            bar_x = x + w - sb_w
            ctx.rect(bar_x, y, sb_w, h, theme.highlight)
            ctx.rect(bar_x, thumb_y, sb_w, thumb_h, theme.muted)


@dataclass
class FormField(Component):
    """Label + TextInput row. Create in on_init (stable across renders).

    Read .submitted after ctx.render() — it contains the text entered
    by the user when they pressed Enter, or None if no submission this frame.
    """
    id: str
    label: str
    placeholder: str = ""
    required: bool = False
    height: float = 48.0

    LABEL_H: float = TEXT_HINT + SPACE_XS  # 11 + 4 = 15px
    LABEL_GAP: float = SPACE_SM            # 8px gap between label and input
    BOTTOM_PAD: float = SPACE_LG          # 16px below input before next item

    def __post_init__(self) -> None:
        self._input: TextInput = TextInput(self.id, placeholder=self.placeholder, height=self.height)

    @property
    def submitted(self) -> Optional[str]:
        """Text submitted this frame (Enter pressed), or None."""
        return self._input.submitted

    def measure(self, avail_w: float) -> float:
        return self.LABEL_H + self.LABEL_GAP + self.height + self.BOTTOM_PAD

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        req_suffix = " *" if self.required else ""
        ctx.text(x, y, f"{self.label}{req_suffix}",
                 size=TEXT_HINT, color=theme.muted)
        input_y = y + self.LABEL_H + self.LABEL_GAP
        self._input.render(ctx, x, input_y, w, self.height)


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

    def __post_init__(self):
        self.children = list(self.children)
        for child in self.children:
            if not isinstance(child, Component):
                raise TypeError(
                    f"Column children must subclass Component, got {type(child).__name__}. "
                    "Ad-hoc widget classes missing _render_clipped will crash at render time."
                )

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


def render_tree(ctx, root: Component, fill: Optional[str] = None) -> None:
    """Clear the pane to `fill`, then render `root` into the full pane rect.

    `fill` defaults to the active host theme background (`theme.bg`).
    Apps normally call `ctx.render(root)` instead, which calls this.
    """
    ctx.clear(fill or theme.bg)
    root.render(ctx, 0.0, 0.0, ctx.w, ctx.h)


@dataclass
class InfoTable(Component):
    """Key-value table with surface background, border, and row dividers.

    Each row is a ``(key, value)`` tuple rendered in a fixed-width key column
    (monospace, green accent) and a value column (monospace, FG).

    Example::

        InfoTable([
            ("app_id", "my-app"),
            ("workspace", "/path/to/ws"),
        ])
    """
    rows: List[tuple]  # list of (key_label, value_text)
    key_width: float = 100.0
    background: Optional[str] = None  # default: theme.surface
    border: Optional[str] = None      # default: theme.highlight
    radius: float = RADIUS_MD

    ROW_H = 30.0
    PAD_H = SPACE_MD

    def measure(self, avail_w: float) -> float:
        if not self.rows:
            return 0.0
        return self.ROW_H * len(self.rows)

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        # Background + border
        background = self.background or theme.surface
        border = self.border or theme.highlight
        ctx.rect(x, y, w, h, background, radius=self.radius)
        ctx.rect(x, y, w, 1.0, border)
        ctx.rect(x, y + h - 1.0, w, 1.0, border)
        ctx.rect(x, y, 1.0, h, border)
        ctx.rect(x + w - 1.0, y, 1.0, h, border)

        val_x = x + self.PAD_H + self.key_width + self.PAD_H
        val_w = w - self.PAD_H - self.key_width - self.PAD_H * 2

        for i, (key, value) in enumerate(self.rows):
            row_y = y + i * self.ROW_H
            cy = row_y + self.ROW_H / 2.0

            # Key (green, monospace)
            ctx.text(x + self.PAD_H, cy, str(key),
                     size=TEXT_CAPTION, color=theme.success, monospace=True,
                     align="left_center", max_width=self.key_width, elide=True)

            # Value (FG, monospace)
            ctx.text(val_x, cy, str(value),
                     size=TEXT_CAPTION, color=theme.fg, monospace=True,
                     align="left_center", max_width=val_w, elide=True)

            # Row divider (skip last row)
            if i < len(self.rows) - 1:
                div_y = row_y + self.ROW_H
                ctx.rect(x + 1.0, div_y, w - 2.0, 1.0, theme.bg)


@dataclass
class ButtonRow(Component):
    """A clickable button rendered as a component in the declarative tree.

    Delegates to ``ctx.button()`` for host-managed hover/click state.
    After ``ctx.render(column)``, check ``.clicked`` to see if the button
    was pressed this frame.

    Example::

        self._btn = ButtonRow("action", "Click me")

        def on_render(self, ctx):
            ctx.render(Column([self._btn]))
            if self._btn.clicked:
                handle_click()
    """
    id: str
    label: str
    text_color: "str | None" = None
    fill: "str | None" = None
    hover_fill: "str | None" = None
    active_fill: "str | None" = None
    font_size: float = TEXT_BODY
    radius: float = RADIUS_MD
    height: float = 36.0
    clicked: bool = field(default=False, init=False, repr=False)

    def measure(self, avail_w: float) -> float:
        return self.height

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        self.clicked = ctx.button(
            id=self.id,
            x=x, y=y, w=w, h=h,
            label=self.label,
            fill=self.fill or theme.surface,
            hover_fill=self.hover_fill or theme.highlight,
            active_fill=self.active_fill or theme.text_section,
            text_color=self.text_color or theme.accent,
            font_size=self.font_size,
            radius=self.radius,
        )


# ── ListView row helpers ───────────────────────────────────────────────────────

@dataclass
class LeadingBadge:
    """Badge leading slot for :class:`ListRow`.

    Renders a pill badge with ``label`` text and the given ``color``.
    """
    label: str
    color: str = "accent"

    def to_dict(self) -> dict:
        return {"variant": "badge", "label": self.label, "color": self.color}


@dataclass
class LeadingAvatar:
    """Circular avatar leading slot for :class:`ListRow`.

    ``handle`` must be a UUID returned by ``emit.load_image(url)``.
    """
    handle: str

    def to_dict(self) -> dict:
        return {"variant": "avatar", "handle": self.handle}


@dataclass
class LeadingIcon:
    """Text/emoji icon leading slot for :class:`ListRow`."""
    name: str

    def to_dict(self) -> dict:
        return {"variant": "icon", "name": self.name}


@dataclass
class RowChip:
    """A small colored chip label on a :class:`ListRow`."""
    label: str
    color: str = "accent"

    def to_dict(self) -> dict:
        return {"label": self.label, "color": self.color}


@dataclass
class ListRow:
    """Typed row descriptor for :meth:`RenderContext.list_view`.

    Example::

        rows = [
            ListRow(
                id=f"issue-{issue['number']}",
                leading=LeadingBadge(f"#{issue['number']}", color="accent"),
                primary=issue["title"],
                chips=[RowChip(lbl["name"], _label_color(lbl["name"])) for lbl in issue["labels"][:2]],
            ).to_dict()
            for issue in self._issues
        ]
        ctx.list_view("issues", rows, selected=self._sel, y=float(HEADER_H))
    """
    id: str
    primary: str
    leading: "LeadingBadge | LeadingAvatar | LeadingIcon | None" = None
    secondary: "str | None" = None
    chips: "list[RowChip]" = field(default_factory=list)
    trailing: "str | None" = None

    def to_dict(self) -> dict:
        return {
            "type": "row",
            "id": self.id,
            "leading": self.leading.to_dict() if self.leading else {"variant": "none"},
            "primary": self.primary,
            "secondary": self.secondary,
            "chips": [c.to_dict() for c in self.chips],
            "trailing": self.trailing,
        }


__all__ = [
    # tokens
    "SPACE_XS", "SPACE_SM", "SPACE_MD", "SPACE_LG", "SPACE_XL",
    "TEXT_HINT", "TEXT_CAPTION", "TEXT_BODY", "TEXT_HEADING",
    "TEXT_TITLE", "TEXT_TITLE_XL",
    "RADIUS_SM", "RADIUS_MD", "RADIUS_LG", "RADIUS_BADGE",
    # components
    "Component", "Column", "Card",
    "AppBar", "Section", "KeyRow", "Heading", "Label",
    "Spacer", "Divider", "ScrollLog", "Scrollable", "Footer", "FooterKeys",
    "ListItem", "Row", "TextInput", "ChatBubble",
    "SelectList", "FormField",
    "InfoTable", "ButtonRow",
    # badge primitive
    "badge",
    # scroll helpers
    "ensure_visible",
    # entry
    "render_tree",
    # ListView row helpers
    "ListRow", "RowChip", "LeadingBadge", "LeadingAvatar", "LeadingIcon",
]
