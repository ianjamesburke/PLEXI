#!/usr/bin/env python3
"""Wikipedia — SDK v3 HTTP-backed article search."""

from __future__ import annotations

import json
from typing import Any

from plexi_sdk import log, state
from plexi_sdk.effects import (
    AiTool,
    ExposeTools,
    HttpFetch,
    SetState,
    SetStatus,
    SetTitle,
    ToolResult,
)
from plexi_sdk.events import HttpResponse, KeyEvent, ToolCall, UiAction, UiValueChange
from plexi_sdk.ui import (
    AppBar,
    Button,
    Card,
    Column,
    EmptyState,
    FooterKeys,
    Scrollable,
    Section,
    Skeleton,
    Spacer,
    Text,
    TextInput,
)

API = "https://en.wikipedia.org/w/api.php"
EXTRACT_API = "https://en.wikipedia.org/api/rest_v1/page/summary/"

DEFAULT_STATE: dict[str, Any] = {
    "query": "",
    "results": [],
    "selected": 0,
    "article": "",
    "article_title": "",
    "loading": False,
    "pending": "",
    "error": "",
    "mode": "search",
}

_SEARCH_TOOL_SCHEMA = {
    "type": "object",
    "properties": {
        "query": {"type": "string", "description": "Article title or search phrase."},
    },
    "required": ["query"],
}

_SEARCH_RESULT_SCHEMA = {
    "type": "object",
    "properties": {
        "title": {"type": "string"},
        "summary": {"type": "string"},
    },
    "required": ["title", "summary"],
}

# Correlates an in-flight assistant tool call across the HTTP round trip(s) it
# takes to resolve (search -> article). Module-level global rather than
# state/SetState: the HttpFetch/HttpResponse transport is single-flight with
# no request id, so at most one fetch (UI or tool) can be outstanding at a
# time, and this bookkeeping is purely about that transport, not app state.
_pending_tool: dict | None = None


def init(size, args) -> list:
    data = _state()
    tools_effect = ExposeTools(_tools())
    effects: list = [
        SetTitle("Wikipedia"),
        SetState(data),
        SetStatus("Wikipedia"),
        tools_effect,
    ]
    if args:
        data["query"] = " ".join(args)
        data["loading"] = True
        data["pending"] = "search"
        effects = [
            SetTitle("Wikipedia"),
            SetState(data),
            SetStatus("Searching"),
            _fetch_search(data["query"]),
            tools_effect,
        ]
    log.info("wikipedia: SDK v3 initialized")
    return effects


def _tools() -> list[AiTool]:
    return [
        AiTool(
            name="wikipedia.search",
            description="Search Wikipedia for an article and return its summary.",
            input_schema=_SEARCH_TOOL_SCHEMA,
            output_schema=_SEARCH_RESULT_SCHEMA,
            read_only=True,
        )
    ]


def update(event) -> list:
    global _pending_tool
    if isinstance(event, ToolCall):
        return _handle_tool_call(event)
    data = _state()
    if isinstance(event, HttpResponse):
        if _pending_tool is not None:
            return _handle_tool_http(event)
        data.update(_handle_http(data, event))
        return [SetState(data), SetStatus(_status(data))]
    if isinstance(event, UiValueChange) and event.handler_id == "wiki-query":
        data["query"] = event.value
        return [SetState(data)]
    if isinstance(event, UiAction) and event.handler_id == "wiki-submit":
        return _start_search(data)
    if isinstance(event, UiAction) and event.handler_id == "wiki-back-search":
        data["mode"] = "search"
        return [SetState(data), SetStatus(_status(data))]
    if isinstance(event, UiAction) and event.handler_id == "wiki-back-results":
        data["mode"] = "results"
        return [SetState(data), SetStatus(_status(data))]
    if isinstance(event, UiAction) and event.handler_id.startswith("wiki-result-"):
        try:
            selected = int(event.handler_id.removeprefix("wiki-result-"))
        except ValueError:
            return []
        if 0 <= selected < len(data["results"]):
            return _start_article(data, selected)
        return []
    if not isinstance(event, KeyEvent) or not event.pressed:
        return []

    key = event.key
    if data["mode"] == "search":
        if key in ("return", "enter"):
            return _start_search(data)
        return []
    if data["mode"] == "results":
        if key in ("down", "j", "ArrowDown"):
            data["selected"] = _clamp(data["selected"] + 1, len(data["results"]))
        elif key in ("up", "k", "ArrowUp"):
            data["selected"] = _clamp(data["selected"] - 1, len(data["results"]))
        elif key in ("return", "enter") and data["results"]:
            return _start_article(data, data["selected"])
        elif key == "escape":
            data["mode"] = "search"
        else:
            return []
        return [SetState(data), SetStatus(_status(data))]
    if data["mode"] == "article" and key == "escape":
        data["mode"] = "results"
        return [SetState(data), SetStatus(_status(data))]
    return []


