"""
Tests for the JSON Viewer Plexi app.

Run:
    python3 -m pytest examples/json-viewer/tests/ -v
"""

import sys
import os
import json
import tempfile

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"))

from plexi_test import AppTestHarness

ENTRY = os.path.join(os.path.dirname(__file__), "..", "json_viewer.py")
W, H = 800, 600


def make_harness(launch_path: str | None = None):
    """Spawn json-viewer; optionally point PLEXI_LAUNCH_PATH at a file."""
    saved = os.environ.get("PLEXI_LAUNCH_PATH")
    if launch_path is not None:
        os.environ["PLEXI_LAUNCH_PATH"] = launch_path
    elif "PLEXI_LAUNCH_PATH" in os.environ:
        del os.environ["PLEXI_LAUNCH_PATH"]
    try:
        h = AppTestHarness(ENTRY)
    finally:
        if saved is not None:
            os.environ["PLEXI_LAUNCH_PATH"] = saved
        else:
            os.environ.pop("PLEXI_LAUNCH_PATH", None)
    h.send_init(width=W, height=H)
    return h


def write_sample(data) -> str:
    """Write a JSON file to a tempdir and return its path."""
    fd, path = tempfile.mkstemp(suffix=".json", prefix="plexi-jv-")
    os.close(fd)
    with open(path, "w") as f:
        json.dump(data, f)
    return path


# ---------------------------------------------------------------------------
# 1. Empty / no-file state
# ---------------------------------------------------------------------------

def test_renders_without_crash_no_file():
    """App starts with no launch path and renders the empty state."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        assert len(frame) > 0


def test_empty_state_message():
    """Without a file, the drop-target hint is shown."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        texts = " ".join(h.find_texts(frame))
        assert "Drop" in texts or "JSON file" in texts, \
            f"Expected drop hint, got: {texts}"


def test_header_visible():
    """The 'JSON Viewer' header is rendered."""
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        h.assert_text_visible("JSON Viewer", frame)


# ---------------------------------------------------------------------------
# 2. With a sample file
# ---------------------------------------------------------------------------

SAMPLE = {
    "name": "plexi",
    "version": 1,
    "tags": ["alpha", "beta"],
    "config": {"debug": True, "port": 8080},
}


def test_loads_sample_file():
    """Launching with a JSON file shows the root key in the header."""
    path = write_sample(SAMPLE)
    try:
        with make_harness(launch_path=path) as h:
            frame = h.send_render(width=W, height=H)
            texts = " ".join(h.find_texts(frame))
            base = os.path.basename(path)
            assert base in texts, f"Expected filename {base!r} in header, got: {texts}"
    finally:
        os.unlink(path)


def test_renders_top_level_keys():
    """Top-level keys from the JSON appear in the rendered text."""
    path = write_sample(SAMPLE)
    try:
        with make_harness(launch_path=path) as h:
            frame = h.send_render(width=W, height=H)
            texts = h.find_texts(frame)
            for key in ("name", "version", "tags", "config"):
                assert key in texts, f"Expected key {key!r} in tree, got: {texts}"
    finally:
        os.unlink(path)


def test_string_value_visible():
    """String values are rendered with surrounding quotes."""
    path = write_sample(SAMPLE)
    try:
        with make_harness(launch_path=path) as h:
            frame = h.send_render(width=W, height=H)
            texts = " ".join(h.find_texts(frame))
            assert '"plexi"' in texts, f"Expected '\"plexi\"' in output, got: {texts}"
    finally:
        os.unlink(path)


def test_navigation_keys_no_crash():
    """j/k arrow keys don't crash with a loaded file."""
    path = write_sample(SAMPLE)
    try:
        with make_harness(launch_path=path) as h:
            h.send_render(width=W, height=H)
            for k in ("j", "j", "k", "ArrowDown", "ArrowUp"):
                h.send_key(k)
                frame = h.send_render(width=W, height=H)
                assert len(frame) > 0
    finally:
        os.unlink(path)


def test_collapse_expand_toggle():
    """Pressing Space on a container node toggles expansion (output text changes)."""
    path = write_sample(SAMPLE)
    try:
        with make_harness(launch_path=path) as h:
            frame_before = h.send_render(width=W, height=H)
            texts_before = h.find_texts(frame_before)
            # Cursor starts at root; Space toggles its expansion
            h.send_key(" ")
            frame_after = h.send_render(width=W, height=H)
            texts_after = h.find_texts(frame_after)
            assert texts_before != texts_after, \
                "Expected text output to change after toggling expand/collapse"
    finally:
        os.unlink(path)


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
