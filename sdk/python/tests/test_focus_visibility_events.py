"""Per-pane focus and visibility wire events reach `update()` as typed events.

The host sends `focus_gained` / `focus_lost` / `pane_shown` / `pane_hidden`
(stint 1180). The runtime must decode each into its `plexi_sdk.events` class,
in arrival order, and `AppHarness` must be able to inject them.
"""

import textwrap
from pathlib import Path

from plexi_sdk.testing import AppHarness
from test_v3_runtime_regression import (
    _collect_until,
    _find_events,
    _init_app,
    _send_event,
    _spawn_v3_app,
)


def _write_probe_app(tmp_path: Path) -> Path:
    app = tmp_path / "app.py"
    app.write_text(textwrap.dedent("""
        from plexi_sdk.effects import SetStatus
        from plexi_sdk.events import FocusGained, FocusLost, PaneHidden, PaneShown
        from plexi_sdk.ui import Text

        _NAMES = {
            FocusGained: "gained",
            FocusLost: "lost",
            PaneShown: "shown",
            PaneHidden: "hidden",
        }
        _log = []

        def init(size, args):
            return []

        def update(event):
            name = _NAMES.get(type(event))
            if name is None:
                return []
            _log.append(name)
            return [SetStatus("log=" + ",".join(_log))]

        def view():
            return Text("probe")
    """))
    return app


def test_wire_events_dispatch_typed_events_in_order(tmp_path):
    proc = _spawn_v3_app(_write_probe_app(tmp_path))
    try:
        _init_app(proc)
        wire = ["pane_shown", "focus_gained", "focus_lost", "pane_hidden"]
        expected = ["shown", "gained", "lost", "hidden"]
        for index, wire_type in enumerate(wire):
            _send_event(proc, {"type": wire_type})
            events = _collect_until(proc, "schedule_render")
            statuses = _find_events(events, "status_summary")
            assert len(statuses) == 1, f"{wire_type} must reach update() once: {events}"
            assert statuses[0]["text"] == "log=" + ",".join(expected[: index + 1])
    finally:
        proc.kill()


def test_app_harness_injects_focus_and_visibility(tmp_path):
    with AppHarness(_write_probe_app(tmp_path), timeout=3.0) as h:
        h.set_visible(True)
        h.set_focus(True)
        h.set_focus(False)
        h.set_visible(False)
        h.run(1)
        statuses = [
            ev["text"] for ev in h._events_seen if ev.get("type") == "status_summary"
        ]
        assert statuses[-1] == "log=shown,gained,lost,hidden", statuses
