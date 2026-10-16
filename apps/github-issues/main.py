#!/usr/bin/env python3
"""GitHub Issues — SDK v3 HTTP-backed issue browser."""

from __future__ import annotations

import json
from typing import Any
from urllib.parse import quote

from plexi_sdk import log, state, tools
from plexi_sdk.effects import HttpFetch, SetState, SetStatus, SetTitle
from plexi_sdk.events import HttpResponse, KeyEvent, UiValueChange
from plexi_sdk.ui import (
    AppBar,
    Column,
    Component,
    FooterKeys,
    Scrollable,
    SelectList,
    Spacer,
    Text,
    TextInput,
)

API = "https://api.github.com"
ISSUE_LIST_LIMIT = "500"
SORT_MODES = ("created_desc", "created_asc", "number_desc", "number_asc")
SORT_LABELS = {
    "created_desc": "created ↓",
    "created_asc": "created ↑",
    "number_desc": "number ↓",
    "number_asc": "number ↑",
}

DEFAULT_STATE: dict[str, Any] = {
    "repo": "",
    "issues": [],
    "selected": 0,
    "loading": False,
    "pending": "",
    "error": "",
    "filter": "",
    "sort_mode": "created_desc",
    "view": "list",
    "detail": None,
    "filter_autofocus": True,
}


@tools.tool(
    "issues.list",
    "List loaded GitHub issues filtered by optional `state`, search `query`, and `limit`.",
    {"state": str, "query": str, "limit": int},
    read_only=True,
)
def _tool_list(
    state: str | None = None,
    query: str | None = None,
    limit: int | None = None,
) -> dict:
    data = _state()
    matched = _query_issues(data, state_filter=state, query=query, limit=limit)
    log.info(f"github-issues: tool issues.list returned {len(matched)} issue(s)")
    return {"issues": matched, "count": len(matched)}


@tools.tool(
    "issues.open",
    "Focus the issue row with `number` in the issue list.",
    {"number": int},
    read_only=True,
)
def _tool_open(number: int) -> tools.Reply:
    if isinstance(number, bool) or not isinstance(number, int):
        raise ValueError(f"issue number must be an integer, got {number!r}")
    data = _state()
    visible = _visible_issues(data)
    idx = next(
        (i for i, issue in enumerate(visible) if int(issue.get("number") or 0) == number),
        None,
    )
    if idx is None and data.get("filter"):
        data["filter"] = ""
        visible = _visible_issues(data)
        idx = next(
            (i for i, issue in enumerate(visible) if int(issue.get("number") or 0) == number),
            None,
        )
    if idx is None:
        raise LookupError(f"no issue #{number} in loaded issues")
    issue = visible[idx]
    data["selected"] = idx
    data["view"] = "list"
    data["detail"] = None
    log.info(f"github-issues: focused issue #{number} at row {idx} via assistant")
    return tools.Reply(
        {
            "number": issue["number"],
            "title": issue["title"],
            "selected": idx,
            "issue": issue,
        },
        [SetState(data), SetStatus(_status(data))],
    )


def init(size, args) -> list:
    repo = args[0] if args else state.get("repo", "")
    data = _state()
    if repo:
        data["repo"] = repo
    effects: list = [SetTitle("GitHub Issues"), tools.expose()]
    if data["repo"]:
        data["loading"] = True
        data["pending"] = "list"
        data["error"] = ""
        effects.append(SetState(data))
        effects.append(_fetch_list(data["repo"]))
        effects.append(SetStatus("Loading issues"))
        log.info(f"github-issues: SDK v3 initialized repo={data['repo']}")
    else:
        effects.extend([SetState(data), SetStatus("Pass owner/repo as launch arg")])
        log.info("github-issues: initialized without repo arg")
    return effects


def update(event) -> list:
    handled = tools.dispatch(event)
    if handled is not None:
        return handled
    data = _state()
    if isinstance(event, HttpResponse):
        data.update(_handle_http(data, event))
        return [SetState(data), SetStatus(_status(data))]

    if isinstance(event, UiValueChange) and event.handler_id == "issues-filter":
        data["filter"] = event.value
        data["selected"] = 0
        return [SetState(data), SetStatus(_status(data))]

    if not isinstance(event, KeyEvent) or not event.pressed:
        return []

    key = event.key
    if key == "escape" and data["view"] == "list":
        data["filter_autofocus"] = False
        return [SetState(data), SetStatus(_status(data))]
    if data["view"] == "detail":
        if key == "escape":
            data["view"] = "list"
            data["detail"] = None
            return [SetState(data), SetStatus(_status(data))]
        return []

    visible = _visible_issues(data)
    if key in ("down", "j", "ArrowDown"):
        data["selected"] = _clamp(data["selected"] + 1, len(visible))
    elif key in ("up", "k", "ArrowUp"):
        data["selected"] = _clamp(data["selected"] - 1, len(visible))
    elif key in ("return", "enter") and visible:
        issue = visible[data["selected"]]
        data["view"] = "detail"
        data["detail"] = None
        data["loading"] = True
        data["pending"] = f"detail:{issue['number']}"
        return [
            SetState(data),
            SetStatus(f"Loading #{issue['number']}"),
            _fetch_detail(data["repo"], issue["number"]),
        ]
    elif key == "r" and data["repo"]:
        data["loading"] = True
        data["pending"] = "list"
        data["error"] = ""
        return [
            SetState(data),
            SetStatus("Refreshing issues"),
            _fetch_list(data["repo"]),
        ]
    elif key == "s":
        data["sort_mode"] = _next_sort_mode(data["sort_mode"])
        data["selected"] = _clamp(data["selected"], len(_visible_issues(data)))
    elif key == "c":
        data["filter"] = ""
        data["selected"] = 0
    else:
        return []
    return [SetState(data), SetStatus(_status(data))]


