from __future__ import annotations
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
  Esc      Back to list (Backspace also works)
"""

import json
import os
import queue
import re
import subprocess
import sys
import threading
import time
import urllib.request
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import (
    App,
    THEME,
    TITLE, BODY, CAPTION, HINT, MONO_BODY,
    PAD, HEADER_H,
)


TOP_N = 30

# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------

VIEW_LIST = "list"
VIEW_DETAIL = "detail"
VIEW_LOAD = "loading"

view: str = VIEW_LOAD
stories: list = []
selected: int = 0
detail: dict | None = None
detail_scroll: int = 0
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


def _render_row(ctx, story, i, x, y, w, is_sel):
    row_h = 52.0
    bg = THEME.highlight if is_sel else THEME.bg
    ctx.rect(x + PAD / 2, y, w - PAD, row_h, fill=bg, radius=6)

    rank = f"#{i + 1}"
    ctx.text(
        x + PAD, y + 6,
        rank,
        size=CAPTION,
        color=THEME.accent if is_sel else THEME.muted,
        bold=True,
    )

    title = story.get("title", "")
    ctx.text(
        x + PAD + 44, y + 6,
        title[:120],
        size=BODY,
        color=THEME.fg if is_sel else THEME.muted,
        bold=is_sel,
    )

    score = story.get("score", 0)
    comments = story.get("descendants", 0)
    domain = _domain(story.get("url"))
    secondary = f"▲ {score}  💬 {comments}  {domain}"
    ctx.text(
        x + PAD + 44, y + 6 + BODY + 4,
        secondary[:140],
        size=CAPTION,
        color=THEME.muted,
    )


@app.on_render
def render(ctx):
    global view, stories, selected, detail, load_msg, detail_scroll

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

    ctx.rect(0, 0, ctx.width, ctx.height, fill=THEME.bg)

    if view == VIEW_LOAD:
        ctx.header("Hacker News")
        ctx.empty_state(load_msg)
        ctx.status_bar([("⌘W", "close")])
        return

    if view == VIEW_DETAIL and detail:
        _render_detail(ctx)
        return

    _render_list(ctx)


def _render_list(ctx):
    ctx.header(f"Hacker News  ·  {len(stories)} stories")

    if not stories:
        ctx.empty_state("No stories")
        ctx.status_bar([("⌘W", "close")])
        return

    ctx.scrollable_list(
        list_id="stories",
        items=stories,
        selected=selected,
        row_height=56.0,
        render_row=_render_row,
    )

    ctx.status_bar(
        [
            ("j/k", "navigate"),
            ("Enter", "open"),
            ("⌘W", "close"),
        ],
    )


def _render_detail(ctx):
    global detail_scroll
    assert detail is not None

    title = detail.get("title", "")
    ctx.header(title[:80])

    # Build body lines: meta + URL + self-text
    score = detail.get("score", 0)
    author = detail.get("by", "")
    comments = detail.get("descendants", 0)
    ts = detail.get("time")
    url = detail.get("url") or ""

    meta_lines: list[str] = []
    meta_lines.append(f"▲ {score}  by {author}  💬 {comments}  {_time_ago(ts)}")
    if url:
        meta_lines.append(_domain(url))
        meta_lines.append(url)
    meta_lines.append("")

    text_body = detail.get("text") or ""
    if text_body:
        clean = re.sub(r"<[^>]+>", " ", text_body).strip()
        wrapped = ctx.wrap_text(
            clean,
            max_width_px=ctx.width - PAD * 2,
            size=BODY,
        )
        body_lines = meta_lines + wrapped
    else:
        body_lines = meta_lines + ["Press o to open in browser."]

    detail_scroll = ctx.scrollable_text(
        text_id="hn_detail",
        lines=body_lines,
        scroll_offset=detail_scroll,
        line_height=BODY + 6,
        size=BODY,
    )

    ctx.status_bar(
        [
            ("j/k", "scroll"),
            ("o", "open URL"),
            ("Esc", "back"),
            ("⌘W", "close"),
        ],
    )


@app.on_key
def on_key(key: str, _mods: dict, _emit):
    global view, selected, detail, detail_scroll

    if view == VIEW_LOAD:
        return

    if view == VIEW_LIST:
        if key in ("j", "ArrowDown"):
            selected = min(selected + 1, max(len(stories) - 1, 0))
        elif key in ("k", "ArrowUp"):
            selected = max(selected - 1, 0)
        elif key == "Enter" and stories:
            detail = stories[selected]
            detail_scroll = 0
            view = VIEW_DETAIL
        return

    if view == VIEW_DETAIL:
        if key in ("j", "ArrowDown"):
            detail_scroll += 1
        elif key in ("k", "ArrowUp"):
            detail_scroll = max(0, detail_scroll - 1)
        elif key in ("Escape", "Backspace", "Enter"):
            view = VIEW_LIST
            detail = None
            detail_scroll = 0
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
