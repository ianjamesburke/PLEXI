from pathlib import Path
import sys

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from plexi_sdk import ComponentState, StateSnapshot, component, use_state
from plexi_sdk import _v3_state
from plexi_sdk.effects import PersistState, SetState
from plexi_sdk.ui import Keyed, Text


def _snapshot(values=None):
    values = values or {}
    return StateSnapshot(values, {}, {"global": values}, ("global",))


def test_component_state_isolated_by_explicit_key_and_returns_effects() -> None:
    _v3_state._state = _snapshot({"__plexi_component_state__": {"left": {"count": 1}}})
    _v3_state._in_view = False
    left = component("left")
    right = ComponentState("right")

    assert left.get("count", 0) == 1
    assert right.get("count", 0) == 0
    effect = left.set("count", 2)
    assert effect == SetState({"__plexi_component_state__": {"left": {"count": 2}}})
    # The next render receives the snapshot the runtime built from the effect.
    _v3_state._state = _snapshot(effect.data)
    assert left.get("count", 0) == 2
    assert right.set("count", 3) == SetState({
        "__plexi_component_state__": {"right": {"count": 3}}
    })


def test_use_state_reads_restored_value_and_persist_uses_existing_effect() -> None:
    stored = {"__plexi_component_state__": {"counter": {"count": 7}}}
    _v3_state._state = _snapshot(stored)
    _v3_state._in_view = True
    count, set_count = use_state("counter", "count", 0)
    assert count == 7
    _v3_state._in_view = False
    assert set_count(8, persist=True) == PersistState({
        "__plexi_component_state__": {"counter": {"count": 8}}
    })


def test_component_setter_is_forbidden_in_view_and_keyed_exposes_component_key() -> None:
    _v3_state._state = _snapshot()
    _v3_state._in_view = True
    with pytest.raises(RuntimeError, match="inside view"):
        component("counter").set("count", 1)
    _v3_state._in_view = False
    assert Keyed(Text("counter"), key="counter").key == "counter"
