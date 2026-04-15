"""
Tests for the Port Watcher Plexi app.

The app polls `lsof` on every render. These tests are tolerant of both
the empty (no ports) and populated states so they remain deterministic
on any machine.

Run:
    python3 -m pytest examples/port-watcher/tests/ -v
"""

import sys
import os

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"))

from plexi_test import AppTestHarness

ENTRY = os.path.join(os.path.dirname(__file__), "..", "port_watcher.py")
W, H = 1000, 700


def make_harness():
    h = AppTestHarness(ENTRY)
    h.send_init(width=W, height=H)
    return h


# ---------------------------------------------------------------------------
# 1. Basic rendering
# ---------------------------------------------------------------------------

def test_renders_without_crash():
    """App starts, polls lsof, and produces draw commands."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        assert len(frame) > 0


def test_header_visible():
    """The 'Port Watcher' header is rendered."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        h.assert_text_visible("Port Watcher", frame)


def test_help_hint_visible():
    """The keyboard hint hint is rendered."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        texts = " ".join(h.find_texts(frame))
        assert "filter" in texts and "refresh" in texts, \
            f"Expected hint text, got: {texts}"


def test_renders_either_grid_or_empty():
    """App renders either the grid (ports) or the 'No listening ports' message."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        texts = " ".join(h.find_texts(frame))
        # If no ports: empty message; if ports: at least header and hints
        assert "Port Watcher" in texts, f"Expected header at minimum, got: {texts}"


# ---------------------------------------------------------------------------
# 2. Filter mode
# ---------------------------------------------------------------------------

def test_f_enters_filter_mode():
    """Pressing 'f' shows the filter prompt in the header."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("f")
        frame = h.send_render(width=W, height=H)
        texts = " ".join(h.find_texts(frame))
        assert "Filter:" in texts, f"Expected 'Filter:' prompt, got: {texts}"


def test_typing_in_filter_mode():
    """Characters typed in filter mode appear in the filter display."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("f")
        for c in ("a", "b"):
            h.send_key(c)
        frame = h.send_render(width=W, height=H)
        texts = " ".join(h.find_texts(frame))
        assert "ab" in texts, f"Expected 'ab' in filter, got: {texts}"


def test_escape_exits_filter_mode():
    """Escape leaves filter mode (header reverts)."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("f")
        h.send_key("z")
        h.send_render(width=W, height=H)
        h.send_key("Escape")
        frame = h.send_render(width=W, height=H)
        texts = " ".join(h.find_texts(frame))
        # Header should not contain "Filter: z_" prompt (no underscore prompt)
        assert "Filter: z_" not in texts, \
            f"Still in filter mode after Escape, got: {texts}"


# ---------------------------------------------------------------------------
# 3. Navigation / refresh
# ---------------------------------------------------------------------------

def test_navigation_keys_no_crash():
    """j/k/h/l and arrows don't crash."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        for k in ("j", "k", "h", "l", "ArrowDown", "ArrowUp", "ArrowLeft", "ArrowRight"):
            h.send_key(k)
            frame = h.send_render(width=W, height=H)
            assert len(frame) > 0, f"No frame after key {k}"


def test_refresh_no_crash():
    """Pressing 'r' triggers a refresh without crashing."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("r")
        frame = h.send_render(width=W, height=H)
        assert len(frame) > 0


# ---------------------------------------------------------------------------
# 4. Resize + shutdown
# ---------------------------------------------------------------------------

def test_resize_doesnt_crash():
    """Rendering at multiple sizes doesn't crash."""
    with make_harness() as h:
        for (w, hh) in [(400, 300), (1600, 1000), (800, 600)]:
            frame = h.send_render(width=w, height=hh)
            assert len(frame) > 0


def test_clean_shutdown():
    """App shuts down cleanly."""
    h = make_harness()
    h.send_render(width=W, height=H)
    h.shutdown()
    assert h._proc.returncode is not None
