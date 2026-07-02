#!/usr/bin/env python3
"""Logs — live tail of the Plexi host log.

Flagship SDK v3 exemplar. Every level is color-coded with a semantic Badge,
rows filter by level / app id / free text, sort newest- or oldest-first, and
the active channel + log path are surfaced in the pane status chrome. The app
reads the channel-scoped ``plexi.log`` (see :func:`_detect`) and polls it on a
repeating timer.

Plexi calls three module-level functions: ``init(size, args)``,
``update(event)``, and ``view()``. State lives in ``plexi_sdk.state`` and is
only mutated by returning effects from ``update`` — ``view`` stays pure.
"""

from __future__ import annotations

import os
import re

from plexi_sdk import log, state
from plexi_sdk.effects import SetState, SetStatus, SetTimer, SetTitle
from plexi_sdk.events import KeyEvent, TimerFired, UiAction, UiValueChange
from plexi_sdk.ui import (
    SPACE_SM,
    TEXT_HINT,
    ActionBar,
    AppBar,
    Badge,
    Button,
    Column,
    FooterKeys,
    HStack,
    Scrollable,
    Sized,
    Text,
    TextEdit,
)

POLL_MS = 2_000
TIMER_ID = 1
MAX_LINES = 500
TAIL_BYTES = 256 * 1024
MSG_ELIDE = 200
SEARCH_ID = "logs-search"

# Level filters. ALL is the unfiltered default; the rest match the log's level
# column exactly.
LEVELS = ["ALL", "ERROR", "WARN", "INFO", "DEBUG"]
LEVEL_KEY = {"a": "ALL", "e": "ERROR", "w": "WARN", "i": "INFO", "d": "DEBUG"}

# Semantic token per level — resolved to the host theme's role color at render
# time. Never hardcode hex here: danger/warning/accent track light+dark themes
# and stay WCAG-legible because the host picks the contrasting badge text.
LEVEL_COLOR = {
    "ERROR": "danger",
    "WARN": "warning",
    "INFO": "accent",
    "DEBUG": "neutral",
    "TRACE": "section",
}

# Row height (px) and column widths so the badge, timestamp, and target align
# across rows. The row is height-bounded because an HStack inside a Scrollable
# would otherwise inherit the full viewport height and misalign its children.
ROW_H = 26.0
TIME_W = 58.0
TARGET_W = 150.0

# [2026-07-02 01:46:18] [INFO] [plexi::config] message text
LOG_RE = re.compile(
    r"^\[(\d{4}-\d{2}-\d{2} (\d{2}:\d{2}:\d{2}))\] \[(\w+)\] \[([^\]]+)\] (.*)$"
)

DEFAULT_STATE = {
    "path": "",
    "channel": "",
    "lines": [],
    "signature": None,
    "level": "ALL",
    "target": "ALL",
    "query": "",
    "searching": False,
    "order": "newest",  # newest | oldest
}


def init(size, args) -> list:
    path, channel = _detect()
    if args:
        path = args[0]
    data = dict(DEFAULT_STATE)
    data["path"] = path
    data["channel"] = channel
    data.update(_read(path))
    log.info(f"logs: init channel={channel!r} path={path!r} lines={len(data['lines'])}")
    return [
        SetTitle("Logs"),
        SetTimer(TIMER_ID, POLL_MS, repeat=True),
        SetStatus(_status(data)),
        SetState(data),
    ]


