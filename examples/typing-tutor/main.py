#!/usr/bin/env python3
"""Typing Tutor — 10 progressive levels with home-row drills and star-gated unlocks."""

import time
from enum import Enum

from plexi_sdk import App, RenderContext, BG, FG, ACCENT, RED, GREEN, MUTED, SURFACE, HIGHLIGHT
from plexi_sdk import BODY, CAPTION, HINT, TITLE, HEADING

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
        if pct > threshold:
            return stars
    return 1


def _total_stars(level_stars: list[int]) -> int:
    return sum(level_stars)


def _is_unlocked(level_idx: int, level_stars: list[int]) -> bool:
    needed = LEVELS[level_idx].stars_needed
    return _total_stars(level_stars) >= needed


# Map shift+key to the typed character
_SHIFT_MAP: dict[str, str] = {
    "1": "!", "2": "@", "3": "#", "4": "$", "5": "%",
    "6": "^", "7": "&", "8": "*", "9": "(", "0": ")",
    "-": "_", "=": "+", "[": "{", "]": "}", "\\": "|",
    ";": ":", "'": '"', ",": "<", ".": ">", "/": "?",
    "`": "~",
}


def _key_to_char(key: str, shift: bool) -> str | None:
    """Convert a key event to the character it types, or None if not typeable."""
    if len(key) == 1:
        if shift:
            return _SHIFT_MAP.get(key, key.upper())
        return key
    if key == "space":
        return " "
    return None


class TypingTutorApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._screen = Screen.LEVEL_SELECT
        self._selected = 0  # level selector index (0-based)
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

        # Play state (initialised in _start_level)
        self._play_level: int = 0
        self._typed: int = 0          # current cursor position
        self._errors: set[int] = set()  # positions that had an error
        self._start_time: float = 0.0
        self._finished: bool = False

        # Result state
        self._result_stars: int = 0
        self._result_accuracy: float = 0.0
        self._result_time_taken: float = 0.0

    # ── Level selector ──────────────────────────────────────────────────────

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
        total_typed = len(level.text) if timed_out else self._typed
        errors = len(self._errors)
        stars = 0 if timed_out and errors >= int(_MAX_ERROR_RATE * max(total_typed, 1)) else \
            _calc_stars(errors, total_typed, time_remaining, level.time_sec)

        accuracy = (1.0 - errors / max(total_typed, 1)) * 100.0
        self._result_stars = stars
        self._result_accuracy = accuracy
        self._result_time_taken = elapsed
        self._finished = True
        self._screen = Screen.RESULT

        # Save best-of
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

    # ── Key handling ────────────────────────────────────────────────────────

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
        elif key in ("return", "space", "Enter"):
            if _is_unlocked(self._selected, self._level_stars):
                self._start_level(self._selected)

    def _key_play(self, ctx: RenderContext, key: str, shift: bool) -> None:
        level = LEVELS[self._play_level]
        text = level.text

        if key == "escape":
            self.emit.info(f"typing-tutor: aborted level={self._play_level + 1}")
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
        if key in ("return", "Enter", "space"):
            self._screen = Screen.LEVEL_SELECT
        elif key == "r":
            self._start_level(self._play_level)

    # ── Rendering ───────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        ctx.rect(0, 0, ctx.w, ctx.h, fill=BG)

        if self._screen == Screen.LEVEL_SELECT:
            self._render_select(ctx)
        elif self._screen == Screen.PLAYING:
            self._render_play(ctx)
        elif self._screen == Screen.RESULT:
            self._render_result(ctx)

    def _render_select(self, ctx: RenderContext) -> None:
        pad = 20.0
        title_h = 52.0

        ctx.text(pad, pad, "Typing Tutor", size=TITLE, color=FG, bold=True)
        total = _total_stars(self._level_stars)
        ctx.text(pad, pad + 28, f"Total stars: {total}/50",
                 size=CAPTION, color=MUTED)

        # 2×5 grid
        cols = 5
        rows = 2
        grid_pad = pad
        card_w = (ctx.w - grid_pad * 2 - (cols - 1) * 12) / cols
        card_h = (ctx.h - title_h - grid_pad * 2 - (rows - 1) * 12) / rows

        for i, level in enumerate(LEVELS):
            col = i % cols
            row = i // cols
            x = grid_pad + col * (card_w + 12)
            y = title_h + grid_pad + row * (card_h + 12)

            unlocked = _is_unlocked(i, self._level_stars)
            selected = i == self._selected
            stars = self._level_stars[i]

            # Card background
            if selected:
                bg = ACCENT + "33"
                border = ACCENT
            elif unlocked:
                bg = SURFACE
                border = HIGHLIGHT
            else:
                bg = BG
                border = MUTED + "55"

            ctx.rect(x, y, card_w, card_h, fill=bg, radius=8.0)
            # Border
            bw = 2.0 if selected else 1.0
            ctx.rect(x, y, card_w, bw, fill=border)
            ctx.rect(x, y + card_h - bw, card_w, bw, fill=border)
            ctx.rect(x, y, bw, card_h, fill=border)
            ctx.rect(x + card_w - bw, y, bw, card_h, fill=border)

            text_color = FG if unlocked else MUTED
            ctx.text(x + 10, y + 10, f"{i + 1}", size=BODY, color=ACCENT if selected else text_color, bold=True)
            ctx.text(x + 10, y + 30, level.name, size=HINT, color=text_color)

            # Stars
            star_str = "★" * stars + "☆" * (5 - stars)
            ctx.text(x + 10, y + card_h - 22, star_str, size=HINT,
                     color=ACCENT if stars > 0 else MUTED)

            # Lock overlay
            if not unlocked:
                ctx.rect(x, y, card_w, card_h, fill="#00000066", radius=8.0)
                needed = level.stars_needed
                ctx.text(x + card_w / 2 - 18, y + card_h / 2 - 10,
                         f"🔒 Need {needed}★", size=HINT, color=MUTED)

        # Footer hints
        ctx.text(pad, ctx.h - 18,
                 "h/← l/→ k/↑ j/↓ navigate   Enter/Space launch",
                 size=HINT, color=MUTED)

    def _render_play(self, ctx: RenderContext) -> None:
        level = LEVELS[self._play_level]
        text = level.text
        now = time.time()
        elapsed = now - self._start_time
        time_remaining = max(0.0, level.time_sec - elapsed)

        # Check timer
        if time_remaining <= 0 and not self._finished:
            self._go_result(ctx, timed_out=True)
            return

        # --- Stats bar ---
        pad = 16.0
        bar_h = 40.0

        # Live star preview
        live_stars = _calc_stars(
            len(self._errors), max(self._typed, 1), time_remaining, level.time_sec
        )
        star_str = "★" * live_stars + "☆" * (5 - live_stars)

        ctx.rect(0, 0, ctx.w, bar_h, fill=SURFACE)
        ctx.text(pad, 12, f"Level {level.id} — {level.name}", size=CAPTION, color=FG)
        ctx.text(ctx.w / 2 - 30, 12, star_str, size=CAPTION, color=ACCENT)
        ctx.text(ctx.w - 120, 12, f"Errors: {len(self._errors)}/{max(self._typed, 1)}",
                 size=CAPTION, color=RED if self._errors else MUTED)

        # --- Character grid ---
        char_size = 16.0
        cell_w = char_size * 0.72
        cell_h = char_size + 6

        chars_per_row = max(1, int((ctx.w - pad * 2) / cell_w))
        grid_top = bar_h + 20.0

        for i, ch in enumerate(text):
            row = i // chars_per_row
            col = i % chars_per_row
            cx = pad + col * cell_w
            cy = grid_top + row * cell_h

            if i < self._typed:
                if i in self._errors:
                    # Error: red fill, original char
                    ctx.rect(cx - 1, cy - 1, cell_w, cell_h, fill=RED + "44", radius=2.0)
                    ctx.text(cx, cy, ch, size=char_size, color=RED, monospace=True)
                else:
                    # Correct: green text
                    ctx.text(cx, cy, ch, size=char_size, color=GREEN, monospace=True)
            elif i == self._typed:
                # Cursor
                ctx.rect(cx - 1, cy - 1, cell_w, cell_h, fill=ACCENT + "55", radius=2.0)
                ctx.text(cx, cy, ch, size=char_size, color=FG, monospace=True)
            else:
                # Remaining
                ctx.text(cx, cy, ch, size=char_size, color=MUTED, monospace=True)

        # --- Progress + timer bar ---
        prog_y = ctx.h - 30
        prog_w = ctx.w - pad * 2
        prog_h = 8.0

        ctx.rect(pad, prog_y, prog_w, prog_h, fill=SURFACE, radius=4.0)
        progress = self._typed / max(len(text), 1)
        ctx.rect(pad, prog_y, prog_w * progress, prog_h, fill=GREEN, radius=4.0)

        timer_color = RED if time_remaining < 10 else FG
        ctx.text(pad, prog_y - 18, f"Time: {time_remaining:.0f}s", size=CAPTION, color=timer_color)
        ctx.text(ctx.w - 80, prog_y - 18, "ESC=quit", size=HINT, color=MUTED)

        # Drive timer updates
        ctx.emit.schedule_render(16)

    def _render_result(self, ctx: RenderContext) -> None:
        level = LEVELS[self._play_level]
        stars = self._result_stars
        accuracy = self._result_accuracy
        elapsed = self._result_time_taken

        # Card
        cw, ch = 340.0, 220.0
        cx = (ctx.w - cw) / 2
        cy = (ctx.h - ch) / 2

        ctx.rect(cx, cy, cw, ch, fill=SURFACE, radius=12.0)
        ctx.rect(cx, cy, cw, 2.0, fill=ACCENT)  # top accent bar

        ctx.text(cx + 20, cy + 20, f"Level {level.id} — {level.name}", size=CAPTION, color=MUTED)
        ctx.text(cx + 20, cy + 44, "★" * stars + "☆" * (5 - stars),
                 size=HEADING, color=ACCENT if stars > 0 else MUTED)

        ctx.text(cx + 20, cy + 90, f"Accuracy:  {accuracy:.1f}%", size=BODY, color=FG)
        ctx.text(cx + 20, cy + 118, f"Time:      {elapsed:.1f}s / {level.time_sec}s",
                 size=BODY, color=FG)

        if stars == 0:
            msg = "Keep practicing — error rate too high"
            color = RED
        elif stars == 5:
            msg = "Perfect run!"
            color = GREEN
        else:
            msg = "Nice work!"
            color = ACCENT

        ctx.text(cx + 20, cy + 152, msg, size=CAPTION, color=color)
        ctx.text(cx + 20, cy + 182, "Enter/Space → menu   r → retry", size=HINT, color=MUTED)


if __name__ == "__main__":
    TypingTutorApp().run()
