"""Tests for the Wikipedia PGAP app."""
from __future__ import annotations
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.parent.parent.parent / "sdk" / "python"))  # examples/
from plexi_test import Harness

APP = Path(__file__).parent.parent / "wikipedia.py"


def test_ready_handshake():
    with Harness.for_app(APP) as h:
        ready = h.init()
        assert ready["type"] == "ready"
        assert ready["sdk"].startswith("plexi-sdk-py/")


def test_search_mode_renders_input():
    with Harness.for_app(APP) as h:
        h.init()
        cmds = h.render_frame(800, 600)
        assert h.has_text(cmds, "Search")


def test_inject_results_state():
    with Harness.for_app(APP) as h:
        h.init()
        h.inject_state({
            "mode": "results",
            "query": "Rust programming language",
            "results": ["Rust (programming language)", "Rust (video game)", "Rust belt"],
        })
        cmds = h.render_frame(800, 600)
        assert h.has_text(cmds, "Rust"), f"got texts: {h.texts(cmds)}"


def test_inject_article_state():
    with Harness.for_app(APP) as h:
        h.init()
        h.inject_state({
            "mode": "article",
            "title": "Rust (programming language)",
            "extract": "Rust is a systems programming language focused on safety.",
        })
        cmds = h.render_frame(800, 600)
        assert h.has_text(cmds, "Rust"), f"got texts: {h.texts(cmds)}"


def test_mock_http_search():
    with Harness.for_app(APP) as h:
        h.init()
        h.mock_http(
            "https://en.wikipedia.org/w/api.php?action=opensearch&search=Rust&limit=10&format=json",
            '["Rust",["Rust (programming language)","Rust (video game)","Rust belt"],[],[]]',
        )
        for ch in ["R", "u", "s", "t"]:
            h.send_key(ch)
        h.send_key("Enter")
        import time; time.sleep(0.3)
        cmds = h.render_frame(800, 600)
        assert h.has_text(cmds, "Rust"), f"got texts: {h.texts(cmds)}"
