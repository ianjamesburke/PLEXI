#!/usr/bin/env python3
"""Logs — live tail of the Plexi host channel log with clickable level filters.

Runs sandboxed as CPython-in-WASM. The channel log lives on the host at
`~/.plexi-<channel>/plexi.log`, outside the WASI mounts (only the SDK and app
dir are preopened), so this app cannot open it directly. It reads the log
through the capability-gated `read_host_log` host effect (`logs.read`): the host
owns path resolution and tails the file; this app owns parsing, filtering, and
presentation. A log it cannot reach renders an explicit error, never a blank
pane.
"""

from __future__ import annotations

import re

from plexi_sdk import log, state
from plexi_sdk.effects import ReadHostLog, SetState, SetStatus, SetTimer, SetTitle
from plexi_sdk.events import (
    HostLogResult,
    KeyEvent,
    TimerFired,
    UiAction,
    UiValueChange,
)
from plexi_sdk.ui import (
    AppBar,
    Column,
    FooterKeys,
    Scrollable,
    Spacer,
    TabBar,
    Text,
    TextInput,
)

POLL_MS = 2_000
TIMER_ID = 1
MAX_LINES = 500
TAIL_BYTES = 256 * 1024
VISIBLE_LINES = 28
LEVELS = ["ALL", "ERROR", "WARN", "INFO", "DEBUG"]
LEVEL_KEY = {"a": "ALL", "e": "ERROR", "w": "WARN", "i": "INFO", "d": "DEBUG"}
LEVEL_TAB = "logs-level"
SEARCH_INPUT = "logs-search"

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
    "error": None,
    "loaded": False,
}


def init(size, args) -> list:
    data = _state()
    log.info("logs: SDK v3 initialized — requesting host log via read_host_log effect")
    return [
        SetTitle("Logs"),
        SetTimer(TIMER_ID, POLL_MS, repeat=True),
        ReadHostLog(TAIL_BYTES),
        SetStatus(_status(data)),
        SetState(data),
    ]


def update(event) -> list:
    data = _state()

    if isinstance(event, HostLogResult):
        return _apply_log_result(data, event)

    if isinstance(event, TimerFired) and event.id == TIMER_ID:
        # Live tail: ask the host for a fresh tail every poll. Lines update when
        # the HostLogResult arrives; nothing to render until then.
        return [ReadHostLog(TAIL_BYTES)]

    if isinstance(event, UiAction) and event.handler_id.startswith(LEVEL_TAB + ":"):
        index = _tab_index(event.handler_id)
        if index is None:
            return []
        data["filter"] = LEVELS[index]
        data["offset"] = 0
        return [SetState(data), SetStatus(_status(data))]

    if isinstance(event, UiValueChange) and event.handler_id == SEARCH_INPUT:
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

    if key in LEVEL_KEY:
        data["filter"] = LEVEL_KEY[key]
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
        # Force a fresh read now, independent of the poll timer.
        return [SetState(data), ReadHostLog(TAIL_BYTES)]
    else:
        return []
    return [SetState(data), SetStatus(_status(data))]


def view():
    data = _state()
    subtitle = "Search" if data["searching"] else _subtitle(data)
    children = [
        AppBar("Logs", subtitle),
        TabBar(LEVEL_TAB, LEVELS, active=LEVELS.index(data["filter"])),
    ]
    if data["searching"]:
        children.append(
            TextInput(
                SEARCH_INPUT,
                value=data["query"],
                placeholder="filter by target or message",
                on_change=SEARCH_INPUT,
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
                    ("r", "refresh"),
                ]
            ),
        ]
    )
    return Column(children, grow=True)


def _state() -> dict:
    data = dict(DEFAULT_STATE)
    for key, value in DEFAULT_STATE.items():
        data[key] = state.get(key, value)
    data["lines"] = [dict(line) for line in data.get("lines") or []]
    data["filter"] = str(data.get("filter") or "ALL")
    if data["filter"] not in LEVELS:
        data["filter"] = "ALL"
    data["query"] = str(data.get("query") or "")
    data["offset"] = max(0, int(data.get("offset") or 0))
    data["follow"] = bool(data.get("follow"))
    data["searching"] = bool(data.get("searching"))
    data["loaded"] = bool(data.get("loaded"))
    error = data.get("error")
    data["error"] = str(error) if error else None
    data["path"] = str(data.get("path") or "")
    return data


def _apply_log_result(data: dict, result: HostLogResult) -> list:
    previous_lines = data["lines"]
    had_error = data["error"] is not None
    was_loaded = data["loaded"]
    if result.path:
        data["path"] = result.path
    if result.error:
        data["error"] = result.error
        data["lines"] = []
        data["loaded"] = True
        log.warn(f"logs: host log unavailable: {result.error}")
        return [SetState(data), SetStatus(_status(data))]
    content = result.content.decode("utf-8", errors="replace") if result.content else ""
    lines = _parse_log(content)
    if was_loaded and not had_error and lines == previous_lines:
        return []  # unchanged tail — skip the repaint
    data["error"] = None
    data["lines"] = lines
    data["loaded"] = True
    if data["follow"]:
        data["offset"] = 0
    return [SetState(data), SetStatus(_status(data))]


def _parse_log(content: str) -> list[dict]:
    parsed = []
    for raw in reversed(content.splitlines()[-MAX_LINES:]):
        item = _parse(raw)
        if item:
            parsed.append(item)
    return parsed


def _parse(raw: str) -> dict | None:
    match = LOG_RE.match(raw.rstrip())
    if not match:
        return None
    _, time, level, target, message = match.groups()
    return {"time": time, "level": level, "target": target, "message": message}


def _tab_index(handler_id: str) -> int | None:
    try:
        index = int(handler_id.rsplit(":", 1)[1])
    except (ValueError, IndexError):
        return None
    return index if 0 <= index < len(LEVELS) else None


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
    if data["error"]:
        location = f" ({data['path']})" if data["path"] else ""
        return f"Cannot read host log{location}:\n{data['error']}"
    lines = _filtered(data)
    if not lines:
        if not data["loaded"]:
            return "Loading host log…"
        where = f" at {data['path']}" if data["path"] else ""
        return f"No log entries{where}"
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
    if data["error"]:
        return "log unavailable"
    if not data["loaded"]:
        return "loading…"
    return f"{len(_filtered(data))} lines"
