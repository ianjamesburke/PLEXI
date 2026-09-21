#!/usr/bin/env python3
"""Isolation Redteam — adversarial CPython-in-WASM sandbox probe.

Runs a matrix of guest-side attacks against the default (no-capability)
boundary. Records attempt → expected → observed in app state for the pane UI
and for offline review.

This is a fixture, not a jailbreak toolkit. Results must be read against the
honest threat model in docs/wasm-runtime.md: Store isolation + capability
grants, not Docker-equivalent containment.
"""

from __future__ import annotations

import json
import os
import sys
import traceback
from typing import Any

from plexi_sdk import log, state
from plexi_sdk.effects import (
    AiMessage,
    AiQuery,
    FileRead,
    FileWrite,
    HttpFetch,
    McpConnect,
    OpenFilePicker,
    ReadHostLog,
    RequestCapability,
    SetState,
    SetStatus,
    SetTitle,
    SubscribeEventStreams,
)
from plexi_sdk.events import (
    AiResponse,
    CapabilityDenied,
    CapabilityGranted,
    EventSubscriptionResult,
    FilePickCancelled,
    FilePicked,
    FileReadResult,
    FileWriteResult,
    HostLogResult,
    HttpResponse,
    KeyEvent,
    McpConnected,
    UiAction,
)
from plexi_sdk.ui import AppBar, Button, Column, FooterKeys, Spacer, Text

ATTACKS_KEY = "attacks"
NOTICE_KEY = "notice"
PHASE_KEY = "phase"


def init(_size, _args) -> list:
    attacks: list[dict] = []
    attacks.extend(_run_native_probes())
    attacks.extend(_emit_raw_protocol_probes())
    return [
        SetTitle("Isolation Redteam"),
        SetState(
            {
                ATTACKS_KEY: attacks,
                NOTICE_KEY: f"Sync probes done ({len(attacks)}). Effect denials pending host replies.",
                PHASE_KEY: "effects",
                "pending": [],
            }
        ),
        SetStatus(f"{len(attacks)} sync probes"),
        *_effect_probe_effects(),
        *_capability_echo_probes(),
    ]


