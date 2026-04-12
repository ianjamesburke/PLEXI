"""Smoke tests for the Wikipedia Plexi app."""
import sys
import os

# Add parent dir to path so we can import plexi_test
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from plexi_test import AppTestHarness, test_app_lifecycle

def test_lifecycle():
    """Verify app starts, renders, and shuts down without crashing."""
    app_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "wikipedia.py")
    test_app_lifecycle(app_path)

def test_renders_content():
    """Verify initial render produces text draw commands."""
    app_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "wikipedia.py")
    with AppTestHarness(app_path) as h:
        h.send_init()
        frames = h.send_render()
        texts = h.find_texts(frames)
        assert len(texts) > 0, "App should render at least one text element"

if __name__ == "__main__":
    test_lifecycle()
    test_renders_content()
    print("All tests passed!")
