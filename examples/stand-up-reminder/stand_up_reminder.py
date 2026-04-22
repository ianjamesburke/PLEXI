#!/usr/bin/env python3
from __future__ import annotations

import random
import time

from plexi_sdk import (
    App, RenderContext,
    BG, SURFACE, HIGHLIGHT, ACCENT, MUTED, FG, GREEN, YELLOW,
    TITLE, HEADING, BODY, CAPTION, HINT,
    PAD, PAD_TIGHT, HEADER_H,
)

PROMPTS = [
    "Stand up and stretch your arms above your head",
    "Take a 2-minute walk around the room",
    "Do 10 desk push-ups against your chair or desk",
    "Roll your shoulders backward 10 times, then forward 10 times",
    "Look away from the screen and focus on something distant for 20 seconds (20-20-20 rule)",
    "Do 10 neck rolls — slowly tilt your head side to side",
    "Stand and do 10 calf raises",
    "Stretch your wrists: extend one arm, pull fingers back gently for 15 seconds each hand",
    "Do 5 slow deep breaths with your eyes closed",
    "Shake out your hands and arms for 30 seconds",
    "Do 10 seated leg raises — extend each leg and hold for 3 seconds",
    "Stand up and do 10 slow squats",
    "Arch your back gently, then round it — cat-cow stretch, 10 reps",
    "Reach across your chest and hold each arm stretch for 15 seconds",
    "Do 20 jumping jacks",
    "Walk to the kitchen and drink a full glass of water",
    "Do 10 shoulder shrugs — hold at the top for 2 seconds each",
    "Tilt your chin to your chest and hold for 20 seconds to stretch the back of your neck",
    "Stand and twist your torso left and right 10 times each side",
    "Do a 60-second wall sit",
    "Press your palms together at chest height and hold for 10 seconds to stretch forearms",
    "Do 10 slow head nods — look up then down, hold at each end",
    "March in place for 60 seconds, lifting your knees high",
    "Interlace your fingers and stretch your arms overhead, then lean side to side",
    "Do 15 seated bicycle crunches",
    "Stand up, touch your toes (or as close as you can), and hold for 20 seconds",
    "Do a 30-second plank",
    "Roll your ankles 10 times in each direction",
    "Sit on the edge of your chair, cross one leg over the other, and do a seated pigeon stretch for 20 seconds each side",
    "Do 10 chest openers — clasp hands behind your back and squeeze your shoulder blades",
    "Take a 3-minute walk outside if you can",
    "Do 15 wall push-ups",
    "Stretch your hip flexors: step forward into a lunge position and hold for 20 seconds each side",
]

INTERVALS = [
    (5 * 60, "5 min"),
    (15 * 60, "15 min"),
    (30 * 60, "30 min"),
    (60 * 60, "60 min"),
]

TIMER_ID = "standup"


def _format_duration(secs: float) -> str:
    secs = int(max(0, secs))
    h = secs // 3600
    m = (secs % 3600) // 60
    s = secs % 60
    if h > 0:
        return f"{h}:{m:02d}:{s:02d}"
    return f"{m:02d}:{s:02d}"


class StandUpReminderApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._interval_idx = 1  # default: 15 min
        self._reminders_today = 0
        self._last_prompt: str = ""
        self._next_fire_at: float = 0.0
        self._schedule_timer(ctx)
        ctx.status_summary("Stand Up Reminder — active")
        self.emit.info("Stand Up Reminder started")

    def _schedule_timer(self, ctx: RenderContext) -> None:
        interval_secs, _ = INTERVALS[self._interval_idx]
        self._next_fire_at = time.time() + interval_secs
        ctx.set_timer(TIMER_ID, interval_secs * 1000)

    def on_timer(self, ctx: RenderContext, timer_id: str) -> None:
        if timer_id != TIMER_ID:
            return
        prompt = random.choice(PROMPTS)
        self._last_prompt = prompt
        self._reminders_today += 1
        ctx.notify(
            title="Time to move!",
            body=prompt,
            level="info",
        )
        self.emit.info(f"Reminder #{self._reminders_today}: {prompt}")
        self._schedule_timer(ctx)
        self.emit.schedule_render(after_ms=100)

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)

        # Header bar
        ctx.rect(0, 0, ctx.w, HEADER_H, SURFACE)
        ctx.text(PAD, 14, "Stand Up Reminder", size=HEADING, color=FG, bold=True)
        ctx.text(PAD, 31, "Running in background", size=HINT, color=MUTED)

        # Separator
        ctx.rect(0, HEADER_H, ctx.w, 1, HIGHLIGHT)

        center_x = ctx.w / 2
        center_y = ctx.h / 2

        # Countdown circle background
        radius = min(ctx.w, ctx.h) * 0.22
        ctx.circle(center_x, center_y - 20, radius, SURFACE)

        # Countdown text
        remaining = max(0.0, self._next_fire_at - time.time())
        countdown_text = _format_duration(remaining)
        # Position countdown centered in circle
        text_x = center_x - (len(countdown_text) * 11)
        ctx.text(text_x, center_y - 46, countdown_text, size=TITLE + 10, color=ACCENT, bold=True)
        ctx.text(center_x - 48, center_y + 10, "until next reminder", size=CAPTION, color=MUTED)

        # Last prompt card
        card_y = center_y + radius + 20
        card_h = 64.0
        card_x = PAD
        card_w = ctx.w - PAD * 2
        ctx.rect(card_x, card_y, card_w, card_h, SURFACE, radius=8.0)

        if self._last_prompt:
            ctx.text(card_x + PAD_TIGHT, card_y + 10, "Last prompt", size=HINT, color=MUTED)
            # Truncate prompt if too long
            max_chars = int((card_w - PAD * 2) / 7.5)
            display_prompt = self._last_prompt
            if len(display_prompt) > max_chars:
                display_prompt = display_prompt[:max_chars - 1] + "…"
            ctx.text(card_x + PAD_TIGHT, card_y + 30, display_prompt, size=CAPTION, color=FG)
        else:
            ctx.text(card_x + PAD_TIGHT, card_y + 24, "No reminders sent yet", size=CAPTION, color=MUTED)

        # Reminders count
        count_y = card_y + card_h + PAD
        count_color = GREEN if self._reminders_today > 0 else MUTED
        count_text = (
            f"{self._reminders_today} reminder{'s' if self._reminders_today != 1 else ''} sent today"
        )
        ctx.text(center_x - (len(count_text) * 4), count_y, count_text, size=CAPTION, color=count_color)

        # Footer
        footer_y = ctx.h - 32.0
        ctx.rect(0, footer_y - PAD_TIGHT, ctx.w, 1, HIGHLIGHT)
        _, interval_label = INTERVALS[self._interval_idx]
        footer_text = f"Interval: {interval_label}   ·   j/k or +/- to adjust"
        ctx.text(PAD, footer_y, footer_text, size=HINT, color=MUTED)

        # Schedule next repaint in 1 second to update countdown
        self.emit.schedule_render(after_ms=1000)

    def on_key(self, ctx: RenderContext, key: str, _mods: dict) -> None:
        changed = False
        if key in ("j", "-"):
            if self._interval_idx > 0:
                self._interval_idx -= 1
                changed = True
        elif key in ("k", "+", "="):
            if self._interval_idx < len(INTERVALS) - 1:
                self._interval_idx += 1
                changed = True

        if changed:
            # Cancel current timer and reschedule with new interval
            ctx.cancel_timer(TIMER_ID)
            self._schedule_timer(ctx)
            _, label = INTERVALS[self._interval_idx]
            self.emit.info(f"Interval changed to {label}")
            self.emit.schedule_render(after_ms=50)

    def on_suspend(self) -> None:
        self.emit.info("Stand Up Reminder suspended (still timing in background)")

    def on_resume(self) -> None:
        self.emit.info("Stand Up Reminder resumed")
        self.emit.schedule_render(after_ms=50)

    def on_shutdown(self) -> None:
        self.emit.info("Stand Up Reminder shutting down")


StandUpReminderApp().run()
