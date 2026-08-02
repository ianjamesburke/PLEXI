"""`state_changed` host event handling in the v3 runtime (stint 0644).

The host sends `state_changed` when a scope's backing file changed outside
the app's own persist flow (external edit, or a persist the app lost to a
concurrent external write). The runtime must replace the scope's values
wholesale — deleted keys vanish, never a merge — then dispatch
`events.StateChanged` through the normal update path so `update()` runs
against the fresh snapshot. A decode error is surfaced on the event while
the previous values are kept.
"""

import textwrap
from pathlib import Path

from test_state_scopes import _init_scoped
from test_v3_runtime_regression import (
    _collect_until,
    _find_events,
    _send_event,
    _spawn_v3_app,
)


def _write_probe_app(tmp_path: Path) -> Path:
    app = tmp_path / "app.py"
    app.write_text(textwrap.dedent("""
        import json

        from plexi_sdk import state
        from plexi_sdk.effects import SetStatus
        from plexi_sdk.events import StateChanged
        from plexi_sdk.ui import Text

        def init(size, args):
            return []

        def update(event):
            if isinstance(event, StateChanged):
                return [SetStatus(
                    "changed scope=%s source=%s error=%r a=%r b=%r values=%s" % (
                        event.scope,
                        event.source,
                        event.error,
                        state.get("a"),
                        state.get("b"),
                        json.dumps(event.values, sort_keys=True),
                    )
                )]
            return []

        def view():
            return Text("probe")
    """))
    return app


def _send_state_changed(proc, payload, error=None, scope="global"):
    _send_event(proc, {
        "type": "state_changed",
        "scope": scope,
        "payload": payload,
        "error": error,
        "source": "external",
    })
    # `_dispatch` emits effects first, then schedule_render — collecting to
    # the render marker captures both.
    return _collect_until(proc, "schedule_render")


def test_state_changed_replaces_scope_and_dispatches_update(tmp_path):
    proc = _spawn_v3_app(_write_probe_app(tmp_path))
    try:
        _init_scoped(proc, scopes=["global"], states={"global": {"a": 1, "b": 2}})

        # External write dropped key "b" — it must vanish from state reads.
        events = _send_state_changed(proc, {"a": 9})
        statuses = _find_events(events, "status_summary")
        assert len(statuses) == 1, f"update() must run exactly once, got {events}"
        text = statuses[0]["text"]
        assert "scope=global" in text
        assert "source=external" in text
        assert "error=None" in text
        assert "a=9" in text, f"fresh value must be visible inside update(): {text}"
        assert "b=None" in text, f"deleted key must vanish (replace, not merge): {text}"
        assert '"b"' not in text.split("values=")[1], "event.values must not carry b"

        # A render is scheduled through the normal update path.
        assert _find_events(events, "schedule_render"), events
    finally:
        proc.kill()


def test_state_changed_error_is_surfaced_and_values_kept(tmp_path):
    proc = _spawn_v3_app(_write_probe_app(tmp_path))
    try:
        _init_scoped(proc, scopes=["global"], states={"global": {"a": 1}})

        events = _send_state_changed(proc, {"a": 1}, error="parse state JSON: boom")
        statuses = _find_events(events, "status_summary")
        assert len(statuses) == 1
        text = statuses[0]["text"]
        assert "error='parse state JSON: boom'" in text
        assert "a=1" in text, f"previous values must be kept on error: {text}"
    finally:
        proc.kill()
