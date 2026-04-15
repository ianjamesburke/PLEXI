"""
Tests for the Snake Plexi app.

Uses plexi_test.AppTestHarness to drive the app via the JSON protocol.
No display or GUI required — all assertions are on draw commands.

Run:
    python3 -m pytest examples/snake/tests/ -v
"""

import sys
import os
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"))

from plexi_test import AppTestHarness

ENTRY = os.path.join(os.path.dirname(__file__), "..", "snake.py")
W, H = 800, 600


def make_harness():
    h = AppTestHarness(ENTRY)
    h.send_init(width=W, height=H)
    return h


# ---------------------------------------------------------------------------
# 1. Startup — title screen
# ---------------------------------------------------------------------------

def test_title_screen_renders():
    """App starts on the title screen with the Snake name visible."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        assert any("snake" in t.lower() for t in texts), f"Expected 'SNAKE' in texts, got: {texts}"


def test_title_screen_has_start_hint():
    """Title screen shows a 'Enter' or 'start' hint."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        combined = " ".join(texts).lower()
        assert "enter" in combined or "start" in combined, \
            f"Expected start hint in title, got: {texts}"


def test_title_screen_has_background_rect():
    """Title screen draws at least one background rect."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        rects = [c for c in frame if c.get("type") == "rect"]
        assert len(rects) > 0, "Expected at least one rect on title screen"


# ---------------------------------------------------------------------------
# 2. Starting the game
# ---------------------------------------------------------------------------

def test_enter_starts_game():
    """Pressing Enter transitions from title to playing state."""
    with make_harness() as h:
        h.send_render(width=W, height=H)   # title screen
        h.send_key("Enter")
        time.sleep(0.05)
        frame = h.send_render(width=W, height=H)
        # Playing state should have a score visible
        texts = h.find_texts(frame)
        combined = " ".join(texts).lower()
        assert "score" in combined or "0" in combined, \
            f"Expected score after starting, got: {texts}"


def test_playing_state_draws_grid_rects():
    """Playing state draws multiple rects (the game grid)."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("Enter")
        time.sleep(0.05)
        frame = h.send_render(width=W, height=H)
        rects = [c for c in frame if c.get("type") == "rect"]
        # At minimum: background + snake head + food = 3+
        assert len(rects) >= 3, f"Expected >= 3 rects in playing state, got {len(rects)}"


# ---------------------------------------------------------------------------
# 3. Direction input
# ---------------------------------------------------------------------------

def test_direction_keys_accepted():
    """Arrow keys are accepted without crashing."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("Enter")
        time.sleep(0.05)
        for key in ("ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"):
            h.send_key(key)
            frame = h.send_render(width=W, height=H)
            assert len(frame) > 0, f"No draw commands after {key}"


def test_wasd_keys_accepted():
    """WASD keys are accepted without crashing."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("Enter")
        time.sleep(0.05)
        for key in ("w", "a", "s", "d"):
            h.send_key(key)
            frame = h.send_render(width=W, height=H)
            assert len(frame) > 0, f"No draw commands after key '{key}'"


# ---------------------------------------------------------------------------
# 4. Pause
# ---------------------------------------------------------------------------

def test_pause_toggle():
    """Pressing 'p' pauses and shows a pause indicator."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("Enter")
        time.sleep(0.05)
        h.send_render(width=W, height=H)
        h.send_key("p")
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        combined = " ".join(texts).lower()
        assert "pause" in combined or "paused" in combined, \
            f"Expected 'paused' text after pressing p, got: {texts}"


# ---------------------------------------------------------------------------
# 5. Escape returns to title
# ---------------------------------------------------------------------------

def test_escape_returns_to_title():
    """Pressing Escape from playing state returns to title."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("Enter")
        time.sleep(0.05)
        h.send_render(width=W, height=H)
        h.send_key("Escape")
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        assert any("snake" in t.lower() for t in texts), \
            f"Expected title screen after Escape, got: {texts}"


# ---------------------------------------------------------------------------
# 6. Resize
# ---------------------------------------------------------------------------

def test_resize_doesnt_crash():
    """Sending different width/height values doesn't crash the app."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("Enter")
        time.sleep(0.05)
        for (w, h_dim) in [(400, 300), (1200, 900), (600, 400)]:
            frame = h.send_render(width=w, height=h_dim)
            assert len(frame) > 0, f"No draw commands after resize to {w}x{h_dim}"
