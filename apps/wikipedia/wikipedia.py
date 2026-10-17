#!/usr/bin/env python3
"""Wikipedia — SDK v3 two-mode encyclopedia search and article reader."""

from __future__ import annotations

import json
import re
from typing import Any

from plexi_sdk import log, state, theme
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
    Badge,
    Banner,
    Card,
    Column,
    Component,
    Divider,
    EmptyState,
    FooterKeys,
    Pending,
    Row,
    Scrollable,
    SelectList,
    Skeleton,
    Spacer,
    Text,
    TextInput,
)

API = "https://en.wikipedia.org/w/api.php"
EXTRACT_API = "https://en.wikipedia.org/api/rest_v1/page/summary/"
USER_AGENT = "Plexi/1.0 (https://plexiapp.com)"
HEADERS: dict[str, str] = {
    "User-Agent": USER_AGENT,
    "Accept": "application/json",
}

SEARCH_INPUT_ID = "wiki-query"
SEARCH_SUBMIT_ACTION = "wiki-submit"
OPEN_ROW_ACTION = "wiki-open-row"

_TAG_RE = re.compile(r"<[^>]+>")
_ENTITY_MAP = {
    "&quot;": '"',
    "&amp;": "&",
    "&lt;": "<",
    "&gt;": ">",
    "&#039;": "'",
    "&#39;": "'",
    "&apos;": "'",
    "&nbsp;": " ",
    "&ndash;": "–",
    "&#8211;": "–",
    "&mdash;": "—",
    "&#8212;": "—",
}
_NUMERIC_ENTITY_RE = re.compile(r"&#(x?[0-9a-fA-F]+);")

