from __future__ import annotations

"""Unit tests for the v3.3 SDK `Agent` base class (issue #338, part 2 of #285).

Covers:
  1. AgentInit handler stores the system prompt on `self.system_prompt`.
  2. UserMessage handler appends a user row, calls `respond()`, then
     auto-appends the returned assistant string.
  3. `respond() -> None` skips the auto-append (manual mode).
  4. `append_*` helpers emit `append_conversation` DrawCommands with the
     correct shape and mirror history correctly.

We don't run the full PGAP event loop — we drive the SDK's event-handler
branches directly, which is exactly what `App.run()` does internally.
This keeps the tests deterministic and dependency-free.
"""

import io
import json
import os
import sys

# Allow importing plexi_sdk from the source tree without install.
# Path: examples/agent-tester/tests/ → repo_root/sdk/python.
sys.path.insert(
    0,
    os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"),
)

from plexi_sdk import Agent  # noqa: E402


def _capture_stdout():
    saved = sys.stdout
    buf = io.StringIO()
    sys.stdout = buf
    return saved, buf


def _restore(saved):
    sys.stdout = saved


def _decoded(buf: io.StringIO) -> list[dict]:
    out = []
    for line in buf.getvalue().splitlines():
        line = line.strip()
        if not line:
            continue
        out.append(json.loads(line))
    return out


def test_agent_class_stores_system_prompt_from_agent_init():
    """The host emits AgentInit once at startup with the manifest's
    `[launch].system_prompt`. The Agent base class must store it on
    `self.system_prompt` so subclasses can pass it to `iq.query`."""
    saved, _buf = _capture_stdout()
    try:
        agent = Agent()
        assert agent.system_prompt is None, "fresh agent has no prompt"
        agent.on_agent_init("You are terse.")
    finally:
        _restore(saved)

    assert agent.system_prompt == "You are terse."

    # `None` from the host means the manifest omitted the field — must
    # round-trip cleanly.
    saved, _buf = _capture_stdout()
    try:
        agent2 = Agent()
        agent2.on_agent_init(None)
    finally:
        _restore(saved)
    assert agent2.system_prompt is None


def test_agent_class_appends_user_message_on_user_event():
    """When the host emits UserMessage, the Agent base class must:
    (1) append a "user" row to history,
    (2) emit an `append_conversation` DrawCommand with role=user."""

    class TestAgent(Agent):
        def respond(self, _text: str) -> str:
            return "noop"  # required to satisfy NotImplementedError

    saved, buf = _capture_stdout()
    try:
        agent = TestAgent()
        ctx = agent._make_ctx()
        agent.on_user_message(ctx, "hello agent")
        cmds = _decoded(buf)
    finally:
        _restore(saved)

    # Exactly two append_conversation commands: user input, then assistant reply.
    appends = [c for c in cmds if c.get("type") == "append_conversation"]
    assert len(appends) == 2, f"expected 2 appends, got {appends}"
    assert appends[0] == {
        "type": "append_conversation",
        "role": "user",
        "content": "hello agent",
    }
    # History reflects both turns.
    assert agent.history == [
        {"role": "user", "content": "hello agent"},
        {"role": "assistant", "content": "noop"},
    ]


def test_agent_class_calls_on_user_message_override():
    """`respond()` is called once per user_message event with the raw text.
    `self.history` already contains the user turn before respond() runs so
    the override can pass it straight to `iq.query`."""

    captured: dict = {}

    class TestAgent(Agent):
        def respond(self, text: str) -> str:
            captured["text"] = text
            captured["history_len_at_respond"] = len(self.history)
            captured["last_role"] = self.history[-1]["role"]
            return "ack"

    saved, _buf = _capture_stdout()
    try:
        agent = TestAgent()
        ctx = agent._make_ctx()
        agent.on_user_message(ctx, "ping")
    finally:
        _restore(saved)

    assert captured["text"] == "ping"
    # When respond() runs, history has the user turn (length 1).
    assert captured["history_len_at_respond"] == 1
    assert captured["last_role"] == "user"


def test_agent_class_appends_assistant_message_after_override_returns():
    """A non-None return from `respond` auto-emits the assistant turn.
    A `None` return means the override appended manually — no auto-append."""

    class AutoAgent(Agent):
        def respond(self, _text: str) -> str:
            return "auto reply"

    saved, buf = _capture_stdout()
    try:
        agent = AutoAgent()
        ctx = agent._make_ctx()
        agent.on_user_message(ctx, "hi")
        cmds = _decoded(buf)
    finally:
        _restore(saved)
    assistant = [c for c in cmds if c.get("role") == "assistant"]
    assert len(assistant) == 1
    assert assistant[0]["content"] == "auto reply"

    # Now the manual-mode path: respond() returns None, override calls
    # append_* helpers itself.
    class ManualAgent(Agent):
        def respond(self, _text: str):
            self.append_tool_message("looking it up")
            self.append_assistant_message("manual reply")
            return None

    saved, buf = _capture_stdout()
    try:
        agent = ManualAgent()
        ctx = agent._make_ctx()
        agent.on_user_message(ctx, "hello")
        cmds = _decoded(buf)
    finally:
        _restore(saved)

    appends = [c for c in cmds if c.get("type") == "append_conversation"]
    # user + tool + assistant = 3 (NOT a fourth auto-assistant)
    assert len(appends) == 3, f"expected 3 (user/tool/assistant), got {appends}"
    roles = [c["role"] for c in appends]
    assert roles == ["user", "tool", "assistant"]
    # Tool messages are NOT mirrored into history.
    assert agent.history == [
        {"role": "user", "content": "hello"},
        {"role": "assistant", "content": "manual reply"},
    ]
