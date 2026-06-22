#!/usr/bin/env python3
"""CSV Viewer — SDK v3 state-backed CSV browser."""

from __future__ import annotations

import csv
from pathlib import Path

from plexi_sdk import log, state
from plexi_sdk.effects import SetState, SetStatus, SetTitle
from plexi_sdk.events import KeyEvent
from plexi_sdk.ui import (
    AppBar,
    Column,
    FooterKeys,
    Scrollable,
    SelectList,
    Spacer,
    Text,
)

VISIBLE_ROWS = 24
VISIBLE_COLS = 5

DEFAULT_STATE = {
    "cwd": "",
    "files": [],
    "file_hints": {},
    "selected": 0,
    "mode": "list",
    "path": "",
    "headers": [],
    "rows": [],
    "row_offset": 0,
    "col_offset": 0,
    "error": "",
}


def init(size, args) -> list:
    launch_dir = Path.cwd()
    data = _state()
    if not data["cwd"]:
        data.update(_scan_dir(launch_dir))
    if args:
        target = Path(args[0])
        if not target.is_absolute():
            target = launch_dir / target
        if target.is_file():
            data.update(_open_csv(target))
            data["files"] = [str(target)]
            data["file_hints"] = {str(target): _file_hint(target)}
            data["selected"] = 0
            data["mode"] = "detail"
    log.info(f"csv_viewer: SDK v3 initialized cwd={data['cwd']!r}")
    return [
        SetTitle("CSV Viewer"),
        SetStatus(_status(data)),
        SetState(data),
    ]


def update(event) -> list:
    if not isinstance(event, KeyEvent) or not event.pressed:
        return []
    data = _state()
    key = event.key

    if data["mode"] == "list":
        if key in ("up", "k", "ArrowUp"):
            data["selected"] = _clamp(data["selected"] - 1, len(data["files"]))
        elif key in ("down", "j", "ArrowDown"):
            data["selected"] = _clamp(data["selected"] + 1, len(data["files"]))
        elif key in ("return", "enter") and data["files"]:
            data.update(_open_csv(Path(data["files"][data["selected"]])))
            data["mode"] = "detail"
            log.info(f"csv_viewer: opened {data['path']}")
        elif key == "r":
            data.update(_scan_dir(Path(data["cwd"] or Path.cwd())))
        else:
            return []
        return [SetState(data), SetStatus(_status(data))]

    if key == "escape":
        data["mode"] = "list"
    elif key in ("up", "k", "ArrowUp"):
        data["row_offset"] = max(0, data["row_offset"] - 1)
    elif key in ("down", "j", "ArrowDown"):
        data["row_offset"] = min(_max_row_offset(data), data["row_offset"] + 1)
    elif key in ("left", "h", "ArrowLeft"):
        data["col_offset"] = max(0, data["col_offset"] - 1)
    elif key in ("right", "l", "ArrowRight"):
        data["col_offset"] = min(_max_col_offset(data), data["col_offset"] + 1)
    else:
        return []
    return [SetState(data), SetStatus(_status(data))]


def view():
    data = _state()
    if data["mode"] == "detail":
        return _detail_view(data)
    return _list_view(data)


def _state() -> dict:
    data = dict(DEFAULT_STATE)
    for key, value in DEFAULT_STATE.items():
        data[key] = state.get(key, value)
    data["files"] = list(data.get("files") or [])
    data["headers"] = list(data.get("headers") or [])
    data["rows"] = [list(row) for row in data.get("rows") or []]
    data["file_hints"] = dict(data.get("file_hints") or {})
    data["selected"] = _clamp(int(data.get("selected") or 0), len(data["files"]))
    data["row_offset"] = max(0, int(data.get("row_offset") or 0))
    data["col_offset"] = max(0, int(data.get("col_offset") or 0))
    return data


def _scan_dir(path: Path) -> dict:
    files = sorted(path.glob("*.csv"), key=lambda item: item.name.lower())
    return {
        "cwd": str(path),
        "files": [str(item) for item in files],
        "file_hints": {str(item): _file_hint(item) for item in files},
        "selected": 0,
        "mode": "list",
        "path": "",
        "headers": [],
        "rows": [],
        "row_offset": 0,
        "col_offset": 0,
        "error": "",
    }


def _file_hint(path: Path) -> str:
    try:
        kb = path.stat().st_size / 1024
    except OSError:
        return ""
    return f"{kb:.1f} KB" if kb < 1024 else f"{kb / 1024:.1f} MB"


def _open_csv(path: Path) -> dict:
    try:
        with path.open(newline="", errors="replace") as handle:
            loaded = list(csv.reader(handle))
    except OSError as exc:
        return {
            "path": str(path),
            "headers": [f"Error: {exc}"],
            "rows": [],
            "row_offset": 0,
            "col_offset": 0,
            "error": str(exc),
        }
    headers = loaded[0] if loaded else []
    return {
        "path": str(path),
        "headers": headers,
        "rows": loaded[1:],
        "row_offset": 0,
        "col_offset": 0,
        "error": "",
    }


def _list_view(data: dict):
    rows = [
        {"name": Path(path).name, "description": data["file_hints"].get(path, "")}
        for path in data["files"]
    ]
    body = (
        SelectList(rows, selected_idx=data["selected"])
        if rows
        else Text("No CSV files found.", size=12.0)
    )
    return Column(
        [
            AppBar("CSV Viewer", data["cwd"]),
            body,
            Spacer(grow=True),
            FooterKeys([("j/k", "select"), ("enter", "open"), ("r", "refresh")]),
        ],
        grow=True,
        padding=0,
    )


def _detail_view(data: dict):
    name = Path(data["path"]).name
    header = " | ".join(
        data["headers"][data["col_offset"] : data["col_offset"] + VISIBLE_COLS]
    )
    lines = [header, "-" * min(120, max(3, len(header)))]
    row_end = min(len(data["rows"]), data["row_offset"] + VISIBLE_ROWS)
    col_start = data["col_offset"]
    col_end = col_start + VISIBLE_COLS
    for idx in range(data["row_offset"], row_end):
        row = data["rows"][idx]
        lines.append(" | ".join(row[col_start:col_end]))
    body = "\n".join(lines) if lines else data["error"] or "Empty CSV."
    return Column(
        [
            AppBar(name, f"{len(data['rows'])} rows x {len(data['headers'])} columns"),
            Scrollable(Text(body, size=12.0)),
            Spacer(grow=True),
            FooterKeys([("j/k", "rows"), ("h/l", "cols"), ("esc", "back")]),
        ],
        grow=True,
        padding=0,
    )


def _status(data: dict) -> str:
    if data["mode"] == "detail":
        return f"{len(data['rows'])} rows x {len(data['headers'])} columns"
    return f"{len(data['files'])} CSV files"


def _clamp(selected: int, total: int) -> int:
    if total <= 0:
        return 0
    return max(0, min(selected, total - 1))


def _max_row_offset(data: dict) -> int:
    return max(0, len(data["rows"]) - VISIBLE_ROWS)


def _max_col_offset(data: dict) -> int:
    return max(0, len(data["headers"]) - VISIBLE_COLS)
