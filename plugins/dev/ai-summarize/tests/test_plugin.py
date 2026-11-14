"""Tests for AI Summarize.

Run with:  plexi plugin test .          (from the plugin directory)

PluginHarness runs main.py as a subprocess and speaks the plugin protocol over
stdin and stdout. It needs no running Plexi host and makes no model call: the
test reads the AiQuery the plugin sent and injects the host's AiResponse.
"""
from pathlib import Path

from plexi_sdk.testing import PluginHarness

PLUGIN = Path(__file__).resolve().parent.parent / "main.py"


def _texts(frame) -> list[str]:
    """Every "text" and "label" value in a rendered frame, in tree order."""
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


def _ask(h: PluginHarness, text: str) -> dict:
    h.run(1)
    h.text_submit("text", text)
    h.ui_action("summarize")
    h.run(1)
    queries = h.ai_queries()
    assert len(queries) == 1, queries
    return queries[0]


def test_summarize_sends_one_fast_query_with_the_text():
    with PluginHarness(str(PLUGIN)) as h:
        query = _ask(h, "a long article")
        assert query["mode"] == "fast"
        assert query["messages"] == [{"role": "user", "content": "a long article"}]


def test_slow_toggle_sends_a_slow_query():
    with PluginHarness(str(PLUGIN)) as h:
        h.run(1)
        h.ui_action("mode:1")
        assert _ask(h, "notes")["mode"] == "slow"


def test_reply_is_shown():
    with PluginHarness(str(PLUGIN)) as h:
        query = _ask(h, "notes")
        h.ai_response(query["request_id"], content="Short summary.")
        assert "Short summary." in _texts(h.run(1))


def test_refusal_is_shown_inline():
    with PluginHarness(str(PLUGIN)) as h:
        query = _ask(h, "notes")
        h.ai_response(query["request_id"], error="capability withheld: ai.query")
        texts = _texts(h.run(1))
        assert any("capability withheld: ai.query" in t for t in texts), texts


def test_empty_text_sends_nothing():
    with PluginHarness(str(PLUGIN)) as h:
        h.run(1)
        h.ui_action("summarize")
        h.run(1)
        assert h.ai_queries() == []
