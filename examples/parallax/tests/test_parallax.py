"""Tests for the Parallax viewer Plexi app."""
import os
import shutil
import sys
import tempfile
import textwrap

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from plexi_test import AppTestHarness, test_app_lifecycle  # noqa: E402

APP_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "parallax.py")


def _make_project_dir(with_manifest: bool = True) -> str:
    """Create a temp dir that looks like a Parallax project."""
    root = tempfile.mkdtemp(prefix="plexi-parallax-test-")
    os.makedirs(os.path.join(root, ".parallax"), exist_ok=True)
    os.makedirs(os.path.join(root, "stills"), exist_ok=True)
    os.makedirs(os.path.join(root, "output"), exist_ok=True)
    if with_manifest:
        manifest = textwrap.dedent(
            """\
            project: "test project"
            scenes:
              - number: 1
                title: "opening shot"
                duration: 2.5
              - number: 2
                title: "closing shot"
                duration: 3.0
            """
        )
        with open(os.path.join(root, ".parallax", "manifest.yaml"), "w") as f:
            f.write(manifest)
    return root


def test_lifecycle():
    """App starts, renders, and shuts down without crashing."""
    test_app_lifecycle(APP_PATH)


def test_renders_without_manifest():
    """Viewer shows friendly onboarding text when no manifest exists."""
    root = _make_project_dir(with_manifest=False)
    try:
        with AppTestHarness(APP_PATH, launch_dir=root) as h:
            h.send_init()
            frames = h.send_render()
            texts = h.find_texts(frames)
            assert len(texts) > 0, "expected at least one text draw command"
            # Must render the header with the project name.
            h.assert_text_visible("Parallax", frames)
            # Onboarding hint should mention the CLI invocation.
            h.assert_text_visible("parallax run", frames)
        print("PASS: test_renders_without_manifest")
    finally:
        shutil.rmtree(root, ignore_errors=True)


def test_renders_with_manifest():
    """Viewer parses a stub manifest and renders scene titles."""
    root = _make_project_dir(with_manifest=True)
    try:
        with AppTestHarness(APP_PATH, launch_dir=root) as h:
            h.send_init()
            frames = h.send_render()
            texts = h.find_texts(frames)
            assert len(texts) > 0
            h.assert_text_visible("Parallax", frames)
            h.assert_text_visible("Scenes", frames)
            # Parsed scene titles should appear in list items or text draws.
            all_text = " ".join(texts)
            in_list = any(
                "opening shot" in (item.get("label", "") or "")
                for cmd in frames
                if cmd.get("type") == "list"
                for item in cmd.get("items", [])
            )
            assert "opening shot" in all_text or in_list, (
                "parsed scene title should be rendered"
            )
        print("PASS: test_renders_with_manifest")
    finally:
        shutil.rmtree(root, ignore_errors=True)


def test_resize():
    """Viewer handles multiple render sizes without crashing."""
    root = _make_project_dir(with_manifest=True)
    try:
        with AppTestHarness(APP_PATH, launch_dir=root) as h:
            h.send_init(width=1200, height=900)
            h.send_render()
            h.send_render(width=800, height=600)
            frames = h.send_render(width=400, height=300)
            assert len(h.find_texts(frames)) > 0
        print("PASS: test_resize")
    finally:
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    test_lifecycle()
    test_renders_without_manifest()
    test_renders_with_manifest()
    test_resize()
    print("All parallax tests passed!")
