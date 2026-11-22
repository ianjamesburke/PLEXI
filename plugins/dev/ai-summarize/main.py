"""AI Summarize: the `ai.query` example (stint 1450).

Type or paste text, pick Fast or Slow, and press Summarize. The plugin sends
one `AiQuery`; the host routes the mode to the user's `[models.fast]` or
`[models.slow]` model and answers with an `AiResponse`. A refusal (no grant,
no `[ai]` config, budget spent) arrives as `AiResponse.error` and is shown.

Guide: sdk/python/AUTHORING.md ("AI Calls") in the Plexi repo.
"""

from plexi_sdk import log, state
from plexi_sdk.effects import AiMessage, AiQuery, SetState, SetTitle
from plexi_sdk.events import AiResponse, UiAction, UiValueChange
from plexi_sdk.ui import (
    Banner,
    Button,
    Column,
    Component,
    FooterKeys,
    Radio,
    Scrollable,
    SurfaceHeader,
    Text,
    TextInput,
)

TEXT_INPUT = "text"
MODE = "mode"
SUMMARIZE = "summarize"
MODES = ["fast", "slow"]
SYSTEM = "Summarize the user's text in one or two plain sentences."


def init(size, args) -> list:
    return [
        SetTitle("AI Summarize"),
        SetState({
            "text": state.get("text", ""),
            "mode": state.get("mode", "fast"),
            "pending": None,
            "summary": "",
            "error": "",
            "asked": state.get("asked", 0),
        }),
    ]


def update(event) -> list:
    if isinstance(event, UiValueChange) and event.handler_id == TEXT_INPUT:
        return [SetState({"text": event.value})]
    if isinstance(event, UiAction) and event.handler_id.startswith(f"{MODE}:"):
        index = int(event.handler_id.split(":", 1)[1])
        return [SetState({"mode": MODES[index]})]
    if isinstance(event, UiAction) and event.handler_id in (SUMMARIZE, TEXT_INPUT):
        return _summarize()
    if isinstance(event, AiResponse) and event.request_id == state.get("pending"):
        log.info(f"AI Summarize: {event.request_id} error={event.error!r}")
        return [SetState({
            "pending": None,
            "summary": event.content or "",
            "error": event.error or "",
        })]
    return []


def _summarize() -> list:
    text = state.get("text", "").strip()
    if not text or state.get("pending"):
        return []
    asked = state.get("asked", 0) + 1
    request_id = f"summarize-{asked}"
    mode = state.get("mode", "fast")
    log.info(f"AI Summarize: {request_id} mode={mode}")
    return [
        SetState({"asked": asked, "pending": request_id, "summary": "", "error": ""}),
        AiQuery(request_id, mode, SYSTEM, [AiMessage("user", text)]),
    ]


def view() -> Component:
    body: list[Component] = [
        TextInput(
            TEXT_INPUT,
            value=state.get("text", ""),
            placeholder="Text to summarize",
            on_submit=SUMMARIZE,
        ),
        Radio(MODE, options=["Fast", "Slow"], selected=MODES.index(state.get("mode", "fast"))),
        Button("Summarize", on_click=SUMMARIZE),
    ]
    if state.get("pending"):
        body.append(Text(f"Asking the {state.get('mode', 'fast')} model...", color="muted"))
    if state.get("summary"):
        body.append(Text(state.get("summary", "")))
    if state.get("error"):
        body.append(Banner(state.get("error", ""), tone="danger", title="Refused"))
    return Column(
        [
            SurfaceHeader("AI Summarize"),
            Scrollable(Column(body)),
            FooterKeys([("Enter", "summarize")]),
        ],
        grow=True,
    )
