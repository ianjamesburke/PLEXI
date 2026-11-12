from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Mapping, Optional, Protocol

from .effects import PersistState, SetState


# Kept inside the existing app-state payload so the runtime, restore path, and
# PersistState transport remain deliberately unaware of component state.
_COMPONENT_STATE_NAMESPACE = "__plexi_component_state__"


class ComponentSetter(Protocol):
    """A deferred component-state update returned by ``use_state``."""

    def __call__(self, value: Any, *, persist: bool = False) -> SetState | PersistState: ...


@dataclass(frozen=True)
class StateSnapshot:
    values: Mapping[str, Any]
    raw_values: Mapping[str, bytes]
    # Per-scope snapshots keyed by declared scope name; ``values`` aliases the
    # default scope's mapping.
    scoped: Mapping[str, Mapping[str, Any]] = field(default_factory=dict)
    # Declared scope names, ordered; the first entry is the default scope.
    scopes: tuple[str, ...] = ("global",)

    @classmethod
    def empty(cls) -> "StateSnapshot":
        return cls({}, {}, {"global": {}}, ("global",))


class StateProxy:
    def get(self, key: str, default: Any = None, scope: Optional[str] = None) -> Any:
        return _scope_values(scope).get(key, default)

    def raw(self, key: str) -> Optional[bytes]:
        snapshot = _require_state()
        return snapshot.raw_values.get(key)

    def all(self, scope: Optional[str] = None) -> dict[str, Any]:
        return dict(_scope_values(scope))

    def load(
        self,
        defaults: Mapping[str, Any],
        *,
        scope: Optional[str] = None,
    ) -> dict[str, Any]:
        """Return a fresh dict merging ``defaults`` with stored state in ``scope``."""
        stored = _scope_values(scope)
        return {key: stored.get(key, default) for key, default in defaults.items()}

    def set(self, key: str, value: Any, scope: Optional[str] = None) -> SetState:
        if _in_view:
            raise RuntimeError("state.set() called inside view() - return SetState from update() instead")
        if scope is not None:
            _require_declared(scope)
        return SetState({key: value}, scope=scope)

    def patch(
        self,
        updates: Optional[Mapping[str, Any]] = None,
        *,
        scope: Optional[str] = None,
        persist: bool = False,
        **kw: Any,
    ) -> SetState | PersistState:
        """Return a ``SetState`` (or ``PersistState`` when ``persist=True``) effect."""
        if _in_view:
            raise RuntimeError("state.patch() called inside view() - return SetState from update() instead")
        if scope is not None:
            _require_declared(scope)
        merged: dict[str, Any] = {}
        if updates is not None:
            merged.update(updates)
        if kw:
            merged.update(kw)
        if persist:
            return PersistState(merged, scope=scope)
        return SetState(merged, scope=scope)

    def scopes(self) -> tuple[str, ...]:
        """The scopes this app declared in its manifest, ordered; the first
        entry is the default scope."""
        return _require_state().scopes


class ComponentState:
    """State owned by one explicitly keyed component.

    A component key is stable app-defined identity, not a renderer identity.
    The state is stored beneath a reserved key in the normal state snapshot,
    which keeps ordinary restore and ``PersistState`` behaviour intact.
    """

    def __init__(self, key: str, *, scope: Optional[str] = None) -> None:
        if not isinstance(key, str) or not key:
            raise ValueError("component key must be a non-empty string")
        self.key = key
        self.scope = scope

    def get(self, name: str, default: Any = None) -> Any:
        return _component_values(self.key, self.scope).get(name, default)

    def set(self, name: str, value: Any, *, persist: bool = False) -> SetState | PersistState:
        """Return an effect updating this component's named local state."""
        if _in_view:
            raise RuntimeError("component state.set() called inside view() - return SetState from update() instead")
        _require_component_name(name)
        return state.patch(
            {_COMPONENT_STATE_NAMESPACE: {self.key: {name: value}}},
            scope=self.scope,
            persist=persist,
        )

    def use_state(self, name: str, default: Any = None) -> tuple[Any, ComponentSetter]:
        """Return the current value and a setter suitable for ``update()``."""
        _require_component_name(name)

        def setter(value: Any, *, persist: bool = False) -> SetState | PersistState:
            return self.set(name, value, persist=persist)

        return self.get(name, default), setter


def component(key: str, *, scope: Optional[str] = None) -> ComponentState:
    """Declare state for a component with a stable explicit ``key``."""
    return ComponentState(key, scope=scope)


def use_state(
    key: str,
    name: str,
    default: Any = None,
    *,
    scope: Optional[str] = None,
) -> tuple[Any, ComponentSetter]:
    """Read local state for component ``key`` and return its effect setter."""
    return component(key, scope=scope).use_state(name, default)


class LogProxy:
    def debug(self, msg: str) -> None:
        _host_log("debug", msg)

    def info(self, msg: str) -> None:
        _host_log("info", msg)

    def warn(self, msg: str) -> None:
        _host_log("warn", msg)

    def error(self, msg: str) -> None:
        _host_log("error", msg)


def _host_log(level: str, msg: str) -> None:
    # The host runtime replaces this hook with a host-log bridge at launch.
    print(f"[{level}] {msg}")


def _require_state() -> StateSnapshot:
    if _state is None:
        raise RuntimeError("plexi_sdk.state is only available inside init/update/view")
    return _state


def _require_declared(scope: str) -> None:
    snapshot = _require_state()
    if scope not in snapshot.scopes:
        raise ValueError(
            f"state scope '{scope}' is not declared in this app's manifest "
            f"[state] scopes {list(snapshot.scopes)}"
        )


def _scope_values(scope: Optional[str]) -> Mapping[str, Any]:
    snapshot = _require_state()
    if scope is None:
        return snapshot.values
    _require_declared(scope)
    return snapshot.scoped.get(scope, {})


def _require_component_name(name: str) -> None:
    if not isinstance(name, str) or not name:
        raise ValueError("component state name must be a non-empty string")


def _all_component_values(scope: Optional[str]) -> dict[str, dict[str, Any]]:
    raw = _scope_values(scope).get(_COMPONENT_STATE_NAMESPACE, {})
    if not isinstance(raw, Mapping):
        return {}
    return {
        str(key): dict(values)
        for key, values in raw.items()
        if isinstance(key, str) and isinstance(values, Mapping)
    }


def _component_values(key: str, scope: Optional[str]) -> Mapping[str, Any]:
    return _all_component_values(scope).get(key, {})


_state: Optional[StateSnapshot] = None
_in_view = False
state = StateProxy()
log = LogProxy()
