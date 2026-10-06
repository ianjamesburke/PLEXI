"""Plexi phone relay.

The desktop host connects outbound (WebSocket). The phone uses HTTPS on the
same origin. Message bodies live only in memory: they are dropped when the
desktop acknowledges delivery, and any still-undelivered body is purged after
two minutes. Logs record ids, sizes, and outcomes — never message text, pairing
codes, or session tokens.

This process does not deploy itself. See DEPLOY.md for the staging note.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import json
import logging
import os
import queue
import secrets
import select
import socket
import struct
import threading
import time
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Callable
from urllib.parse import parse_qs, urlparse

log = logging.getLogger("plexi.relay")

WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
CODE_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"
UNDELIVERED_TTL_SECONDS = 120.0
PAIRING_TTL_SECONDS = 300.0
HEARTBEAT_TIMEOUT_SECONDS = 45.0
MAX_BODY_BYTES = 64 * 1024
MAX_TEXT_CHARS = 8000
COOKIE = "plexi_phone"

# Structured logs may name these fields only. Values are short tokens.
_LOG_FIELDS = (
    "host_id",
    "device_id",
    "pairing_id",
    "delivery_id",
    "request_id",
    "bytes",
    "state",
    "outcome",
    "status",
    "conversation_id",
)


def trace(event: str, **fields: object) -> None:
    """info log of allowlisted ids and sizes. Refuses free text."""
    parts = [f"event={event}"]
    for key in _LOG_FIELDS:
        if key not in fields or fields[key] is None:
            continue
        value = fields[key]
        if not isinstance(value, (int, str)):
            continue
        text = str(value)
        if len(text) > 80 or any(ch.isspace() for ch in text):
            continue
        parts.append(f"{key}={text}")
    log.info(" ".join(parts))


class _BodyFilter(logging.Filter):
    """Last-resort drop if a record interpolates a body-sized string.

    The canary test still fails when application code logs the marker: this
    filter does not rewrite messages, it only rejects a record whose rendered
    text is implausibly long for an allowlisted line. Short canaries must be
    kept out by `trace` never receiving them.
    """

    def filter(self, record: logging.LogRecord) -> bool:
        try:
            rendered = record.getMessage()
        except Exception:
            return False
        return len(rendered) <= 512 and "\n" not in rendered


def install_log_guard() -> None:
    log.addFilter(_BodyFilter())
    log.setLevel(logging.INFO)
    log.propagate = True


def phone_static_dir() -> Path:
    override = os.environ.get("RELAY_PHONE_STATIC")
    if override:
        return Path(override)
    return Path(__file__).resolve().parents[2] / "clients" / "phone-web" / "static"


def _now_unix() -> float:
    return time.time()


def _code() -> str:
    return "".join(secrets.choice(CODE_ALPHABET) for _ in range(8))


def _hash(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


def _fingerprint(host_id: str, label: str, nonce: str) -> str:
    digest = hashlib.sha256(f"{host_id}|{label}|{nonce}".encode()).hexdigest()
    return digest[:16]


def _iso(ts: float) -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(ts))


@dataclass
class Pairing:
    pairing_id: str
    host_id: str
    code_hash: str
    expires_at: float
    status: str = "open"  # open | pending_desktop | confirmed | denied | expired
    device_label: str = ""
    nonce: str = ""
    fingerprint: str = ""
    device_id: str | None = None


@dataclass
class Device:
    device_id: str
    host_id: str
    label: str
    fingerprint: str
    revoked: bool = False


@dataclass
class Delivery:
    delivery_id: str
    request_id: str
    host_id: str
    device_id: str
    conversation_id: str
    body: str | None
    body_hash: str
    expires_at: float
    acked: bool = False
    state: str = "queued"


@dataclass
class PhoneEvent:
    cursor: int
    kind: str
    request_id: str
    state: str | None = None
    text: str | None = None
    status: str | None = None
    error: str | None = None

    def public(self) -> dict:
        payload: dict = {"cursor": self.cursor, "kind": self.kind, "request_id": self.request_id}
        if self.kind == "receipt":
            payload["state"] = self.state
            if self.status:
                payload["status"] = self.status
            if self.error:
                payload["error"] = self.error
        elif self.text:
            payload["text"] = self.text
        return payload


@dataclass
class DesktopLink:
    host_id: str
    label: str
    last_seen: float
    outbound: queue.Queue = field(default_factory=queue.Queue)
    alive: bool = True

    def push(self, message: dict) -> None:
        if self.alive:
            self.outbound.put(message)


class Relay:
    """In-memory pairing and delivery. No message body is written to disk."""

    def __init__(
        self,
        clock: Callable[[], float] | None = None,
        undelivered_ttl: float = UNDELIVERED_TTL_SECONDS,
        pairing_ttl: float = PAIRING_TTL_SECONDS,
        heartbeat_timeout: float = HEARTBEAT_TIMEOUT_SECONDS,
        public_origin: str = "",
    ) -> None:
        self.clock = clock or _now_unix
        self.undelivered_ttl = undelivered_ttl
        self.pairing_ttl = pairing_ttl
        self.heartbeat_timeout = heartbeat_timeout
        self.public_origin = public_origin.rstrip("/")
        self.lock = threading.Lock()
        self.hosts: dict[str, str] = {}  # host_id -> token hash
        self.links: dict[str, DesktopLink] = {}
        self.pairings: dict[str, Pairing] = {}
        self.code_index: dict[str, str] = {}  # code hash -> pairing_id
        self.devices: dict[str, Device] = {}
        self.sessions: dict[str, str] = {}  # session hash -> device_id
        self.deliveries: dict[str, Delivery] = {}
        self.by_request: dict[tuple[str, str], str] = {}  # (device_id, request_id) -> delivery_id
        self.events: dict[str, list[PhoneEvent]] = {}
        self.cursors: dict[str, int] = {}

    def now(self) -> float:
        return self.clock()

    # ── desktop ────────────────────────────────────────────────────────────

    def hello(self, host_id: str, host_token: str, label: str) -> DesktopLink | None:
        if not host_id or not host_token or len(host_id) > 80 or len(host_token) > 128:
            trace("hello_rejected", outcome="invalid")
            return None
        token_hash = _hash(host_token)
        with self.lock:
            existing = self.hosts.get(host_id)
            if existing is None:
                self.hosts[host_id] = token_hash
            elif not hmac.compare_digest(existing, token_hash):
                trace("hello_rejected", host_id=host_id, outcome="token_mismatch")
                return None
            previous = self.links.get(host_id)
            if previous is not None:
                previous.alive = False
            link = DesktopLink(host_id=host_id, label=label[:80] or "desktop", last_seen=self.now())
            self.links[host_id] = link
        trace("desktop_online", host_id=host_id, outcome="connected")
        return link

    def disconnect(self, host_id: str, link: DesktopLink) -> None:
        with self.lock:
            current = self.links.get(host_id)
            if current is link:
                link.alive = False
                self.links.pop(host_id, None)
        trace("desktop_offline", host_id=host_id, outcome="disconnected")

    def note_seen(self, link: DesktopLink) -> None:
        link.last_seen = self.now()

    def start_pairing(self, host_id: str) -> dict | None:
        if host_id not in self.hosts:
            return None
        code = _code()
        pairing_id = "pair-" + secrets.token_hex(8)
        pairing = Pairing(
            pairing_id=pairing_id,
            host_id=host_id,
            code_hash=_hash(code),
            expires_at=self.now() + self.pairing_ttl,
        )
        with self.lock:
            self.pairings[pairing_id] = pairing
            self.code_index[pairing.code_hash] = pairing_id
        origin = self.public_origin or ""
        qr_url = f"{origin}/" if origin else "/"
        trace("pairing_started", host_id=host_id, pairing_id=pairing_id, outcome="code_issued")
        return {
            "type": "pair_code",
            "pairing_id": pairing_id,
            "code": code,
            "expires_at": _iso(pairing.expires_at),
            "qr_url": qr_url,
        }

    def confirm(self, host_id: str, pairing_id: str) -> dict:
        with self.lock:
            pairing = self.pairings.get(pairing_id)
            if pairing is None or pairing.host_id != host_id:
                return {"type": "error", "error": "unknown_pairing"}
            if pairing.expires_at <= self.now() and pairing.status != "confirmed":
                pairing.status = "expired"
                self.code_index.pop(pairing.code_hash, None)
                trace("pairing_expired", host_id=host_id, pairing_id=pairing_id, outcome="expired")
                return {"type": "error", "error": "pairing_expired"}
            if pairing.status == "confirmed" and pairing.device_id:
                trace("pairing_confirmed", host_id=host_id, pairing_id=pairing_id, device_id=pairing.device_id, outcome="retry")
                return {"type": "pair_confirmed", "pairing_id": pairing_id, "device_id": pairing.device_id, "fingerprint": pairing.fingerprint}
            if pairing.status != "pending_desktop":
                return {"type": "error", "error": "not_waiting_for_confirm"}
            device_id = "dev-" + secrets.token_hex(8)
            self.devices[device_id] = Device(
                device_id=device_id,
                host_id=host_id,
                label=pairing.device_label,
                fingerprint=pairing.fingerprint,
            )
            pairing.status = "confirmed"
            pairing.device_id = device_id
            self.code_index.pop(pairing.code_hash, None)
            trace("pairing_confirmed", host_id=host_id, pairing_id=pairing_id, device_id=device_id, outcome="confirmed")
            return {
                "type": "pair_confirmed",
                "pairing_id": pairing_id,
                "device_id": device_id,
                "fingerprint": pairing.fingerprint,
            }

    def deny(self, host_id: str, pairing_id: str) -> dict:
        with self.lock:
            pairing = self.pairings.get(pairing_id)
            if pairing is None or pairing.host_id != host_id:
                return {"type": "error", "error": "unknown_pairing"}
            if pairing.status == "confirmed":
                return {"type": "error", "error": "already_confirmed"}
            pairing.status = "denied"
            self.code_index.pop(pairing.code_hash, None)
        trace("pairing_denied", host_id=host_id, pairing_id=pairing_id, outcome="denied")
        return {"type": "pair_denied", "pairing_id": pairing_id}

    def revoke(self, host_id: str, device_id: str) -> dict:
        with self.lock:
            device = self.devices.get(device_id)
            if device is None or device.host_id != host_id:
                return {"type": "error", "error": "unknown_device"}
            device.revoked = True
            dead = [token for token, bound in self.sessions.items() if bound == device_id]
            for token in dead:
                self.sessions.pop(token, None)
        trace("device_revoked", host_id=host_id, device_id=device_id, outcome="revoked")
        return {"type": "revoked", "device_id": device_id}

    def ack(self, host_id: str, delivery_id: str) -> None:
        with self.lock:
            delivery = self.deliveries.get(delivery_id)
            if delivery is None or delivery.host_id != host_id:
                return
            delivery.acked = True
            delivery.body = None
            if delivery.state == "queued":
                delivery.state = "delivered"
        trace("delivery_acked", host_id=host_id, delivery_id=delivery_id, outcome="body_dropped")

    def reply(self, host_id: str, message: dict) -> None:
        delivery_id = message.get("delivery_id")
        if not isinstance(delivery_id, str):
            return
        state = message.get("state") if isinstance(message.get("state"), str) else "failed"
        reply_text = message.get("reply") if isinstance(message.get("reply"), str) else None
        error = message.get("error") if isinstance(message.get("error"), str) else None
        turn_id = message.get("turn_id") if isinstance(message.get("turn_id"), str) else None
        with self.lock:
            delivery = self.deliveries.get(delivery_id)
            if delivery is None or delivery.host_id != host_id:
                trace("reply_ignored", host_id=host_id, outcome="unknown_delivery")
                return
            delivery.acked = True
            delivery.body = None
            delivery.state = state
            request_id = delivery.request_id
            device_id = delivery.device_id
            if state == "waiting_for_permission":
                self._append(device_id, PhoneEvent(0, "receipt", request_id, state="waiting_for_permission", status="waiting on desktop"))
            else:
                if reply_text:
                    self._append(device_id, PhoneEvent(0, "assistant_reply", request_id, text=reply_text))
                event = PhoneEvent(0, "receipt", request_id, state=state)
                if state != "succeeded" and error:
                    event.error = error[:240]
                self._append(device_id, event)
        trace(
            "reply_forwarded",
            host_id=host_id,
            delivery_id=delivery_id,
            request_id=request_id,
            state=state if state in {"succeeded", "failed", "cancelled", "expired", "waiting_for_permission"} else "other",
            bytes=len(reply_text or ""),
            outcome="body_dropped",
            conversation_id=turn_id if turn_id and len(turn_id) <= 80 and " " not in turn_id else None,
        )

    def desktop_online(self, host_id: str) -> bool:
        with self.lock:
            return self._online_locked(host_id)

    def _online_locked(self, host_id: str) -> bool:
        link = self.links.get(host_id)
        if link is None or not link.alive:
            return False
        if self.now() - link.last_seen > self.heartbeat_timeout:
            return False
        return True

    def reap(self) -> None:
        """Drop stale desktop links and purge undelivered bodies."""
        now = self.now()
        with self.lock:
            stale = [host_id for host_id, link in self.links.items() if now - link.last_seen > self.heartbeat_timeout]
            for host_id in stale:
                link = self.links.pop(host_id)
                link.alive = False
                trace("desktop_offline", host_id=host_id, outcome="heartbeat_timeout")
            self._purge_locked(now)

    def purge(self) -> None:
        with self.lock:
            self._purge_locked(self.now())

    def _purge_locked(self, now: float) -> None:
        for pairing in self.pairings.values():
            if pairing.status in {"open", "pending_desktop"} and pairing.expires_at <= now:
                pairing.status = "expired"
                self.code_index.pop(pairing.code_hash, None)
                trace("pairing_expired", host_id=pairing.host_id, pairing_id=pairing.pairing_id, outcome="expired")
        expired_ids = []
        for delivery_id, delivery in self.deliveries.items():
            if delivery.body is None:
                continue
            if delivery.acked or delivery.expires_at <= now:
                expired_ids.append(delivery_id)
        for delivery_id in expired_ids:
            delivery = self.deliveries[delivery_id]
            delivery.body = None
            if not delivery.acked and delivery.expires_at <= now:
                delivery.state = "expired"
                self._blank_request(delivery.device_id, delivery.request_id)
                self._append(
                    delivery.device_id,
                    PhoneEvent(0, "receipt", delivery.request_id, state="expired", status="expired before the desktop accepted it"),
                )
                trace("delivery_expired", host_id=delivery.host_id, delivery_id=delivery_id, request_id=delivery.request_id, outcome="purged")
            else:
                trace("delivery_acked", host_id=delivery.host_id, delivery_id=delivery_id, outcome="body_dropped")

    def _blank_request(self, device_id: str, request_id: str) -> None:
        for event in self.events.get(device_id, []):
            if event.request_id == request_id:
                event.text = None

    # ── phone ──────────────────────────────────────────────────────────────

    def redeem(self, code: str, label: str) -> tuple[int, dict]:
        if not isinstance(code, str) or not isinstance(label, str):
            return 400, {"error": "invalid_pairing"}
        label = label.strip()[:40] or "phone"
        code_hash = _hash(code.strip().upper())
        with self.lock:
            pairing_id = self.code_index.get(code_hash)
            pairing = self.pairings.get(pairing_id) if pairing_id else None
            if pairing is None or pairing.status not in {"open", "pending_desktop"}:
                trace("pairing_rejected", outcome="invalid_code")
                return 404, {"error": "invalid_code"}
            if pairing.expires_at <= self.now():
                pairing.status = "expired"
                self.code_index.pop(pairing.code_hash, None)
                trace("pairing_expired", pairing_id=pairing.pairing_id, host_id=pairing.host_id, outcome="expired")
                return 410, {"error": "pairing_expired"}
            if pairing.status == "open":
                nonce = secrets.token_hex(8)
                pairing.nonce = nonce
                pairing.device_label = label
                pairing.fingerprint = _fingerprint(pairing.host_id, label, nonce)
                pairing.status = "pending_desktop"
            link = self.links.get(pairing.host_id)
            pending = {
                "type": "pair_pending",
                "pairing_id": pairing.pairing_id,
                "device_label": pairing.device_label,
                "fingerprint": pairing.fingerprint,
            }
            if link is not None and link.alive:
                link.push(pending)
            trace("pairing_pending", host_id=pairing.host_id, pairing_id=pairing.pairing_id, outcome="awaiting_desktop")
            return 202, {
                "pairing_id": pairing.pairing_id,
                "status": "pending_desktop",
                "fingerprint": pairing.fingerprint,
            }

    def pairing_status(self, pairing_id: str) -> tuple[int, dict, str | None]:
        """Returns HTTP status, body, and a new session token once confirmed."""
        with self.lock:
            pairing = self.pairings.get(pairing_id)
            if pairing is None:
                return 404, {"error": "unknown_pairing"}, None
            if pairing.status in {"open", "pending_desktop"} and pairing.expires_at <= self.now():
                pairing.status = "expired"
                self.code_index.pop(pairing.code_hash, None)
            body = {"pairing_id": pairing.pairing_id, "status": pairing.status, "fingerprint": pairing.fingerprint}
            if pairing.status != "confirmed" or not pairing.device_id:
                return 200, body, None
            device = self.devices.get(pairing.device_id)
            if device is None or device.revoked:
                return 401, {"error": "revoked"}, None
            token = secrets.token_urlsafe(32)
            self.sessions[_hash(token)] = device.device_id
            body["device_id"] = device.device_id
            trace("session_issued", host_id=device.host_id, device_id=device.device_id, pairing_id=pairing.pairing_id, outcome="confirmed")
            return 200, body, token

    def device_for_token(self, token: str | None) -> Device | None:
        if not token:
            return None
        with self.lock:
            device_id = self.sessions.get(_hash(token))
            device = self.devices.get(device_id) if device_id else None
            if device is None or device.revoked:
                return None
            return device

    def device_summaries(self, host_id: str) -> list[dict]:
        """Paired phones this desktop already has. A reconnect uses this list
        instead of minting a new pairing."""
        with self.lock:
            return [
                {
                    "device_id": device.device_id,
                    "label": device.label,
                    "fingerprint": device.fingerprint,
                }
                for device in self.devices.values()
                if device.host_id == host_id and not device.revoked
            ]

    def status_for(self, device: Device) -> dict:
        online = self.desktop_online(device.host_id)
        return {
            "mode": "relay",
            "host": "online" if online else "desktop_offline",
            "desktop": "online" if online else "offline",
            "message": None if online else "desktop offline",
        }

    def submit(self, device: Device, envelope: dict) -> tuple[int, dict]:
        problem = validate_envelope(envelope)
        if problem:
            trace("turn_rejected", device_id=device.device_id, outcome=problem if problem.isidentifier() or "_" in problem else "invalid")
            return 400, {"error": problem}
        text = envelope["content"][0]["text"]
        request_id = envelope["request_id"]
        canonical = json.dumps(envelope, sort_keys=True)
        body_hash = _hash(canonical)
        with self.lock:
            if device.revoked:
                return 401, {"error": "revoked"}
            key = (device.device_id, request_id)
            existing_id = self.by_request.get(key)
            if existing_id:
                existing = self.deliveries[existing_id]
                if existing.body_hash != body_hash:
                    trace("turn_rejected", device_id=device.device_id, request_id=request_id, outcome="operation_conflict")
                    return 409, {"error": "operation_conflict"}
                return 200, {"request_id": request_id, "delivery_id": existing.delivery_id, "state": existing.state}
            if not self._online_locked(device.host_id):
                trace("turn_refused", host_id=device.host_id, device_id=device.device_id, request_id=request_id, bytes=len(text), outcome="desktop_offline")
                return 409, {"error": "desktop_offline", "message": "desktop offline"}
            delivery_id = "del-" + secrets.token_hex(8)
            conversation_id = f"phone-{device.host_id}"
            delivery = Delivery(
                delivery_id=delivery_id,
                request_id=request_id,
                host_id=device.host_id,
                device_id=device.device_id,
                conversation_id=conversation_id,
                body=text,
                body_hash=body_hash,
                expires_at=self.now() + self.undelivered_ttl,
            )
            self.deliveries[delivery_id] = delivery
            self.by_request[key] = delivery_id
            self._append(device.device_id, PhoneEvent(0, "user", request_id, text=text))
            self._append(device.device_id, PhoneEvent(0, "receipt", request_id, state="queued"))
            link = self.links.get(device.host_id)
            if link is not None:
                link.push(
                    {
                        "type": "deliver",
                        "delivery_id": delivery_id,
                        "request_id": request_id,
                        "conversation_id": conversation_id,
                        "device_id": device.device_id,
                        "text": text,
                        "expires_at": _iso(delivery.expires_at),
                    }
                )
        trace(
            "turn_accepted",
            host_id=device.host_id,
            device_id=device.device_id,
            delivery_id=delivery_id,
            request_id=request_id,
            conversation_id=conversation_id,
            bytes=len(text),
            outcome="queued",
        )
        return 202, {"request_id": request_id, "delivery_id": delivery_id, "state": "queued"}

    def cancel(self, device: Device, request_id: str) -> tuple[int, dict]:
        with self.lock:
            delivery_id = self.by_request.get((device.device_id, request_id))
            if delivery_id is None:
                return 404, {"error": "unknown_request_id"}
            delivery = self.deliveries[delivery_id]
            if delivery.state in {"succeeded", "failed", "cancelled", "expired"}:
                return 200, {"request_id": request_id, "state": delivery.state}
            if delivery.acked:
                return 200, {"request_id": request_id, "state": delivery.state, "message": "waiting on desktop"}
            delivery.state = "cancelled"
            delivery.body = None
            self._blank_request(device.device_id, request_id)
            self._append(device.device_id, PhoneEvent(0, "receipt", request_id, state="cancelled"))
            link = self.links.get(device.host_id)
            if link is not None:
                link.push({"type": "cancel", "delivery_id": delivery.delivery_id, "request_id": request_id})
        trace("turn_cancelled", device_id=device.device_id, request_id=request_id, delivery_id=delivery_id, outcome="cancelled")
        return 200, {"request_id": request_id, "state": "cancelled"}

    def conversation(self, device: Device, after: int) -> dict:
        with self.lock:
            events = self.events.get(device.device_id, [])
            page = [event.public() for event in events if event.cursor > after]
            for event in events:
                if event.cursor > after:
                    event.text = None
            cursor = events[-1].cursor if events else 0
            return {"cursor": cursor, "events": page}

    def approve(self, device: Device) -> tuple[int, dict]:
        trace("approval_refused", device_id=device.device_id, host_id=device.host_id, outcome="waiting_on_desktop")
        return 403, {"error": "waiting_on_desktop", "message": "waiting on desktop"}

    def _append(self, device_id: str, event: PhoneEvent) -> None:
        cursor = self.cursors.get(device_id, 0) + 1
        self.cursors[device_id] = cursor
        event.cursor = cursor
        self.events.setdefault(device_id, []).append(event)

    def memory_text(self) -> str:
        """Every message body still held in memory, for the purge test."""
        chunks: list[str] = []
        with self.lock:
            for delivery in self.deliveries.values():
                if delivery.body:
                    chunks.append(delivery.body)
            for events in self.events.values():
                for event in events:
                    if event.text:
                        chunks.append(event.text)
                    if event.error:
                        chunks.append(event.error)
        return "\n".join(chunks)


def validate_envelope(body: object) -> str | None:
    if not isinstance(body, dict):
        return "body_must_be_object"
    if body.get("schema_version") != 1:
        return "unsupported_schema_version"
    request_id = body.get("request_id")
    if not isinstance(request_id, str) or not 1 <= len(request_id) <= 128:
        return "invalid_request_id"
    content = body.get("content")
    if not isinstance(content, list) or len(content) != 1 or not isinstance(content[0], dict):
        return "content_must_be_one_text_part"
    part = content[0]
    if part.get("type") != "text" or not isinstance(part.get("text"), str):
        return "content_must_be_one_text_part"
    text = part["text"]
    if not text.strip() or len(text) > MAX_TEXT_CHARS:
        return "invalid_text"
    return None


PAIR_PAGE = """<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Pair Plexi</title>
  <link rel="icon" href="/icon.svg" type="image/svg+xml">
  <link rel="stylesheet" href="/app.css">
