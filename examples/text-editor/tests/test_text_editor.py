"""
Tests for the external Python text editor Plexi app.

Uses plexi_test.AppTestHarness to drive the app via the JSON protocol.
No display or GUI required — all assertions are against draw commands.

Run:
    python3 -m pytest examples/text-editor/tests/ -v
"""
from __future__ import annotations

import os
import sys
import tempfile

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(
    0,
    os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"),
)

from plexi_test import AppTestHarness  # noqa: E402

ENTRY = os.path.join(os.path.dirname(__file__), "..", "text_editor.py")
W, H = 800, 600


def _isolated_env() -> dict:
    """Return an env that redirects HOME so scratch/autosave doesn't touch the real home."""
    tmp_home = tempfile.mkdtemp(prefix="plexi-text-editor-test-")
    env = os.environ.copy()
    env["HOME"] = tmp_home
    return env


def make_harness(argv_extra: list[str] | None = None) -> AppTestHarness:
    """Spawn the editor in a fresh sandboxed HOME."""
    import subprocess  # local import — the harness already uses subprocess
    # Subclass-lite: reuse AppTestHarness but patch env + argv before spawn.
    h = object.__new__(AppTestHarness)
    h.entry_point = os.path.abspath(ENTRY)
    h.launch_dir = tempfile.mkdtemp(prefix="plexi-text-editor-launch-")
    h._last_frames = []  # type: ignore[attr-defined]
    import queue  # noqa: E402
    import threading  # noqa: E402
    h._output_queue = queue.Queue()  # type: ignore[attr-defined]
    h._stderr_lines = []  # type: ignore[attr-defined]
    h._closed = False  # type: ignore[attr-defined]

    env = _isolated_env()
    env["PYTHONUNBUFFERED"] = "1"

    cmd = [sys.executable, h.entry_point]
    if argv_extra:
        cmd.extend(argv_extra)

    h._proc = subprocess.Popen(  # type: ignore[attr-defined]
        cmd,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        cwd=h.launch_dir,
        env=env,
    )
    h._reader_thread = threading.Thread(  # type: ignore[attr-defined]
        target=h._read_stdout, daemon=True
    )
    h._reader_thread.start()
    h._stderr_thread = threading.Thread(  # type: ignore[attr-defined]
        target=h._read_stderr, daemon=True
    )
    h._stderr_thread.start()

    h.send_init(width=W, height=H)
    return h


# ---------------------------------------------------------------------------
# 1. Blank state renders
# ---------------------------------------------------------------------------


def test_renders_blank_state():
    with make_harness() as h:
        frame = h.send_render(width=W, height=H)
        # Background rect + status bar rect + status text, at minimum.
        rects = [c for c in frame if c.get("type") == "rect"]
        assert len(rects) >= 2, f"Expected >= 2 rects, got {len(rects)}"
        texts = h.find_texts(frame)
        combined = " ".join(texts)
        assert "[scratch]" in combined, f"Expected [scratch] in status, got: {texts}"


# ---------------------------------------------------------------------------
# 2. Loads file from argv
# ---------------------------------------------------------------------------


def test_loads_file_from_arg():
    with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as tf:
        tf.write("hello\nworld\n")
        path = tf.name
    try:
        with make_harness(argv_extra=[path]) as h:
            frame = h.send_render(width=W, height=H)
            texts = h.find_texts(frame)
            combined = " ".join(texts)
            assert "hello" in combined, f"Expected 'hello' in file body, got: {texts}"
            assert "world" in combined, f"Expected 'world' in file body, got: {texts}"
            assert os.path.basename(path) in combined or path[-30:] in combined
    finally:
        os.unlink(path)


# ---------------------------------------------------------------------------
# 3. Typing inserts text
# ---------------------------------------------------------------------------


def test_typing_inserts_text():
    with make_harness() as h:
        h.send_render(width=W, height=H)
        for ch in "abc":
            h.send_key(ch)
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        combined = "".join(texts)
        assert "abc" in combined, f"Expected 'abc' in texts, got: {texts}"


# ---------------------------------------------------------------------------
# 4. Backspace deletes
# ---------------------------------------------------------------------------


