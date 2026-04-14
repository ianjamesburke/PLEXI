"""
Tests for the App Store Plexi app.

Smoke test via plexi_test.AppTestHarness plus inline assertions for the
version-compare helper. No display or GUI required.

Run:
    python3 -m pytest examples/app-store/tests/ -v
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"))

from plexi_test import AppTestHarness  # noqa: E402

ENTRY = os.path.join(os.path.dirname(__file__), "..", "app_store.py")
W, H = 800, 600


def test_app_store_renders_without_crash():
    """App boots and produces draw commands on first render."""
    h = AppTestHarness(ENTRY)
    try:
        h.send_init(width=W, height=H)
        frame = h.send_render(width=W, height=H)
        assert isinstance(frame, list)
        assert len(frame) > 0, "Expected draw commands on first frame"
    finally:
        h.__exit__(None, None, None)


# ---------------------------------------------------------------------------
# Version-compare helper — direct unit assertions
# ---------------------------------------------------------------------------

def test_compare_versions_equal_newer_older():
    """compare_versions covers equal, newer-available, older-available."""
    # Import after sys.path insert above
    import importlib.util
    spec = importlib.util.spec_from_file_location(
        "app_store_module", os.path.join(os.path.dirname(__file__), "..", "app_store.py")
    )
    assert spec is not None and spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    # Don't actually run app.run() — patch out App.run before exec.
    import plexi_sdk
    original_run = plexi_sdk.App.run
    plexi_sdk.App.run = lambda self: None  # type: ignore[assignment]
    try:
        spec.loader.exec_module(mod)
    finally:
        plexi_sdk.App.run = original_run  # type: ignore[assignment]

    cmp = mod.compare_versions

    # Equal
    assert cmp("0.1.0", "0.1.0") == 0
    assert cmp("1.2.3", "1.2.3") == 0
    assert cmp("1.2", "1.2.0") == 0  # padded comparison

    # Newer available (a > b)
    assert cmp("0.2.0", "0.1.0") == 1
    assert cmp("1.0.0", "0.9.9") == 1
    assert cmp("0.1.1", "0.1.0") == 1

    # Older available (a < b)
    assert cmp("0.1.0", "0.2.0") == -1
    assert cmp("0.0.0", "0.0.1") == -1

    # Empty / missing version handled (treated as 0.0.0)
    assert cmp("", "") == 0
    assert cmp("0.1.0", "") == 1

    # Pre-release suffix is stripped (limitation — equal to base)
    assert cmp("1.2.3-alpha", "1.2.3") == 0