def view():
    data = _state()
    if data["mode"] == "results":
        return _results_view(data)
    if data["mode"] == "article":
        return _article_view(data)
    return _search_view(data)


def _state() -> dict:
    data = dict(DEFAULT_STATE)
    for key, value in DEFAULT_STATE.items():
        data[key] = state.get(key, value)
    data["results"] = list(data.get("results") or [])
    data["selected"] = _clamp(int(data.get("selected") or 0), len(data["results"]))
    data["query"] = str(data.get("query") or "")
    data["article"] = str(data.get("article") or "")
    data["error"] = str(data.get("error") or "")
    return data


def _start_search(data: dict) -> list:
    query = data["query"].strip()
    if not query:
        return []
    if _pending_tool is not None:
        log.info("wikipedia: UI search blocked, tool fetch in flight")
        return []
    log.info(f"wikipedia: searching for {query!r}")
    data["loading"] = True
    data["pending"] = "search"
    data["error"] = ""
    return [SetState(data), SetStatus("Searching"), _fetch_search(query)]


def _start_article(data: dict, selected: int) -> list:
    if _pending_tool is not None:
        log.info("wikipedia: UI article load blocked, tool fetch in flight")
        return []
    title = data["results"][selected]
    log.info(f"wikipedia: loading article {title!r}")
    data["selected"] = selected
    data["loading"] = True
    data["pending"] = "article"
    data["article_title"] = title
    data["error"] = ""
    return [
        SetState(data),
        SetStatus(f"Loading {title}"),
        _fetch_article(title),
    ]


def _fetch_search(query: str) -> HttpFetch:
    params = (
        "action=opensearch"
        f"&search={_quote(query)}"
        "&limit=10"
        "&format=json"
    )
    return HttpFetch(f"{API}?{params}", headers={"Accept": "application/json"})


def _fetch_article(title: str) -> HttpFetch:
    return HttpFetch(
        EXTRACT_API + _quote(title), headers={"Accept": "application/json"}
    )


def _quote(value: str) -> str:
    """Percent-encode UTF-8 without importing urllib, which stalls in CPython-WASM."""
    safe = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"
    return "".join(
        chr(byte) if byte in safe else f"%{byte:02X}" for byte in value.encode("utf-8")
    )


def _handle_tool_call(call: ToolCall) -> list:
    global _pending_tool
    log.info(f"wikipedia: tool call {call.name} from {call.caller_id}")
    if call.name != "wikipedia.search":
        return [ToolResult(call.call_id, error=f"unknown tool '{call.name}'")]
    try:
        payload = json.loads(call.input_json or "{}")
    except ValueError as e:
        return [ToolResult(call.call_id, error=f"input_json is not valid JSON: {e}")]
    if not isinstance(payload, dict):
        return [ToolResult(call.call_id, error="input must be a JSON object")]
    query = payload.get("query")
    if not isinstance(query, str) or not query.strip():
        return [ToolResult(call.call_id, error="'query' must be a non-empty string")]
    query = query.strip()

    data = _state()
    if data["loading"] or _pending_tool is not None:
        log.info(f"wikipedia: tool call {call.name} rejected, request in flight")
        return [
            ToolResult(
                call.call_id,
                error="Wikipedia is busy with another request; try again shortly.",
            )
        ]

    _pending_tool = {"call_id": call.call_id, "stage": "search", "query": query}
    log.info(f"wikipedia: tool search stage started for {query!r}")
    return [_fetch_search(query)]


def _handle_tool_http(event: HttpResponse) -> list:
    global _pending_tool
    assert _pending_tool is not None
    call_id = _pending_tool["call_id"]

    if event.status < 200 or event.status >= 300:
        log.warn(f"wikipedia: tool request failed {event.status}")
        _pending_tool = None
        return [
            ToolResult(call_id, error=f"HTTP {event.status}: {_body_text(event)[:240]}")
        ]

    try:
        payload = json.loads(_body_text(event))
    except json.JSONDecodeError as exc:
        _pending_tool = None
        return [ToolResult(call_id, error=f"invalid JSON response: {exc}")]

    if _pending_tool["stage"] == "search":
        results = _parse_search(payload)
        if not results:
            log.info(f"wikipedia: tool search for {_pending_tool['query']!r} -> 0 results")
            _pending_tool = None
            return [ToolResult(call_id, error="No Wikipedia article found for that query.")]
        title = results[0]
        log.info(f"wikipedia: tool search stage -> article stage for {title!r}")
        _pending_tool["stage"] = "article"
        _pending_tool["title"] = title
        return [_fetch_article(title)]

    # stage == "article"
    title = _pending_tool["title"]
    summary = str(payload.get("extract") or "No extract available.")
    _pending_tool = None
    log.info(f"wikipedia: tool article stage complete for {title!r}")
    return [
        ToolResult(call_id, output_json=json.dumps({"title": title, "summary": summary}))
    ]


