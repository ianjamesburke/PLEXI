#!/usr/bin/env python3
"""OpenAI-compatible chat completions stand-in for lead turns.

The first stdout line is the listen port. Later lines are logs. The server
answers from the messages in that one request, so two heads never share a
transcript. It does not read a keychain, an API key, or any remote host.
"""

from __future__ import annotations

import json
import re
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def _text_of(message: dict) -> str:
    content = message.get("content")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts = []
        for item in content:
            if isinstance(item, dict) and isinstance(item.get("text"), str):
                parts.append(item["text"])
        return "\n".join(parts)
    return ""


def _reply(content: str | None, tool_calls: list | None = None) -> bytes:
    message: dict = {"role": "assistant", "content": content}
    if tool_calls:
        message["tool_calls"] = tool_calls
    body = {"choices": [{"message": message}]}
    return json.dumps(body).encode()


def _tool(name: str, arguments: dict, call_id: str = "call_mock") -> dict:
    return {
        "id": call_id,
        "type": "function",
        "function": {"name": name, "arguments": json.dumps(arguments)},
    }


def decide(messages: list) -> bytes:
    blob = "\n".join(_text_of(message) for message in messages if isinstance(message, dict))
    users = [
        _text_of(message)
        for message in messages
        if isinstance(message, dict) and message.get("role") == "user"
    ]
    latest = users[-1] if users else blob
    if "a lead cannot read another lead" in blob:
        return _reply("I could not read it")
    if "wrote " in blob or "wrote the file" in blob:
        return _reply("wrote the file")
    read_other = re.search(r"read-other\s+([a-z0-9-]+)", latest)
    if read_other:
        return _reply(
            None,
            [_tool("leads.conversation.read", {"head": read_other.group(1)})],
        )
    if "write-file" in latest or "write file" in latest:
        return _reply(
            None,
            [_tool("host.files.write", {"file": "out.txt", "content": "ok"})],
        )
    if "long-task" in latest or "long-task" in blob:
        done = len(re.findall(r"step \d+", blob))
        if done >= 4:
            return _reply("long task done")
        return _reply(None, [_tool("lead.step", {"n": done})])
    if "what number?" in latest:
        remembered = re.findall(r"remember (\d+)", blob)
        if remembered:
            return _reply(remembered[-1])
        return _reply("I do not know a number")
    remembered = re.search(r"remember (\d+)", latest)
    if remembered:
        return _reply(f"Noted {remembered.group(1)}")
    if "status?" in latest:
        return _reply("idle")
    return _reply("ok")


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):  # noqa: N802
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length)
        try:
            body = json.loads(raw.decode() or "{}")
        except json.JSONDecodeError:
            body = {}
        messages = body.get("messages") if isinstance(body, dict) else []
        if not isinstance(messages, list):
            messages = []
        payload = decide(messages)
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, fmt: str, *args) -> None:
        sys.stderr.write("lead_mock: " + (fmt % args) + "\n")


def main() -> None:
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    port = server.server_address[1]
    print(port, flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
