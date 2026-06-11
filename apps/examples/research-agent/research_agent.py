#!/usr/bin/env python3
"""Research Agent — AI assistant specialized for research, summarization, and analysis.

L1 Example App
==============
Demonstrates:
  - Specialized system prompt (research analyst persona)
  - Citation-aware responses (analyst is prompted to cite claims)
  - Streaming via on_ai_stream_chunk + schedule_render
  - Session persistence via JSON in .plexi/research_agent_sessions/
  - Same patterns as apps/dev/assistant-pgap/ with a focused persona
"""

import json
import os
import threading
from datetime import datetime, timezone
from pathlib import Path

from plexi_sdk import App, RenderContext
from plexi_sdk.ui import (
    Column, AppBar, ChatBubble, Scrollable, Spacer,
    FooterKeys, TextInput, Label,
)

SYSTEM_PROMPT = (
    "You are a research analyst with expertise in synthesizing information across "
    "domains. Your strengths are: summarizing complex topics clearly, identifying "
    "key claims and their supporting evidence, noting where sources conflict or "
    "where evidence is weak, and structuring findings in a scannable format. "
    "When you make factual claims, indicate whether they are well-established, "
    "contested, or uncertain. Use structured output (bullet points, numbered lists, "
    "headers) to make your summaries easy to scan. Flag gaps in your knowledge "
    "honestly rather than speculating. When asked to research a topic, start with "
    "a brief overview, then go deeper on request."
)
MODEL_TIER = "medium"


class ResearchAgentApp(App):
    def on_init(self) -> None:
        self._messages: list[dict] = []
        self._streaming_text = ""
        self._is_streaming = False
        self._input = TextInput("chat-input", placeholder="Ask about a topic to research...", height=48.0)
        self._scroll = Scrollable(child=Spacer())
        self._session_id = _new_session_id()
        self._sessions_dir: Path | None = _resolve_sessions_dir(self.workspace_root)
        self._load_latest_session()
        self.emit.info(f"ResearchAgentApp ready — sessions dir: {self._sessions_dir}")

    # ── Session persistence ──────────────────────────────────────────────────

    def _load_latest_session(self) -> None:
        if self._sessions_dir is None:
            return
        try:
            files = sorted(self._sessions_dir.glob("*.json"), reverse=True)
            if not files:
                return
            latest = files[0]
            data = json.loads(latest.read_text(encoding="utf-8"))
            messages = data.get("messages", [])
            if not isinstance(messages, list):
                raise ValueError("messages is not a list")
            self._messages = messages
            self._session_id = latest.stem
            self.emit.info(f"Resumed session {self._session_id} ({len(messages)} messages)")
        except Exception as e:
            self.emit.warn(f"Could not load session (starting fresh): {e}")
            self._messages = []
            self._session_id = _new_session_id()

    def _save_session(self) -> None:
        if self._sessions_dir is None:
            return
        try:
            self._sessions_dir.mkdir(parents=True, exist_ok=True)
            path = self._sessions_dir / f"{self._session_id}.json"
            payload = {
                "session_id": self._session_id,
                "created": self._session_id,
                "model_tier": MODEL_TIER,
                "messages": self._messages,
            }
            path.write_text(json.dumps(payload, indent=2, ensure_ascii=False), encoding="utf-8")
        except Exception as e:
            self.emit.warn(f"Could not save session: {e}")

    def _new_conversation(self) -> None:
        if self._is_streaming:
            return
        self._messages = []
        self._session_id = _new_session_id()
        self.emit.info(f"New session started: {self._session_id}")
        self.emit.schedule_render()

    # ── AI query ─────────────────────────────────────────────────────────────

    def on_ai_stream_chunk(self, _request_id: str, delta: str, _done: bool) -> None:
        self._streaming_text += delta
        self.emit.schedule_render()

    def _send_query(self) -> None:
        messages = [{"role": m["role"], "content": m["content"]} for m in self._messages]
        try:
            resp = self.emit.run_sync(
                self.emit.ai_query(
                    model_tier=MODEL_TIER,
                    system=SYSTEM_PROMPT,
                    messages=messages,
                )
            )
            if not self._streaming_text and resp.content:
                self._streaming_text = resp.content
            self._messages.append({"role": "assistant", "content": self._streaming_text})
            self._save_session()
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

    # ── Rendering ─────────────────────────────────────────────────────────────

    def _build_chat_column(self) -> Column:
        children = []
        for msg in self._messages:
            role = "user" if msg["role"] == "user" else "assistant"
            children.append(ChatBubble(text=msg["content"], role=role, max_lines=200))
        if self._is_streaming and self._streaming_text:
            children.append(ChatBubble(
                text=self._streaming_text, role="assistant", max_lines=200,
            ))
        elif self._is_streaming:
            children.append(Label("Researching...", tone="hint"))
        if not children:
            children.append(Label("Ask about any topic — I'll summarize and cite key findings.", tone="hint"))
        return Column(children, padding=0.0, padding_top=0.0, gap=8.0)

    def on_render(self, ctx: RenderContext) -> None:
        self._scroll.child = self._build_chat_column()
        self._scroll.scroll_offset = max(0.0, self._scroll._child_h - self._scroll._avail_h)

        subtitle = f"model: {MODEL_TIER} | research analyst"
        footer_keys = [("Enter", "send"), ("N", "new"), ("Esc", "quit")]
        if self._is_streaming:
            footer_keys = [("...", "researching"), ("Esc", "quit")]

        ctx.render(Column([
            AppBar(title="Research Agent", subtitle=subtitle),
            self._scroll,
            self._input,
            FooterKeys(footer_keys),
        ]))

        submitted = self._input.submitted
        if submitted is not None:
            self._submit(submitted)

    def on_key(self, key: str, mods: dict) -> None:
        if self._scroll.handle_key(key):
            self.emit.schedule_render()
            return
        if key == "n" and mods.get("cmd"):
            self._new_conversation()


# ── Helpers ───────────────────────────────────────────────────────────────────

def _new_session_id() -> str:
    return datetime.now(tz=timezone.utc).strftime("%Y-%m-%dT%H-%M-%S")


def _resolve_sessions_dir(workspace_root: str) -> Path | None:
    if not workspace_root:
        return None
    root = Path(workspace_root)
    plexi_dir = root / ".plexi"
    if not plexi_dir.exists():
        script_dir = Path(os.path.abspath(__file__)).parent
        return script_dir / "sessions"
    return plexi_dir / "research_agent_sessions"


if __name__ == "__main__":
    ResearchAgentApp().run()
