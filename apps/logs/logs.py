#!/usr/bin/env python3
"""Logs — SDK v3 runtime-state live tail of the Plexi host log."""

from __future__ import annotations

import os
import re

from plexi_sdk import log, state
from plexi_sdk.effects import SetState, SetStatus, SetTimer, SetTitle
from plexi_sdk.events import KeyEvent, TimerFired, UiValueChange
from plexi_sdk.ui import AppBar, Column, FooterKeys, Scrollable, Spacer, Text, TextInput

POLL_MS = 2_000
TIMER_ID = 1
MAX_LINES = 500
TAIL_BYTES = 256 * 1024
VISIBLE_LINES = 28
FILTERS = ["ALL", "ERROR", "WARN", "INFO", "DEBUG"]
FILTER_KEY = {"a": "ALL", "e": "ERROR", "w": "WARN", "i": "INFO", "d": "DEBUG"}

LOG_RE = re.compile(
    r"^\[(\d{4}-\d{2}-\d{2} (\d{2}:\d{2}:\d{2}))\] \[(\w+)\] \[([^\]]+)\] (.*)$"
)

DEFAULT_STATE = {
    "path": "",
    "lines": [],
    "filter": "ALL",
    "query": "",
    "searching": False,
    "offset": 0,
    "follow": True,
    "signature": None,
}


def init(size, args) -> list:
    path = args[0] if args else _detect_log_path()
    data = _state()
    data["path"] = data["path"] or path
    data.update(_refresh(data))
    log.info(f"logs: SDK v3 initialized path={data['path']!r}")
    return [
        SetTitle("Logs"),
        SetTimer(TIMER_ID, POLL_MS, repeat=True),
        SetStatus(_status(data)),
        SetState(data),
    ]


def update(event) -> list:
    data = _state()
    if isinstance(event, TimerFired) and event.id == TIMER_ID:
        refreshed = _refresh(data)
        if refreshed["signature"] == data["signature"]:
            return []
        data.update(refreshed)
        return [SetState(data), SetStatus(_status(data))]

    if isinstance(event, UiValueChange) and event.handler_id == "logs-search":
        data["query"] = event.value
        data["offset"] = 0
        data["follow"] = False
        return [SetState(data), SetStatus(_status(data))]

    if not isinstance(event, KeyEvent) or not event.pressed:
        return []

    key = event.key
    if data["searching"]:
        if key == "escape":
            data["searching"] = False
            data["query"] = ""
            data["offset"] = 0
            data["follow"] = True
            return [SetState(data), SetStatus(_status(data))]
        return []

    if key in FILTER_KEY:
        data["filter"] = FILTER_KEY[key]
        data["offset"] = 0
    elif key == "/":
        data["searching"] = True
        data["follow"] = False
    elif key == "escape" and data["query"]:
        data["query"] = ""
        data["offset"] = 0
        data["follow"] = True
    elif key in ("down", "j", "ArrowDown"):
        data["offset"] = min(_max_offset(data), data["offset"] + 1)
        data["follow"] = False
    elif key in ("up", "k", "ArrowUp"):
        data["offset"] = max(0, data["offset"] - 1)
        data["follow"] = False
    elif key == "g":
        data["offset"] = 0
        data["follow"] = True
    elif key == "G":
        data["offset"] = _max_offset(data)
        data["follow"] = False
    elif key == "f":
        data["follow"] = not data["follow"]
        if data["follow"]:
            data["offset"] = 0
    elif key == "r":
        data.update(_refresh(data, force=True))
    else:
        return []
    return [SetState(data), SetStatus(_status(data))]


def view():
    data = _state()
    title = "Search" if data["searching"] else _subtitle(data)
    children = [AppBar("Logs", title)]
    if data["searching"]:
        children.append(
            TextInput(
                "logs-search",
                value=data["query"],
                placeholder="filter by target or message",
                on_change="logs-search",
            )
        )
    children.extend(
        [
            Scrollable(Text(_visible_text(data), size=11.0)),
            Spacer(grow=True),
            FooterKeys(
                [
                    ("a/e/w/i/d", "level"),
                    ("/", "search"),
                    ("j/k", "scroll"),
                    ("f", "follow"),
                ]
            ),
        ]
    )
    return Column(children, grow=True, padding=0)


