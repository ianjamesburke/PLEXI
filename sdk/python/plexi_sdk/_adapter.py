from __future__ import annotations

import base64
import importlib
import json
from dataclasses import asdict, fields, is_dataclass
from typing import Any, cast

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
            setattr(sdk, "_in_view", False)
            _v3_state._in_view = False
    raise ValueError(f"unknown lifecycle function: {fn_name}")


def _set_sdk_state(encoded: dict[str, str], in_view: bool) -> None:
    snapshot = _decode_state(encoded)
    setattr(sdk, "_state", snapshot)
    setattr(sdk, "_in_view", in_view)
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
    return StateSnapshot(values, raw, {"global": values}, ("global",))


def _encode_effect(effect: Any) -> dict[str, Any]:
    if not is_dataclass(effect):
        raise TypeError(f"Unknown effect type: {type(effect).__name__}")
    if getattr(effect_types, type(effect).__name__, None) is not type(effect):
        raise TypeError(f"Unknown effect type: {type(effect).__name__}")
    payload = _jsonable(asdict(cast(Any, effect)))
    if isinstance(effect, effect_types.HttpFetch):
        payload["url"] = effect.resolved_url()
        if payload.get("params") is None:
            payload.pop("params", None)
        if payload.get("tag") is None:
            payload.pop("tag", None)
        if payload.get("request_id") is None:
            payload.pop("request_id", None)
    payload["type"] = type(effect).__name__
    return payload


def _jsonable(value: Any) -> Any:
    if isinstance(value, bytes):
        return list(value)
    if isinstance(value, dict):
        return {key: _jsonable(item) for key, item in value.items()}
    if isinstance(value, list):
        return [_jsonable(item) for item in value]
    if isinstance(value, tuple):
        return [_jsonable(item) for item in value]
    return value


def _decode_event(payload: dict[str, Any]) -> Any:
    event_type = payload.get("type")
    if not isinstance(event_type, str):
        raise TypeError("event payload missing string 'type'")
    cls = getattr(event_types, event_type, None)
    if cls is None or not is_dataclass(cls):
        raise TypeError(f"Unknown event type: {event_type}")
    kwargs = {
        f.name: payload[f.name]
        for f in fields(cast(Any, cls))
        if f.name in payload
    }
    if cls is event_types.KeyEvent and isinstance(kwargs.get("modifiers"), dict):
        kwargs["modifiers"] = event_types.Modifiers(**kwargs["modifiers"])
    if cls is event_types.SystemStatsResult and isinstance(kwargs.get("stats"), dict):
        kwargs["stats"] = event_types.SystemStats(**kwargs["stats"])
    if cls is event_types.HttpResponse:
        if isinstance(kwargs.get("body"), list):
            kwargs["body"] = bytes(kwargs["body"])
        elif isinstance(kwargs.get("body"), str):
            kwargs["body"] = kwargs["body"].encode("utf-8")
        raw_headers = kwargs.get("headers")
        if isinstance(raw_headers, dict):
            kwargs["headers"] = [(str(k), str(v)) for k, v in raw_headers.items()]
        elif isinstance(raw_headers, list):
            kwargs["headers"] = [
                (str(item[0]), str(item[1]))
                for item in raw_headers
                if isinstance(item, (list, tuple)) and len(item) == 2
            ]
    if cls is event_types.PipeMessage and isinstance(kwargs.get("payload"), dict):
        kwargs["payload"] = event_types.PipePayload(**kwargs["payload"])
    return cast(Any, cls)(**kwargs)


def _require_view_component(root: Any) -> None:
    """Teachable failure when view() breaks the effect contract.

    The most common generated-app mistake is `return tree, SetTimer(...)` from
    view(); without this check it dies later as an opaque
    "Unknown UINode type: tuple".
    """
    if isinstance(root, (tuple, list)):
        raise TypeError(
            f"view() must return a single component tree, got {type(root).__name__}. "
            "Effects (SetTimer, SetState, ...) are returned from update() or init(), "
            "never from view()."
        )
    if is_dataclass(root) and getattr(effect_types, type(root).__name__, None) is type(root):
        raise TypeError(
            f"view() returned the effect {type(root).__name__}. Effects are returned "
            "from update() or init(); view() returns exactly one component."
        )


