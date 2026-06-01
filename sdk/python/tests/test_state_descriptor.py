"""Tests for the reactive State descriptor (epic #1897, task B1)."""

from unittest.mock import MagicMock, patch

import pytest

from plexi_sdk import App, State


# ── State descriptor protocol ──────────────────────────────────────────────────

def test_state_default_value() -> None:
    """State returns the declared default before any assignment."""
    class MyApp(App):
        count = State(0)
        label = State("hello")

    app = MyApp()
    assert app.count == 0
    assert app.label == "hello"


def test_state_default_none() -> None:
    """State() with no argument defaults to None."""
    class MyApp(App):
        value = State()

    app = MyApp()
    assert app.value is None


def test_state_set_stores_value() -> None:
    """Assigning a State field stores the new value."""
    class MyApp(App):
        count = State(0)

    app = MyApp()
    # Suppress the schedule_render side-effect for this test
    app.emit.schedule_render = MagicMock()

    app.count = 42
    assert app.count == 42


def test_state_set_calls_schedule_render() -> None:
    """Assigning a State field calls emit.schedule_render()."""
    class MyApp(App):
        count = State(0)

    app = MyApp()
    app.emit.schedule_render = MagicMock()

    app.count = 1
    app.emit.schedule_render.assert_called_once()


def test_state_multiple_assignments_call_schedule_render_each_time() -> None:
    """Each assignment triggers schedule_render, not just the first."""
    class MyApp(App):
        count = State(0)

    app = MyApp()
    app.emit.schedule_render = MagicMock()

    app.count = 1
    app.count = 2
    app.count = 3
    assert app.emit.schedule_render.call_count == 3


def test_state_instances_are_independent() -> None:
    """Two App instances do not share State storage."""
    class MyApp(App):
        count = State(0)

    a = MyApp()
    b = MyApp()
    a.emit.schedule_render = MagicMock()
    b.emit.schedule_render = MagicMock()

    a.count = 10
    assert a.count == 10
    assert b.count == 0  # b is unaffected


def test_state_class_access_returns_descriptor() -> None:
    """Accessing State on the class (not an instance) returns the descriptor itself."""
    class MyApp(App):
        count = State(0)

    assert isinstance(MyApp.count, State)


def test_state_augmented_assignment() -> None:
    """``self.count += 1`` (augmented assignment) triggers schedule_render."""
    class MyApp(App):
        count = State(0)

    app = MyApp()
    app.emit.schedule_render = MagicMock()

    app.count += 1
    assert app.count == 1
    app.emit.schedule_render.assert_called_once()


# ── view() method ──────────────────────────────────────────────────────────────

def test_view_returns_none_by_default() -> None:
    """App.view() returns None when not overridden."""
    app = App()
    assert app.view() is None  # type: ignore[misc]


def test_view_can_be_overridden() -> None:
    """Subclasses may override view() to return a component tree."""
    sentinel = object()

    class MyApp(App):
        def view(self):  # type: ignore[override]
            return sentinel

    app = MyApp()
    assert app.view() is sentinel


# ── Public export ──────────────────────────────────────────────────────────────

def test_state_exported_from_package() -> None:
    """State is importable directly from plexi_sdk."""
    import plexi_sdk
    assert hasattr(plexi_sdk, "State")
    assert plexi_sdk.State is State