def _handle_http(data: dict, event: HttpResponse) -> dict:
    if event.status < 200 or event.status >= 300:
        data["loading"] = False
        data["pending"] = ""
        data["error"] = f"HTTP {event.status}: {_body_text(event)[:240]}"
        log.warn(f"wikipedia: request failed {event.status}")
        return data
    try:
        payload = json.loads(_body_text(event))
    except json.JSONDecodeError as exc:
        data["loading"] = False
        data["pending"] = ""
        data["error"] = str(exc)
        return data
    if data["pending"] == "search":
        data["results"] = _parse_search(payload)
        data["selected"] = 0
        data["mode"] = "results"
        log.info(
            f"wikipedia: search {data['query']!r} -> {len(data['results'])} results"
        )
    elif data["pending"] == "article":
        data["article"] = str(payload.get("extract") or "No extract available.")
        data["mode"] = "article"
        log.info(f"wikipedia: article {data['article_title']!r} loaded")
    data["loading"] = False
    data["pending"] = ""
    data["error"] = ""
    return data


def _parse_search(payload) -> list[str]:
    if isinstance(payload, list) and len(payload) > 1 and isinstance(payload[1], list):
        return [str(item) for item in payload[1]]
    return []


def _body_text(event: HttpResponse) -> str:
    if isinstance(event.body, bytes):
        return event.body.decode("utf-8", errors="replace")
    if isinstance(event.body, list):
        return bytes(event.body).decode("utf-8", errors="replace")
    return str(event.body)


def _search_view(data: dict):
    content = (
        Card([Skeleton(rows=3)])
        if data["loading"]
        else Card(
            [
                Text("Search the free encyclopedia", size=16.0, bold=True),
                Text(
                    "Find an article, then read its summary without leaving Plexi.",
                    size=12.0,
                ),
                TextInput(
                    "wiki-query",
                    value=data["query"],
                    placeholder="Try “Detroit”, “Tetris”, or “Ada Lovelace”",
                    on_change="wiki-query",
                    on_submit="wiki-submit",
                ),
                Button(
                    "Search Wikipedia",
                    "wiki-submit",
                    style="primary",
                    disabled=not data["query"].strip(),
                ),
            ],
            gap=12.0,
        )
    )
    return Column(
        [
            AppBar("Wikipedia", "The free encyclopedia"),
            content,
            Text(data["error"], size=12.0) if data["error"] else Spacer(),
            Spacer(grow=True),
            FooterKeys([("enter", "search")]),
        ],
        grow=True,
    )


def _results_view(data: dict):
    rows = [
        Button(
            title,
            f"wiki-result-{index}",
            style="primary" if index == data["selected"] else "secondary",
        )
        for index, title in enumerate(data["results"])
    ]
    body = Scrollable(Column(rows, gap=8.0)) if rows else EmptyState(
        "No articles found",
        "Try a broader or differently spelled search.",
        icon="∅",
    )
    return Column(
        [
            AppBar("Wikipedia", f"Search results for “{data['query']}”"),
            Button("← New search", "wiki-back-search", style="secondary"),
            Section("Articles"),
            body,
            FooterKeys([("j/k", "select"), ("enter", "open"), ("esc", "search")]),
        ],
        grow=True,
    )


def _article_view(data: dict):
    article = (
        Card([Skeleton(rows=5)])
        if data["loading"]
        else Card([Text(data["article"] or data["error"] or "No summary available.", size=13.0)])
    )
    return Column(
        [
            AppBar("Wikipedia", data["article_title"]),
            Button("← Search results", "wiki-back-results", style="secondary"),
            Section("Summary"),
            Scrollable(article),
            FooterKeys([("esc", "results")]),
        ],
        grow=True,
    )


def _status(data: dict) -> str:
    if data["loading"]:
        return "Loading"
    if data["error"]:
        return "Error"
    if data["mode"] == "results":
        return f"{len(data['results'])} results"
    return data["article_title"] if data["mode"] == "article" else "Wikipedia"


def _clamp(selected: int, total: int) -> int:
    if total <= 0:
        return 0
    return max(0, min(selected, total - 1))