def view():
    data = _state()
    if not data["repo"]:
        return Column(
            [
                AppBar("GitHub Issues", "missing repo"),
                Text("Launch with owner/repo, for example plexiapp/plexi.", size=12.0),
                Spacer(grow=True),
            ],
            grow=True,
        )
    if data["view"] == "detail":
        return _detail_view(data)
    return _list_view(data)


def _state() -> dict:
    data = dict(DEFAULT_STATE)
    for key, value in DEFAULT_STATE.items():
        data[key] = state.get(key, value)
    data["issues"] = [dict(issue) for issue in data.get("issues") or []]
    data["selected"] = max(0, int(data.get("selected") or 0))
    data["filter"] = str(data.get("filter") or "")
    data["sort_mode"] = (
        data.get("sort_mode") if data.get("sort_mode") in SORT_MODES else "created_desc"
    )
    return data


def _fetch_list(repo: str) -> HttpFetch:
    return HttpFetch(
        f"{API}/repos/{quote(repo, safe='/')}/issues?state=open&per_page={ISSUE_LIST_LIMIT}",
        headers=_headers(),
    )


def _fetch_detail(repo: str, number: int) -> HttpFetch:
    return HttpFetch(
        f"{API}/repos/{quote(repo, safe='/')}/issues/{number}", headers=_headers()
    )


def _headers() -> dict:
    return {"Accept": "application/vnd.github+json", "User-Agent": "Plexi"}


def _handle_http(data: dict, event: HttpResponse) -> dict:
    if event.status < 200 or event.status >= 300:
        data["loading"] = False
        data["error"] = f"HTTP {event.status}: {_body_text(event)[:240]}"
        data["pending"] = ""
        log.warn(f"github-issues: request failed {event.status}")
        return data
    try:
        payload = json.loads(_body_text(event))
    except json.JSONDecodeError as exc:
        data["loading"] = False
        data["error"] = str(exc)
        data["pending"] = ""
        return data
    pending = data["pending"]
    if pending == "list":
        data["issues"] = _normalize_issues(payload if isinstance(payload, list) else [])
        data["selected"] = _clamp(data["selected"], len(_visible_issues(data)))
        log.info(f"github-issues: loaded {len(data['issues'])} issues")
    elif pending.startswith("detail:") and isinstance(payload, dict):
        data["detail"] = _normalize_issue(payload)
        log.info(f"github-issues: loaded detail #{data['detail'].get('number')}")
    data["loading"] = False
    data["pending"] = ""
    data["error"] = ""
    return data


def _body_text(event: HttpResponse) -> str:
    body = event.body
    if isinstance(body, bytes):
        return body.decode("utf-8", errors="replace")
    if isinstance(body, list):
        return bytes(body).decode("utf-8", errors="replace")
    return str(body)


def _normalize_issues(raw: list[dict]) -> list[dict]:
    return [_normalize_issue(issue) for issue in raw if "pull_request" not in issue]


def _normalize_issue(issue: dict) -> dict:
    return {
        "number": int(issue.get("number") or 0),
        "title": str(issue.get("title") or ""),
        "state": str(issue.get("state") or "open"),
        "body": str(issue.get("body") or ""),
        "createdAt": str(issue.get("created_at") or issue.get("createdAt") or ""),
        "labels": [
            {"name": str(label.get("name") or "")}
            for label in issue.get("labels") or []
            if isinstance(label, dict)
        ],
        "assignees": [
            {"login": str(assignee.get("login") or "")}
            for assignee in issue.get("assignees") or []
            if isinstance(assignee, dict)
        ],
        "html_url": str(issue.get("html_url") or ""),
    }


def _list_view(data: dict):
    visible = _visible_issues(data)
    rows = [
        {
            "name": f"#{issue['number']} {issue['title']}",
            "description": ", ".join(_issue_labels(issue)),
        }
        for issue in visible
    ]
    body = (
        SelectList(rows, selected_idx=data["selected"])
        if rows
        else Text(_empty_text(data), size=12.0)
    )
    return Column(
        [
            AppBar("GitHub Issues", _list_subtitle(data)),
            TextInput(
                "issues-filter",
                value=data["filter"],
                placeholder="filter labels or title",
                on_change="issues-filter",
                autofocus=bool(data["filter_autofocus"]),
            ),
            body,
            Spacer(grow=True),
            FooterKeys(
                [
                    ("j/k", "select"),
                    ("enter", "detail"),
                    ("s", "sort"),
                    ("r", "refresh"),
                    ("c", "clear"),
                ]
            ),
        ],
        grow=True,
    )


