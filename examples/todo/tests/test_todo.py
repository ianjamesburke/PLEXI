"""
Tests for the Todo Plexi app.

Each test uses a temporary HOME so the real ~/.plexi-alpha/todo.txt is untouched.

Run:
    python3 -m pytest examples/todo/tests/ -v
"""

import sys
import os
import tempfile

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"))

from plexi_test import AppTestHarness

ENTRY = os.path.join(os.path.dirname(__file__), "..", "todo.py")
W, H = 800, 600


def make_harness():
    """Spawn a fresh harness with HOME pointing at a temp dir so todo.txt is isolated."""
    tmp = tempfile.mkdtemp(prefix="plexi-todo-test-")
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
    """Header text 'Todo' is rendered."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        h.assert_text_visible("Todo", frame)


def test_empty_state_message():
    """With an empty todo file, the empty-state message is shown."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        texts = " ".join(h.find_texts(frame))
        assert "No todos yet" in texts, f"Expected empty state message, got: {texts}"


# ---------------------------------------------------------------------------
# 2. Add mode
# ---------------------------------------------------------------------------

def test_a_enters_add_mode():
    """Pressing 'a' shows the 'New item:' prompt."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("a")
        frame = h.send_render(width=W, height=H)
        texts = " ".join(h.find_texts(frame))
        assert "New item" in texts, f"Expected 'New item' prompt, got: {texts}"


def test_typing_in_add_mode_shows_buffer():
    """Typed characters appear in the add buffer."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("a")
        for c in ("h", "i"):
            h.send_key(c)
        frame = h.send_render(width=W, height=H)
        texts = " ".join(h.find_texts(frame))
        assert "hi" in texts, f"Expected 'hi' in buffer, got: {texts}"


def test_enter_adds_item():
    """Typing then Enter adds an entry to the list."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("a")
        for c in ("b", "u", "y", " ", "m", "i", "l", "k"):
            h.send_key(c)
        h.send_key("Enter")
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        assert any("buy milk" in t for t in texts), \
            f"Expected 'buy milk' added, got: {texts}"


def test_escape_cancels_add():
    """Escape from add mode discards the buffer."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("a")
        for c in ("x", "y", "z"):
            h.send_key(c)
        h.send_key("Escape")
        frame = h.send_render(width=W, height=H)
        texts = " ".join(h.find_texts(frame))
        assert "xyz" not in texts, f"Buffer not cleared after Escape, got: {texts}"
        # Empty state should be back
        assert "No todos yet" in texts


# ---------------------------------------------------------------------------
# 3. Toggle / delete
# ---------------------------------------------------------------------------

def test_space_toggles_done():
    """Space marks the selected todo as done (✓ checkbox appears)."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        # Add an item
        h.send_key("a")
        for c in ("a", "b", "c"):
            h.send_key(c)
        h.send_key("Enter")
        h.send_render(width=W, height=H)
        # Toggle
        h.send_key(" ")
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        # Done checkbox is "✓"
        assert any("✓" in t for t in texts), \
            f"Expected ✓ checkbox after toggling done, got: {texts}"


def test_d_deletes_item():
    """'d' removes the selected todo from the list."""
    with make_harness() as h:
        h.send_render(width=W, height=H)
        h.send_key("a")
        for c in ("z", "z", "z"):
            h.send_key(c)
        h.send_key("Enter")
        h.send_render(width=W, height=H)
        h.send_key("d")
        frame = h.send_render(width=W, height=H)
        texts = " ".join(h.find_texts(frame))
        assert "zzz" not in texts, f"Item not deleted, got: {texts}"
        assert "No todos yet" in texts, f"Expected empty state after delete, got: {texts}"


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
