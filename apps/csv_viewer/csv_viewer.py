#!/usr/bin/env python3
"""CSV Viewer - SDK v3 host-file-list CSV browser."""

from __future__ import annotations

import csv
import io
from pathlib import Path

import plexi_sdk as sdk
from plexi_sdk import log, state
from plexi_sdk.effects import FileList, FileRead, RequestCapability, SetState, SetStatus, SetTitle
from plexi_sdk.events import (
    CapabilityDenied,
    CapabilityGranted,
    FileListResult,
    FileReadResult,
    KeyEvent,
    UiAction,
    UiValueChange,
)
from plexi_sdk.ui import AppBar, Button, Column, FooterKeys, Scrollable, SelectList, Text, TextEdit

VISIBLE_ROWS = 24
VISIBLE_COLS = 5

DEFAULT_STATE = {
    "dir_input": "",
    "cwd": "",
    "files": [],
    "selected": 0,
    "pending_capability": "",
    "pending_action": "",
    "pending_path": "",
    "mode": "list",
    "path": "",
    "headers": [],
    "rows": [],
    "row_offset": 0,
    "col_offset": 0,
    "loading": False,
    "error": "",
}


def init(size, args) -> list:
    data = _state()
    target = Path(args[0]).expanduser() if args else _base_dir()
    if target.suffix.lower() == ".csv":
        data["dir_input"] = str(target.parent)
        effects = _request_open(data, str(target))
    else:
        data["dir_input"] = str(target)
        effects = _request_list(data, str(target))
    log.info("csv_viewer: SDK v3 initialized")
    return [SetTitle("CSV Viewer"), *effects]


def update(event) -> list:
    data = _state()

    if isinstance(event, UiValueChange) and event.handler_id == "csv-dir":
        data["dir_input"] = event.value
        data["error"] = ""
        return [SetState(data)]

    if isinstance(event, UiAction):
        if event.handler_id == "csv-dir" or event.handler_id == "csv-refresh":
            return _request_list(data, data["dir_input"])
        if event.handler_id == "csv-back-list":
            data["mode"] = "list"
            return _set(data)

    if isinstance(event, CapabilityGranted) and event.name == data["pending_capability"]:
        if data["pending_action"] == "list":
            return [SetStatus("Listing CSV files"), FileList(data["pending_path"], ["csv"])]
        if data["pending_action"] == "read":
            return [SetStatus("Loading CSV"), FileRead(data["pending_path"])]

    if isinstance(event, CapabilityDenied) and event.name == data["pending_capability"]:
        data["loading"] = False
        data["error"] = "File access denied."
        data["pending_capability"] = ""
        data["pending_action"] = ""
        data["pending_path"] = ""
        return _set(data)

    if isinstance(event, FileListResult):
        entries = event.entries or []
        data["files"] = [
            {
                "name": entry.name,
                "path": entry.path,
                "description": _file_hint(entry.size_bytes),
            }
            for entry in entries
            if not entry.is_dir and entry.name.lower().endswith(".csv")
        ]
        data["selected"] = _clamp(data["selected"], len(data["files"]))
        data["cwd"] = data["pending_path"]
        data["dir_input"] = data["cwd"]
        data["pending_capability"] = ""
        data["pending_action"] = ""
        data["pending_path"] = ""
        data["loading"] = False
        data["error"] = event.error or ("" if data["files"] else "No CSV files found.")
        data["mode"] = "list"
        log.info(f"csv_viewer: listed cwd={data['cwd']} files={len(data['files'])}")
        return _set(data)

    if isinstance(event, FileReadResult):
        data = _handle_file(data, event)
        return _set(data)

    if not isinstance(event, KeyEvent) or not event.pressed:
        return []

    key = event.key
    if data["mode"] == "list":
        if key in ("up", "k"):
            data["selected"] = _clamp(data["selected"] - 1, len(data["files"]))
        elif key in ("down", "j"):
            data["selected"] = _clamp(data["selected"] + 1, len(data["files"]))
        elif key in ("return", "enter") and data["files"]:
            return _request_open(data, data["files"][data["selected"]]["path"])
        elif key == "r":
            return _request_list(data, data["dir_input"])
        else:
            return []
        return _set(data)

    if data["mode"] == "detail":
        if key == "escape":
            data["mode"] = "list"
        elif key in ("up", "k"):
            data["row_offset"] = max(0, data["row_offset"] - 1)
        elif key in ("down", "j"):
            data["row_offset"] = min(_max_row_offset(data), data["row_offset"] + 1)
        elif key in ("left", "h"):
            data["col_offset"] = max(0, data["col_offset"] - 1)
        elif key in ("right", "l"):
            data["col_offset"] = min(_max_col_offset(data), data["col_offset"] + 1)
        else:
            return []
        return _set(data)
    return []


def view():
    data = _state()
    if data["mode"] == "detail":
        return _detail_view(data)
    return _list_view(data)


