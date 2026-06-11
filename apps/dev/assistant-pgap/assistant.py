#!/usr/bin/env python3
from __future__ import annotations

"""Assistant PGAP — developer reference chat app powered by Plexi's ai.query broker.

L1 Reference Implementation
============================
Patterns demonstrated:
  - ChatBubble for user/assistant/error messages
  - Scrollable container for conversation history
  - Live token + thinking streaming via on_ai_stream_chunk /
    on_ai_thinking_chunk + schedule_render
  - Background ai_query with emit.run_sync
  - Column + AppBar + Scrollable + multiline TextInput + FooterKeys
  - Session persistence via JSON in the workspace channel dir
  - ExposeTools: registers an ask_assistant tool so other workspace apps
    can invoke the assistant programmatically (#2034)
  - Tool consumption: ai_query automatically receives tools exposed by
    other panes in the same workspace via the host tool dispatcher
  - --demo-state: seed the UI from JSON for headless scene tests
"""

import json
import os
import subprocess
import threading
import time
from datetime import datetime, timezone
from pathlib import Path

PLEXI_BINARY = os.environ.get("PLEXI_BINARY", "plexi-alpha")

from plexi_sdk import App, Arg
from plexi_sdk.ui import (
    Column, AppBar, ChatBubble, Scrollable, Spacer,
    FooterKeys, TextInput, Label, ListItem,
    SPACE_SM, SPACE_MD, SPACE_LG,
)

SYSTEM_PROMPT = (
    "You are a helpful assistant. Be concise and clear. "
    "You have access to tools offered by other running Plexi apps in this workspace — "
    "use them when they help answer the user's request."
)
MODEL_TIERS = ["low", "medium", "high"]
_SPINNER_FRAMES = ["◐", "◓", "◑", "◒"]
# How much of the live thinking stream to show while it scrolls past.
_THINKING_TAIL_CHARS = 280


