"""Tests that App subclasses work when __init__ omits super().__init__()."""

from plexi_sdk import App, RenderContext


def test_subclass_without_super_init_has_sdk_state() -> None:
    """A subclass that defines __init__ without super() must still have all SDK attributes."""
    class MyApp(App):
        def __init__(self) -> None:
            self.custom_value = 42  # no super().__init__()

    app = MyApp()
    # Core SDK attributes must exist without AttributeError
    assert isinstance(app._rect, dict)
    assert isinstance(app._pipes, dict)
    assert isinstance(app._pending_capability, dict)
    assert app._loop is None
    assert isinstance(app._background_tasks, set)
    assert app.custom_value == 42


def test_subclass_with_super_init_still_works() -> None:
    """A subclass that calls super().__init__() must still work (no double-init issues)."""
    class MyApp(App):
        def __init__(self) -> None:
            super().__init__()
            self.custom_value = 99

    app = MyApp()
    assert isinstance(app._rect, dict)
    assert app.custom_value == 99


def test_no_super_subclass_runs_without_error() -> None:
    """Instantiating App subclass without super() must not raise any error."""
    class MyApp(App):
        def __init__(self) -> None:
            self.name = "hello"
        def on_render(self, _ctx: RenderContext) -> None:
            pass

    # Should not raise AttributeError or any other error
    app = MyApp()
    assert app.name == "hello"
    assert hasattr(app, "emit")
