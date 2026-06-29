#!/usr/bin/env python3
"""Wikipedia - SDK v3 search and summary browser."""

from __future__ import annotations

import json
import urllib.parse

from plexi_sdk import log, state
from plexi_sdk.effects import HttpFetch, RequestCapability, SetState, SetStatus, SetTitle
from plexi_sdk.events import CapabilityDenied, CapabilityGranted, HttpResponse, KeyEvent, UiAction, UiValueChange
from plexi_sdk.ui import ActionBar, AppBar, Button, Column, FooterKeys, Scrollable, SelectList, Spacer, Text, TextEdit

SEARCH_API = "https://en.wikipedia.org/w/api.php"
SUMMARY_API = "https://en.wikipedia.org/api/rest_v1/page/summary/"

DEFAULT_STATE = {
    "query": "",
    "results": [],
    "selected": 0,
    "mode": "search",
    "loading": False,
    "pending": "",
    "return_mode": "search",
    "article_title": "",
    "article": "",
    "error": "",
    "net_http_granted": False,
}


def init(size, args) -> list:
    data = _state()
    if args:
        data["query"] = " ".join(args)
    log.info("wikipedia: sdk v3 app initialized")
    return [
        SetTitle("Wikipedia"),
        SetState(data),
        SetStatus("Requesting network access"),
        RequestCapability("net.http"),
    ]


def update(event) -> list:
    data = _state()

    if isinstance(event, CapabilityGranted) and event.name == "net.http":
        data["net_http_granted"] = True
        if data["query"].strip():
            return _search(data)
        return _set(data)

    if isinstance(event, CapabilityDenied) and event.name == "net.http":
        data["net_http_granted"] = False
        data["error"] = "Network access denied"
        return _set(data)

    if isinstance(event, HttpResponse):
        data = _handle_http(data, event)
        return _set(data)

    if isinstance(event, UiValueChange) and event.handler_id == "wiki-query":
        data["query"] = event.value
        data["error"] = ""
        return [SetState(data)]

    if isinstance(event, UiAction):
        if event.handler_id in ("wiki-query", "wiki-search"):
            return _search(data)
        if event.handler_id == "wiki-back-search":
            data["mode"] = "search"
            data["loading"] = False
            data["pending"] = ""
            return _set(data)
        if event.handler_id == "wiki-back-results":
            data["mode"] = "results"
            return _set(data)

    if not isinstance(event, KeyEvent) or not event.pressed:
        return []

    key = event.key
    if data["loading"]:
        if key == "escape":
            data["loading"] = False
            data["pending"] = ""
            data["mode"] = data["return_mode"] if data["return_mode"] in ("search", "results") else "search"
            return _set(data)
        return []

    if data["mode"] == "results":
        if key in ("j", "down"):
            data["selected"] = _clamp(data["selected"] + 1, len(data["results"]))
            return _set(data)
        if key in ("k", "up"):
            data["selected"] = _clamp(data["selected"] - 1, len(data["results"]))
            return _set(data)
        if key in ("enter", "return") and data["results"]:
            return _open_article(data, data["results"][data["selected"]])
        if key == "escape":
            data["mode"] = "search"
            return _set(data)
    elif data["mode"] == "article":
        if key == "escape":
            data["mode"] = "results"
            return _set(data)
    return []


def view():
    data = _state()
    if data["loading"]:
        return _loading_view(data)
    if data["mode"] == "results":
        return _results_view(data)
    if data["mode"] == "article":
        return _article_view(data)
    return _search_view(data)


def _state() -> dict:
    data = dict(DEFAULT_STATE)
    for key, value in DEFAULT_STATE.items():
        data[key] = state.get(key, value)
    data["query"] = str(data.get("query") or "")
    data["results"] = [str(item) for item in list(data.get("results") or [])]
    data["selected"] = _clamp(int(data.get("selected") or 0), len(data["results"]))
    data["mode"] = data.get("mode") if data.get("mode") in ("search", "results", "article") else "search"
    data["loading"] = bool(data.get("loading"))
    data["pending"] = str(data.get("pending") or "")
    data["return_mode"] = str(data.get("return_mode") or "search")
    data["article_title"] = str(data.get("article_title") or "")
    data["article"] = str(data.get("article") or "")
    data["error"] = str(data.get("error") or "")
    data["net_http_granted"] = bool(data.get("net_http_granted"))
    return data


