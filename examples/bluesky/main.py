#!/usr/bin/env python3
"""bluesky — read-only Bluesky feed browser via the public AT Protocol API.

No login required. Uses public.api.bsky.app XRPC endpoints.

Views:
  FEED    — Discover / What's Hot feed (or author feed after profile lookup)
  THREAD  — Full thread for the selected post

Keys (FEED): j/k nav · Enter thread · p profile · r refresh · o browser · n/N page
Keys (THREAD): Esc back · o browser
"""

import asyncio
import json
import urllib.parse
import webbrowser

from plexi_sdk import (
    App, RenderContext, CapabilityDeniedError,
    BG, SURFACE, FG, ACCENT, MUTED, HIGHLIGHT,
    BODY, CAPTION, HEADING, HINT, HEADER_H,
    RED,
    PAD, PAD_TIGHT,
)

BASE     = "https://public.api.bsky.app/xrpc"
DISCOVER = "at://did:plc:z72i7hdynmk6r22z27h6tvur/app.bsky.feed.generator/whats-hot"
LIMIT    = 30

AVATAR_R  = 16.0   # avatar circle radius
TEXT_X    = PAD + AVATAR_R * 2 + 8.0   # left edge of text column
ROW_BASE  = 48.0   # row height with no extra text lines
LINE_H    = CAPTION + 4.0   # height per extra wrapped line
MAX_LINES = 3      # max post body lines in feed

THREAD_H  = 88.0   # thread row base height
IMG_H     = 80.0   # height per image in thread view


# ── helpers ───────────────────────────────────────────────────────────────────

def _short_ts(ts: str) -> str:
    try:
        return ts[5:10].replace("-", "/")
    except Exception:
        return ""


def _author(post: dict) -> str:
    a    = post.get("author") or {}
    name = a.get("displayName") or ""
    hand = a.get("handle") or "?"
    return f"{name} @{hand}" if name and name != hand else f"@{hand}"


def _avatar_url(post: dict) -> str:
    return (post.get("author") or {}).get("avatar") or ""


def _did(post: dict) -> str:
    return (post.get("author") or {}).get("did") or ""


def _text(post: dict) -> str:
    return (post.get("record") or {}).get("text", "")


def _thumbs(post: dict) -> list[str]:
    e = post.get("embed") or {}
    t = e.get("$type", "")
    if "images#view" in t:
        return [i["thumb"] for i in e.get("images", []) if i.get("thumb")]
    media = e.get("media") or {}
    if "images#view" in media.get("$type", ""):
        return [i["thumb"] for i in media.get("images", []) if i.get("thumb")]
    return []


def _at_web(uri: str) -> str:
    if not uri.startswith("at://"):
        return ""
    try:
        parts = uri[5:].split("/")
        if len(parts) >= 3:
            return f"https://bsky.app/profile/{parts[0]}/post/{parts[2]}"
    except Exception:
        pass
    return ""


# ── app ───────────────────────────────────────────────────────────────────────

