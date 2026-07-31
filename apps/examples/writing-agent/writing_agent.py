"""V3 example: a writing assistant using declarative UI and effects."""

from plexi_sdk import state
from plexi_sdk.effects import AiMessage, AiQuery, SetState, SetTitle
from plexi_sdk.events import AiResponse, UiAction, UiValueChange
from plexi_sdk.ui import AppBar, Button, Column, FooterKeys, Label, TextEdit

SYSTEM_PROMPT = "You are a precise writing editor. Preserve the author's voice."


def init(_size, _args):
    return [SetTitle("Writing Agent"), SetState({"prompt": "", "answer": ""})]


def update(event):
    if isinstance(event, UiValueChange) and event.handler_id == "prompt":
        return [SetState({"prompt": event.value})]
    if isinstance(event, UiAction) and event.handler_id in ("prompt", "ask"):
        prompt = state.get("prompt", "").strip()
        if prompt:
            return [AiQuery("writing-agent-query", "medium", SYSTEM_PROMPT,
                            [AiMessage("user", prompt)])]
    if isinstance(event, AiResponse) and event.request_id == "writing-agent-query":
        return [SetState({"answer": event.error or event.content or "No response."})]
    return []


def view():
    return Column([
        AppBar("Writing Agent", subtitle="V3 AI-query example"),
        TextEdit("prompt", value=state.get("prompt", ""),
                 placeholder="Paste text or describe what you are writing...", multiline=True),
        Button("Edit", on_click="ask", style="primary"),
        Label(state.get("answer", "Paste a draft to get focused editorial feedback."), max_lines=20),
        FooterKeys([("enter", "submit text"), ("ask", "send query")]),
    ], grow=True)
