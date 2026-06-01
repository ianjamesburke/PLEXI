#!/usr/bin/env python3
"""Typing Tutor — 10 progressive levels with home-row drills and star-gated unlocks."""

import time
from enum import Enum

from plexi_sdk import App, RenderContext, dim
from plexi_sdk.ui import (
    Column, AppBar, Card, Label, Spacer, FooterKeys, Component,
    SPACE_SM, SPACE_XS,
    TEXT_HINT, TEXT_CAPTION, TEXT_BODY,
    RADIUS_MD,
)

from levels import LEVELS


class Screen(Enum):
    LEVEL_SELECT = "level_select"
    PLAYING = "playing"
    RESULT = "result"


# Scoring thresholds: (min_time_pct_remaining, stars)
_STAR_THRESHOLDS = [
    (0.70, 5),
    (0.50, 4),
    (0.30, 3),
    (0.10, 2),
    (0.00, 1),
]

_MAX_ERROR_RATE = 0.10  # >= 10% errors → 0 stars


def _calc_stars(errors: int, total: int, time_remaining: float, time_limit: float) -> int:
    if total == 0:
        return 0
    error_rate = errors / total
    if error_rate >= _MAX_ERROR_RATE:
        return 0
    pct = time_remaining / time_limit if time_limit > 0 else 0
    for threshold, stars in _STAR_THRESHOLDS:
        if pct >= threshold:
            return stars
    return 1


def _total_stars(level_stars: list[int]) -> int:
    return sum(level_stars)


def _is_unlocked(level_idx: int, level_stars: list[int]) -> bool:
    needed = LEVELS[level_idx].stars_needed
    return _total_stars(level_stars) >= needed


_SHIFT_MAP: dict[str, str] = {
    "1": "!", "2": "@", "3": "#", "4": "$", "5": "%",
    "6": "^", "7": "&", "8": "*", "9": "(", "0": ")",
    "-": "_", "=": "+", "[": "{", "]": "}", "\\": "|",
    ";": ":", "'": '"', ",": "<", ".": ">", "/": "?",
    "`": "~",
}


def _key_to_char(key: str, shift: bool) -> str | None:
    if len(key) == 1:
        if shift:
            return _SHIFT_MAP.get(key, key.upper())
        return key
    if key == "space":
        return " "
    return None


# ── Custom components ────────────────────────────────────────────────────────


class _LevelGrid(Component):
    """2×5 grid of level cards with selection highlight and lock overlays."""

    def __init__(self, app: "TypingTutorApp") -> None:
        self._app = app

    def measure(self, avail_w: float) -> float:
        return 0.0

    def is_grow(self) -> bool:
        return True

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        app = self._app
        cols = 5
        rows = (len(LEVELS) + cols - 1) // cols
        gap = 12.0
        card_w = (w - (cols - 1) * gap) / cols
        card_h = (h - (rows - 1) * gap) / rows

        for i, level in enumerate(LEVELS):
            col = i % cols
            row = i // cols
            cx = x + col * (card_w + gap)
            cy = y + row * (card_h + gap)

            unlocked = _is_unlocked(i, app._level_stars)
            selected = i == app._selected
            stars = app._level_stars[i]

            if selected:
                bg, border, bw = ctx.theme.surface, ctx.theme.accent, 2.0
            elif unlocked:
                bg, border, bw = ctx.theme.surface, ctx.theme.highlight, 1.0
            else:
                bg, border, bw = ctx.theme.bg, dim(ctx.theme.muted, 85), 1.0

            ctx.rect(cx, cy, card_w, card_h, fill=bg, radius=RADIUS_MD)
            ctx.rect(cx, cy, card_w, bw, fill=border)
            ctx.rect(cx, cy + card_h - bw, card_w, bw, fill=border)
            ctx.rect(cx, cy, bw, card_h, fill=border)
            ctx.rect(cx + card_w - bw, cy, bw, card_h, fill=border)

            text_color = ctx.theme.fg if unlocked else ctx.theme.muted
            num_color = ctx.theme.accent if selected else (ctx.theme.accent if unlocked else ctx.theme.muted)
            ctx.text(cx + 10, cy + 10, f"{i + 1}", size=TEXT_BODY,
                     color=num_color, bold=True)
            ctx.text(cx + 10, cy + 28, level.name, size=TEXT_HINT, color=text_color)
            ctx.text(cx + 10, cy + card_h - 20,
                     "★" * stars + "☆" * (5 - stars),
                     size=TEXT_HINT, color=ctx.theme.accent if stars > 0 else ctx.theme.muted)

            if not unlocked:
                ctx.rect(cx, cy, card_w, card_h, fill="#00000066", radius=RADIUS_MD)
                ctx.text(cx + card_w / 2, cy + card_h / 2,
                         f"🔒 Need {level.stars_needed}★", size=TEXT_HINT, color=ctx.theme.muted,
                         align="center")


