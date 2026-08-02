"""MCP Bridge — re-exposes a configured MCP server's tools on Plexi's connector plane.

Plexi does not embed an MCP client. MCP servers are consumed through the
existing app-connector plane: this app hosts the MCP client and re-exposes the
server's tools via ``ExposeTools``, so every MCP tool the assistant can reach is
an ordinary connector tool — capability-scoped, permission-gated, and visible in
the event log. There is no second tool path.

Division of labour with the host:

- **The host owns the server process.** ``McpConnect`` names a server by id; the
  host resolves that id against the user's ``mcp_servers.toml``, spawns it, and
  pipes newline-delimited JSON. This app never supplies argv and cannot reach a
  binary the user did not configure.
- **This app owns the MCP protocol.** ``initialize`` → ``notifications/
  initialized`` → ``tools/list`` → schema translation → ``ExposeTools``, then
  ``ToolCall`` → ``tools/call`` → ``ToolResult``.

Usage: ``plexi app open <path-to-mcp-bridge> -- <server-id> [<server-id> ...]``
where each id names a ``[servers.<id>]`` entry in ``mcp_servers.toml``.
"""

from __future__ import annotations

import json
from typing import Any

from plexi_sdk import log, state
from plexi_sdk.effects import (
    AiTool,
    CancelTimer,
    ExposeTools,
    McpConnect,
    McpDisconnect,
    McpSend,
    SetState,
    SetStatus,
    SetTimer,
    SetTitle,
    ToolResult,
)
from plexi_sdk.events import KeyEvent, McpClosed, McpConnected, McpMessage, TimerFired, ToolCall
from plexi_sdk.ui import AppBar, Column, FooterKeys, Section, Text

# MCP revision this bridge speaks. Bumping it is a deliberate act: a server may
# negotiate down, but the client must not claim a revision it has not been
# exercised against.
PROTOCOL_VERSION = "2024-11-05"

CLIENT_INFO = {"name": "plexi-mcp-bridge", "version": "0.1.0"}

# Tool timeout handed to the host broker for every proxied MCP tool. An MCP
# call crosses two process boundaries, so it gets more room than a local
# connector tool, but never unbounded.
TOOL_TIMEOUT_MS = 60_000

# JSON-RPC ids this app allocates. Namespaced per server so two servers'
# replies can never be confused for one another.
_ID_INITIALIZE = 1
_ID_TOOLS_LIST = 2
_FIRST_CALL_ID = 100

# Handshake deadline: every server must finish connect → initialize →
# tools/list within this window or it is disconnected with a visible error.
# The app owns the protocol, so the app owns this deadline — the host only
# enforces a coarser "produced any output at all" backstop. Shorter than the
# host's backstop so the interactive stuck-pane case resolves here first.
HANDSHAKE_TIMEOUT_MS = 90_000
_HANDSHAKE_TIMER_ID = 1

# Server statuses that mean the handshake has not completed yet.
_HANDSHAKE_STATUSES = ("connecting", "initializing", "listing")


def init(size, args) -> list:
    server_ids = [arg for arg in (args or []) if arg.strip()]
    if not server_ids:
        # No silent default. A bridge with no server is a misconfiguration the
        # user must see, not an app that sits there exposing nothing.
        log.error("mcp-bridge: no server ids given")
        return [
            SetTitle("MCP Bridge"),
            SetStatus("no servers"),
            SetState({
                "servers": {},
                "fatal": (
                    "No MCP server given. Open with the server ids to bridge, "
                    "each naming a [servers.<id>] entry in mcp_servers.toml — "
                    "for example: plexi app open <app> -- filesystem"
                ),
            }),
        ]

    log.info(f"mcp-bridge: connecting to {len(server_ids)} server(s): {', '.join(server_ids)}")
    servers: dict[str, dict[str, Any]] = {
        server_id: {
            "status": "connecting",
            "tool_records": [],
            "skipped": [],
            "error": None,
            "next_call_id": _FIRST_CALL_ID,
            "pending": {},
        }
        for server_id in server_ids
    }
    effects: list = [
        SetTitle("MCP Bridge"),
        SetStatus(f"connecting to {len(server_ids)} server(s)"),
        SetState({"servers": servers, "fatal": None}),
        SetTimer(id=_HANDSHAKE_TIMER_ID, delay_ms=HANDSHAKE_TIMEOUT_MS),
    ]
    effects.extend(
        McpConnect(request_id=f"connect:{server_id}", server_id=server_id)
        for server_id in server_ids
    )
    return effects


