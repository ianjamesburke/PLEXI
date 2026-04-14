"""
Tests for the Stopwatch Plexi app.

Run:
    python3 -m pytest examples/stopwatch/tests/ -v
"""

import sys
import os
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"))

from plexi_test import AppTestHarness

ENTRY = os.path.join(os.path.dirname(__file__), "..", "stopwatch.py")
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


def test_initial_state_stopped():
    """On startup the stopwatch shows STOPPED."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        h.assert_text_visible("STOPPED", frame)


def test_initial_time_is_zero():
    """Initial display shows 00:00.00."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        assert any("00:00" in t for t in texts), \
            f"Expected '00:00' in display, got: {texts}"


def test_no_laps_initially():
    """Empty lap list shows 'No laps recorded.'"""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        h.assert_text_visible("No laps recorded.", frame)


# ---------------------------------------------------------------------------
# 2. Start/stop
# ---------------------------------------------------------------------------

def test_space_starts_stopwatch():
    """Pressing Space transitions to RUNNING."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key(" ")
        frame = h.send_render(width=W, height=H)
        h.assert_text_visible("RUNNING", frame)


def test_time_advances_while_running():
    """Time display changes between renders while running."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key(" ")
        frame1 = h.send_render(width=W, height=H)
        time.sleep(0.05)
        frame2 = h.send_render(width=W, height=H)
        # Find the time strings — they should differ
        t1 = [t for t in h.find_texts(frame1) if ":" in t and "." in t]
        t2 = [t for t in h.find_texts(frame2) if ":" in t and "." in t]
        assert t1 != t2, f"Time did not advance: {t1} vs {t2}"


def test_space_toggles_to_stopped():
    """Pressing Space twice → RUNNING then STOPPED."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key(" ")
        h.send_render(width=W, height=H)
        h.send_key(" ")
        frame = h.send_render(width=W, height=H)
        h.assert_text_visible("STOPPED", frame)


# ---------------------------------------------------------------------------
# 3. Laps
# ---------------------------------------------------------------------------

def test_lap_only_records_while_running():
    """Pressing 'l' while stopped does not add a lap."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("l")
        frame = h.send_render(width=W, height=H)
        # Still no laps
        h.assert_text_visible("No laps recorded.", frame)


def test_lap_records_while_running():
    """Pressing 'l' while running records a lap."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key(" ")  # start
        h.send_render(width=W, height=H)
        h.send_key("l")
        frame = h.send_render(width=W, height=H)
        texts = " ".join(h.find_texts(frame)).lower()
        assert "lap" in texts, f"Expected 'lap' label after recording, got: {texts}"


# ---------------------------------------------------------------------------
# 4. Reset
# ---------------------------------------------------------------------------

def test_reset_only_when_stopped():
    """'r' resets timer to 00:00 when stopped."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key(" ")  # start
        h.send_render(width=W, height=H)
        time.sleep(0.05)
        h.send_key(" ")  # stop
        h.send_key("r")  # reset
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        assert any("00:00.00" in t for t in texts), \
            f"Expected '00:00.00' after reset, got: {texts}"


# ---------------------------------------------------------------------------
# 5. Resize
# ---------------------------------------------------------------------------

def test_resize_doesnt_crash():
    """Rendering at multiple sizes doesn't crash."""
    with make_harness() as h:
        for (w, hh) in [(400, 300), (1200, 900), (640, 480)]:
            frame = h.send_render(width=w, height=hh)
            assert len(frame) > 0


# ---------------------------------------------------------------------------
# 6. Shutdown
# ---------------------------------------------------------------------------

def test_clean_shutdown():
    """App shuts down cleanly."""
    h = make_harness()
    h.send_render(width=W, height=H)
    h.shutdown()
    assert h._proc.returncode is not None
