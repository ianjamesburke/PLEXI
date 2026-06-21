"""Verify that overnight heartbeats (idle computer) produce no activity entries."""
from __future__ import annotations

import os
import sys
from datetime import datetime, timedelta, timezone

sys.path.insert(
    0,
    os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"),
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from stats import Metrics, DAY_START_HOUR  # noqa: E402

IDLE_THRESHOLD_SECS = 15 * 60


def _today_window_start() -> datetime:
    """Return 4 AM local today (the start of the current stats day window)."""
    now_local = datetime.now(timezone.utc).astimezone()
    start = now_local.replace(hour=DAY_START_HOUR, minute=0, second=0, microsecond=0)
    if now_local.hour < DAY_START_HOUR:
        start -= timedelta(days=1)
    return start


def _heartbeat(ts: datetime, duration: int) -> dict:
    return {
        "kind": "focus_changed",
        "timestamp": ts.isoformat(),
        "_ts": ts,
        "duration_secs": duration,
        "reason": "heartbeat",
        "pane_id": 1,
        "context_name": "PLEXI",
        "context_root": "/Users/ianburke/Documents/GitHub/PLEXI",
        "cwd": "/Users/ianburke/Documents/GitHub/PLEXI",
    }


def _pane_switch(ts: datetime, duration: int) -> dict:
    return {
        "kind": "focus_changed",
        "timestamp": ts.isoformat(),
        "_ts": ts,
        "duration_secs": duration,
        "reason": "pane_switch",
        "pane_id": 1,
        "context_name": "PLEXI",
        "context_root": "/Users/ianburke/Documents/GitHub/PLEXI",
        "cwd": "/Users/ianburke/Documents/GitHub/PLEXI",
    }


def test_overnight_heartbeats_produce_no_switches():
    """Heartbeats while the computer is idle must not inflate the Switches tile."""
    # Place a pane_switch shortly after the day window opens (4 AM), then fill
    # the morning with idle heartbeats.  All events are in today's window.
    base = _today_window_start() + timedelta(hours=1)  # 5 AM local
    events = [_pane_switch(base, 30)]
    for i in range(1, 10):
        events.append(_heartbeat(base + timedelta(minutes=15 * i), IDLE_THRESHOLD_SECS))
    events.sort(key=lambda e: e["_ts"])

    m = Metrics.compute(events, day_offset=0)

    assert m.switches_today == 1, (
        f"Expected 1 switch (pane_switch only), got {m.switches_today} — "
        "heartbeats are leaking into the Switches tile"
    )


def test_overnight_heartbeats_produce_no_waveform_spokes():
    """Idle-filtered heartbeats (secs==0) must not produce waveform spokes."""
    base = _today_window_start() + timedelta(hours=1)  # 5 AM local
    events = [_pane_switch(base, 30)]
    # First heartbeat is clamped to 60s; subsequent ones return 0.
    for i in range(1, 15):
        events.append(_heartbeat(base + timedelta(minutes=15 * i), IDLE_THRESHOLD_SECS))
    events.sort(key=lambda e: e["_ts"])

    m = Metrics.compute(events, day_offset=0)

    non_zero_buckets = [b for b, (count, _) in enumerate(m.wave) if count > 0]
    # Only the pane_switch + first clamped heartbeat may be non-zero.
    # The 13 idle heartbeats after that must produce no spokes.
    assert len(non_zero_buckets) <= 2, (
        f"Expected ≤2 active wave buckets, got {len(non_zero_buckets)} — "
        "idle heartbeats are painting overnight activity spokes"
    )
