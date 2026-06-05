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
  - ExposeTools: registers an ask_assistant tool so other workspace apps
    can invoke the assistant programmatically (#2034)
  - Tool consumption: ai_query automatically receives tools exposed by
    other panes in the same workspace via the host tool dispatcher
"""

import json
import os
import subprocess
import threading
from datetime import datetime, timezone
from pathlib import Path

PLEXI_BINARY = os.environ.get("PLEXI_BINARY", "plexi-alpha")

from plexi_sdk import App, RenderContext
from plexi_sdk.ui import (
    Column, AppBar, ChatBubble, Scrollable, Spacer,
    FooterKeys, TextInput, Label,
)

SYSTEM_PROMPT = (
    "You are a helpful assistant. Be concise and clear. "
    "You have access to tools offered by other running Plexi apps in this workspace — "
    "use them when they help answer the user's request."
)
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
        self._register_tools()
        self._register_cli_tools()
        ctx.info(f"AssistantApp ready — sessions dir: {self._sessions_dir}")

    # ── Tool registration (ExposeTools) ────────────────────────────────────────

    def _register_tools(self) -> None:
        """Register tools this app exposes to other workspace apps via ExposeTools.

        The ask_assistant tool lets any other PGAP app in the same workspace
        send a plain-text question to the assistant and receive a response
        without requiring a full UI interaction.
        """
        @self.tool(
            "ask_assistant",
            description=(
                "Send a question to the AI assistant and get a response. "
                "Use this to query the assistant from another app without opening the chat UI."
            ),
            schema={
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "The question or instruction to send to the assistant.",
                    },
                },
                "required": ["question"],
            },
            timeout_ms=35000,
        )
        async def handle_ask_assistant(args: dict) -> dict:
            question = args.get("question", "").strip()
            if not question:
                return {"error": "question must not be empty"}
            self.emit.info(f"ask_assistant tool invoked: {question[:80]!r}")
            try:
                resp = await self.emit.ai_query(
                    model_tier=MODEL_TIER,
                    system=SYSTEM_PROMPT,
                    messages=[{"role": "user", "content": question}],
                )
                answer = resp.content or ""
                return {"answer": answer}
            except Exception as exc:
                self.emit.error(f"ask_assistant ai_query failed: {exc}")
                return {"error": str(exc)}

        self.emit.info("AssistantApp: exposed ask_assistant tool to workspace")

    def _register_cli_tools(self) -> None:
        """Register CLI tools that let the AI agent control the Plexi workspace.

        Tools registered:
          - open_terminal: spawn a new terminal pane (optionally running a command)
          - open_app: open a Plexi app by ID in a new pane
          - list_panes: return the current pane list as structured JSON
          - send_command: send a text command to a specific pane
        """

        @self.tool(
            "open_terminal",
            description=(
                "Spawn a new terminal pane in the Plexi workspace. "
                "Optionally run a shell command in it."
            ),
            schema={
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command to run in the terminal (optional).",
                    },
                },
                "required": [],
            },
        )
        async def handle_open_terminal(args: dict) -> dict:
            cmd = [PLEXI_BINARY, "terminal"]
            command_arg = args.get("command", "").strip()
            if command_arg:
                cmd += ["--command", command_arg]
            self.emit.info(f"open_terminal: {cmd}")
            try:
                result = subprocess.run(cmd, capture_output=True, text=True, timeout=10)
                if result.returncode != 0:
                    err = result.stderr.strip() or result.stdout.strip()
                    self.emit.error(f"open_terminal failed (rc={result.returncode}): {err}")
                    return {"error": err}
                pane_id = result.stdout.strip()
                return {"pane_id": pane_id}
            except Exception as exc:
                self.emit.error(f"open_terminal exception: {exc}")
                return {"error": str(exc)}

        @self.tool(
            "open_app",
            description="Open a Plexi app by its app ID in a new pane.",
            schema={
                "type": "object",
                "properties": {
                    "app_id": {
                        "type": "string",
                        "description": "The app ID to open (e.g. 'calculator', 'file-browser').",
                    },
                },
                "required": ["app_id"],
            },
        )
        async def handle_open_app(args: dict) -> dict:
            app_id = args.get("app_id", "").strip()
            if not app_id:
                return {"error": "app_id must not be empty"}
            cmd = [PLEXI_BINARY, "app", "open", app_id]
            self.emit.info(f"open_app: {cmd}")
            try:
                result = subprocess.run(cmd, capture_output=True, text=True, timeout=10)
                if result.returncode != 0:
                    err = result.stderr.strip() or result.stdout.strip()
                    self.emit.error(f"open_app failed (rc={result.returncode}): {err}")
                    return {"error": err}
                pane_id = result.stdout.strip()
                return {"pane_id": pane_id}
            except Exception as exc:
                self.emit.error(f"open_app exception: {exc}")
                return {"error": str(exc)}

        @self.tool(
            "list_panes",
            description="Return the current list of open panes in the Plexi workspace.",
            schema={
                "type": "object",
                "properties": {},
                "required": [],
            },
        )
        async def handle_list_panes(_args: dict) -> dict:
            cmd = [PLEXI_BINARY, "pane", "list", "--json"]
            self.emit.info("list_panes: fetching pane list")
            try:
                result = subprocess.run(cmd, capture_output=True, text=True, timeout=10)
                if result.returncode != 0:
                    err = result.stderr.strip() or result.stdout.strip()
                    self.emit.error(f"list_panes failed (rc={result.returncode}): {err}")
                    return {"error": err}
                try:
                    panes = json.loads(result.stdout)
                except json.JSONDecodeError:
                    panes = result.stdout.strip()
                return {"panes": panes}
            except Exception as exc:
                self.emit.error(f"list_panes exception: {exc}")
                return {"error": str(exc)}

        @self.tool(
            "send_command",
            description="Send a text string as input to a specific pane by ID.",
            schema={
                "type": "object",
                "properties": {
                    "pane_id": {
                        "type": "string",
                        "description": "The target pane ID.",
                    },
                    "text": {
                        "type": "string",
                        "description": "The text/command to send to the pane.",
                    },
                },
                "required": ["pane_id", "text"],
            },
        )
        async def handle_send_command(args: dict) -> dict:
            pane_id = args.get("pane_id", "").strip()
            text = args.get("text", "")
            if not pane_id:
                return {"error": "pane_id must not be empty"}
            if not text:
                return {"error": "text must not be empty"}
            cmd = [PLEXI_BINARY, "pane", "command", pane_id, text]
            self.emit.info(f"send_command: pane={pane_id!r} text={text[:60]!r}")
            try:
                result = subprocess.run(cmd, capture_output=True, text=True, timeout=10)
                if result.returncode != 0:
                    err = result.stderr.strip() or result.stdout.strip()
                    self.emit.error(f"send_command failed (rc={result.returncode}): {err}")
                    return {"ok": False, "error": err}
                return {"ok": True}
            except Exception as exc:
                self.emit.error(f"send_command exception: {exc}")
                return {"ok": False, "error": str(exc)}

        self.emit.info("AssistantApp: registered CLI tools (open_terminal, open_app, list_panes, send_command)")

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
        """Run ai_query on a background thread.

        The host tool dispatcher automatically injects tools exposed by other
        panes in the same workspace into the ai_query call. The broker handles
        the tool call loop internally, so this method receives the final
        resolved response after any tool rounds complete.
        """
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

    def on_key(self, _ctx: RenderContext, key: str, mods: dict) -> None:
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
