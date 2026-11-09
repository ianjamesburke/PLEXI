"""Focus Visibility Probe: host focus/visibility events drive the view."""
import json
from pathlib import Path

from plexi_sdk.testing import AppHarness

APP = Path(__file__).resolve().parent.parent / "main.py"


def _rendered(h):
    return json.dumps(h.run(1))


def test_view_follows_focus_and_visibility_events():
    with AppHarness(APP, timeout=3.0) as h:
        assert "not focused" in _rendered(h)
        h.set_visible(True)
        h.set_focus(True)
        text = _rendered(h)
        assert '"focused"' in text and "history: shown, focus gained" in text
        h.set_focus(False)
        h.set_visible(False)
        text = _rendered(h)
        assert "not focused" in text and '"hidden"' in text
        assert "shown, focus gained, focus lost, hidden" in text