</head>
<body>
  <header class="bar"><h1>Plexi</h1><p id="pair-status" role="status">Enter the code from your desktop</p></header>
  <main>
    <form id="pair-form" class="composer" autocomplete="off">
      <label for="code">Pairing code</label>
      <input id="code" name="code" inputmode="text" autocapitalize="characters" maxlength="8" required>
      <label for="label">Name this phone</label>
      <input id="label" name="label" maxlength="40" placeholder="phone" value="phone">
      <button type="submit">Pair</button>
    </form>
    <p id="fingerprint"></p>
    <p>The desktop must confirm this phone. A code alone does not pair it.</p>
    <p>Messages pass through this relay in the clear on the server. Transport is protected by TLS. This is not end-to-end encryption.</p>
  </main>
  <script>
    const status = document.getElementById("pair-status");
    const fingerprint = document.getElementById("fingerprint");
    document.getElementById("pair-form").addEventListener("submit", async (event) => {
      event.preventDefault();
      const code = document.getElementById("code").value.trim();
      const label = document.getElementById("label").value.trim() || "phone";
      status.textContent = "Waiting for the desktop to confirm…";
      const redeem = await fetch("/api/pair", {method:"POST", headers:{"Content-Type":"application/json"}, body: JSON.stringify({code, label})});
      const body = await redeem.json();
      if (!redeem.ok) { status.textContent = body.message || body.error || "Pairing failed"; return; }
      fingerprint.textContent = "Fingerprint " + (body.fingerprint || "");
      const id = body.pairing_id;
      const timer = setInterval(async () => {
        const res = await fetch("/api/pair/" + encodeURIComponent(id), {credentials:"same-origin"});
        const poll = await res.json();
        if (poll.status === "confirmed") { clearInterval(timer); location.replace("/"); }
        else if (poll.status === "denied" || poll.status === "expired") { clearInterval(timer); status.textContent = poll.status === "denied" ? "Desktop denied this phone" : "Code expired"; }
      }, 1000);
    });
  </script>
