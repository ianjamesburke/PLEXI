"""Relay contract tests. Run: python3 -m unittest services/relay/test_relay.py"""

from __future__ import annotations

import base64
import hashlib
import json
import logging
import os
import socket
import struct
import sys
import threading
import time
import unittest
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import relay  # noqa: E402

CANARY = "CANARY-plexi-relay-9f3a2c1b"
STATIC = Path(__file__).resolve().parents[2] / "clients" / "phone-web" / "static"
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


def _http(method: str, url: str, body: dict | None = None, cookie: str | None = None) -> tuple[int, dict, str | None]:
    data = json.dumps(body).encode() if body is not None else None
    headers = {"Content-Type": "application/json"} if body is not None else {}
    if cookie:
        headers["Cookie"] = f"{relay.COOKIE}={cookie}"
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=5) as response:
            raw = response.read().decode()
            token = _cookie(response.headers.get("Set-Cookie"))
            return response.status, json.loads(raw or "{}"), token
    except urllib.error.HTTPError as exc:
        raw = exc.read().decode()
        token = _cookie(exc.headers.get("Set-Cookie") if exc.headers else None)
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
    def __init__(self, port: int, host_id: str = "host-1", token: str = "token-1") -> None:
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
        self.send({"type": "hello", "host_id": host_id, "host_token": token, "host_label": "desk"})
        hello = self.recv()
        if hello.get("type") != "hello_ok":
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
            {"schema_version": 1, "request_id": "req-early", "content": [{"type": "text", "text": "hi"}]},
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
            {"schema_version": 1, "request_id": "req-ok", "content": [{"type": "text", "text": "hello"}]},
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
            {"schema_version": 1, "request_id": "req-resume", "content": [{"type": "text", "text": "still paired"}]},
            cookie=cookie,
        )
        self.assertEqual(status, 202, queued)

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
            {"schema_version": 1, "request_id": "req-revoked", "content": [{"type": "text", "text": "nope"}]},
            cookie=second,
        )
        self.assertEqual(status, 401, rejected)
        status, kept, _ = _http(
            "POST",
            f"{self.base}/api/turns",
            {"schema_version": 1, "request_id": "req-kept", "content": [{"type": "text", "text": "still here"}]},
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
        status, queued, _ = _http(
            "POST",
            f"{self.base}/api/turns",
            {"schema_version": 1, "request_id": "req-1", "conversation_id": "browser-supplied", "content": [{"type": "text", "text": text}]},
            cookie=cookie,
        )
        self.assertEqual(status, 202, queued)
        delivered = desk.recv()
        self.assertEqual(delivered["type"], "deliver")
        self.assertEqual(delivered["text"], text)
        self.assertTrue(delivered["conversation_id"].startswith("phone-"))
        self.assertNotEqual(delivered["conversation_id"], "browser-supplied")
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
            {"schema_version": 1, "request_id": "req-off", "content": [{"type": "text", "text": CANARY}]},
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
        status, queued, _ = _http(
            "POST",
            f"{self.base}/api/turns",
            {"schema_version": 1, "request_id": "req-canary", "content": [{"type": "text", "text": CANARY}]},
            cookie=cookie,
        )
        self.assertEqual(status, 202, queued)
        delivered = desk.recv()
        desk.send({"type": "ack", "delivery_id": delivered["delivery_id"]})
        desk.send(
            {
                "type": "reply",
                "delivery_id": delivered["delivery_id"],
                "request_id": "req-canary",
                "state": "succeeded",
                "reply": CANARY,
            }
        )
        self.assertTrue(self._wait(lambda: self.relay.deliveries[delivered["delivery_id"]].body is None))
        status, page, _ = _http("GET", f"{self.base}/api/conversation?after=0", cookie=cookie)
        self.assertTrue(any(event.get("text") == CANARY for event in page["events"]))
        self.assertNotIn(CANARY, self.logs.text)
        self.assertIsNone(self.relay.deliveries[delivered["delivery_id"]].body)

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
        status, queued = box.submit(
            device,
            {"schema_version": 1, "request_id": "req-ttl", "content": [{"type": "text", "text": CANARY}]},
        )
        self.assertEqual(status, 202, queued)
        self.assertIn(CANARY, box.memory_text())
        clock.advance(119)
        box.purge()
        self.assertIn(CANARY, box.memory_text())
        clock.advance(2)
        box.purge()
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
            {"schema_version": 1, "request_id": "req-ack", "content": [{"type": "text", "text": CANARY}]},
        )
        self.assertEqual(status, 202)
        box.ack("host-ack", queued["delivery_id"])
        self.assertIsNone(box.deliveries[queued["delivery_id"]].body)
        # The phone has not polled, so its one-shot projection may still hold
        # the text. The delivery record itself no longer does.
        self.assertTrue(all(item.body is None for item in box.deliveries.values()))


if __name__ == "__main__":
    unittest.main()
