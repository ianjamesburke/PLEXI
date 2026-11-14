"""Stint 1375: SDK surface the host never handles is deleted, not kept as a trap.

Each name below was an effect the live runtime refused or an event class no
host path sends. Re-add one only together with the host code that honours it.
"""

import plexi_sdk.effects as effects
import plexi_sdk.events as events

REMOVED_EFFECTS = ["GetSystemStats"]
REMOVED_EVENTS = [
    "SystemStats",
    "SystemStatsResult",
    "AiStreamChunk",
    "PipePeerConnected",
    "PipeClosed",
    "PipeError",
    "PaymentComplete",
    "PaymentFailed",
]


def test_removed_effects_are_gone():
    present = [name for name in REMOVED_EFFECTS if hasattr(effects, name)]
    assert present == [], f"effects the host never handles are still defined: {present}"


def test_removed_event_classes_are_gone():
    present = [name for name in REMOVED_EVENTS if hasattr(events, name)]
    assert present == [], f"event classes no host path sends are still defined: {present}"


def test_ai_surface_kept_for_stint_1450():
    assert hasattr(effects, "AiQuery")
    assert hasattr(events, "AiResponse")
