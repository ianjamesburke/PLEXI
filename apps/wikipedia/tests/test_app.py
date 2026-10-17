from __future__ import annotations

import importlib.util
import json
from pathlib import Path

import plexi_sdk as sdk
from plexi_sdk import _v3_state
from plexi_sdk._adapter import _encode_uitree
from plexi_sdk.effects import ExposeTools, HttpFetch, SetState, ToolResult
from plexi_sdk.events import HttpResponse, KeyEvent, ToolCall, UiAction, UiValueChange
from plexi_sdk.testing import AppHarness

# Alias for stint wording compatibility.
AppTestHarness = AppHarness

ROOT = Path(__file__).resolve().parents[3]
APP = ROOT / "apps" / "wikipedia" / "wikipedia.py"


def _load_app_module():
    spec = importlib.util.spec_from_file_location("wikipedia_app", APP)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module._pending_tools.clear()
    return module


def _set_state(values: dict) -> None:
    raw = {key: json.dumps(val).encode("utf-8") for key, val in values.items()}
    _v3_state._state = sdk.StateSnapshot(
        dict(values), raw, {"global": dict(values)}, ("global",)
    )
    _v3_state._in_view = False


def _search_payload(items: list[tuple[str, str]]) -> bytes:
    return json.dumps(
        {
            "batchcomplete": "",
            "query": {
                "search": [
                    {"title": title, "snippet": snippet}
                    for title, snippet in items
                ]
            },
        }
    ).encode("utf-8")


def _search_response(
    items: list[tuple[str, str]] | list[str],
    *,
    tag: str | None = None,
) -> HttpResponse:
    normalized = [
        (item, f"Summary snippet for {item}") if isinstance(item, str) else item
        for item in items
    ]
    return HttpResponse(
        status=200,
        headers=[],
        body=_search_payload(normalized),
        tag=tag,
        request_id=tag,
    )


def _article_response(
    title: str,
    extract: str,
    description: str = "Encyclopedia article",
    *,
    tag: str | None = None,
) -> HttpResponse:
    return HttpResponse(
        status=200,
        headers=[],
        body=json.dumps(
            {
                "title": title,
                "description": description,
                "extract": extract,
            }
        ).encode("utf-8"),
        tag=tag,
        request_id=tag,
    )


# ── HTTP headers, search snippet parsing & title encoding ───────────────────


def test_http_fetches_include_wikimedia_user_agent_and_accept_headers():
    app = _load_app_module()

    search_req = app._fetch_search("Alan Turing", tag="ui:search:1")
    article_req = app._fetch_article("Alan Turing", tag="ui:article:1")

    for req in (search_req, article_req):
        assert req.headers.get("User-Agent") == "Plexi/1.0 (https://plexiapp.com)"
        assert req.headers.get("Accept") == "application/json"


def test_search_url_uses_mediawiki_query_search_and_encodes_without_urllib():
    app = _load_app_module()

    request = app._fetch_search("café & tea", tag="ui:search:1")

    assert "action=query" in request.url
    assert "list=search" in request.url
    assert "srsearch=caf%C3%A9%20%26%20tea" in request.url
    assert "utf8=1" in request.url
    assert "format=json" in request.url
    assert "import urllib" not in APP.read_text()


def test_article_url_replaces_spaces_with_underscores_before_percent_encoding():
    app = _load_app_module()

    turing_req = app._fetch_article("Alan Turing")
    assert turing_req.url == "https://en.wikipedia.org/api/rest_v1/page/summary/Alan_Turing"

    special_req = app._fetch_article("  AC/DC & Café  ")
    assert special_req.url.endswith("/AC%2FDC_%26_Caf%C3%A9")


def test_parse_search_strips_html_tags_and_entities_from_snippets():
    app = _load_app_module()
    payload = {
        "query": {
            "search": [
                {
                    "title": "Alan Turing",
                    "snippet": (
                        '<span class="searchmatch">Alan</span> Mathison '
                        '<span class="searchmatch">Turing</span> '
                        "(&#8211; 1954) &amp; pioneer of &quot;computing&quot;."
                    ),
                },
                {
                    "title": "Ada Lovelace",
                    "snippet": "English mathematician &amp; writer&#039;s notes on Babbage.",
                },
            ]
        }
    }

    parsed = app._parse_search(payload)

    assert parsed == [
        {
            "title": "Alan Turing",
            "snippet": 'Alan Mathison Turing (– 1954) & pioneer of "computing".',
        },
        {
            "title": "Ada Lovelace",
            "snippet": "English mathematician & writer's notes on Babbage.",
        },
    ]


