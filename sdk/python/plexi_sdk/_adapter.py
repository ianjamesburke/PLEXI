from __future__ import annotations

import base64
import importlib
import json
from dataclasses import asdict, fields, is_dataclass
from typing import Any

import plexi_sdk as sdk

from ._v3_state import StateSnapshot
from . import _v3_state
from . import effects as effect_types
from . import events as event_types

_module = None


def load_app(module_name: str) -> None:
    global _module
    _module = importlib.import_module(module_name)


def call_lifecycle(fn_name: str, json_arg: str) -> str:
    if _module is None:
        raise RuntimeError("no Plexi app module loaded")
    fn = getattr(_module, fn_name)
    arg = json.loads(json_arg) if json_arg else {}

    if fn_name == "init":
        _set_sdk_state(arg.get("state", {}), in_view=False)
        result = fn(tuple(arg.get("size", [0.0, 0.0])), arg.get("args", []))
        return json.dumps([_encode_effect(e) for e in result])
    if fn_name == "update":
        _set_sdk_state(arg.get("state", {}), in_view=False)
        event = _decode_event(arg.get("event", {}))
        result = fn(event)
        return json.dumps([_encode_effect(e) for e in result])
    if fn_name == "view":
        _set_sdk_state(arg.get("state", {}), in_view=True)
        try:
            tree = fn()
            return json.dumps(_encode_uitree(tree))
        finally:
            sdk._in_view = False
            _v3_state._in_view = False
    raise ValueError(f"unknown lifecycle function: {fn_name}")


def _set_sdk_state(encoded: dict[str, str], in_view: bool) -> None:
    snapshot = _decode_state(encoded)
    sdk._state = snapshot
    sdk._in_view = in_view
    _v3_state._state = snapshot
    _v3_state._in_view = in_view


def _decode_state(encoded: dict[str, str]) -> StateSnapshot:
    values: dict[str, Any] = {}
    raw: dict[str, bytes] = {}
    for key, value in encoded.items():
        if value.startswith("b64:"):
            raw[key] = base64.b64decode(value[4:])
            continue
        payload = base64.b64decode(value)
        raw[key] = payload
        values[key] = json.loads(payload.decode("utf-8"))
    return StateSnapshot(values, raw)


def _encode_effect(effect: Any) -> dict[str, Any]:
    if not is_dataclass(effect):
        raise TypeError(f"Unknown effect type: {type(effect).__name__}")
    if getattr(effect_types, type(effect).__name__, None) is not type(effect):
        raise TypeError(f"Unknown effect type: {type(effect).__name__}")
    payload = asdict(effect)
    payload["type"] = type(effect).__name__
    return payload


def _decode_event(payload: dict[str, Any]) -> Any:
    event_type = payload.get("type")
    if not isinstance(event_type, str):
        raise TypeError("event payload missing string 'type'")
    cls = getattr(event_types, event_type, None)
    if cls is None or not is_dataclass(cls):
        raise TypeError(f"Unknown event type: {event_type}")
    kwargs = {f.name: payload[f.name] for f in fields(cls) if f.name in payload}
    if cls is event_types.KeyEvent and isinstance(kwargs.get("modifiers"), dict):
        kwargs["modifiers"] = event_types.Modifiers(**kwargs["modifiers"])
    if cls is event_types.SystemStatsResult and isinstance(kwargs.get("stats"), dict):
        kwargs["stats"] = event_types.SystemStats(**kwargs["stats"])
    if cls is event_types.PipeMessage and isinstance(kwargs.get("payload"), dict):
        kwargs["payload"] = event_types.PipePayload(**kwargs["payload"])
    return cls(**kwargs)


def _encode_uitree(root: Any) -> dict[str, Any]:
    arena: list[dict[str, Any]] = []

    def flatten(node: Any, key: str) -> int:
        node_id = len(arena)
        arena.append({"id": node_id, "key": key, "data": {"type": "Pending"}})
        explicit_key = getattr(node, "key", "") or key
        if isinstance(node, dict):
            data = node
        elif hasattr(node, "to_node"):
            data = node.to_node()
        elif is_dataclass(node):
            data = asdict(node)
            data["type"] = type(node).__name__
        else:
            raise TypeError(f"Unknown UINode type: {type(node).__name__}")
        data = _normalize_node_data(data, explicit_key, flatten)
        arena[node_id] = {"id": node_id, "key": explicit_key, "data": data}
        return node_id

    root_id = flatten(root, "0")
    return {"root": root_id, "nodes": arena}


def _normalize_node_data(data: dict[str, Any], key: str, flatten) -> dict[str, Any]:
    node_type = data.get("type") or data.get("kind")
    if node_type == "label":
        return {
            "type": "Text",
            "text": data.get("text", ""),
            "size": data.get("size"),
            "bold": data.get("bold", False),
            "truncate": data.get("truncate", False),
            "align": data.get("align", "start"),
        }
    if node_type == "column":
        children = data.get("children", [])
        child_ids = [flatten(child, f"{key}/{idx}") for idx, child in enumerate(children)]
        return {
            "type": "Column",
            "children": child_ids,
            "gap": data.get("gap", 0.0),
            "align": data.get("align", "start"),
            "grow": data.get("grow", False),
        }
    if node_type == "button":
        return {
            "type": "Button",
            "label": data.get("label", ""),
            "on_click": data.get("on_click", ""),
            "style": data.get("style", "secondary"),
            "disabled": data.get("disabled", False),
        }
    return data
