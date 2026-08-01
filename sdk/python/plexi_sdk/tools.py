"""Assistant-callable tools, declared next to the function that answers them.

Exposing a tool by hand means writing a JSON Schema literal, an ``AiTool``
declaration, and a ``ToolCall`` dispatch arm — three places to keep in sync per
tool, for every app. This module collapses that to a decorator::

    from plexi_sdk import tools

    @tools.tool("todo.add", "Add a todo item.", {"text": str})
    def _add(text: str) -> tools.Reply:
        items = state.get("items", []) + [{"text": text, "done": False}]
        return tools.Reply({"count": len(items)}, [PersistState({"items": items})])

    def init(size, args):
        return [tools.expose()]

    def update(event):
        return tools.dispatch(event) or []

``expose()`` returns the ``ExposeTools`` effect for every registered tool, and
``dispatch(event)`` returns the ``ToolResult`` effects for a ``ToolCall`` (plus
whatever effects the tool returned) or ``None`` when the event is not a tool
call — so an app's ``update()`` never grows a per-tool arm.

A tool that only reads app state passes ``read_only=True``; the Assistant runs
those without a write-grant prompt. Raising inside a tool is reported to the
Assistant as that call's ``error`` — never as a crashed app.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Callable, Optional, Sequence

from .effects import AiTool, ExposeTools, ToolResult
from .events import ToolCall

# `params` values are Python types; these are their JSON Schema spellings. A
# type outside this map is a declaration error, raised at decoration time
# rather than when the Assistant first calls the tool.
_JSON_TYPES: dict[type, str] = {
    str: "string",
    int: "integer",
    float: "number",
    bool: "boolean",
}


@dataclass
class Reply:
    """A tool's answer: the JSON output, plus any effects it wants applied.

    ``output`` must match the tool's declared ``returns`` schema. ``effects``
    is the ordinary effect list — a mutating tool returns its ``PersistState``
    / ``SetStatus`` here instead of touching state directly.
    """

    output: dict[str, Any]
    effects: Sequence[Any] = field(default_factory=tuple)


@dataclass(frozen=True)
class _Registered:
    decl: AiTool
    fn: Callable[..., Any]


_REGISTRY: "dict[str, _Registered]" = {}


def _schema(spec: "dict[str, type] | None", *, who: str) -> dict[str, Any]:
    properties: dict[str, Any] = {}
    for name, py_type in (spec or {}).items():
        json_type = _JSON_TYPES.get(py_type)
        if json_type is None:
            raise TypeError(
                f"{who}: parameter '{name}' has unsupported type "
                f"{getattr(py_type, '__name__', py_type)!r}; "
                f"use one of {', '.join(t.__name__ for t in _JSON_TYPES)}"
            )
        properties[name] = {"type": json_type}
    return {
        "type": "object",
        "properties": properties,
        "required": list(properties),
    }


def tool(
    name: str,
    description: str,
    params: "dict[str, type] | None" = None,
    returns: "dict[str, type] | None" = None,
    *,
    read_only: bool = False,
) -> Callable[[Callable[..., Any]], Callable[..., Any]]:
    """Register the decorated function as the handler for tool ``name``.

    ``params`` and ``returns`` map argument names to Python types (``str``,
    ``int``, ``float``, ``bool``); both become JSON Schema objects with every
    listed key required. The function is called with the Assistant's arguments
    as keyword arguments and returns a :class:`Reply` (or a bare dict when it
    has no effects).
    """
    if not name:
        raise ValueError("tool name must be non-empty")
    if name in _REGISTRY:
        raise ValueError(f"tool '{name}' is already registered")

    decl = AiTool(
        name=name,
        description=description,
        input_schema=_schema(params, who=f"tool '{name}'"),
        output_schema=_schema(returns, who=f"tool '{name}' returns"),
        read_only=read_only,
    )

    def register(fn: Callable[..., Any]) -> Callable[..., Any]:
        _REGISTRY[name] = _Registered(decl, fn)
        return fn

    return register


def declarations() -> list[AiTool]:
    """Every registered tool declaration, in registration order."""
    return [entry.decl for entry in _REGISTRY.values()]


def expose() -> ExposeTools:
    """The `ExposeTools` effect declaring every registered tool. Return it from
    ``init()``; a declaration replaces the pane's previous tool set."""
    return ExposeTools(declarations())


def dispatch(event: Any) -> Optional[list]:
    """Run the tool named by ``event`` and return its effects.

    Returns ``None`` when ``event`` is not a :class:`~plexi_sdk.events.ToolCall`,
    so an app can write ``return tools.dispatch(event) or <its own handling>``.
    The returned list always starts with the call's ``ToolResult``.
    """
    if not isinstance(event, ToolCall):
        return None

    entry = _REGISTRY.get(event.name)
    if entry is None:
        return [ToolResult(event.call_id, error=f"unknown tool '{event.name}'")]

    try:
        arguments = json.loads(event.input_json or "{}")
        if not isinstance(arguments, dict):
            raise TypeError(
                f"tool input must be a JSON object, got {type(arguments).__name__}"
            )
        reply = entry.fn(**arguments)
    except Exception as exc:  # surfaced to the Assistant, never crashes the app
        return [ToolResult(event.call_id, error=f"{type(exc).__name__}: {exc}")]

    if isinstance(reply, Reply):
        output, effects = reply.output, list(reply.effects)
    else:
        output, effects = reply, []
    return [ToolResult(event.call_id, output_json=json.dumps(output))] + effects


def _reset_for_tests() -> None:
    """Clear the registry. Only for tests that import an app module twice."""
    _REGISTRY.clear()
