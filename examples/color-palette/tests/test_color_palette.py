"""
Tests for the Color Palette Plexi app.

Run:
    python3 -m pytest examples/color-palette/tests/ -v
"""

import sys
import os

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"))

from plexi_test import AppTestHarness

ENTRY = os.path.join(os.path.dirname(__file__), "..", "color_palette.py")
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
    """Header text is rendered."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        texts = " ".join(h.find_texts(frame))
        assert "Catppuccin Mocha" in texts, f"Expected header, got: {texts}"


def test_color_names_visible():
    """Several known color names from the palette are rendered."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        for name in ("Rosewater", "Mauve", "Blue", "Green"):
            assert name in texts, f"Expected color name {name!r}, got: {texts}"


def test_hex_values_visible():
    """At least one hex value from the palette is rendered as text."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        # Rosewater
        assert "#f5e0dc" in texts, f"Expected hex value '#f5e0dc', got: {texts}"


def test_swatch_rects_present():
    """Multiple rects exist for swatches (palette has 26 entries)."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        rects = [c for c in frame if c.get("type") == "rect"]
        # At least one rect per palette entry, plus header/bg
        assert len(rects) >= 26, f"Expected >= 26 rects, got {len(rects)}"


# ---------------------------------------------------------------------------
# 2. Navigation (key handling — should not crash)
# ---------------------------------------------------------------------------

def test_navigation_keys_no_crash():
    """All navigation keys produce a frame without crashing."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        for k in ("j", "k", "h", "l",
                   "ArrowDown", "ArrowUp", "ArrowLeft", "ArrowRight"):
            h.send_key(k)
            frame = h.send_render(width=W, height=H)
            assert len(frame) > 0, f"No frame after key {k}"


def test_enter_copies_without_crash():
    """Pressing Enter triggers copy code path without crashing."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("Enter")
        frame = h.send_render(width=W, height=H)
        assert len(frame) > 0


def test_space_copies_without_crash():
    """Pressing Space also triggers the copy path."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key(" ")
        frame = h.send_render(width=W, height=H)
        assert len(frame) > 0


# ---------------------------------------------------------------------------
# 3. Resize + shutdown
# ---------------------------------------------------------------------------

def test_resize_doesnt_crash():
    """Rendering at multiple sizes doesn't crash."""
    with make_harness() as h:
        for (w, hh) in [(400, 300), (1200, 900), (640, 480)]:
            frame = h.send_render(width=w, height=hh)
            assert len(frame) > 0


def test_clean_shutdown():
    """App shuts down cleanly."""
    h = make_harness()
    h.send_render(width=W, height=H)
    h.shutdown()
    assert h._proc.returncode is not None