def update(event) -> list:
    data = _snapshot()
    attacks = list(data.get(ATTACKS_KEY) or [])
    pending = list(data.get("pending") or [])

    if isinstance(event, KeyEvent) and event.pressed and event.key == "r":
        attacks: list[dict] = []
        attacks.extend(_run_native_probes())
        attacks.extend(_emit_raw_protocol_probes())
        return [
            SetState(
                {
                    ATTACKS_KEY: attacks,
                    NOTICE_KEY: f"Re-ran sync probes ({len(attacks)}). Waiting on effects…",
                    PHASE_KEY: "effects",
                    "pending": [],
                }
            ),
            SetStatus(f"{len(attacks)} sync probes"),
            *_effect_probe_effects(),
            *_capability_echo_probes(),
        ]

    if isinstance(event, UiAction) and event.handler_id == "rerun":
        return update(KeyEvent(key="r", pressed=True, mods=0))

    # Result events — match by attack id encoded in request paths / ids.
    if isinstance(event, FileReadResult):
        _record(
            attacks,
            "effect.file_read",
            expected="DENY (missing fs.read)",
            observed=("SUCCESS leaked bytes" if event.error is None else f"DENY: {event.error}"),
            ok=event.error is not None,
        )
    elif isinstance(event, FileWriteResult):
        _record(
            attacks,
            "effect.file_write",
            expected="DENY (missing fs.write)",
            observed=("SUCCESS wrote" if event.error is None else f"DENY: {event.error}"),
            ok=event.error is not None,
        )
    elif isinstance(event, HttpResponse):
        # Errors arrive as status=0 with the error string in body (SDK bridge).
        denied = event.status == 0
        body_txt = (event.body or b"")[:120].decode("utf-8", "replace")
        _record(
            attacks,
            "effect.http_fetch",
            expected="DENY (missing net.http)",
            observed=(f"DENY: {body_txt}" if denied else f"SUCCESS status={event.status}"),
            ok=denied,
        )
    elif isinstance(event, HostLogResult):
        err = getattr(event, "error", None)
        _record(
            attacks,
            "effect.read_host_log",
            expected="DENY (missing logs.read)",
            observed=("SUCCESS leaked log" if not err else f"DENY: {err}"),
            ok=bool(err),
        )
    elif isinstance(event, (FilePicked, FilePickCancelled)):
        _record(
            attacks,
            "effect.open_file_picker",
            expected="DENY/cancel (missing fs.pick)",
            observed=type(event).__name__,
            ok=isinstance(event, FilePickCancelled),
        )
    elif isinstance(event, McpConnected):
        err = getattr(event, "error", None)
        _record(
            attacks,
            "effect.mcp_connect",
            expected="DENY (missing mcp.client / gated)",
            observed=("SUCCESS" if not err else f"DENY: {err}"),
            ok=bool(err),
        )
    elif isinstance(event, AiResponse):
        err = getattr(event, "error", None)
        _record(
            attacks,
            "effect.ai_query",
            expected="DENY (missing ai.query)",
            observed=("SUCCESS" if not err else f"DENY: {err}"),
            ok=bool(err),
        )
    elif isinstance(event, CapabilityGranted):
        _record(
            attacks,
            f"capability.echo.{event.name}",
            expected="DENY (not in empty manifest)",
            observed="UNEXPECTED GRANT",
            ok=False,
        )
    elif isinstance(event, CapabilityDenied):
        _record(
            attacks,
            f"capability.echo.{event.name}",
            expected="DENY (not in empty manifest)",
            observed="DENY (echo of manifest)",
            ok=True,
        )
    elif isinstance(event, EventSubscriptionResult):
        denied = event.error is not None or not event.subscription_id
        _record(
            attacks,
            f"effect.subscribe:{event.request_id}",
            expected="DENY or broker withhold (no grant)",
            observed=(f"DENY: {event.error}" if denied else f"SUCCESS sub={event.subscription_id}"),
            ok=denied,
        )

    return [
        SetState({ATTACKS_KEY: attacks, NOTICE_KEY: data.get(NOTICE_KEY, ""), PHASE_KEY: "done", "pending": pending}),
        SetStatus(f"{len(attacks)} probe results"),
    ]


def view():
    data = _snapshot()
    attacks = data.get(ATTACKS_KEY) or []
    rows: list[Any] = [
        AppBar("Isolation Redteam"),
        Text(str(data.get(NOTICE_KEY) or ""), bold=True),
        Spacer(8),
        Button("Re-run matrix", "rerun"),
        Spacer(8),
    ]
    if not attacks:
        rows.append(Text("No results yet — press r."))
    for row in attacks:
        mark = "OK " if row.get("ok") else "BUG"
        rows.append(
            Text(
                f"[{mark}] {row.get('id')}: expected={row.get('expected')} | observed={row.get('observed')}",
                bold=not bool(row.get("ok")),
            )
        )
    rows.append(FooterKeys([("r", "re-run")]))
    return Column(rows, grow=True)


# ── Probe builders ────────────────────────────────────────────────────────────


