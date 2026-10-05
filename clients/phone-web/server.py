"""Local stub server for the Plexi phone shell.

Serves the static phone page and a stand-in turn API on one origin. This is
NOT the intake contract: no authentication, no host connection, no
persistence. Accepted turns are queued in memory and echoed back after a short
delay so the page's send / cancel / transcript paths can be exercised.

The request body loosely follows the proposed turn envelope
(schema_version, request_id, conversation_id, content) so the page will not
need reshaping when the real intake contract lands.

Run: python3 clients/phone-web/server.py [--host 127.0.0.1] [--port 8787]
"""

from __future__ import annotations

import argparse
import hmac
import json
import logging
import mimetypes
import os
import secrets
import socket
import subprocess
import threading
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

STATIC_DIR = Path(__file__).resolve().parent / "static"
ECHO_DELAY_SECONDS = 1.5
MAX_BODY_BYTES = 64 * 1024
MAX_TEXT_CHARS = 8000
TERMINAL_STATES = {"succeeded", "failed", "cancelled", "expired", "waiting_for_permission"}

log = logging.getLogger("plexi.phone_web")

mimetypes.add_type("application/manifest+json", ".webmanifest")
mimetypes.add_type("image/svg+xml", ".svg")


def now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


class StubStore:
    """In-memory transcript and receipts. One conversation, one process."""

    def __init__(self, echo_delay: float = ECHO_DELAY_SECONDS) -> None:
        self.lock = threading.Lock()
        self.events: list[dict] = []
        self.receipts: dict[str, dict] = {}
        self.payloads: dict[str, str] = {}
        self.echo_delay = echo_delay

    def _append(self, event: dict) -> None:
        event["cursor"] = len(self.events) + 1
        event["at"] = now()
        self.events.append(event)

    def submit(self, envelope: dict) -> tuple[int, dict]:
        request_id = envelope["request_id"]
        canonical = json.dumps(envelope, sort_keys=True)
        with self.lock:
            if request_id in self.receipts:
                if self.payloads[request_id] != canonical:
                    return 409, {"error": "request_id_reused_with_different_payload"}
                return 200, dict(self.receipts[request_id])
            receipt = {"request_id": request_id, "state": "queued", "updated_at": now()}
            self.receipts[request_id] = receipt
            self.payloads[request_id] = canonical
            text = envelope["content"][0]["text"]
            self._append({"kind": "user", "request_id": request_id, "text": text})
            self._append({"kind": "receipt", "request_id": request_id, "state": "queued"})
        log.info("turn accepted request_id=%s chars=%d state=queued", request_id, len(text))
        timer = threading.Timer(self.echo_delay, self._echo, args=(request_id, text))
        timer.daemon = True
        timer.start()
        return 202, dict(receipt)

    def _echo(self, request_id: str, text: str) -> None:
        with self.lock:
            receipt = self.receipts[request_id]
            if receipt["state"] in TERMINAL_STATES:
                return
            receipt.update(state="succeeded", updated_at=now())
            self._append({"kind": "stub_reply", "request_id": request_id, "text": f"Stub echo: {text}"})
            self._append({"kind": "receipt", "request_id": request_id, "state": "succeeded"})
        log.info("turn echoed request_id=%s", request_id)

    def cancel(self, request_id: str) -> tuple[int, dict]:
        with self.lock:
            receipt = self.receipts.get(request_id)
            if receipt is None:
                return 404, {"error": "unknown_request_id"}
            if receipt["state"] not in TERMINAL_STATES:
                receipt.update(state="cancelled", updated_at=now())
                self._append({"kind": "receipt", "request_id": request_id, "state": "cancelled"})
                log.info("turn cancelled request_id=%s", request_id)
            return 200, dict(receipt)

    def since(self, cursor: int) -> dict:
        with self.lock:
            return {"cursor": len(self.events), "events": [dict(e) for e in self.events[cursor:]]}


class HostStore(StubStore):
    """In-memory phone receipts backed by the installed host CLI."""
    def __init__(self, plexi_bin: str) -> None:
        super().__init__()
        self.plexi_bin = plexi_bin

    def submit(self, envelope: dict) -> tuple[int, dict]:
        status, receipt = super().submit(envelope)
        if status == 202:
            threading.Thread(target=self._host_turn, args=(envelope["request_id"], envelope["content"][0]["text"]), daemon=True).start()
        return status, receipt

    def _echo(self, request_id: str, text: str) -> None:
        # HostStore starts its own worker; the inherited timer is deliberately not used.
        return

    def _host_turn(self, request_id: str, text: str) -> None:
        with self.lock:
            self.receipts[request_id].update(state="running", updated_at=now())
            self._append({"kind": "receipt", "request_id": request_id, "state": "running"})
        try:
            proc = subprocess.run([self.plexi_bin, "assistant", "send", "--text", text, "--request-id", request_id, "--json"], capture_output=True, text=True, timeout=120, check=False)
            if not proc.stdout.strip():
                raise RuntimeError("no reply from the host Assistant within 120 s (a permission prompt may be waiting on the desktop)")
            payload = json.loads(proc.stdout.strip())
            state = payload.get("state", "failed")
            reply = payload.get("reply")
            error = payload.get("error") or proc.stderr.strip()
            if state != "succeeded" and not error:
                error = f"host turn ended in {state}"
        except subprocess.TimeoutExpired:
            state, reply, error = "failed", None, "no reply from the host Assistant within 120 s (a permission prompt may be waiting on the desktop)"
        except (OSError, RuntimeError, json.JSONDecodeError) as exc:
            state, reply, error = "failed", None, str(exc)
        with self.lock:
            receipt = self.receipts[request_id]
            if receipt["state"] == "cancelled":
                return
            receipt.update(state=state, updated_at=now())
            if reply:
                self._append({"kind": "assistant_reply", "request_id": request_id, "text": reply})
            if error:
                receipt["error"] = error
            event = {"kind": "receipt", "request_id": request_id, "state": state}
            if state != "succeeded" and error:
                event["error"] = error
            self._append(event)


