#!/usr/bin/env python3
"""Stats — SDK v3 activity summary from Plexi focus events."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

from plexi_sdk import state
from plexi_sdk.effects import PersistState, SetStatus, SetTimer, SetTitle
from plexi_sdk.events import FocusChanged, KeyEvent, TimerFired
from plexi_sdk.ui import AppBar, Column, FooterKeys, SelectList, Spacer, Text

DAY_START_HOUR = 4
IDLE_THRESHOLD_SECS = 15 * 60
IDLE_CLAMP_SECS = 60
REFRESH_TIMER_ID = 1
REFRESH_MS = 60_000
TOP_CONTEXTS = 5


@dataclass
class Metrics:
    has_data: bool = False
    active_secs: float = 0.0
    switches_today: int = 0
    contexts_today: int = 0
    peak_hour_label: str = "-"
    contexts: list[dict] = field(default_factory=list)
    hourly_secs: list[float] = field(default_factory=lambda: [0.0] * 24)
    idle_stats: dict = field(default_factory=dict)


def init(size, args) -> list:
    effects: list = [
        SetTitle("Stats"),
        SetTimer(REFRESH_TIMER_ID, REFRESH_MS, repeat=True),
    ]
    missing: dict[str, Any] = {}
    if state.get("focus_events", None) is None:
        missing["focus_events"] = []
    if state.get("day_start_hour", None) is None:
        missing["day_start_hour"] = DAY_START_HOUR
    if missing:
        effects.append(PersistState(missing))
    return effects


def update(event) -> list:
    if isinstance(event, FocusChanged):
        focus_events = list(state.get("focus_events", []))
        focus_events.append(_event_from_focus_changed(event))
        return [
            PersistState({"focus_events": focus_events[-5000:]}),
            SetStatus(_summary_text(_compute_metrics(focus_events, _day_start_hour()))),
        ]
    if isinstance(event, TimerFired) and event.id == REFRESH_TIMER_ID:
        return [SetStatus(_summary_text(_metrics_from_state()))]
    if isinstance(event, KeyEvent) and event.pressed and event.key == "r":
        return [SetStatus(_summary_text(_metrics_from_state()))]
    return []


def view():
    metrics = _metrics_from_state()
    if not metrics.has_data:
        return Column([
            AppBar("Stats", "no focus events"),
            Text("No focus events yet.", size=16.0, bold=True),
            Text("Stats will fill in as Plexi sends focus-change events.", size=12.0),
            Spacer(grow=True),
            FooterKeys([("r", "refresh")]),
        ], grow=True)

    rows = [
        {"name": ctx["label"], "description": _fmt_secs(ctx["secs"])}
        for ctx in metrics.contexts
    ]
    return Column([
        AppBar("Stats", _summary_text(metrics)),
        Text(_activity_clock(metrics), size=14.0),
        Text(_summary_tiles(metrics), size=12.0),
        Text("Top contexts", size=12.0, bold=True),
        SelectList(rows, selected_idx=0) if rows else Text("No contexts yet.", size=12.0),
        Spacer(grow=True),
        Text("r refreshes.", size=11.0),
    ], grow=True)


def _day_start_hour() -> int:
    try:
        return int(state.get("day_start_hour", DAY_START_HOUR))
    except (TypeError, ValueError):
        return DAY_START_HOUR


def _metrics_from_state() -> Metrics:
    return _compute_metrics(state.get("focus_events", []), _day_start_hour())


def _event_from_focus_changed(event: FocusChanged) -> dict:
    return {
        "kind": "focus_changed",
        "timestamp": event.timestamp,
        "duration_secs": event.duration_secs,
        "reason": event.reason,
        "pane_id": event.pane_id,
        "context_name": event.context_name,
        "context_root": event.context_root,
        "cwd": event.cwd,
    }


def _coerce_focus_events(events: list[dict]) -> list[dict]:
    coerced = []
    for event in events or []:
        if event.get("kind", "focus_changed") != "focus_changed":
            continue
        try:
            ts = datetime.fromisoformat(str(event.get("timestamp", "")))
        except ValueError:
            continue
        if ts.tzinfo is None:
            ts = ts.replace(tzinfo=timezone.utc)
        item = dict(event)
        item["kind"] = "focus_changed"
        item["_ts"] = ts
        coerced.append(item)
    coerced.sort(key=lambda e: e["_ts"])
    return coerced


def _normalize_focus_events(events: list[dict]) -> tuple[list[dict], dict]:
    idle = False
    normalized = []
    stats = {
        "raw_secs": 0,
        "counted_secs": 0,
        "clamped_secs": 0,
        "skipped_secs": 0,
        "counted_events": 0,
        "clamped_events": 0,
        "skipped_events": 0,
    }

    for event in events:
        raw = int(_event_duration(event))
        reason = event.get("reason") or ""
        counted = raw
        idle_state = "active"
        if raw >= IDLE_THRESHOLD_SECS:
            if idle:
                counted = 0
                idle_state = "skipped"
            else:
                counted = IDLE_CLAMP_SECS
                idle_state = "clamped"
            idle = True
        elif idle and reason != "pane_switch":
            counted = 0
            idle_state = "skipped"
        else:
            idle = False

        item = dict(event)
        item["_raw_duration_secs"] = raw
        item["duration_secs"] = counted
        item["_idle_state"] = idle_state
        normalized.append(item)

        stats["raw_secs"] += raw
        stats["counted_secs"] += counted
        if idle_state == "clamped":
            stats["clamped_secs"] += raw - counted
            stats["clamped_events"] += 1
            stats["counted_events"] += 1
        elif idle_state == "skipped":
            stats["skipped_secs"] += raw
            stats["skipped_events"] += 1
        else:
            stats["counted_events"] += 1

    return normalized, stats


def _timeline_fractions(
    timestamp: datetime,
    counted_secs: float,
    raw_secs: float,
    idle_state: str,
    start_window: datetime,
) -> tuple[float, float, float, float]:
    raw_start = max(0.0, (timestamp - timedelta(seconds=raw_secs) - start_window).total_seconds() / 86400.0)
    raw_end = min(1.0, (timestamp - start_window).total_seconds() / 86400.0)
    if idle_state == "clamped":
        counted_start = raw_start
        counted_end = min(raw_end, counted_start + counted_secs / 86400.0)
    else:
        counted_end = raw_end
        counted_start = max(raw_start, counted_end - counted_secs / 86400.0)
    return raw_start, raw_end, counted_start, counted_end


def _compute_metrics(raw_events: list[dict], day_start_hour: int = DAY_START_HOUR) -> Metrics:
    events, idle_stats = _normalize_focus_events(_coerce_focus_events(raw_events))
    metrics = Metrics(has_data=bool(events), idle_stats=idle_stats)
    if not events:
        return metrics

    now_local = datetime.now(timezone.utc).astimezone()
    start = _day_start(now_local, day_start_hour)
    end = start + timedelta(days=1)
    by_context: dict[str, float] = {}

    for event in events:
        local = event["_ts"].astimezone()
        if local < start or local >= end:
            continue
        metrics.switches_today += 1
        secs = _event_duration(event)
        if secs <= 0:
            continue
        metrics.active_secs += secs
        metrics.hourly_secs[local.hour] += secs
        label = _context_label(event)
        by_context[label] = by_context.get(label, 0.0) + secs

    metrics.contexts_today = len(by_context)
    metrics.contexts = [
        {"label": label, "secs": secs}
        for label, secs in sorted(by_context.items(), key=lambda item: item[1], reverse=True)[:TOP_CONTEXTS]
    ]
    if any(metrics.hourly_secs):
        peak = max(range(24), key=lambda hour: metrics.hourly_secs[hour])
        metrics.peak_hour_label = _hour_label(peak)
    return metrics


def _event_duration(event: dict) -> float:
    try:
        return max(0.0, float(event.get("duration_secs", 0) or 0))
    except (TypeError, ValueError):
        return 0.0


def _day_start(local_dt: datetime, day_start_hour: int = DAY_START_HOUR) -> datetime:
    start = local_dt.replace(hour=day_start_hour, minute=0, second=0, microsecond=0)
    if local_dt.hour < day_start_hour:
        start -= timedelta(days=1)
    return start


def _context_label(event: dict) -> str:
    for key in ("context_name", "context_root", "cwd"):
        value = event.get(key)
        if isinstance(value, str) and value.strip() and value.strip() not in ("(none)", "Default"):
            if key in ("context_root", "cwd"):
                return Path(value).name or value
            return value.strip()
    return "Untitled"


def _fmt_secs(secs: float) -> str:
    hours = int(secs) // 3600
    minutes = (int(secs) % 3600) // 60
    if hours:
        return f"{hours}h {minutes:02d}m"
    return f"{minutes}m"


def _hour_label(hour: int) -> str:
    ampm = "AM" if hour < 12 else "PM"
    h12 = hour % 12 or 12
    nxt = (hour + 1) % 12 or 12
    return f"{h12}-{nxt} {ampm}"


def _summary_text(metrics: Metrics) -> str:
    return (
        f"{_fmt_secs(metrics.active_secs)} active, "
        f"{metrics.switches_today} switches, "
        f"{metrics.contexts_today} contexts"
    )


def _summary_tiles(metrics: Metrics) -> str:
    return (
        f"Peak {metrics.peak_hour_label}  |  "
        f"Idle clamped {_fmt_secs(metrics.idle_stats.get('clamped_secs', 0))}  |  "
        f"Skipped {_fmt_secs(metrics.idle_stats.get('skipped_secs', 0))}"
    )


def _activity_clock(metrics: Metrics) -> str:
    max_hour = max(metrics.hourly_secs) or 1.0
    blocks = " .:-=+*#%@"
    chars = []
    for value in metrics.hourly_secs:
        idx = round((value / max_hour) * (len(blocks) - 1)) if value else 0
        chars.append(blocks[idx])
    return "24h activity " + "".join(chars)
