"""In-tenant chess-opponent stand-in.

The process speaks the phone relay as a desktop host. Every tool call is
admitted by the Plexi permission gate inside this container. This file does
not approve calls and it does not read a host secret.
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import socket
import subprocess
import sys
import time
from pathlib import Path
from urllib.parse import urlparse

PROFILE = Path("/tenant/profile")
WORKSPACE = Path("/tenant")
EFFECTS = WORKSPACE / "effects"
PAIRING = WORKSPACE / "pairing.json"
DECISION = WORKSPACE / "pairing-decision.json"
NEEDS = WORKSPACE / "needs-you.json"
VAULT_KEY = WORKSPACE / "vault" / "model"
AGENT_MD = Path("/opt/house/agent/AGENT.md")


def log(event: str, **fields: str) -> None:
    parts = [f"house agent: {event}"]
    for key, value in fields.items():
        if any(ch.isspace() for ch in value):
            continue
        parts.append(f"{key}={value}")
    print(" ".join(parts), file=sys.stderr, flush=True)


def plexi(*args: str) -> subprocess.CompletedProcess[str]:
    binary = os.environ["PLEXI_BIN"]
    return subprocess.run(
        [binary, "cloud", "agent", *args],
        check=False,
        capture_output=True,
        text=True,
    )


def admit(tool: str, payload: dict) -> tuple[int, str]:
    proc = plexi(
        "admit",
        "--tenant-profile",
        str(PROFILE),
        "--workspace",
        str(WORKSPACE),
        "--actor",
        "chess-opponent",
        "--tool",
        tool,
        "--input",
        json.dumps(payload, separators=(",", ":")),
    )
    decision = "error"
    for line in proc.stdout.splitlines():
        try:
            body = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(body, dict) and isinstance(body.get("decision"), str):
            decision = body["decision"]
    if proc.returncode not in {0, 3, 4}:
        log("admit_failed", tool=tool, code=str(proc.returncode))
    return proc.returncode, decision


def safe_tool(name: str) -> str:
    cleaned = "".join(ch if ch.isalnum() or ch in "._-" else "_" for ch in name)
    if not cleaned or cleaned.startswith(".") or "/" in cleaned or ".." in cleaned:
        raise ValueError("unsafe tool name")
    return cleaned


def execute(tool: str) -> None:
    EFFECTS.mkdir(parents=True, exist_ok=True)
    (EFFECTS / safe_tool(tool)).write_text("executed\n", encoding="utf-8")


def handle_text(text: str) -> dict:
    try:
        body = json.loads(text)
    except json.JSONDecodeError:
        body = {"kind": "message", "text": text}
    if not isinstance(body, dict):
        body = {"kind": "message", "text": text}
    if body.get("kind") == "tool":
        tool = str(body.get("name") or "")
        payload = body.get("input") if isinstance(body.get("input"), dict) else {}
        code, decision = admit(tool, payload)
        if code == 0 and decision == "allow":
            execute(tool)
            return {"state": "succeeded", "reply": "ok", "decision": decision, "executed": True}
        error = "permission_denied" if decision == "deny" else "waiting_for_permission"
        state = "waiting_for_permission" if decision == "ask" else "failed"
        return {
            "state": state,
            "reply": f"blocked {error}",
            "error": error,
            "decision": decision,
            "executed": False,
        }
    if body.get("kind") == "model":
        code, decision = admit("model.complete", {})
        if code != 0 or decision != "allow":
            error = "permission_denied" if decision == "deny" else "waiting_for_permission"
            state = "waiting_for_permission" if decision == "ask" else "failed"
            return {
                "state": state,
                "reply": f"blocked {error}",
                "error": error,
                "decision": decision,
                "executed": False,
            }
        key = model_key()
        if not key:
            return {
                "state": "failed",
                "reply": "blocked vault_revoked",
                "error": "vault_revoked",
                "decision": decision,
                "executed": False,
            }
        digest = hashlib.sha256(key.encode()).hexdigest()[:12]
        return {
            "state": "succeeded",
            "reply": f"model {digest}",
            "decision": decision,
            "executed": True,
        }
    payload = {"text": str(body.get("text") or text)}
    code, decision = admit("agent.turn", payload)
    if code == 0 and decision == "allow":
        execute("agent.turn")
        return {
            "state": "succeeded",
            "reply": "Chess Opponent ready.",
            "decision": decision,
            "executed": True,
        }
    return {
        "state": "failed",
        "reply": "blocked",
        "error": "permission_denied",
        "decision": decision,
        "executed": False,
    }


def exact(sock: socket.socket, count: int) -> bytes:
    buf = b""
    while len(buf) < count:
        chunk = sock.recv(count - len(buf))
        if not chunk:
            raise ConnectionError("socket closed")
        buf += chunk
    return buf


def ws_send(sock: socket.socket, payload: bytes, opcode: int = 0x1) -> None:
    mask = os.urandom(4)
    masked = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
    length = len(payload)
    if length < 126:
        header = bytes([0x80 | opcode, 0x80 | length])
    elif length < 65536:
        header = bytes([0x80 | opcode, 0x80 | 126]) + length.to_bytes(2, "big")
    else:
        header = bytes([0x80 | opcode, 0x80 | 127]) + length.to_bytes(8, "big")
    sock.sendall(header + mask + masked)


def ws_recv(sock: socket.socket) -> tuple[int, bytes]:
    head = exact(sock, 2)
    opcode = head[0] & 0x0F
    length = head[1] & 0x7F
    if length == 126:
        length = int.from_bytes(exact(sock, 2), "big")
    elif length == 127:
        length = int.from_bytes(exact(sock, 8), "big")
    payload = exact(sock, length) if length else b""
    return opcode, payload


def connect() -> socket.socket:
    parsed = urlparse(os.environ["RELAY_URL"])
    host = parsed.hostname or "relay"
    port = parsed.port or 8080
    deadline = time.monotonic() + 20
    while True:
        try:
            sock = socket.create_connection((host, port), timeout=3)
            break
        except OSError:
            if time.monotonic() >= deadline:
                raise
            time.sleep(0.25)
    key = base64.b64encode(os.urandom(16)).decode()
    sock.sendall(
        (
            "GET /v1/desktop HTTP/1.1\r\n"
            f"Host: {host}:{port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        ).encode()
    )
    raw = b""
    while b"\r\n\r\n" not in raw:
        chunk = sock.recv(4096)
        if not chunk:
            raise ConnectionError("relay closed during upgrade")
        raw += chunk
    if b"101" not in raw.split(b"\r\n", 1)[0]:
        raise ConnectionError("websocket upgrade failed")
    sock.settimeout(None)
    return sock


def send_json(sock: socket.socket, body: dict) -> dict:
    ws_send(sock, json.dumps(body).encode())
    while True:
        opcode, payload = ws_recv(sock)
        if opcode == 0x9:
            ws_send(sock, payload, opcode=0xA)
            continue
        if opcode != 0x1:
            raise ConnectionError(f"unexpected frame {opcode}")
        parsed = json.loads(payload.decode())
        if not isinstance(parsed, dict):
            raise ConnectionError("relay frame was not an object")
        return parsed


def model_key() -> str:
    try:
        return VAULT_KEY.read_text(encoding="utf-8").strip()
    except OSError:
        return ""


def write_secret(path: Path, body: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as handle:
        json.dump(body, handle)


def read_decision() -> dict | None:
    try:
        body = json.loads(DECISION.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    return body if isinstance(body, dict) else None


def apply_decision(sock: socket.socket, pending_id: str, tenant: str) -> bool:
    body = read_decision()
    if not body or body.get("pairing_id") != pending_id:
        return False
    choice = body.get("decision")
    if choice == "approve":
        reply = send_json(sock, {"type": "pair_confirm", "pairing_id": pending_id})
        if reply.get("type") != "pair_confirmed":
            log("pair_approve_rejected", tenant=tenant)
            return False
        DECISION.unlink(missing_ok=True)
        NEEDS.unlink(missing_ok=True)
        log("pair_approved", tenant=tenant)
        return True
    if choice == "deny":
        send_json(sock, {"type": "pair_deny", "pairing_id": pending_id})
        DECISION.unlink(missing_ok=True)
        NEEDS.unlink(missing_ok=True)
        log("pair_denied", tenant=tenant)
        return True
    return False


def prepare() -> None:
    if not AGENT_MD.is_file():
        raise SystemExit("chess-opponent package is missing from the image")
    WORKSPACE.joinpath("home").mkdir(parents=True, exist_ok=True)
    PROFILE.mkdir(parents=True, exist_ok=True)
    EFFECTS.mkdir(parents=True, exist_ok=True)
    grant = plexi(
        "grant",
        "--tenant-profile",
        str(PROFILE),
        "--workspace",
        str(WORKSPACE),
    )
    if grant.returncode != 0:
        log("grant_failed", code=str(grant.returncode))
        raise SystemExit(grant.stderr.strip() or "house grant failed")
    retain = plexi("retain", "--tenant-profile", str(PROFILE))
    if retain.returncode != 0:
        log("retain_failed", code=str(retain.returncode))
        raise SystemExit(retain.stderr.strip() or "house retain failed")


def serve() -> None:
    tenant = os.environ.get("TENANT_ID", "local")
    token = base64.b64encode(os.urandom(24)).decode()
    sock = connect()
    hello = send_json(
        sock,
        {
            "type": "hello",
            "host_id": f"house-{tenant}",
            "host_token": token,
            "host_label": "chess-opponent",
            "protocol": 1,
        },
    )
    if hello.get("type") != "hello_ok":
        raise SystemExit("relay hello was rejected")
    issued = send_json(sock, {"type": "pair_start"})
    if issued.get("type") != "pair_code" or not issued.get("code"):
        raise SystemExit("relay did not issue a pairing code")
    fd = os.open(PAIRING, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as handle:
        json.dump({"pairing_id": issued.get("pairing_id", ""), "code": issued["code"]}, handle)
    log("paired_waiting", tenant=tenant)
    sock.settimeout(0.5)
    waiting = ""
    while True:
        if waiting and apply_decision(sock, waiting, tenant):
            waiting = ""
        try:
            opcode, payload = ws_recv(sock)
        except TimeoutError:
            continue
        if opcode == 0x8:
            return
        if opcode == 0x9:
            ws_send(sock, payload, opcode=0xA)
            continue
        if opcode != 0x1:
            continue
        message = json.loads(payload.decode())
        if not isinstance(message, dict):
            continue
        kind = message.get("type")
        if kind == "pair_pending":
            pending_id = str(message.get("pairing_id") or "")
            write_secret(
                NEEDS,
                {
                    "needs_you": [
                        {
                            "kind": "pair",
                            "tenant": tenant,
                            "pairing_id": pending_id,
                            "device_label": str(message.get("device_label") or ""),
                            "fingerprint": str(message.get("fingerprint") or ""),
                        }
                    ]
                },
            )
            log("needs_you", tenant=tenant, pairing_id=pending_id)
            waiting = pending_id
            if waiting and apply_decision(sock, waiting, tenant):
                waiting = ""
            continue
        if kind != "deliver":
            continue
        result = handle_text(str(message.get("text") or ""))
        reply = {
            "type": "reply",
            "delivery_id": message.get("delivery_id"),
            "request_id": message.get("request_id"),
            "state": result["state"],
            "reply": result["reply"],
        }
        if result.get("error"):
            reply["error"] = result["error"]
        ws_send(sock, json.dumps(reply).encode())
        log(
            "turn",
            tenant=tenant,
            decision=str(result.get("decision") or ""),
            executed="yes" if result.get("executed") else "no",
        )


def main() -> None:
    prepare()
    serve()


if __name__ == "__main__":
    try:
        main()
    except (OSError, ConnectionError, json.JSONDecodeError, subprocess.SubprocessError, ValueError) as exc:
        log("stopped", outcome=exc.__class__.__name__)
        raise SystemExit(1) from exc