def test_backspace_deletes():
    with make_harness() as h:
        h.send_render(width=W, height=H)
        for ch in "hello":
            h.send_key(ch)
        h.send_key("Backspace")
        h.send_key("Backspace")
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        combined = "".join(texts)
        assert "hel" in combined, f"Expected 'hel' after two backspaces, got: {texts}"
        assert "hello" not in combined, f"Did not expect 'hello' after backspace, got: {texts}"


# ---------------------------------------------------------------------------
# 5. Arrow keys move cursor (don't crash; status bar col updates)
# ---------------------------------------------------------------------------


def test_arrow_keys_move_cursor():
    with make_harness() as h:
        h.send_render(width=W, height=H)
        for ch in "ab":
            h.send_key(ch)
        h.send_render(width=W, height=H)
        # Move left twice, down once — cursor should clamp, no crash.
        for key in ("ArrowLeft", "ArrowLeft", "ArrowDown", "ArrowRight"):
            h.send_key(key)
            frame = h.send_render(width=W, height=H)
            assert len(frame) > 0, f"No draw commands after {key}"


# ---------------------------------------------------------------------------
# 6. Undo / redo round trip
# ---------------------------------------------------------------------------


def test_undo_redo_round_trip():
    with make_harness() as h:
        h.send_render(width=W, height=H)
        for ch in "hello":
            h.send_key(ch)
        # Force a non-coalesced edit by sending a space then more chars.
        h.send_key(" ")
        for ch in "world":
            h.send_key(ch)
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        assert any("hello" in t and "world" in t for t in texts) or (
            "hello world" in "".join(texts)
        ), f"Expected both words before undo, got: {texts}"

        # Undo a few times.
        for _ in range(5):
            h.send_key("z", command=True)
        frame = h.send_render(width=W, height=H)
        texts_after_undo = h.find_texts(frame)
        combined = "".join(texts_after_undo)
        # At minimum the tail should have changed.
        assert combined != "".join(texts), "Undo had no visible effect"

        # Redo the undone edits.
        for _ in range(5):
            h.send_key("z", command=True, shift=True)
        frame = h.send_render(width=W, height=H)
        texts_after_redo = h.find_texts(frame)
        assert "".join(texts_after_redo) == "".join(texts), \
            f"Redo did not restore original state:\n  before: {texts}\n  after:  {texts_after_redo}"


# ---------------------------------------------------------------------------
# 7. Find bar highlights matches
# ---------------------------------------------------------------------------


def test_find_highlights_matches():
    with make_harness() as h:
        h.send_render(width=W, height=H)
        for ch in "foo bar foo baz":
            h.send_key(ch)
        h.send_render(width=W, height=H)
        # Open find bar with Cmd+F
        h.send_key("f", command=True)
        h.send_render(width=W, height=H)
        # Type query 'foo'
        for ch in "foo":
            h.send_key(ch)
        frame = h.send_render(width=W, height=H)
        texts = h.find_texts(frame)
        combined = " ".join(texts)
        # The find info string '2/2' should appear somewhere in the bar.
        assert "2/2" in combined or "1/2" in combined, \
            f"Expected match count 2/2, got: {texts}"


# ---------------------------------------------------------------------------
# 8. Resize doesn't crash
# ---------------------------------------------------------------------------


def test_resize_doesnt_crash():
    with make_harness() as h:
        h.send_render(width=W, height=H)
        for ch in "resize test":
            h.send_key(ch)
        for (w, hdim) in [(400, 300), (1200, 900), (600, 450), (320, 200)]:
            frame = h.send_render(width=w, height=hdim)
            assert len(frame) > 0, f"No draw commands after resize to {w}x{hdim}"


# ---------------------------------------------------------------------------
# 9. Save writes file
# ---------------------------------------------------------------------------


def test_save_to_file():
    with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as tf:
        tf.write("starter\n")
        path = tf.name
    try:
        with make_harness(argv_extra=[path]) as h:
            h.send_render(width=W, height=H)
            for ch in "X":
                h.send_key(ch)
            h.send_key("s", command=True)
            h.send_render(width=W, height=H)
        # After shutdown, the file should contain the X we inserted.
        with open(path) as f:
            content = f.read()
        assert "X" in content, f"Expected 'X' saved to file, got: {content!r}"
    finally:
        os.unlink(path)
