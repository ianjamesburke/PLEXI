"""Phone-desktop message seals.

The relay process does not import this module. It only forwards the opaque
body. A malicious relay never sees the desktop public key: that key travels in
the URL fragment (``#k=``), which browsers do not send.

X25519, HKDF-SHA256, and ChaCha20-Poly1305 come from the ``cryptography``
package. This file does not implement those primitives.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey, X25519PublicKey
from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
from cryptography.hazmat.primitives.kdf.hkdf import HKDF

MAGIC = b"P1"
INFO = b"plexi-phone-e2e-v1"
KIND_HANDSHAKE = 0x01
KIND_DATA = 0x02
DIR_PHONE = 1
DIR_DESKTOP = 2
PUB_LEN = 32


class SealError(Exception):
    def __init__(self, outcome: str) -> None:
        super().__init__(outcome)
        self.outcome = outcome


def b64u_encode(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).decode().rstrip("=")


def b64u_decode(text: str) -> bytes:
    pad = "=" * (-len(text) % 4)
    return base64.urlsafe_b64decode(text + pad)


def key_fingerprint(public: bytes) -> str:
    return hashlib.sha256(public).hexdigest()[:16]


def _raw_private(key: X25519PrivateKey) -> bytes:
    return key.private_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PrivateFormat.Raw,
        encryption_algorithm=serialization.NoEncryption(),
    )


def _raw_public(key: X25519PrivateKey) -> bytes:
    return key.public_key().public_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PublicFormat.Raw,
    )


def public_from_private(private: bytes) -> bytes:
    return _raw_public(X25519PrivateKey.from_private_bytes(private))


def shared_secret(private: bytes, peer_public: bytes) -> bytes:
    key = X25519PrivateKey.from_private_bytes(private)
    peer = X25519PublicKey.from_public_bytes(peer_public)
    return key.exchange(peer)


def session_key(shared: bytes, desktop_pub: bytes, phone_pub: bytes) -> bytes:
    return HKDF(
        algorithm=hashes.SHA256(),
        length=32,
        salt=desktop_pub + phone_pub,
        info=INFO,
    ).derive(shared)


def _nonce(direction: int, counter: int) -> bytes:
    return bytes((direction, 0, 0, 0)) + counter.to_bytes(8, "little")


def _aad_handshake(phone_pub: bytes, desktop_pub: bytes) -> bytes:
    return MAGIC + bytes((KIND_HANDSHAKE,)) + phone_pub + desktop_pub


def _aad_data(direction: int, counter: int, request_id: str) -> bytes:
    return MAGIC + bytes((KIND_DATA, direction)) + counter.to_bytes(8, "little") + request_id.encode()


def plain_bytes(text: str, join_desktop: bool, error: str | None = None) -> bytes:
    payload: dict = {"v": 1, "text": text, "join_desktop": bool(join_desktop)}
    if error:
        payload["error"] = error
    return json.dumps(payload, separators=(",", ":")).encode()


def parse_plain(raw: bytes) -> dict:
    try:
        payload = json.loads(raw.decode())
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise SealError("tamper") from exc
    if not isinstance(payload, dict) or payload.get("v") != 1 or not isinstance(payload.get("text"), str):
        raise SealError("tamper")
    if not isinstance(payload.get("join_desktop"), bool):
        raise SealError("tamper")
    error = payload.get("error")
    if error is not None and not isinstance(error, str):
        raise SealError("tamper")
    return payload


def _seal(key: bytes, nonce: bytes, plaintext: bytes, aad: bytes) -> bytes:
    return ChaCha20Poly1305(key).encrypt(nonce, plaintext, aad)


def _open(key: bytes, nonce: bytes, ciphertext: bytes, aad: bytes) -> bytes:
    try:
        return ChaCha20Poly1305(key).decrypt(nonce, ciphertext, aad)
    except Exception as exc:
        raise SealError("tamper") from exc


def seal_handshake(
    key: bytes, phone_pub: bytes, desktop_pub: bytes, text: str, join_desktop: bool
) -> bytes:
    body = _seal(
        key,
        _nonce(DIR_PHONE, 0),
        plain_bytes(text, join_desktop),
        _aad_handshake(phone_pub, desktop_pub),
    )
    return MAGIC + bytes((KIND_HANDSHAKE,)) + phone_pub + body


def seal_data(
    key: bytes, direction: int, counter: int, request_id: str, text: str, join_desktop: bool, error: str | None = None
) -> bytes:
    if counter == 0:
        raise SealError("replay")
    body = _seal(
        key,
        _nonce(direction, counter),
        plain_bytes(text, join_desktop, error),
        _aad_data(direction, counter, request_id),
    )
    return MAGIC + bytes((KIND_DATA, direction)) + counter.to_bytes(8, "little") + body


def open_handshake(desktop_private: bytes, desktop_pub: bytes, raw: bytes) -> tuple[dict, bytes, bytes]:
    if len(raw) < 3 + PUB_LEN + 16 or raw[:2] != MAGIC or raw[2] != KIND_HANDSHAKE:
        raise SealError("malformed")
    phone_pub = raw[3 : 3 + PUB_LEN]
    shared = shared_secret(desktop_private, phone_pub)
    key = session_key(shared, desktop_pub, phone_pub)
    plaintext = _open(key, _nonce(DIR_PHONE, 0), raw[3 + PUB_LEN :], _aad_handshake(phone_pub, desktop_pub))
    return parse_plain(plaintext), key, phone_pub


def open_data(key: bytes, raw: bytes, request_id: str, last_counter: int, expect_direction: int) -> tuple[dict, int]:
    if len(raw) < 2 + 1 + 1 + 8 + 16 or raw[:2] != MAGIC or raw[2] != KIND_DATA:
        raise SealError("malformed")
    direction = raw[3]
    counter = int.from_bytes(raw[4:12], "little")
    if direction != expect_direction:
        raise SealError("tamper")
    if counter == 0 or counter <= last_counter:
        raise SealError("replay")
    plaintext = _open(key, _nonce(direction, counter), raw[12:], _aad_data(direction, counter, request_id))
    return parse_plain(plaintext), counter


class PhoneState:
    """One phone's view of a desktop public key."""

    def __init__(self, desktop_pub: bytes, private: bytes | None = None) -> None:
        self.desktop_pub = desktop_pub
        if private is None:
            generated = X25519PrivateKey.generate()
            self.private = _raw_private(generated)
            self.public = _raw_public(generated)
        else:
            self.private = private
            self.public = public_from_private(private)
        self.key = session_key(shared_secret(self.private, desktop_pub), desktop_pub, self.public)
        self.send = 0
        self.recv = 0
        self.handshook = False
        self.opened: dict[str, str] = {}

    def seal_turn(self, text: str, request_id: str, join_desktop: bool = False) -> str:
        if not self.handshook:
            raw = seal_handshake(self.key, self.public, self.desktop_pub, text, join_desktop)
            self.handshook = True
            return b64u_encode(raw)
        self.send += 1
        raw = seal_data(self.key, DIR_PHONE, self.send, request_id, text, join_desktop)
        return b64u_encode(raw)

    def open_reply(self, body: str, request_id: str) -> dict:
        cached = self.opened.get(request_id)
        if cached is not None:
            return {"text": cached, "join_desktop": False}
        raw = b64u_decode(body)
        payload, counter = open_data(self.key, raw, request_id, self.recv, DIR_DESKTOP)
        self.recv = counter
        self.opened[request_id] = payload["text"]
        return payload

    def to_json(self) -> dict:
        return {
            "desktop_pub": b64u_encode(self.desktop_pub),
            "phone_priv": b64u_encode(self.private),
            "send": self.send,
            "recv": self.recv,
            "handshook": self.handshook,
            "opened": self.opened,
        }

    @classmethod
    def from_json(cls, payload: dict) -> PhoneState:
        phone = cls(b64u_decode(payload["desktop_pub"]), b64u_decode(payload["phone_priv"]))
        phone.send = int(payload.get("send") or 0)
        phone.recv = int(payload.get("recv") or 0)
        phone.handshook = bool(payload.get("handshook"))
        opened = payload.get("opened") or {}
        phone.opened = {str(key): str(value) for key, value in opened.items()}
        return phone


