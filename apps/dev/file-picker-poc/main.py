"""File Picker POC (stint 0508).

Exercises `OpenFilePicker` end to end: save-as pick -> FileWrite through the
picker grant -> open pick -> FileRead back -> a cancelled pick. Drive it
manually (s / o / f keys) or pass --auto to run the whole round trip without
input — tests/scenes/file-picker.toml does that against a scripted picker
backend, so no dialog ever opens.
"""

from plexi_sdk import log, state
from plexi_sdk.effects import (
    FileRead,
    FileWrite,
    OpenFilePicker,
    SetState,
    SetTitle,
)
from plexi_sdk.events import (
    FilePickCancelled,
    FilePicked,
    FileReadResult,
    FileWriteResult,
    KeyEvent,
)
from plexi_sdk.ui import AppBar, Column, FooterKeys, Text

PAYLOAD = "hello from the picker round trip"


def init(size, args):
    auto = "--auto" in list(args)
    effects = [
        SetTitle("File Picker POC"),
        SetState({"auto": auto, "status": "idle", "picked": "", "content": ""}),
    ]
    if auto:
        log.info("file-picker-poc: starting auto round trip")
        effects += [
            SetState({"status": "picking-save"}),
            OpenFilePicker(request_id="save-1", mode="save"),
        ]
    return effects


def update(event):
    if isinstance(event, KeyEvent) and event.pressed:
        if event.key == "s":
            return [
                SetState({"status": "picking-save"}),
                OpenFilePicker(request_id="save-1", mode="save"),
            ]
        if event.key == "o":
            return [
                SetState({"status": "picking-open"}),
                OpenFilePicker(request_id="open-1"),
            ]
        if event.key == "f":
            return [
                SetState({"status": "picking-folder"}),
                OpenFilePicker(request_id="folder-1", mode="folder"),
            ]
        return []
    if isinstance(event, FilePicked):
        path = event.paths[0]
        log.info(f"file-picker-poc: picked {path} for {event.request_id}")
        if event.request_id == "save-1":
            return [
                SetState({"status": "writing", "picked": path}),
                FileWrite(path, PAYLOAD.encode()),
            ]
        if event.request_id == "open-1":
            return [
                SetState({"status": "reading", "picked": path}),
                FileRead(path),
            ]
        return [SetState({"status": "picked", "picked": path})]
    if isinstance(event, FilePickCancelled):
        log.info(f"file-picker-poc: pick {event.request_id} cancelled")
        return [SetState({"status": f"cancelled:{event.request_id}"})]
    if isinstance(event, FileWriteResult):
        if event.error:
            return [SetState({"status": f"write-error:{event.error}"})]
        if state.get("auto"):
            return [
                SetState({"status": "picking-open"}),
                OpenFilePicker(request_id="open-1"),
            ]
        return [SetState({"status": "written"})]
    if isinstance(event, FileReadResult):
        if event.error:
            return [SetState({"status": f"read-error:{event.error}"})]
        content = (event.content or b"").decode("utf-8", "replace").strip()
        if state.get("auto"):
            return [
                SetState({"status": "roundtrip-complete", "content": content}),
                OpenFilePicker(request_id="cancel-1"),
            ]
        return [SetState({"status": "read", "content": content})]
    return []


def view():
    return Column(
        [
            AppBar("File Picker POC"),
            Text("status: " + str(state.get("status", "idle"))),
            Text("picked: " + str(state.get("picked", ""))),
            Text("content: " + str(state.get("content", ""))),
            FooterKeys([("s", "save-as"), ("o", "open"), ("f", "folder")]),
        ],
        grow=True,
    )