def _state() -> dict:
    data = dict(DEFAULT_STATE)
    for key, value in DEFAULT_STATE.items():
        data[key] = state.get(key, value)
    data["files"] = [dict(item) for item in data.get("files") or []]
    data["headers"] = [str(item) for item in data.get("headers") or []]
    data["rows"] = [[str(cell) for cell in row] for row in data.get("rows") or []]
    data["selected"] = _clamp(int(data.get("selected") or 0), len(data["files"]))
    data["row_offset"] = max(0, int(data.get("row_offset") or 0))
    data["col_offset"] = max(0, int(data.get("col_offset") or 0))
    data["loading"] = bool(data.get("loading"))
    data["mode"] = data.get("mode") if data.get("mode") in ("list", "detail") else "list"
    for key in ("dir_input", "cwd", "pending_capability", "pending_action", "pending_path", "path", "error"):
        data[key] = str(data.get(key) or "")
    return data


def _request_list(data: dict, raw_path: str) -> list:
    path = Path(str(raw_path or "").strip() or ".").expanduser()
    if not path.is_absolute():
        path = _base_dir() / path
    data["pending_path"] = str(path)
    data["pending_capability"] = "fs.read"
    data["pending_action"] = "list"
    data["loading"] = True
    data["error"] = ""
    return [SetState(data), SetStatus("Requesting file access"), RequestCapability(data["pending_capability"])]


def _request_open(data: dict, raw_path: str) -> list:
    path = Path(raw_path).expanduser()
    if not path.is_absolute():
        path = _base_dir() / path
    data["pending_path"] = str(path)
    data["pending_capability"] = "fs.read"
    data["pending_action"] = "read"
    data["loading"] = True
    data["error"] = ""
    return [SetState(data), SetStatus("Requesting file access"), RequestCapability(data["pending_capability"])]


def _handle_file(data: dict, event: FileReadResult) -> dict:
    data["loading"] = False
    data["pending_capability"] = ""
    data["pending_action"] = ""
    if event.error:
        data["error"] = event.error
        return data
    content = event.content or b""
    try:
        loaded = list(csv.reader(io.StringIO(content.decode("utf-8-sig", errors="replace"))))
    except csv.Error as exc:
        data["error"] = f"CSV parse error: {exc}"
        return data
    data["path"] = data["pending_path"]
    data["pending_path"] = ""
    data["headers"] = loaded[0] if loaded else []
    data["rows"] = loaded[1:]
    data["row_offset"] = 0
    data["col_offset"] = 0
    data["mode"] = "detail"
    data["error"] = "" if loaded else "Empty CSV."
    log.info(f"csv_viewer: loaded path={data['path']} rows={len(data['rows'])}")
    return data


def _list_view(data: dict):
    body = (
        SelectList(data["files"], selected_idx=data["selected"])
        if data["files"]
        else Text(data["error"] or "No CSV files found.", size=12.0)
    )
    return Column(
        [
            AppBar("CSV Viewer", data["cwd"] or "choose folder"),
            TextEdit("csv-dir", value=data["dir_input"], placeholder="/path/to/folder"),
            Button("Refresh", "csv-refresh", style="primary"),
            body,
            FooterKeys([("j/k", "select"), ("enter", "open"), ("r", "refresh")]),
        ],
        grow=True,
        padding=0,
    )


def _detail_view(data: dict):
    name = Path(data["path"]).name or "CSV"
    header = " | ".join(data["headers"][data["col_offset"] : data["col_offset"] + VISIBLE_COLS])
    lines = [header or "(no headers)", "-" * min(120, max(3, len(header)))]
    for idx in range(data["row_offset"], min(len(data["rows"]), data["row_offset"] + VISIBLE_ROWS)):
        row = data["rows"][idx]
        lines.append(" | ".join(row[data["col_offset"] : data["col_offset"] + VISIBLE_COLS]))
    return Column(
        [
            AppBar(name, f"{len(data['rows'])} rows x {len(data['headers'])} columns"),
            Scrollable(Text("\n".join(lines), size=12.0)),
            Button("Back", "csv-back-list", style="ghost"),
            FooterKeys([("j/k", "rows"), ("h/l", "cols"), ("esc", "list")]),
        ],
        grow=True,
        padding=0,
    )


def _set(data: dict) -> list:
    return [SetState(data), SetStatus(_status(data))]


def _base_dir() -> Path:
    root = getattr(sdk, "_workspace_root", "") or ""
    return Path(root).expanduser() if root else Path.home()


def _status(data: dict) -> str:
    if data["loading"]:
        return "Loading"
    if data["mode"] == "detail":
        return f"{len(data['rows'])} rows x {len(data['headers'])} columns"
    return f"{len(data['files'])} CSV files" if not data["error"] else "Error"


def _file_hint(size: int | None) -> str:
    if size is None:
        return ""
    kb = size / 1024
    return f"{kb:.1f} KB" if kb < 1024 else f"{kb / 1024:.1f} MB"


def _clamp(selected: int, total: int) -> int:
    if total <= 0:
        return 0
    return max(0, min(selected, total - 1))


def _max_row_offset(data: dict) -> int:
    return max(0, len(data["rows"]) - VISIBLE_ROWS)


def _max_col_offset(data: dict) -> int:
    return max(0, len(data["headers"]) - VISIBLE_COLS)