def opaque_body(plaintext: str) -> str:
    """A real seal of ``plaintext`` under a throwaway key.

    The relay stores this string and must not be able to read ``plaintext``.
    """
    desk = X25519PrivateKey.generate()
    phone = PhoneState(_raw_public(desk))
    body = phone.seal_turn(plaintext, "opaque", False)
    if plaintext in body:
        raise SealError("plaintext_leaked")
    return body


def desktop_pub_from_status(status_path: str) -> bytes:
    payload = json.loads(Path(status_path).read_text())
    qr = payload.get("qr_url") or ""
    fragment = qr.split("#", 1)[1] if "#" in qr else ""
    token = ""
    for part in fragment.split("&"):
        if part.startswith("k="):
            token = part[2:]
            break
    if not token:
        raise SealError("missing_desktop_key")
    public = b64u_decode(token)
    if len(public) != PUB_LEN:
        raise SealError("malformed")
    return public


def load_phone(state_path: str, status_path: str | None) -> PhoneState:
    path = Path(state_path)
    if path.is_file():
        return PhoneState.from_json(json.loads(path.read_text()))
    if not status_path:
        raise SealError("missing_desktop_key")
    phone = PhoneState(desktop_pub_from_status(status_path))
    save_phone(path, phone)
    return phone


def save_phone(path: Path, phone: PhoneState) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(phone.to_json()))


