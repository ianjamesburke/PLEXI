"""Tests for App.state proxy — host-persisted state API (replaces ctx.load_state/save_state)."""

from unittest.mock import patch

from plexi_sdk import App


def _make_app_with_state(state_dict):
    app = App()
    app._app_state = dict(state_dict)
    return app


def test_state_get_returns_default_when_missing():
    app = _make_app_with_state({})
    assert app.state.get("x", 99) == 99


def test_state_get_returns_value_when_present():
    app = _make_app_with_state({"x": 42})
    assert app.state.get("x") == 42


def test_state_all_returns_copy():
    app = _make_app_with_state({"a": 1, "b": 2})
    d = app.state.all()
    assert d == {"a": 1, "b": 2}
    d["extra"] = 99
    assert "extra" not in app._app_state


def test_state_save_updates_internal_dict():
    app = _make_app_with_state({})
    with patch("plexi_sdk._app._emit"):
        app.state.save({"count": 5})
    assert app._app_state == {"count": 5}


def test_state_save_emits_save_app_state():
    app = _make_app_with_state({})
    emitted = []
    with patch("plexi_sdk._app._emit", side_effect=lambda d: emitted.append(d)):
        app.state.save({"count": 5})
    assert emitted == [{"type": "save_app_state", "payload": {"count": 5}}]


def test_state_available_without_ctx():
    """App.state works before the event loop starts — no async needed."""
    app = _make_app_with_state({"greeting": "hello"})
    assert app.state.get("greeting") == "hello"