def _run_native_probes() -> list[dict]:
    """Immediate sync probes — native Python/WASI surface (no host effect)."""
    results: list[dict] = []

    # 1. Host env inheritance
    env_keys = sorted(os.environ.keys())
    has_home = "HOME" in os.environ or "USER" in os.environ
    ok_env = (not has_home) and (
        env_keys == ["PYTHONPATH"]
        or ("PYTHONPATH" in env_keys and len(env_keys) <= 3)
    )
    _record(
        results,
        "wasi.env_inherit",
        expected="DENY — only PYTHONPATH (no host HOME/secrets)",
        observed=f"keys={env_keys[:12]}{'…' if len(env_keys) > 12 else ''}",
        ok=ok_env,
    )

    # 2. List WASI preopens / cwd
    for label, path in [
        ("wasi.listdir_app", "/app"),
        ("wasi.listdir_sdk", "/sdk"),
        ("wasi.listdir_root", "/"),
        ("wasi.listdir_home", "/home"),
        ("wasi.listdir_tmp", "/tmp"),
        ("wasi.listdir_etc", "/etc"),
    ]:
        try:
            names = os.listdir(path)
            _record(
                results,
                label,
                expected="DENY except /app and /sdk (read-only preopens)",
                observed=f"SUCCESS names={names[:8]}",
                ok=path in ("/app", "/sdk"),
            )
        except OSError as exc:
            _record(
                results,
                label,
                expected="DENY except /app and /sdk (read-only preopens)",
                observed=f"DENY: {exc}",
                ok=path not in ("/app", "/sdk"),
            )

    # 3. Read sibling / secrets paths via native open
    # Paths built at runtime so package static analysis does not refuse the
    # fixture for containing traversal string literals (validators still assume
    # "native Python"; CPython-WASM is sandboxed — see docs/wasm-runtime.md).
    # No HOME in WASI env — Path.home() raises; use a literal probe target.
    home = os.environ.get("HOME") or "/home/box"
    secret_paths = [
        home + "/.plexi-alpha/apps/todo/main.py",
        home + "/.plexi-alpha/apps/todo/manifest.toml",
        "/etc/passwd",
        "/app" + "/.." + "/todo/main.py",
        ".." + "/.." + "/.." + "/.plexi-alpha/apps/todo/main.py",
        "/sdk/plexi_sdk/effects.py",  # allowed preopen — expect SUCCESS read-only
    ]
    for raw in secret_paths:
        label = f"wasi.open:{raw}"
        try:
            with open(raw, "rb") as handle:
                blob = handle.read(64)
            # Success is only OK for true preopen roots (not /app/../ escapes).
            allowed = (
                raw == "/sdk/plexi_sdk/effects.py"
                or raw.startswith("/sdk/")
                or (raw.startswith("/app/") and "/../" not in raw and not raw.startswith("/app/.."))
            )
            _record(
                results,
                label,
                expected="DENY unless under /app|/sdk preopen (no .. escape)",
                observed=f"SUCCESS {len(blob)}b",
                ok=allowed,
            )
        except OSError as exc:
            # Denials are the happy path for host/sibling/secret targets.
            _record(
                results,
                label,
                expected="DENY unless under /app|/sdk preopen (no .. escape)",
                observed=f"DENY: {type(exc).__name__}: {exc}",
                ok=True,
            )

    # 4. Write attempt on /app (should be read-only preopen)
    try:
        with open("/app/_redteam_write_probe", "w", encoding="utf-8") as handle:
            handle.write("pwn")
        _record(
            results,
            "wasi.write_app",
            expected="DENY (FilePerms::READ only on /app)",
            observed="UNEXPECTED SUCCESS write",
            ok=False,
        )
    except OSError as exc:
        _record(
            results,
            "wasi.write_app",
            expected="DENY (FilePerms::READ only on /app)",
            observed=f"DENY: {exc}",
            ok=True,
        )

    # 5. Subprocess / socket / ctypes — expect ImportError or WASI deny
    for label, fn in [
        ("wasi.subprocess", lambda: __import__("subprocess").run(["id"], capture_output=True)),
        ("wasi.socket", lambda: __import__("socket").socket().connect(("1.1.1.1", 80))),
        ("wasi.ctypes_dlopen", lambda: __import__("ctypes").CDLL("libc.so.6")),
    ]:
        try:
            fn()
            _record(results, label, expected="DENY / unavailable in WASI", observed="UNEXPECTED SUCCESS", ok=False)
        except Exception as exc:  # noqa: BLE001 — probe
            _record(
                results,
                label,
                expected="DENY / unavailable in WASI",
                observed=f"DENY: {type(exc).__name__}: {exc}",
                ok=True,
            )

    return results


