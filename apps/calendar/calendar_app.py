#!/usr/bin/env python3
"""Calendar — month-view calendar app for the PAM voice demo.

Keyboard nav: j/k=day, h/l=week, H/L=month, a=add, e=edit, d=delete.
Pipe interface: accepts JSON commands from the voice agent daemon (#748).
"""

import calendar
from datetime import date, timedelta
from typing import Any

from plexi_sdk import (
    App, RenderContext,
    BODY, CAPTION, HINT, HEADING, PAD,
)

DAY_NAMES = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
CELL_H = 52.0
CELL_RADIUS = 6.0


class CalendarApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        today = date.today()
        self._year = today.year
        self._month = today.month
        self._day = today.day
        self._events: list[dict] = []
        self._next_id = 1
        self._mode = "view"       # view | add | edit | confirm_delete
        self._add_stage = "title" # title | date | time
        self._input = ""
        self._editing_id: str | None = None
        self._status = ""
        self._staged: dict = {}
        self.emit.info("calendar: ready")

    # ── Calendar math ─────────────────────────────────────────────────────────

    def _selected_date_str(self) -> str:
        return f"{self._year:04d}-{self._month:02d}-{self._day:02d}"

    def _events_for(self, date_str: str) -> list[dict]:
        return [e for e in self._events if e["date"] == date_str]

    def _move_day(self, delta: int) -> None:
        d = date(self._year, self._month, self._day) + timedelta(days=delta)
        self._year, self._month, self._day = d.year, d.month, d.day

    def _move_month(self, delta: int) -> None:
        m, y = self._month + delta, self._year
        while m > 12: m -= 12; y += 1
        while m < 1:  m += 12; y -= 1
        self._year, self._month = y, m
        self._day = min(self._day, calendar.monthrange(y, m)[1])

    # ── Event store ───────────────────────────────────────────────────────────

    def _add_event(self, title: str, date_str: str,
                   time_str: str = "", duration: int = 60) -> str:
        eid = str(self._next_id)
        self._next_id += 1
        self._events.append({"id": eid, "title": title, "date": date_str,
                              "time": time_str, "duration": duration})
        self.emit.info(f"calendar: add event id={eid} title={title!r} date={date_str}")
        return eid

    def _update_event(self, eid: str, **kwargs) -> bool:
        for e in self._events:
            if e["id"] == eid:
                for k, v in kwargs.items():
                    if v is not None:
                        e[k] = v
                self.emit.info(f"calendar: update event id={eid}")
                return True
        self.emit.warn(f"calendar: update_event id={eid} not found")
        return False

    def _delete_event(self, eid: str) -> bool:
        before = len(self._events)
        self._events = [e for e in self._events if e["id"] != eid]
        if len(self._events) < before:
            self.emit.info(f"calendar: delete event id={eid}")
            return True
        self.emit.warn(f"calendar: delete_event id={eid} not found")
        return False

    # ── Pipe interface ────────────────────────────────────────────────────────

    def on_pipe_message(self, ctx: RenderContext, pipe_id: str, payload: Any) -> None:
        if not isinstance(payload, dict):
            self.emit.warn(f"calendar: non-dict pipe payload from pipe_id={pipe_id}")
            self.emit.pipe_send(pipe_id, {"ok": False, "error": "payload must be a JSON object"})
            return

        action = payload.get("action")
        self.emit.info(f"calendar: pipe action={action!r} pipe_id={pipe_id}")

        if action == "add_event":
            title = str(payload.get("title", "Untitled"))
            date_str = str(payload.get("date", self._selected_date_str()))
            time_str = str(payload.get("time", ""))
            duration = int(payload.get("duration", 60))
            eid = self._add_event(title, date_str, time_str, duration)
            self.emit.pipe_send(pipe_id, {"ok": True, "event_id": eid})
            self.emit.schedule_render()

        elif action == "update_event":
            eid = str(payload.get("id", ""))
            ok = self._update_event(
                eid,
                title=payload.get("title"),
                date=payload.get("date"),
                time=payload.get("time"),
                duration=payload.get("duration"),
            )
            if ok:
                self.emit.pipe_send(pipe_id, {"ok": True, "event_id": eid})
                self.emit.schedule_render()
            else:
                self.emit.pipe_send(pipe_id, {"ok": False, "error": f"event {eid!r} not found"})

        elif action == "delete_event":
            eid = str(payload.get("id", ""))
            ok = self._delete_event(eid)
            if ok:
                self.emit.pipe_send(pipe_id, {"ok": True})
                self.emit.schedule_render()
            else:
                self.emit.pipe_send(pipe_id, {"ok": False, "error": f"event {eid!r} not found"})

        elif action == "list_events":
            date_str = str(payload.get("date", self._selected_date_str()))
            self.emit.pipe_send(pipe_id, {"ok": True, "events": self._events_for(date_str)})

        else:
            self.emit.warn(f"calendar: unknown pipe action {action!r}")
            self.emit.pipe_send(pipe_id, {"ok": False, "error": f"unknown action {action!r}"})

    # ── Key input ─────────────────────────────────────────────────────────────

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if self._mode == "view":
            self._key_view(key)
        elif self._mode in ("add", "edit"):
            self._key_input(key)
        elif self._mode == "confirm_delete":
            self._key_delete(key)

    def _key_view(self, key: str) -> None:
        if key == "j":        self._move_day(1)
        elif key == "k":      self._move_day(-1)
        elif key == "h":      self._move_day(-7)
        elif key == "l":      self._move_day(7)
        elif key == "H":      self._move_month(-1)
        elif key == "L":      self._move_month(1)
        elif key == "a":      self._begin_add()
        elif key == "e":      self._begin_edit()
        elif key == "d":      self._begin_delete()

    def _key_input(self, key: str) -> None:
        if key == "Escape":
            self._mode = "view"
            self._status = ""
        elif key == "Enter":
            self._advance_stage()
        elif key == "Backspace":
            self._input = self._input[:-1]
        elif len(key) == 1:
            self._input += key

    def _key_delete(self, key: str) -> None:
        if key == "y":
            evs = self._events_for(self._selected_date_str())
            if evs:
                self._delete_event(evs[0]["id"])
                self._status = "Deleted"
            self._mode = "view"
        elif key in ("n", "Escape"):
            self._mode = "view"
            self._status = ""

    def _begin_add(self) -> None:
        self._mode = "add"
        self._add_stage = "title"
        self._input = ""
        self._staged = {"date": self._selected_date_str(), "time": "", "duration": 60}
        self._editing_id = None
        self._status = ""

    def _begin_edit(self) -> None:
        evs = self._events_for(self._selected_date_str())
        if not evs:
            self._status = "No events today"
            return
        e = evs[0]
        self._mode = "edit"
        self._add_stage = "title"
        self._input = e["title"]
        self._staged = dict(e)
        self._editing_id = e["id"]
        self._status = ""

    def _begin_delete(self) -> None:
        evs = self._events_for(self._selected_date_str())
        if not evs:
            self._status = "No events today"
            return
        self._mode = "confirm_delete"

    def _advance_stage(self) -> None:
        if self._add_stage == "title":
            self._staged["title"] = self._input.strip() or "Untitled"
            self._add_stage = "date"
            self._input = self._staged["date"]
        elif self._add_stage == "date":
            self._staged["date"] = self._input.strip() or self._staged["date"]
            self._add_stage = "time"
            self._input = self._staged.get("time", "")
        elif self._add_stage == "time":
            self._staged["time"] = self._input.strip()
            if self._editing_id:
                self._update_event(
                    self._editing_id,
                    title=self._staged["title"],
                    date=self._staged["date"],
                    time=self._staged["time"],
                )
                self._status = "Updated"
            else:
                self._add_event(
                    self._staged["title"],
                    self._staged["date"],
                    self._staged["time"],
                    int(self._staged.get("duration", 60)),
                )
                self._status = "Added"
            self._mode = "view"

    # ── Render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(ctx.theme.bg)

        # Header bar
        header_h = 48.0
        ctx.rect(0, 0, ctx.w, header_h, fill=ctx.theme.surface)
        month_name = date(self._year, self._month, 1).strftime("%B %Y")
        ctx.text(PAD, 10.0, "Calendar", size=HEADING, color=ctx.theme.fg, bold=True)
        ctx.text(PAD, 30.0, month_name, size=CAPTION, color=ctx.theme.muted)

        # Day-of-week column headers
        grid_left = PAD
        grid_w = ctx.w - 2 * PAD
        cell_w = grid_w / 7.0
        dow_y = header_h + 8.0
        for i, name in enumerate(DAY_NAMES):
            ctx.text(grid_left + i * cell_w + 6.0, dow_y, name, size=CAPTION, color=ctx.theme.muted)

        # Month grid
        cells_top = dow_y + 20.0
        first_wd = calendar.monthrange(self._year, self._month)[0]
        days_count = calendar.monthrange(self._year, self._month)[1]
        today = date.today()

        row, col = 0, first_wd
        for day_num in range(1, days_count + 1):
            cx = grid_left + col * cell_w
            cy = cells_top + row * CELL_H
            d_str = f"{self._year:04d}-{self._month:02d}-{day_num:02d}"
            is_today = (date(self._year, self._month, day_num) == today)
            is_selected = (day_num == self._day)

            if is_selected:
                ctx.rect(cx + 2, cy + 2, cell_w - 4, CELL_H - 4, fill=ctx.theme.accent, radius=CELL_RADIUS)
                text_col = ctx.theme.bg
            elif is_today:
                ctx.rect(cx + 2, cy + 2, cell_w - 4, CELL_H - 4, fill=ctx.theme.surface, radius=CELL_RADIUS)
                text_col = ctx.theme.accent
            else:
                text_col = ctx.theme.fg

            ctx.text(cx + 6.0, cy + 8.0, str(day_num), size=BODY, color=text_col)

            if self._events_for(d_str):
                dot_col = ctx.theme.bg if is_selected else ctx.theme.accent
                ctx.circle(cx + cell_w / 2.0, cy + CELL_H - 10.0, 3.0, fill=dot_col)

            col += 1
            if col == 7:
                col = 0
                row += 1

        # Event list for selected day
        num_rows = (first_wd + days_count + 6) // 7
        footer_h = 44.0
        ev_top = cells_top + num_rows * CELL_H + 10.0
        ev_h = ctx.h - ev_top - footer_h

        self._draw_event_list(ctx, grid_left, ev_top, grid_w, ev_h)

        # Modal overlays
        if self._mode in ("add", "edit"):
            self._draw_input_modal(ctx)
        elif self._mode == "confirm_delete":
            self._draw_delete_modal(ctx)

        # Footer bar
        fy = ctx.h - footer_h
        ctx.rect(0, fy, ctx.w, footer_h, fill=ctx.theme.surface)
        if self._status:
            ctx.text(PAD, fy + 14.0, self._status, size=CAPTION, color=ctx.theme.success)
        elif self._mode == "view":
            ctx.shortcuts(PAD, fy + 14.0, ctx.w - 2 * PAD, [
                (["j", "k"], "day"),
                (["h", "l"], "week"),
                (["H", "L"], "month"),
                ("a", "add"),
                ("e", "edit"),
                ("d", "delete"),
            ])

    def _draw_event_list(self, ctx: RenderContext, x: float, y: float,
                         w: float, h: float) -> None:
        if h < 24:
            return
        date_str = self._selected_date_str()
        evs = self._events_for(date_str)
        d_label = date(self._year, self._month, self._day).strftime("%A, %B %-d")
        ctx.text(x, y, d_label, size=CAPTION, color=ctx.theme.muted)
        ey = y + 20.0
        if not evs:
            ctx.text(x, ey, "No events", size=BODY, color=ctx.theme.muted)
            return
        for ev in evs:
            if ey + 24 > y + h:
                break
            time_part = f"{ev['time']}  " if ev.get("time") else ""
            ctx.rect(x, ey, w, 22.0, fill=ctx.theme.surface, radius=4.0)
            ctx.text(x + 8.0, ey + 4.0, f"{time_part}{ev['title']}", size=CAPTION, color=ctx.theme.fg)
            ey += 26.0

    def _draw_input_modal(self, ctx: RenderContext) -> None:
        mw = min(400.0, ctx.w - 2 * PAD)
        mh = 120.0
        mx = (ctx.w - mw) / 2.0
        my = (ctx.h - mh) / 2.0
        ctx.rect(mx, my, mw, mh, fill=ctx.theme.surface, radius=8.0)

        title_text = "Edit Event" if self._editing_id else "Add Event"
        stage_prompt = {
            "title": "Event title:",
            "date":  "Date (YYYY-MM-DD):",
            "time":  "Time (HH:MM, or Enter to skip):",
        }[self._add_stage]

        ctx.text(mx + 16.0, my + 12.0, title_text, size=BODY, color=ctx.theme.fg, bold=True)
        ctx.text(mx + 16.0, my + 36.0, stage_prompt, size=CAPTION, color=ctx.theme.muted)
        ctx.rect(mx + 16.0, my + 56.0, mw - 32.0, 30.0, fill=ctx.theme.highlight, radius=4.0)
        ctx.text(mx + 22.0, my + 64.0, self._input + "▌", size=BODY, color=ctx.theme.fg)
        ctx.text(mx + 16.0, my + 98.0, "↵ confirm  esc cancel", size=HINT, color=ctx.theme.muted)

    def _draw_delete_modal(self, ctx: RenderContext) -> None:
        evs = self._events_for(self._selected_date_str())
        if not evs:
            self._mode = "view"
            return
        mw = min(360.0, ctx.w - 2 * PAD)
        mh = 90.0
        mx = (ctx.w - mw) / 2.0
        my = (ctx.h - mh) / 2.0
        ctx.rect(mx, my, mw, mh, fill=ctx.theme.surface, radius=8.0)
        ctx.text(mx + 16.0, my + 12.0, "Delete event?", size=BODY, color=ctx.theme.fg, bold=True)
        ctx.text(mx + 16.0, my + 34.0, evs[0]["title"], size=CAPTION, color=ctx.theme.muted)
        ctx.text(mx + 16.0, my + 62.0, "y  confirm    n / esc  cancel", size=HINT, color=ctx.theme.muted)


if __name__ == "__main__":
    CalendarApp().run()