def update(event) -> list:
    data = _state()

    if isinstance(event, TimerFired) and event.id == TIMER_ID:
        refreshed = _read(data["path"], data["signature"])
        if refreshed["signature"] == data["signature"]:
            return []
        data.update(refreshed)
        log.debug(f"logs: refreshed lines={len(data['lines'])}")
        return [SetState(data), SetStatus(_status(data))]

    if isinstance(event, UiValueChange) and event.handler_id == SEARCH_ID:
        data["query"] = event.value
        return [SetState(data), SetStatus(_status(data))]

    if isinstance(event, UiAction) and event.handler_id.startswith("lvl:"):
        data["level"] = event.handler_id.split(":", 1)[1]
        log.info(f"logs: level filter -> {data['level']}")
        return [SetState(data), SetStatus(_status(data))]

    if not isinstance(event, KeyEvent) or not event.pressed:
        return []

    key = event.key
    if data["searching"]:
        if key == "escape":
            data["searching"] = False
            data["query"] = ""
            log.info("logs: search cancelled")
            return [SetState(data), SetStatus(_status(data))]
        return []

    if key in LEVEL_KEY:
        data["level"] = LEVEL_KEY[key]
        log.info(f"logs: level filter -> {data['level']}")
    elif key == "t":
        data["target"] = _cycle_target(data)
        log.info(f"logs: app filter -> {data['target']}")
    elif key == "s":
        data["order"] = "oldest" if data["order"] == "newest" else "newest"
        log.info(f"logs: sort -> {data['order']}")
    elif key == "/":
        data["searching"] = True
        log.info("logs: search opened")
    elif key == "escape" and data["query"]:
        data["query"] = ""
    elif key == "escape" and data["target"] != "ALL":
        data["target"] = "ALL"
    elif key == "r":
        data.update(_read(data["path"]))
        log.info("logs: manual refresh")
    else:
        return []
    return [SetState(data), SetStatus(_status(data))]


def view():
    data = _state()
    rows = _filtered(data)

    children: list = [
        AppBar("Logs", _subtitle(data)),
        ActionBar([
            Button(
                level,
                f"lvl:{level}",
                style="primary" if level == data["level"] else "secondary",
            )
            for level in LEVELS
        ]),
    ]
    if data["searching"]:
        children.append(
            TextEdit(SEARCH_ID, value=data["query"], placeholder="filter by target or message")
        )

    if rows:
        children.append(Scrollable(Column([_row(line) for line in rows], padding=0, gap=SPACE_SM)))
    else:
        children.append(Text(f"no matching entries in {data['path']}", tone="hint"))

    children.append(
        FooterKeys([
            ("a/e/w/i/d", "level"),
            ("t", "app"),
            ("s", "sort"),
            ("/", "search"),
            ("r", "refresh"),
        ])
    )
    return Column(children, grow=True, padding=SPACE_SM)


def _row(line: dict):
    level = line["level"]
    message = line["message"]
    if len(message) > MSG_ELIDE:
        message = message[:MSG_ELIDE] + "…"
    return Sized(
        HStack(
            [
                Badge(level, color=LEVEL_COLOR.get(level, "neutral")),
                Sized(Text(line["time"], size=TEXT_HINT, color="muted"), width=TIME_W),
                Sized(Text(line["target"], size=TEXT_HINT, color="section"), width=TARGET_W),
                Text(message, size=TEXT_HINT),
            ],
            gap=SPACE_SM,
        ),
        height=ROW_H,
    )


# ── State helpers ───────────────────────────────────────────────────────────


def _state() -> dict:
    data = dict(DEFAULT_STATE)
    for key, default in DEFAULT_STATE.items():
        data[key] = state.get(key, default)
    data["lines"] = [dict(line) for line in data.get("lines") or []]
    data["level"] = data["level"] if data["level"] in LEVELS else "ALL"
    data["query"] = str(data.get("query") or "")
    data["target"] = str(data.get("target") or "ALL")
    data["order"] = "oldest" if data.get("order") == "oldest" else "newest"
    data["searching"] = bool(data.get("searching"))
    return data


def _filtered(data: dict) -> list[dict]:
    lines = data["lines"]  # newest-first as read
    if data["level"] != "ALL":
        lines = [ln for ln in lines if ln["level"] == data["level"]]
    if data["target"] != "ALL":
        lines = [ln for ln in lines if ln["target"] == data["target"]]
    query = data["query"].lower().strip()
    if query:
        lines = [
            ln for ln in lines
            if query in ln["target"].lower() or query in ln["message"].lower()
        ]
    if data["order"] == "oldest":
        lines = list(reversed(lines))
    return lines