DEFAULT_STATE: dict[str, Any] = {
    "mode": "list",
    "query": "",
    "searched_query": "",
    "results": [],
    "selected": 0,
    "article": "",
    "article_title": "",
    "article_description": "",
    "loading": False,
    "pending": "",
    "error": "",
    "editing_search": True,
    "blur_gen": 0,
    "request_seq": 0,
    "active_tag": "",
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

# In-flight assistant tool calls keyed by their active HTTP request tag.
_pending_tools: dict[str, dict[str, Any]] = {}


def init(size, args) -> list:
    data = _state()
    status = "Wikipedia"
    in_flight: list = []
    if args:
        query = " ".join(str(a) for a in args).strip()
        if query:
            data["query"] = query
            data["searched_query"] = query
            data["editing_search"] = False
            data["blur_gen"] = 1
            data["loading"] = True
            data["pending"] = "search"
            data["request_seq"] = 1
            tag = "ui:search:1"
            data["active_tag"] = tag
            status = "Searching"
            in_flight = [_fetch_search(query, tag=tag)]
            log.info(f"wikipedia: launch search started for {query!r} (tag={tag})")
    log.info("wikipedia: SDK v3 initialized")
    return [
        SetTitle("Wikipedia"),
        SetState(data),
        SetStatus(status),
        *in_flight,
        ExposeTools(_tools()),
    ]


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
    if isinstance(event, ToolCall):
        return _handle_tool_call(event)

    data = _state()

    if isinstance(event, HttpResponse):
        resp_tag = _response_tag(event)
        if resp_tag and resp_tag in _pending_tools:
            return _handle_tool_http(resp_tag, event)
        if resp_tag and resp_tag.startswith("tool:"):
            log.info(f"wikipedia: ignoring stale tool HTTP response tag={resp_tag!r}")
            return []
        if not resp_tag and _pending_tools and not data["loading"]:
            first_tag = next(iter(_pending_tools))
            return _handle_tool_http(first_tag, event)
        if resp_tag and data["active_tag"] and resp_tag != data["active_tag"]:
            log.info(
                f"wikipedia: ignoring stale UI HTTP response tag={resp_tag!r} "
                f"(active={data['active_tag']!r})"
            )
            return []
        data.update(_handle_http(data, event))
        return [SetState(data), SetStatus(_status(data))]

    if isinstance(event, UiValueChange) and event.handler_id == SEARCH_INPUT_ID:
        data["query"] = event.value
        return [SetState(data)]

    if isinstance(event, UiAction):
        if event.handler_id == SEARCH_SUBMIT_ACTION:
            return _submit_or_open(data)
        if event.handler_id.startswith(f"{OPEN_ROW_ACTION}:"):
            idx = _parse_row_index(event.handler_id)
            if idx is None or not (0 <= idx < len(data["results"])):
                log.warn(f"wikipedia: ignored out-of-range row action {event.handler_id!r}")
                return []
            return _start_article(data, idx)
        return []

    if not isinstance(event, KeyEvent) or not event.pressed:
        return []

    key = _canonical_key(event.key)
    if data["mode"] == "article":
        return _handle_article_key(data, key)
    return _handle_list_key(data, key)


def _submit_or_open(data: dict[str, Any]) -> list:
    query = data["query"].strip()
    if data["editing_search"] or query != data["searched_query"] or not data["results"]:
        if not query:
            if data["results"]:
                data["editing_search"] = False
                data["blur_gen"] = int(data.get("blur_gen") or 0) + 1
                log.info("wikipedia: empty search submit -> released search focus")
            else:
                data["editing_search"] = True
                log.info("wikipedia: empty search submit with no results -> keeping search focus")
            return [SetState(data), SetStatus(_status(data))]
        return _start_search(data)
    return _start_article(data, data["selected"])


def _handle_list_key(data: dict[str, Any], key: str) -> list:
    if key == "escape":
        if data["editing_search"]:
            if data["results"]:
                data["editing_search"] = False
                data["blur_gen"] = int(data.get("blur_gen") or 0) + 1
                log.info("wikipedia: search focus released via escape")
            else:
                data["editing_search"] = True
                log.info("wikipedia: escape with no results -> keeping search focus")
            return [SetState(data), SetStatus(_status(data))]
        return []

    if key == "/":
        data["editing_search"] = True
        log.info("wikipedia: focused search input via /")
        return [SetState(data), SetStatus(_status(data))]

    if key == "enter":
        return _submit_or_open(data)

    if key == "r":
        retry_query = data["query"].strip() or data["searched_query"].strip()
        if not retry_query:
            return []
        data["query"] = retry_query
        log.info(f"wikipedia: retrying search for {retry_query!r}")
        return _start_search(data)

    next_idx = SelectList.move_cursor(key, len(data["results"]), data["selected"])
    if next_idx is not None:
        data["selected"] = next_idx
        if data["editing_search"]:
            data["editing_search"] = False
            data["blur_gen"] = int(data.get("blur_gen") or 0) + 1
        title = _result_title(data["results"][next_idx]) if data["results"] else ""
        log.info(f"wikipedia: selected row {next_idx} ({title!r})")
        return [SetState(data), SetStatus(_status(data))]

    return []


def _handle_article_key(data: dict[str, Any], key: str) -> list:
    if key in ("escape", "b", "backspace"):
        data["mode"] = "list"
        data["editing_search"] = False
        if data["pending"] == "article":
            data["loading"] = False
            data["pending"] = ""
            data["active_tag"] = ""
        data["error"] = ""
        log.info(f"wikipedia: returned to results list at row {data['selected']}")
        return [SetState(data), SetStatus(_status(data))]

    if key in ("n", "]"):
        if not data["results"]:
            return []
        if data["selected"] + 1 >= len(data["results"]):
            log.info("wikipedia: already at last search result")
            return []
        return _start_article(data, data["selected"] + 1)

    if key in ("p", "["):
        if not data["results"]:
            return []
        if data["selected"] <= 0:
            log.info("wikipedia: already at first search result")
            return []
        return _start_article(data, data["selected"] - 1)

    if key == "/":
        data["mode"] = "list"
        data["editing_search"] = True
        data["loading"] = False
        data["pending"] = ""
        data["active_tag"] = ""
        data["error"] = ""
        log.info("wikipedia: jumped from article reader to new search")
        return [SetState(data), SetStatus(_status(data))]

    if key == "r":
        if data["results"] and 0 <= data["selected"] < len(data["results"]):
            return _start_article(data, data["selected"])
        if data["article_title"]:
            return _start_article_by_title(
                data,
                data["article_title"],
                data.get("article_description", ""),
            )
        return []

    return []


def view() -> Component:
    data = _state()
    if data["mode"] == "article":
        return _article_view(data)
    return _list_view(data)


def _list_view(data: dict[str, Any]) -> Component:
    search_field = TextInput(
        SEARCH_INPUT_ID,
        value=data["query"],
        on_change=SEARCH_INPUT_ID,
        on_submit=SEARCH_SUBMIT_ACTION,
        autofocus=bool(data["editing_search"]),
        placeholder="Search Wikipedia (e.g. Alan Turing, Ada Lovelace, Computing)…",
    )
    # When `editing_search` transitions to False on submit/Escape, wrapping the
    # unfocused TextInput shifts its arena node id so the host focus reconciler
    # surrenders the previously focused surface immediately instead of trapping
    # j/k/enter inside the text box.
    if data["editing_search"]:
        search_bar: Component = search_field
    else:
        depth = 1 + (int(data.get("blur_gen") or 0) % 2)
        node: Component = search_field
        for _ in range(depth):
            node = Column([node], gap=0.0)
        search_bar = node

    rows = [
        {
            "name": _result_title(item),
            "description": _result_snippet(item),
            "leading": "▸" if idx == data["selected"] else "·",
        }
        for idx, item in enumerate(data["results"])
    ]

    if rows:
        results_body: Component = SelectList(
            rows,
            selected_idx=data["selected"],
            on_select=OPEN_ROW_ACTION,
        )
    elif data["error"]:
        results_body = EmptyState(
            "Search unavailable",
            "Press r to retry or / to enter a new search query.",
            icon="!",
        )
    elif data["searched_query"]:
        results_body = EmptyState(
            "No matching articles",
            f"No Wikipedia articles matched “{data['searched_query']}”. Try another search term.",
            icon="∅",
        )
    else:
        results_body = EmptyState(
            "Explore Wikipedia",
            "Type a topic above and press Enter to search articles.",
            icon="W",
        )

    children: list[Component] = [
        AppBar("Wikipedia", _list_subtitle(data)),
        search_bar,
    ]
    if data["error"]:
        children.append(Banner(data["error"], tone="danger", title="Error"))
    children.extend(
        [
            Pending(
                active=bool(data["loading"] and data["pending"] == "search"),
                child=results_body,
                placeholder=Skeleton(rows=6),
            ),
            Spacer(grow=True),
            FooterKeys(
                [
                    ("/", "search"),
                    ("j/k", "navigate"),
                    ("enter", "read article"),
                    ("r", "retry"),
                ]
            ),
        ]
    )
    return Column(children, grow=True)


def _article_view(data: dict[str, Any]) -> Component:
    title = data["article_title"] or "Wikipedia Article"
    subtitle = _article_subtitle(data)
    total = len(data["results"])
    position_label = (
        f"Article {data['selected'] + 1} of {total}" if total > 0 else "Encyclopedia"
    )

    meta_children: list[Component] = [Badge(position_label, color="accent")]
    search_context = data["searched_query"] or data["query"].strip()
    if search_context:
        meta_children.append(
            Text(
                f"Search: “{search_context}”",
                size=12.0,
                color=theme.muted,
                truncate=True,
            )
        )
    meta_row = Row(meta_children, gap=8.0, align="center")

    children: list[Component] = [
        AppBar(title, subtitle),
        meta_row,
        Divider(),
    ]

    if data["error"]:
        children.append(Banner(data["error"], tone="danger", title="Error"))
        children.append(
            EmptyState(
                "Could not load article summary",
                "Press r to retry or esc/b to return to search results.",
                icon="!",
            )
        )
    else:
        summary_text = data["article"] or "No summary available."
        paragraphs = [p.strip() for p in summary_text.split("\n\n") if p.strip()] or [
            summary_text
        ]
        card_children: list[Component] = [
            Text(title, size=18.0, bold=True, color=theme.fg),
        ]
        if subtitle and subtitle != "Article reader":
            card_children.append(Text(subtitle, size=13.0, color=theme.muted))
        card_children.append(Divider())
        for para in paragraphs:
            card_children.append(Text(para, size=14.0, color=theme.fg))
        reader_body = Scrollable(
            Card(
                card_children,
                padding=20.0,
                gap=12.0,
            )
        )
        children.append(
            Pending(
                active=bool(data["loading"] and data["pending"] == "article"),
                child=reader_body,
                placeholder=Skeleton(rows=8),
            )
        )

    children.extend(
        [
            Spacer(grow=True),
            FooterKeys(
                [
                    ("esc/b", "back to results"),
                    ("n/p", "next/prev article"),
                    ("/", "new search"),
                ]
            ),
        ]
    )
    return Column(children, grow=True)


def _state() -> dict[str, Any]:
    data = dict(DEFAULT_STATE)
    for key, value in DEFAULT_STATE.items():
        data[key] = state.get(key, value)
    mode = str(data.get("mode") or "list")
    data["mode"] = mode if mode in ("list", "article") else "list"
    data["results"] = _normalize_results(data.get("results") or [])
    data["selected"] = _clamp(int(data.get("selected") or 0), len(data["results"]))
    data["query"] = str(data.get("query") or "")
    data["searched_query"] = str(data.get("searched_query") or "")
    data["article"] = str(data.get("article") or "")
    data["article_title"] = str(data.get("article_title") or "")
    data["article_description"] = str(data.get("article_description") or "")
    data["loading"] = bool(data.get("loading"))
    data["pending"] = str(data.get("pending") or "")
    data["error"] = str(data.get("error") or "")
    if "editing_search" in state.all():
        data["editing_search"] = bool(state.get("editing_search"))
    elif "query_autofocus" in state.all():
        data["editing_search"] = bool(state.get("query_autofocus"))
    else:
        data["editing_search"] = True
    data["blur_gen"] = int(data.get("blur_gen") or 0)
    data["request_seq"] = int(data.get("request_seq") or 0)
    data["active_tag"] = str(data.get("active_tag") or "")
    return data


def _normalize_results(raw_results: Any) -> list[dict[str, str]]:
    if not isinstance(raw_results, list):
        return []
    normalized: list[dict[str, str]] = []
    for item in raw_results:
        if isinstance(item, str):
            title = item.strip()
            if title:
                normalized.append({"title": title, "snippet": ""})
        elif isinstance(item, dict):
            title = str(item.get("title") or item.get("name") or "").strip()
            snippet = _strip_html(
                str(item.get("snippet") or item.get("description") or "")
            )
            if title:
                normalized.append({"title": title, "snippet": snippet})
    return normalized


def _result_title(item: Any) -> str:
    if isinstance(item, dict):
        return str(item.get("title") or item.get("name") or "")
    return str(item or "")


def _result_snippet(item: Any) -> str:
    if isinstance(item, dict):
        return str(item.get("snippet") or item.get("description") or "")
    return ""


def _start_search(data: dict[str, Any]) -> list:
    query = data["query"].strip()
    if not query:
        return []
    seq = int(data.get("request_seq") or 0) + 1
    tag = f"ui:search:{seq}"
    log.info(f"wikipedia: searching for {query!r} (tag={tag})")
    data["mode"] = "list"
    data["editing_search"] = False
    data["blur_gen"] = int(data.get("blur_gen") or 0) + 1
    data["loading"] = True
    data["pending"] = "search"
    data["error"] = ""
    data["searched_query"] = query
    data["request_seq"] = seq
    data["active_tag"] = tag
    return [SetState(data), SetStatus("Searching"), _fetch_search(query, tag=tag)]


def _start_article(data: dict[str, Any], selected: int) -> list:
    if not data["results"]:
        return []
    idx = _clamp(selected, len(data["results"]))
    item = data["results"][idx]
    title = _result_title(item)
    snippet = _result_snippet(item)
    data["selected"] = idx
    return _start_article_by_title(data, title, snippet)


def _start_article_by_title(
    data: dict[str, Any], title: str, description: str = ""
) -> list:
    seq = int(data.get("request_seq") or 0) + 1
    tag = f"ui:article:{seq}"
    log.info(f"wikipedia: loading article {title!r} (tag={tag})")
    data["mode"] = "article"
    data["editing_search"] = False
    data["loading"] = True
    data["pending"] = "article"
    data["article_title"] = title
    data["article_description"] = description
    data["article"] = ""
    data["error"] = ""
    data["request_seq"] = seq
    data["active_tag"] = tag
    return [
        SetState(data),
        SetStatus(f"Loading {title}"),
        _fetch_article(title, tag=tag),
    ]


def _fetch_search(query: str, tag: str | None = None) -> HttpFetch:
    params = (
        "action=query"
        "&list=search"
        f"&srsearch={_quote(query)}"
        "&srlimit=10"
        "&utf8=1"
        "&format=json"
    )
    return HttpFetch(
        f"{API}?{params}",
        headers=dict(HEADERS),
        tag=tag,
        request_id=tag,
    )


def _fetch_article(title: str, tag: str | None = None) -> HttpFetch:
    return HttpFetch(
        EXTRACT_API + _quote_title(title),
        headers=dict(HEADERS),
        tag=tag,
        request_id=tag,
    )


def _quote(value: str) -> str:
    """Percent-encode UTF-8 without importing urllib, which stalls in CPython-WASM."""
    safe = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"
    return "".join(
        chr(byte) if byte in safe else f"%{byte:02X}"
        for byte in value.encode("utf-8")
    )


def _quote_title(title: str) -> str:
    """Normalize a Wikipedia title for `/api/rest_v1/page/summary/{title}`.

    Replaces whitespace runs with underscores before percent-encoding reserved
    characters so multi-word titles resolve without Wikimedia redirect/404 bugs.
    """
    normalized = "_".join(title.strip().split())
    return _quote(normalized)


def _strip_html(raw: str) -> str:
    """Remove HTML tags and decode HTML entities into clean single-line text."""
    if not raw:
        return ""
    without_tags = _TAG_RE.sub("", raw)
    for entity, char in _ENTITY_MAP.items():
        if entity in without_tags:
            without_tags = without_tags.replace(entity, char)

    def _replace_numeric(match: re.Match[str]) -> str:
        token = match.group(1)
        try:
            codepoint = int(token[1:], 16) if token.lower().startswith("x") else int(token)
            return chr(codepoint)
        except (ValueError, OverflowError):
            return match.group(0)

    unescaped = _NUMERIC_ENTITY_RE.sub(_replace_numeric, without_tags)
    return " ".join(unescaped.split())


def _handle_tool_call(call: ToolCall) -> list:
    log.info(f"wikipedia: tool call {call.name} from {call.caller_id}")
    if call.name != "wikipedia.search":
        return [ToolResult(call.call_id, error=f"unknown tool '{call.name}'")]
    try:
        payload = json.loads(call.input_json or "{}")
    except ValueError as exc:
        return [ToolResult(call.call_id, error=f"input_json is not valid JSON: {exc}")]
    if not isinstance(payload, dict):
        return [ToolResult(call.call_id, error="input must be a JSON object")]
    query = payload.get("query")
    if not isinstance(query, str) or not query.strip():
        return [ToolResult(call.call_id, error="'query' must be a non-empty string")]
    query = query.strip()

    tag = f"tool:search:{call.call_id}"
    _pending_tools[tag] = {
        "call_id": call.call_id,
        "stage": "search",
        "query": query,
    }
    log.info(f"wikipedia: tool search stage started for {query!r} (tag={tag})")
    return [_fetch_search(query, tag=tag)]


def _handle_tool_http(tag: str, event: HttpResponse) -> list:
    entry = _pending_tools.pop(tag, None)
    if entry is None:
        return []
    call_id = entry["call_id"]

    if event.status < 200 or event.status >= 300:
        log.warn(f"wikipedia: tool request failed status={event.status} tag={tag}")
        return [
            ToolResult(
                call_id,
                error=f"HTTP {event.status}: {_body_text(event)[:240]}",
            )
        ]

    try:
        payload = json.loads(_body_text(event))
    except json.JSONDecodeError as exc:
        log.warn(f"wikipedia: tool response invalid JSON: {exc}")
        return [ToolResult(call_id, error=f"invalid JSON response: {exc}")]

    if entry["stage"] == "search":
        results = _parse_search(payload)
        if not results:
            log.info(f"wikipedia: tool search for {entry['query']!r} -> 0 results")
            return [
                ToolResult(
                    call_id,
                    error="No Wikipedia article found for that query.",
                )
            ]
        title = _result_title(results[0])
        next_tag = f"tool:article:{call_id}"
        _pending_tools[next_tag] = {
            "call_id": call_id,
            "stage": "article",
            "query": entry["query"],
            "title": title,
        }
        log.info(
            f"wikipedia: tool search stage -> article stage for {title!r} (tag={next_tag})"
        )
        return [_fetch_article(title, tag=next_tag)]

    title = str(entry.get("title") or payload.get("title") or "")
    summary = str(payload.get("extract") or "No extract available.")
    log.info(f"wikipedia: tool article stage complete for {title!r}")
    return [
        ToolResult(
            call_id,
            output_json=json.dumps({"title": title, "summary": summary}),
        )
    ]


def _handle_http(data: dict[str, Any], event: HttpResponse) -> dict[str, Any]:
    if event.status < 200 or event.status >= 300:
        detail = _body_text(event).strip()[:240]
        data["loading"] = False
        data["pending"] = ""
        data["active_tag"] = ""
        data["error"] = f"HTTP {event.status}: {detail}" if detail else f"HTTP {event.status}"
        log.warn(f"wikipedia: request failed {data['error']}")
        return data

    if event.error:
        data["loading"] = False
        data["pending"] = ""
        data["active_tag"] = ""
        data["error"] = str(event.error)[:240]
        log.warn(f"wikipedia: transport error {data['error']}")
        return data

    try:
        payload = json.loads(_body_text(event))
    except json.JSONDecodeError as exc:
        data["loading"] = False
        data["pending"] = ""
        data["active_tag"] = ""
        data["error"] = f"Invalid JSON response: {exc}"
        log.warn(f"wikipedia: invalid JSON response: {exc}")
        return data

    resp_tag = _response_tag(event)
    is_search = data["pending"] == "search" or (
        resp_tag is not None and resp_tag.startswith("ui:search:")
    )
    is_article = data["pending"] == "article" or (
        resp_tag is not None and resp_tag.startswith("ui:article:")
    )

    if is_search:
        data["results"] = _parse_search(payload)
        data["selected"] = 0
        log.info(
            f"wikipedia: search {data['searched_query'] or data['query']!r} -> "
            f"{len(data['results'])} results"
        )
    elif is_article and isinstance(payload, dict):
        canonical_title = str(payload.get("title") or "").strip()
        if canonical_title:
            data["article_title"] = canonical_title
        description = _strip_html(str(payload.get("description") or ""))
        if description:
            data["article_description"] = description
        data["article"] = str(payload.get("extract") or "No extract available.")
        log.info(f"wikipedia: article {data['article_title']!r} loaded")

    data["loading"] = False
    data["pending"] = ""
    data["active_tag"] = ""
    data["error"] = ""
    return data


def _parse_search(payload: Any) -> list[dict[str, str]]:
    """Parse MediaWiki `query.search`, REST `pages`, or legacy `opensearch` payloads."""
    if isinstance(payload, dict):
        query_block = payload.get("query")
        if isinstance(query_block, dict):
            search_items = query_block.get("search")
            if isinstance(search_items, list):
                results: list[dict[str, str]] = []
                for item in search_items:
                    if not isinstance(item, dict):
                        continue
                    title = str(item.get("title") or "").strip()
                    if not title:
                        continue
                    snippet = _strip_html(str(item.get("snippet") or ""))
                    results.append({"title": title, "snippet": snippet})
                return results
        pages = payload.get("pages")
        if isinstance(pages, list):
            results = []
            for page in pages:
                if not isinstance(page, dict):
                    continue
                title = str(page.get("title") or page.get("key") or "").strip()
                if not title:
                    continue
                raw_desc = str(page.get("description") or page.get("excerpt") or "")
                results.append({"title": title, "snippet": _strip_html(raw_desc)})
            return results

    if isinstance(payload, list) and len(payload) > 1 and isinstance(payload[1], list):
        titles = payload[1]
        descriptions = (
            payload[2]
            if len(payload) > 2 and isinstance(payload[2], list)
            else []
        )
        results = []
        for idx, raw_title in enumerate(titles):
            title = str(raw_title or "").strip()
            if not title:
                continue
            desc = (
                _strip_html(str(descriptions[idx] or ""))
                if idx < len(descriptions)
                else ""
            )
            results.append({"title": title, "snippet": desc})
        return results

    return []


def _response_tag(event: HttpResponse) -> str | None:
    if event.tag:
        return str(event.tag)
    if event.request_id:
        return str(event.request_id)
    return None


def _body_text(event: HttpResponse) -> str:
    if isinstance(event.body, (bytes, bytearray)):
        return bytes(event.body).decode("utf-8", errors="replace")
    if isinstance(event.body, list):
        return bytes(event.body).decode("utf-8", errors="replace")
    return str(event.body or "")


def _list_subtitle(data: dict[str, Any]) -> str:
    if data["loading"] and data["pending"] == "search":
        target = data["searched_query"] or data["query"].strip()
        return f"Searching “{target}”…" if target else "Searching…"
    if data["error"]:
        return "Request failed"
    if data["results"]:
        count = len(data["results"])
        noun = "result" if count == 1 else "results"
        if data["searched_query"]:
            return f"{count} {noun} for “{data['searched_query']}”"
        return f"{count} {noun}"
    if data["searched_query"]:
        return f"0 results for “{data['searched_query']}”"
    return "The free encyclopedia"


def _article_subtitle(data: dict[str, Any]) -> str:
    if data["loading"] and data["pending"] == "article":
        return "Loading summary…"
    if data["error"]:
        return "Failed to load summary"
    if data["article_description"]:
        return data["article_description"]
    if data["results"]:
        return f"Result {data['selected'] + 1} of {len(data['results'])}"
    return "Wikipedia Summary"


def _status(data: dict[str, Any]) -> str:
    if data["loading"]:
        return "Searching" if data["pending"] == "search" else "Loading"
    if data["error"]:
        return "Error"
    if data["mode"] == "article" and data["article_title"]:
        return data["article_title"]
    if data["results"]:
        return f"{len(data['results'])} results"
    return "Wikipedia"


def _canonical_key(raw_key: str) -> str:
    mapping = {
        "ArrowDown": "down",
        "ArrowUp": "up",
        "Return": "enter",
        "return": "enter",
        "Escape": "escape",
        "BackSpace": "backspace",
    }
    return mapping.get(raw_key, raw_key)


def _parse_row_index(handler_id: str) -> int | None:
    _, _, raw_idx = handler_id.partition(":")
    try:
        return int(raw_idx)
    except ValueError:
        return None


def _clamp(selected: int, total: int) -> int:
    if total <= 0:
        return 0
    return max(0, min(selected, total - 1))
