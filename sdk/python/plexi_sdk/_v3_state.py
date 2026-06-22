from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping, Optional

from .effects import SetState


@dataclass(frozen=True)
class StateSnapshot:
    values: Mapping[str, Any]
    raw_values: Mapping[str, bytes]

    @classmethod
    def empty(cls) -> "StateSnapshot":
        return cls({}, {})


class StateProxy:
    def get(self, key: str, default: Any = None) -> Any:
        snapshot = _require_state()
        return snapshot.values.get(key, default)

    def raw(self, key: str) -> Optional[bytes]:
        snapshot = _require_state()
        return snapshot.raw_values.get(key)

    def all(self) -> dict[str, Any]:
        snapshot = _require_state()
        return dict(snapshot.values)

    def set(self, key: str, value: Any) -> SetState:
        if _in_view:
            raise RuntimeError("state.set() called inside view() - return SetState from update() instead")
        return SetState({key: value})


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
    # CPython-in-WASM will replace this hook with a host-log import bridge.
    print(f"[{level}] {msg}")


def _require_state() -> StateSnapshot:
    if _state is None:
        raise RuntimeError("plexi_sdk.state is only available inside init/update/view")
    return _state


_state: Optional[StateSnapshot] = None
_in_view = False
state = StateProxy()
log = LogProxy()
