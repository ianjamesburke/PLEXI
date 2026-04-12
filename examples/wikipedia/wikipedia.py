#!/usr/bin/env python3
"""
wikipedia — Plexi app
Browse Wikipedia: search articles and read summaries.

Controls (Search view):
  Type         Build search query
  Backspace    Remove last character
  Enter        Search / open selected result
  j / k        Move selection through results

Controls (Article view):
  j / k        Scroll body text
  Backspace    Back to search
"""

import json
import queue
import re
import sys
import os
import textwrap
import threading
import urllib.request
import urllib.parse
import urllib.error

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------

VIEW_SEARCH  = "search"
VIEW_ARTICLE = "article"
VIEW_LOADING = "loading"

view          = VIEW_SEARCH
query         = ""
results       = []          # list of {"title": str, "description": str}
selected      = 0
article       = None        # {"title": str, "description": str, "extract": str}
article_scroll = 0          # first visible wrapped line index
loading_msg   = ""

# Queue from background thread → render thread
result_queue: queue.Queue = queue.Queue()

# ---------------------------------------------------------------------------
# Colors — Catppuccin Mocha
# ---------------------------------------------------------------------------

C = {
    "bg":      "#1e1e2e",
    "surface": "#313244",
    "text":    "#cdd6f4",
    "subtext": "#6c7086",
    "accent":  "#89b4fa",
    "green":   "#a6e3a1",
    "header":  "#181825",
}

HEADER_H  = 52   # height of the top bar / search bar area
PADDING   = 16
ITEM_H    = 44

# ---------------------------------------------------------------------------
# Wikipedia API helpers (run on background thread)
# ---------------------------------------------------------------------------

SUMMARY_BASE = "https://en.wikipedia.org/api/rest_v1"
SEARCH_BASE  = "https://en.wikipedia.org/w/api.php"
HEADERS = {"User-Agent": "Plexi/0.1 (https://github.com/ianjamesburke/PLEXI)"}

_HTML_TAG = re.compile(r"<[^>]+>")


def _strip_html(s: str) -> str:
    return _HTML_TAG.sub("", s)


def _get(url: str) -> dict:
    req = urllib.request.Request(url, headers=HEADERS)
    with urllib.request.urlopen(req, timeout=8) as resp:
        return json.loads(resp.read().decode())


def fetch_search(q: str):
    """Fetch search results via MediaWiki action API and push to queue."""
    try:
        params = urllib.parse.urlencode({
            "action": "query", "list": "search",
            "srsearch": q, "srlimit": 10, "format": "json",
        })
        data = _get(f"{SEARCH_BASE}?{params}")
        results = data.get("query", {}).get("search", [])
        items = [
            {"title": r["title"], "description": _strip_html(r.get("snippet", ""))}
            for r in results
        ]
        result_queue.put({"action": "search_done", "items": items})
    except Exception as e:
        result_queue.put({"action": "error", "message": str(e)})


def fetch_article(title: str):
    """Fetch article summary and push to queue."""
    try:
        encoded = urllib.parse.quote(title)
        data = _get(f"{SUMMARY_BASE}/page/summary/{encoded}")
        result_queue.put({
            "action": "article_done",
            "title":       data.get("title", title),
            "description": data.get("description", "") or "",
            "extract":     data.get("extract", "") or "",
        })
    except Exception as e:
        result_queue.put({"action": "error", "message": str(e)})


def start_fetch(fn, *args):
    t = threading.Thread(target=fn, args=args, daemon=True)
    t.start()

# ---------------------------------------------------------------------------
# Text wrapping helper
# ---------------------------------------------------------------------------

def wrap_text(text: str, width: int) -> list[str]:
    """Wrap text to width characters, preserving paragraph breaks."""
    lines = []
    for para in text.split("\n"):
        if para.strip() == "":
            lines.append("")
        else:
            lines.extend(textwrap.wrap(para, width) or [""])
    return lines

# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

app = App()


@app.on_render
def render(ctx):
    global view, results, selected, article, article_scroll, loading_msg

    # Drain the result queue before drawing
    try:
        while True:
            msg = result_queue.get_nowait()
            action = msg["action"]
            if action == "search_done":
                results = msg["items"]
                selected = 0
                loading_msg = ""
                view = VIEW_SEARCH
            elif action == "article_done":
                article = {
                    "title":       msg["title"],
                    "description": msg["description"],
                    "extract":     msg["extract"],
                }
                article_scroll = 0
                view = VIEW_ARTICLE
            elif action == "error":
                loading_msg = f"Error: {msg['message']}"
                view = VIEW_SEARCH
    except queue.Empty:
        pass

    # Background
    ctx.rect(0, 0, ctx.width, ctx.height, fill=C["bg"])

    if view == VIEW_LOADING:
        _render_loading(ctx)
    elif view == VIEW_ARTICLE:
        _render_article(ctx)
    else:
        _render_search(ctx)


