"""Active-time idle clamping — the core of the centre gauge's focus total.

`_counted_duration` walks focus events in order, carrying a single-element
`idle_stream` flag between calls. Very long single-pane spans are treated as
idle: the first one is clamped to a short residual, later ones are skipped
entirely until the next real `pane_switch` re-activates the stream.
"""
from __future__ import annotations

import os
import sys

sys.path.insert(
    0,
    os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"),
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from stats import (  # noqa: E402
    IDLE_CLAMP_SECS,
    IDLE_THRESHOLD_SECS,
    _counted_duration,
)


def _counted(sequence: "list[tuple[str, int]]") -> "list[float]":
    idle_stream = [False]
    return [
        _counted_duration({"reason": reason, "duration_secs": secs}, idle_stream)
        for reason, secs in sequence
    ]


def test_first_stale_segment_clamps_then_later_stale_segments_skip() -> None:
    counted = _counted(
        [
            ("pane_switch", 120),
            ("heartbeat", IDLE_THRESHOLD_SECS * 2),
            ("heartbeat", IDLE_THRESHOLD_SECS),
            ("shutdown", 300),
            ("pane_switch", 240),
        ]
    )
    assert counted == [120, IDLE_CLAMP_SECS, 0, 0, 240]
    assert sum(counted) == 120 + IDLE_CLAMP_SECS + 240


def test_short_active_sessions_are_counted_in_full() -> None:
    counted = _counted(
        [
            ("pane_switch", 60),
            ("heartbeat", 300),
            ("shutdown", 480),
        ]
    )
    assert counted == [60, 300, 480]
    assert sum(counted) == 840
