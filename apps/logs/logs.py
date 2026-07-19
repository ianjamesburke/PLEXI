#!/usr/bin/env python3
"""Logs — live tail of the Plexi host channel log with clickable level filters.

Runs sandboxed as CPython-in-WASM. The channel log lives on the host at
`~/.plexi-<channel>/plexi.log`, outside the WASI mounts (only the SDK and app
dir are preopened), so this app cannot open it directly. It reads the log
through the capability-gated `read_host_log` host effect (`logs.read`): the host
owns path resolution and tails the file; this app owns parsing, filtering, and
presentation. A log it cannot reach renders an explicit error, never a blank
pane.

Each parsed entry renders as its own row node — timestamp, a color-coded level
badge, module, and message — so the delta protocol re-serializes only the rows
that actually change (a quiet pane emits near-empty deltas). Follow ON keeps
polling and shows the newest entry at the top; follow OFF freezes the visible
tail entirely (polling stops, fresh results are dropped) so the view holds still
for reading.
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
    SPACE_SM,
    SPACE_XS,
    TEXT_HINT,
    AppBar,
    Badge,
    BadgeColor,
    Column,
    Component,
    FooterKeys,
    HStack,
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
LEVELS = ["ALL", "ERROR", "WARN", "INFO", "DEBUG"]
LEVEL_KEY = {"a": "ALL", "e": "ERROR", "w": "WARN", "i": "INFO", "d": "DEBUG"}
LEVEL_TAB = "logs-level"
SEARCH_INPUT = "logs-search"

# Each level maps to a semantic badge color the host decoder understands
# (`decode_badge_color` in src/host/wasm_python.rs). Badges are the only tree
# node that carries color — plain Text nodes drop their color field — so the
# level badge is where severity color lives. Unknown levels fall back to
# neutral rather than being dropped.
NEUTRAL: BadgeColor = "neutral"
LEVEL_COLOR: dict[str, BadgeColor] = {
    "ERROR": "danger",
    "WARN": "warning",
    "INFO": "accent",
    "DEBUG": "neutral",
}

LOG_RE = re.compile(
    r"^\[(\d{4}-\d{2}-\d{2} (\d{2}:\d{2}:\d{2}))\] \[(\w+)\] \[([^\]]+)\] (.*)$"
)

DEFAULT_STATE = {
    "path": "",
    "lines": [],
    "filter": "ALL",
    "query": "",
    "searching": False,
    "follow": True,
    "pending_force": False,
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
        # Live tail only while following. Frozen (follow off) means the poll
        # stops entirely — no fresh read, so the visible tail cannot move.
        if data["follow"]:
            return [ReadHostLog(TAIL_BYTES)]
        return []

    if isinstance(event, UiAction) and event.handler_id.startswith(LEVEL_TAB + ":"):
        index = _tab_index(event.handler_id)
        if index is None:
            return []
        data["filter"] = LEVELS[index]
        return [SetState(data), SetStatus(_status(data))]

    if isinstance(event, UiValueChange) and event.handler_id == SEARCH_INPUT:
        data["query"] = event.value
        data["follow"] = False
        return [SetState(data), SetStatus(_status(data))]

    if not isinstance(event, KeyEvent) or not event.pressed:
        return []

    key = event.key
    if data["searching"]:
        if key == "escape":
            data["searching"] = False
            data["query"] = ""
            data["follow"] = True
            return [SetState(data), SetStatus(_status(data)), ReadHostLog(TAIL_BYTES)]
        return []

    if key in LEVEL_KEY:
        data["filter"] = LEVEL_KEY[key]
    elif key == "/":
        data["searching"] = True
        data["follow"] = False
    elif key == "escape" and data["query"]:
        data["query"] = ""
        data["follow"] = True
    elif key == "f":
        data["follow"] = not data["follow"]
        if data["follow"]:
            # Resuming follow: catch up immediately instead of waiting for the
            # next poll tick.
            return [SetState(data), SetStatus(_status(data)), ReadHostLog(TAIL_BYTES)]
    elif key == "r":
        # Force one fresh read now, even while frozen. `pending_force` lets the
        # incoming result bypass the follow-off freeze exactly once.
        data["pending_force"] = True
        return [SetState(data), ReadHostLog(TAIL_BYTES)]
    else:
        return []
    return [SetState(data), SetStatus(_status(data))]


def view():
    data = _state()
    subtitle = "Search" if data["searching"] else _subtitle(data)
    # Search replaces the level TabBar row rather than stacking a third header
    # row — keeps Logs' header height in line with other Core apps (Snake),
    # which never stack more than AppBar + one control row.
    header_row: Component = (
        TextInput(
            SEARCH_INPUT,
            value=data["query"],
            placeholder="filter by target or message",
            on_change=SEARCH_INPUT,
        )
        if data["searching"]
        else TabBar(LEVEL_TAB, LEVELS, active=LEVELS.index(data["filter"]))
    )
    children: list[Component] = [
        AppBar("Logs", subtitle),
        header_row,
    ]
    children.extend(
        [
            Scrollable(_body(data)),
            Spacer(grow=True),
            FooterKeys(
                [
                    ("a/e/w/i/d", "level"),
                    ("/", "search"),
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
    data["follow"] = bool(data.get("follow"))
    data["pending_force"] = bool(data.get("pending_force"))
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
        data["pending_force"] = False
        log.warn(f"logs: host log unavailable: {result.error}")
        return [SetState(data), SetStatus(_status(data))]

    forced = data["pending_force"]
    data["pending_force"] = False
    # Follow-freeze: once loaded, a fresh tail is only applied while following
    # or when an explicit refresh (`r`) forced this read. Otherwise the visible
    # tail holds still.
    if was_loaded and not had_error and not data["follow"] and not forced:
        return []

    content = result.content.decode("utf-8", errors="replace") if result.content else ""
    lines = _parse_log(content)
    if was_loaded and not had_error and not forced and lines == previous_lines:
        return []  # unchanged tail — skip the repaint
    data["error"] = None
    data["lines"] = lines
    data["loaded"] = True
    return [SetState(data), SetStatus(_status(data))]


def _parse_log(content: str) -> list[dict]:
    parsed = []
    for raw in reversed(content.splitlines()[-MAX_LINES:]):
        item = _parse(raw)
        if item is None:
            stripped = raw.rstrip()
            if not stripped:
                continue
            # Unparseable line: keep it, rendered as a plain full-width message
            # row. Never dropped — a malformed line is still signal.
            item = {"time": "", "level": "", "target": "", "message": stripped}
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


def _body(data: dict) -> Component:
    """The scrollable tail: one row per entry, or a placeholder message."""
    lines = [] if data["error"] else _filtered(data)
    if data["error"] or not lines:
        return Text(_placeholder_text(data), size=TEXT_HINT)
    return Column([_row(line) for line in lines], gap=SPACE_XS)


def _row(line: dict) -> Component:
    """One log entry as a row of separated fields.

    Parseable entries lay out as ``time · LEVEL badge · module · message``; the
    badge is the sole color-carrying node. Unparseable entries collapse to a
    single full-width message so nothing is silently dropped.
    """
    level = line["level"]
    if not level:
        message: list[Component] = [Text(line["message"], size=TEXT_HINT)]
        return HStack(message, gap=SPACE_SM)
    fields: list[Component] = [
        Text(line["time"], size=TEXT_HINT),
        Badge(level, color=LEVEL_COLOR.get(level, NEUTRAL)),
        Text(line["target"], size=TEXT_HINT),
        Text(line["message"], size=TEXT_HINT),
    ]
    return HStack(fields, gap=SPACE_SM)


def _placeholder_text(data: dict) -> str:
    if data["error"]:
        location = f" ({data['path']})" if data["path"] else ""
        return f"Cannot read host log{location}:\n{data['error']}"
    if not data["loaded"]:
        return "Loading host log…"
    where = f" at {data['path']}" if data["path"] else ""
    return f"No log entries{where}"


def _subtitle(data: dict) -> str:
    parts = [data["filter"]]
    if data["query"]:
        parts.append(f"/{data['query']}")
    parts.append("follow" if data["follow"] else "frozen")
    return " · ".join(parts)


def _status(data: dict) -> str:
    if data["error"]:
        return "log unavailable"
    if not data["loaded"]:
        return "loading…"
    return f"{len(_filtered(data))} lines"