def _cycle_target(data: dict) -> str:
    """Advance the app/target filter to the next unique target, ALL-inclusive."""
    seen: list[str] = ["ALL"]
    for line in data["lines"]:
        if line["target"] not in seen:
            seen.append(line["target"])
    current = data["target"] if data["target"] in seen else "ALL"
    return seen[(seen.index(current) + 1) % len(seen)]


# ── Log file ────────────────────────────────────────────────────────────────


def _detect() -> tuple[str, str]:
    """Resolve the (log path, channel label) for the active Plexi profile.

    ``PLEXI_CONFIG_DIR`` wins (the host exports it per pane); the channel is the
    profile dir suffix. Otherwise ``PLEXI_CHANNEL`` selects the profile. As a
    last resort, pick the most recently written known profile log.
    """
    config_dir = os.environ.get("PLEXI_CONFIG_DIR")
    if config_dir:
        return os.path.join(config_dir, "plexi.log"), _channel_of(config_dir)

    channel = os.environ.get("PLEXI_CHANNEL", "").strip()
    if channel:
        return _channel_path(channel), _normalize_channel(channel)

    candidates = [
        (os.path.expanduser("~/.plexi-alpha/plexi.log"), "alpha"),
        (os.path.expanduser("~/.plexi-beta/plexi.log"), "beta"),
        (os.path.expanduser("~/.plexi/plexi.log"), "default"),
    ]
    existing = [(os.path.getmtime(p), p, c) for p, c in candidates if os.path.exists(p)]
    if existing:
        _, path, chan = max(existing)
        return path, chan
    return candidates[0][0], candidates[0][1]


def _normalize_channel(channel: str) -> str:
    return "default" if channel in ("main", "default") else channel


def _channel_path(channel: str) -> str:
    profile = ".plexi" if channel in ("main", "default") else f".plexi-{channel}"
    return os.path.expanduser(os.path.join("~", profile, "plexi.log"))


def _channel_of(config_dir: str) -> str:
    """Channel label from a config dir like ``~/.plexi-alpha`` -> ``alpha``."""
    name = os.path.basename(config_dir.rstrip("/"))
    if name == ".plexi":
        return "default"
    if name.startswith(".plexi-"):
        return name[len(".plexi-"):]
    return name or "default"


def _read(path: str, prev_signature=None) -> dict:
    """Return updated ``{signature, lines}``; lines are newest-first."""
    signature = _signature(path)
    if signature is not None and signature == prev_signature:
        return {"signature": signature}
    if signature is None:
        return {"signature": None, "lines": []}
    try:
        size = os.path.getsize(path)
        with open(path, "rb") as handle:
            if size > TAIL_BYTES:
                handle.seek(-TAIL_BYTES, os.SEEK_END)
                handle.readline()
            raw = handle.readlines()[-MAX_LINES:]
    except OSError as exc:
        log.warn(f"logs: cannot read {path!r}: {exc}")
        return {"signature": signature, "lines": []}
    lines = []
    for entry in reversed(raw):
        parsed = _parse(entry.decode("utf-8", errors="replace"))
        if parsed:
            lines.append(parsed)
    return {"signature": signature, "lines": lines}


def _signature(path: str):
    try:
        stat = os.stat(path)
    except OSError:
        return None
    return [stat.st_size, stat.st_mtime_ns]


def _parse(raw: str) -> "dict | None":
    match = LOG_RE.match(raw.rstrip())
    if not match:
        return None
    _, time, level, target, message = match.groups()
    return {"time": time, "level": level, "target": target, "message": message}


# ── Chrome text ─────────────────────────────────────────────────────────────


def _subtitle(data: dict) -> str:
    parts = [data["level"]]
    if data["target"] != "ALL":
        parts.append(f"@{data['target']}")
    if data["query"]:
        parts.append(f"/{data['query']}")
    parts.append(data["order"])
    return " · ".join(parts)


def _status(data: dict) -> str:
    return f"{data['channel']} · {len(_filtered(data))} lines · {data['path']}"
