#!/usr/bin/env python3
from __future__ import annotations

import threading
from dataclasses import dataclass

from plexi_sdk import App, RenderContext, CapabilityDeniedError, AiResponse
from plexi_sdk.ui import (
    AppBar,
    ChatBubble,
    Column,
    Divider,
    Label,
    Scrollable,
    Spacer,
    TextInput,
)

SYSTEM_PROMPT = (
    "You are a helpful, concise assistant. "
    "Answer clearly without unnecessary padding."
)


@dataclass
class Turn:
    role: str   # "user" | "assistant" | "error"
    text: str


class ChatApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._turns: list[Turn] = []
        self._in_flight: bool = False
        self._scroll = Scrollable(child=Column([]))
        self._input = TextInput("chat-input", multiline=True, height=64.0)
        self._scroll_to_bottom_next: bool = False
        ctx.status_summary("Chat")
        self.emit.info("chat-poc started")

    def _maybe_scroll_to_bottom(self) -> None:
        if not self._scroll_to_bottom_next:
            return
        self._scroll_to_bottom_next = False
        self._scroll.scroll_offset = 1_000_000.0

    def _send(self, text: str) -> None:
        if not text.strip() or self._in_flight:
            return

        self._turns.append(Turn("user", text))
        self._in_flight = True
        self._scroll_to_bottom_next = True
        self.emit.schedule_render(after_ms=20)

        history = [{"role": t.role, "content": t.text}
                   for t in self._turns if t.role in ("user", "assistant")]

        def runner() -> None:
            try:
                resp: AiResponse = self.emit.run_sync(self.emit.ai_query(
                    model_tier="low",
                    system=SYSTEM_PROMPT,
                    messages=history,
                ))
                self._turns.append(Turn("assistant", resp.content))
                self.emit.info(f"chat: {resp.tokens_in}/{resp.tokens_out} tokens")
            except CapabilityDeniedError as e:
                self._turns.append(Turn("error", f"Capability denied: {e}"))
                self.emit.warn(f"chat: capability denied: {e}")
            except Exception as e:
                self._turns.append(Turn("error", str(e)))
                self.emit.warn(f"chat: query failed: {e}")
            finally:
                self._in_flight = False
                self._scroll_to_bottom_next = True
                self.emit.schedule_render(after_ms=20)

        threading.Thread(target=runner, daemon=True).start()

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        self._scroll.handle_key(key)

    def _build_history(self) -> Column:
        children: list = []
        for turn in self._turns:
            children.append(ChatBubble(turn.text, role=turn.role))
            children.append(Spacer(size=6.0))
        if self._in_flight:
            children.append(Label("thinking…", tone="hint"))
        if not children:
            children.append(Label("Type a message below to start chatting.", tone="hint"))
        return Column(children, padding=0, gap=0)

    def on_render(self, ctx: RenderContext) -> None:
        self._scroll.child = self._build_history()
        self._input.placeholder = (
            "Waiting for response…" if self._in_flight
            else "Type a message… (Shift+Enter for newline)"
        )
        ctx.render(Column([
            AppBar(title="Chat"),
            self._scroll,
            Divider(),
            self._input,
        ]))

        if self._input.submitted is not None and not self._in_flight:
            self._send(self._input.submitted)

        self._maybe_scroll_to_bottom()


ChatApp().run()
