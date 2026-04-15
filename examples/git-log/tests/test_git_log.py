"""Tests for the Git Log Plexi app."""
import sys
import os

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from plexi_test import AppTestHarness, test_app_lifecycle, test_app_key_handling

APP_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "git-log")

# Use a known git repo as launch_dir so the app finds commits
REPO_DIR = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(
    os.path.abspath(__file__)))))  # repo root


def test_lifecycle():
    """App starts, renders, and shuts down without crashing."""
    with AppTestHarness(APP_PATH, launch_dir=REPO_DIR) as h:
        h.send_init()
        frames = h.send_render()
        assert isinstance(frames, list)
    print("PASS: lifecycle test for git-log")


def test_initial_render():
    """Initial render shows the git log header."""
    with AppTestHarness(APP_PATH, launch_dir=REPO_DIR) as h:
        h.send_init()
        frames = h.send_render()
        texts = h.find_texts(frames)
        assert len(texts) > 0, "App should render at least one text element"
        h.assert_text_visible("git log", frames)
    print("PASS: test_initial_render")


def test_key_handling():
    """j/k navigation keys don't crash."""
    with AppTestHarness(APP_PATH, launch_dir=REPO_DIR) as h:
        h.send_init()
        h.send_render()
        for key in ["j", "j", "k", "r"]:
            h.send_key(key)
            h.send_render()
    print("PASS: key handling test for git-log")


def test_resize():
    """App handles different render dimensions."""
    with AppTestHarness(APP_PATH, launch_dir=REPO_DIR) as h:
        h.send_init(width=800, height=600)
        frames1 = h.send_render()
        assert len(h.find_texts(frames1)) > 0

        frames2 = h.send_render(width=400, height=300)
        assert len(h.find_texts(frames2)) > 0

        frames3 = h.send_render(width=1200, height=900)
        assert len(h.find_texts(frames3)) > 0
    print("PASS: test_resize")


def test_command():
    """Sending a command doesn't crash."""
    with AppTestHarness(APP_PATH, launch_dir=REPO_DIR) as h:
        h.send_init()
        h.send_render()
        h.send_command("r")
        frames = h.send_render()
        assert len(h.find_texts(frames)) > 0
    print("PASS: test_command")


if __name__ == "__main__":
    test_lifecycle()
    test_initial_render()
    test_key_handling()
    test_resize()
    test_command()
    print("All git-log tests passed!")