class _CharGrid(Component):
    """Character-by-character typing feedback with per-char color coding."""

    def __init__(self, app: "TypingTutorApp") -> None:
        self._app = app

    def measure(self, avail_w: float) -> float:
        return 0.0

    def is_grow(self) -> bool:
        return True

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        app = self._app
        text = LEVELS[app._play_level].text
        char_size = 16.0
        cell_w = char_size * 0.72
        cell_h = char_size + 6
        chars_per_row = max(1, int((w - SPACE_SM * 2) / cell_w))
        ox = x + SPACE_SM
        oy = y + SPACE_SM

        for i, ch in enumerate(text):
            row = i // chars_per_row
            col = i % chars_per_row
            cx = ox + col * cell_w
            cy = oy + row * cell_h

            if i < app._typed:
                if i in app._errors:
                    ctx.rect(cx - 1, cy - 1, cell_w, cell_h, fill=dim(ctx.theme.danger, 68), radius=2.0)
                    ctx.text(cx, cy, ch, size=char_size, color=ctx.theme.danger, monospace=True)
                else:
                    ctx.text(cx, cy, ch, size=char_size, color=ctx.theme.success, monospace=True)
            elif i == app._typed:
                ctx.rect(cx - 1, cy - 1, cell_w, cell_h, fill=ctx.theme.accent, radius=2.0)
                ctx.text(cx, cy, ch, size=char_size, color=ctx.theme.bg, monospace=True)
            else:
                ctx.text(cx, cy, ch, size=char_size, color=ctx.theme.muted, monospace=True)


class _TimerBar(Component):
    """Progress bar + countdown timer pinned above the footer."""

    HEIGHT = 36.0

    def __init__(self, app: "TypingTutorApp") -> None:
        self._app = app

    def measure(self, avail_w: float) -> float:
        return self.HEIGHT

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        app = self._app
        level = LEVELS[app._play_level]
        elapsed = time.time() - app._start_time
        time_remaining = max(0.0, level.time_sec - elapsed)

        timer_color = ctx.theme.danger if time_remaining < 10 else ctx.theme.fg
        ctx.text(x + SPACE_SM, y + SPACE_XS,
                 f"Time: {time_remaining:.0f}s", size=TEXT_CAPTION, color=timer_color)

        bar_y = y + TEXT_CAPTION + SPACE_XS * 2
        bar_w = w - SPACE_SM * 2
        ctx.rect(x + SPACE_SM, bar_y, bar_w, 8.0, fill=ctx.theme.surface, radius=4.0)
        progress = app._typed / max(len(level.text), 1)
        ctx.rect(x + SPACE_SM, bar_y, bar_w * progress, 8.0, fill=ctx.theme.success, radius=4.0)
        ctx.schedule_render(100)


# ── App ──────────────────────────────────────────────────────────────────────


class TypingTutorApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._screen = Screen.LEVEL_SELECT
        self._selected = 0
        self._level_stars: list[int] = [0] * len(LEVELS)

        state = ctx.load_state()
        if state and "level_stars" in state:
            saved = state["level_stars"]
            for i, s in enumerate(saved[: len(self._level_stars)]):
                self._level_stars[i] = int(s)
            self.emit.info(
                f"typing-tutor: loaded state total_stars={_total_stars(self._level_stars)}"
            )
        else:
            self.emit.info("typing-tutor: no saved state, starting fresh")

        self.emit.info(f"typing-tutor: ready levels={len(LEVELS)}")

        self._play_level: int = 0
        self._typed: int = 0
        self._errors: set[int] = set()
        self._start_time: float = 0.0
        self._finished: bool = False

        self._result_stars: int = 0
        self._result_accuracy: float = 0.0
        self._result_time_taken: float = 0.0

    # ── Level management ──────────────────────────────────────────────────

    def _start_level(self, idx: int) -> None:
        level = LEVELS[idx]
        self.emit.info(
            f"typing-tutor: start level={idx + 1} name={level.name!r} len={len(level.text)}"
        )
        self._play_level = idx
        self._typed = 0
        self._errors = set()
        self._start_time = time.time()
        self._finished = False
        self._screen = Screen.PLAYING

    def _go_result(self, ctx: RenderContext, timed_out: bool = False) -> None:
        level = LEVELS[self._play_level]
        elapsed = time.time() - self._start_time
        time_remaining = max(0.0, level.time_sec - elapsed)
        errors = len(self._errors)
        stars = 0 if timed_out else _calc_stars(errors, self._typed, time_remaining, level.time_sec)

        accuracy = (1.0 - errors / max(self._typed, 1)) * 100.0
        self._result_stars = stars
        self._result_accuracy = accuracy
        self._result_time_taken = elapsed
        self._finished = True
        self._screen = Screen.RESULT

        if stars > self._level_stars[self._play_level]:
            self._level_stars[self._play_level] = stars
            ctx.save_state({"level_stars": self._level_stars})
            self.emit.info(
                f"typing-tutor: saved stars level={self._play_level + 1} stars={stars}"
            )

        self.emit.info(
            f"typing-tutor: result level={self._play_level + 1} stars={stars} "
            f"accuracy={accuracy:.1f}% elapsed={elapsed:.1f}s timed_out={timed_out}"
        )

    # ── Key handling ───────────────────────────────────────────────────────

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        shift = bool(mods.get("shift"))
        if self._screen == Screen.LEVEL_SELECT:
            self._key_select(key)
        elif self._screen == Screen.PLAYING:
            self._key_play(ctx, key, shift)
        elif self._screen == Screen.RESULT:
            self._key_result(key)

    def _key_select(self, key: str) -> None:
        n = len(LEVELS)
        if key in ("h", "left"):
            self._selected = max(0, self._selected - 1)
        elif key in ("l", "right"):
            self._selected = min(n - 1, self._selected + 1)
        elif key in ("k", "up"):
            self._selected = max(0, self._selected - 5)
        elif key in ("j", "down"):
            self._selected = min(n - 1, self._selected + 5)
        elif key in ("space", "return"):
            if _is_unlocked(self._selected, self._level_stars):
                self._start_level(self._selected)

    def _key_play(self, ctx: RenderContext, key: str, shift: bool) -> None:
        level = LEVELS[self._play_level]
        text = level.text

        if key == "escape":
            ctx.info(f"typing-tutor: aborted level={self._play_level + 1}")
            self._screen = Screen.LEVEL_SELECT
            return

        if key == "backspace":
            if self._typed > 0:
                self._typed -= 1
                self._errors.discard(self._typed)
            return

        char = _key_to_char(key, shift)
        if char is None:
            return

        if self._typed >= len(text):
            return

        expected = text[self._typed]
        if char != expected:
            self._errors.add(self._typed)
        self._typed += 1

        if self._typed >= len(text):
            self._go_result(ctx, timed_out=False)

    def _key_result(self, key: str) -> None:
        if key in ("space", "return"):
            self._screen = Screen.LEVEL_SELECT
        elif key == "r":
            self._start_level(self._play_level)

    # ── Rendering ──────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        if self._screen == Screen.LEVEL_SELECT:
            self._render_select(ctx)
        elif self._screen == Screen.PLAYING:
            self._render_play(ctx)
        elif self._screen == Screen.RESULT:
            self._render_result(ctx)

    def _render_select(self, ctx: RenderContext) -> None:
        total = _total_stars(self._level_stars)
        ctx.render(Column([
            AppBar(title="Typing Tutor", subtitle=f"Total stars: {total}/50"),
            _LevelGrid(self),
            FooterKeys([
                (["h", "←"], "prev"),
                (["l", "→"], "next"),
                (["k", "↑"], "back"),
                (["j", "↓"], "fwd"),
                ("Enter", "launch"),
            ]),
        ]))

    def _render_play(self, ctx: RenderContext) -> None:
        level = LEVELS[self._play_level]
        now = time.time()
        elapsed = now - self._start_time
        time_remaining = max(0.0, level.time_sec - elapsed)

        if time_remaining <= 0 and not self._finished:
            self._go_result(ctx, timed_out=True)
            return

        live_stars = (
            _calc_stars(len(self._errors), self._typed, time_remaining, level.time_sec)
            if self._typed > 0 else 0
        )
        star_str = "★" * live_stars + "☆" * (5 - live_stars)
        errors_str = f"Errors: {len(self._errors)}/{self._typed}"

        ctx.render(Column([
            AppBar(
                title=f"Level {level.id} — {level.name}",
                subtitle=f"{star_str}  {errors_str}",
            ),
            _CharGrid(self),
            _TimerBar(self),
            FooterKeys([("Esc", "quit")]),
        ]))

    def _render_result(self, ctx: RenderContext) -> None:
        level = LEVELS[self._play_level]
        stars = self._result_stars
        accuracy = self._result_accuracy
        elapsed = self._result_time_taken

        timed_out = elapsed >= level.time_sec
        if stars == 0:
            msg = "Time's up!" if timed_out else "Keep practicing — error rate too high"
            msg_color = ctx.theme.danger
        elif stars == 5:
            msg = "Perfect run!"
            msg_color = ctx.theme.success
        else:
            msg = "Nice work!"
            msg_color = ctx.theme.accent

        ctx.render(Column([
            AppBar(title=f"Level {level.id} — {level.name}"),
            Spacer(grow=True),
            Card([
                Label("★" * stars + "☆" * (5 - stars),
                      color=ctx.theme.accent if stars > 0 else ctx.theme.muted, bold=True),
                Label(f"Accuracy:  {accuracy:.1f}%"),
                Label(f"Time:      {elapsed:.1f}s / {level.time_sec}s"),
                Label(msg, color=msg_color),
            ]),
            Spacer(grow=True),
            FooterKeys([
                ("Enter", "menu"),
                ("r", "retry"),
            ]),
        ]))


if __name__ == "__main__":
    TypingTutorApp().run()
