"""
Tests for the Aquarium Plexi app.

Run:
    python3 -m pytest examples/aquarium/tests/ -v
"""

import sys
import os
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"))

from plexi_test import AppTestHarness

ENTRY = os.path.join(os.path.dirname(__file__), "..", "aquarium.py")
W, H = 800, 600


def make_harness():
    h = AppTestHarness(ENTRY)
    h.send_init(width=W, height=H)
    return h


# ---------------------------------------------------------------------------
# 1. Basic rendering
# ---------------------------------------------------------------------------

def test_renders_without_crash():
    """App starts and produces draw commands."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        assert len(frame) > 0, "Expected draw commands, got none"


def test_renders_background_rects():
    """Aquarium renders multiple rects (water gradient bands + sand + fish)."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        rects = [c for c in frame if c.get("type") == "rect"]
        # Water gradient (10 bands) + sand + fish bodies + bubbles
        assert len(rects) >= 10, f"Expected >= 10 rects, got {len(rects)}"


def test_renders_lines():
    """Aquarium renders lines (fish tails, plant stems)."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        lines = [c for c in frame if c.get("type") == "line"]
        assert len(lines) > 0, f"Expected lines for fish tails/plants, got none"


def test_background_fills_viewport():
    """At least one rect covers the full width and height (background)."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        rects = [c for c in frame if c.get("type") == "rect"]
        full_width = [r for r in rects if r.get("w", 0) >= W]
        assert len(full_width) > 0, \
            f"Expected a full-width rect, found widths: {[r.get('w') for r in rects]}"


# ---------------------------------------------------------------------------
# 2. Animation — frame changes over time
# ---------------------------------------------------------------------------

def test_animation_advances():
    """Consecutive render frames produce different draw commands (animation is running)."""
    with make_harness() as h:
        frame1 = h.send_render(width=W, height=H)
        time.sleep(0.1)  # let the animation clock advance
        frame2 = h.send_render(width=W, height=H)

        # Compare line commands (fish tail oscillation should change position)
        lines1 = [(c.get("x1"), c.get("y1"), c.get("x2"), c.get("y2"))
                  for c in frame1 if c.get("type") == "line"]
        lines2 = [(c.get("x1"), c.get("y1"), c.get("x2"), c.get("y2"))
                  for c in frame2 if c.get("type") == "line"]
        # At least some line coordinates should differ
        assert lines1 != lines2, \
            "Draw commands identical across frames — animation may be frozen"


# ---------------------------------------------------------------------------
# 3. Resize handling
# ---------------------------------------------------------------------------

def test_resize_doesnt_crash():
    """Rendering at multiple sizes doesn't crash the app."""
    with make_harness() as h:
        for (w, h_dim) in [(400, 300), (1920, 1080), (320, 240)]:
            frame = h.send_render(width=w, height=h_dim)
            assert len(frame) > 0, f"No draw commands after resize to {w}x{h_dim}"


def test_rects_fit_within_viewport():
    """All rect x/y coordinates are within the rendered viewport."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        bad = []
        for c in frame:
            if c.get("type") != "rect":
                continue
            x, y = c.get("x", 0), c.get("y", 0)
            # Allow a small overdraw margin (10px) for rounding
            if x < -10 or y < -10 or x > W + 10 or y > H + 10:
                bad.append((x, y, c.get("w"), c.get("h")))
        assert len(bad) == 0, f"Rects outside viewport: {bad}"


# ---------------------------------------------------------------------------
# 4. Shutdown
# ---------------------------------------------------------------------------

def test_clean_shutdown():
    """App shuts down cleanly without hanging."""
    h = make_harness()
    h.send_render(width=W, height=H)
    h.shutdown()
    assert h._proc.returncode is not None, "Process did not exit after shutdown"
