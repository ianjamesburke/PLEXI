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
  - Session persistence via JSON in .plexi/assistant_sessions/
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

SYSTEM_PROMPT = "You are a helpful assistant. Be concise and clear."
MODEL_TIER = "medium"


class AssistantApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._messages: list[dict] = []  # {role, content}
        self._streaming_text = ""
        self._is_streaming = False
        self._input = TextInput("chat-input", placeholder="Type a message...", height=48.0)
        self._scroll = Scrollable(child=Spacer())  # replaced each render
        self._session_id = _new_session_id()
        self._sessions_dir: Path | None = _resolve_sessions_dir(ctx.workspace_root)
        self._load_latest_session()
        ctx.info(f"AssistantApp ready — sessions dir: {self._sessions_dir}")

    # ── Session persistence ────────────────────────────────────────────────────

    def _load_latest_session(self) -> None:
        """Load the most recent session file, or start fresh if none exists."""
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
        """Persist the current conversation to a JSON file."""
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
        """Start a fresh session, discarding in-memory history."""
        if self._is_streaming:
            return
        self._messages = []
        self._session_id = _new_session_id()
        self.emit.info(f"New session started: {self._session_id}")
        self.emit.schedule_render()

    # ── AI query ───────────────────────────────────────────────────────────────

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

    # ── Rendering ──────────────────────────────────────────────────────────────

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
        footer_keys = [("Enter", "send"), ("N", "new"), ("Esc", "quit")]
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

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if self._scroll.handle_key(key):
            self.emit.schedule_render()
            return
        # Cmd+N or bare 'n' (when not focused on input) starts a new conversation.
        if key == "n" and mods.get("cmd"):
            self._new_conversation()


# ── Helpers ────────────────────────────────────────────────────────────────────

def _new_session_id() -> str:
    """Generate a session ID from the current UTC timestamp."""
    return datetime.now(tz=timezone.utc).strftime("%Y-%m-%dT%H-%M-%S")


def _resolve_sessions_dir(workspace_root: str) -> Path | None:
    """Return .plexi/assistant_sessions/ under workspace_root, or None if unavailable."""
    if not workspace_root:
        return None
    root = Path(workspace_root)
    plexi_dir = root / ".plexi"
    # Only use workspace storage if .plexi/ already exists (i.e. workspace is initialised).
    if not plexi_dir.exists():
        # Fall back to a dir next to the script so the app still persists.
        script_dir = Path(os.path.abspath(__file__)).parent
        return script_dir / "sessions"
    return plexi_dir / "assistant_sessions"


if __name__ == "__main__":
    AssistantApp().run()
