"""Tests for the Wikipedia Plexi app."""
import sys
import os

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from plexi_test import AppTestHarness, test_app_lifecycle, test_app_key_handling

APP_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "wikipedia.py")


def test_lifecycle():
    """App starts, renders, and shuts down without crashing."""
    test_app_lifecycle(APP_PATH)


def test_initial_render():
    """Initial render produces text draw commands with search UI."""
    with AppTestHarness(APP_PATH) as h:
        h.send_init()
        frames = h.send_render()
        texts = h.find_texts(frames)
        assert len(texts) > 0, "App should render at least one text element"
        # Should show the Wikipedia header
        h.assert_text_visible("Wikipedia", frames)
    print("PASS: test_initial_render")


def test_key_handling():
    """Common keys don't crash the app."""
    test_app_key_handling(APP_PATH, ["j", "k", "Backspace", "Escape"])


def test_typing_query():
    """Typing characters builds a search query without crashing."""
    with AppTestHarness(APP_PATH) as h:
        h.send_init()
        h.send_render()
        for char in "python":
            h.send_key(char)
        frames = h.send_render()
        texts = h.find_texts(frames)
        assert len(texts) > 0
    print("PASS: test_typing_query")


def test_backspace():
    """Backspace removes characters from query."""
    with AppTestHarness(APP_PATH) as h:
        h.send_init()
        h.send_render()
        for char in "test":
            h.send_key(char)
        h.send_key("Backspace")
        h.send_key("Backspace")
        frames = h.send_render()
        texts = h.find_texts(frames)
        assert len(texts) > 0
    print("PASS: test_backspace")


def test_resize():
    """App handles different render dimensions."""
    with AppTestHarness(APP_PATH) as h:
        h.send_init(width=800, height=600)
        frames1 = h.send_render()
        assert len(h.find_texts(frames1)) > 0

        frames2 = h.send_render(width=400, height=300)
        assert len(h.find_texts(frames2)) > 0

        frames3 = h.send_render(width=1200, height=900)
        assert len(h.find_texts(frames3)) > 0
    print("PASS: test_resize")


def test_navigation_keys():
    """j/k for selection navigation don't crash."""
    with AppTestHarness(APP_PATH) as h:
        h.send_init()
        h.send_render()
        h.send_key("j")
        h.send_render()
        h.send_key("j")
        h.send_render()
        h.send_key("k")
        frames = h.send_render()
        assert len(h.find_texts(frames)) > 0
    print("PASS: test_navigation_keys")


if __name__ == "__main__":
    test_lifecycle()
    test_initial_render()
    test_key_handling()
    test_typing_query()
    test_backspace()
    test_resize()
    test_navigation_keys()
    print("All wikipedia tests passed!")
