import importlib.util
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SDK = ROOT / "sdk" / "python"
APP = ROOT / "apps" / "wikipedia" / "wikipedia.py"

sys.path.insert(0, str(SDK))

from plexi_sdk.events import HttpResponse


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


def test_search_url_percent_encodes_utf8_without_urllib():
    app = _load_app_module()

    request = app._fetch_search("café & tea")

    assert "search=caf%C3%A9%20%26%20tea" in request.url
    assert "import urllib" not in APP.read_text()


def test_search_response_moves_to_results():
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)
    data.update({"query": "Plexi", "loading": True, "pending": "search"})
    event = HttpResponse(
        status=200,
        headers=[],
        body=json.dumps(["Plexi", ["Plexi", "Plexiglass"], [], []]).encode(),
    )

    result = app._handle_http(data, event)

    assert result["mode"] == "results"
    assert result["results"] == ["Plexi", "Plexiglass"]
    assert result["loading"] is False
    assert result["pending"] == ""


def test_result_action_starts_article_request():
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)
    data["results"] = ["Plexi", "Plexiglass"]

    effects = app._start_article(data, 1)

    assert data["selected"] == 1
    assert data["pending"] == "article"
    assert data["article_title"] == "Plexiglass"
    assert effects[-1].url.endswith("/Plexiglass")
