"""
Tests for the Pomodoro Plexi app.

Run:
    python3 -m pytest examples/pomodoro/tests/ -v
"""

import sys
import os

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"))

from plexi_test import AppTestHarness

ENTRY = os.path.join(os.path.dirname(__file__), "..", "pomodoro.py")
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
        assert len(frame) > 0


def test_header_visible():
    """Header text 'Pomodoro' is rendered."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        h.assert_text_visible("Pomodoro", frame)


def test_initial_idle_state():
    """Initial state shows 'Ready to focus' label."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        h.assert_text_visible("Ready to focus", frame)


def test_initial_timer_is_25_minutes():
    """Initial countdown shows 25:00."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        h.assert_text_visible("25:00", frame)


# ---------------------------------------------------------------------------
# 2. State transitions
# ---------------------------------------------------------------------------

def test_space_starts_focus_session():
    """Pressing Space transitions from idle → Focus state."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key(" ")
        frame = h.send_render(width=W, height=H)
        h.assert_text_visible("Focus", frame)


def test_b_switches_to_short_break():
    """Pressing 'b' switches to Short Break and resets timer."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("b")
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        assert any("Short Break" in t for t in texts), \
            f"Expected 'Short Break' label, got: {texts}"
        # 5-minute timer
        assert any("05:00" in t for t in texts), \
            f"Expected '05:00' for short break, got: {texts}"


def test_l_switches_to_long_break():
    """Pressing 'l' switches to Long Break (15 min)."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("l")
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        assert any("Long Break" in t for t in texts), \
            f"Expected 'Long Break' label, got: {texts}"
        assert any("15:00" in t for t in texts), \
            f"Expected '15:00', got: {texts}"


# ---------------------------------------------------------------------------
# 3. Reset
# ---------------------------------------------------------------------------

def test_reset_restores_full_duration():
    """After starting and pressing 'r', timer returns to original duration."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key(" ")  # start focus
        h.send_render(width=W, height=H)
        h.send_key("r")  # reset
        frame = h.send_render(width=W, height=H)
        h.assert_text_visible("25:00", frame)


# ---------------------------------------------------------------------------
# 4. Resize
# ---------------------------------------------------------------------------

def test_resize_doesnt_crash():
    """Rendering at multiple sizes doesn't crash."""
    with make_harness() as h:
        for (w, hh) in [(400, 300), (1200, 900), (640, 480)]:
            frame = h.send_render(width=w, height=hh)
            assert len(frame) > 0


# ---------------------------------------------------------------------------
# 5. Shutdown
# ---------------------------------------------------------------------------

def test_clean_shutdown():
    """App shuts down cleanly."""
    h = make_harness()
    h.send_render(width=W, height=H)
    h.shutdown()
    assert h._proc.returncode is not None