class BlueskyApp(App):
    VIEW_FEED   = "feed"
    VIEW_THREAD = "thread"

    async def on_init(self, ctx: RenderContext) -> None:
        self._view   = self.VIEW_FEED
        self._feed   : list[dict] = []
        self._sel    = 0
        self._scroll  = 0.0
        self._loading = True
        self._error   : str | None = None

        self._cursors   : list[str | None] = [None]
        self._page_idx  = 0
        self._next_cur  : str | None = None

        self._feed_label    = "Discover"
        self._author_handle : str | None = None
        self._show_input    = False

        self._thread  : list[dict] = []
        self._t_scroll = 0.0

        # per-post measured row heights (pre-computed at fetch time)
        self._row_heights: list[float] = []
        # avatar handles keyed by author did
        self._avatar_handles: dict[str, str] = {}

        self.emit.info("bluesky: init")
        ctx.status_summary("Loading Discover feed…")
        try:
            await self.emit.capability_request("net.http")
        except CapabilityDeniedError:
            self._loading = False
            self._error = "Network access denied."
            self.emit.warn("bluesky: net.http capability denied")
            return
        asyncio.create_task(self._fetch_discover(None))

    # ── data ──────────────────────────────────────────────────────────────────

    async def _fetch_discover(self, cursor: str | None) -> None:
        self._loading = True
        self._error   = None
        self.emit.schedule_render()
        url = (f"{BASE}/app.bsky.feed.getFeed"
               f"?feed={urllib.parse.quote(DISCOVER, safe='')}&limit={LIMIT}")
        if cursor:
            url += f"&cursor={urllib.parse.quote(cursor)}"
        try:
            data = json.loads(await self.emit.http_get(url)) or {}
            if data.get("error"):
                raise RuntimeError(data["error"])
            self._feed     = [item["post"] for item in data.get("feed", []) if "post" in item]
            self._next_cur = data.get("cursor")
            self._sel      = 0
            self.emit.info(f"bluesky: discover loaded {len(self._feed)} posts")
            await self._post_fetch_setup()
        except Exception as exc:
            self.emit.warn(f"bluesky: discover error: {exc}")
            self._error = str(exc)
        self._loading = False
        self.emit.schedule_render()

    async def _fetch_author(self, handle: str, cursor: str | None) -> None:
        self._loading = True
        self._error   = None
        self.emit.schedule_render()
        url = (f"{BASE}/app.bsky.feed.getAuthorFeed"
               f"?actor={urllib.parse.quote(handle)}&limit={LIMIT}")
        if cursor:
            url += f"&cursor={urllib.parse.quote(cursor)}"
        try:
            data  = json.loads(await self.emit.http_get(url)) or {}
            if data.get("error"):
                raise RuntimeError(data["error"])
            posts = [item["post"] for item in data.get("feed", []) if "post" in item]
            self._feed     = [p for p in posts if not (p.get("record") or {}).get("reply")]
            self._next_cur = data.get("cursor")
            self._sel      = 0
            self.emit.info(f"bluesky: author @{handle} loaded {len(self._feed)} posts")
            await self._post_fetch_setup()
        except Exception as exc:
            self.emit.warn(f"bluesky: author feed error: {exc}")
            self._error = str(exc)
        self._loading = False
        self.emit.schedule_render()

    async def _post_fetch_setup(self) -> None:
        """Pre-measure row heights and load avatars after any feed fetch."""
        pane_w = 700.0  # conservative default; real width used for render
        text_w = pane_w - TEXT_X - PAD

        heights = []
        for post in self._feed:
            body = _text(post)
            if body:
                try:
                    h = await self.emit.measure_text_wrapped(
                        body, CAPTION, text_w, max_lines=MAX_LINES
                    )
                    heights.append(ROW_BASE + max(0.0, h - CAPTION))
                except Exception as exc:
                    self.emit.warn(f"bluesky: measure_text_wrapped failed: {exc}")
                    heights.append(ROW_BASE + LINE_H * MAX_LINES)
            else:
                heights.append(ROW_BASE)
        self._row_heights = heights

        # Load avatars for unique authors
        for post in self._feed:
            did = _did(post)
            url = _avatar_url(post)
            if did and url and did not in self._avatar_handles:
                try:
                    handle = await self.emit.load_image(url)
                    self._avatar_handles[did] = handle
                except Exception as exc:
                    self.emit.warn(f"bluesky: avatar load failed for {did}: {exc}")
                    # placeholder circle renders fine

    async def _fetch_thread(self, uri: str) -> None:
        self._loading = True
        self._error   = None
        self.emit.schedule_render()
        url = f"{BASE}/app.bsky.feed.getPostThread?uri={urllib.parse.quote(uri)}&depth=6"
        try:
            data  = json.loads(await self.emit.http_get(url)) or {}
            if data.get("error"):
                raise RuntimeError(data["error"])
            posts : list[dict] = []
            self._walk(data.get("thread", {}), posts, depth=0)
            self._thread   = posts
            self._t_scroll = 0.0
            self.emit.info(f"bluesky: thread loaded {len(posts)} nodes uri={uri!r}")
        except Exception as exc:
            self.emit.warn(f"bluesky: thread error: {exc}")
            self._error = str(exc)
        self._loading = False
        self.emit.schedule_render()

    def _walk(self, node: dict, out: list, depth: int) -> None:
        if not node or node.get("$type") != "app.bsky.feed.defs#threadViewPost":
            return
        if node.get("post"):
            out.append({"post": node["post"], "depth": depth})
        for reply in node.get("replies", [])[:5]:
            self._walk(reply, out, depth + 1)

    # ── pagination ────────────────────────────────────────────────────────────

    def _page_fetch(self, cursor: str | None) -> None:
        if self._author_handle:
            asyncio.create_task(self._fetch_author(self._author_handle, cursor))
        else:
            asyncio.create_task(self._fetch_discover(cursor))

    def _next_page(self) -> None:
        if not self._next_cur:
            return
        self._page_idx += 1
        if self._page_idx >= len(self._cursors):
            self._cursors.append(self._next_cur)
        self.emit.info(f"bluesky: next page idx={self._page_idx}")
        self._page_fetch(self._cursors[self._page_idx])

    def _prev_page(self) -> None:
        if self._page_idx == 0:
            return
        self._page_idx -= 1
        self.emit.info(f"bluesky: prev page idx={self._page_idx}")
        self._page_fetch(self._cursors[self._page_idx])

    # ── render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)
        self._draw_header(ctx)

        if self._loading:
            self._draw_loading_skeleton(ctx)
            return

        if self._error:
            ctx.text(PAD, HEADER_H + PAD, f"Error: {self._error}",
                     size=CAPTION, color=RED, max_width=ctx.w - PAD * 2)
            ctx.text(PAD, HEADER_H + PAD + BODY + PAD_TIGHT, "r — retry",
                     size=HINT, color=MUTED)
            return

        if self._view == self.VIEW_THREAD:
            self._draw_thread(ctx)
        else:
            self._draw_feed(ctx)
            if self._show_input:
                self._draw_input_overlay(ctx)

    def _draw_loading_skeleton(self, ctx: RenderContext) -> None:
        """Render animated skeleton rows instead of plain 'Loading…'."""
        y = HEADER_H + PAD
        for _ in range(8):
            row_h = ROW_BASE + LINE_H * 2
            # avatar circle placeholder
            ctx.skeleton(PAD, y + (row_h - AVATAR_R * 2) / 2,
                         AVATAR_R * 2, AVATAR_R * 2, radius=AVATAR_R)
            # author line
            ctx.skeleton(TEXT_X, y + 6, ctx.w * 0.35, HINT, radius=3.0)
            # text lines
            ctx.skeleton(TEXT_X, y + 6 + HINT + 6, ctx.w * 0.85 - TEXT_X, CAPTION, radius=3.0)
            ctx.skeleton(TEXT_X, y + 6 + HINT + 6 + LINE_H, ctx.w * 0.60 - TEXT_X, CAPTION, radius=3.0)
            y += row_h + 1

    def _draw_header(self, ctx: RenderContext) -> None:
        ctx.rect(0, 0, ctx.w, HEADER_H, fill=SURFACE)
        ctx.rect(0, HEADER_H - 1, ctx.w, 1, fill=BG)
        mid_y = HEADER_H / 2

        label = ("Thread" if self._view == self.VIEW_THREAD
                 else self._feed_label)
        ctx.text(PAD, mid_y - HEADING / 2, label,
                 size=HEADING, color=ACCENT, bold=True, monospace=True)

        if self._view == self.VIEW_FEED and self._page_idx > 0:
            ctx.badge(x=PAD + len(label) * 9 + 10, y_center=mid_y,
                      label=f"p{self._page_idx + 1}", fill=SURFACE, fg=MUTED,
                      font_size=HINT, radius=3.0)

        pairs = (
            [("esc", "back"), ("o", "browser")]
            if self._view == self.VIEW_THREAD else
            [(["j", "k"], "nav"), ("↩", "thread"),
             ("p", "profile"), ("r", "refresh"),
             ("o", "browser"), (["n", "N"], "page")]
        )
        ctx.shortcuts(x=ctx.w / 3, y=mid_y - HINT / 2,
                      max_width=ctx.w * 2 / 3 - PAD, pairs=pairs)

    def _row_height(self, i: int) -> float:
        if i < len(self._row_heights):
            return self._row_heights[i]
        return ROW_BASE + LINE_H * 2

    def _draw_feed(self, ctx: RenderContext) -> None:
        if not self._feed:
            ctx.text(PAD, HEADER_H + PAD, "No posts.", size=BODY, color=MUTED)
            return

        content_h = sum(self._row_height(i) for i in range(len(self._feed)))
        list_y = HEADER_H
        ctx.begin_scroll("feed-list", 0.0, list_y, ctx.w,
                         ctx.h - list_y, content_h)

        y = list_y - self._scroll
        for i, post in enumerate(self._feed):
            row_h = self._row_height(i)
            sel   = i == self._sel
            ctx.rect(0, y, ctx.w, row_h,
                     fill=HIGHLIGHT if sel else (SURFACE if i % 2 == 0 else BG))
            if i > 0:
                ctx.rect(0, y, ctx.w, 1, fill=BG)

            # Avatar
            did = _did(post)
            av_handle = self._avatar_handles.get(did, "")
            av_cy = y + row_h / 2
            if av_handle:
                ctx.avatar(av_handle, x=PAD, y_center=av_cy, radius=AVATAR_R)
            else:
                ctx.circle(PAD + AVATAR_R, av_cy, AVATAR_R, fill="#3a3a4a")

            # Author + timestamp
            ts = _short_ts((post.get("record") or {}).get("createdAt", ""))
            ctx.text(TEXT_X, y + 6, _author(post),
                     size=HINT, color=ACCENT,
                     max_width=ctx.w - TEXT_X - PAD - 36)
            if ts:
                ctx.text(ctx.w - PAD - 32, y + 6, ts, size=HINT, color=MUTED)

            # Post body (up to MAX_LINES lines)
            body = _text(post)
            if body:
                ctx.text(TEXT_X, y + 6 + HINT + 4, body,
                         size=CAPTION, color=FG,
                         max_width=ctx.w - TEXT_X - PAD,
                         max_lines=MAX_LINES)

            # Stats
            likes   = post.get("likeCount", 0)
            reposts = post.get("repostCount", 0)
            replies = post.get("replyCount", 0)
            ctx.text(TEXT_X, y + row_h - HINT - 4,
                     f"♥ {likes}  ↺ {reposts}  💬 {replies}",
                     size=HINT, color=MUTED)

            y += row_h

        ctx.end_scroll()

    def _draw_thread(self, ctx: RenderContext) -> None:
        if not self._thread:
            ctx.text(PAD, HEADER_H + PAD, "Empty thread.", size=BODY, color=MUTED)
            return

        heights   = [THREAD_H + min(len(_thumbs(item["post"])), 2) * IMG_H
                     for item in self._thread]
        content_h = sum(heights)
        list_y    = HEADER_H

        ctx.begin_scroll("thread-list", 0.0, list_y, ctx.w,
                         ctx.h - list_y, content_h)

        y = list_y - self._t_scroll
        for idx, item in enumerate(self._thread):
            post   = item["post"]
            depth  = item["depth"]
            h      = heights[idx]
            indent = min(depth, 3) * 12.0

            if idx > 0:
                ctx.rect(PAD + indent, y, ctx.w - PAD - indent, 1, fill=SURFACE)

            # Avatar
            did = _did(post)
            av_handle = self._avatar_handles.get(did, "")
            av_cy = y + HINT / 2 + 6
            if av_handle:
                ctx.avatar(av_handle, x=PAD + indent, y_center=av_cy, radius=AVATAR_R)
            else:
                ctx.circle(PAD + indent + AVATAR_R, av_cy, AVATAR_R, fill="#3a3a4a")

            t_x = PAD + indent + AVATAR_R * 2 + 6

            ts = _short_ts((post.get("record") or {}).get("createdAt", ""))
            ctx.text(t_x, y + 6, _author(post),
                     size=HINT, color=ACCENT,
                     max_width=ctx.w - t_x - PAD - 36)
            if ts:
                ctx.text(ctx.w - PAD - 32, y + 6, ts, size=HINT, color=MUTED)

            ctx.markdown(t_x, y + 6 + HINT + 4,
                         ctx.w - t_x - PAD, _text(post))

            img_y = y + THREAD_H - 6.0
            for thumb in _thumbs(post)[:2]:
                ctx.image(thumb, t_x, img_y, 120.0, 72.0, fit="cover")
                img_y += IMG_H

            likes   = post.get("likeCount", 0)
            reposts = post.get("repostCount", 0)
            replies = post.get("replyCount", 0)
            ctx.text(t_x, y + h - HINT - 4,
                     f"♥ {likes}  ↺ {reposts}  💬 {replies}",
                     size=HINT, color=MUTED)

            y += h

        ctx.end_scroll()

    def _draw_input_overlay(self, ctx: RenderContext) -> None:
        pw = min(380.0, ctx.w - PAD * 4)
        px = (ctx.w - pw) / 2
        py = HEADER_H + 36.0
        ctx.rect(px - 12, py - 12, pw + 24, 84.0, fill=SURFACE, radius=8.0)
        ctx.text(px, py + 4, "Enter handle:", size=CAPTION, color=MUTED)
        ctx.text_input("profile-handle", x=px,
                       y=py + 4 + CAPTION + PAD_TIGHT,
                       w=pw, placeholder="e.g. jay.bsky.social")
        ctx.text(px, py + 60.0, "Esc to cancel", size=HINT, color=MUTED)

    # ── input ─────────────────────────────────────────────────────────────────

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        if self._loading:
            return

        if self._show_input:
            if key == "escape":
                self._show_input = False
                self.emit.schedule_render()
            return

        if self._view == self.VIEW_FEED:
            if key in ("j", "down"):
                self._sel = min(self._sel + 1, max(0, len(self._feed) - 1))
                self.emit.schedule_render()
            elif key in ("k", "up"):
                self._sel = max(self._sel - 1, 0)
                self.emit.schedule_render()
            elif key == "return":
                self._open_thread()
            elif key == "p":
                self._show_input = True
                self.emit.schedule_render()
            elif key == "r":
                self.emit.info("bluesky: refresh")
                self._page_fetch(self._cursors[self._page_idx])
            elif key == "o":
                self._open_browser(from_thread=False)
            elif key == "n":
                self._next_page()
            elif key == "N":
                self._prev_page()

        elif self._view == self.VIEW_THREAD:
            if key == "escape":
                self._view   = self.VIEW_FEED
                self._thread = []
                self.emit.info("bluesky: back to feed")
                self.emit.schedule_render()
            elif key == "o":
                self._open_browser(from_thread=True)

    def on_text_submitted(self, _ctx: RenderContext, id: str, text: str) -> None:
        if id != "profile-handle" or not text.strip():
            return
        handle = text.strip().lstrip("@")
        self._show_input    = False
        self._author_handle = handle
        self._feed_label    = f"@{handle}"
        self._cursors       = [None]
        self._page_idx      = 0
        self._next_cur      = None
        self.emit.info(f"bluesky: profile lookup @{handle}")
        asyncio.create_task(self._fetch_author(handle, None))

    def on_scroll(self, _ctx: RenderContext, id: str, offset_y: float) -> None:
        if id == "feed-list":
            self._scroll = offset_y
        elif id == "thread-list":
            self._t_scroll = offset_y

    def on_click(self, _ctx: RenderContext, _x: float, y: float, button: str) -> None:
        if self._view == self.VIEW_FEED and not self._loading and self._feed:
            local_y = y - HEADER_H + self._scroll
            if local_y >= 0:
                acc = 0.0
                for i, rh in enumerate(self._row_heights):
                    if local_y < acc + rh:
                        self._sel = i
                        if button == "primary":
                            self._open_thread()
                        break
                    acc += rh

    # ── actions ───────────────────────────────────────────────────────────────

    def _open_thread(self) -> None:
        if not self._feed:
            return
        post = self._feed[self._sel]
        uri  = post.get("uri", "")
        self.emit.info(f"bluesky: open thread uri={uri!r}")
        self._view   = self.VIEW_THREAD
        self._thread = []
        asyncio.create_task(self._fetch_thread(uri))

    def _open_browser(self, from_thread: bool) -> None:
        uri = ""
        if from_thread and self._thread:
            uri = self._thread[0]["post"].get("uri", "")
        elif self._feed:
            uri = self._feed[self._sel].get("uri", "")
        url = _at_web(uri)
        if url:
            webbrowser.open(url)
            self.emit.info(f"bluesky: open browser {url}")


BlueskyApp().run()