class AssistantApp(App):
    demo_state: Arg[str | None] = Arg("--demo-state", default=None)

    def on_init(self) -> None:
        self._messages: list[dict] = []  # {role, content, thinking?, thinking_secs?}
        self._streaming_text = ""
        self._thinking_text = ""
        self._is_streaming = False
        self._stream_started = 0.0
        self._model_tier = "low"
        self._input = TextInput(
            "chat-input",
            placeholder="Message assistant PGAP…",
            height=72.0,
            multiline=True,
        )
        self._composer_text = ""
        self._slash_selected = 0
        self._scroll = Scrollable(child=Spacer(), align="bottom")  # replaced each render
        self._pin_to_bottom = False
        self._session_id = _new_session_id()
        if self.demo_state:
            self._sessions_dir: Path | None = None  # demo mode never persists
            self._load_demo_state(str(self.demo_state))
        else:
            self._sessions_dir = _resolve_sessions_dir(self.workspace_root)
            self._load_latest_session()
        self._skills_dir = _resolve_skills_dir(self.workspace_root)
        self._ensure_demo_skill()
        self._skills = self._load_skills()
        self._register_tools()
        self._register_cli_tools()
        self.emit.info(
            f"AssistantApp ready — model={self._model_tier} "
            f"sessions dir: {self._sessions_dir} skills dir: {self._skills_dir}"
        )

    # ── Demo state (scene tests) ───────────────────────────────────────────────

    def _load_demo_state(self, raw: str) -> None:
        """Seed messages/streaming state from a JSON blob for headless scenes."""
        try:
            data = json.loads(raw)
            self._messages = data.get("messages", [])
            self._streaming_text = data.get("streaming_text", "")
            self._thinking_text = data.get("thinking_text", "")
            self._is_streaming = bool(data.get("is_streaming", False))
            self._model_tier = data.get("model_tier", self._model_tier)
            self._stream_started = time.monotonic()
            self.emit.info(f"demo state loaded: {len(self._messages)} messages")
        except Exception as e:
            self.emit.error(f"invalid --demo-state JSON: {e}")

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
        self._pin_to_bottom = True
        self.emit.schedule_render()

    def on_ai_thinking_chunk(self, _request_id: str, delta: str, _done: bool) -> None:
        self._thinking_text += delta
        self._pin_to_bottom = True
        self.emit.schedule_render()

    def _send_query(self) -> None:
        """Run ai_query on a background thread.

        The host tool dispatcher automatically injects tools exposed by other
        panes in the same workspace into the ai_query call. The broker handles
        the tool call loop internally, so this method receives the final
        resolved response after any tool rounds complete; tokens and thinking
        stream in live via on_ai_stream_chunk / on_ai_thinking_chunk.
        """
        messages = [
            {"role": m["role"], "content": m["content"]}
            for m in self._messages
            if m["role"] in ("user", "assistant")
        ]
        model_tier = self._model_tier
        try:
            resp = self.emit.run_sync(
                self.emit.ai_query(
                    model_tier=model_tier,
                    system=SYSTEM_PROMPT,
                    messages=messages,
                )
            )
            # The final response content is authoritative; streamed deltas are
            # the live preview (and the fallback if the host didn't stream).
            content = resp.content or self._streaming_text
            msg: dict = {"role": "assistant", "content": content}
            if self._thinking_text:
                msg["thinking"] = self._thinking_text
                msg["thinking_secs"] = round(time.monotonic() - self._stream_started, 1)
            self._messages.append(msg)
            self._save_session()
        except Exception as e:
            self.emit.error(f"ai_query failed: {e}")
            self._messages.append({"role": "error", "content": str(e)})
        finally:
            self._is_streaming = False
            self._streaming_text = ""
            self._thinking_text = ""
            self._pin_to_bottom = True
            self.emit.schedule_render()

    def _submit(self, text: str) -> None:
        if self._is_streaming:
            return
        text = text.strip()
        if not text:
            return
        if text.startswith("/"):
            if self._invoke_skill(text):
                self._composer_text = ""
                self.emit.schedule_render()
                return
        self._messages.append({"role": "user", "content": text})
        self._is_streaming = True
        self._streaming_text = ""
        self._thinking_text = ""
        self._stream_started = time.monotonic()
        self._pin_to_bottom = True
        threading.Thread(target=self._send_query, daemon=True).start()
        self.emit.schedule_render()

    # ── Rendering ──────────────────────────────────────────────────────────────

    def _spinner_label(self, text: str) -> Label:
        """Animated indicator — Braille spinner at 8 fps."""
        frame = _SPINNER_FRAMES[int(time.monotonic() * 8) % len(_SPINNER_FRAMES)]
        self.emit.schedule_render()
        return Label(f"{frame}  {text}", tone="hint")

    def _thought_marker(self, msg: dict) -> Label | None:
        """Collapsed one-line marker for a completed thinking phase."""
        if not msg.get("thinking"):
            return None
        secs = msg.get("thinking_secs")
        label = f"✦ thought for {secs}s" if secs else "✦ thought"
        return Label(label, tone="hint")

    def _build_chat_column(self) -> Column:
        """Build a Column of ChatBubble nodes for the message history."""
        children: list = []
        for msg in self._messages:
            role = msg["role"] if msg["role"] in ("user", "error") else "assistant"
            marker = self._thought_marker(msg)
            if marker is not None:
                children.append(marker)
            children.append(ChatBubble(text=msg["content"], role=role, max_lines=100))

        if self._is_streaming:
            if self._thinking_text and not self._streaming_text:
                # Live thinking phase: spinner + scrolling tail of the thought.
                children.append(self._spinner_label("thinking…"))
                tail = self._thinking_text[-_THINKING_TAIL_CHARS:]
                children.append(Label(tail, tone="hint", max_lines=4))
            elif self._streaming_text:
                if self._thinking_text:
                    secs = round(time.monotonic() - self._stream_started, 1)
                    children.append(Label(f"✦ thought for {secs}s", tone="hint"))
                children.append(ChatBubble(
                    text=self._streaming_text, role="assistant", max_lines=100,
                ))
            else:
                children.append(self._spinner_label("waiting for model…"))

        if not children:
            children.append(Label("Ask anything. ⇧↵ for a new line.", tone="hint"))
        return Column(children, padding=SPACE_LG, padding_top=SPACE_MD, gap=SPACE_MD)

    # ── Slash skills ──────────────────────────────────────────────────────────

    def _ensure_demo_skill(self) -> None:
        """Seed a hello-world skill for a fresh workspace POC."""
        if self._skills_dir is None:
            return
        try:
            self._skills_dir.mkdir(parents=True, exist_ok=True)
            if any(self._skills_dir.iterdir()):
                return
            demo_dir = self._skills_dir / "hello-world"
            demo_dir.mkdir(parents=True, exist_ok=True)
            (demo_dir / "SKILL.md").write_text(
                "---\n"
                "name: hello-world\n"
                "description: Proof-of-concept workspace skill for Plexi slash commands.\n"
                "---\n\n"
                "# Hello World\n\n"
                "When invoked, greet the user and echo any provided arguments.\n",
                encoding="utf-8",
            )
            self.emit.info(f"seeded demo workspace skill at {demo_dir}")
        except Exception as exc:
            self.emit.warn(f"could not seed demo skill: {exc}")

    def _load_skills(self) -> list[dict]:
        if self._skills_dir is None or not self._skills_dir.exists():
            return []
        skills: list[dict] = []
        for skill_file in sorted(self._skills_dir.glob("*/SKILL.md")):
            try:
                raw = skill_file.read_text(encoding="utf-8")
            except Exception as exc:
                self.emit.warn(f"could not read skill {skill_file}: {exc}")
                continue
            slug = skill_file.parent.name
            name = _frontmatter_value(raw, "name") or slug
            description = _frontmatter_value(raw, "description") or _first_heading(raw)
            skills.append({
                "name": name,
                "slug": slug,
                "description": description,
                "path": str(skill_file),
                "body": raw,
            })
        return skills

    def _slash_parts(self, text: str) -> tuple[str, str] | None:
        if not text.startswith("/") or text.startswith("//"):
            return None
        body = text[1:]
        first, sep, rest = body.partition(" ")
        return first.strip(), rest if sep else ""

    def _slash_matches(self) -> list[dict]:
        parts = self._slash_parts(self._composer_text)
        if parts is None:
            return []
        query, rest = parts
        exact = next((s for s in self._skills if s["slug"] == query or s["name"] == query), None)
        if exact and rest:
            return []
        scored: list[tuple[int, dict]] = []
        for skill in self._skills:
            hay = f"{skill['slug']} {skill['name']} {skill['description']}".lower()
            score = _fuzzy_score(query.lower(), hay, skill["slug"].lower())
            if score is not None:
                scored.append((score, skill))
        scored.sort(key=lambda item: (item[0], item[1]["slug"]))
        return [skill for _, skill in scored[:5]]

    def _complete_selected_skill(self) -> None:
        matches = self._slash_matches()
        if not matches:
            return
        skill = matches[min(self._slash_selected, len(matches) - 1)]
        parts = self._slash_parts(self._composer_text)
        args = parts[1] if parts else ""
        suffix = f" {args}" if args else " "
        self._composer_text = f"/{skill['slug']}{suffix}"
        self._input.value = self._composer_text
        self._slash_selected = 0
        self.emit.info(f"slash skill completed: /{skill['slug']}")
        self.emit.schedule_render()

    def _invoke_skill(self, text: str) -> bool:
        parts = self._slash_parts(text)
        if parts is None:
            return False
        name, args = parts
        skill = next(
            (s for s in self._skills if s["slug"] == name or s["name"] == name),
            None,
        )
        if skill is None:
            matches = self._slash_matches()
            if not matches:
                return False
            skill = matches[0]
        self.emit.info(f"slash skill invoked: /{skill['slug']} args={args[:80]!r}")
        self._messages.append({"role": "user", "content": text})
        response = (
            f"Loaded workspace skill `/{skill['slug']}`"
            + (f" with arguments: `{args}`" if args else ".")
            + f"\n\n{skill['description']}"
        )
        self._messages.append({"role": "assistant", "content": response})
        self._pin_to_bottom = True
        self._save_session()
        return True

    def _slash_popup(self) -> Column | None:
        matches = self._slash_matches()
        if not matches:
            return None
        self._slash_selected = min(self._slash_selected, len(matches) - 1)
        rows = [
            ListItem(
                title=f"/{skill['slug']}",
                subtitle=skill["description"],
                trailing="Tab",
                selected=(idx == self._slash_selected),
            )
            for idx, skill in enumerate(matches)
        ]
        return Column(rows, padding=0.0, padding_top=0.0, gap=SPACE_SM)

    def on_render(self, ctx) -> None:
        self._scroll.child = self._build_chat_column()

        if self._is_streaming:
            footer_keys: list[tuple] = []
        else:
            footer_keys = [
                ("↵", "send"),
                ("⇧↵", "newline"),
                ("/", "skills"),
                ("Tab", "complete"),
            ]

        composer_children = []
        popup = self._slash_popup()
        if popup is not None:
            composer_children.append(popup)
        composer_children.append(self._input)

        composer = Column(
            composer_children,
            padding=SPACE_LG,
            padding_top=SPACE_SM,
            gap=SPACE_SM,
        )

        root_children = [
            AppBar(title="Assistant PGAP", subtitle=self._model_tier),
            self._scroll,
            composer,
        ]
        if footer_keys:
            root_children.append(FooterKeys(footer_keys))

        ctx.render(Column(root_children, padding=0.0, padding_top=0, gap=0.0))

        if self._pin_to_bottom:
            next_offset = max(0.0, self._scroll._child_h - self._scroll._avail_h)
            if abs(next_offset - self._scroll.scroll_offset) > 0.5:
                self._scroll.scroll_offset = next_offset
                self.emit.schedule_render()
            if not self._is_streaming:
                self._pin_to_bottom = False

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
            self._pin_to_bottom = False
            self.emit.schedule_render()
            return

    def on_text_changed(self, id: str, text: str) -> None:
        if id != self._input.id:
            return
        self._composer_text = text
        self._slash_selected = min(self._slash_selected, max(0, len(self._slash_matches()) - 1))
        self.emit.schedule_render()

    def on_text_input_key(self, id: str, key: str, mods: dict) -> None:
        if id != self._input.id:
            return
        matches = self._slash_matches()
        if not matches:
            return
        if key == "tab":
            self._complete_selected_skill()
            return
        if key == "down":
            self._slash_selected = (self._slash_selected + 1) % len(matches)
            self.emit.schedule_render()
            return
        if key == "up":
            self._slash_selected = (self._slash_selected - 1) % len(matches)
            self.emit.schedule_render()
            return
        if key == "escape":
            self._composer_text = ""
            self._input.value = ""
            self.emit.schedule_render()


