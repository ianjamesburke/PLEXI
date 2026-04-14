"""
Tests for the Calculator Plexi app.

Run:
    python3 -m pytest examples/calc/tests/ -v
"""

import sys
import os

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"))

from plexi_test import AppTestHarness

ENTRY = os.path.join(os.path.dirname(__file__), "..", "calc.py")
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


def test_header_visible():
    """Header text 'Calculator' is rendered."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        h.assert_text_visible("Calculator", frame)


def test_initial_expression_is_placeholder():
    """Empty expression renders as '0' placeholder."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        assert "0" in texts, f"Expected '0' placeholder, got: {texts}"


def test_keypad_labels_visible():
    """All digits 0-9 and operators are present in the keypad."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        for label in ("1", "2", "3", "4", "5", "6", "7", "8", "9", "0",
                       "+", "-", "*", "/", "="):
            assert label in texts, f"Expected keypad label {label!r}, missing"


# ---------------------------------------------------------------------------
# 2. Digit input
# ---------------------------------------------------------------------------

def test_digit_input_updates_expression():
    """Pressing digit keys builds up the expression."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        for k in ("1", "2", "3"):
            h.send_key(k)
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        assert "123" in texts, f"Expected '123' in expression, got: {texts}"


# ---------------------------------------------------------------------------
# 3. Evaluation
# ---------------------------------------------------------------------------

def test_simple_addition():
    """2+3 then Enter → expression becomes '5'."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        for k in ("2", "+", "3"):
            h.send_key(k)
        h.send_key("Enter")
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        assert "5" in texts, f"Expected '5' result, got: {texts}"


def test_multiplication():
    """6*7 = 42."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        for k in ("6", "*", "7"):
            h.send_key(k)
        h.send_key("Enter")
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        assert "42" in texts, f"Expected '42', got: {texts}"


def test_division_by_zero_shows_error():
    """1/0 produces an error result."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        for k in ("1", "/", "0"):
            h.send_key(k)
        h.send_key("Enter")
        frame = h.send_render(width=W, height=H)
        texts = " ".join(h.find_texts(frame)).lower()
        assert "error" in texts or "division" in texts, \
            f"Expected error text, got: {texts}"


# ---------------------------------------------------------------------------
# 4. Editing
# ---------------------------------------------------------------------------

def test_backspace_deletes_last_char():
    """Backspace removes the last character of the expression."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        for k in ("1", "2", "3"):
            h.send_key(k)
        h.send_key("Backspace")
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        assert "12" in texts, f"Expected '12' after backspace, got: {texts}"


def test_escape_clears_expression():
    """Escape resets the expression to placeholder '0'."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        for k in ("4", "2"):
            h.send_key(k)
        h.send_key("Escape")
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        # '0' placeholder should be present
        assert "0" in texts, f"Expected '0' placeholder after clear, got: {texts}"
        # '42' should NOT remain visible as expression
        assert "42" not in texts, f"Expression not cleared, got: {texts}"


# ---------------------------------------------------------------------------
# 5. Resize
# ---------------------------------------------------------------------------

def test_resize_doesnt_crash():
    """Rendering at multiple sizes doesn't crash."""
    with make_harness() as h:
        for (w, hh) in [(400, 300), (1200, 900), (640, 480)]:
            frame = h.send_render(width=w, height=hh)
            assert len(frame) > 0, f"No draw commands after resize to {w}x{hh}"


# ---------------------------------------------------------------------------
# 6. Shutdown
# ---------------------------------------------------------------------------

def test_clean_shutdown():
    """App shuts down cleanly without hanging."""
    h = make_harness()
    h.send_render(width=W, height=H)
    h.shutdown()
    assert h._proc.returncode is not None, "Process did not exit after shutdown"
