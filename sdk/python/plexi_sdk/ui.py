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
from typing import List, Optional

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

# Approximate character-width ratios for width-based measurements. These are
# empirical, not exact — used for ellipsis/wrap decisions. Real rendering
# width is determined by the host's text shaper and may differ by a few px.
_CHAR_W_PROPORTIONAL = 0.55
_CHAR_W_MONO = 0.60


# ── Utilities ──────────────────────────────────────────────────────────────


def _char_px(font_size: float, mono: bool = False) -> float:
    """Approximate pixel width of a single character at `font_size`."""
    ratio = _CHAR_W_MONO if mono else _CHAR_W_PROPORTIONAL
    return font_size * ratio


def _truncate_to_width(text: str, avail_px: float, font_size: float,
                       mono: bool = False) -> str:
    """Shorten `text` with an ellipsis if it exceeds `avail_px`."""
    if avail_px <= 0 or not text:
        return ""
    char_w = _char_px(font_size, mono)
    max_chars = max(1, int(avail_px / char_w))
    if len(text) <= max_chars:
        return text
    if max_chars <= 1:
        return "…"  # just the ellipsis
    return text[: max_chars - 1] + "…"


def _wrap_to_width(text: str, avail_px: float, font_size: float,
                   mono: bool = False, max_lines: int = 3) -> List[str]:
    """Word-wrap `text` into up to `max_lines` lines. Final line gets an
    ellipsis if content was truncated."""
    if avail_px <= 0 or not text:
        return []
    char_w = _char_px(font_size, mono)
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
        text = _truncate_to_width(self.text, w, fs)
        ctx.text(x, y, text, size=fs, color=self.color, bold=self.bold)


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
class Header(Component):
    """Top-of-pane heading block: title + optional subtitle + divider.

    Vertical stack (top to bottom):
        title                       (TEXT_TITLE_XL tall)
        TITLE_TO_SUB_GAP (if subtitle)
        subtitle                    (TEXT_HINT tall)
        BEFORE_DIVIDER              (gap before the divider line)
        divider                     (1px)
    """
    title: str
    subtitle: Optional[str] = None
    accent: str = FG

    TITLE_TO_SUB_GAP = 6.0
    BEFORE_DIVIDER = 14.0

    def measure(self, avail_w: float) -> float:
        h = TEXT_TITLE_XL
        if self.subtitle:
            h += self.TITLE_TO_SUB_GAP + TEXT_HINT
        h += self.BEFORE_DIVIDER + 1.0
        return h

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        cursor = y
        title_text = _truncate_to_width(self.title, w, TEXT_TITLE_XL)
        ctx.text(x, cursor, title_text,
                 size=TEXT_TITLE_XL, color=self.accent, bold=True)
        cursor += TEXT_TITLE_XL
        if self.subtitle:
            cursor += self.TITLE_TO_SUB_GAP
            sub_text = _truncate_to_width(self.subtitle, w, TEXT_HINT)
            ctx.text(x, cursor, sub_text, size=TEXT_HINT, color=MUTED)
            cursor += TEXT_HINT
        cursor += self.BEFORE_DIVIDER
        ctx.rect(x, cursor, w, 1.0, HIGHLIGHT)


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
        title_text = _truncate_to_width(self.title.upper(), w, TEXT_HINT)
        ctx.text(x, label_y, title_text,
                 size=TEXT_HINT, color=MUTED, bold=True)
        line_y = label_y + TEXT_HINT + SPACE_XS
        ctx.rect(x, line_y, w, 1.0, HIGHLIGHT)


@dataclass
class KeyRow(Component):
    """A `[k]` key badge followed by a description, left-aligned. The badge
    uses a monospace glyph on an elevated background so single keys and
    chords (⌘K, ⎋) all look tidy. Description text is proportional.

    Layout math: the badge and description text are both vertically centered
    within the row's HEIGHT. Text is drawn with LEFT_TOP anchor, so we
    compute `top = y + (HEIGHT - font_size) / 2`.
    """
    key: str
    description: str
    badge_color: str = HIGHLIGHT
    key_color: str = ACCENT

    HEIGHT = 34.0
    BADGE_H = 26.0
    BADGE_W = 44.0  # wide enough for 2–3 glyph chords like "⌘K" / "Esc"

    def measure(self, avail_w: float) -> float:
        return self.HEIGHT

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        badge_y = y + (self.HEIGHT - self.BADGE_H) / 2.0
        ctx.rect(x, badge_y, self.BADGE_W, self.BADGE_H, self.badge_color,
                 radius=RADIUS_MD)
        # Both the badge glyph and the description use host-side anchors so
        # real font metrics drive the centering — no Python-side char-width
        # approximation that falls apart on narrow glyphs like `i`.
        glyph = self.key[:3]
        ctx.text(
            x + self.BADGE_W / 2.0,
            badge_y + self.BADGE_H / 2.0,
            glyph,
            size=TEXT_BODY, color=self.key_color,
            bold=True, monospace=True, align="center",
        )
        desc_x = x + self.BADGE_W + SPACE_MD
        desc_avail = w - (desc_x - x)
        desc_text = _truncate_to_width(self.description, desc_avail, TEXT_BODY)
        ctx.text(
            desc_x,
            y + self.HEIGHT / 2.0,
            desc_text,
            size=TEXT_BODY, color=FG,
            align="left_center",
        )


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
            truncated = _truncate_to_width(line, w, self.line_size, mono=True)
            ctx.text(x, y + i * line_h, truncated,
                     size=self.line_size, color=FG, monospace=True)


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
            child.render(ctx, inner_x, cursor, inner_w, ch)
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
            child.render(ctx, inner_x, cursor, inner_w, ch)
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
    "Header", "Section", "KeyRow", "Heading", "Label",
    "Spacer", "Divider", "ScrollLog", "Footer",
    # entry
    "render_tree",
]
