"""Cli Op Fixture: POC for approved typed CLI operations (stint 1175).

Keys: r runs a read operation, m a mutating one, s a slow one, c cancels the
request in flight. Each result lands in the view, so a person or a driver can
exercise approval, denial, remembered consent and revocation by key.
"""

from plexi_sdk import log, state
from plexi_sdk.effects import CliOperation, CliOperationCancel, SetState, SetTitle
from plexi_sdk.events import CliOperationResult, KeyEvent

from app import ui as app_ui

OPERATIONS = {
    "r": ("fixture.echo", {"word": "hello"}),
    "m": ("fixture.set", {"priority": "p1", "task": "1175"}),
    "s": ("fixture.slow", {"ms": 8000}),
}


def init(size, args):
    log.info("Cli Op Fixture: init")
    return [
        SetTitle("Cli Op Fixture"),
        SetState({"status": "ready", "seq": 0, "inflight": "", "last": ""}),
    ]


def update(event):
    if isinstance(event, KeyEvent) and event.pressed:
        if event.key in OPERATIONS and not state.get("inflight", ""):
            operation_id, values = OPERATIONS[event.key]
            request_id = f"req-{state.get('seq', 0) + 1}"
            log.info(f"Cli Op Fixture: requesting {operation_id} as {request_id}")
            return [
                SetState({
                    "seq": state.get("seq", 0) + 1,
                    "inflight": request_id,
                    "status": f"waiting for {request_id} ({operation_id})",
                }),
                CliOperation(request_id=request_id, operation_id=operation_id, values=values),
            ]
        if event.key == "c" and state.get("inflight", ""):
            request_id = state.get("inflight", "")
            return [
                CliOperationCancel(request_id=request_id),
                SetState({"status": f"cancelling {request_id}"}),
            ]
    if isinstance(event, CliOperationResult):
        outcome = "ok" if event.ok else f"failed: {event.error_code}"
        return [
            SetState({
                "inflight": "",
                "status": f"{event.request_id} {outcome}",
                "last": str(event.data) if event.ok else str(event.error),
            }),
            SetTitle(f"Cli Op Fixture: {outcome}"),
        ]
    return []


def view():
    return app_ui.build_view(state.get("status", ""), state.get("last", ""))