</body>
</html>
"""


def cookie_token(header: str | None) -> str | None:
    if not header:
        return None
    for part in header.split(";"):
        name, _, value = part.strip().partition("=")
        if name == COOKIE and value:
            return value
    return None


def make_handler(relay: Relay, static_dir: Path, secure_cookie: bool) -> type[BaseHTTPRequestHandler]:
    class Handler(BaseHTTPRequestHandler):
        server_version = "PlexiRelay/0"
        protocol_version = "HTTP/1.1"

        def log_message(self, fmt: str, *args: object) -> None:
            # Request lines only: method, path, status. No bodies.
            trace("http", outcome="request", status=str(args[1]) if len(args) > 1 else None)

        def _origin_ok(self) -> bool:
            origin = self.headers.get("Origin")
            if not origin:
                return True
            host = self.headers.get("Host", "")
            parsed = urlparse(origin)
            return parsed.netloc == host

        def _json(self, status: int, payload: dict, cookie: str | None = None) -> None:
            data = json.dumps(payload).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Connection", "close")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.close_connection = True
            if cookie is not None:
                flag = "; Secure" if secure_cookie else ""
                self.send_header("Set-Cookie", f"{COOKIE}={cookie}; HttpOnly; SameSite=Lax; Path=/{flag}")
            self.end_headers()
            self.wfile.write(data)

        def _device(self) -> Device | None:
            return relay.device_for_token(cookie_token(self.headers.get("Cookie")))

        def do_GET(self) -> None:  # noqa: N802
            if self._upgrade():
                return
            url = urlparse(self.path)
            if url.path == "/healthz":
                self._json(200, {"ok": True, "service": "relay"})
                return
            if url.path == "/api/status":
                device = self._device()
                if device is None:
                    self._json(401, {"error": "unauthorized"})
                    return
                self._json(200, relay.status_for(device))
                return
            if url.path == "/api/conversation":
                device = self._device()
                if device is None:
                    self._json(401, {"error": "unauthorized"})
                    return
                try:
                    after = max(0, int(parse_qs(url.query).get("after", ["0"])[0]))
                except ValueError:
                    self._json(400, {"error": "invalid_cursor"})
                    return
                self._json(200, relay.conversation(device, after))
                return
            if url.path.startswith("/api/pair/"):
                pairing_id = url.path.split("/")[3] if len(url.path.split("/")) > 3 else ""
                status, body, token = relay.pairing_status(pairing_id)
                self._json(status, body, token)
                return
            if url.path in ("", "/"):
                if self._device() is None:
                    data = PAIR_PAGE.encode()
                    self.send_response(200)
                    self.send_header("Content-Type", "text/html; charset=utf-8")
                    self.send_header("Cache-Control", "no-store")
                    self.send_header("Content-Length", str(len(data)))
                    self.send_header("Connection", "close")
                    self.close_connection = True
                    self.end_headers()
                    self.wfile.write(data)
                    return
            self._static(url.path)

        def do_POST(self) -> None:  # noqa: N802
            if not self._origin_ok():
                self._json(403, {"error": "bad_origin"})
                return
            url = urlparse(self.path)
            parts = [part for part in url.path.split("/") if part]
            try:
                length = int(self.headers.get("Content-Length", "0"))
            except ValueError:
                self._json(400, {"error": "invalid_content_length"})
                return
            if length > MAX_BODY_BYTES:
                self._json(413, {"error": "body_too_large"})
                return
            raw = self.rfile.read(length) if length else b""
            if parts == ["api", "pair"]:
                try:
                    body = json.loads(raw or b"null")
                except json.JSONDecodeError:
                    self._json(400, {"error": "invalid_json"})
                    return
                if not isinstance(body, dict):
                    self._json(400, {"error": "invalid_pairing"})
                    return
                self._json(*relay.redeem(str(body.get("code", "")), str(body.get("label", "phone"))))
                return
            if parts == ["api", "approvals"] or (len(parts) >= 2 and parts[-1] == "approve"):
                device = self._device()
                if device is None:
                    self._json(401, {"error": "unauthorized"})
                    return
                self._json(*relay.approve(device))
                return
            device = self._device()
            if device is None:
                self._json(401, {"error": "unauthorized"})
                return
            if parts == ["api", "turns"]:
                try:
                    body = json.loads(raw or b"null")
                except json.JSONDecodeError:
                    trace("turn_rejected", outcome="invalid_json")
                    self._json(400, {"error": "invalid_json"})
                    return
                self._json(*relay.submit(device, body))
                return
            if len(parts) == 4 and parts[:2] == ["api", "turns"] and parts[3] == "cancel":
                self._json(*relay.cancel(device, parts[2]))
                return
            self._json(404, {"error": "not_found"})

        def _static(self, path: str) -> None:
            rel = "index.html" if path in ("", "/") else path.lstrip("/")
            target = (static_dir / rel).resolve()
            if static_dir.resolve() not in target.parents and target != static_dir.resolve():
                self._json(404, {"error": "not_found"})
                return
            if not target.is_file():
                self._json(404, {"error": "not_found"})
                return
            try:
                data = target.read_bytes()
            except OSError as exc:
                trace("static_failed", outcome="read_failed")
                log.warning("static read failed: %s", exc.__class__.__name__)
                self._json(500, {"error": "read_failed"})
                return
            import mimetypes

            kind = mimetypes.guess_type(target.name)[0] or "application/octet-stream"
            self.send_response(200)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Cache-Control", "no-store")
            self.send_header("Connection", "close")
            self.close_connection = True
            self.end_headers()
            self.wfile.write(data)

        def _upgrade(self) -> bool:
            if urlparse(self.path).path != "/v1/desktop":
                return False
            if self.headers.get("Upgrade", "").lower() != "websocket":
                return False
            key = self.headers.get("Sec-WebSocket-Key", "")
            if not key:
                self._json(400, {"error": "missing_websocket_key"})
                return True
            origin = self.headers.get("Origin")
            if origin and not self._origin_ok():
                self._json(403, {"error": "bad_origin"})
                return True
            accept = base64.b64encode(hashlib.sha1((key + WS_GUID).encode()).digest()).decode()
            self.send_response(101, "Switching Protocols")
            self.send_header("Upgrade", "websocket")
            self.send_header("Connection", "Upgrade")
            self.send_header("Sec-WebSocket-Accept", accept)
            self.end_headers()
            serve_desktop_socket(self.connection, relay)
            self.close_connection = True
            return True

    return Handler


def _read_exact(sock: socket.socket, n: int) -> bytes | None:
    buf = b""
    while len(buf) < n:
        try:
            chunk = sock.recv(n - len(buf))
        except OSError:
            return None
        if not chunk:
            return None
        buf += chunk
    return buf


def ws_send(sock: socket.socket, payload: bytes, opcode: int = 0x1) -> None:
    frame = bytes([0x80 | opcode])
    length = len(payload)
    if length < 126:
        frame += bytes([length])
    elif length < 65536:
        frame += bytes([126]) + struct.pack("!H", length)
    else:
        frame += bytes([127]) + struct.pack("!Q", length)
    sock.sendall(frame + payload)


def ws_recv(sock: socket.socket) -> tuple[int, bytes] | None:
    """Return (opcode, payload), or None when the socket closes."""
    head = _read_exact(sock, 2)
    if not head:
        return None
    opcode = head[0] & 0x0F
    masked = head[1] & 0x80
    length = head[1] & 0x7F
    if length == 126:
        extra = _read_exact(sock, 2)
        if not extra:
            return None
        length = struct.unpack("!H", extra)[0]
    elif length == 127:
        extra = _read_exact(sock, 8)
        if not extra:
            return None
        length = struct.unpack("!Q", extra)[0]
    if length > MAX_BODY_BYTES + 4096:
        return None
    mask = b""
    if masked:
        mask = _read_exact(sock, 4) or b""
        if len(mask) != 4:
            return None
    payload = _read_exact(sock, length) if length else b""
    if payload is None:
        return None
    if masked and payload:
        payload = bytes(byte ^ mask[i % 4] for i, byte in enumerate(payload))
    return opcode, payload


def serve_desktop_socket(sock: socket.socket, relay: Relay) -> None:
    link: DesktopLink | None = None
    host_id = ""
    try:
        while True:
            if link is not None:
                while True:
                    try:
                        outgoing = link.outbound.get_nowait()
                    except queue.Empty:
                        break
                    ws_send(sock, json.dumps(outgoing).encode())
            ready, _, _ = select.select([sock], [], [], 0.2)
            if not ready:
                if link is not None and not link.alive:
                    break
                continue
            frame = ws_recv(sock)
            if frame is None:
                break
            opcode, payload = frame
            if opcode == 0x8:
                break
            if opcode == 0x9:
                ws_send(sock, payload, opcode=0xA)
                if link is not None:
                    relay.note_seen(link)
                continue
            if opcode == 0xA:
                if link is not None:
                    relay.note_seen(link)
                continue
            if opcode != 0x1:
                continue
            try:
                message = json.loads(payload.decode())
            except (UnicodeDecodeError, json.JSONDecodeError):
                trace("desktop_frame_rejected", outcome="invalid_json")
                continue
            if not isinstance(message, dict):
                continue
            kind = message.get("type")
            if link is None:
                if kind != "hello":
                    ws_send(sock, json.dumps({"type": "error", "error": "hello_required"}).encode())
                    continue
                host_id = str(message.get("host_id", ""))
                token = str(message.get("host_token", ""))
                label = str(message.get("host_label", "desktop"))
                link = relay.hello(host_id, token, label)
                if link is None:
                    ws_send(sock, json.dumps({"type": "error", "error": "hello_rejected"}).encode())
                    break
                ws_send(
                    sock,
                    json.dumps(
                        {
                            "type": "hello_ok",
                            "host_id": host_id,
                            "devices": relay.device_summaries(host_id),
                        }
                    ).encode(),
                )
                continue
            relay.note_seen(link)
            reply = handle_desktop_message(relay, host_id, message)
            if reply is not None:
                ws_send(sock, json.dumps(reply).encode())
    except OSError as exc:
        trace("desktop_socket_closed", host_id=host_id or None, outcome=exc.__class__.__name__)
    finally:
        if link is not None and host_id:
            relay.disconnect(host_id, link)
        try:
            sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        sock.close()


def handle_desktop_message(relay: Relay, host_id: str, message: dict) -> dict | None:
    kind = message.get("type")
    if kind == "ping":
        return {"type": "pong"}
    if kind == "pair_start":
        started = relay.start_pairing(host_id)
        return started or {"type": "error", "error": "pair_failed"}
    if kind == "pair_confirm":
        return relay.confirm(host_id, str(message.get("pairing_id", "")))
    if kind == "pair_deny":
        return relay.deny(host_id, str(message.get("pairing_id", "")))
    if kind == "revoke":
        return relay.revoke(host_id, str(message.get("device_id", "")))
    if kind == "ack":
        relay.ack(host_id, str(message.get("delivery_id", "")))
        return None
    if kind == "reply":
        relay.reply(host_id, message)
        return None
    trace("desktop_frame_ignored", host_id=host_id, outcome="unknown_type")
    return {"type": "error", "error": "unknown_type"}


class RelayServer(ThreadingHTTPServer):
    allow_reuse_address = True
    daemon_threads = True


def build_server(host: str, port: int, relay: Relay, static_dir: Path, secure_cookie: bool = False) -> RelayServer:
    return RelayServer((host, port), make_handler(relay, static_dir, secure_cookie))


def _reaper(relay: Relay, stop: threading.Event) -> None:
    while not stop.wait(1.0):
        try:
            relay.reap()
        except Exception as exc:
            trace("reaper_failed", outcome=exc.__class__.__name__)


def main() -> None:
    parser = argparse.ArgumentParser(description="Plexi phone relay")
    parser.add_argument("--host", default=os.environ.get("RELAY_HOST", "127.0.0.1"))
    parser.add_argument("--port", type=int, default=int(os.environ.get("PORT") or os.environ.get("RELAY_PORT") or "8790"))
    parser.add_argument("--public-origin", default=os.environ.get("RELAY_PUBLIC_ORIGIN", ""))
    parser.add_argument("--phone-static", default=os.environ.get("RELAY_PHONE_STATIC", ""))
    parser.add_argument(
        "--undelivered-ttl",
        type=float,
        default=float(os.environ.get("RELAY_UNDELIVERED_TTL") or UNDELIVERED_TTL_SECONDS),
        help="seconds to keep an undelivered body when the desktop is offline (default 120)",
    )
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s %(message)s")
    install_log_guard()
    static_dir = Path(args.phone_static) if args.phone_static else phone_static_dir()
    if not static_dir.is_dir():
        log.error("phone static dir missing: %s", static_dir)
        raise SystemExit(1)
    secure = os.environ.get("RELAY_COOKIE_SECURE") == "1"
    relay = Relay(
        undelivered_ttl=args.undelivered_ttl,
        public_origin=args.public_origin or f"http://{args.host}:{args.port}",
    )
    try:
        server = build_server(args.host, args.port, relay, static_dir, secure_cookie=secure)
    except OSError as exc:
        log.error("relay failed to bind %s:%s: %s", args.host, args.port, exc.__class__.__name__)
        raise SystemExit(1) from exc
    stop = threading.Event()
    threading.Thread(target=_reaper, args=(relay, stop), name="relay-reaper", daemon=True).start()
    trace("relay_listening", outcome="ready", status=str(args.port))
    log.info("phone UI %s/", relay.public_origin)
    log.info("desktop websocket %s/v1/desktop", relay.public_origin.replace("https://", "wss://").replace("http://", "ws://"))
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        trace("relay_stopping", outcome="stopped")
    finally:
        stop.set()
        server.server_close()


if __name__ == "__main__":
    main()