def _encode_uitree(root: Any) -> dict[str, Any]:
    _require_view_component(root)
    arena: list[dict[str, Any]] = []

    def flatten(node: Any, key: str) -> int:
        node_id = len(arena)
        arena.append({"id": node_id, "key": key, "data": {"type": "Pending"}})
        # An explicit Keyed key is a path segment, not a replacement for the
        # path accumulated above it.  That makes the key unique in the tree
        # and keeps every descendant on that keyed identity when siblings move.
        own_key = getattr(node, "key", "") or (
            node.get("_plexi_key", "") if isinstance(node, dict) else ""
        )
        # `flatten` receives a positional child address. Replace that final
        # address segment for Keyed rather than appending to it: the keyed node
        # is the direct child of its container, regardless of its list index.
        key_parent = key.rsplit("/", 1)[0] if "/" in key else ""
        explicit_key = f"{key_parent}/@{own_key}" if own_key else key
        if isinstance(node, dict):
            data = {k: v for k, v in node.items() if k != "_plexi_key"}
        elif hasattr(node, "to_node"):
            data = node.to_node()
        elif is_dataclass(node):
            data = asdict(cast(Any, node))
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
    if node_type == "pinned":
        child = data.get("child")
        if not isinstance(child, dict):
            raise TypeError("pinned node requires a child object")
        return {
            "type": "Pinned",
            "edge": data.get("edge", "bottom"),
            "child": flatten(child, f"{key}/child"),
        }
    if node_type == "footer_keys":
        return {
            "type": "FooterKeys",
            "entries": data.get("entries", []),
            "divider": data.get("divider", True),
        }
    if node_type == "app_bar":
        return {
            "type": "AppBar",
            "title": data.get("title", ""),
            "subtitle": data.get("subtitle", ""),
        }
    if node_type == "spinner":
        return {
            "type": "Spinner",
            "label": data.get("label", ""),
        }
    if node_type == "column":
        children = data.get("children", [])
        child_ids = [
            flatten(child, f"{key}/{idx}") for idx, child in enumerate(children)
        ]
        out: dict[str, Any] = {
            "type": "Column",
            "children": child_ids,
            "align": data.get("align", "start"),
            "grow": data.get("grow", False),
        }
        # Carry gap only when the app set one: absence lets the host apply its
        # default inter-child spacing, an explicit value (including 0) wins.
        if data.get("gap") is not None:
            out["gap"] = data["gap"]
        return out
    if node_type in ("card", "Card"):
        if "child" in data and isinstance(data["child"], int):
            child_id = data["child"]
        elif "child" in data and data["child"] is not None:
            child_id = flatten(data["child"], f"{key}/child")
        else:
            inner_col: dict[str, Any] = {
                "type": "column",
                "children": data.get("children", []),
            }
            if data.get("gap") is not None:
                inner_col["gap"] = data["gap"]
            child_id = flatten(inner_col, f"{key}/body")
        pad = float(data.get("padding", data.get("top", 12.0)))
        return {
            "type": "Card",
            "card": True,
            "child": child_id,
            "top": pad,
            "right": pad,
            "bottom": pad,
            "left": pad,
        }
    if node_type in ("DataTable", "data_table", "table", "Table"):
        children = data.get("children", [])
        child_ids = [
            flatten(child, f"{key}/{idx}") for idx, child in enumerate(children)
        ]
        selected = data.get("selected", data.get("selected_idx"))
        if selected is not None:
            selected = int(selected)
        out = {
            "type": "DataTable",
            "columns": [str(c) for c in data.get("columns", [])],
            "rows": [[str(cell) for cell in r] for r in data.get("rows", [])],
            "selected": selected,
            "on_select": data.get("on_select"),
            "children": child_ids,
            "align": data.get("align", "start"),
            "grow": data.get("grow", False),
        }
        if data.get("gap") is not None:
            out["gap"] = data["gap"]
        return out
    if node_type in ("row", "Row"):
        children = data.get("children", [])
        child_ids = [
            flatten(child, f"{key}/{idx}") for idx, child in enumerate(children)
        ]
        out = {
            "type": "Row",
            "children": child_ids,
            "align": data.get("align", "start"),
            "grow": data.get("grow", False),
        }
        if data.get("gap") is not None:
            out["gap"] = data["gap"]
        return out
    if node_type == "button":
        return {
            "type": "Button",
            "label": data.get("label", ""),
            "on_click": data.get("on_click", ""),
            "style": data.get("style", "secondary"),
            "disabled": data.get("disabled", False),
        }
    if node_type in ("TextInput", "text_input", "text-input"):
        return {
            "type": "TextInput",
            "value": data.get("value", ""),
            "placeholder": data.get("placeholder", ""),
            "on_change": data.get("on_change", data.get("on-change", "")),
            "on_submit": data.get("on_submit", data.get("on-submit", "")),
            "password": data.get("password", False),
            "autofocus": data.get("autofocus", False),
        }
    if node_type == "select_list":
        items = data.get("items", [])
        child_ids = []
        for idx, item in enumerate(items):
            if hasattr(item, "to_node"):
                item_node: Any = item
            elif isinstance(item, dict) and ("type" in item or "kind" in item):
                item_node = item
            else:
                text = (
                    str(item.get("name", item.get("label", "")))
                    if isinstance(item, dict)
                    else str(item)
                )
                description = ""
                leading = ""
                trailing = ""
                if isinstance(item, dict):
                    description = str(item.get("description", "") or "")
                    leading = str(item.get("leading", "") or "")
                    trailing = str(item.get("trailing", "") or "")
                if description:
                    main: Any = {
                        "type": "column",
                        "children": [
                            {"type": "text", "text": text, "bold": True},
                            {
                                "type": "text",
                                "text": description,
                                "size": 12.0,
                                "color": "muted",
                            },
                        ],
                        "gap": 2.0,
                    }
                else:
                    main = {"type": "text", "text": text}
                if leading or trailing:
                    row_children: list[dict[str, Any]] = []
                    if leading:
                        row_children.append({"type": "text", "text": leading})
                    row_children.append(main)
                    if trailing:
                        row_children.append(
                            {
                                "type": "text",
                                "text": trailing,
                                "size": 12.0,
                                "color": "muted",
                            }
                        )
                    item_node = {"type": "row", "children": row_children, "gap": 8.0}
                else:
                    item_node = main
            child_ids.append(flatten(item_node, f"{key}/{idx}"))
        selected = data.get("selected_idx", data.get("selected"))
        if selected is not None:
            selected = int(selected)
        return {
            "type": "ListView",
            "items": child_ids,
            "selected": selected,
            "on_select": data.get("on_select"),
        }
    if node_type in ("scroll", "Scroll"):
        child = data.get("child")
        if child is None:
            raise TypeError("scroll node requires a child")
        return {
            "type": "Scroll",
            "child": flatten(child, f"{key}/0"),
            "horizontal": data.get("horizontal", False),
        }
    return data