def validate_envelope(body: object) -> str | None:
    if not isinstance(body, dict):
        return "body_must_be_object"
    if body.get("schema_version") != 1:
        return "unsupported_schema_version"
    request_id = body.get("request_id")
    if not isinstance(request_id, str) or not 1 <= len(request_id) <= 128:
        return "invalid_request_id"
    content = body.get("content")
    if not isinstance(content, list) or len(content) != 1:
        return "content_must_be_one_text_part"
    part = content[0]
    if not isinstance(part, dict) or part.get("type") != "text":
        return "content_must_be_one_text_part"
    text = part.get("text")
    if not isinstance(text, str) or not text.strip() or len(text) > MAX_TEXT_CHARS:
        return "invalid_text"
    return None


def make_handler(store: StubStore, token: str | None = None) -> type[BaseHTTPRequestHandler]:
    class Handler(BaseHTTPRequestHandler):
        server_version = "PlexiPhoneShellStub/0"

        def log_message(self, fmt: str, *args: object) -> None:
            log.info("%s %s", self.address_string(), fmt % args)

        def _json(self, status: int, payload: dict) -> None:
            data = json.dumps(payload).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def _authorized(self) -> bool:
            if token is None:
                return True
            supplied = self.headers.get("Authorization", "")
            return hmac.compare_digest(supplied, f"Bearer {token}")

        def _api_auth(self, path: str) -> bool:
            if path.startswith("/api/") and not self._authorized():
                self._json(401, {"error": "unauthorized"})
                return False
            return True

        def do_GET(self) -> None:  # noqa: N802
            url = urlparse(self.path)
            if not self._api_auth(url.path): return
            if url.path == "/api/status":
                self._json(200, {"mode": "host" if isinstance(store, HostStore) else "local_stub", "host": "configured" if isinstance(store, HostStore) else "not_connected", "intake_contract": None})
                return
            if url.path == "/api/conversation":
                try:
                    cursor = max(0, int(parse_qs(url.query).get("after", ["0"])[0]))
                except ValueError:
                    self._json(400, {"error": "invalid_cursor"})
                    return
                self._json(200, store.since(cursor))
                return
            self._static(url.path)

        def do_POST(self) -> None:  # noqa: N802
            url = urlparse(self.path)
            if not self._api_auth(url.path): return
            parts = url.path.strip("/").split("/")
            try:
                length = int(self.headers.get("Content-Length", "0"))
            except ValueError:
                self._json(400, {"error": "invalid_content_length"})
                return
            if length > MAX_BODY_BYTES:
                self._json(413, {"error": "body_too_large"})
                return
            raw = self.rfile.read(length) if length else b""
            if parts == ["api", "turns"]:
                try:
                    body = json.loads(raw or b"null")
                except json.JSONDecodeError as exc:
                    log.warning("turn rejected: bad json (%s)", exc)
                    self._json(400, {"error": "invalid_json"})
                    return
                problem = validate_envelope(body)
                if problem:
                    log.warning("turn rejected: %s", problem)
                    self._json(400, {"error": problem})
                    return
                self._json(*store.submit(body))
                return
            if len(parts) == 4 and parts[:2] == ["api", "turns"] and parts[3] == "cancel":
                self._json(*store.cancel(parts[2]))
                return
            self._json(404, {"error": "not_found"})

        def _static(self, path: str) -> None:
            rel = "index.html" if path in ("", "/") else path.lstrip("/")
            target = (STATIC_DIR / rel).resolve()
            if STATIC_DIR not in target.parents or not target.is_file():
                self._json(404, {"error": "not_found"})
                return
            try:
                data = target.read_bytes()
            except OSError as exc:
                log.error("static read failed path=%s: %s", target, exc)
                self._json(500, {"error": "read_failed"})
                return
            self.send_response(200)
            self.send_header("Content-Type", mimetypes.guess_type(target.name)[0] or "application/octet-stream")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

    return Handler


def build_server(host: str, port: int, store: StubStore | None = None, token: str | None = None) -> ThreadingHTTPServer:
    return ThreadingHTTPServer((host, port), make_handler(store or StubStore(), token))


def main() -> None:
    parser = argparse.ArgumentParser(description="Plexi phone shell")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8787)
    parser.add_argument("--backend", choices=("stub", "host"), default=os.getenv("PLEXI_PHONE_BACKEND", "stub"))
    parser.add_argument("--plexi-bin", default=os.getenv("PLEXI_BIN", "plexi"))
    parser.add_argument("--lan", action="store_true", default=os.getenv("PLEXI_PHONE_LAN") == "1")
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s %(message)s")
    host = "0.0.0.0" if args.lan else args.host
    require_token = args.backend == "host" or host not in ("127.0.0.1", "::1", "localhost")
    token = os.getenv("PLEXI_PHONE_TOKEN") or (secrets.token_urlsafe(32) if require_token else None)
    store = HostStore(args.plexi_bin) if args.backend == "host" else StubStore()
    server = build_server(host, args.port, store, token)
    ips = {"127.0.0.1"}
    try:
        ips.update(socket.gethostbyname_ex(socket.gethostname())[2])
    except socket.gaierror:
        pass
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
            probe.connect(("8.8.8.8", 80))
            ips.add(probe.getsockname()[0])
    except OSError:
        pass
    suffix = f"/?token={token}" if token else "/"
    for ip in sorted(ips): log.info("phone shell URL: http://%s:%d%s", ip, args.port, suffix)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        log.info("phone shell stopping")
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
