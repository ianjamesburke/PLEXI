from __future__ import annotations

from typing import Any


class State:
    """Reactive state descriptor. Assigning a new value calls ``self.emit.schedule_render()``.

    Usage on an App subclass::

        class Counter(App):
            count = State(0)

        def on_key(self, ctx, key, mods):
            if key == "j":
                self.count += 1  # triggers a re-render
    """

    def __init__(self, default: Any = None) -> None:
        self._default = default
        self._name: str = ""  # set by __set_name__

    def __set_name__(self, _owner: type, name: str) -> None:
        self._name = f"_state_{name}"
        # Validate that State is used on an App subclass (or at least has emit).
        # This catches the mistake of using State() on a plain class at definition time.
        from plexi_sdk._app import App
        if not issubclass(_owner, App):
            import warnings
            warnings.warn(
                f"State() descriptor '{name}' is declared on {_owner.__name__}, "
                f"which does not subclass App. State() triggers self.emit.schedule_render() "
                f"on assignment, which requires an App instance. "
                f"Fix: make {_owner.__name__} subclass App, or use a plain attribute instead.",
                stacklevel=2,
            )

    def __get__(self, obj: Any, _objtype: Any = None) -> Any:
        if obj is None:
            return self
        return getattr(obj, self._name, self._default)

    def __set__(self, obj: Any, value: Any) -> None:
        setattr(obj, self._name, value)
        emit = getattr(obj, "emit", None)
        if emit is not None and hasattr(emit, "schedule_render"):
            emit.schedule_render()
