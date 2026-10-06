"""Plexi phone relay.

The desktop host connects outbound (WebSocket). The phone uses HTTPS on the
same origin. Message bodies are sealed envelopes. This process forwards them
and cannot read them. They live only in memory: they are dropped when the
desktop acknowledges delivery, and any still-undelivered body is purged after
two minutes. Logs record ids, sizes, and outcomes — never message text, pairing
codes, or session tokens.

Pairing records can outlive this process. When RELAY_STATE_PATH is set, SQLite
stores host_id, device_id, a hash of the device token, fingerprint, created,
last_seen, revoked, a hash of the host token, and queued-envelope metadata
(request_id, host_id, queued_at, size). It never stores a message body, a
pairing code, or a raw token. Unset, the registry stays in memory.
`PairingRegistry.retain` deletes rows older than 30 days.

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
import sqlite3
import struct
import threading
import time
from collections import deque
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
DEVICE_IDLE_SECONDS = 30 * 24 * 3600
PROTOCOL_VERSION = 1
MAX_BODY_BYTES = 64 * 1024
MAX_TEXT_CHARS = 8000
# A sealed body is the plaintext cap plus the handshake header, tag, and
# base64url expansion. Truncating it would break the authenticator, so an
# oversize seal is rejected whole.
MAX_SEALED_CHARS = 12000
MAX_INFLIGHT_PER_DEVICE = 8
MAX_DESKTOP_SOCKETS = 64
HELLO_DEADLINE_SECONDS = 10.0
CODE_LENGTH = 8
# 32-symbol alphabet × 8 characters is 40 bits. The failure budget below
# makes an online guess of one live code negligible inside PAIRING_TTL.
PAIR_FAIL_LIMIT = 10
PAIR_FAIL_GLOBAL_LIMIT = 100
PAIR_FAIL_WINDOW_SECONDS = PAIRING_TTL_SECONDS
COOKIE = "plexi_phone"
PROTOCOL_MISMATCH = (
    "This relay speaks protocol 1. The desktop sent a different version. "
    "Update both so they match, then pair again."
)
INVALID_CODE_MESSAGE = (
    "That code is not valid. If the desktop and this relay are different "
    "versions, update both so they speak the same protocol, then pair again."
)

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


# Long credentials (session tokens, host tokens). Pairing codes are shorter
# and are kept out by `trace` never accepting them, which the canary test
# covers. A short code must not be a scrub key: it can be a substring of a
# logged id.
_SECRET_MIN = 20
_secrets: deque[str] = deque(maxlen=256)
_secrets_lock = threading.Lock()


def note_secret(value: str) -> None:
    """Remember a credential so a later log line that contains it is dropped."""
    if len(value) < _SECRET_MIN or any(ch.isspace() for ch in value):
        return
    with _secrets_lock:
        if value not in _secrets:
            _secrets.append(value)


def _scrubbed(rendered: str) -> bool:
    lowered = rendered.lower()
    if "plexi_phone=" in lowered or "host_token" in lowered or "bearer " in lowered:
        return True
    with _secrets_lock:
        secrets_now = list(_secrets)
    return any(secret in rendered for secret in secrets_now)


class _BodyFilter(logging.Filter):
    """Drop body-sized lines and any line that contains a live credential.

    The canary test still fails when application code logs the marker: this
    filter does not rewrite messages. Short canaries must be kept out by
    `trace` never receiving them. Credentials registered with `note_secret`
    are dropped even when the line is otherwise short.
    """

    def filter(self, record: logging.LogRecord) -> bool:
        try:
            rendered = record.getMessage()
        except Exception:
            return False
        if len(rendered) > 512 or "\n" in rendered:
            return False
        return not _scrubbed(rendered)


def install_log_guard() -> None:
    if not any(isinstance(item, _BodyFilter) for item in log.filters):
        log.addFilter(_BodyFilter())
    log.setLevel(logging.INFO)
    log.propagate = True


def phone_static_dir() -> Path:
    override = os.environ.get("RELAY_PHONE_STATIC")
    if override:
        return Path(override)
    return Path(__file__).resolve().parents[2] / "clients" / "phone-web" / "static"


def _prepare_private_file(path: str) -> None:
    """Create `path` as 0o600 before SQLite opens it.

    `sqlite3.connect` would otherwise create the file with the process umask,
    which is world-readable on a typical 0o022 umask, until a later chmod.
    """
    fd = os.open(path, os.O_CREAT | os.O_RDWR, 0o600)
    os.close(fd)
    os.chmod(path, 0o600)
    mode = os.stat(path).st_mode & 0o777
    if mode & 0o077:
        raise OSError(f"relay registry {path} mode {mode:o} is not private")


def _now_unix() -> float:
    return time.time()


def _code() -> str:
    return "".join(secrets.choice(CODE_ALPHABET) for _ in range(CODE_LENGTH))


def _hash(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


def secret_equal(left: str, right: str) -> bool:
    """Constant-time equality, including when the inputs differ in length.

    Both sides are hashed to a fixed digest first so a length mismatch cannot
    return early inside `compare_digest`.
    """
    return hmac.compare_digest(
        hashlib.sha256(left.encode()).digest(),
        hashlib.sha256(right.encode()).digest(),
    )


def cookie_header(token: str, secure: bool) -> str:
    parts = [f"{COOKIE}={token}", "HttpOnly", "SameSite=Lax", "Path=/"]
    if secure:
        parts.append("Secure")
    return "; ".join(parts)


def cookies_should_be_secure(bind_host: str, public_origin: str) -> bool:
    """Secure cookies on any public bind. Loopback HTTP stays usable in tests.

    `RELAY_COOKIE_SECURE=1` forces the flag on. `=0` forces it off. Otherwise
    an https public origin or a non-loopback bind (the container listens on
    0.0.0.0 behind the TLS proxy) sets Secure.
    """
    flag = os.environ.get("RELAY_COOKIE_SECURE", "").strip()
    if flag == "1":
        return True
    if flag == "0":
        return False
    if public_origin.lower().startswith("https://"):
        return True
    host = bind_host.strip().lower().strip("[]")
    return host not in {"127.0.0.1", "::1", "localhost"}


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
    # Raw session token, memory only, cleared once the phone presents it.
    # Re-polling before that repeats the same cookie. It is not written to SQLite.
    session_token: str | None = None
    token_consumed: bool = False


@dataclass
class Device:
    device_id: str
    host_id: str
    label: str
    fingerprint: str
    revoked: bool = False
    created: float = 0.0
    last_seen: float = 0.0
    token_hash: str = ""


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


_REGISTRY_SCHEMA = """
CREATE TABLE IF NOT EXISTS hosts (
    host_id TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL,
    created REAL
);
CREATE TABLE IF NOT EXISTS devices (
    device_id TEXT PRIMARY KEY,
    host_id TEXT NOT NULL,
    token_hash TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    created REAL NOT NULL,
    last_seen REAL NOT NULL,
    revoked INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS envelopes (
    request_id TEXT PRIMARY KEY,
    host_id TEXT NOT NULL,
    queued_at REAL NOT NULL,
    size INTEGER NOT NULL
);
"""


class PairingRegistry:
    """SQLite pairing records. Rows are the allowlisted fields only.

    `hosts.token_hash` is the same host credential the process already kept in
    memory, so a restart cannot bind a different token to an existing host_id.
    Device labels, pairing codes, raw tokens, and message bodies are not columns.
    """

    def __init__(self, path: str) -> None:
        parent = os.path.dirname(path)
        if parent:
            # chmod only a directory this process created. A path under /tmp
            # or a mounted volume must not have its existing parent locked down.
            created = not os.path.isdir(parent)
            os.makedirs(parent, mode=0o700, exist_ok=True)
            if created:
                os.chmod(parent, 0o700)
        self.path = path
        _prepare_private_file(path)
        self.conn = sqlite3.connect(path, check_same_thread=False)
        self.conn.execute("PRAGMA journal_mode=WAL")
        self.conn.executescript(_REGISTRY_SCHEMA)
        self._migrate()
        self.conn.commit()
        self._tighten()

    def _migrate(self) -> None:
        """Add columns the original registry shipped without.

        `hosts.created` stays NULL on rows written before this column existed
        so the first retention pass does not wipe every legacy desktop.
        """
        host_cols = {row[1] for row in self.conn.execute("PRAGMA table_info(hosts)")}
        if "created" not in host_cols:
            self.conn.execute("ALTER TABLE hosts ADD COLUMN created REAL")

    def _tighten(self) -> None:
        for suffix in ("", "-wal", "-shm"):
            candidate = self.path + suffix
            if os.path.exists(candidate):
                os.chmod(candidate, 0o600)
                mode = os.stat(candidate).st_mode & 0o777
                if mode & 0o077:
                    raise OSError(f"relay registry {candidate} mode {mode:o} is not private")

    def load(self, now: float) -> tuple[dict[str, str], dict[str, Device], dict[str, str]]:
        self.retain(now)
        hosts = {
            host_id: token_hash
            for host_id, token_hash in self.conn.execute("SELECT host_id, token_hash FROM hosts")
        }
        devices: dict[str, Device] = {}
        sessions: dict[str, str] = {}
        expired: list[str] = []
        rows = self.conn.execute(
            "SELECT device_id, host_id, token_hash, fingerprint, created, last_seen, revoked FROM devices"
        )
        for device_id, host_id, token_hash, fingerprint, created, last_seen, revoked in rows:
            if now - float(last_seen) > DEVICE_IDLE_SECONDS:
                expired.append(device_id)
                continue
            device = Device(
                device_id=device_id,
                host_id=host_id,
                label="phone",
                fingerprint=fingerprint,
                revoked=bool(revoked),
                created=float(created),
                last_seen=float(last_seen),
                token_hash=token_hash or "",
            )
            devices[device_id] = device
            if device.token_hash and not device.revoked:
                sessions[device.token_hash] = device_id
        for device_id in expired:
            self.delete_device(device_id)
            trace("device_expired", device_id=device_id, outcome="idle")
        return hosts, devices, sessions

    def upsert_host(self, host_id: str, token_hash: str, created: float | None = None) -> None:
        """Insert a host. A later upsert refreshes the token hash and leaves `created`."""
        when = time.time() if created is None else created
        self.conn.execute(
            "INSERT INTO hosts (host_id, token_hash, created) VALUES (?, ?, ?) "
            "ON CONFLICT(host_id) DO UPDATE SET token_hash=excluded.token_hash",
            (host_id, token_hash, when),
        )
        self.conn.commit()
        self._tighten()

    def note_envelope(self, request_id: str, host_id: str, queued_at: float, size: int) -> None:
        """Record that an envelope was queued. The body is not a column."""
        self.conn.execute(
            "INSERT INTO envelopes (request_id, host_id, queued_at, size) VALUES (?, ?, ?, ?) "
            "ON CONFLICT(request_id) DO UPDATE SET host_id=excluded.host_id, "
            "queued_at=excluded.queued_at, size=excluded.size",
            (request_id, host_id, queued_at, int(size)),
        )
        self.conn.commit()
        self._tighten()

    def retain(self, now: float) -> dict[str, list[str]]:
        """Delete registry rows and queued-envelope metadata older than 30 days.

        Devices use `last_seen`. Hosts use `created`; a NULL `created` (a row
        from before that column) is kept. Deleting a host also deletes its
        devices and envelope metadata. Envelope rows are ids and sizes only.
        """
        cutoff = now - DEVICE_IDLE_SECONDS
        old_devices = [
            row[0]
            for row in self.conn.execute("SELECT device_id FROM devices WHERE last_seen < ?", (cutoff,))
        ]
        old_hosts = [
            row[0]
            for row in self.conn.execute(
                "SELECT host_id FROM hosts WHERE created IS NOT NULL AND created < ?",
                (cutoff,),
            )
        ]
        cascaded = []
        for host_id in old_hosts:
            cascaded.extend(
                row[0]
                for row in self.conn.execute(
                    "SELECT device_id FROM devices WHERE host_id = ?",
                    (host_id,),
                )
            )
        old_envelopes = [
            row[0]
            for row in self.conn.execute(
                "SELECT request_id FROM envelopes WHERE queued_at < ?",
                (cutoff,),
            )
        ]
        for host_id in old_hosts:
            old_envelopes.extend(
                row[0]
                for row in self.conn.execute(
                    "SELECT request_id FROM envelopes WHERE host_id = ? AND queued_at >= ?",
                    (host_id, cutoff),
                )
            )
        for device_id in old_devices:
            self.conn.execute("DELETE FROM devices WHERE device_id = ?", (device_id,))
        for host_id in old_hosts:
            self.conn.execute("DELETE FROM devices WHERE host_id = ?", (host_id,))
            self.conn.execute("DELETE FROM envelopes WHERE host_id = ?", (host_id,))
            self.conn.execute("DELETE FROM hosts WHERE host_id = ?", (host_id,))
        for request_id in old_envelopes:
            self.conn.execute("DELETE FROM envelopes WHERE request_id = ?", (request_id,))
        self.conn.commit()
        device_ids = list(dict.fromkeys([*old_devices, *cascaded]))
        envelope_ids = list(dict.fromkeys(old_envelopes))
        trace(
            "retention",
            outcome="pruned",
            status=f"h{len(old_hosts)}d{len(device_ids)}e{len(envelope_ids)}",
        )
        return {"hosts": old_hosts, "devices": device_ids, "envelopes": envelope_ids}

    def upsert_device(self, device: Device) -> None:
        self.conn.execute(
            "INSERT INTO devices (device_id, host_id, token_hash, fingerprint, created, last_seen, revoked) "
            "VALUES (?, ?, ?, ?, ?, ?, ?) "
            "ON CONFLICT(device_id) DO UPDATE SET "
            "host_id=excluded.host_id, token_hash=excluded.token_hash, "
            "fingerprint=excluded.fingerprint, created=excluded.created, "
            "last_seen=excluded.last_seen, revoked=excluded.revoked",
            (
                device.device_id,
                device.host_id,
                device.token_hash,
                device.fingerprint,
                device.created,
                device.last_seen,
                1 if device.revoked else 0,
            ),
        )
        self.conn.commit()
        self._tighten()

    def delete_device(self, device_id: str) -> None:
        self.conn.execute("DELETE FROM devices WHERE device_id = ?", (device_id,))
        self.conn.commit()


class Relay:
    """Pairing plus in-memory delivery. No message body is written to disk."""

    def __init__(
        self,
        clock: Callable[[], float] | None = None,
        undelivered_ttl: float = UNDELIVERED_TTL_SECONDS,
        pairing_ttl: float = PAIRING_TTL_SECONDS,
        heartbeat_timeout: float = HEARTBEAT_TIMEOUT_SECONDS,
        public_origin: str = "",
        state_path: str | None = None,
        pair_fail_limit: int = PAIR_FAIL_LIMIT,
        pair_fail_global: int = PAIR_FAIL_GLOBAL_LIMIT,
        pair_fail_window: float = PAIR_FAIL_WINDOW_SECONDS,
    ) -> None:
        self.clock = clock or _now_unix
        self.undelivered_ttl = undelivered_ttl
        self.pairing_ttl = pairing_ttl
        self.heartbeat_timeout = heartbeat_timeout
        self.public_origin = public_origin.rstrip("/")
        self.pair_fail_limit = pair_fail_limit
        self.pair_fail_global = pair_fail_global
        self.pair_fail_window = pair_fail_window
        self._pair_fails: dict[str, list[float]] = {}
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
        # request_id -> {"event": Event, "body": dict | None}. Phone needs-you
        # waits here until the desktop answers on its socket.
        self.asks: dict[str, dict] = {}
        self.registry: PairingRegistry | None = None
        path = (state_path or "").strip()
        if path:
            self.registry = PairingRegistry(path)
            self.hosts, self.devices, self.sessions = self.registry.load(self.now())
            trace("registry_open", outcome="sqlite", status=str(len(self.devices)))
        else:
            trace("registry_open", outcome="memory")

    def now(self) -> float:
        return self.clock()

    # ── desktop ────────────────────────────────────────────────────────────

    def hello(self, host_id: str, host_token: str, label: str) -> DesktopLink | None:
        if not host_id or not host_token or len(host_id) > 80 or len(host_token) > 128:
            trace("hello_rejected", outcome="invalid")
            return None
        note_secret(host_token)
        token_hash = _hash(host_token)
        with self.lock:
            existing = self.hosts.get(host_id)
            if existing is None:
                self.hosts[host_id] = token_hash
                self._persist_host_locked(host_id, token_hash)
            elif not secret_equal(existing, token_hash):
                trace("hello_rejected", host_id=host_id, outcome="token_mismatch")
                return None
            self._expire_devices_locked(self.now())
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
            now = self.now()
            device = Device(
                device_id=device_id,
                host_id=host_id,
                label=pairing.device_label,
                fingerprint=pairing.fingerprint,
                created=now,
                last_seen=now,
            )
            self.devices[device_id] = device
            self._persist_device_locked(device)
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
            device.token_hash = ""
            dead = [token for token, bound in self.sessions.items() if bound == device_id]
            for token in dead:
                self.sessions.pop(token, None)
            for pairing in self.pairings.values():
                if pairing.device_id == device_id:
                    pairing.session_token = None
                    pairing.token_consumed = True
            self._persist_device_locked(device)
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
        if reply_text is not None and len(reply_text) > MAX_SEALED_CHARS:
            trace("reply_ignored", host_id=host_id, outcome="reply_too_long")
            reply_text = None
        error = message.get("error") if isinstance(message.get("error"), str) else None
        if error is not None and len(error) > 240:
            error = error[:240]
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
            self._expire_devices_locked(now)
            self._retain_locked(now)
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

    def redeem(self, code: str, label: str, client: str = "") -> tuple[int, dict]:
        if not isinstance(code, str) or not isinstance(label, str):
            return 400, {"error": "invalid_pairing"}
        label = label.strip()[:40] or "phone"
        code_hash = _hash(code.strip().upper())
        with self.lock:
            if self._pair_blocked_locked(client):
                trace("pairing_rejected", outcome="rate_limited")
                return 429, {"error": "rate_limited", "message": "Too many pairing attempts. Wait and try again."}
            pairing_id = self.code_index.get(code_hash)
            pairing = self.pairings.get(pairing_id) if pairing_id else None
            if pairing is None or pairing.status not in {"open", "pending_desktop"}:
                self._note_pair_fail_locked(client)
                trace("pairing_rejected", outcome="invalid_code")
                return 404, {"error": "invalid_code", "message": INVALID_CODE_MESSAGE}
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
                pairing.session_token = None
                pairing.token_consumed = True
                return 401, {"error": "revoked"}, None
            body["device_id"] = device.device_id
            # The pairing id is logged and is not a second credential. The
            # first confirmed poll mints the cookie. Further polls repeat that
            # same cookie until the phone presents it, then they stop.
            if pairing.token_consumed:
                return 200, body, None
            if pairing.session_token:
                return 200, body, pairing.session_token
            token = secrets.token_urlsafe(32)
            note_secret(token)
            digest = _hash(token)
            stale = [hashed for hashed, bound in self.sessions.items() if bound == device.device_id]
            for hashed in stale:
                self.sessions.pop(hashed, None)
            self.sessions[digest] = device.device_id
            device.token_hash = digest
            pairing.session_token = token
            self._persist_device_locked(device)
            trace("session_issued", host_id=device.host_id, device_id=device.device_id, pairing_id=pairing.pairing_id, outcome="confirmed")
            return 200, body, token

    def device_for_token(self, token: str | None) -> Device | None:
        if not token:
            return None
        digest = _hash(token)
        with self.lock:
            self._expire_devices_locked(self.now())
            device_id = self.sessions.get(digest)
            device = self.devices.get(device_id) if device_id else None
            if device is None or device.revoked:
                return None
            if not secret_equal(device.token_hash, digest):
                return None
            device.last_seen = self.now()
            self._persist_device_locked(device)
            for pairing in self.pairings.values():
                if pairing.device_id == device.device_id:
                    pairing.session_token = None
                    pairing.token_consumed = True
            return device

    def _prune_fails_locked(self, key: str, now: float) -> list[float]:
        hits = [stamp for stamp in self._pair_fails.get(key, []) if now - stamp < self.pair_fail_window]
        if hits:
            self._pair_fails[key] = hits
        else:
            self._pair_fails.pop(key, None)
        return hits

    def _pair_blocked_locked(self, client: str) -> bool:
        if len(self._pair_fails) > 4096:
            return True
        now = self.now()
        client_key = client or "unknown"
        if len(self._prune_fails_locked(client_key, now)) >= self.pair_fail_limit:
            return True
        return len(self._prune_fails_locked("*", now)) >= self.pair_fail_global

    def _note_pair_fail_locked(self, client: str) -> None:
        now = self.now()
        for key in (client or "unknown", "*"):
            hits = self._prune_fails_locked(key, now)
            hits.append(now)
            self._pair_fails[key] = hits

    def note_reconcile(self, host_id: str, claimed: object) -> None:
        """Log how the desktop's list differs from the registry.

        Membership stays the registry. The desktop drops ids the registry
        does not have and adopts ids it did not list.
        """
        claimed_ids: set[str] = set()
        if isinstance(claimed, list):
            for item in claimed[:32]:
                if not isinstance(item, dict):
                    continue
                device_id = item.get("device_id")
                if isinstance(device_id, str) and device_id:
                    claimed_ids.add(device_id)
        with self.lock:
            live = {
                device.device_id
                for device in self.devices.values()
                if device.host_id == host_id and not device.revoked
            }
        trace(
            "registry_reconcile",
            host_id=host_id,
            outcome="reconciled",
            status=f"adopted={len(live - claimed_ids)},dropped={len(claimed_ids - live)}",
        )

    def _persist_host_locked(self, host_id: str, token_hash: str) -> None:
        if self.registry is not None:
            self.registry.upsert_host(host_id, token_hash, self.now())

    def _retain_locked(self, now: float) -> None:
        if self.registry is None:
            return
        report = self.registry.retain(now)
        for host_id in report["hosts"]:
            if host_id not in self.links:
                self.hosts.pop(host_id, None)
        for device_id in report["devices"]:
            device = self.devices.pop(device_id, None)
            if device is None:
                continue
            dead = [token for token, bound in self.sessions.items() if bound == device_id]
            for token in dead:
                self.sessions.pop(token, None)

    def _persist_device_locked(self, device: Device) -> None:
        if self.registry is not None:
            self.registry.upsert_device(device)

    def _expire_devices_locked(self, now: float) -> None:
        stale = [
            device.device_id
            for device in self.devices.values()
            if now - device.last_seen > DEVICE_IDLE_SECONDS
        ]
        for device_id in stale:
            device = self.devices.pop(device_id)
            dead = [token for token, bound in self.sessions.items() if bound == device_id]
            for token in dead:
                self.sessions.pop(token, None)
            if self.registry is not None:
                self.registry.delete_device(device_id)
            trace("device_expired", host_id=device.host_id, device_id=device_id, outcome="idle")

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
        text = envelope["content"][0]["body"]
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
            inflight = sum(
                1
                for item in self.deliveries.values()
                if item.device_id == device.device_id and item.body is not None
            )
            if inflight >= MAX_INFLIGHT_PER_DEVICE:
                trace("turn_rejected", device_id=device.device_id, host_id=device.host_id, outcome="too_many_pending")
                return 429, {"error": "too_many_pending"}
            if not self._online_locked(device.host_id):
                trace("turn_refused", host_id=device.host_id, device_id=device.device_id, request_id=request_id, bytes=len(text), outcome="desktop_offline")
                return 409, {"error": "desktop_offline", "message": "desktop offline"}
            delivery_id = "del-" + secrets.token_hex(8)
            conversation_id = f"phone-{device.host_id}"
            join_desktop = envelope.get("join_desktop") is True
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
            if self.registry is not None:
                self.registry.note_envelope(request_id, device.host_id, self.now(), len(text))
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
                        "join_desktop": join_desktop,
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

    def ask_desktop(self, host_id: str, message: dict, timeout: float = 8.0) -> dict | None:
        """Push one request to the desktop socket and wait for needs_you_result."""
        request_id = secrets.token_hex(8)
        event = threading.Event()
        with self.lock:
            link = self.links.get(host_id)
            if link is None or not link.alive:
                trace("needs_you", host_id=host_id, outcome="desktop_offline")
                return None
            self.asks[request_id] = {"event": event, "body": None}
            payload = dict(message)
            payload["request_id"] = request_id
            link.push(payload)
        if not event.wait(timeout):
            with self.lock:
                self.asks.pop(request_id, None)
            trace("needs_you", host_id=host_id, outcome="timeout")
            return None
        with self.lock:
            slot = self.asks.pop(request_id, None)
        body = slot.get("body") if isinstance(slot, dict) else None
        return body if isinstance(body, dict) else None

    def complete_ask(self, message: dict) -> None:
        request_id = str(message.get("request_id", ""))
        with self.lock:
            slot = self.asks.get(request_id)
            if slot is None:
                return
            slot["body"] = message
            slot["event"].set()

    def needs_you_list(self, device: Device) -> tuple[int, dict]:
        body = self.ask_desktop(device.host_id, {"type": "needs_you_list"})
        if body is None:
            return 503, {"ok": False, "error": "desktop_offline"}
        items = body.get("items") if isinstance(body.get("items"), list) else []
        trace("needs_you", host_id=device.host_id, device_id=device.device_id, outcome="listed", status=str(len(items)))
        return 200, {"ok": True, "items": items}

    def needs_you_resolve(self, device: Device, item_id: str, decision: str) -> tuple[int, dict]:
        if decision not in {"approve", "deny"}:
            return 400, {"ok": False, "error": "invalid_decision"}
        body = self.ask_desktop(
            device.host_id,
            {"type": "needs_you_resolve", "id": item_id, "approve": decision == "approve"},
        )
        if body is None:
            return 503, {"ok": False, "error": "desktop_offline"}
        error = body.get("error")
        if error in {"waiting on desktop", "irreversible"}:
            trace(
                "needs_you",
                host_id=device.host_id,
                device_id=device.device_id,
                outcome="waiting_on_desktop",
            )
            return 403, {"ok": False, "error": "waiting_on_desktop", "message": "waiting on desktop"}
        if body.get("ok") is False:
            trace("needs_you", host_id=device.host_id, device_id=device.device_id, outcome="refused")
            return 409, {"ok": False, "error": error or "not_resolved"}
        trace("needs_you", host_id=device.host_id, device_id=device.device_id, outcome=decision)
        return 200, {
            "ok": True,
            "id": body.get("id", item_id),
            "resolution": body.get("resolution"),
            "already": bool(body.get("already")),
        }

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
        return "content_must_be_one_sealed_part"
    part = content[0]
    if part.get("type") == "text":
        return "plaintext_rejected"
    if part.get("type") != "sealed" or not isinstance(part.get("body"), str):
        return "content_must_be_one_sealed_part"
    sealed = part["body"]
    if not sealed or len(sealed) > MAX_SEALED_CHARS:
        return "invalid_sealed"
    if any(ch not in "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_=" for ch in sealed):
        return "invalid_sealed"
    return None


PAIR_PAGE = """<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="referrer" content="no-referrer">
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
    <p id="key-fingerprint"></p>
    <p id="fingerprint"></p>
    <p>The desktop must confirm this phone. A code alone does not pair it.</p>
    <p>Message bodies are sealed. This relay only routes the envelope. Compare the key fingerprint with the desktop. It comes from the QR fragment, which this server never receives.</p>
  </main>
  <script type="module">
    import { keyFingerprint, rememberDesktopKey } from "/e2e.js";
    const status = document.getElementById("pair-status");
    const fingerprint = document.getElementById("fingerprint");
    const keyLine = document.getElementById("key-fingerprint");
    const desktopKey = rememberDesktopKey();
    if (desktopKey) keyLine.textContent = "Key fingerprint " + keyFingerprint(desktopKey);
    else keyLine.textContent = "Open the QR from the desktop. This page has no key, so it will not send messages.";
    document.getElementById("pair-form").addEventListener("submit", async (event) => {
      event.preventDefault();
      if (!desktopKey) { status.textContent = "Open the QR from the desktop first"; return; }
      const code = document.getElementById("code").value.trim();
      const label = document.getElementById("label").value.trim() || "phone";
      status.textContent = "Waiting for the desktop to confirm…";
      const redeem = await fetch("/api/pair", {method:"POST", credentials:"same-origin", headers:{"Content-Type":"application/json"}, body: JSON.stringify({code, label})});
      const body = await redeem.json();
      if (!redeem.ok) { status.textContent = body.message || body.error || "Pairing failed"; return; }
      fingerprint.textContent = "Device fingerprint " + (body.fingerprint || "");
      const id = body.pairing_id;
      const timer = setInterval(async () => {
        const res = await fetch("/api/pair/" + encodeURIComponent(id), {credentials:"same-origin"});
        const poll = await res.json();
        if (poll.status === "confirmed") { clearInterval(timer); location.replace("/" + location.hash); }
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
            if parsed.scheme not in {"http", "https"}:
                return False
            return parsed.netloc == host

        def _post_allowed(self) -> bool:
            site = (self.headers.get("Sec-Fetch-Site") or "").lower()
            if site == "cross-site":
                return False
            return self._origin_ok()

        def _security_headers(self) -> None:
            self.send_header("X-Content-Type-Options", "nosniff")
            self.send_header("Referrer-Policy", "no-referrer")
            self.send_header("Cache-Control", "no-store")

        def _json(self, status: int, payload: dict, cookie: str | None = None) -> None:
            data = json.dumps(payload).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Connection", "close")
            self._security_headers()
            self.close_connection = True
            if cookie is not None:
                self.send_header("Set-Cookie", cookie_header(cookie, secure_cookie))
            self.end_headers()
            self.wfile.write(data)

        def _read_body(self) -> bytes | None:
            raw_length = self.headers.get("Content-Length", "0").strip()
            try:
                length = int(raw_length)
            except ValueError:
                self._json(400, {"error": "invalid_content_length"})
                return None
            if length < 0:
                self._json(400, {"error": "invalid_content_length"})
                return None
            if length > MAX_BODY_BYTES:
                self._json(413, {"error": "body_too_large"})
                return None
            if length == 0:
                return b""
            raw = self.rfile.read(length)
            if len(raw) != length:
                self._json(400, {"error": "truncated_body"})
                return None
            return raw

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
            if url.path == "/api/needs-you":
                device = self._device()
                if device is None:
                    self._json(401, {"error": "unauthorized"})
                    return
                self._json(*relay.needs_you_list(device))
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
                    self.send_header("Content-Length", str(len(data)))
                    self.send_header("Connection", "close")
                    self._security_headers()
                    self.close_connection = True
                    self.end_headers()
                    self.wfile.write(data)
                    return
            self._static(url.path)

        def do_POST(self) -> None:  # noqa: N802
            if not self._post_allowed():
                trace("http", outcome="bad_origin")
                self._json(403, {"error": "bad_origin"})
                return
            url = urlparse(self.path)
            parts = [part for part in url.path.split("/") if part]
            raw = self._read_body()
            if raw is None:
                return
            if parts == ["api", "pair"]:
                try:
                    body = json.loads(raw or b"null")
                except json.JSONDecodeError:
                    self._json(400, {"error": "invalid_json"})
                    return
                if not isinstance(body, dict):
                    self._json(400, {"error": "invalid_pairing"})
                    return
                client = self.client_address[0] if self.client_address else ""
                self._json(*relay.redeem(str(body.get("code", "")), str(body.get("label", "phone")), client))
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
            if len(parts) == 4 and parts[:2] == ["api", "needs-you"] and parts[3] == "resolve":
                try:
                    body = json.loads(raw or b"null")
                except json.JSONDecodeError:
                    self._json(400, {"error": "invalid_json"})
                    return
                decision = body.get("decision") if isinstance(body, dict) else None
                if decision not in {"approve", "deny"}:
                    self._json(400, {"error": "invalid_decision"})
                    return
                self._json(*relay.needs_you_resolve(device, parts[2], str(decision)))
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
            self.send_header("Connection", "close")
            self._security_headers()
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


_ws_lock = threading.Lock()
_ws_open = 0


def _ws_enter() -> bool:
    global _ws_open
    with _ws_lock:
        if _ws_open >= MAX_DESKTOP_SOCKETS:
            return False
        _ws_open += 1
        return True


def _ws_leave() -> None:
    global _ws_open
    with _ws_lock:
        _ws_open = max(0, _ws_open - 1)


def serve_desktop_socket(sock: socket.socket, relay: Relay) -> None:
    if not _ws_enter():
        trace("hello_rejected", outcome="too_many_sockets")
        try:
            ws_send(sock, json.dumps({"type": "error", "error": "unavailable"}).encode())
        except OSError:
            pass
        try:
            sock.close()
        except OSError:
            pass
        return
    link: DesktopLink | None = None
    host_id = ""
    started = time.monotonic()
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
                if link is None and time.monotonic() - started > HELLO_DEADLINE_SECONDS:
                    trace("hello_rejected", outcome="timeout")
                    break
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
                version = message.get("protocol")
                if isinstance(version, bool) or not isinstance(version, int) or version != PROTOCOL_VERSION:
                    trace("hello_rejected", outcome="protocol_mismatch")
                    ws_send(
                        sock,
                        json.dumps(
                            {
                                "type": "error",
                                "error": "protocol_mismatch",
                                "protocol": PROTOCOL_VERSION,
                                "message": PROTOCOL_MISMATCH,
                            }
                        ).encode(),
                    )
                    break
                host_id = str(message.get("host_id", ""))
                token = str(message.get("host_token", ""))
                label = str(message.get("host_label", "desktop"))
                link = relay.hello(host_id, token, label)
                if link is None:
                    ws_send(sock, json.dumps({"type": "error", "error": "hello_rejected"}).encode())
                    break
                relay.note_reconcile(host_id, message.get("devices"))
                ws_send(
                    sock,
                    json.dumps(
                        {
                            "type": "hello_ok",
                            "protocol": PROTOCOL_VERSION,
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
        _ws_leave()
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
    if kind == "needs_you_result":
        relay.complete_ask(message)
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
    origin = args.public_origin or f"http://{args.host}:{args.port}"
    secure = cookies_should_be_secure(args.host, origin)
    state_path = os.environ.get("RELAY_STATE_PATH", "").strip()
    try:
        relay = Relay(
            undelivered_ttl=args.undelivered_ttl,
            public_origin=origin,
            state_path=state_path or None,
        )
    except (OSError, sqlite3.Error) as exc:
        log.error("relay registry failed to open outcome=%s", exc.__class__.__name__)
        raise SystemExit(1) from exc
    try:
        server = build_server(args.host, args.port, relay, static_dir, secure_cookie=secure)
    except OSError as exc:
        log.error("relay failed to bind %s:%s: %s", args.host, args.port, exc.__class__.__name__)
        raise SystemExit(1) from exc
    stop = threading.Event()
    threading.Thread(target=_reaper, args=(relay, stop), name="relay-reaper", daemon=True).start()
    trace(
        "relay_listening",
        outcome="secure_cookie" if secure else "local_cookie",
        status=str(args.port),
    )
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
