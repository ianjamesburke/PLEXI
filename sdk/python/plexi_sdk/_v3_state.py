from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Mapping, Optional

from .effects import SetState


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

    def set(self, key: str, value: Any, scope: Optional[str] = None) -> SetState:
        if _in_view:
            raise RuntimeError("state.set() called inside view() - return SetState from update() instead")
        if scope is not None:
            _require_declared(scope)
        return SetState({key: value}, scope=scope)

    def scopes(self) -> tuple[str, ...]:
        """The scopes this app declared in its manifest, ordered; the first
        entry is the default scope."""
        return _require_state().scopes


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


_state: Optional[StateSnapshot] = None
_in_view = False
state = StateProxy()
log = LogProxy()
