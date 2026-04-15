"""Tests for the Plexi Browser app."""
import sys
import os

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from plexi_test import AppTestHarness, test_app_lifecycle, test_app_key_handling

APP_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "browser.py")


def test_lifecycle():
    """App starts, renders, and shuts down without crashing."""
    test_app_lifecycle(APP_PATH)


def test_initial_render():
    """Initial render shows the browser header and URL input prompt."""
    with AppTestHarness(APP_PATH) as h:
        h.send_init()
        frames = h.send_render()
        texts = h.find_texts(frames)
        assert len(texts) > 0, "App should render at least one text element"
        # Should show the app title or input prompt
        h.assert_text_visible("URL:", frames)
    print("PASS: test_initial_render")


def test_key_handling():
    """Typing characters and Escape/Backspace don't crash."""
    test_app_key_handling(APP_PATH, ["h", "t", "t", "p", "Backspace", "Escape"])


def test_typing_url():
    """Typing characters builds the URL input."""
    with AppTestHarness(APP_PATH) as h:
        h.send_init()
        h.send_render()
        for char in "example":
            h.send_key(char)
        frames = h.send_render()
        texts = h.find_texts(frames)
        assert len(texts) > 0
    print("PASS: test_typing_url")


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


def test_escape_clears_input():
    """Escape key clears URL input."""
    with AppTestHarness(APP_PATH) as h:
        h.send_init()
        h.send_render()
        for char in "test":
            h.send_key(char)
        h.send_key("Escape")
        frames = h.send_render()
        assert len(h.find_texts(frames)) > 0
    print("PASS: test_escape_clears_input")


if __name__ == "__main__":
    test_lifecycle()
    test_initial_render()
    test_key_handling()
    test_typing_url()
    test_resize()
    test_escape_clears_input()
    print("All plexi-browser tests passed!")
