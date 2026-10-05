#!/usr/bin/env python3
"""Tiny OpenAI-compatible chat server for the phone-relay install check.

A prompt containing HOLD-FOR-TTL sleeps before answering, so the desktop can
die while the relay still holds the body. A prompt containing APPROVAL-TOOL
returns a host.files.write tool call. That tool is not read-only, so the
desktop's default ask posture opens a permission sheet.
"""

from __future__ import annotations

import json
import os
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HOLD_SECONDS = float(os.environ.get("MOCK_HOLD_SECONDS", "20"))


def sse(payload: dict) -> bytes:
    return f"data: {json.dumps(payload)}\n\n".encode()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt: str, *args: object) -> None:
        print(f"mock {self.command} {self.path} {args[1] if len(args) > 1 else ''}", flush=True)

    def do_POST(self) -> None:  # noqa: N802
        length = int(self.headers.get("Content-Length", "0") or 0)
        raw = self.rfile.read(length) if length else b""
        try:
            body = json.loads(raw.decode() or "{}")
        except json.JSONDecodeError:
            body = {}
        text = ""
        for message in body.get("messages") or []:
            content = message.get("content")
            if isinstance(content, str):
                text += content
        if "HOLD-FOR-TTL" in text:
            time.sleep(HOLD_SECONDS)
            chunks = [text_chunk("held"), stop_chunk()]
        elif "APPROVAL-TOOL" in text:
            chunks = [tool_chunk(), tool_done_chunk()]
        else:
            chunks = [text_chunk("mock-reply"), stop_chunk()]
        data = b"".join(chunks) + b"data: [DONE]\n\n"
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self) -> None:  # noqa: N802
        data = b'{"ok":true}'
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


def text_chunk(text: str) -> bytes:
    return sse({"choices": [{"index": 0, "delta": {"content": text}, "finish_reason": None}]})


def stop_chunk() -> bytes:
    return sse({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]})


def tool_chunk() -> bytes:
    return sse(
        {
            "choices": [
                {
                    "index": 0,
                    "delta": {
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": "call_phone",
                                "type": "function",
                                "function": {
                                    "name": "host_files_write",
                                    "arguments": "{\"path\":\"phone-e2e.txt\",\"content\":\"held\"}",
                                },
                            }
                        ]
                    },
                    "finish_reason": None,
                }
            ]
        }
    )


def tool_done_chunk() -> bytes:
    return sse({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]})


def main() -> None:
    host = os.environ.get("MOCK_HOST", "127.0.0.1")
    port = int(os.environ.get("MOCK_PORT", "8766"))
    server = ThreadingHTTPServer((host, port), Handler)
    print(f"mock openai listening on {host}:{port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
