"""Local server for the Plexi phone shell.

Serves the static phone page and a turn API on one origin. The default mode
is an in-memory echo. Host mode runs `plexi assistant send` and requires a
bearer token even on loopback. LAN mode is plain HTTP. Receipts stay in
memory. This is not a phone relay and not the intake contract.

The request body loosely follows the proposed turn envelope
(schema_version, request_id, conversation_id, content) so the page will not
need reshaping when the real intake contract lands.

Run: python3 clients/phone-web/server.py [--host 127.0.0.1] [--port 8787]
"""

from __future__ import annotations

import argparse
import hmac
import ipaddress
import json
import logging
import mimetypes
import os
import re
import secrets
import socket
import subprocess
import sys
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


def parse_ifconfig_ipv4(output: str) -> list[tuple[str, str]]:
    """Parse IPv4 addresses from BSD/macOS (and compatible Linux) ifconfig."""
    addresses: list[tuple[str, str]] = []
    interface: str | None = None
    for line in output.splitlines():
        match = re.match(r"^([^\s:]+)(?::[^\s]*)?:\s", line)
        if match:
            interface = match.group(1)
            continue
        match = re.match(r"^\s*inet\s+(?:addr:)?(\d+\.\d+\.\d+\.\d+)", line)
        if interface and match:
            addresses.append((interface, match.group(1)))
    return addresses


def parse_ip_o_ipv4(output: str) -> list[tuple[str, str]]:
    """Parse `ip -o -4 addr show` output without invoking any network APIs."""
    addresses: list[tuple[str, str]] = []
    for line in output.splitlines():
        match = re.match(r"^\d+:\s+([^@\s:]+)(?:@\S+)?\s+inet\s+(\d+\.\d+\.\d+\.\d+)/", line)
        if match:
            addresses.append((match.group(1), match.group(2)))
    return addresses


def primary_ipv4() -> str | None:
    """Return the IPv4 address selected by the OS default route, if usable."""
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
            # UDP connect only selects a route; it does not send a packet.
            probe.connect(("8.8.8.8", 80))
            address = probe.getsockname()[0]
    except OSError:
        return None
    return address if _is_reachable_ipv4(address) else None


def interface_ipv4s() -> list[tuple[str, str]]:
    """Best-effort interface IPv4 discovery for display only."""
    try:
        if sys.platform.startswith(("darwin", "freebsd", "openbsd", "netbsd")):
            result = subprocess.run(["ifconfig"], capture_output=True, text=True, timeout=1, check=True)
            return parse_ifconfig_ipv4(result.stdout)
        try:
            result = subprocess.run(["ip", "-o", "-4", "addr", "show"], capture_output=True, text=True, timeout=1, check=True)
            return parse_ip_o_ipv4(result.stdout)
        except FileNotFoundError:
            result = subprocess.run(["ifconfig"], capture_output=True, text=True, timeout=1, check=True)
            return parse_ifconfig_ipv4(result.stdout)
    except (OSError, subprocess.SubprocessError):
        return []


def _is_reachable_ipv4(address: str) -> bool:
    try:
        parsed = ipaddress.IPv4Address(address)
    except ipaddress.AddressValueError:
        return False
    return not (parsed.is_loopback or parsed.is_unspecified or parsed.is_link_local)


def _is_tailscale(address: str) -> bool:
    try:
        return ipaddress.IPv4Address(address) in ipaddress.IPv4Network("100.64.0.0/10")
    except ipaddress.AddressValueError:
        return False


def _is_virtual_interface(name: str) -> bool:
    lowered = name.lower()
    prefixes = ("bridge", "utun", "awdl", "llw", "anpi", "ap", "gif", "stf", "vmnet", "vboxnet",
                "docker", "br-", "veth", "virbr", "cni", "flannel", "cali", "zt", "tun", "tap")
    return lowered.startswith(prefixes)


def _is_real_interface(name: str) -> bool:
    return name.lower().startswith(("en", "eth", "wl", "wlan"))