def _detail_view(data: dict):
    detail = data.get("detail")
    body: Component
    if data["loading"]:
        body = Text("Loading issue...", size=12.0)
    elif data["error"]:
        body = Text(data["error"], size=12.0)
    elif not detail:
        body = Text("No detail loaded.", size=12.0)
    else:
        labels = ", ".join(_issue_labels(detail)) or "none"
        assignees = (
            ", ".join(a["login"] for a in detail.get("assignees", [])) or "unassigned"
        )
        body = Scrollable(
            Text(
                f"#{detail['number']} {detail['title']}\n"
                f"state: {detail['state']}\nlabels: {labels}\nassignees: {assignees}\n\n"
                f"{detail.get('body') or 'No body.'}",
                size=12.0,
            )
        )
    return Column(
        [
            AppBar("Issue Detail"),
            body,
            Spacer(grow=True),
            FooterKeys([("esc", "back")]),
        ],
        grow=True,
    )


def _visible_issues(data: dict) -> list[dict]:
    labels = (
        {data["filter"]}
        if data.get("filter") and not _is_text_filter(data["filter"])
        else set()
    )
    visible = _filter_and_sort_issues(data["issues"], labels, data["sort_mode"])
    query = data["filter"].lower().strip()
    if query and _is_text_filter(query):
        visible = [
            issue
            for issue in visible
            if query in issue["title"].lower()
            or query in " ".join(_issue_labels(issue)).lower()
        ]
    return visible


def _query_issues(
    data: dict,
    state_filter: str | None = None,
    query: str | None = None,
    limit: int | None = None,
) -> list[dict]:
    issues = _filter_and_sort_issues(data["issues"], set(), data["sort_mode"])
    if state_filter is not None and str(state_filter).strip():
        norm_state = str(state_filter).strip().lower()
        if norm_state not in ("open", "closed", "all"):
            raise ValueError(
                f"unknown issue state {state_filter!r}; expected open, closed, or all"
            )
        if norm_state != "all":
            issues = [
                issue
                for issue in issues
                if str(issue.get("state") or "open").lower() == norm_state
            ]
    if query is not None and str(query).strip():
        raw_query = str(query).strip()
        needle = raw_query.lower()
        issues = [
            issue
            for issue in issues
            if raw_query in set(_issue_labels(issue))
            or needle in str(issue.get("title") or "").lower()
            or needle in " ".join(_issue_labels(issue)).lower()
            or needle in str(issue.get("body") or "").lower()
        ]
    if limit is not None:
        if isinstance(limit, bool) or not isinstance(limit, int) or limit < 0:
            raise ValueError(f"limit must be a non-negative integer, got {limit!r}")
        issues = issues[:limit]
    return issues


def _is_text_filter(value: str) -> bool:
    return " " in value or ":" not in value


def _list_subtitle(data: dict) -> str:
    return f"{len(_visible_issues(data))} open · {SORT_LABELS[data['sort_mode']]}"


def _empty_text(data: dict) -> str:
    if data["loading"]:
        return "Loading issues..."
    if data["error"]:
        return data["error"]
    return "No matching issues."


def _status(data: dict) -> str:
    if data["loading"]:
        return "Loading"
    if data["error"]:
        return "Error"
    return f"{len(_visible_issues(data))} open issues"


def _clamp(selected: int, total: int) -> int:
    if total <= 0:
        return 0
    return max(0, min(selected, total - 1))


def _issue_labels(issue: dict) -> list[str]:
    return [
        str(label.get("name", ""))
        for label in (issue.get("labels") or [])
        if isinstance(label, dict) and label.get("name")
    ]


def _next_sort_mode(mode: str) -> str:
    try:
        idx = SORT_MODES.index(mode)
    except ValueError:
        idx = 0
    return SORT_MODES[(idx + 1) % len(SORT_MODES)]


def _sort_key(issue: dict, mode: str) -> tuple:
    if mode.startswith("number"):
        return (int(issue.get("number") or 0),)
    return (str(issue.get("createdAt") or ""), int(issue.get("number") or 0))


def _filter_and_sort_issues(
    issues: list[dict], filter_labels: set[str], sort_mode: str
) -> list[dict]:
    visible = list(issues)
    if filter_labels:
        visible = [
            issue for issue in visible if filter_labels <= set(_issue_labels(issue))
        ]
    reverse = sort_mode in ("created_desc", "number_desc")
    return sorted(
        visible, key=lambda issue: _sort_key(issue, sort_mode), reverse=reverse
    )