# ── Focus release on submit / escape & "/" search refocus ────────────────────


def test_search_submit_releases_text_input_focus_and_starts_search():
    app = _load_app_module()
    _set_state(dict(app.DEFAULT_STATE, query="Alan Turing", editing_search=True))

    effects = app.update(UiAction("wiki-submit"))
    new_state = next(e for e in effects if isinstance(e, SetState)).data
    fetch = next(e for e in effects if isinstance(e, HttpFetch))

    assert new_state["editing_search"] is False
    assert new_state["searched_query"] == "Alan Turing"
    assert new_state["loading"] is True
    assert new_state["pending"] == "search"
    assert fetch.tag == new_state["active_tag"]

    _set_state(new_state)
    tree = _encode_uitree(app.view())
    inputs = [
        n["data"] for n in tree["nodes"] if n["data"].get("type") == "TextInput"
    ]
    assert len(inputs) == 1
    assert inputs[0]["autofocus"] is False


def test_escape_and_slash_toggle_search_focus_in_list_mode():
    app = _load_app_module()
    _set_state(dict(app.DEFAULT_STATE, editing_search=True, results=[]))

    # When results are empty, Escape and empty submit keep search focused so
    # keyboard users are not stranded on an empty screen.
    effects = app.update(KeyEvent("escape"))
    state_empty_esc = next(e for e in effects if isinstance(e, SetState)).data
    assert state_empty_esc["editing_search"] is True

    effects = app.update(UiAction("wiki-submit"))
    state_empty_submit = next(e for e in effects if isinstance(e, SetState)).data
    assert state_empty_submit["editing_search"] is True

    # Once results exist, Escape releases search focus.
    _set_state(
        dict(
            app.DEFAULT_STATE,
            editing_search=True,
            results=[{"title": "Alan Turing", "snippet": "English mathematician"}],
        )
    )
    effects = app.update(KeyEvent("escape"))
    state_after_esc = next(e for e in effects if isinstance(e, SetState)).data
    assert state_after_esc["editing_search"] is False

    # "/" focuses the search input again.
    _set_state(state_after_esc)
    effects = app.update(KeyEvent("/"))
    state_after_slash = next(e for e in effects if isinstance(e, SetState)).data
    assert state_after_slash["editing_search"] is True


# ── j/k / Up/Down navigation & Enter / mouse click article opening ───────────


def test_j_k_and_arrow_keys_navigate_results_and_enter_opens_article():
    app = _load_app_module()
    results = [
        {"title": "Alan Turing", "snippet": "English mathematician"},
        {"title": "Turing machine", "snippet": "Mathematical model of computation"},
        {"title": "Turing test", "snippet": "Test of machine intelligence"},
    ]
    _set_state(
        dict(
            app.DEFAULT_STATE,
            query="Alan Turing",
            searched_query="Alan Turing",
            results=results,
            selected=0,
            editing_search=False,
        )
    )

    # j moves down to row 1.
    effects = app.update(KeyEvent("j"))
    st = next(e for e in effects if isinstance(e, SetState)).data
    assert st["selected"] == 1

    # down moves to row 2 and clamps at the end.
    _set_state(st)
    effects = app.update(KeyEvent("down"))
    st = next(e for e in effects if isinstance(e, SetState)).data
    assert st["selected"] == 2

    _set_state(st)
    effects = app.update(KeyEvent("j"))
    st = next(e for e in effects if isinstance(e, SetState)).data
    assert st["selected"] == 2

    # k moves back up to row 1.
    _set_state(st)
    effects = app.update(KeyEvent("k"))
    st = next(e for e in effects if isinstance(e, SetState)).data
    assert st["selected"] == 1

    # Enter opens row 1 ("Turing machine") in article mode.
    _set_state(st)
    effects = app.update(KeyEvent("enter"))
    st = next(e for e in effects if isinstance(e, SetState)).data
    fetch = next(e for e in effects if isinstance(e, HttpFetch))
    assert st["mode"] == "article"
    assert st["article_title"] == "Turing machine"
    assert st["loading"] is True
    assert st["pending"] == "article"
    assert fetch.url.endswith("/Turing_machine")