# ── Helpers ────────────────────────────────────────────────────────────────────

def _new_session_id() -> str:
    """Generate a session ID from the current UTC timestamp."""
    return datetime.now(tz=timezone.utc).strftime("%Y-%m-%dT%H-%M-%S")


def _workspace_channel_dir() -> str:
    channel = os.environ.get("PLEXI_CHANNEL", "").strip()
    if channel:
        return f".plexi-{channel}"
    binary = Path(PLEXI_BINARY).name
    if binary.startswith("plexi-") and len(binary) > len("plexi-"):
        return f".plexi-{binary[len('plexi-'):]}"
    return ".plexi"


def _resolve_sessions_dir(workspace_root: str) -> Path | None:
    """Return the channel-scoped assistant_sessions dir under workspace_root."""
    if not workspace_root:
        return None
    root = Path(workspace_root)
    plexi_dir = root / _workspace_channel_dir()
    # Only use workspace storage if the channel dir exists (i.e. workspace is initialised).
    if not plexi_dir.exists():
        # Fall back to a dir next to the script so the app still persists.
        script_dir = Path(os.path.abspath(__file__)).parent
        return script_dir / "sessions"
    return plexi_dir / "assistant_pgap_sessions"


def _resolve_skills_dir(workspace_root: str) -> Path | None:
    if not workspace_root:
        return None
    root = Path(workspace_root)
    return root / _workspace_channel_dir() / "agents" / "skills"


def _frontmatter_value(raw: str, key: str) -> str:
    if not raw.startswith("---"):
        return ""
    end = raw.find("\n---", 3)
    if end < 0:
        return ""
    prefix = f"{key}:"
    for line in raw[3:end].splitlines():
        if line.strip().startswith(prefix):
            return line.split(":", 1)[1].strip().strip('"').strip("'")
    return ""


def _first_heading(raw: str) -> str:
    for line in raw.splitlines():
        stripped = line.strip()
        if stripped.startswith("#"):
            return stripped.lstrip("#").strip()
        if stripped and stripped != "---" and ":" not in stripped:
            return stripped
    return "Workspace skill"


def _fuzzy_score(query: str, haystack: str, slug: str) -> int | None:
    if not query:
        return 0
    if slug.startswith(query):
        return 1
    if query in slug:
        return 2
    if query in haystack:
        return 3
    pos = 0
    for ch in query:
        found = haystack.find(ch, pos)
        if found < 0:
            return None
        pos = found + 1
    return 4 + pos


if __name__ == "__main__":
    AssistantApp().run()
