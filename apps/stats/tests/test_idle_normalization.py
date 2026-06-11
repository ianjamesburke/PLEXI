from __future__ import annotations

import os
import sys
from datetime import datetime, timezone


sys.path.insert(
    0,
    os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"),
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from stats import _normalize_focus_events  # noqa: E402


def _event(duration: int, reason: str, pane_id: int = 1) -> dict:
    return {
        "kind": "focus_changed",
        "timestamp": datetime(2026, 6, 10, 12, 0, tzinfo=timezone.utc).isoformat(),
        "_ts": datetime(2026, 6, 10, 12, 0, tzinfo=timezone.utc),
        "duration_secs": duration,
        "reason": reason,
        "pane_id": pane_id,
        "context_name": "PLEXI",
        "context_root": "/Users/ianburke/Documents/GitHub/PLEXI",
        "cwd": "/Users/ianburke/Documents/GitHub/PLEXI",
    }


def test_idle_normalization_clamps_first_stale_segment_and_skips_until_switch():
    events = [
        _event(120, "pane_switch"),
        _event(1800, "heartbeat"),
        _event(900, "heartbeat"),
        _event(300, "shutdown"),
        _event(240, "pane_switch", pane_id=2),
    ]

    normalized, stats = _normalize_focus_events(events)

    assert [ev["duration_secs"] for ev in normalized] == [120, 60, 0, 0, 240]
    assert [ev["_idle_state"] for ev in normalized] == [
        "active",
        "clamped",
        "skipped",
        "skipped",
        "active",
    ]
    assert stats["raw_secs"] == 3360
    assert stats["counted_secs"] == 420
    assert stats["clamped_secs"] == 1740
    assert stats["skipped_secs"] == 1200
    assert stats["counted_events"] == 3
    assert stats["clamped_events"] == 1
    assert stats["skipped_events"] == 2


def test_idle_normalization_keeps_short_active_sessions():
    events = [
        _event(60, "pane_switch"),
        _event(300, "heartbeat"),
        _event(480, "shutdown"),
    ]

    normalized, stats = _normalize_focus_events(events)

    assert [ev["duration_secs"] for ev in normalized] == [60, 300, 480]
    assert [ev["_idle_state"] for ev in normalized] == ["active", "active", "active"]
    assert stats["raw_secs"] == 840
    assert stats["counted_secs"] == 840
    assert stats["clamped_secs"] == 0
    assert stats["skipped_secs"] == 0