def test_clicking_select_list_row_dispatches_on_select_and_opens_article():
    app = _load_app_module()
    results = [
        {"title": "Alan Turing", "snippet": "English mathematician"},
        {"title": "Ada Lovelace", "snippet": "First computer programmer"},
    ]
    _set_state(
        dict(
            app.DEFAULT_STATE,
            query="computing",
            searched_query="computing",
            results=results,
            selected=0,
            editing_search=False,
        )
    )

    # Verify SelectList in list mode carries on_select="wiki-open-row" and descriptions.
    tree = _encode_uitree(app.view())
    list_nodes = [
        n["data"] for n in tree["nodes"] if n["data"].get("type") == "ListView"
    ]
    assert len(list_nodes) == 1
    assert list_nodes[0]["on_select"] == "wiki-open-row"
    text_values = [
        n["data"].get("text")
        for n in tree["nodes"]
        if n["data"].get("type") == "text"
    ]
    assert "Alan Turing" in text_values
    assert "English mathematician" in text_values
    assert "Ada Lovelace" in text_values
    assert "First computer programmer" in text_values

    # Clicking row 1 opens "Ada Lovelace" in article reader mode.
    effects = app.update(UiAction("wiki-open-row:1"))
    st = next(e for e in effects if isinstance(e, SetState)).data
    fetch = next(e for e in effects if isinstance(e, HttpFetch))
    assert st["selected"] == 1
    assert st["mode"] == "article"
    assert st["article_title"] == "Ada Lovelace"
    assert fetch.url.endswith("/Ada_Lovelace")


# ── Article mode: back navigation (esc/b/backspace), n/p cycling, "/" ────────


def test_article_mode_back_navigation_and_next_prev_cycling():
    app = _load_app_module()
    results = [
        {"title": "Alan Turing", "snippet": "English mathematician"},
        {"title": "Ada Lovelace", "snippet": "English writer and mathematician"},
        {"title": "Grace Hopper", "snippet": "American computer scientist"},
    ]
    _set_state(
        dict(
            app.DEFAULT_STATE,
            mode="article",
            query="pioneers",
            searched_query="pioneers",
            results=results,
            selected=0,
            article_title="Alan Turing",
            article_description="English mathematician",
            article="Alan Mathison Turing was an English mathematician.",
            editing_search=False,
        )
    )

    # Article view renders full-height Scrollable without SelectList or TextInput.
    article_tree = _encode_uitree(app.view())
    article_types = {n["data"].get("type") for n in article_tree["nodes"]}
    assert "Scroll" in article_types
    assert "ListView" not in article_types
    assert "TextInput" not in article_types

    # "n" advances to result 1 ("Ada Lovelace").
    effects = app.update(KeyEvent("n"))
    st = next(e for e in effects if isinstance(e, SetState)).data
    fetch = next(e for e in effects if isinstance(e, HttpFetch))
    assert st["mode"] == "article"
    assert st["selected"] == 1
    assert st["article_title"] == "Ada Lovelace"
    assert fetch.url.endswith("/Ada_Lovelace")

    # Simulate article response for Ada Lovelace.
    _set_state(st)
    effects = app.update(
        _article_response(
            "Ada Lovelace",
            "Augusta Ada King, Countess of Lovelace was an English mathematician.",
            "English mathematician and writer",
            tag=st["active_tag"],
        )
    )
    st = next(e for e in effects if isinstance(e, SetState)).data
    assert st["article"].startswith("Augusta Ada King")
    assert st["article_description"] == "English mathematician and writer"

    # "p" returns to result 0 ("Alan Turing").
    _set_state(st)
    effects = app.update(KeyEvent("p"))
    st = next(e for e in effects if isinstance(e, SetState)).data
    assert st["selected"] == 0
    assert st["article_title"] == "Alan Turing"

    # "b" returns to list mode preserving the selection index.
    _set_state(dict(st, selected=1, loading=False, pending=""))
    effects = app.update(KeyEvent("b"))
    st = next(e for e in effects if isinstance(e, SetState)).data
    assert st["mode"] == "list"
    assert st["selected"] == 1
    assert st["editing_search"] is False

    # "escape" from article mode also returns to list mode.
    _set_state(dict(st, mode="article"))
    effects = app.update(KeyEvent("escape"))
    st = next(e for e in effects if isinstance(e, SetState)).data
    assert st["mode"] == "list"
    assert st["selected"] == 1

    # "/" from article mode switches to list mode with search focused.
    _set_state(dict(st, mode="article", editing_search=False))
    effects = app.update(KeyEvent("/"))
    st = next(e for e in effects if isinstance(e, SetState)).data
    assert st["mode"] == "list"
    assert st["editing_search"] is True


# ── Non-200 HTTP error Banner rendering & request tag correlation ────────────


