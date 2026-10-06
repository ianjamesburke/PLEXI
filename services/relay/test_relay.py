"""Relay contract tests. Run: python3 -m unittest services/relay/test_relay.py"""

from __future__ import annotations

import base64
import hashlib
import json
import logging
import os
import queue
import socket
import sqlite3
import struct
import sys
import threading
import time
import unittest
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import phone_crypto  # noqa: E402
import relay  # noqa: E402

CANARY = "CANARY-plexi-relay-9f3a2c1b"
STATIC = Path(__file__).resolve().parents[2] / "clients" / "phone-web" / "static"


def sealed_envelope(request_id: str, text: str, **extra: object) -> tuple[dict, str]:
    sealed = phone_crypto.opaque_body(text)
    body = {
        "schema_version": 1,
        "request_id": request_id,
        "content": [{"type": "sealed", "body": sealed}],
    }
    body.update(extra)
    return body, sealed
WS_GUID = relay.WS_GUID


class Clock:
    def __init__(self) -> None:
        self.t = 1_700_000_000.0

    def __call__(self) -> float:
        return self.t

    def advance(self, seconds: float) -> None:
        self.t += seconds


class LogCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.INFO)
        self.lines: list[str] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.lines.append(record.getMessage())

    @property
    def text(self) -> str:
        return "\n".join(self.lines)


def _http(
    method: str,
    url: str,
    body: dict | None = None,
    cookie: str | None = None,
    extra_headers: dict | None = None,
    raw_body: bytes | None = None,
    return_header: bool = False,
) -> tuple[int, dict, str | None]:
    if raw_body is not None:
        data: bytes | None = raw_body
        headers = {"Content-Type": "application/octet-stream"}
    elif body is not None:
        data = json.dumps(body).encode()
        headers = {"Content-Type": "application/json"}
    else:
        data = None
        headers = {}
    if extra_headers:
        headers.update(extra_headers)
    if cookie:
        headers["Cookie"] = f"{relay.COOKIE}={cookie}"
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=5) as response:
            raw = response.read().decode()
            set_cookie = response.headers.get("Set-Cookie")
            token = set_cookie if return_header else _cookie(set_cookie)
            return response.status, json.loads(raw or "{}"), token
    except urllib.error.HTTPError as exc:
        raw = exc.read().decode()
        set_cookie = exc.headers.get("Set-Cookie") if exc.headers else None
        token = set_cookie if return_header else _cookie(set_cookie)
        return exc.code, json.loads(raw or "{}"), token


def _cookie(header: str | None) -> str | None:
    if not header:
        return None
    for part in header.split(";"):
        name, _, value = part.strip().partition("=")
        if name == relay.COOKIE and value:
            return value
    return None


def _ws_send(sock: socket.socket, payload: bytes, opcode: int = 0x1) -> None:
    mask = os.urandom(4)
    masked = bytes(byte ^ mask[i % 4] for i, byte in enumerate(payload))
    length = len(payload)
    if length < 126:
        header = bytes([0x80 | opcode, 0x80 | length])
    elif length < 65536:
        header = bytes([0x80 | opcode, 0x80 | 126]) + struct.pack("!H", length)
    else:
        header = bytes([0x80 | opcode, 0x80 | 127]) + struct.pack("!Q", length)
    sock.sendall(header + mask + masked)


def _ws_recv(sock: socket.socket) -> tuple[int, bytes]:
    head = _exact(sock, 2)
    opcode = head[0] & 0x0F
    length = head[1] & 0x7F
    if length == 126:
        length = struct.unpack("!H", _exact(sock, 2))[0]
    payload = _exact(sock, length) if length else b""
    return opcode, payload


def _exact(sock: socket.socket, n: int) -> bytes:
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("socket closed")
        buf += chunk
    return buf


