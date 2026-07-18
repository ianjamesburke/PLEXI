#!/usr/bin/env python3
"""Wikipedia — SDK v3 HTTP-backed article search."""

from __future__ import annotations

import json
import urllib.parse

from plexi_sdk import log, state
from plexi_sdk.effects import HttpFetch, SetState, SetStatus, SetTitle
from plexi_sdk.events import HttpResponse, KeyEvent, UiAction, UiValueChange
from plexi_sdk.ui import (
    AppBar,
    Button,
    Column,
    FooterKeys,
    Scrollable,
    SelectList,
    Spacer,
    Text,
    TextInput,
)

API = "https://en.wikipedia.org/w/api.php"
EXTRACT_API = "https://en.wikipedia.org/api/rest_v1/page/summary/"

DEFAULT_STATE = {
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


def init(size, args) -> list:
    data = _state()
    effects: list = [SetTitle("Wikipedia"), SetState(data), SetStatus("Wikipedia")]
    if args:
        data["query"] = " ".join(args)
        data["loading"] = True
        data["pending"] = "search"
        effects = [
            SetTitle("Wikipedia"),
            SetState(data),
            SetStatus("Searching"),
            _fetch_search(data["query"]),
        ]
    log.info("wikipedia: SDK v3 initialized")
    return effects


def update(event) -> list:
    data = _state()
    if isinstance(event, HttpResponse):
        data.update(_handle_http(data, event))
        return [SetState(data), SetStatus(_status(data))]
    if isinstance(event, UiValueChange) and event.handler_id == "wiki-query":
        data["query"] = event.value
        return [SetState(data)]
    if isinstance(event, UiAction) and event.handler_id == "wiki-submit":
        return _start_search(data)
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
            title = data["results"][data["selected"]]
            data["loading"] = True
            data["pending"] = "article"
            data["article_title"] = title
            data["error"] = ""
            return [
                SetState(data),
                SetStatus(f"Loading {title}"),
                _fetch_article(title),
            ]
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
    data["loading"] = True
    data["pending"] = "search"
    data["error"] = ""
    return [SetState(data), SetStatus("Searching"), _fetch_search(query)]


def _fetch_search(query: str) -> HttpFetch:
    params = urllib.parse.urlencode(
        {
            "action": "opensearch",
            "search": query,
            "limit": 10,
            "format": "json",
        }
    )
    return HttpFetch(f"{API}?{params}", headers={"Accept": "application/json"})


def _fetch_article(title: str) -> HttpFetch:
    return HttpFetch(
        EXTRACT_API + urllib.parse.quote(title), headers={"Accept": "application/json"}
    )


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
    return Column(
        [
            AppBar("Wikipedia", "search"),
            TextInput(
                "wiki-query",
                value=data["query"],
                placeholder="Search Wikipedia",
                on_change="wiki-query",
                on_submit="wiki-submit",
            ),
            Button(
                "Search",
                "wiki-submit",
                style="primary",
                disabled=not data["query"].strip(),
            ),
            Text(
                data["error"] if data["error"] else "Type a query and press Enter.",
                size=12.0,
            ),
            Spacer(grow=True),
            FooterKeys([("enter", "search")]),
        ],
        grow=True,
    )


def _results_view(data: dict):
    rows = [{"name": title} for title in data["results"]]
    body = (
        SelectList(rows, selected_idx=data["selected"])
        if rows
        else Text(data["error"] or "No results.", size=12.0)
    )
    return Column(
        [
            AppBar("Wikipedia", f"Results for {data['query']}"),
            body,
            Spacer(grow=True),
            FooterKeys([("j/k", "select"), ("enter", "open"), ("esc", "search")]),
        ],
        grow=True,
    )


def _article_view(data: dict):
    return Column(
        [
            AppBar("Wikipedia", data["article_title"]),
            Scrollable(
                Text(data["article"] or data["error"] or "Loading...", size=12.0)
            ),
            Spacer(grow=True),
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
