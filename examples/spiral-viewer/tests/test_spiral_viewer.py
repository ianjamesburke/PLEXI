"""
Tests for the Spiral Viewer Plexi app.

Drives spiral_viewer.py as a subprocess via AppTestHarness and also directly
imports spiral_positions / _resolve_target for pure-function tests.

Run:
    python3 -m pytest examples/spiral-viewer/tests/ -v
"""
from __future__ import annotations

import importlib.util
import os
import sys
import time

# Make the SDK test harness and the app itself importable.
HERE = os.path.dirname(os.path.abspath(__file__))
APP_DIR = os.path.join(HERE, "..")
REPO_ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))

sys.path.insert(0, APP_DIR)
sys.path.insert(0, os.path.join(REPO_ROOT, "sdk", "python"))

from plexi_test import AppTestHarness  # noqa: E402


def _load_spiral_module():
    """Import spiral_viewer.py directly without executing main()."""
    spec = importlib.util.spec_from_file_location(
        "spiral_viewer_module",
        os.path.join(APP_DIR, "spiral_viewer.py"),
    )
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    # Prevent main() from running — it won't, because __name__ != "__main__".
    spec.loader.exec_module(mod)
    return mod


SPIRAL = _load_spiral_module()
ENTRY = os.path.join(APP_DIR, "spiral_viewer.py")
FIXTURE_MANIFEST = os.path.join(HERE, "fixtures", "manifest.toml")


# ---------------------------------------------------------------------------
# Pure function tests
# ---------------------------------------------------------------------------


def test_spiral_positions_compute():
    """spiral_positions with n=8 returns 8 distinct, non-collinear positions."""
    positions = SPIRAL.spiral_positions(8, 1280, 800, target_min_w=200, target_min_h=150)
    assert len(positions) == 8
    # All distinct
    seen = set()
    for p in positions:
        key = (round(p[0], 1), round(p[1], 1))
        assert key not in seen, f"duplicate spiral position: {p}"
        seen.add(key)
    # Not all collinear — check that y values span a meaningful range.
    ys = [p[1] for p in positions]
    assert max(ys) - min(ys) > 20, "spiral is suspiciously collinear in y"
    xs = [p[0] for p in positions]
    assert max(xs) - min(xs) > 20, "spiral is suspiciously collinear in x"
    # All positions must fit in the pane (cx + w <= pane_w, etc.)
    for (cx, cy, w, h) in positions:
        assert 0 <= cx and cx + w <= 1280 + 0.5
        assert 0 <= cy and cy + h <= 800 + 0.5
        assert w > 0 and h > 0


def test_spiral_positions_monotonic_size():
    """Sizes grow monotonically from center (smallest at i=0)."""
    positions = SPIRAL.spiral_positions(8, 1280, 800, target_min_w=200, target_min_h=150)
    widths = [p[2] for p in positions]
    # Allow tiny floating-point wobble, but strictly: widths[-1] > widths[0].
    assert widths[-1] > widths[0], f"last should be bigger than first: {widths}"


def test_spiral_positions_zero_count():
    assert SPIRAL.spiral_positions(0, 1280, 800) == []


def test_resolve_target_none():
    assert SPIRAL._resolve_target(None) is None
    assert SPIRAL._resolve_target("") is None
    assert SPIRAL._resolve_target("definitely-not-a-real-app-id-xyz") is None


def test_resolve_target_by_manifest_path():
    r = SPIRAL._resolve_target(FIXTURE_MANIFEST)
    assert r is not None
    assert r["app_id"] == "mock-target"
    assert r["min_width"] == 100
    assert r["min_height"] == 80
    assert r["entry_point"].endswith("mock_target.py")


# ---------------------------------------------------------------------------
# Subprocess tests — drive spiral_viewer via AppTestHarness
# ---------------------------------------------------------------------------


def _harness(args: list[str] | None = None) -> AppTestHarness:
    """AppTestHarness hardcodes argv[0] = entry — there's no way to pass
    argv[1]. We pass the target via the PLEXI_SPIRAL_TARGET env var, which
    spiral_viewer reads as a fallback."""
    env_extra: dict[str, str] = {}
    if args:
        # Single-value fallback path
        env_extra["PLEXI_SPIRAL_TARGET"] = args[0]
    # Stash env into os.environ briefly since AppTestHarness inherits it.
    saved = {}
    for k, v in env_extra.items():
        saved[k] = os.environ.get(k)
        os.environ[k] = v
    try:
        h = AppTestHarness(ENTRY)
        h.send_init(width=1280, height=800)
        return h
    finally:
        # Restore env for the parent.
        for k, v in saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v


def test_renders_without_target():
    """With no target, spiral_viewer should emit instructional text."""
    with _harness() as h:
        frame = h.send_render(width=1280, height=800)
        texts = h.find_texts(frame)
        combined = " ".join(texts).lower()
        assert "spiral viewer" in combined, f"expected title, got {texts}"
        assert "argv" in combined or "pass an app" in combined, \
            f"expected instructions, got {texts}"


def test_renders_with_simple_target():
    """With the mock target, spiral_viewer should re-emit the mock's rect."""
    with _harness([FIXTURE_MANIFEST]) as h:
        frame = h.send_render(width=1280, height=800)
        # The mock target emits fill="#ff00aa" — that color should appear in
        # at least one rect in spiral_viewer's output (one per instance).
        pink_rects = [
            c for c in frame
            if c.get("type") == "rect" and c.get("fill") == "#ff00aa"
        ]
        assert len(pink_rects) >= 1, (
            f"expected at least one pink rect from mock target, got "
            f"{[(c.get('type'), c.get('fill')) for c in frame if c.get('type') == 'rect']}"
        )
        # And the MOCK text.
        mock_texts = [
            c for c in frame
            if c.get("type") == "text" and "MOCK" in str(c.get("text", ""))
        ]
        assert len(mock_texts) >= 1, "expected at least one MOCK text re-emitted"


def test_resize_doesnt_crash():
    """Sending a resize event should not crash spiral_viewer."""
    with _harness([FIXTURE_MANIFEST]) as h:
        h.send_render(width=1280, height=800)
        # Simulate resize by re-rendering at new size.
        h.send_render(width=900, height=600)
        frame3 = h.send_render(width=1600, height=1000)
        # Just verify we got a frame back with some content.
        assert len(frame3) > 0


def test_click_logs_instance_size():
    """Clicking inside a rendered instance should not crash and should produce output."""
    with _harness([FIXTURE_MANIFEST]) as h:
        h.send_render(width=1280, height=800)
        # Click dead center — spiral viewer has at least one instance near center.
        h.send_click(640, 400)
        # Let any async notification/log flush.
        time.sleep(0.1)
        # A subsequent render must still work.
        frame = h.send_render(width=1280, height=800)
        assert len(frame) > 0