def envelope(phone: PhoneState, request_id: str, text: str, join_desktop: bool = False) -> dict:
    return {
        "schema_version": 1,
        "request_id": request_id,
        "join_desktop": False,
        "content": [{"type": "sealed", "body": phone.seal_turn(text, request_id, join_desktop)}],
    }


def _cmd_seal(args: argparse.Namespace) -> int:
    phone = load_phone(args.state, args.status)
    payload = envelope(phone, args.request_id, args.text, args.join_desktop)
    save_phone(Path(args.state), phone)
    sys.stdout.write(json.dumps(payload))
    return 0


def _cmd_saw(args: argparse.Namespace) -> int:
    page = json.loads(sys.stdin.read() or "{}")
    phone = load_phone(args.state, None)
    needle = args.needle
    found = False
    for event in page.get("events") or []:
        if event.get("kind") != "assistant_reply" or event.get("request_id") != args.request_id:
            continue
        text = event.get("text") or ""
        try:
            opened = phone.open_reply(text, args.request_id)
        except SealError:
            continue
        if needle in (opened.get("text") or ""):
            found = True
    save_phone(Path(args.state), phone)
    return 0 if found else 1


def _cmd_handshake_for(args: argparse.Namespace) -> int:
    desktop_pub = b64u_decode(args.desktop_pub)
    phone = PhoneState(desktop_pub)
    request_id = "req-vector"
    text = "hello-vector"
    body = phone.seal_turn(text, request_id, False)
    data = phone.seal_turn("second-vector", request_id, True)
    sys.stdout.write(
        json.dumps(
            {
                "request_id": request_id,
                "handshake_request_id": request_id,
                "data_request_id": request_id,
                "handshake": body,
                "data": data,
                "plaintext": text,
                "data_plaintext": "second-vector",
                "data_join_desktop": True,
                "phone_pub": b64u_encode(phone.public),
                "phone_priv": b64u_encode(phone.private),
            }
        )
    )
    return 0


def _cmd_open_reply(args: argparse.Namespace) -> int:
    phone = PhoneState(b64u_decode(args.desktop_pub), b64u_decode(args.phone_priv))
    phone.handshook = True
    phone.send = 1
    opened = phone.open_reply(args.body, args.request_id)
    sys.stdout.write(json.dumps({"text": opened.get("text") or "", "error": opened.get("error")}))
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Seal or open a phone relay body")
    sub = parser.add_subparsers(dest="cmd", required=True)
    seal = sub.add_parser("seal")
    seal.add_argument("--status", required=True)
    seal.add_argument("--state", required=True)
    seal.add_argument("--request-id", required=True)
    seal.add_argument("--text", required=True)
    seal.add_argument("--join-desktop", action="store_true")
    saw = sub.add_parser("saw")
    saw.add_argument("--state", required=True)
    saw.add_argument("--request-id", required=True)
    saw.add_argument("--needle", required=True)
    handshake = sub.add_parser("handshake-for")
    handshake.add_argument("desktop_pub")
    open_reply = sub.add_parser("open-reply")
    open_reply.add_argument("--desktop-pub", required=True)
    open_reply.add_argument("--phone-priv", required=True)
    open_reply.add_argument("--request-id", required=True)
    open_reply.add_argument("--body", required=True)
    post = sub.add_parser("post")
    post.add_argument("--status", required=True)
    post.add_argument("--state", required=True)
    post.add_argument("--url", required=True)
    post.add_argument("--cookie", required=True)
    post.add_argument("--request-id", required=True)
    post.add_argument("--text", required=True)
    args = parser.parse_args(argv)
    if args.cmd == "seal":
        return _cmd_seal(args)
    if args.cmd == "saw":
        return _cmd_saw(args)
    if args.cmd == "handshake-for":
        return _cmd_handshake_for(args)
    if args.cmd == "open-reply":
        return _cmd_open_reply(args)
    if args.cmd == "post":
        phone = load_phone(args.state, args.status)
        payload = json.dumps(envelope(phone, args.request_id, args.text)).encode()
        save_phone(Path(args.state), phone)
        request = urllib.request.Request(
            args.url,
            data=payload,
            headers={"content-type": "application/json", "cookie": f"plexi_phone={args.cookie}"},
            method="POST",
        )
        try:
            with urllib.request.urlopen(request, timeout=10) as response:
                sys.stdout.write(str(response.status))
                Path("/tmp/relay-e2e-body").write_bytes(response.read())
        except urllib.error.HTTPError as exc:
            sys.stdout.write(str(exc.code))
            Path("/tmp/relay-e2e-body").write_bytes(exc.read())
        return 0
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