def _render_search(ctx):
    w = ctx.width

    # Header bar
    ctx.rect(0, 0, w, HEADER_H, fill=C["header"])
    ctx.text(PADDING, 10, "Wikipedia", size=13, color=C["accent"], bold=True)

    # Search box — draw inline input
    search_label = f"Search: {query}_"
    ctx.text(PADDING, 30, search_label, size=13, color=C["text"])

    hint = "Enter=search  j/k=select  Enter=open"
    hint_x = w - len(hint) * 7.2 - PADDING
    if hint_x > 0:
        ctx.text(hint_x, 10, hint, size=11, color=C["subtext"])

    # Divider
    ctx.line(0, HEADER_H, w, HEADER_H, color=C["surface"], width=1.0)


    if not results:
        if not loading_msg:
            ctx.text(PADDING, HEADER_H + 16, "Type a query and press Enter to search.",
                     size=12, color=C["subtext"])
        return

    # Results list
    list_items = [
        {"label": r["title"], "secondary": r["description"] or None}
        for r in results
    ]
    # Render list below header — use ctx.list which handles scroll/selection
    # We need to clip the list area; use a rect clip by drawing header on top after
    ctx.list(list_items, selected=selected, item_height=float(ITEM_H))

    # Re-paint header on top so it covers any list overlap
    ctx.rect(0, 0, w, HEADER_H, fill=C["header"])
    ctx.text(PADDING, 10, "Wikipedia", size=13, color=C["accent"], bold=True)
    ctx.text(PADDING, 30, search_label, size=13, color=C["text"])
    if hint_x > 0:
        ctx.text(hint_x, 10, hint, size=11, color=C["subtext"])
    ctx.line(0, HEADER_H, w, HEADER_H, color=C["surface"], width=1.0)


def _render_article(ctx):
    if not article:
        return

    w = ctx.width
    h = ctx.height

    # Title bar
    ctx.rect(0, 0, w, HEADER_H, fill=C["header"])
    title = article["title"]
    ctx.text(PADDING, 10, title, size=18, color=C["text"], bold=True)
    hint = "j/k=scroll  Backspace=back"
    hint_x = w - len(hint) * 7.2 - PADDING
    if hint_x > 0:
        ctx.text(hint_x, 16, hint, size=11, color=C["subtext"])

    # Description
    desc_y = HEADER_H + 10
    if article["description"]:
        ctx.text(PADDING, desc_y, article["description"], size=13, color=C["subtext"])
        desc_y += 22

    ctx.line(0, desc_y + 4, w, desc_y + 4, color=C["surface"], width=1.0)
    body_start_y = desc_y + 14

    # Wrap extract to fit width
    char_width = 80
    lines = wrap_text(article["extract"], char_width)

    line_h = 18
    visible_lines = max(1, int((h - body_start_y - PADDING) / line_h))

    # Clamp scroll
    global article_scroll
    max_scroll = max(0, len(lines) - visible_lines)
    article_scroll = max(0, min(article_scroll, max_scroll))

    for i, line in enumerate(lines[article_scroll:article_scroll + visible_lines]):
        y = body_start_y + i * line_h
        ctx.text(PADDING, y, line, size=13, color=C["text"])

    # Scroll indicator
    if len(lines) > visible_lines:
        pct = article_scroll / max(1, max_scroll)
        indicator = f"{int(pct * 100)}%"
        ctx.text(w - PADDING - len(indicator) * 7, h - PADDING, indicator,
                 size=11, color=C["subtext"])


def _render_loading(ctx):
    w = ctx.width
    h = ctx.height
    ctx.rect(0, 0, w, HEADER_H, fill=C["header"])
    ctx.text(PADDING, 16, "Wikipedia", size=13, color=C["accent"], bold=True)
    ctx.text(w / 2 - 60, h / 2 - 8, loading_msg, size=14, color=C["subtext"])


@app.on_key
def on_key(key, _mods, _emit):
    global query, view, selected, article_scroll, loading_msg

    if view == VIEW_LOADING:
        return  # ignore input while loading

    if view == VIEW_SEARCH:
        if key == "Backspace":
            query = query[:-1]
        elif key == "Enter":
            if results:
                # Results visible — open the selected one
                _open_selected()
            elif query:
                # No results yet — trigger a search
                loading_msg = "Searching\u2026"
                view = VIEW_LOADING
                start_fetch(fetch_search, query)
        elif key == "ArrowDown" or (key == "j" and results):
            selected = min(selected + 1, len(results) - 1)
        elif key == "ArrowUp" or (key == "k" and results):
            selected = max(selected - 1, 0)
        elif len(key) == 1 and key.isprintable():
            query += key
            # Typing new chars clears stale results so Enter triggers a fresh search
            results.clear()
            selected = 0
            loading_msg = ""

    elif view == VIEW_ARTICLE:
        if key == "Backspace":
            view = VIEW_SEARCH
        elif key in ("j", "ArrowDown"):
            article_scroll += 1
        elif key in ("k", "ArrowUp"):
            article_scroll = max(0, article_scroll - 1)


def _open_selected():
    global view, loading_msg
    if not results or selected < 0 or selected >= len(results):
        return
    title = results[selected]["title"]
    view = VIEW_LOADING
    loading_msg = "Loading article\u2026"
    start_fetch(fetch_article, title)


app.run()
