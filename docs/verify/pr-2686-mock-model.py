#!/usr/bin/env python3
"""OpenAI-compatible SSE mock. Returns one chess.play tool call, then a short reply.

Control file (JSON): {"arguments": {...}, "tool_substr": "chess_play"}
The tool name is taken from the incoming request's tools list so it matches
the host's wire name (dots become underscores).
"""
from __future__ import annotations

import json
import os
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

CONTROL = os.environ.get("MOCK_CONTROL", "/tmp/verify-pr-2686/move.json")
PORT = int(os.environ.get("MOCK_PORT", "8765"))


def control() -> dict:
    try:
        with open(CONTROL, encoding="utf-8") as handle:
            return json.load(handle)
    except OSError:
        return {}


def wire(name: str) -> str:
    return "".join(ch if ch.isalnum() or ch in "_-" else "_" for ch in name)


def sse(payload: dict) -> bytes:
    return f"data: {json.dumps(payload)}\n\n".encode()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt: str, *args) -> None:
        sys.stderr.write("mock_model: " + (fmt % args) + "\n")

    def do_POST(self) -> None:  # noqa: N802
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length)
        try:
            body = json.loads(raw.decode() or "{}")
        except json.JSONDecodeError:
            body = {}
        messages = body.get("messages") or []
        tools = body.get("tools") or []
        has_tool_result = any(m.get("role") == "tool" for m in messages)
        if has_tool_result:
            chunks = [
                {"choices": [{"index": 0, "delta": {"content": "Move submitted."}}]},
                {"choices": [{"index": 0, "finish_reason": "stop"}]},
                {"choices": [], "usage": {"prompt_tokens": 1, "completion_tokens": 1}},
            ]
        else:
            spec = control()
            want = spec.get("tool_substr", "play")
            chosen = None
            for tool in tools:
                fn = (tool.get("function") or {}).get("name") or ""
                if want in fn:
                    chosen = fn
                    break
            if chosen is None and tools:
                chosen = (tools[0].get("function") or {}).get("name") or "chess__chess_play"
            chosen = chosen or "chess__chess_play"
            arguments = json.dumps(spec.get("arguments") or {})
            chunks = [
                {
                    "choices": [
                        {
                            "index": 0,
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_mock_1",
                                        "function": {"name": chosen, "arguments": ""},
                                    }
                                ]
                            },
                        }
                    ]
                },
                {
                    "choices": [
                        {
                            "index": 0,
                            "delta": {
                                "tool_calls": [
                                    {"index": 0, "function": {"arguments": arguments}}
                                ]
                            },
                        }
                    ]
                },
                {"choices": [{"index": 0, "finish_reason": "tool_calls"}]},
            ]
        blob = b"".join(sse(chunk) for chunk in chunks) + b"data: [DONE]\n\n"
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Content-Length", str(len(blob)))
        self.end_headers()
        self.wfile.write(blob)


def main() -> None:
    server = ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    print(f"mock_model listening on 127.0.0.1:{PORT}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
