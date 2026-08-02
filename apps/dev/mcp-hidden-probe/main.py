"""MCP Hidden Probe — HostHarness fixture (stint 0382 fix round).

Connects to the MCP server configured as ``probe`` in the user's
``mcp_servers.toml``, sends one JSON-RPC request, and mirrors every phase into
the pane title. Titles are host-side state updated on logic-only passes, so a
harness test can drive this app with `hidden_frame` alone and prove the MCP
reply reached the guest while `App::ui` never ran — the regression that killed
healthy servers at the bridge's handshake deadline.

Phases: ``mcp-probe:connecting`` → ``mcp-probe:sent`` → ``mcp-probe:reply``,
with ``mcp-probe:error:*`` / ``mcp-probe:closed:*`` terminal states.
"""

from plexi_sdk import state
from plexi_sdk.effects import McpConnect, McpSend, SetState, SetTitle
from plexi_sdk.events import McpClosed, McpConnected, McpMessage
from plexi_sdk.ui import AppBar, Column, Text


def init(size, args):
    return [
        SetTitle("mcp-probe:connecting"),
        SetState({"phase": "connecting"}),
        McpConnect(request_id="connect:probe", server_id="probe"),
    ]


def update(event):
    if isinstance(event, McpConnected):
        if event.error:
            return [
                SetTitle(f"mcp-probe:error:{event.error}"),
                SetState({"phase": "error"}),
            ]
        return [
            SetTitle("mcp-probe:sent"),
            SetState({"phase": "sent"}),
            McpSend(
                server_id="probe",
                message={"jsonrpc": "2.0", "id": 1, "method": "ping", "params": {}},
            ),
        ]
    if isinstance(event, McpMessage):
        return [SetTitle("mcp-probe:reply"), SetState({"phase": "reply"})]
    if isinstance(event, McpClosed):
        return [
            SetTitle(f"mcp-probe:closed:{event.reason}"),
            SetState({"phase": "closed"}),
        ]
    return []


def view():
    return Column(
        [AppBar("MCP Hidden Probe"), Text(state.get("phase", "?"))],
        grow=True,
    )