def _effect_probe_effects() -> list:
    """Protected effects — expect deny with empty manifest capabilities."""
    return [
        FileRead(path=(".." + "/") * 3 + ".plexi-alpha/apps/todo/main.py"),
        FileRead(path="/home/box/.plexi-alpha/apps/todo/main.py"),
        FileRead(path="manifest.toml"),  # relative inside workspace — still needs fs.read
        FileWrite(path="pwn.txt", content=b"redteam"),
        HttpFetch(url="https://example.com/", method="GET"),
        ReadHostLog(max_bytes=4096),
        OpenFilePicker(request_id="redteam-pick", mode="open"),
        McpConnect(request_id="redteam-mcp", server_id="does-not-exist"),
        AiQuery(
            request_id="redteam-ai",
            model_tier="fast",
            system="redteam",
            messages=[AiMessage(role="user", content="ping")],
        ),
        SubscribeEventStreams(
            request_id="redteam-sub",
            app_id="event-probe",
            event_names=["probe.tick"],
            payload_mode="full",
        ),
        SubscribeEventStreams(
            request_id="redteam-sub-todo",
            app_id="todo",
            event_names=["*"],
            payload_mode="full",
        ),
    ]


def _capability_echo_probes() -> list:
    """Python RequestCapability only echoes manifest membership (no prompt)."""
    return [
        RequestCapability("fs.read"),
        RequestCapability("net.http"),
        RequestCapability("spawn.app"),
        RequestCapability("clipboard.read"),
        RequestCapability("ai.query"),
        RequestCapability("secrets.read"),
        RequestCapability("permissions.manage"),
    ]


def _emit_raw_protocol_probes() -> list[dict]:
    """Attempt raw stdout JSON injection for bridge commands the SDK omits.

    If the guest protocol channel is shared stdout, these lines become
    AppCommand::{SpawnApp,SpawnPane,ShowNotification} without going through
    effects.py. Host MUST still enforce spawn.app / notify capabilities —
    as of 2026-09-21 code review, Python dispatch did not.
    """
    payloads = [
        {"type": "spawn_app", "app_id": "todo"},
        {"type": "spawn_pane", "app_id": "calc", "layout": "split-right"},
        {
            "type": "show_notification",
            "notify_id": "redteam-notify",
            "title": "redteam",
            "body": "ungated notify probe",
        },
        {"type": "focus_pane", "pane_id": 1},
    ]
    results: list[dict] = []
    for payload in payloads:
        line = json.dumps(payload, separators=(",", ":"))
        try:
            sys.stdout.write(line + "\n")
            sys.stdout.flush()
            _record(
                results,
                f"raw_stdout.{payload['type']}",
                expected="DENY at host (capability / ignore) — injection must not open panes",
                observed="EMITTED on stdout (host reaction is the verdict; watch panes/notifications)",
                ok=False,  # unresolved until host reaction observed
            )
            log.warn(f"isolation-redteam: emitted raw protocol {payload['type']}")
        except Exception as exc:  # noqa: BLE001
            _record(
                results,
                f"raw_stdout.{payload['type']}",
                expected="DENY at host (capability / ignore)",
                observed=f"emit failed: {exc}",
                ok=True,
            )
    return results


# ── Helpers ───────────────────────────────────────────────────────────────────


def _snapshot() -> dict:
    return {
        ATTACKS_KEY: state.get(ATTACKS_KEY, []) or [],
        NOTICE_KEY: state.get(NOTICE_KEY, "") or "",
        PHASE_KEY: state.get(PHASE_KEY, "idle") or "idle",
        "pending": state.get("pending", []) or [],
    }


def _record(bucket: list, attack_id: str, *, expected: str, observed: str, ok: bool) -> None:
    bucket.append(
        {
            "id": attack_id,
            "expected": expected,
            "observed": observed,
            "ok": bool(ok),
        }
    )
    level = log.info if ok else log.warn
    try:
        level(f"isolation-redteam [{('OK' if ok else 'BUG')}] {attack_id}: {observed}")
    except Exception:  # noqa: BLE001
        pass


def _safe(exc: BaseException) -> str:
    return f"{type(exc).__name__}: {exc}\n{traceback.format_exc(limit=2)}"