def test_http_403_and_404_render_single_danger_banner_without_duplication():
    app = _load_app_module()
    _set_state(
        dict(
            app.DEFAULT_STATE,
            mode="list",
            query="Alan Turing",
            searched_query="Alan Turing",
            loading=True,
            pending="search",
            active_tag="ui:search:1",
            editing_search=False,
        )
    )

    effects = app.update(
        HttpResponse(
            status=403,
            headers=[],
            body=b"Forbidden: missing User-Agent",
            tag="ui:search:1",
        )
    )
    st = next(e for e in effects if isinstance(e, SetState)).data
    assert st["loading"] is False
    assert "HTTP 403" in st["error"]

    _set_state(st)
    tree = _encode_uitree(app.view())
    nodes = [n["data"] for n in tree["nodes"]]
    danger_badges = [
        d for d in nodes if d.get("type") == "badge" and d.get("color") == "danger"
    ]
    assert len(danger_badges) == 1
    error_texts = [
        d for d in nodes if d.get("type") == "text" and "HTTP 403" in str(d.get("text", ""))
    ]
    assert len(error_texts) == 1, "error message must appear once in Banner, never duplicated"

    # Article 404 also renders a single danger Banner in article mode.
    _set_state(
        dict(
            app.DEFAULT_STATE,
            mode="article",
            article_title="Missing Page",
            loading=True,
            pending="article",
            active_tag="ui:article:2",
        )
    )
    effects = app.update(
        HttpResponse(status=404, headers=[], body=b"Not found", tag="ui:article:2")
    )
    st = next(e for e in effects if isinstance(e, SetState)).data
    assert "HTTP 404" in st["error"]
    _set_state(st)
    article_tree = _encode_uitree(app.view())
    article_nodes = [n["data"] for n in article_tree["nodes"]]
    assert (
        len([d for d in article_nodes if d.get("type") == "badge" and d.get("color") == "danger"])
        == 1
    )
    assert (
        len(
            [
                d
                for d in article_nodes
                if d.get("type") == "text" and "HTTP 404" in str(d.get("text", ""))
            ]
        )
        == 1
    )


def test_stale_tagged_ui_response_is_ignored():
    app = _load_app_module()
    _set_state(
        dict(
            app.DEFAULT_STATE,
            query="Second Query",
            searched_query="Second Query",
            loading=True,
            pending="search",
            request_seq=2,
            active_tag="ui:search:2",
        )
    )

    # Stale response from request 1 arrives late and must be ignored.
    stale_effects = app.update(_search_response(["Stale Result"], tag="ui:search:1"))
    assert stale_effects == []

    # Active response from request 2 updates state.
    fresh_effects = app.update(_search_response(["Fresh Result"], tag="ui:search:2"))
    st = next(e for e in fresh_effects if isinstance(e, SetState)).data
    assert [r["title"] for r in st["results"]] == ["Fresh Result"]


# ── ExposeTools / ToolCall coverage ──────────────────────────────────────────


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


def test_init_with_launch_args_starts_search_and_exposes_tools():
    app = _load_app_module()
    _set_state(dict(app.DEFAULT_STATE))

    effects = app.init(None, ["Alan", "Turing"])

    assert any(isinstance(e, ExposeTools) for e in effects)
    fetch = next(e for e in effects if isinstance(e, HttpFetch))
    assert "srsearch=Alan%20Turing" in fetch.url
    st = next(e for e in effects if isinstance(e, SetState)).data
    assert st["editing_search"] is False


def test_tool_call_happy_path_search_then_article_and_does_not_block_ui():
    app = _load_app_module()
    _set_state(dict(app.DEFAULT_STATE, query="Ada Lovelace"))

    effects = app.update(
        ToolCall("call-1", "wikipedia.search", '{"query":"Alan Turing"}', "assistant")
    )
    assert len(effects) == 1
    assert isinstance(effects[0], HttpFetch)
    search_tag = effects[0].tag
    assert search_tag == "tool:search:call-1"

    # A concurrent UI search is NOT blocked while the tool call is in flight.
    ui_effects = app.update(UiAction("wiki-submit"))
    assert any(isinstance(e, HttpFetch) for e in ui_effects)

    # Resolve tool search -> article fetch.
    effects = app.update(
        _search_response([("Alan Turing", "English mathematician")], tag=search_tag)
    )
    assert len(effects) == 1
    assert isinstance(effects[0], HttpFetch)
    assert effects[0].url.endswith("/Alan_Turing")
    article_tag = effects[0].tag
    assert article_tag == "tool:article:call-1"

    # Resolve tool article -> ToolResult.
    effects = app.update(
        _article_response(
            "Alan Turing",
            "Alan Mathison Turing was an English mathematician.",
            tag=article_tag,
        )
    )
    assert len(effects) == 1
    result = effects[0]
    assert isinstance(result, ToolResult)
    assert result.call_id == "call-1"
    assert result.error is None
    assert json.loads(result.output_json or "{}") == {
        "title": "Alan Turing",
        "summary": "Alan Mathison Turing was an English mathematician.",
    }


