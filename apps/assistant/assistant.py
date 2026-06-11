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
  - Host-mediated pane tools (stint 0014): the AI sees and drives other
    panes via capability-gated SDK calls (panes.spawn/read/control) —
    no subprocess, no ambient plexi CLI access
"""

import asyncio
import json
import os
import threading
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path

from plexi_sdk import App, CapabilityDeniedError
from plexi_sdk.ui import (
    Column, AppBar, ChatBubble, Scrollable, Spacer,
    FooterKeys, TextInput, Label,
)

SYSTEM_PROMPT = (
    "You are a helpful assistant. Be concise and clear. "
    "You have access to tools offered by other running Plexi apps in this workspace — "
    "use them when they help answer the user's request. "
    "You can also see and drive the user's panes: list_panes first, then "
    "get_pane_state (app panes) or capture_pane (terminal panes) to look inside, "
    "then send_app_action or send_command to act, then read the state again to "
    "verify the action worked."
)
MODEL_TIERS = ["low", "medium", "high"]
_THINKING_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]


class AssistantApp(App):
    def on_init(self) -> None:
        self._messages: list[dict] = []  # {role, content}
        self._streaming_text = ""
        self._is_streaming = False
        self._model_tier = "low"
        self._input = TextInput("chat-input", placeholder="Message assistant…", height=56.0)
        self._scroll = Scrollable(child=Spacer())  # replaced each render
        self._session_id = _new_session_id()
        self._sessions_dir: Path | None = _resolve_sessions_dir(self.workspace_root)
        self._spawn_waiters: dict[str, asyncio.Future] = {}
        self._load_latest_session()
        self._register_tools()
        self._register_pane_tools()
        self.emit.info(f"AssistantApp ready — model={self._model_tier} sessions dir: {self._sessions_dir}")

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
                    model_tier=self._model_tier,
                    system=SYSTEM_PROMPT,
                    messages=[{"role": "user", "content": question}],
                )
                answer = resp.content or ""
                return {"answer": answer}
            except Exception as exc:
                self.emit.error(f"ask_assistant ai_query failed: {exc}")
                return {"error": str(exc)}

        self.emit.info("AssistantApp: exposed ask_assistant tool to workspace")

    # ── Host-mediated pane tools (stint 0014) ──────────────────────────────────

    async def _call_with_capability(self, cap: str, op_name: str, attempt) -> dict:
        """Run an async pane operation, popping the host grant modal when needed.

        Pane capabilities are sensitive: even when declared in the manifest they
        are withheld until the user grants them. First use either raises
        CapabilityDeniedError (awaitable reads) or is silently dropped by the
        host (fire-and-forget control), so when the capability isn't held yet we
        request it up front via the host grant modal. A mid-flight denial gets
        one grant-and-retry; a user denial is returned to the model as an error.
        """
        if cap not in self.capabilities:
            try:
                await self.emit.capability_request(cap)
            except CapabilityDeniedError:
                self.emit.warn(f"{op_name}: user denied {cap}")
                return {"error": f"user denied {cap}"}
        try:
            return await attempt()
        except CapabilityDeniedError:
            pass
        except Exception as exc:
            self.emit.error(f"{op_name} failed: {exc}")
            return {"error": str(exc)}
        try:
            await self.emit.capability_request(cap)
        except CapabilityDeniedError:
            self.emit.warn(f"{op_name}: user denied {cap}")
            return {"error": f"user denied {cap}"}
        try:
            return await attempt()
        except Exception as exc:
            self.emit.error(f"{op_name} retry failed: {exc}")
            return {"error": str(exc)}

    async def _spawn_pane_and_wait(self, type_id: str,
                                   args: "list[str] | None" = None) -> dict:
        """spawn_pane + await the on_pane_spawned/on_pane_spawn_error callback."""
        request_id = str(uuid.uuid4())
        fut: asyncio.Future = asyncio.get_running_loop().create_future()
        self._spawn_waiters[request_id] = fut
        self.emit.spawn_pane(type_id, args=args, request_id=request_id)
        try:
            return await asyncio.wait_for(fut, timeout=10.0)
        except asyncio.TimeoutError:
            self.emit.error(f"spawn of {type_id!r} timed out")
            return {"error": f"spawn of {type_id!r} timed out after 10s"}
        finally:
            self._spawn_waiters.pop(request_id, None)

    def on_pane_spawned(self, pane_id: int, request_id: "str | None" = None) -> None:
        fut = self._spawn_waiters.get(request_id) if request_id else None
        if fut is not None and not fut.done():
            fut.set_result({"pane_id": pane_id})

    def on_pane_spawn_error(self, reason: str, request_id: "str | None" = None) -> None:
        fut = self._spawn_waiters.get(request_id) if request_id else None
        if fut is not None and not fut.done():
            fut.set_result({"error": reason})

    def _register_pane_tools(self) -> None:
        """Register host-mediated tools that let the AI see and drive panes.

        Everything goes through capability-gated PGAP requests — the app has
        no socket or subprocess route to the host. Capabilities used:
        panes.spawn (open_terminal, open_app), panes.read (list_panes,
        get_pane_state, capture_pane), panes.control (send_command,
        send_app_action, focus_pane).
        """

        @self.tool(
            "open_terminal",
            description=(
                "Spawn a new terminal pane in the Plexi workspace, optionally "
                "running a shell command in it. Returns the new pane_id — use "
                "capture_pane on it to read the command's output."
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
            command_arg = args.get("command", "").strip()
            self.emit.info(f"open_terminal tool: command={command_arg[:60]!r}")
            return await self._call_with_capability(
                "panes.spawn", "open_terminal",
                lambda: self._spawn_pane_and_wait(
                    "terminal", args=[command_arg] if command_arg else None
                ),
            )

        @self.tool(
            "open_app",
            description=(
                "Open a Plexi app by its app ID in a new pane. Returns the new "
                "pane_id — use get_pane_state on it to see the app's UI."
            ),
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
            self.emit.info(f"open_app tool: app_id={app_id!r}")
            return await self._call_with_capability(
                "panes.spawn", "open_app",
                lambda: self._spawn_pane_and_wait(app_id),
            )

        @self.tool(
            "list_panes",
            description=(
                "List all open panes in the Plexi workspace with their id, "
                "title, and type. Start here: pick a pane_id, then use "
                "get_pane_state (app panes) or capture_pane (terminal panes) "
                "to see what's inside it."
            ),
            schema={
                "type": "object",
                "properties": {},
                "required": [],
            },
        )
        async def handle_list_panes(_args: dict) -> dict:
            self.emit.info("list_panes tool invoked")

            async def attempt() -> dict:
                return {"panes": await self.emit.list_panes()}

            return await self._call_with_capability(
                "panes.read", "list_panes", attempt)

        @self.tool(
            "get_pane_state",
            description=(
                "Read the current UI tree of an app pane — every label, button, "
                "and input it is rendering. Use this to see an app before acting "
                "on it with send_app_action, and again afterwards to verify the "
                "action worked."
            ),
            schema={
                "type": "object",
                "properties": {
                    "pane_id": {
                        "type": "integer",
                        "description": "Target app pane id (from list_panes).",
                    },
                },
                "required": ["pane_id"],
            },
        )
        async def handle_get_pane_state(args: dict) -> dict:
            pane_id = int(args["pane_id"])
            self.emit.info(f"get_pane_state tool: pane={pane_id}")

            async def attempt() -> dict:
                return {"state": await self.emit.get_pane_state(pane_id)}

            return await self._call_with_capability(
                "panes.read", "get_pane_state", attempt)

        @self.tool(
            "capture_pane",
            description=(
                "Read the last lines of a terminal pane's scrollback. Use this "
                "to see a terminal's output before and after send_command."
            ),
            schema={
                "type": "object",
                "properties": {
                    "pane_id": {
                        "type": "integer",
                        "description": "Target terminal pane id (from list_panes).",
                    },
                    "lines": {
                        "type": "integer",
                        "description": "Number of lines from the end of the scrollback (default 50).",
                    },
                },
                "required": ["pane_id"],
            },
        )
        async def handle_capture_pane(args: dict) -> dict:
            pane_id = int(args["pane_id"])
            lines = int(args.get("lines", 50))
            self.emit.info(f"capture_pane tool: pane={pane_id} lines={lines}")

            async def attempt() -> dict:
                return {"lines": await self.emit.capture_pane(pane_id, lines=lines)}

            return await self._call_with_capability(
                "panes.read", "capture_pane", attempt)

        @self.tool(
            "send_command",
            description=(
                "Send text input to a terminal pane's stdin ('\\n' is Enter). "
                "Verify the result with capture_pane afterwards. For app panes "
                "use send_app_action instead."
            ),
            schema={
                "type": "object",
                "properties": {
                    "pane_id": {
                        "type": "integer",
                        "description": "Target terminal pane id (from list_panes).",
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
            pane_id = int(args["pane_id"])
            text = args.get("text", "")
            if not text:
                return {"error": "text must not be empty"}
            self.emit.info(f"send_command tool: pane={pane_id} text={text[:60]!r}")

            async def attempt() -> dict:
                self.emit.send_to_pane(pane_id, text)
                return {"ok": True, "pane_id": pane_id}

            return await self._call_with_capability(
                "panes.control", "send_command", attempt)

        @self.tool(
            "send_app_action",
            description=(
                "Dispatch a semantic action to an app pane (e.g. 'refresh', "
                "'navigate-to'). Workflow: get_pane_state to see the app, "
                "send_app_action to drive it, get_pane_state again to verify."
            ),
            schema={
                "type": "object",
                "properties": {
                    "pane_id": {
                        "type": "integer",
                        "description": "Target app pane id (from list_panes).",
                    },
                    "action": {
                        "type": "string",
                        "description": "Action name the app understands.",
                    },
                    "args": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Optional extra arguments for the action.",
                    },
                },
                "required": ["pane_id", "action"],
            },
        )
        async def handle_send_app_action(args: dict) -> dict:
            pane_id = int(args["pane_id"])
            action = args.get("action", "").strip()
            if not action:
                return {"error": "action must not be empty"}
            extra = [str(a) for a in args.get("args", [])]
            self.emit.info(f"send_app_action tool: pane={pane_id} action={action!r} args={extra}")

            async def attempt() -> dict:
                self.emit.send_app_action(pane_id, action, args=extra or None)
                return {"ok": True, "pane_id": pane_id, "action": action}

            return await self._call_with_capability(
                "panes.control", "send_app_action", attempt)

        @self.tool(
            "focus_pane",
            description="Move the user's UI focus to a pane by id (from list_panes).",
            schema={
                "type": "object",
                "properties": {
                    "pane_id": {
                        "type": "integer",
                        "description": "The pane id to focus.",
                    },
                },
                "required": ["pane_id"],
            },
        )
        async def handle_focus_pane(args: dict) -> dict:
            pane_id = int(args["pane_id"])
            self.emit.info(f"focus_pane tool: pane={pane_id}")

            async def attempt() -> dict:
                self.emit.focus_pane(pane_id)
                return {"ok": True, "pane_id": pane_id}

            return await self._call_with_capability(
                "panes.control", "focus_pane", attempt)

        self.emit.info(
            "AssistantApp: registered pane tools (open_terminal, open_app, "
            "list_panes, get_pane_state, capture_pane, send_command, "
            "send_app_action, focus_pane)"
        )

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
                "model_tier": self._model_tier,
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
        model_tier = self._model_tier
        try:
            resp = self.emit.run_sync(
                self.emit.ai_query(
                    model_tier=model_tier,
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

    def _thinking_label(self) -> Label:
        """Animated thinking indicator — Braille spinner at 8 fps."""
        frame = _THINKING_FRAMES[int(time.monotonic() * 8) % len(_THINKING_FRAMES)]
        return Label(f"{frame}  thinking…", tone="hint")

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
            children.append(self._thinking_label())
            self.emit.schedule_render()
        if not children:
            children.append(Label("Send a message to start.", tone="hint"))
        return Column(children, padding=0.0, padding_top=0.0, gap=8.0)

    def on_render(self, ctx) -> None:
        self._scroll.child = self._build_chat_column()
        self._scroll.scroll_offset = max(0.0, self._scroll._child_h - self._scroll._avail_h)

        if self._is_streaming:
            footer_keys: list[tuple] = [("esc", "stop")]
        else:
            footer_keys = [
                ("↵", "send"),
                ("⌘M", "model"),
                ("⌘N", "new"),
                ("esc", "quit"),
            ]

        ctx.render(Column([
            AppBar(title="Assistant", subtitle=self._model_tier),
            self._scroll,
            self._input,
            Spacer(grow=False),
            FooterKeys(footer_keys),
        ], padding=0.0, padding_top=0, gap=0.0))

        submitted = self._input.submitted
        if submitted is not None:
            self._submit(submitted)

    def _cycle_model(self) -> None:
        if self._is_streaming:
            return
        try:
            idx = MODEL_TIERS.index(self._model_tier)
        except ValueError:
            idx = -1  # unknown tier (e.g. stale session) → wrap to first
        self._model_tier = MODEL_TIERS[(idx + 1) % len(MODEL_TIERS)]
        self.emit.info(f"model tier → {self._model_tier}")
        self.emit.schedule_render()

    def on_key(self, key: str, mods: dict) -> None:
        if self._scroll.handle_key(key):
            self.emit.schedule_render()
            return
        if key == "m" and mods.get("cmd"):
            self._cycle_model()
            return
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
