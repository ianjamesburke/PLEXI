"""Local stub server for the Plexi phone shell.

Serves the static phone page and a stand-in turn API on one origin. This is
NOT the intake contract: no authentication, no host connection, no
persistence. Accepted turns are queued in memory and echoed back after a short
delay so the page's send / cancel / transcript paths can be exercised.

The request body loosely follows the proposed turn envelope
(schema_version, request_id, conversation_id, content) so the page will not
need reshaping when the real intake contract lands.

Run: python3 clients/phone-web/server.py [--host 127.0.0.1] [--port 8787]
     python3 clients/phone-web/server.py --tailscale [--port 8787]
"""

from __future__ import annotations

import argparse
import hashlib
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
import time
import uuid
from collections.abc import Callable
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


class TailscaleUnavailable(RuntimeError):
    """`tailscale ip -4` could not provide a bind address."""


def select_tailscale_ipv4(output: str) -> str:
    """First Tailscale IPv4 in `tailscale ip -4` stdout. No I/O."""
    for line in output.splitlines():
        text = line.strip()
        if not text:
            continue
        try:
            parsed = ipaddress.IPv4Address(text)
        except ipaddress.AddressValueError:
            continue
        if _is_tailscale(str(parsed)):
            return str(parsed)
    raise TailscaleUnavailable(
        "`tailscale ip -4` did not return a Tailscale IPv4 (100.64.0.0/10). "
        "Start the Tailscale app, then retry --tailscale."
    )


def magicdns_name_from_status(output: str) -> str | None:
    """MagicDNS hostname from `tailscale status --json`, or None."""
    try:
        payload = json.loads(output)
    except json.JSONDecodeError:
        return None
    self_info = payload.get("Self") if isinstance(payload, dict) else None
    if not isinstance(self_info, dict):
        return None
    name = self_info.get("DNSName")
    if not isinstance(name, str):
        return None
    cleaned = name.strip().rstrip(".")
    if not re.fullmatch(
        r"[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?)+",
        cleaned,
    ):
        return None
    return cleaned


def resolve_tailscale_endpoint(run: Callable[..., subprocess.CompletedProcess[str]] | None = None) -> tuple[str, str | None]:
    """Return (Tailscale IPv4, MagicDNS name or None).

    The address always comes from `tailscale ip -4`. MagicDNS is read from
    `tailscale status --json` when that command succeeds. `run` is injectable
    and defaults to subprocess.run.
    """
    invoke = run or subprocess.run
    try:
        ip_result = invoke(["tailscale", "ip", "-4"], capture_output=True, text=True, timeout=5, check=False)
    except FileNotFoundError as exc:
        raise TailscaleUnavailable(
            "Tailscale is not available (`tailscale` was not found). "
            "Install Tailscale, start it, then retry --tailscale."
        ) from exc
    except (OSError, subprocess.SubprocessError) as exc:
        raise TailscaleUnavailable(
            "Tailscale is not available (`tailscale ip -4` could not be run). "
            "Start the Tailscale app, then retry --tailscale."
        ) from exc
    if ip_result.returncode != 0:
        detail = (ip_result.stderr or ip_result.stdout or "").strip()
        message = "Tailscale is not running (`tailscale ip -4` failed)."
        if detail:
            message += f" {detail}"
        message += " Start the Tailscale app, then retry --tailscale."
        raise TailscaleUnavailable(message)
    address = select_tailscale_ipv4(ip_result.stdout or "")
    return address, _magicdns_name(invoke)