def update(event) -> list:
    if isinstance(event, McpConnected):
        return _on_connected(event)
    if isinstance(event, McpMessage):
        return _on_message(event)
    if isinstance(event, McpClosed):
        return _on_closed(event)
    if isinstance(event, ToolCall):
        return _on_tool_call(event)
    if isinstance(event, TimerFired) and event.id == _HANDSHAKE_TIMER_ID:
        return _on_handshake_deadline()
    if isinstance(event, KeyEvent) and event.pressed and event.key == "r":
        return _reconnect_all()
    return []


# ── MCP protocol ─────────────────────────────────────────────────────────────


def _on_connected(event: McpConnected) -> list:
    servers = _servers()
    server = servers.get(event.server_id)
    if server is None:
        log.warn(f"mcp-bridge: connected reply for unknown server {event.server_id}")
        return []

    if event.error:
        log.error(f"mcp-bridge: connect to {event.server_id} failed: {event.error}")
        server["status"] = "failed"
        server["error"] = event.error
        return [SetState({"servers": servers}), SetStatus(_summary(servers))]

    log.info(f"mcp-bridge: {event.server_id} connected; sending initialize")
    server["status"] = "initializing"
    return [
        SetState({"servers": servers}),
        McpSend(
            server_id=event.server_id,
            message={
                "jsonrpc": "2.0",
                "id": _ID_INITIALIZE,
                "method": "initialize",
                "params": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": CLIENT_INFO,
                },
            },
        ),
    ]


def _on_message(event: McpMessage) -> list:
    servers = _servers()
    server = servers.get(event.server_id)
    if server is None:
        log.warn(f"mcp-bridge: message from unknown server {event.server_id}")
        return []

    message = event.message
    message_id = message.get("id")
    if message_id is None:
        # A notification. The bridge does not act on any of them today; log so
        # an unexpected one is visible rather than invisible.
        log.debug(f"mcp-bridge: {event.server_id} notification {message.get('method')}")
        return []

    if "error" in message:
        return _on_rpc_error(servers, server, event.server_id, message_id, message["error"])

    result = message.get("result", {})
    if message_id == _ID_INITIALIZE:
        return _after_initialize(servers, server, event.server_id, result)
    if message_id == _ID_TOOLS_LIST:
        return _after_tools_list(servers, server, event.server_id, result)
    return _after_tool_call(servers, server, event.server_id, message_id, result)


