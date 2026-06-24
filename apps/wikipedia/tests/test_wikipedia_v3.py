import importlib.util
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
SDK = ROOT / "sdk" / "python"
APP = ROOT / "apps" / "wikipedia" / "wikipedia.py"

sys.path.insert(0, str(SDK))

import plexi_sdk as sdk  # noqa: E402
from plexi_sdk import _v3_state  # noqa: E402
from plexi_sdk.effects import SetState  # noqa: E402
from plexi_sdk.events import KeyEvent  # noqa: E402


def _load_app_module():
    spec = importlib.util.spec_from_file_location("wikipedia_app", APP)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_parse_search_reads_opensearch_titles():
    app = _load_app_module()

    assert app._parse_search(["plexi", ["Plexi", "Plexiglass"], [], []]) == [
        "Plexi",
        "Plexiglass",
    ]


def _set_state(values: dict) -> None:
    raw = {key: b"" for key in values}
    _v3_state._state = sdk.StateSnapshot(values, raw)
    _v3_state._in_view = False


def _has_footer_keys(node: dict) -> bool:
    if node.get("type") == "footer_keys":
        return True
    child = node.get("child")
    if isinstance(child, dict) and _has_footer_keys(child):
        return True
    for child in node.get("children", []):
        if isinstance(child, dict) and _has_footer_keys(child):
            return True
    return False


def test_every_wikipedia_mode_has_footer_shortcuts():
    app = _load_app_module()
    cases = [
        {"mode": "search", "query": "plexi", "net_http_granted": True},
        {"mode": "search", "loading": True, "pending": "search", "query": "plexi"},
        {"mode": "results", "query": "plexi", "results": ["Plexi"], "selected": 0},
        {"mode": "article", "article_title": "Plexi", "article": "Summary"},
    ]

    for case in cases:
        state = dict(app.DEFAULT_STATE)
        state.update(case)
        _set_state(state)
        assert _has_footer_keys(app.view().to_node()), case


def test_escape_from_article_loading_returns_to_results():
    app = _load_app_module()
    state = dict(app.DEFAULT_STATE)
    state.update(
        {
            "mode": "results",
            "loading": True,
            "pending": "article",
            "return_mode": "results",
            "query": "plexi",
            "results": ["Plexi"],
        }
    )
    _set_state(state)

    effects = app.update(KeyEvent("escape"))
    data = next(effect.data for effect in effects if isinstance(effect, SetState))

    assert data["mode"] == "results"
    assert data["loading"] is False