def _search(data: dict) -> list:
    query = data["query"].strip()
    if not query:
        data["error"] = "Enter a search term."
        return _set(data)
    if not data["net_http_granted"]:
        data["error"] = "Waiting for network access."
        return _set(data)
    data["loading"] = True
    data["pending"] = "search"
    data["return_mode"] = "search"
    data["error"] = ""
    log.info(f"wikipedia: search start query={query!r}")
    return [_state_effect(data), SetStatus("Searching"), _fetch_search(query)]


def _open_article(data: dict, title: str) -> list:
    data["loading"] = True
    data["pending"] = "article"
    data["return_mode"] = "results"
    data["article_title"] = title
    data["article"] = ""
    data["error"] = ""
    log.info(f"wikipedia: article start title={title!r}")
    return [_state_effect(data), SetStatus(f"Loading {title}"), _fetch_article(title)]


def _handle_http(data: dict, event: HttpResponse) -> dict:
    data["loading"] = False
    status = int(event.status or 0)
    if status < 200 or status >= 300:
        data["pending"] = ""
        data["error"] = f"HTTP {status}: {_body_text(event)[:180]}"
        log.warn(f"wikipedia: http failed status={status}")
        return data

    try:
        payload = json.loads(_body_text(event))
    except json.JSONDecodeError as exc:
        data["pending"] = ""
        data["error"] = f"Invalid response: {exc}"
        return data

    if data["pending"] == "search":
        data["results"] = _parse_search(payload)
        data["selected"] = 0
        data["mode"] = "results"
        log.info(f"wikipedia: search complete results={len(data['results'])}")
    elif data["pending"] == "article":
        data["article"] = str(payload.get("extract") or "No summary available.")
        data["mode"] = "article"
        log.info(f"wikipedia: article complete title={data['article_title']!r}")
    data["pending"] = ""
    data["error"] = "" if data["results"] or data["mode"] == "article" else "No results."
    return data


def _search_view(data: dict):
    subtitle = "network ready" if data["net_http_granted"] else "requesting network"
    return Column(
        [
            AppBar("Wikipedia", subtitle),
            TextEdit("wiki-query", value=data["query"], placeholder="Search Wikipedia"),
            ActionBar([Button("Search", "wiki-search", style="primary", disabled=not data["query"].strip())]),
            Text(data["error"] or "Type a query and press Enter.", size=12.0),
            FooterKeys([("enter", "search")]),
        ],
        grow=True,
        padding=0,
    )


def _loading_view(data: dict):
    label = "Searching" if data["pending"] == "search" else f"Loading {data['article_title']}"
    return Column(
        [
            AppBar("Wikipedia", label),
            Spacer(size=24),
            Text(label, size=16.0, bold=True),
            Text("Please wait.", size=12.0),
            FooterKeys([("esc", "search")]),
        ],
        grow=True,
        padding=16,
    )


def _results_view(data: dict):
    rows = [{"name": title} for title in data["results"]]
    body = SelectList(rows, selected_idx=data["selected"]) if rows else Text(data["error"] or "No results.", size=12.0)
    return Column(
        [
            AppBar("Wikipedia", f"Results for {data['query']}"),
            body,
            ActionBar([Button("Back", "wiki-back-search", style="ghost")]),
            FooterKeys([("j/k", "select"), ("enter", "open"), ("esc", "search")]),
        ],
        grow=True,
        padding=0,
    )


def _article_view(data: dict):
    return Column(
        [
            AppBar("Wikipedia", data["article_title"]),
            Scrollable(Text(data["article"] or data["error"], size=12.0)),
            ActionBar([Button("Back", "wiki-back-results", style="ghost")]),
            FooterKeys([("esc", "results")]),
        ],
        grow=True,
        padding=0,
    )


def _set(data: dict) -> list:
    return [_state_effect(data), SetStatus(_status(data))]


def _state_effect(data: dict) -> SetState:
    return SetState(data)


def _status(data: dict) -> str:
    if data["loading"]:
        return "Loading"
    if data["error"]:
        return "Error"
    if data["mode"] == "results":
        return f"{len(data['results'])} results"
    if data["mode"] == "article":
        return data["article_title"]
    return "Wikipedia"


def _fetch_search(query: str) -> HttpFetch:
    params = urllib.parse.urlencode(
        {"action": "opensearch", "search": query, "limit": 12, "format": "json"}
    )
    return HttpFetch(f"{SEARCH_API}?{params}", headers={"Accept": "application/json"})


def _fetch_article(title: str) -> HttpFetch:
    return HttpFetch(SUMMARY_API + urllib.parse.quote(title), headers={"Accept": "application/json"})


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


def _clamp(selected: int, total: int) -> int:
    if total <= 0:
        return 0
    return max(0, min(selected, total - 1))
