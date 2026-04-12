#!/usr/bin/env python3
"""
hacker-news — Plexi app
Browse Hacker News top stories.

Controls (list view):
  j / ↓    Next story
  k / ↑    Previous story
  Enter    Open detail

Controls (detail view):
  o        Open URL in browser
  Escape/q Back to list
"""
from __future__ import annotations

import json
import math
import os
import queue
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Catppuccin Mocha
# ---------------------------------------------------------------------------

C = {
    "bg":      "#1e1e2e",
    "surface": "#313244",
    "overlay": "#45475a",
    "text":    "#cdd6f4",
    "subtext": "#6c7086",
    "accent":  "#89b4fa",
    "green":   "#a6e3a1",
    "yellow":  "#f9e2af",
    "red":     "#f38ba8",
    "orange":  "#fab387",
    "header":  "#181825",
}

PADDING  = 16
HEADER_H = 48
ITEM_H   = 52
TOP_N    = 30

# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------

VIEW_LIST   = "list"
VIEW_DETAIL = "detail"
VIEW_LOAD   = "loading"

view: str     = VIEW_LOAD
stories: list = []
selected: int = 0
detail: dict | None  = None
load_msg: str = "Loading top stories…"

result_q: queue.Queue = queue.Queue()

# ---------------------------------------------------------------------------
# HN API helpers
# ---------------------------------------------------------------------------

BASE = "https://hacker-news.firebaseio.com/v0"
HEADERS = {"User-Agent": "Plexi/0.1 (https://github.com/ianjamesburke/PLEXI)"}


def _get(url: str) -> object:
    req = urllib.request.Request(url, headers=HEADERS)
    with urllib.request.urlopen(req, timeout=8) as r:
        return json.loads(r.read().decode())


def _fetch_story(sid: int) -> dict | None:
    try:
        return _get(f"{BASE}/item/{sid}.json")  # type: ignore[return-value]
    except Exception:
        return None


def _domain(url: str | None) -> str:
    if not url:
        return "self"
    try:
        host = url.split("//", 1)[1].split("/")[0]
        return host.removeprefix("www.")
    except Exception:
        return ""


def _time_ago(ts: int | None) -> str:
    if not ts:
        return ""
    delta = int(time.time()) - ts
    if delta < 60:
        return "just now"
    if delta < 3600:
        return f"{delta // 60}m ago"
    if delta < 86400:
        return f"{delta // 3600}h ago"
    return f"{delta // 86400}d ago"


def fetch_top_stories():
    try:
        ids = _get(f"{BASE}/topstories.json")
        top_ids = ids[:TOP_N]  # type: ignore[index]
        with ThreadPoolExecutor(max_workers=5) as pool:
            items = list(pool.map(_fetch_story, top_ids))
        stories_out = [s for s in items if s and s.get("type") == "story"]
        result_q.put({"action": "list_done", "stories": stories_out})
    except Exception as exc:
        result_q.put({"action": "error", "message": str(exc)})


def start_fetch_list():
    global view, load_msg
    view = VIEW_LOAD
    load_msg = "Loading top stories…"
    t = threading.Thread(target=fetch_top_stories, daemon=True)
    t.start()

# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

app = App(app_id="hacker-news")


@app.on_render
def render(ctx):
    global view, stories, selected, detail, load_msg

    # Drain queue
    try:
        while True:
            msg = result_q.get_nowait()
            action = msg["action"]
            if action == "list_done":
                stories = msg["stories"]
                selected = 0
                view = VIEW_LIST
            elif action == "error":
                load_msg = f"Error: {msg['message']}"
                view = VIEW_LOAD
    except queue.Empty:
        pass

    w = ctx.width
    h = ctx.height

    ctx.rect(0, 0, w, h, fill=C["bg"])

    # Header
    ctx.rect(0, 0, w, HEADER_H, fill=C["header"])
    ctx.text(PADDING, 14, "Hacker News", size=14, color=C["orange"], bold=True)

    if view == VIEW_LOAD:
        _render_loading(ctx, w, h)
    elif view == VIEW_DETAIL:
        _render_detail(ctx, w, h)
    else:
        _render_list(ctx, w, h)