def select_urls(primary: str | None, interfaces: list[tuple[str, str]]) -> list[tuple[str, str]]:
    """Order safe phone URLs as (human label, IPv4 address), without I/O."""
    by_ip: dict[str, list[str]] = {}
    for name, address in interfaces:
        if _is_reachable_ipv4(address) or address.startswith("127."):
            by_ip.setdefault(address, []).append(name)

    def name_for(address: str) -> str | None:
        names = by_ip.get(address, [])
        return next((name for name in names if _is_real_interface(name)), names[0] if names else None)

    chosen: list[tuple[str, str]] = []
    used: set[str] = set()

    def add(label: str, address: str) -> None:
        if address not in used:
            chosen.append((label, address))
            used.add(address)

    primary_name = name_for(primary) if primary else None
    primary_is_virtual = bool(primary_name and _is_virtual_interface(primary_name))
    if primary and _is_reachable_ipv4(primary) and not primary_is_virtual:
        suffix = f", {primary_name}" if primary_name else ""
        add(f"LAN (default route, use this on the same Wi-Fi{suffix})", primary)

    for address, names in by_ip.items():
        if _is_tailscale(address):
            add("Tailscale", address)

    for address, names in by_ip.items():
        name = next((candidate for candidate in names if _is_real_interface(candidate)), None)
        if name and not _is_tailscale(address):
            add(f"other interface ({name})", address)

    if primary and _is_reachable_ipv4(primary) and primary_is_virtual:
        add("virtual interface, probably not reachable from your phone", primary)

    add("this machine only", "127.0.0.1")
    return chosen


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

    def needs_you(self) -> tuple[int, dict]:
        return 200, {"ok": True, "items": []}

    def resolve_needs_you(self, item_id: str, decision: str) -> tuple[int, dict]:
        log.info("needs-you stub resolve id=%s decision=%s refused", item_id, decision)
        return 503, {"ok": False, "error": "needs-you requires the host"}


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

    def needs_you(self) -> tuple[int, dict]:
        return self._needs_you_cli(["needs-you", "list", "--json"])

    def resolve_needs_you(self, item_id: str, decision: str) -> tuple[int, dict]:
        if decision == "approve" and not self._phone_may_approve(item_id):
            log.info("needs-you phone refused approve id=%s", item_id)
            return 403, {
                "ok": False,
                "error_code": "permission_denied",
                "error": "only the person at the desktop can resolve a permission",
                "id": item_id,
            }
        flag = "--approve" if decision == "approve" else "--deny"
        return self._needs_you_cli(["needs-you", "resolve", item_id, flag])

    def _phone_may_approve(self, item_id: str) -> bool:
        """A phone may answer a question or a blocked run. It never grants a permission."""
        status, body = self.needs_you()
        if status != 200 or body.get("ok") is not True:
            log.info("needs-you phone approve blocked; list failed id=%s", item_id)
            return False
        items = body.get("items")
        if not isinstance(items, list):
            return False
        for item in items:
            if not isinstance(item, dict) or item.get("id") != item_id:
                continue
            kind = item.get("kind")
            allowed = kind in ("question", "blocked_run")
            log.info("needs-you phone approve id=%s kind=%s allowed=%s", item_id, kind, allowed)
            return allowed
        log.info("needs-you phone approve blocked; unknown id=%s", item_id)
        return False

    def _needs_you_cli(self, args: list[str]) -> tuple[int, dict]:
        log.info("needs-you phone %s", " ".join(args))
        try:
            proc = subprocess.run(
                [self.plexi_bin, *args],
                capture_output=True,
                text=True,
                timeout=20,
                check=False,
            )
        except (OSError, subprocess.SubprocessError) as exc:
            log.error("needs-you phone failed: %s", exc)
            return 502, {"ok": False, "error": str(exc)}
        raw = proc.stdout.strip()
        try:
            payload = json.loads(raw) if raw else {"ok": False, "error": proc.stderr.strip() or "empty reply"}
        except json.JSONDecodeError:
            log.error("needs-you phone bad json: %s", raw)
            return 502, {"ok": False, "error": proc.stderr.strip() or "bad json"}
        if not isinstance(payload, dict):
            return 502, {"ok": False, "error": "bad json"}
        error = str(payload.get("error", ""))
        if payload.get("ok") is True:
            status = 200
        elif "unknown" in error:
            status = 404
        elif payload.get("already") is True:
            status = 409
        else:
            status = 400
        return status, payload


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
            if url.path == "/api/needs-you":
                self._json(*store.needs_you())
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
            if len(parts) == 4 and parts[0] == "api" and parts[1] == "needs-you" and parts[3] == "resolve":
                try:
                    body = json.loads(raw or b"null")
                except json.JSONDecodeError:
                    self._json(400, {"ok": False, "error": "invalid_json"})
                    return
                decision = body.get("decision") if isinstance(body, dict) else None
                if decision not in ("approve", "deny"):
                    log.warning("needs-you rejected decision=%s", decision)
                    self._json(400, {"ok": False, "error": "decision must be approve or deny"})
                    return
                self._json(*store.resolve_needs_you(parts[2], decision))
                return
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
    suffix = f"/?token={token}" if token else "/"
    if host in ("127.0.0.1", "::1", "localhost"):
        urls = [("this machine only", "127.0.0.1")]
    else:
        urls = select_urls(primary_ipv4(), interface_ipv4s())
        if not urls:
            urls = [("this machine only", "127.0.0.1")]
    for index, (label, ip) in enumerate(urls):
        prefix = "open this on your phone, " if index == 0 and ip != "127.0.0.1" else ""
        log.info("phone shell URL (%s%s): http://%s:%d%s", prefix, label, ip, args.port, suffix)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        log.info("phone shell stopping")
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
