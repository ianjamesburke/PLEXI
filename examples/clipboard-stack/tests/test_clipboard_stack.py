"""
Tests for the Clipboard Stack Plexi app.

Each test uses a temporary HOME so the persisted clipboard_stack.json
is isolated from the user's real data.

Run:
    python3 -m pytest examples/clipboard-stack/tests/ -v
"""

import sys
import os
import tempfile

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"))

from plexi_test import AppTestHarness

ENTRY = os.path.join(os.path.dirname(__file__), "..", "clipboard_stack.py")
W, H = 800, 600


def make_harness():
    tmp = tempfile.mkdtemp(prefix="plexi-clip-test-")
    saved_home = os.environ.get("HOME")
    os.environ["HOME"] = tmp
    try:
        h = AppTestHarness(ENTRY)
    finally:
        if saved_home is not None:
            os.environ["HOME"] = saved_home
        else:
            os.environ.pop("HOME", None)
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
    """The Clipboard Stack header is rendered."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        texts = " ".join(h.find_texts(frame))
        assert "Clipboard Stack" in texts, f"Expected header, got: {texts}"


def test_help_hint_visible():
    """The bottom help hint is rendered when not in search mode."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        texts = " ".join(h.find_texts(frame))
        assert "navigate" in texts, f"Expected help hint with 'navigate', got: {texts}"


# ---------------------------------------------------------------------------
# 2. Search mode
# ---------------------------------------------------------------------------

def test_slash_enters_search_mode():
    """Pressing '/' enters search mode and shows the search prompt."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("/")
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        # Search bar shows a leading "/" character
        assert any(t == "/" for t in texts), \
            f"Expected '/' search prompt, got: {texts}"


def test_typing_in_search_mode():
    """Typed characters appear in the search query display."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("/")
        for c in ("a", "b", "c"):
            h.send_key(c)
        frame = h.send_render(width=W, height=H)
        texts = " ".join(h.find_texts(frame))
        assert "abc" in texts, f"Expected 'abc' in search query, got: {texts}"


def test_escape_exits_search_mode():
    """Escape leaves search mode and the help hint reappears."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("/")
        h.send_key("a")
        h.send_render(width=W, height=H)
        h.send_key("Escape")
        frame = h.send_render(width=W, height=H)
        texts = " ".join(h.find_texts(frame))
        assert "navigate" in texts, \
            f"Expected help hint after exiting search, got: {texts}"


# ---------------------------------------------------------------------------
# 3. Navigation keys (no crash with empty stack)
# ---------------------------------------------------------------------------

def test_navigation_keys_no_crash():
    """Arrow / hjkl keys don't crash on an empty stack."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        for k in ("j", "k", "ArrowDown", "ArrowUp"):
            h.send_key(k)
            frame = h.send_render(width=W, height=H)
            assert len(frame) > 0, f"No frame after key {k}"


def test_action_keys_no_crash_on_empty():
    """Action keys (d/p/Enter/y) don't crash when the stack is empty."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        for k in ("d", "p", "Enter", "y"):
            h.send_key(k)
            frame = h.send_render(width=W, height=H)
            assert len(frame) > 0


# ---------------------------------------------------------------------------
# 4. Resize + shutdown
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