def _magicdns_name(invoke: Callable[..., subprocess.CompletedProcess[str]]) -> str | None:
    try:
        status = invoke(["tailscale", "status", "--json"], capture_output=True, text=True, timeout=5, check=False)
    except (OSError, subprocess.SubprocessError):
        return None
    if status.returncode != 0:
        return None
    return magicdns_name_from_status(status.stdout or "")


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
        # Stable for this server process, and not the desktop Assistant transcript.
        self.conversation_id = f"phone-{uuid.uuid4()}"

    def submit(self, envelope: dict) -> tuple[int, dict]:
        status, receipt = super().submit(envelope)
        if status == 202:
            join_desktop = envelope.get("join_desktop") is True
            threading.Thread(
                target=self._host_turn,
                args=(envelope["request_id"], envelope["content"][0]["text"], join_desktop),
                daemon=True,
            ).start()
        return status, receipt

    def _echo(self, request_id: str, text: str) -> None:
        # HostStore starts its own worker; the inherited timer is deliberately not used.
        return

    def _host_turn(self, request_id: str, text: str, join_desktop: bool = False) -> None:
        with self.lock:
            self.receipts[request_id].update(state="running", updated_at=now())
            self._append({"kind": "receipt", "request_id": request_id, "state": "running"})
        turn_id = None
        status = None
        pending = None
        try:
            command = [self.plexi_bin, "assistant", "send", "--text", text, "--request-id", request_id]
            if join_desktop:
                command.append("--desktop")
            else:
                command.extend(["--conversation", self.conversation_id])
            command.append("--json")
            proc = subprocess.run(
                command,
                capture_output=True, text=True, timeout=120, check=False,
            )
            if not proc.stdout.strip():
                raise RuntimeError("no reply from the host Assistant within 120 s (a permission prompt may be waiting on the desktop)")
            payload = json.loads(proc.stdout.strip())
            # The host names the turn it created. This subprocess's stdout is
            # that turn's reply; nothing here is matched by arrival order.
            returned_request = payload.get("request_id")
            if returned_request and returned_request != request_id:
                raise RuntimeError(f"host reply was for request {returned_request}, not {request_id}")
            state = payload.get("state", "failed")
            reply = payload.get("reply")
            turn_id = payload.get("turn_id")
            error = payload.get("error") or proc.stderr.strip()
            if state == "succeeded" and not turn_id:
                raise RuntimeError("host reply missing turn_id")
            if state == "waiting_for_permission":
                status = payload.get("status") or "waiting for approval on desktop"
                pending = payload.get("pending_request_id")
                if pending:
                    status = f"{status} ({pending})"
                error = ""
            elif state != "succeeded" and not error:
                error = f"host turn ended in {state}"
        except subprocess.TimeoutExpired:
            state, reply, turn_id, error, status, pending = (
                "failed", None, None,
                "no reply from the host Assistant within 120 s (a permission prompt may be waiting on the desktop)",
                None, None,
            )
        except (OSError, RuntimeError, json.JSONDecodeError) as exc:
            state, reply, turn_id, error, status, pending = "failed", None, None, str(exc), None, None
        if not self._commit_host_result(request_id, state, reply, turn_id, error, status):
            return
        if state == "waiting_for_permission" and pending:
            self._poll_permission_outcome(request_id, pending)

    def _commit_host_result(self, request_id, state, reply, turn_id, error, status) -> bool:
        with self.lock:
            receipt = self.receipts.get(request_id)
            if receipt is None or receipt["state"] == "cancelled":
                return False
            receipt.update(state=state, updated_at=now())
            if turn_id:
                receipt["turn_id"] = turn_id
            if reply:
                event = {"kind": "assistant_reply", "request_id": request_id, "text": reply}
                if turn_id:
                    event["turn_id"] = turn_id
                self._append(event)
            if error:
                receipt["error"] = error
            elif "error" in receipt and state == "succeeded":
                receipt.pop("error", None)
            if status:
                receipt["status"] = status
            event = {"kind": "receipt", "request_id": request_id, "state": state}
            if turn_id:
                event["turn_id"] = turn_id
            if status:
                event["status"] = status
            elif state != "succeeded" and error:
                event["error"] = error
            self._append(event)
            log.info("turn host request_id=%s turn_id=%s state=%s", request_id, turn_id, state)
            return True

    def _poll_permission_outcome(self, request_id: str, pending_request_id: str) -> None:
        """Ask the host again until the desktop approval has a final status."""
        log.info(
            "phone permission poll request_id=%s pending_request_id=%s",
            request_id, pending_request_id,
        )
        deadline = time.time() + 120
        while time.time() < deadline:
            time.sleep(0.25)
            with self.lock:
                receipt = self.receipts.get(request_id)
                if receipt is None or receipt["state"] != "waiting_for_permission":
                    return
            try:
                proc = subprocess.run(
                    [self.plexi_bin, "assistant", "send", "--status-for", pending_request_id,
                     "--request-id", request_id, "--json"],
                    capture_output=True, text=True, timeout=30, check=False,
                )
                if not proc.stdout.strip():
                    continue
                payload = json.loads(proc.stdout.strip())
            except (OSError, subprocess.TimeoutExpired, json.JSONDecodeError) as exc:
                log.info("phone permission poll failed request_id=%s error=%s", request_id, exc)
                continue
            state = payload.get("state", "failed")
            if state == "waiting_for_permission":
                continue
            reply = payload.get("reply")
            turn_id = payload.get("turn_id") or pending_request_id
            error = payload.get("error") or proc.stderr.strip()
            status = payload.get("status")
            if state != "succeeded" and not error:
                error = f"host turn ended in {state}"
            self._commit_host_result(request_id, state, reply, turn_id, error, status)
            return


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
            # The request line can carry `?token=`. Drop the query before logging.
            try:
                rendered = fmt % args
            except Exception:
                rendered = "unprintable"
            path_only = rendered.split("?", 1)[0]
            log.info("phone request %s", path_only[:180])

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
            # Hash both sides so a length mismatch cannot return early.
            return hmac.compare_digest(
                hashlib.sha256(supplied.encode()).digest(),
                hashlib.sha256(f"Bearer {token}".encode()).digest(),
            )

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
    parser.add_argument("--tailscale", action="store_true", default=os.getenv("PLEXI_PHONE_TAILSCALE") == "1",
                        help="bind to this machine's Tailscale IPv4 (tailscale ip -4)")
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s %(message)s")
    if args.tailscale and args.lan:
        log.error("phone shell: use either --tailscale or --lan, not both")
        sys.exit(2)
    magicdns: str | None = None
    if args.tailscale:
        try:
            host, magicdns = resolve_tailscale_endpoint()
        except TailscaleUnavailable as exc:
            log.error("phone shell Tailscale bind failed: %s", exc)
            sys.exit(1)
        log.info("phone shell binding to Tailscale IPv4 %s", host)
    else:
        host = "0.0.0.0" if args.lan else args.host
    require_token = args.backend == "host" or args.tailscale or host not in ("127.0.0.1", "::1", "localhost")
    token = os.getenv("PLEXI_PHONE_TOKEN") or (secrets.token_urlsafe(32) if require_token else None)
    store = HostStore(args.plexi_bin) if args.backend == "host" else StubStore()
    try:
        server = build_server(host, args.port, store, token)
    except OSError as exc:
        log.error("phone shell failed to bind %s:%d: %s", host, args.port, exc)
        sys.exit(1)
    suffix = f"/?token={token}" if token else "/"
    if args.tailscale:
        urls = [("Tailscale", host)]
        if magicdns:
            urls.append(("MagicDNS", magicdns))
        else:
            log.info("phone shell MagicDNS name unavailable")
    elif host in ("127.0.0.1", "::1", "localhost"):
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
