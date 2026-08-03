import importlib.util
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SDK = ROOT / "sdk" / "python"
APP = ROOT / "apps" / "wikipedia" / "wikipedia.py"

sys.path.insert(0, str(SDK))

import plexi_sdk as sdk  # noqa: E402
from plexi_sdk import _v3_state  # noqa: E402
from plexi_sdk.effects import ExposeTools, HttpFetch, ToolResult  # noqa: E402
from plexi_sdk.events import HttpResponse, ToolCall  # noqa: E402


def _load_app_module():
    spec = importlib.util.spec_from_file_location("wikipedia_app", APP)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _set_state(values: dict) -> None:
    raw = {key: b"" for key in values}
    _v3_state._state = sdk.StateSnapshot(values, raw)
    _v3_state._in_view = False


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


# --- ExposeTools / ToolCall coverage -----------------------------------


def _search_response(titles: list[str]) -> HttpResponse:
    return HttpResponse(
        status=200,
        headers=[],
        body=json.dumps(["q", titles, [], []]).encode(),
    )


def _article_response(extract: str) -> HttpResponse:
    return HttpResponse(
        status=200,
        headers=[],
        body=json.dumps({"extract": extract}).encode(),
    )


def test_init_exposes_exactly_the_search_tool():
    app = _load_app_module()
    _set_state(dict(app.DEFAULT_STATE))

    declaration = next(e for e in app.init(None, None) if isinstance(e, ExposeTools))
    names = {tool.name for tool in declaration.tools}

    assert names == {"wikipedia.search"}
    tool = declaration.tools[0]
    assert tool.read_only is True
    assert tool.description
    assert tool.input_schema["required"] == ["query"]
    assert set(tool.output_schema["required"]) == {"title", "summary"}


def test_init_with_launch_args_still_exposes_tools():
    app = _load_app_module()
    _set_state(dict(app.DEFAULT_STATE))

    effects = app.init(None, ["Detroit"])

    assert any(isinstance(e, ExposeTools) for e in effects)
    assert any(isinstance(e, HttpFetch) for e in effects)


def test_tool_call_happy_path_search_then_article():
    app = _load_app_module()
    _set_state(dict(app.DEFAULT_STATE))

    effects = app.update(ToolCall("call-1", "wikipedia.search", '{"query":"Plexi"}', "assistant"))
    assert len(effects) == 1
    assert isinstance(effects[0], HttpFetch)
    assert app._pending_tool == {"call_id": "call-1", "stage": "search", "query": "Plexi"}

    effects = app.update(_search_response(["Plexi", "Plexiglass"]))
    assert len(effects) == 1
    assert isinstance(effects[0], HttpFetch)
    assert effects[0].url.endswith("/Plexi")
    assert app._pending_tool["stage"] == "article"
    assert app._pending_tool["title"] == "Plexi"

    effects = app.update(_article_response("Plexi is a brand of acrylic glass."))
    assert len(effects) == 1
    result = effects[0]
    assert isinstance(result, ToolResult)
    assert result.call_id == "call-1"
    assert result.error is None
    assert result.output_json is not None
    output = json.loads(result.output_json)
    assert output == {"title": "Plexi", "summary": "Plexi is a brand of acrylic glass."}
    assert app._pending_tool is None


def test_tool_call_malformed_input_json_is_an_error():
    app = _load_app_module()
    _set_state(dict(app.DEFAULT_STATE))

    effects = app.update(ToolCall("call-2", "wikipedia.search", "{not json", "assistant"))

    assert len(effects) == 1
    assert isinstance(effects[0], ToolResult)
    assert effects[0].error
    assert app._pending_tool is None


def test_tool_call_missing_query_is_an_error():
    app = _load_app_module()
    _set_state(dict(app.DEFAULT_STATE))

    effects = app.update(ToolCall("call-3", "wikipedia.search", "{}", "assistant"))

    assert len(effects) == 1
    assert isinstance(effects[0], ToolResult)
    assert effects[0].error
    assert app._pending_tool is None

    effects = app.update(ToolCall("call-4", "wikipedia.search", '{"query": "   "}', "assistant"))
    assert effects[0].error


def test_tool_call_empty_search_results_is_an_error():
    app = _load_app_module()
    _set_state(dict(app.DEFAULT_STATE))

    app.update(ToolCall("call-5", "wikipedia.search", '{"query":"zzzznotarealthing"}', "assistant"))
    effects = app.update(_search_response([]))

    assert len(effects) == 1
    assert isinstance(effects[0], ToolResult)
    assert effects[0].error
    assert app._pending_tool is None


def test_tool_call_http_failure_at_search_stage_is_an_error():
    app = _load_app_module()
    _set_state(dict(app.DEFAULT_STATE))

    app.update(ToolCall("call-6", "wikipedia.search", '{"query":"Plexi"}', "assistant"))
    effects = app.update(HttpResponse(status=500, headers=[], body=b"boom"))

    assert len(effects) == 1
    assert isinstance(effects[0], ToolResult)
    assert "HTTP 500" in effects[0].error
    assert app._pending_tool is None


def test_tool_call_http_failure_at_article_stage_is_an_error():
    app = _load_app_module()
    _set_state(dict(app.DEFAULT_STATE))

    app.update(ToolCall("call-7", "wikipedia.search", '{"query":"Plexi"}', "assistant"))
    app.update(_search_response(["Plexi"]))
    effects = app.update(HttpResponse(status=404, headers=[], body=b"missing"))

    assert len(effects) == 1
    assert isinstance(effects[0], ToolResult)
    assert "HTTP 404" in effects[0].error
    assert app._pending_tool is None


def test_concurrent_tool_call_while_one_pending_is_busy():
    app = _load_app_module()
    _set_state(dict(app.DEFAULT_STATE))

    app.update(ToolCall("call-8", "wikipedia.search", '{"query":"Plexi"}', "assistant"))
    effects = app.update(ToolCall("call-9", "wikipedia.search", '{"query":"Other"}', "assistant"))

    assert len(effects) == 1
    assert isinstance(effects[0], ToolResult)
    assert effects[0].call_id == "call-9"
    assert "busy" in effects[0].error


def test_ui_fetch_in_flight_blocks_tool_call():
    app = _load_app_module()
    _set_state(dict(app.DEFAULT_STATE, loading=True, pending="search"))

    effects = app.update(ToolCall("call-10", "wikipedia.search", '{"query":"Plexi"}', "assistant"))

    assert len(effects) == 1
    assert isinstance(effects[0], ToolResult)
    assert "busy" in effects[0].error
    assert app._pending_tool is None


def test_tool_fetch_in_flight_blocks_ui_fetch():
    app = _load_app_module()
    _set_state(dict(app.DEFAULT_STATE, query="Plexi"))

    app.update(ToolCall("call-11", "wikipedia.search", '{"query":"Plexi"}', "assistant"))
    assert app._pending_tool is not None

    data = app._state()
    effects = app._start_search(data)

    assert effects == []


def test_unknown_tool_name_is_an_error():
    app = _load_app_module()
    _set_state(dict(app.DEFAULT_STATE))

    effects = app.update(ToolCall("call-12", "wikipedia.nope", "{}", "assistant"))

    assert len(effects) == 1
    assert isinstance(effects[0], ToolResult)
    assert effects[0].error