def test_tool_call_validation_and_http_errors():
    app = _load_app_module()
    _set_state(dict(app.DEFAULT_STATE))

    # Malformed JSON
    res = app.update(ToolCall("c-bad", "wikipedia.search", "{bad", "assistant"))
    assert isinstance(res[0], ToolResult) and res[0].error

    # Missing query
    res = app.update(ToolCall("c-empty", "wikipedia.search", '{"query":"  "}', "assistant"))
    assert isinstance(res[0], ToolResult) and res[0].error

    # Unknown tool
    res = app.update(ToolCall("c-unknown", "wikipedia.other", "{}", "assistant"))
    assert isinstance(res[0], ToolResult) and res[0].error

    # Search HTTP 500 error cleans up tool state
    fetches = app.update(ToolCall("c-500", "wikipedia.search", '{"query":"Plexi"}', "assistant"))
    res = app.update(HttpResponse(status=500, headers=[], body=b"boom", tag=fetches[0].tag))
    assert isinstance(res[0], ToolResult) and "HTTP 500" in (res[0].error or "")
    assert not app._pending_tools


# ── End-to-end AppHarness integration test ───────────────────────────────────


def test_app_harness_search_snippets_open_article_and_navigate_back():
    with AppTestHarness(str(APP), width=900, height=620) as h:
        h.run(1)

        # Enter a search query and submit.
        h.text_submit("wiki-query", "Alan Turing")
        h.ui_action("wiki-submit")
        h.run(1)

        http_reqs = [e for e in h._events_seen if e.get("type") == "http_request"]
        assert http_reqs, "submitting search must emit http_request"
        search_req = http_reqs[-1]
        assert search_req["headers"]["User-Agent"] == "Plexi/1.0 (https://plexiapp.com)"
        assert "srsearch=Alan%20Turing" in search_req["url"]

        # Reply with MediaWiki search results containing HTML snippets.
        h._send(
            {
                "type": "http_response",
                "request_id": search_req["request_id"],
                "status": 200,
                "body": json.dumps(
                    {
                        "query": {
                            "search": [
                                {
                                    "title": "Alan Turing",
                                    "snippet": '<span class="searchmatch">Alan</span> Mathison Turing was an English mathematician.',
                                },
                                {
                                    "title": "Turing machine",
                                    "snippet": "Mathematical model of computation.",
                                },
                            ]
                        }
                    }
                ),
            }
        )
        h.run(1)

        # Navigate down with "j" and press "enter" to open "Turing machine".
        h.key("j")
        h.key("enter")
        h.run(1)

        article_reqs = [e for e in h._events_seen if e.get("type") == "http_request"]
        assert len(article_reqs) >= 2
        article_req = article_reqs[-1]
        assert article_req["url"].endswith("/Turing_machine")

        # Reply with REST summary payload.
        h._send(
            {
                "type": "http_response",
                "request_id": article_req["request_id"],
                "status": 200,
                "body": json.dumps(
                    {
                        "title": "Turing machine",
                        "description": "Mathematical model of computation",
                        "extract": "A Turing machine is a mathematical model of computation describing an abstract machine.",
                    }
                ),
            }
        )
        cmds = h.run(1)
        trees = [c for c in cmds if c.get("type") == "component_tree"]
        assert trees
        nodes = [n["data"] for n in trees[-1]["tree"]["nodes"]]
        texts = [d.get("text", "") for d in nodes if d.get("type") == "text"]
        assert any("mathematical model of computation" in t for t in texts)

        # Press "b" to return to the results list at row 1.
        h.key("b")
        cmds = h.run(1)
        trees = [c for c in cmds if c.get("type") == "component_tree"]
        assert trees
        list_views = [
            n["data"]
            for n in trees[-1]["tree"]["nodes"]
            if n["data"].get("type") == "ListView"
        ]
        assert list_views and list_views[0]["selected"] == 1
