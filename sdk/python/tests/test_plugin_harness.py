import json
import textwrap

import pytest

from plexi_sdk.testing import PluginHarness


@pytest.fixture
def probe_plugin(tmp_path):
    path = tmp_path / "probe.py"
    path.write_text(textwrap.dedent("""
        import json
        from dataclasses import asdict

        from plexi_sdk.effects import SetStatus
        from plexi_sdk.events import UiAction, UiValueChange
        from plexi_sdk.ui import TextInput

        def init(size, args):
            return [SetStatus(json.dumps({"size": size}))]

        def update(event):
            if isinstance(event, (UiAction, UiValueChange)):
                return [SetStatus(json.dumps({"type": type(event).__name__, **asdict(event)}))]
            return []

        def view():
            return TextInput(id="query", on_change="edit", on_submit="submit")
    """))
    return path


def _status_events(harness):
    harness.run()
    return [
        json.loads(event["text"])
        for event in harness._events_seen
        if event.get("type") == "status_summary"
    ]


@pytest.mark.parametrize(
    "dimensions, expected",
    [({}, [800, 600]), ({"width": 420, "height": 260}, [420, 260])],
)
def test_init_receives_harness_dimensions(probe_plugin, dimensions, expected):
    with PluginHarness(probe_plugin, **dimensions) as h:
        assert _status_events(h) == [{"size": expected}]


def test_text_submit_dispatches_action_without_a_value(probe_plugin):
    with PluginHarness(probe_plugin) as h:
        h.text_submit("submit")
        assert _status_events(h)[1:] == [{"type": "UiAction", "handler_id": "submit"}]


def test_text_change_dispatches_value_to_change_handler(probe_plugin):
    with PluginHarness(probe_plugin) as h:
        h.text_change("edit", "draft")
        assert _status_events(h)[1:] == [
            {"type": "UiValueChange", "handler_id": "edit", "value": "draft"}
        ]