def _after_initialize(servers: dict, server: dict, server_id: str, result: dict) -> list:
    info = result.get("serverInfo", {})
    negotiated = result.get("protocolVersion", "unknown")
    log.info(
        f"mcp-bridge: {server_id} initialized — {info.get('name', '?')} "
        f"{info.get('version', '?')} (protocol {negotiated})"
    )
    server["status"] = "listing"
    server["server_name"] = info.get("name", server_id)
    return [
        SetState({"servers": servers}),
        McpSend(
            server_id=server_id,
            message={"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}},
        ),
        McpSend(
            server_id=server_id,
            message={"jsonrpc": "2.0", "id": _ID_TOOLS_LIST, "method": "tools/list", "params": {}},
        ),
    ]


def _after_tools_list(servers: dict, server: dict, server_id: str, result: dict) -> list:
    raw_tools = result.get("tools") or []
    translated, skipped = translate_tools(server_id, raw_tools)

    for name, reason in skipped:
        # Loud by contract: a tool the bridge cannot represent must be visible,
        # never quietly missing from the assistant's tool list.
        log.warn(f"mcp-bridge: {server_id} skipped tool {name!r}: {reason}")

    server["status"] = "ready"
    # State crosses the host bridge as plain JSON, so tools are stored as
    # records and rebuilt into AiTool on every re-declaration.
    server["tool_records"] = [
        {
            "name": tool.name,
            "description": tool.description,
            "input_schema": tool.input_schema,
            "output_schema": tool.output_schema,
        }
        for tool in translated
    ]
    server["skipped"] = [{"name": name, "reason": reason} for name, reason in skipped]
    log.info(
        f"mcp-bridge: {server_id} exposed {len(translated)} tool(s), "
        f"skipped {len(skipped)}"
    )
    effects: list = [
        SetState({"servers": servers}),
        SetStatus(_summary(servers)),
        ExposeTools(_all_tools(servers)),
    ]
    if not any(s.get("status") in _HANDSHAKE_STATUSES for s in servers.values()):
        effects.append(CancelTimer(id=_HANDSHAKE_TIMER_ID))
    return effects


def _after_tool_call(
    servers: dict, server: dict, server_id: str, message_id: Any, result: dict
) -> list:
    call_id = server.get("pending", {}).pop(str(message_id), None)
    if call_id is None:
        log.warn(f"mcp-bridge: {server_id} replied to unknown request id {message_id}")
        return []
    log.info(f"mcp-bridge: {server_id} completed call {call_id}")
    return [
        SetState({"servers": servers}),
        ToolResult(call_id=call_id, output_json=json.dumps(result)),
    ]


def _on_rpc_error(
    servers: dict, server: dict, server_id: str, message_id: Any, error: dict
) -> list:
    detail = f"{error.get('code', '?')}: {error.get('message', 'unknown error')}"
    log.error(f"mcp-bridge: {server_id} JSON-RPC error on id {message_id}: {detail}")

    call_id = server.get("pending", {}).pop(str(message_id), None)
    if call_id is not None:
        return [
            SetState({"servers": servers}),
            ToolResult(call_id=call_id, error=f"MCP server '{server_id}' error — {detail}"),
        ]

    server["status"] = "failed"
    server["error"] = detail
    return [SetState({"servers": servers}), SetStatus(_summary(servers))]


def _on_closed(event: McpClosed) -> list:
    servers = _servers()
    server = servers.get(event.server_id)
    if server is None:
        return []
    log.warn(f"mcp-bridge: {event.server_id} closed: {event.reason}")
    if server.get("status") != "failed":
        # A failed server keeps its diagnosis: the close that follows our own
        # McpDisconnect ("disconnected by app") must not overwrite the reason
        # the user actually needs to see.
        server["status"] = "closed"
        server["error"] = event.reason
    server["tool_records"] = []
    server["skipped"] = []

    # Every request still in flight will never be answered. Fail them now
    # rather than leave the broker blocked until its timeout.
    effects: list = []
    for call_id in server.get("pending", {}).values():
        effects.append(
            ToolResult(
                call_id=call_id,
                error=f"MCP server '{event.server_id}' closed before replying: {event.reason}",
            )
        )
    server["pending"] = {}

    return [
        SetState({"servers": servers}),
        SetStatus(_summary(servers)),
        ExposeTools(_all_tools(servers)),
        *effects,
    ]


def _on_tool_call(event: ToolCall) -> list:
    servers = _servers()
    server_id = _server_for_tool(servers, event.name)
    if server_id is None:
        log.warn(f"mcp-bridge: tool call for unknown tool {event.name!r}")
        return [ToolResult(call_id=event.call_id, error=f"unknown tool '{event.name}'")]

    server = servers[server_id]
    if server.get("status") != "ready":
        return [
            ToolResult(
                call_id=event.call_id,
                error=f"MCP server '{server_id}' is {server.get('status')}, not ready",
            )
        ]

    try:
        arguments = json.loads(event.input_json) if event.input_json else {}
    except json.JSONDecodeError as e:
        log.error(f"mcp-bridge: tool call {event.name} has invalid input JSON: {e}")
        return [ToolResult(call_id=event.call_id, error=f"invalid tool input JSON: {e}")]

    request_id = server["next_call_id"]
    server["next_call_id"] = request_id + 1
    server.setdefault("pending", {})[str(request_id)] = event.call_id
    mcp_name = tool_name_to_mcp(server_id, event.name)
    log.info(
        f"mcp-bridge: {server_id} calling {mcp_name!r} for call {event.call_id} "
        f"(caller {event.caller_id})"
    )
    return [
        SetState({"servers": servers}),
        McpSend(
            server_id=server_id,
            message={
                "jsonrpc": "2.0",
                "id": request_id,
                "method": "tools/call",
                "params": {"name": mcp_name, "arguments": arguments},
            },
        ),
    ]


def _on_handshake_deadline() -> list:
    """Fail every server whose handshake never completed.

    A server that connects but never answers ``initialize`` or ``tools/list``
    would otherwise sit at "initializing" forever — alive, invisible to the
    assistant, and blocking nothing loudly. The deadline converts that silence
    into a visible error and a disconnect (which makes the host kill the
    process). Servers that already settled are untouched.
    """
    servers = _servers()
    effects: list = []
    for server_id, server in servers.items():
        if server.get("status") not in _HANDSHAKE_STATUSES:
            continue
        log.error(
            f"mcp-bridge: {server_id} handshake not completed within "
            f"{HANDSHAKE_TIMEOUT_MS // 1000}s (stuck at {server.get('status')})"
        )
        server["status"] = "failed"
        server["error"] = (
            f"handshake not completed within {HANDSHAKE_TIMEOUT_MS // 1000}s "
            f"(server unresponsive)"
        )
        effects.append(McpDisconnect(server_id=server_id))
    if not effects:
        return []
    return [SetState({"servers": servers}), SetStatus(_summary(servers)), *effects]


def _reconnect_all() -> list:
    servers = _servers()
    if not servers:
        return []
    log.info("mcp-bridge: reconnecting all servers")
    effects: list = []
    for server_id, server in servers.items():
        effects.append(McpDisconnect(server_id=server_id))
        effects.append(McpConnect(request_id=f"connect:{server_id}", server_id=server_id))
        server["status"] = "connecting"
        server["error"] = None
        server["tool_records"] = []
        server["skipped"] = []
        server["pending"] = {}
    return [
        SetState({"servers": servers}),
        SetStatus("reconnecting"),
        SetTimer(id=_HANDSHAKE_TIMER_ID, delay_ms=HANDSHAKE_TIMEOUT_MS),
        *effects,
    ]


# ── Schema translation ───────────────────────────────────────────────────────


def tool_name_for(server_id: str, mcp_name: str) -> str:
    """Connector tool name for one MCP tool.

    Every exposed name carries its server, so the permission prompt and the
    event log both name which server a tool came from — two servers exposing
    ``read_file`` are two distinct, separately-approvable tools.
    """
    return f"mcp.{server_id}.{mcp_name}"


def tool_name_to_mcp(server_id: str, tool_name: str) -> str:
    """Inverse of :func:`tool_name_for`."""
    prefix = f"mcp.{server_id}."
    return tool_name[len(prefix):] if tool_name.startswith(prefix) else tool_name


def translate_tools(
    server_id: str, raw_tools: list
) -> tuple[list[AiTool], list[tuple[str, str]]]:
    """Translate MCP tool declarations into connector tools.

    Returns ``(translated, skipped)`` where ``skipped`` is ``(name, reason)``.
    A tool this bridge cannot represent is skipped **with a reason**, never
    silently dropped and never exposed with a guessed schema — an assistant
    calling a tool whose schema was invented fails in a far worse way than one
    that can see the tool is unavailable.

    Deliberately absent: ``read_only``. MCP servers advertise
    ``annotations.readOnlyHint``, and honouring it would let an external,
    less-trusted tool source turn the host's default ``Ask`` into an automatic
    allow. Every MCP-derived tool is therefore ask-gated. Whether the host can
    ever establish mutability for a bridged tool is host policy, not this
    app's to assert.
    """
    translated: list[AiTool] = []
    skipped: list[tuple[str, str]] = []
    seen: set[str] = set()

    for raw in raw_tools:
        if not isinstance(raw, dict):
            skipped.append((str(raw)[:60], "tool declaration is not an object"))
            continue
        name = raw.get("name")
        if not isinstance(name, str) or not name:
            skipped.append((str(raw.get("name"))[:60], "tool has no string 'name'"))
            continue
        if name in seen:
            skipped.append((name, "duplicate tool name in this server's tools/list"))
            continue

        schema = raw.get("inputSchema")
        if not isinstance(schema, dict):
            skipped.append((name, "inputSchema is missing or not an object"))
            continue
        if schema.get("type") != "object":
            skipped.append((
                name,
                f"inputSchema.type is {schema.get('type')!r}; the connector tool "
                "schema requires a top-level object",
            ))
            continue
        properties = schema.get("properties", {})
        if not isinstance(properties, dict):
            skipped.append((name, "inputSchema.properties is not an object"))
            continue

        seen.add(name)
        translated.append(
            AiTool(
                name=tool_name_for(server_id, name),
                description=_describe(server_id, raw),
                input_schema=_clean_schema(schema),
                output_schema=_output_schema(raw),
                timeout_ms=TOOL_TIMEOUT_MS,
            )
        )

    return translated, skipped


def _describe(server_id: str, raw: dict) -> str:
    """Tool description, always attributed to its server.

    The description is model-facing text supplied by an external server, so it
    is attacker-reachable. Attribution is prepended by the bridge rather than
    trusted from the server, so the model always sees where a tool came from
    even if the server's own description claims otherwise.
    """
    description = raw.get("description")
    body = description.strip() if isinstance(description, str) and description.strip() else "No description provided."
    return f"[MCP server '{server_id}'] {body}"


def _clean_schema(schema: dict) -> dict:
    """Strip JSON-Schema meta keys the connector schema has no use for.

    ``$schema`` is a draft declaration, not a constraint — real servers emit
    it (``http://json-schema.org/draft-07/schema#``) and passing it through
    only adds noise to every model request.
    """
    return {key: value for key, value in schema.items() if key != "$schema"}


def _output_schema(raw: dict) -> dict:
    """Output schema for a bridged tool.

    A server that declares ``outputSchema`` gets it verbatim. Otherwise the
    schema describes MCP's actual reply envelope — a ``content`` array of typed
    blocks — because that is what the bridge returns, and claiming a richer
    shape would be a lie the model plans against.
    """
    declared = raw.get("outputSchema")
    if isinstance(declared, dict) and declared.get("type") == "object":
        return _clean_schema(declared)
    return {
        "type": "object",
        "properties": {
            "content": {
                "type": "array",
                "description": "MCP content blocks — each has a 'type' (text, image, resource).",
                "items": {"type": "object"},
            },
            "isError": {
                "type": "boolean",
                "description": "True when the server reported a tool-level failure.",
            },
        },
    }


# ── State helpers ────────────────────────────────────────────────────────────


def _servers() -> dict:
    servers = state.get("servers", None)
    return dict(servers) if isinstance(servers, dict) else {}


def _all_tools(servers: dict) -> list[AiTool]:
    """Rebuild the full declaration set.

    ``ExposeTools`` replaces the pane's whole registration, so a server going
    down must re-declare everything that is still up — otherwise its dead tools
    stay callable in the broker's snapshot.
    """
    tools: list[AiTool] = []
    for server in servers.values():
        if server.get("status") != "ready":
            continue
        tools.extend(_tool_from_record(record) for record in server.get("tool_records", []))
    return tools


def _tool_from_record(record: dict) -> AiTool:
    return AiTool(
        name=record["name"],
        description=record["description"],
        input_schema=record["input_schema"],
        output_schema=record["output_schema"],
        timeout_ms=TOOL_TIMEOUT_MS,
    )


def _server_for_tool(servers: dict, tool_name: str) -> str | None:
    for server_id, server in servers.items():
        if any(record["name"] == tool_name for record in server.get("tool_records", [])):
            return server_id
    return None


def _summary(servers: dict) -> str:
    ready = sum(1 for s in servers.values() if s.get("status") == "ready")
    tools = sum(len(s.get("tool_records", [])) for s in servers.values())
    skipped = sum(len(s.get("skipped", [])) for s in servers.values())
    parts = [f"{ready}/{len(servers)} servers", f"{tools} tools"]
    if skipped:
        parts.append(f"{skipped} skipped")
    return " · ".join(parts)


# ── View ─────────────────────────────────────────────────────────────────────


def view():
    fatal = state.get("fatal", None)
    if fatal:
        return Column(
            [AppBar("MCP Bridge"), Section("Not configured"), Text(fatal)],
            padding=12.0,
            gap=8.0,
        )

    servers = _servers()
    children: list = [AppBar("MCP Bridge", subtitle=_summary(servers))]
    for server_id, server in sorted(servers.items()):
        children.append(Section(server_id))
        children.append(Text(f"status: {server.get('status', 'unknown')}"))
        if server.get("error"):
            children.append(Text(f"error: {server['error']}"))
        for record in server.get("tool_records", []):
            children.append(Text(record["name"]))
        for skip in server.get("skipped", []):
            children.append(Text(f"skipped {skip['name']} — {skip['reason']}"))
    children.append(FooterKeys([("r", "reconnect")]))
    return Column(children, padding=12.0, gap=8.0)
