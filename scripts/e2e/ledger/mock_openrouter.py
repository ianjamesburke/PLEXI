#!/usr/bin/env python3
"""OpenAI-compatible mock for the Assistant ledger end-to-end check.

The live host always posts `stream: true` to `{PLEXI_OPENROUTER_BASE_URL}/chat/completions`.
This server selects a fixture from the last user message:

- ``LEDGER_STREAM_A`` — SSE. Content chunk, a finish chunk with no usage, then
  OpenRouter's final chunk: non-empty ``choices`` repeating ``finish_reason``
  plus ``usage`` 194 prompt / 12 completion.
- ``LEDGER_STREAM_B`` — same SSE shape, usage 10 / 4.
- ``LEDGER_SYSTEM`` — same SSE shape, usage 80 / 15.
- ``LEDGER_NONE`` — SSE that ends without a ``usage`` object.
- ``LEDGER_JSON`` — one ``application/json`` chat completion (non-streaming),
  usage 50 / 7, even though the client asked to stream.

No ``X-Generation-Id`` header. The host then records the completion's own
usage immediately instead of querying the real OpenRouter generation API.

Prints ``PORT=<n>`` on stdout and serves until killed. ``GET /health`` returns
200.
"""

from __future__ import annotations

import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


SCENARIOS = {
    "LEDGER_STREAM_A": ("sse", 194, 12, "stream-a"),
    "LEDGER_STREAM_B": ("sse", 10, 4, "stream-b"),
    "LEDGER_SYSTEM": ("sse", 80, 15, "system-turn"),
    "LEDGER_NONE": ("sse-none", None, None, "no-usage"),
    "LEDGER_JSON": ("json", 50, 7, "json-body"),
}


def last_user_text(body: dict) -> str:
    messages = body.get("messages") or []
    for message in reversed(messages):
        if not isinstance(message, dict) or message.get("role") != "user":
            continue
        content = message.get("content")
        if isinstance(content, str):
            return content
        if isinstance(content, list):
            parts = []
            for part in content:
                if isinstance(part, str):
                    parts.append(part)
                elif isinstance(part, dict) and isinstance(part.get("text"), str):
                    parts.append(part["text"])
            return "\n".join(parts)
    return ""


def scenario_for(text: str) -> tuple:
    for marker, spec in SCENARIOS.items():
        if marker in text:
            return spec
    return SCENARIOS["LEDGER_STREAM_A"]


def sse_chunk(payload: dict) -> bytes:
    return f"data: {json.dumps(payload, separators=(',', ':'))}\n\n".encode()


def sse_body(text: str, prompt: int | None, completion: int | None) -> bytes:
    chunks = [
        sse_chunk(
            {
                "id": "gen-e2e",
                "object": "chat.completion.chunk",
                "choices": [
                    {
                        "index": 0,
                        "delta": {"role": "assistant", "content": text},
                        "finish_reason": None,
                    }
                ],
            }
        ),
        sse_chunk(
            {
                "id": "gen-e2e",
                "object": "chat.completion.chunk",
                "choices": [
                    {
                        "index": 0,
                        "delta": {},
                        "finish_reason": "stop",
                        "native_finish_reason": "stop",
                    }
                ],
            }
        ),
    ]
    if prompt is not None and completion is not None:
        chunks.append(
            sse_chunk(
                {
                    "id": "gen-e2e",
                    "object": "chat.completion.chunk",
                    "choices": [
                        {
                            "index": 0,
                            "delta": {},
                            "finish_reason": "stop",
                            "native_finish_reason": "stop",
                        }
                    ],
                    "usage": {
                        "prompt_tokens": prompt,
                        "completion_tokens": completion,
                        "total_tokens": prompt + completion,
                    },
                }
            )
        )
    chunks.append(b"data: [DONE]\n\n")
    return b"".join(chunks)


def json_body(text: str, prompt: int, completion: int) -> bytes:
    return json.dumps(
        {
            "id": "chatcmpl-e2e",
            "object": "chat.completion",
            "choices": [
                {
                    "index": 0,
                    "message": {"role": "assistant", "content": text},
                    "finish_reason": "stop",
                }
            ],
            "usage": {
                "prompt_tokens": prompt,
                "completion_tokens": completion,
                "total_tokens": prompt + completion,
            },
        }
    ).encode()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt: str, *args) -> None:
        sys.stderr.write("mock_openrouter: " + (fmt % args) + "\n")

    def do_GET(self) -> None:  # noqa: N802
        if self.path.split("?", 1)[0] == "/health":
            body = b"ok\n"
            self._send(200, "text/plain", body)
            return
        self._send(404, "text/plain", b"not found\n")

    def do_POST(self) -> None:  # noqa: N802
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length)
        try:
            body = json.loads(raw.decode() or "{}")
        except json.JSONDecodeError:
            self._send(400, "application/json", b'{"error":{"message":"invalid json"}}')
            return
        kind, prompt, completion, text = scenario_for(last_user_text(body))
        sys.stderr.write(f"mock_openrouter: scenario={kind} text={text!r}\n")
        if kind == "json":
            self._send(200, "application/json", json_body(text, prompt, completion))
            return
        if kind == "sse-none":
            payload = sse_body(text, None, None)
        else:
            payload = sse_body(text, prompt, completion)
        self._send(200, "text/event-stream", payload)

    def _send(self, status: int, content_type: str, payload: bytes) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


def main() -> None:
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 0
    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    bound = server.server_address[1]
    print(f"PORT={bound}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
