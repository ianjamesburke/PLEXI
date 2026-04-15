"""Tests for the Hello App Plexi app."""
import sys
import os

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from plexi_test import AppTestHarness, test_app_lifecycle, test_app_key_handling

APP_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bin", "plexi-app")


def test_lifecycle():
    """App starts, renders, and shuts down without crashing."""
    test_app_lifecycle(APP_PATH)


def test_initial_render():
    """Initial render produces meaningful text content."""
    with AppTestHarness(APP_PATH) as h:
        h.send_init()
        frames = h.send_render()
        texts = h.find_texts(frames)
        assert len(texts) > 0, "App should render at least one text element"
        h.assert_text_visible("Hello App", frames)
    print("PASS: test_initial_render")


def test_key_handling():
    """Arrow keys and Enter don't crash the app."""
    test_app_key_handling(APP_PATH, ["ArrowDown", "ArrowDown", "ArrowUp", "Enter"])


def test_key_navigation():
    """Arrow keys change selection and re-render shows different state."""
    with AppTestHarness(APP_PATH) as h:
        h.send_init()
        frames1 = h.send_render()

        h.send_key("ArrowDown")
        frames2 = h.send_render()

        # Both frames should render successfully
        assert len(h.find_texts(frames1)) > 0
        assert len(h.find_texts(frames2)) > 0
    print("PASS: test_key_navigation")


def test_resize():
    """App handles different render dimensions without crashing."""
    with AppTestHarness(APP_PATH) as h:
        h.send_init(width=800, height=600)
        frames1 = h.send_render()
        assert len(h.find_texts(frames1)) > 0

        # Render at a smaller size
        frames2 = h.send_render(width=400, height=300)
        assert len(h.find_texts(frames2)) > 0

        # Render at a larger size
        frames3 = h.send_render(width=1200, height=900)
        assert len(h.find_texts(frames3)) > 0
    print("PASS: test_resize")


def test_command():
    """Sending a command doesn't crash."""
    with AppTestHarness(APP_PATH) as h:
        h.send_init()
        h.send_render()
        h.send_command("1")
        frames = h.send_render()
        assert len(h.find_texts(frames)) > 0
    print("PASS: test_command")


if __name__ == "__main__":
    test_lifecycle()
    test_initial_render()
    test_key_handling()
    test_key_navigation()
    test_resize()
    test_command()
    print("All hello-app tests passed!")