def _render_loading(ctx, w: float, h: float):
    ctx.text(PADDING, HEADER_H + 20, load_msg, size=13, color=C["subtext"])


def _render_list(ctx, w: float, h: float):
    hint = "j/k=nav  Enter=open"
    ctx.text(w - len(hint) * 7.2 - PADDING, 16, hint, size=11, color=C["subtext"])
    ctx.line(0, HEADER_H, w, HEADER_H, color=C["surface"], width=1.0)

    items = []
    for i, s in enumerate(stories):
        rank = f"#{i+1}"
        title = s.get("title", "")
        score = s.get("score", 0)
        comments = s.get("descendants", 0)
        domain = _domain(s.get("url"))
        label = f"{rank}  {title}"
        secondary = f"▲ {score}  💬 {comments}  {domain}"
        items.append({"label": label, "secondary": secondary})

    ctx.list(items, selected=selected, item_height=float(ITEM_H))

    # Re-paint header
    ctx.rect(0, 0, w, HEADER_H, fill=C["header"])
    ctx.text(PADDING, 14, "Hacker News", size=14, color=C["orange"], bold=True)
    ctx.text(w - len(hint) * 7.2 - PADDING, 16, hint, size=11, color=C["subtext"])
    ctx.line(0, HEADER_H, w, HEADER_H, color=C["surface"], width=1.0)


def _render_detail(ctx, w: float, h: float):
    if not detail:
        return

    hint = "o=open URL  Esc/q=back"
    ctx.text(w - len(hint) * 7.2 - PADDING, 16, hint, size=11, color=C["subtext"])
    ctx.line(0, HEADER_H, w, HEADER_H, color=C["surface"], width=1.0)

    y = HEADER_H + PADDING

    # Title
    title = detail.get("title", "")
    ctx.text(PADDING, y, title, size=16, color=C["text"], bold=True)
    y += 28

    # URL / domain
    url = detail.get("url") or ""
    if url:
        ctx.text(PADDING, y, _domain(url), size=12, color=C["accent"])
        y += 20

    # Meta row
    score    = detail.get("score", 0)
    author   = detail.get("by", "")
    comments = detail.get("descendants", 0)
    ts       = detail.get("time")
    meta = f"▲ {score}  by {author}  💬 {comments}  {_time_ago(ts)}"
    ctx.text(PADDING, y, meta, size=12, color=C["subtext"])
    y += 28

    ctx.line(0, y, w, y, color=C["surface"], width=1.0)
    y += 12

    # Self-post text if present
    text_body = detail.get("text") or ""
    if text_body:
        # Strip HTML tags simply
        import re
        clean = re.sub(r"<[^>]+>", " ", text_body).strip()
        import textwrap
        lines = textwrap.wrap(clean, 80)
        for line in lines[:30]:
            ctx.text(PADDING, y, line, size=12, color=C["text"])
            y += 18
    else:
        ctx.text(PADDING, y, "Press o to open in browser.", size=12, color=C["subtext"])


@app.on_key
def on_key(key: str, _mods: dict, _emit):
    global view, selected, detail

    if view == VIEW_LOAD:
        return

    if view == VIEW_LIST:
        if key in ("j", "ArrowDown"):
            selected = min(selected + 1, len(stories) - 1)
        elif key in ("k", "ArrowUp"):
            selected = max(selected - 1, 0)
        elif key == "Enter" and stories:
            detail = stories[selected]
            view = VIEW_DETAIL

    elif view == VIEW_DETAIL:
        if key in ("Escape", "q"):
            view = VIEW_LIST
            detail = None
        elif key == "o":
            url = detail.get("url") if detail else None
            if url:
                try:
                    subprocess.Popen(["open", url])
                except Exception:
                    pass


# Kick off initial load
start_fetch_list()

app.run()
