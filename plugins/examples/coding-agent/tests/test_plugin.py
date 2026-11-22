"""Tests for the Coding Agent example.

Run with:  plexi plugin test .          (from the plugin directory)

PluginHarness runs the plugin as a subprocess; no host and no model call.
"""
from pathlib import Path

from plexi_sdk.testing import PluginHarness

PLUGIN = Path(__file__).resolve().parent.parent / "coding_agent.py"


def _texts(frame) -> list[str]:
    found: list[str] = []

    def walk(value) -> None:
        if isinstance(value, dict):
            for key in ("text", "label"):
                if isinstance(value.get(key), str):
                    found.append(value[key])
            for child in value.values():
                walk(child)
        elif isinstance(value, list):
            for child in value:
                walk(child)

    walk(frame)
    return found


def _ask(h: PluginHarness, prompt: str) -> str:
    h.text_submit("prompt", prompt)
    h.ui_action("ask")
    h.run(1)
    return h.ai_queries()[-1]["request_id"]


def test_a_late_reply_to_an_older_ask_does_not_replace_the_newest_answer():
    with PluginHarness(str(PLUGIN)) as h:
        h.run(1)
        first = _ask(h, "first question")
        second = _ask(h, "second question")
        assert first != second, "each ask has its own request id"

        h.ai_response(second, content="answer to the second")
        assert "answer to the second" in _texts(h.run(1))

        h.ai_response(first, content="answer to the first")
        texts = _texts(h.run(1))
        assert "answer to the second" in texts, texts
        assert "answer to the first" not in texts, texts
