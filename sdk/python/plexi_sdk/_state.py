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

    def __get__(self, obj: Any, _objtype: Any = None) -> Any:
        if obj is None:
            return self
        return getattr(obj, self._name, self._default)

    def __set__(self, obj: Any, value: Any) -> None:
        setattr(obj, self._name, value)
        emit = getattr(obj, "emit", None)
        if emit is not None and hasattr(emit, "schedule_render"):
            emit.schedule_render()
