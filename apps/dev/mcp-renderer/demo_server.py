#!/usr/bin/env python3
"""Minimal stdio MCP server — 3 toy tools for testing mcp-renderer.

Launch via:
    plexi open mcp-renderer python3 examples/mcp-renderer/demo_server.py
"""

import json
import sys

TOOLS = [
    {
        "name": "echo",
        "description": "Returns whatever text you send it",
        "inputSchema": {
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "Text to echo back"},
            },
            "required": ["text"],
        },
    },
    {
        "name": "add",
        "description": "Adds two numbers together",
        "inputSchema": {
            "type": "object",
            "properties": {
                "a": {"type": "number", "description": "First number"},
                "b": {"type": "number", "description": "Second number"},
            },
            "required": ["a", "b"],
        },
    },
    {
        "name": "reverse",
        "description": "Reverses a string",
        "inputSchema": {
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "Text to reverse"},
            },
            "required": ["text"],
        },
    },
]


def send(msg: dict) -> None:
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


def call_tool(name: str, args: dict) -> str:
    if name == "echo":
        return args.get("text", "")
    if name == "add":
        a = float(args.get("a", 0))
        b = float(args.get("b", 0))
        result = a + b
        return str(int(result) if result == int(result) else result)
    if name == "reverse":
        return args.get("text", "")[::-1]
    return f"Unknown tool: {name}"


def main() -> None:
    for raw in sys.stdin:
        raw = raw.strip()
        if not raw:
            continue
        try:
            msg = json.loads(raw)
        except json.JSONDecodeError:
            continue

        msg_id = msg.get("id")
        method = msg.get("method", "")

        # Notifications (no id) — no response
        if msg_id is None:
            continue

        if method == "initialize":
            send({
                "jsonrpc": "2.0",
                "id": msg_id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "demo-server", "version": "0.1.0"},
                },
            })
        elif method == "tools/list":
            send({"jsonrpc": "2.0", "id": msg_id, "result": {"tools": TOOLS}})
        elif method == "tools/call":
            params = msg.get("params", {})
            name = params.get("name", "")
            args = params.get("arguments", {})
            text = call_tool(name, args)
            send({
                "jsonrpc": "2.0",
                "id": msg_id,
                "result": {"content": [{"type": "text", "text": text}]},
            })
        else:
            send({
                "jsonrpc": "2.0",
                "id": msg_id,
                "error": {"code": -32601, "message": f"Method not found: {method}"},
            })


if __name__ == "__main__":
    main()