def _state() -> dict:
    data = dict(DEFAULT_STATE)
    for key, value in DEFAULT_STATE.items():
        data[key] = state.get(key, value)
    data["lines"] = [dict(line) for line in data.get("lines") or []]
    data["filter"] = str(data.get("filter") or "ALL")
    if data["filter"] not in FILTERS:
        data["filter"] = "ALL"
    data["query"] = str(data.get("query") or "")
    data["offset"] = max(0, int(data.get("offset") or 0))
    data["follow"] = bool(data.get("follow"))
    data["searching"] = bool(data.get("searching"))
    return data


def _detect_log_path() -> str:
    config_dir = os.environ.get("PLEXI_CONFIG_DIR")
    if config_dir:
        return os.path.join(config_dir, "plexi.log")
    channel = os.environ.get("PLEXI_CHANNEL", "").strip()
    if channel:
        profile = ".plexi" if channel in ("main", "default") else f".plexi-{channel}"
        return os.path.expanduser(os.path.join("~", profile, "plexi.log"))
    candidates = [
        os.path.expanduser("~/.plexi-alpha/plexi.log"),
        os.path.expanduser("~/.plexi-beta/plexi.log"),
        os.path.expanduser("~/.plexi/plexi.log"),
    ]
    existing = [
        (os.path.getmtime(path), path) for path in candidates if os.path.exists(path)
    ]
    return max(existing)[1] if existing else candidates[0]


def _signature(path: str):
    try:
        stat = os.stat(path)
    except OSError:
        return None
    return [stat.st_size, stat.st_mtime_ns]


def _refresh(data: dict, force: bool = False) -> dict:
    signature = _signature(data["path"])
    if not force and signature == data.get("signature"):
        return {"signature": signature, "lines": data["lines"]}
    lines = _read_log(data["path"])
    if data.get("follow", True):
        data["offset"] = 0
    return {"signature": signature, "lines": lines}


def _read_log(path: str) -> list[dict]:
    try:
        size = os.path.getsize(path)
        with open(path, "rb") as handle:
            if size > TAIL_BYTES:
                handle.seek(-TAIL_BYTES, os.SEEK_END)
                handle.readline()
            raw_lines = handle.readlines()[-MAX_LINES:]
    except OSError:
        return []
    parsed = []
    for raw in reversed(raw_lines):
        item = _parse(raw.decode("utf-8", errors="replace"))
        if item:
            parsed.append(item)
    return parsed


def _parse(raw: str) -> dict | None:
    match = LOG_RE.match(raw.rstrip())
    if not match:
        return None
    _, time, level, target, message = match.groups()
    return {"time": time, "level": level, "target": target, "message": message}


def _filtered(data: dict) -> list[dict]:
    lines = data["lines"]
    if data["filter"] != "ALL":
        lines = [line for line in lines if line["level"] == data["filter"]]
    query = data["query"].lower().strip()
    if query:
        lines = [
            line
            for line in lines
            if query in line["target"].lower() or query in line["message"].lower()
        ]
    return lines


def _visible_text(data: dict) -> str:
    lines = _filtered(data)
    if not lines:
        return f"No log entries at {data['path']}"
    offset = min(data["offset"], _max_offset(data))
    visible = lines[offset : offset + VISIBLE_LINES]
    return "\n".join(
        f"{line['time']} {line['level']:<5} {line['target']:<24} {line['message']}"
        for line in visible
    )


def _max_offset(data: dict) -> int:
    return max(0, len(_filtered(data)) - VISIBLE_LINES)


def _subtitle(data: dict) -> str:
    parts = [data["filter"]]
    if data["query"]:
        parts.append(f"/{data['query']}")
    if data["follow"]:
        parts.append("follow")
    return " · ".join(parts)


def _status(data: dict) -> str:
    return f"{len(_filtered(data))} lines"