class Desktop:
    def __init__(
        self,
        port: int,
        host_id: str = "host-1",
        token: str = "token-1",
        protocol: int | None = 1,
        devices: list | None = None,
        expect_ok: bool = True,
    ) -> None:
        self.host_id = host_id
        self.token = token
        sock = socket.create_connection(("127.0.0.1", port), timeout=5)
        key = base64.b64encode(os.urandom(16)).decode()
        request = (
            "GET /v1/desktop HTTP/1.1\r\n"
            f"Host: 127.0.0.1:{port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        )
        sock.sendall(request.encode())
        raw = b""
        while b"\r\n\r\n" not in raw:
            raw += sock.recv(4096)
        status = raw.split(b"\r\n", 1)[0].decode()
        if "101" not in status:
            raise AssertionError(status)
        accept = base64.b64encode(hashlib.sha1((key + WS_GUID).encode()).digest()).decode()
        if accept.encode() not in raw:
            raise AssertionError("missing websocket accept")
        self.sock = sock
        hello_message: dict = {"type": "hello", "host_id": host_id, "host_token": token, "host_label": "desk"}
        if protocol is not None:
            hello_message["protocol"] = protocol
        if devices is not None:
            hello_message["devices"] = devices
        self.send(hello_message)
        hello = self.recv()
        self.hello = hello
        if not expect_ok:
            self.devices = []
            return
        if hello.get("type") != "hello_ok" or hello.get("protocol") != relay.PROTOCOL_VERSION:
            raise AssertionError(hello)
        if not isinstance(hello.get("devices"), list):
            raise AssertionError(hello)
        self.devices = hello["devices"]

    def send(self, message: dict) -> None:
        _ws_send(self.sock, json.dumps(message).encode())

    def recv(self) -> dict:
        opcode, payload = _ws_recv(self.sock)
        if opcode != 0x1:
            raise AssertionError(opcode)
        body = json.loads(payload.decode())
        if not isinstance(body, dict):
            raise AssertionError(body)
        return body

    def close(self) -> None:
        try:
            _ws_send(self.sock, b"", opcode=0x8)
        except OSError:
            pass
        self.sock.close()


class RelayHttpTest(unittest.TestCase):
    def setUp(self) -> None:
        relay.install_log_guard()
        self.logs = LogCapture()
        relay.log.addHandler(self.logs)
        relay.log.setLevel(logging.INFO)
        self.relay = relay.Relay(public_origin="http://127.0.0.1")
        self.server = relay.build_server("127.0.0.1", 0, self.relay, STATIC)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.port = self.server.server_address[1]
        self.base = f"http://127.0.0.1:{self.port}"

    def tearDown(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        relay.log.removeHandler(self.logs)

    def test_bind_keeps_the_literal_address(self) -> None:
        self.assertEqual(self.server.server_name, "127.0.0.1")
        self.assertEqual(self.server.server_port, self.port)

    def test_pairing_confirm_and_revoke(self) -> None:
        desk = Desktop(self.port)
        self.addCleanup(desk.close)
        desk.send({"type": "pair_start"})
        issued = desk.recv()
        self.assertEqual(issued["type"], "pair_code")
        code = issued["code"]
        self.assertEqual(len(code), 8)
        self.assertNotIn(code, self.logs.text)
        self.assertTrue(issued["qr_url"].endswith("/"))
        self.assertNotIn(code, issued["qr_url"])

        status, missing, _ = _http("POST", f"{self.base}/api/pair", {"code": "ZZZZZZZZ", "label": "nope"})
        self.assertEqual(status, 404)
        self.assertIn("protocol", missing.get("message", ""))
        self.assertNotIn("ZZZZZZZZ", self.logs.text)

        status, pending, token = _http("POST", f"{self.base}/api/pair", {"code": code, "label": "pixel"})
        self.assertEqual(status, 202, pending)
        self.assertEqual(pending["status"], "pending_desktop")
        self.assertIsNone(token)
        self.assertEqual(len(self.relay.devices), 0)
        notice = desk.recv()
        self.assertEqual(notice["type"], "pair_pending")
        self.assertEqual(notice["fingerprint"], pending["fingerprint"])

        status, turns, _ = _http(
            "POST",
            f"{self.base}/api/turns",
            sealed_envelope("req-early", "hi")[0],
        )
        self.assertEqual(status, 401)

        desk.send({"type": "pair_confirm", "pairing_id": issued["pairing_id"]})
        confirmed = desk.recv()
        self.assertEqual(confirmed["type"], "pair_confirmed")
        desk.send({"type": "pair_confirm", "pairing_id": issued["pairing_id"]})
        again = desk.recv()
        self.assertEqual(again["device_id"], confirmed["device_id"])
        self.assertEqual(len(self.relay.devices), 1)

        status, polled, cookie = _http("GET", f"{self.base}/api/pair/{issued['pairing_id']}")
        self.assertEqual(polled["status"], "confirmed")
        self.assertIsNotNone(cookie)

        status, denied_turns, _ = _http(
            "POST",
            f"{self.base}/api/turns",
            sealed_envelope("req-ok", "hello")[0],
            cookie=cookie,
        )
        self.assertEqual(status, 202, denied_turns)

        desk.send({"type": "revoke", "device_id": confirmed["device_id"]})
        revoked = desk.recv()
        self.assertEqual(revoked["type"], "revoked")
        status, after, _ = _http("GET", f"{self.base}/api/status", cookie=cookie)
        self.assertEqual(status, 401, after)
        self.assertEqual(len(self.relay.devices), 1)
        self.assertTrue(self.relay.devices[confirmed["device_id"]].revoked)

    def test_reconnect_lists_the_same_device_and_keeps_the_cookie(self) -> None:
        desk = Desktop(self.port)
        cookie = self._pair(desk)
        device_id = self.relay.device_summaries("host-1")[0]["device_id"]
        desk.close()
        again = Desktop(self.port)
        self.addCleanup(again.close)
        self.assertEqual([item["device_id"] for item in again.devices], [device_id])
        status, queued, _ = _http(
            "POST",
            f"{self.base}/api/turns",
            sealed_envelope("req-resume", "still paired")[0],
            cookie=cookie,
        )
        self.assertEqual(status, 202, queued)

    def test_join_desktop_is_opt_in_on_the_deliver_frame(self) -> None:
        desk = Desktop(self.port)
        self.addCleanup(desk.close)
        cookie = self._pair(desk)
        status, queued, _ = _http(
            "POST",
            f"{self.base}/api/turns",
            sealed_envelope("req-desk", "continue on the desktop", join_desktop=True)[0],
            cookie=cookie,
        )
        self.assertEqual(status, 202, queued)
        delivered = desk.recv()
        self.assertIs(delivered["join_desktop"], True)
        self.assertTrue(delivered["conversation_id"].startswith("phone-"))

    def test_two_devices_and_revoking_one_leaves_the_other(self) -> None:
        desk = Desktop(self.port)
        self.addCleanup(desk.close)
        first = self._pair(desk)
        desk.send({"type": "pair_start"})
        issued = desk.recv()
        status, pending, _ = _http("POST", f"{self.base}/api/pair", {"code": issued["code"], "label": "tablet"})
        self.assertEqual(status, 202, pending)
        desk.recv()  # pair_pending
        desk.send({"type": "pair_confirm", "pairing_id": issued["pairing_id"]})
        confirmed = desk.recv()
        _, _, second = _http("GET", f"{self.base}/api/pair/{issued['pairing_id']}")
        self.assertIsNotNone(second)
        self.assertEqual(len(self.relay.device_summaries("host-1")), 2)
        desk.send({"type": "revoke", "device_id": confirmed["device_id"]})
        desk.recv()
        status, rejected, _ = _http(
            "POST",
            f"{self.base}/api/turns",
            sealed_envelope("req-revoked", "nope")[0],
            cookie=second,
        )
        self.assertEqual(status, 401, rejected)
        status, kept, _ = _http(
            "POST",
            f"{self.base}/api/turns",
            sealed_envelope("req-kept", "still here")[0],
            cookie=first,
        )
        self.assertEqual(status, 202, kept)
        self.assertEqual(len(self.relay.device_summaries("host-1")), 1)

    def test_message_round_trip_and_desktop_offline(self) -> None:
        desk = Desktop(self.port)
        self.addCleanup(desk.close)
        cookie = self._pair(desk)
        status, state, _ = _http("GET", f"{self.base}/api/status", cookie=cookie)
        self.assertEqual(state["host"], "online")

        text = "round trip hello"
        payload, sealed = sealed_envelope("req-1", text, conversation_id="browser-supplied")
        status, queued, _ = _http(
            "POST",
            f"{self.base}/api/turns",
            payload,
            cookie=cookie,
        )
        self.assertEqual(status, 202, queued)
        delivered = desk.recv()
        self.assertEqual(delivered["type"], "deliver")
        self.assertEqual(delivered["text"], sealed)
        self.assertNotIn(text, delivered["text"])
        self.assertTrue(delivered["conversation_id"].startswith("phone-"))
        self.assertNotEqual(delivered["conversation_id"], "browser-supplied")
        self.assertIs(delivered["join_desktop"], False)
        desk.send({"type": "ack", "delivery_id": delivered["delivery_id"]})
        desk.send(
            {
                "type": "reply",
                "delivery_id": delivered["delivery_id"],
                "request_id": delivered["request_id"],
                "state": "succeeded",
                "reply": "desk says hi",
                "turn_id": "turn-1",
            }
        )
        self.assertTrue(self._wait(lambda: self.relay.deliveries[delivered["delivery_id"]].state == "succeeded"))
        self.assertIsNone(self.relay.deliveries[delivered["delivery_id"]].body)
        status, page, _ = _http("GET", f"{self.base}/api/conversation?after=0", cookie=cookie)
        kinds = [event["kind"] for event in page["events"]]
        self.assertIn("assistant_reply", kinds)
        reply = next(event for event in page["events"] if event["kind"] == "assistant_reply")
        self.assertEqual(reply["text"], "desk says hi")
        self.assertNotIn(text, self.logs.text)
        self.assertNotIn("desk says hi", self.logs.text)

        desk.close()
        self.assertTrue(self._wait(lambda: not self.relay.desktop_online("host-1")))
        status, offline, _ = _http("GET", f"{self.base}/api/status", cookie=cookie)
        self.assertEqual(offline["host"], "desktop_offline")
        self.assertEqual(offline["message"], "desktop offline")
        status, refused, _ = _http(
            "POST",
            f"{self.base}/api/turns",
            sealed_envelope("req-off", CANARY)[0],
            cookie=cookie,
        )
        self.assertEqual(status, 409, refused)
        self.assertEqual(refused["error"], "desktop_offline")
        self.assertNotIn(CANARY, self.relay.memory_text())
        self.assertNotIn(CANARY, self.logs.text)

    def test_canary_never_appears_in_logs(self) -> None:
        desk = Desktop(self.port)
        self.addCleanup(desk.close)
        cookie = self._pair(desk)
        payload, sealed = sealed_envelope("req-canary", CANARY)
        self.assertNotIn(CANARY, sealed)
        status, queued, _ = _http(
            "POST",
            f"{self.base}/api/turns",
            payload,
            cookie=cookie,
        )
        self.assertEqual(status, 202, queued)
        delivered = desk.recv()
        self.assertEqual(delivered["text"], sealed)
        self.assertNotIn(CANARY, json.dumps(delivered))
        desk.send({"type": "ack", "delivery_id": delivered["delivery_id"]})
        desk.send(
            {
                "type": "reply",
                "delivery_id": delivered["delivery_id"],
                "request_id": "req-canary",
                "state": "succeeded",
                "reply": sealed,
            }
        )
        self.assertTrue(self._wait(lambda: self.relay.deliveries[delivered["delivery_id"]].body is None))
        status, page, _ = _http("GET", f"{self.base}/api/conversation?after=0", cookie=cookie)
        self.assertTrue(any(event.get("text") == sealed for event in page["events"]))
        self.assertNotIn(CANARY, json.dumps(page))
        self.assertNotIn(CANARY, self.logs.text)
        self.assertNotIn(CANARY, self.relay.memory_text())
        self.assertIsNone(self.relay.deliveries[delivered["delivery_id"]].body)

    def test_phone_needs_you_refuses_an_irreversible_click(self) -> None:
        desk = Desktop(self.port)
        self.addCleanup(desk.close)
        cookie = self._pair(desk)

        listed: dict = {}

        def fetch_list() -> None:
            status, body, _ = _http("GET", f"{self.base}/api/needs-you", cookie=cookie)
            listed["status"] = status
            listed["body"] = body

        thread = threading.Thread(target=fetch_list)
        thread.start()
        asked = desk.recv()
        self.assertEqual(asked["type"], "needs_you_list")
        desk.send(
            {
                "type": "needs_you_result",
                "request_id": asked["request_id"],
                "ok": True,
                "items": [
                    {
                        "id": "click-1",
                        "kind": "approval_click",
                        "summary": "write a file",
                        "phone_can_approve": False,
                    },
                    {
                        "id": "q-1",
                        "kind": "question",
                        "summary": "which file",
                        "phone_can_approve": True,
                    },
                ],
            }
        )
        thread.join(timeout=5)
        self.assertEqual(listed["status"], 200)
        self.assertEqual(listed["body"]["items"][0]["phone_can_approve"], False)
        self.assertEqual(listed["body"]["items"][1]["kind"], "question")

        refused: dict = {}

        def approve_click() -> None:
            status, body, _ = _http(
                "POST",
                f"{self.base}/api/needs-you/click-1/resolve",
                {"decision": "approve"},
                cookie=cookie,
            )
            refused["status"] = status
            refused["body"] = body

        thread = threading.Thread(target=approve_click)
        thread.start()
        asked = desk.recv()
        self.assertEqual(asked["type"], "needs_you_resolve")
        self.assertEqual(asked["id"], "click-1")
        self.assertTrue(asked["approve"])
        desk.send(
            {
                "type": "needs_you_result",
                "request_id": asked["request_id"],
                "ok": False,
                "error": "waiting on desktop",
            }
        )
        thread.join(timeout=5)
        self.assertEqual(refused["status"], 403)
        self.assertEqual(refused["body"]["message"], "waiting on desktop")

        allowed: dict = {}

        def approve_question() -> None:
            status, body, _ = _http(
                "POST",
                f"{self.base}/api/needs-you/q-1/resolve",
                {"decision": "approve"},
                cookie=cookie,
            )
            allowed["status"] = status
            allowed["body"] = body

        thread = threading.Thread(target=approve_question)
        thread.start()
        asked = desk.recv()
        desk.send(
            {
                "type": "needs_you_result",
                "request_id": asked["request_id"],
                "ok": True,
                "id": "q-1",
                "resolution": "approved",
                "already": False,
            }
        )
        thread.join(timeout=5)
        self.assertEqual(allowed["status"], 200)
        self.assertEqual(allowed["body"]["resolution"], "approved")

        desk.close()
        self.assertTrue(self._wait(lambda: "host-1" not in self.relay.links))
        status, body, _ = _http("GET", f"{self.base}/api/needs-you", cookie=cookie)
        self.assertEqual(status, 503, body)

    def test_approval_is_waiting_on_desktop(self) -> None:
        desk = Desktop(self.port)
        self.addCleanup(desk.close)
        cookie = self._pair(desk)
        status, body, _ = _http("POST", f"{self.base}/api/approvals", {"request_id": "req-a"}, cookie=cookie)
        self.assertEqual(status, 403, body)
        self.assertEqual(body["message"], "waiting on desktop")
        status, body, _ = _http("POST", f"{self.base}/api/turns/req-a/approve", cookie=cookie)
        self.assertEqual(status, 403, body)
        self.assertEqual(body["message"], "waiting on desktop")

    def test_phone_page_is_served(self) -> None:
        with urllib.request.urlopen(f"{self.base}/app.js", timeout=5) as response:
            body = response.read()
        self.assertIn(b"composer", body)
        with urllib.request.urlopen(f"{self.base}/", timeout=5) as response:
            page = response.read()
        self.assertIn(b"Pairing code", page)

    def _wait(self, ready) -> bool:
        for _ in range(50):
            if ready():
                return True
            time.sleep(0.02)
        return False

    def _pair(self, desk: Desktop) -> str:
        desk.send({"type": "pair_start"})
        issued = desk.recv()
        status, pending, _ = _http("POST", f"{self.base}/api/pair", {"code": issued["code"], "label": "pixel"})
        self.assertEqual(status, 202, pending)
        self.assertEqual(desk.recv()["type"], "pair_pending")
        desk.send({"type": "pair_confirm", "pairing_id": issued["pairing_id"]})
        self.assertEqual(desk.recv()["type"], "pair_confirmed")
        status, polled, cookie = _http("GET", f"{self.base}/api/pair/{issued['pairing_id']}")
        self.assertEqual(polled["status"], "confirmed")
        self.assertIsNotNone(cookie)
        assert cookie is not None
        return cookie


    def test_cross_site_post_is_rejected(self) -> None:
        status, body, _ = _http(
            "POST",
            f"{self.base}/api/pair",
            {"code": "ZZZZZZZZ", "label": "pixel"},
            extra_headers={"Origin": "https://evil.example", "Sec-Fetch-Site": "cross-site"},
        )
        self.assertEqual(status, 403, body)
        status, missing, _ = _http(
            "POST",
            f"{self.base}/api/pair",
            {"code": "ZZZZZZZZ", "label": "pixel"},
            extra_headers={"Origin": f"http://127.0.0.1:{self.port}"},
        )
        self.assertEqual(status, 404, missing)

    def test_negative_and_huge_bodies_are_rejected(self) -> None:
        sock = socket.create_connection(("127.0.0.1", self.port), timeout=2)
        self.addCleanup(sock.close)
        sock.sendall(
            b"POST /api/pair HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: -1\r\nConnection: close\r\n\r\n"
        )
        raw = b""
        while True:
            chunk = sock.recv(4096)
            if not chunk:
                break
            raw += chunk
        self.assertIn(b"400", raw.split(b"\r\n", 1)[0])
        huge = b"{" + b"x" * (relay.MAX_BODY_BYTES + 8)
        status, body, _ = _http("POST", f"{self.base}/api/pair", None, raw_body=huge)
        self.assertEqual(status, 413, body)

    def test_secure_cookie_and_one_shot_session(self) -> None:
        box = relay.Relay(public_origin="http://127.0.0.1")
        server = relay.build_server("127.0.0.1", 0, box, STATIC, secure_cookie=True)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(server.shutdown)
        self.addCleanup(server.server_close)
        port = server.server_address[1]
        base = f"http://127.0.0.1:{port}"
        desk = Desktop(port)
        self.addCleanup(desk.close)
        desk.send({"type": "pair_start"})
        issued = desk.recv()
        status, pending, _ = _http("POST", f"{base}/api/pair", {"code": issued["code"], "label": "pixel"})
        self.assertEqual(status, 202, pending)
        self.assertEqual(desk.recv()["type"], "pair_pending")
        desk.send({"type": "pair_confirm", "pairing_id": issued["pairing_id"]})
        self.assertEqual(desk.recv()["type"], "pair_confirmed")
        status, polled, header = _http("GET", f"{base}/api/pair/{issued['pairing_id']}", return_header=True)
        self.assertEqual(polled["status"], "confirmed")
        self.assertIn("Secure", header)
        self.assertIn("HttpOnly", header)
        self.assertIn("SameSite=Lax", header)
        cookie = _cookie(header)
        self.assertIsNotNone(cookie)
        _status, _again, header2 = _http("GET", f"{base}/api/pair/{issued['pairing_id']}", return_header=True)
        self.assertEqual(_cookie(header2), cookie)
        status, _state, _ = _http("GET", f"{base}/api/status", cookie=cookie)
        self.assertEqual(status, 200)
        _status, still, header3 = _http("GET", f"{base}/api/pair/{issued['pairing_id']}", return_header=True)
        self.assertEqual(still["status"], "confirmed")
        self.assertIsNone(_cookie(header3))
        status, _state, _ = _http("GET", f"{base}/api/status", cookie=cookie)
        self.assertEqual(status, 200)
        desk.send({"type": "revoke", "device_id": polled["device_id"]})
        self.assertEqual(desk.recv()["type"], "revoked")
        status, denied, _ = _http("GET", f"{base}/api/status", cookie=cookie)
        self.assertEqual(status, 401, denied)
        status, _page, _ = _http("GET", f"{base}/api/conversation?after=0", cookie=cookie)
        self.assertEqual(status, 401)


class TtlTest(unittest.TestCase):
    def test_undelivered_body_is_purged_after_ttl(self) -> None:
        clock = Clock()
        logs = LogCapture()
        relay.log.addHandler(logs)
        self.addCleanup(relay.log.removeHandler, logs)
        box = relay.Relay(clock=clock, undelivered_ttl=120, public_origin="http://127.0.0.1")
        link = box.hello("host-ttl", "token-ttl", "desk")
        self.assertIsNotNone(link)
        issued = box.start_pairing("host-ttl")
        assert issued is not None
        status, pending = box.redeem(issued["code"], "pixel")
        self.assertEqual(status, 202, pending)
        confirmed = box.confirm("host-ttl", issued["pairing_id"])
        self.assertEqual(confirmed["type"], "pair_confirmed")
        _status, polled, token = box.pairing_status(issued["pairing_id"])
        assert token is not None
        device = box.device_for_token(token)
        assert device is not None
        payload, sealed = sealed_envelope("req-ttl", CANARY)
        self.assertNotIn(CANARY, sealed)
        status, queued = box.submit(device, payload)
        self.assertEqual(status, 202, queued)
        self.assertIn(sealed, box.memory_text())
        self.assertNotIn(CANARY, box.memory_text())
        clock.advance(119)
        box.purge()
        self.assertIn(sealed, box.memory_text())
        clock.advance(2)
        box.purge()
        self.assertNotIn(sealed, box.memory_text())
        self.assertNotIn(CANARY, box.memory_text())
        self.assertIsNone(box.deliveries[queued["delivery_id"]].body)
        self.assertEqual(box.deliveries[queued["delivery_id"]].state, "expired")
        self.assertNotIn(CANARY, logs.text)

    def test_ack_drops_body_immediately(self) -> None:
        clock = Clock()
        box = relay.Relay(clock=clock, public_origin="http://127.0.0.1")
        box.hello("host-ack", "token-ack", "desk")
        issued = box.start_pairing("host-ack")
        assert issued is not None
        box.redeem(issued["code"], "pixel")
        box.confirm("host-ack", issued["pairing_id"])
        _status, _body, token = box.pairing_status(issued["pairing_id"])
        assert token is not None
        device = box.device_for_token(token)
        assert device is not None
        status, queued = box.submit(
            device,
            sealed_envelope("req-ack", CANARY)[0],
        )
        self.assertEqual(status, 202)
        box.ack("host-ack", queued["delivery_id"])
        self.assertIsNone(box.deliveries[queued["delivery_id"]].body)
        # The phone has not polled, so its one-shot projection may still hold
        # the text. The delivery record itself no longer does.
        self.assertTrue(all(item.body is None for item in box.deliveries.values()))


class RegistryTest(unittest.TestCase):
    def test_pairing_survives_a_new_process_and_skips_bodies(self) -> None:
        clock = Clock()
        path = str(Path(os.environ.get("TMPDIR", "/tmp")) / f"relay-registry-{os.getpid()}.sqlite")
        self.addCleanup(lambda: _unlink_sqlite(path))
        first = relay.Relay(clock=clock, public_origin="http://127.0.0.1", state_path=path)
        link = first.hello("host-keep", "host-secret", "desk")
        self.assertIsNotNone(link)
        issued = first.start_pairing("host-keep")
        assert issued is not None
        status, pending = first.redeem(issued["code"], "pixel")
        self.assertEqual(status, 202, pending)
        confirmed = first.confirm("host-keep", issued["pairing_id"])
        self.assertEqual(confirmed["type"], "pair_confirmed")
        _status, _polled, token = first.pairing_status(issued["pairing_id"])
        assert token is not None
        device = first.device_for_token(token)
        assert device is not None
        payload, sealed = sealed_envelope("req-keep", CANARY)
        self.assertNotIn(CANARY, sealed)
        status, queued = first.submit(device, payload)
        self.assertEqual(status, 202, queued)
        stored = Path(path).read_bytes()
        self.assertNotIn(CANARY.encode(), stored)
        self.assertNotIn(sealed.encode(), stored)
        self.assertNotIn(token.encode(), stored)
        self.assertNotIn(b"host-secret", stored)
        second = relay.Relay(clock=clock, public_origin="http://127.0.0.1", state_path=path)
        self.assertEqual(len(second.device_summaries("host-keep")), 1)
        resumed = second.device_for_token(token)
        self.assertIsNotNone(resumed)
        assert resumed is not None
        self.assertEqual(resumed.device_id, device.device_id)
        self.assertEqual(resumed.fingerprint, device.fingerprint)
        self.assertNotIn(CANARY, second.memory_text())
        self.assertIsNone(second.hello("host-keep", "someone-else", "desk"))
        self.assertIsNotNone(second.hello("host-keep", "host-secret", "desk"))

    def test_idle_device_expires_after_thirty_days(self) -> None:
        clock = Clock()
        path = str(Path(os.environ.get("TMPDIR", "/tmp")) / f"relay-expire-{os.getpid()}.sqlite")
        self.addCleanup(lambda: _unlink_sqlite(path))
        box = relay.Relay(clock=clock, public_origin="http://127.0.0.1", state_path=path)
        box.hello("host-idle", "token-idle", "desk")
        issued = box.start_pairing("host-idle")
        assert issued is not None
        box.redeem(issued["code"], "pixel")
        box.confirm("host-idle", issued["pairing_id"])
        _status, _body, token = box.pairing_status(issued["pairing_id"])
        assert token is not None
        clock.advance(relay.DEVICE_IDLE_SECONDS)
        self.assertEqual(len(box.device_summaries("host-idle")), 1)
        clock.advance(1)
        box.reap()
        self.assertEqual(box.device_summaries("host-idle"), [])
        self.assertIsNone(box.device_for_token(token))
        reloaded = relay.Relay(clock=clock, public_origin="http://127.0.0.1", state_path=path)
        self.assertEqual(reloaded.device_summaries("host-idle"), [])
        self.assertIsNone(reloaded.device_for_token(token))

    def test_retention_drops_rows_older_than_thirty_days(self) -> None:
        clock = Clock()
        path = str(Path(os.environ.get("TMPDIR", "/tmp")) / f"relay-retain-{os.getpid()}.sqlite")
        self.addCleanup(lambda: _unlink_sqlite(path))
        box = relay.Relay(clock=clock, public_origin="http://127.0.0.1", state_path=path)
        registry = box.registry
        assert registry is not None
        now = clock()
        old = now - relay.DEVICE_IDLE_SECONDS - 10
        fresh = now - 3600
        edge = now - relay.DEVICE_IDLE_SECONDS
        registry.upsert_host("host-old", "hash-old", created=old)
        registry.upsert_host("host-new", "hash-new", created=fresh)
        registry.upsert_host("host-edge", "hash-edge", created=edge)
        registry.conn.execute(
            "INSERT INTO hosts (host_id, token_hash, created) VALUES ('host-legacy', 'hash-legacy', NULL)"
        )
        registry.conn.commit()
        registry.upsert_device(
            relay.Device(
                device_id="dev-old",
                host_id="host-new",
                label="phone",
                fingerprint="fp-old",
                created=old,
                last_seen=old,
                token_hash="th-old",
            )
        )
        registry.upsert_device(
            relay.Device(
                device_id="dev-new",
                host_id="host-new",
                label="phone",
                fingerprint="fp-new",
                created=fresh,
                last_seen=fresh,
                token_hash="th-new",
            )
        )
        registry.upsert_device(
            relay.Device(
                device_id="dev-edge",
                host_id="host-new",
                label="phone",
                fingerprint="fp-edge",
                created=edge,
                last_seen=edge,
                token_hash="th-edge",
            )
        )
        registry.note_envelope("req-old", "host-new", old, len(CANARY))
        registry.note_envelope("req-new", "host-new", fresh, 4)
        registry.note_envelope("req-edge", "host-new", edge, 4)
        columns = {row[1] for row in registry.conn.execute("PRAGMA table_info(envelopes)")}
        self.assertEqual(columns, {"request_id", "host_id", "queued_at", "size"})
        logs = LogCapture()
        relay.install_log_guard()
        relay.log.addHandler(logs)
        self.addCleanup(relay.log.removeHandler, logs)
        report = registry.retain(now)
        self.assertIn("host-old", report["hosts"])
        self.assertIn("dev-old", report["devices"])
        self.assertIn("req-old", report["envelopes"])
        self.assertNotIn("host-new", report["hosts"])
        self.assertNotIn("host-edge", report["hosts"])
        self.assertNotIn("host-legacy", report["hosts"])
        hosts = {row[0] for row in registry.conn.execute("SELECT host_id FROM hosts")}
        devices = {row[0] for row in registry.conn.execute("SELECT device_id FROM devices")}
        envelopes = {row[0] for row in registry.conn.execute("SELECT request_id FROM envelopes")}
        self.assertEqual(hosts, {"host-new", "host-edge", "host-legacy"})
        self.assertEqual(devices, {"dev-new", "dev-edge"})
        self.assertEqual(envelopes, {"req-new", "req-edge"})
        stored = Path(path).read_bytes()
        self.assertNotIn(CANARY.encode(), stored)
        self.assertNotIn(CANARY, logs.text)
        self.assertTrue(any(line.startswith("event=retention") for line in logs.lines))
        reloaded = relay.Relay(clock=clock, public_origin="http://127.0.0.1", state_path=path)
        self.assertIn("host-new", reloaded.hosts)
        self.assertNotIn("host-old", reloaded.hosts)
        self.assertEqual(
            {device.device_id for device in reloaded.devices.values()},
            {"dev-new", "dev-edge"},
        )

    def test_unset_state_path_stays_in_memory(self) -> None:
        box = relay.Relay(public_origin="http://127.0.0.1")
        self.assertIsNone(box.registry)
        box.hello("host-mem", "token-mem", "desk")
        self.assertIn("host-mem", box.hosts)


class ProtocolTest(unittest.TestCase):
    def setUp(self) -> None:
        self.relay = relay.Relay(public_origin="http://127.0.0.1")
        self.server = relay.build_server("127.0.0.1", 0, self.relay, STATIC)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.port = self.server.server_address[1]

    def tearDown(self) -> None:
        self.server.shutdown()
        self.server.server_close()

    def test_missing_or_wrong_protocol_does_not_register_the_host(self) -> None:
        missing = Desktop(self.port, host_id="host-old", protocol=None, expect_ok=False)
        self.addCleanup(missing.close)
        self.assertEqual(missing.hello.get("error"), "protocol_mismatch")
        self.assertIn("protocol", missing.hello.get("message", ""))
        self.assertNotIn("host-old", self.relay.hosts)
        wrong = Desktop(self.port, host_id="host-new", protocol=2, expect_ok=False)
        self.addCleanup(wrong.close)
        self.assertEqual(wrong.hello.get("error"), "protocol_mismatch")
        self.assertNotIn("host-new", self.relay.hosts)

    def test_unwritable_state_path_does_not_fall_back_to_memory(self) -> None:
        with self.assertRaises((OSError, sqlite3.Error)):
            relay.Relay(state_path="/proc/does-not-exist/relay.sqlite")


class SecurityTest(unittest.TestCase):
    """Cheap checks for the relay threat model. See docs/security/relay-threat-model.md."""

    def test_pairing_code_entropy(self) -> None:
        alphabet = relay.CODE_ALPHABET
        self.assertEqual(len(alphabet), len(set(alphabet)))
        self.assertGreaterEqual(len(alphabet), 32)
        self.assertGreaterEqual(relay.CODE_LENGTH, 8)
        self.assertGreaterEqual(len(alphabet) ** relay.CODE_LENGTH, 2**40)
        codes = {relay._code() for _ in range(30)}
        self.assertGreater(len(codes), 1)
        for code in codes:
            self.assertEqual(len(code), relay.CODE_LENGTH)
            self.assertTrue(set(code) <= set(alphabet))

    def test_tokens_are_hashed_and_compared_in_constant_time(self) -> None:
        self.assertTrue(relay.secret_equal("same-token", "same-token"))
        self.assertFalse(relay.secret_equal("same-token", "same-token-2"))
        self.assertFalse(relay.secret_equal("short", "a-much-longer-value"))
        box = relay.Relay(public_origin="http://127.0.0.1")
        self.assertIsNotNone(box.hello("host-hash", "desktop-token-aaa", "desk"))
        stored = box.hosts["host-hash"]
        self.assertNotEqual(stored, "desktop-token-aaa")
        self.assertEqual(len(stored), 64)
        self.assertTrue(relay.secret_equal(stored, relay._hash("desktop-token-aaa")))
        self.assertIsNone(box.hello("host-hash", "desktop-token-bbb", "desk"))
        self.assertIsNotNone(box.hello("host-hash", "desktop-token-aaa", "desk"))

    def test_cookie_secure_flag_follows_the_bind(self) -> None:
        previous = os.environ.pop("RELAY_COOKIE_SECURE", None)
        try:
            self.assertFalse(relay.cookies_should_be_secure("127.0.0.1", "http://127.0.0.1:8790"))
            self.assertFalse(relay.cookies_should_be_secure("localhost", "http://localhost:8790"))
            self.assertTrue(relay.cookies_should_be_secure("0.0.0.0", "http://0.0.0.0:8080"))
            self.assertTrue(relay.cookies_should_be_secure("127.0.0.1", "https://plexi-relay.example"))
            os.environ["RELAY_COOKIE_SECURE"] = "0"
            self.assertFalse(relay.cookies_should_be_secure("0.0.0.0", "https://plexi-relay.example"))
            os.environ["RELAY_COOKIE_SECURE"] = "1"
            self.assertTrue(relay.cookies_should_be_secure("127.0.0.1", "http://127.0.0.1:8790"))
        finally:
            os.environ.pop("RELAY_COOKIE_SECURE", None)
            if previous is not None:
                os.environ["RELAY_COOKIE_SECURE"] = previous
        header = relay.cookie_header("abc", secure=True)
        self.assertIn("HttpOnly", header)
        self.assertIn("SameSite=Lax", header)
        self.assertIn("Secure", header)
        self.assertNotIn("Secure", relay.cookie_header("abc", secure=False))

    def test_failed_pairing_attempts_are_rate_limited(self) -> None:
        clock = Clock()
        box = relay.Relay(
            clock=clock,
            public_origin="http://127.0.0.1",
            pair_fail_limit=2,
            pair_fail_global=5,
        )
        self.assertIsNotNone(box.hello("host-rate", "token-rate", "desk"))
        issued = box.start_pairing("host-rate")
        assert issued is not None
        for _ in range(2):
            status, body = box.redeem("ZZZZZZZZ", "pixel", client="203.0.113.5")
            self.assertEqual(status, 404, body)
        status, blocked = box.redeem(issued["code"], "pixel", client="203.0.113.5")
        self.assertEqual(status, 429, blocked)
        self.assertNotIn(issued["code"], json.dumps(blocked))
        status, pending = box.redeem(issued["code"], "pixel", client="203.0.113.9")
        self.assertEqual(status, 202, pending)
        clock.advance(relay.PAIRING_TTL_SECONDS)
        issued = box.start_pairing("host-rate")
        assert issued is not None
        status, pending = box.redeem(issued["code"], "pixel", client="203.0.113.5")
        self.assertEqual(status, 202, pending)

    def test_sqlite_registry_is_private_and_stores_no_secrets(self) -> None:
        path = str(Path(os.environ.get("TMPDIR", "/tmp")) / f"relay-mode-{os.getpid()}" / "relay.sqlite")

        def cleanup() -> None:
            _unlink_sqlite(path)
            parent = os.path.dirname(path)
            if os.path.isdir(parent):
                os.rmdir(parent)

        self.addCleanup(cleanup)
        box = relay.Relay(public_origin="http://127.0.0.1", state_path=path)
        self.assertIsNotNone(box.hello("host-mode", "host-token-not-stored", "desk"))
        issued = box.start_pairing("host-mode")
        assert issued is not None
        mode = os.stat(path).st_mode & 0o777
        self.assertEqual(mode, 0o600)
        parent_mode = os.stat(os.path.dirname(path)).st_mode & 0o777
        self.assertEqual(parent_mode, 0o700)
        stored = Path(path).read_bytes()
        self.assertNotIn(issued["code"].encode(), stored)
        self.assertNotIn(b"host-token-not-stored", stored)
        for suffix in ("-wal", "-shm"):
            candidate = path + suffix
            if os.path.exists(candidate):
                self.assertEqual(os.stat(candidate).st_mode & 0o777, 0o600)

    def test_inflight_turns_are_capped(self) -> None:
        box = relay.Relay(public_origin="http://127.0.0.1")
        device, _token = _paired_device(box, "host-cap", "token-cap")
        first = sealed_envelope("req-0", "x" * 20)[0]
        status, queued = box.submit(device, first)
        self.assertEqual(status, 202, queued)
        for index in range(1, relay.MAX_INFLIGHT_PER_DEVICE):
            status, queued = box.submit(device, sealed_envelope(f"req-{index}", "x" * 20)[0])
            self.assertEqual(status, 202, queued)
        status, rejected = box.submit(device, sealed_envelope("req-over", "one more")[0])
        self.assertEqual((status, rejected["error"]), (429, "too_many_pending"))
        status, again = box.submit(device, first)
        self.assertEqual(status, 200, again)

    def test_text_and_negative_length_are_rejected(self) -> None:
        self.assertEqual(
            relay.validate_envelope(
                {"schema_version": 1, "request_id": "req", "content": [{"type": "text", "text": "hello"}]}
            ),
            "plaintext_rejected",
        )
        self.assertEqual(
            relay.validate_envelope(
                {
                    "schema_version": 1,
                    "request_id": "req",
                    "content": [{"type": "sealed", "body": "a" * (relay.MAX_SEALED_CHARS + 1)}],
                }
            ),
            "invalid_sealed",
        )

    def test_log_guard_drops_tokens(self) -> None:
        relay.install_log_guard()
        logs = LogCapture()
        relay.log.addHandler(logs)
        self.addCleanup(relay.log.removeHandler, logs)
        relay.note_secret("super-secret-token-value-0123456789")
        relay.log.info("leaked super-secret-token-value-0123456789")
        relay.log.info("cookie plexi_phone=not-a-real-token")
        relay.log.info("event=ok outcome=ready")
        self.assertNotIn("super-secret-token-value-0123456789", logs.text)
        self.assertNotIn("plexi_phone=", logs.text)
        self.assertIn("outcome=ready", logs.text)

    def test_one_host_cannot_see_another_hosts_turn(self) -> None:
        box = relay.Relay(public_origin="http://127.0.0.1")
        device_a, _token_a = _paired_device(box, "host-a", "token-a")
        _device_b, _token_b = _paired_device(box, "host-b", "token-b")
        payload, sealed = sealed_envelope("req-a", "only-for-a")
        status, queued = box.submit(device_a, payload)
        self.assertEqual(status, 202, queued)
        link_b = box.links["host-b"]
        stolen = _drain(link_b)
        self.assertFalse(any(item.get("text") == sealed or item.get("text") == "only-for-a" for item in stolen))
        delivered = next(item for item in _drain(box.links["host-a"]) if item.get("type") == "deliver")
        self.assertEqual(delivered["text"], sealed)
        self.assertNotIn("only-for-a", delivered["text"])
        self.assertEqual(delivered["device_id"], device_a.device_id)
        box.reply("host-b", {"type": "reply", "delivery_id": delivered["delivery_id"], "state": "succeeded", "reply": "from-b"})
        page = box.conversation(device_a, 0)
        self.assertFalse(any(event.get("text") == "from-b" for event in page["events"]))
        self.assertEqual(box.deliveries[delivered["delivery_id"]].state, "queued")

    def test_revoke_drops_the_live_session_and_approval_changes_nothing(self) -> None:
        box = relay.Relay(public_origin="http://127.0.0.1")
        device, token = _paired_device(box, "host-rev", "token-rev")
        status, queued = box.submit(
            device,
            sealed_envelope("req-rev", "hold")[0],
        )
        self.assertEqual(status, 202, queued)
        code, body = box.approve(device)
        self.assertEqual(code, 403, body)
        self.assertEqual(box.deliveries[queued["delivery_id"]].state, "queued")
        revoked = box.revoke("host-rev", device.device_id)
        self.assertEqual(revoked["type"], "revoked")
        self.assertIsNone(box.device_for_token(token))
        self.assertEqual(box.revoke("host-other", device.device_id)["type"], "error")


class PhoneCryptoTest(unittest.TestCase):
    def test_tamper_replay_and_swapped_key_are_rejected(self) -> None:
        from cryptography.hazmat.primitives import serialization
        from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey

        desk = X25519PrivateKey.generate()
        private = desk.private_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PrivateFormat.Raw,
            encryption_algorithm=serialization.NoEncryption(),
        )
        public = desk.public_key().public_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PublicFormat.Raw,
        )
        phone = phone_crypto.PhoneState(public)
        canary = "secret-canary-not-for-the-relay"
        handshake = phone.seal_turn(canary, "req-1", False)
        self.assertNotIn(canary, handshake)
        opened, key, _phone_pub = phone_crypto.open_handshake(private, public, phone_crypto.b64u_decode(handshake))
        self.assertEqual(opened["text"], canary)
        self.assertIs(opened["join_desktop"], False)

        swapped = bytearray(phone_crypto.b64u_decode(handshake))
        swapped[3] ^= 0x01
        with self.assertRaises(phone_crypto.SealError) as swapped_error:
            phone_crypto.open_handshake(private, public, bytes(swapped))
        self.assertEqual(swapped_error.exception.outcome, "tamper")

        data = phone.seal_turn("second", "req-2", True)
        payload, counter = phone_crypto.open_data(key, phone_crypto.b64u_decode(data), "req-2", 0, phone_crypto.DIR_PHONE)
        self.assertEqual(payload["text"], "second")
        self.assertIs(payload["join_desktop"], True)
        with self.assertRaises(phone_crypto.SealError) as replay:
            phone_crypto.open_data(key, phone_crypto.b64u_decode(data), "req-2", counter, phone_crypto.DIR_PHONE)
        self.assertEqual(replay.exception.outcome, "replay")
        tampered = bytearray(phone_crypto.b64u_decode(data))
        tampered[-1] ^= 0x01
        with self.assertRaises(phone_crypto.SealError) as tamper:
            phone_crypto.open_data(key, bytes(tampered), "req-2", 0, phone_crypto.DIR_PHONE)
        self.assertEqual(tamper.exception.outcome, "tamper")


def _paired_device(box: relay.Relay, host_id: str, token: str) -> tuple[relay.Device, str]:
    link = box.hello(host_id, token, "desk")
    assert link is not None
    issued = box.start_pairing(host_id)
    assert issued is not None
    status, pending = box.redeem(issued["code"], "pixel")
    assert status == 202, pending
    confirmed = box.confirm(host_id, issued["pairing_id"])
    assert confirmed["type"] == "pair_confirmed", confirmed
    _status, _body, session = box.pairing_status(issued["pairing_id"])
    assert session is not None
    device = box.device_for_token(session)
    assert device is not None
    return device, session


def _drain(link: relay.DesktopLink) -> list[dict]:
    messages: list[dict] = []
    while True:
        try:
            messages.append(link.outbound.get_nowait())
        except queue.Empty:
            return messages


def _unlink_sqlite(path: str) -> None:
    for suffix in ("", "-wal", "-shm"):
        try:
            os.remove(path + suffix)
        except OSError:
            pass


if __name__ == "__main__":
    unittest.main()
