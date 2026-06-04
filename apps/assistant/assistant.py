#!/usr/bin/env python3
"""Assistant — AI chat app powered by Plexi's ai.query broker.

L1 Reference Implementation
============================
Patterns demonstrated:
  - ChatBubble for user/assistant/error messages
  - Scrollable container for conversation history
  - Streaming via on_ai_stream_chunk + schedule_render
  - Background ai_query with emit.run_sync
  - Column + AppBar + Scrollable + TextInput + FooterKeys
"""

import threading

from plexi_sdk import App, RenderContext
from plexi_sdk.ui import (
    Column, AppBar, ChatBubble, Scrollable, Spacer,
    FooterKeys, TextInput, Label,
)

SYSTEM_PROMPT = "You are a helpful assistant. Be concise and clear."
MODEL_TIER = "medium"


class AssistantApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._messages: list[dict] = []  # {role, content}
        self._streaming_text = ""
        self._is_streaming = False
        self._input = TextInput("chat-input", placeholder="Type a message...", height=48.0)
        self._scroll = Scrollable(child=Spacer())  # replaced each render
        ctx.info("AssistantApp ready")

    def on_ai_stream_chunk(self, _request_id: str, delta: str, _done: bool) -> None:
        self._streaming_text += delta
        self.emit.schedule_render()

    def _send_query(self) -> None:
        """Run ai_query on a background thread."""
        messages = [{"role": m["role"], "content": m["content"]} for m in self._messages]
        try:
            resp = self.emit.run_sync(
                self.emit.ai_query(
                    model_tier=MODEL_TIER,
                    system=SYSTEM_PROMPT,
                    messages=messages,
                )
            )
            # Streaming chunks already built _streaming_text; use final content
            # if streaming produced nothing (host might not stream).
            if not self._streaming_text and resp.content:
                self._streaming_text = resp.content
            self._messages.append({"role": "assistant", "content": self._streaming_text})
        except Exception as e:
            self.emit.error(f"ai_query failed: {e}")
            self._messages.append({"role": "assistant", "content": f"Error: {e}"})
        finally:
            self._is_streaming = False
            self._streaming_text = ""
            self.emit.schedule_render()

    def _submit(self, text: str) -> None:
        if self._is_streaming:
            return
        text = text.strip()
        if not text:
            return
        self._messages.append({"role": "user", "content": text})
        self._is_streaming = True
        self._streaming_text = ""
        threading.Thread(target=self._send_query, daemon=True).start()
        self.emit.schedule_render()

    def _build_chat_column(self) -> Column:
        """Build a Column of ChatBubble nodes for the message history."""
        children = []
        for msg in self._messages:
            role = "user" if msg["role"] == "user" else "assistant"
            children.append(ChatBubble(text=msg["content"], role=role, max_lines=100))
        if self._is_streaming and self._streaming_text:
            children.append(ChatBubble(
                text=self._streaming_text, role="assistant", max_lines=100,
            ))
        elif self._is_streaming:
            children.append(Label("Thinking...", tone="hint"))
        if not children:
            children.append(Label("Send a message to start.", tone="hint"))
        return Column(children, padding=0.0, padding_top=0.0, gap=8.0)

    def on_render(self, ctx: RenderContext) -> None:
        self._scroll.child = self._build_chat_column()
        # Auto-scroll to bottom on new content
        self._scroll.scroll_offset = max(0.0, self._scroll._child_h - self._scroll._avail_h)

        subtitle = f"model: {MODEL_TIER}"
        footer_keys = [("Enter", "send"), ("Esc", "quit")]
        if self._is_streaming:
            footer_keys = [("...", "streaming"), ("Esc", "quit")]

        ctx.render(Column([
            AppBar(title="Assistant", subtitle=subtitle),
            self._scroll,
            self._input,
            FooterKeys(footer_keys),
        ]))

        submitted = self._input.submitted
        if submitted is not None:
            self._submit(submitted)

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        if self._scroll.handle_key(key):
            self.emit.schedule_render()


if __name__ == "__main__":
    AssistantApp().run()
