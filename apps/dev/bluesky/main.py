#!/usr/bin/env python3
"""bluesky — read-only Bluesky feed browser via the public AT Protocol API.

No login required. Uses public.api.bsky.app XRPC endpoints.

Views:
  FEED    — Discover / What's Hot feed (or author feed after profile lookup)
  THREAD  — Full thread for the selected post

Keys (FEED): j/k nav · Enter thread · p profile · r refresh · o browser · n/N page · Esc close
Keys (THREAD): Esc back · o browser
"""

import asyncio
import json
import urllib.parse
import webbrowser

from plexi_sdk import (
    App, RenderContext, CapabilityDeniedError,
    CAPTION, HINT, PAD,
)
from plexi_sdk.ui import (
    AppBar, FooterKeys,
    ListRow, LeadingAvatar, LeadingIcon,
)

BASE     = "https://public.api.bsky.app/xrpc"
DISCOVER = "at://did:plc:z72i7hdynmk6r22z27h6tvur/app.bsky.feed.generator/whats-hot"
LIMIT    = 30

AVATAR_R  = 14.0   # avatar circle radius in thread view
IMG_H     = 116.0  # slot height per image in thread view (104px image + 12px gap)
NARROW_W  = 400    # pane width threshold for compact layout
# Thread row layout constants
_TH_TOP   = 6.0    # padding above author line
_TH_GAP   = 4.0    # gap between author line and text
_TH_BOT   = 8.0    # padding below stats line
_TH_IMGAP = 8.0    # gap between text and first image


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


def _est_text_h(text: str, avail_w: float) -> float:
    """Estimate markdown text height at HINT font size."""
    chars_per_line = max(20, avail_w / 7.5)
    lines = max(1, -(-len(text) // int(chars_per_line)))  # ceil
    return lines * (HINT + 5.0)


def _thread_row_h(post: dict, avail_w: float) -> float:
    text_h = _est_text_h(_text(post), avail_w)
    n_imgs = min(len(_thumbs(post)), 2)
    return _TH_TOP + HINT + _TH_GAP + text_h + _TH_IMGAP + n_imgs * IMG_H + HINT + _TH_BOT


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

    def on_init(self) -> None:
        self._view    = self.VIEW_FEED
        self._feed    : list[dict] = []
        self._sel     = 0
        self._loading = True
        self._error   : str | None = None

        self._cursors  : list[str | None] = [None]
        self._page_idx = 0
        self._next_cur : str | None = None

        self._feed_label    = "Discover"
        self._author_handle : str | None = None
        self._show_input    = False

        self._thread   : list[dict] = []
        self._t_scroll = 0.0

        self._avatar_handles: dict[str, str] = {}

        # Headless render: --state JSON lands in self._app_state before on_init.
        seeded = self._app_state
        if "_feed" in seeded or "_thread" in seeded:
            self._loading = seeded.get("_loading", False)
            self._error   = seeded.get("_error")
            self._sel     = seeded.get("_sel", 0)
            if "_feed" in seeded:
                self._feed = seeded["_feed"]
            if "_thread" in seeded:
                raw = seeded["_thread"]
                self._thread = [{"post": p, "depth": i} for i, p in enumerate(raw)]
                self._view   = self.VIEW_THREAD
            self.emit.info(f"bluesky: seeded feed={len(self._feed)} thread={len(self._thread)}")
            return

        self.emit.info("bluesky: init")
        self.emit.status_summary("Loading Discover feed…")
        asyncio.create_task(self._init_async())

    async def _init_async(self) -> None:
        try:
            await self.emit.capability_request("net.http")
        except CapabilityDeniedError:
            self._loading = False
            self._error = "Network access denied."
            self.emit.warn("bluesky: net.http capability denied")
            self.emit.schedule_render()
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
            self._loading  = False
            self.emit.info(f"bluesky: discover loaded {len(self._feed)} posts")
            self.emit.schedule_render()
            asyncio.create_task(self._load_avatars(self._feed))
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
            self._loading  = False
            self.emit.info(f"bluesky: author @{handle} loaded {len(self._feed)} posts")
            self.emit.schedule_render()
            asyncio.create_task(self._load_avatars(self._feed))
        except Exception as exc:
            self.emit.warn(f"bluesky: author feed error: {exc}")
            self._error = str(exc)
            self._loading = False
            self.emit.schedule_render()

    async def _load_avatars(self, posts: list[dict]) -> None:
        for post in posts:
            did = _did(post)
            url = _avatar_url(post)
            if did and url and did not in self._avatar_handles:
                try:
                    handle = await self.emit.load_image(url)
                    self._avatar_handles[did] = handle
                    self.emit.schedule_render()
                except Exception as exc:
                    self.emit.warn(f"bluesky: avatar load failed for {did}: {exc}")

    async def _fetch_thread(self, uri: str) -> None:
        self._loading = True
        self._error   = None
        self.emit.schedule_render()
        url = f"{BASE}/app.bsky.feed.getPostThread?uri={urllib.parse.quote(uri)}&depth=6"
        try:
            data  = json.loads(await self.emit.http_get(url)) or {}
            if data.get("error"):
                raise RuntimeError(data["error"])
            posts: list[dict] = []
            self._walk(data.get("thread", {}), posts, depth=0)
            self._thread   = posts
            self._t_scroll = 0.0
            self._loading  = False
            self.emit.info(f"bluesky: thread loaded {len(posts)} nodes uri={uri!r}")
            self.emit.schedule_render()
            asyncio.create_task(self._load_avatars([item["post"] for item in self._thread]))
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

    def _build_rows(self, narrow: bool) -> list[dict]:
        rows = []
        for i, post in enumerate(self._feed):
            did       = _did(post)
            av_handle = self._avatar_handles.get(did, "")
            ts        = _short_ts((post.get("record") or {}).get("createdAt", ""))
            likes     = post.get("likeCount", 0)
            reposts   = post.get("repostCount", 0)
            a         = post.get("author") or {}
            name      = a.get("displayName") or ""
            hand      = a.get("handle") or "?"
            text      = _text(post)

            # primary: @handle  secondary: Name · text · stats
            stat_str = f"♥{likes}  ↺{reposts}"
            parts = [p for p in [name if name != hand else "", text, stat_str] if p]
            secondary = "  ·  ".join(parts) if parts else None

            leading = (
                LeadingIcon("👤") if (narrow or not av_handle)
                else LeadingAvatar(av_handle)
            )

            rows.append(ListRow(
                id=f"post-{i}",
                leading=leading,
                primary=f"@{hand}",
                secondary=secondary,
                chips=[],
                trailing=ts or None,
            ).to_dict())
        return rows

    def on_render(self, ctx: RenderContext) -> None:
        label = "Thread" if self._view == self.VIEW_THREAD else self._feed_label
        subtitle = f"page {self._page_idx + 1}" if (self._view == self.VIEW_FEED and self._page_idx > 0) else None
        appbar = AppBar(title=label, subtitle=subtitle, accent=ctx.theme.accent)
        appbar_h = appbar.measure(ctx.w)

        feed_keys = [
            (["j", "k"], "nav"), ("↩", "thread"),
            ("p", "profile"), ("r", "refresh"),
            ("o", "browser"), (["n", "N"], "page"), ("esc", "close"),
        ]
        thread_keys = [("esc", "back"), ("o", "browser")]
        footer = FooterKeys(thread_keys if self._view == self.VIEW_THREAD else feed_keys)
        footer_h = footer.measure(ctx.w)

        ctx.clear(ctx.theme.bg)
        appbar.render(ctx, 0.0, 0.0, ctx.w, appbar_h)
        footer.render(ctx, 0.0, ctx.h - footer_h, ctx.w, footer_h)

        content_y = appbar_h
        content_h = ctx.h - appbar_h - footer_h

        if self._error and not self._loading:
            ctx.text(PAD, content_y + PAD, f"Error: {self._error}",
                     size=CAPTION, color=ctx.theme.danger, max_width=ctx.w - PAD * 2)
            ctx.text(PAD, content_y + PAD + CAPTION + 8, "r — retry",
                     size=HINT, color=ctx.theme.muted)
            return

        if self._view == self.VIEW_THREAD:
            self._draw_thread(ctx, content_y, content_h)
        else:
            if self._show_input:
                self._draw_input_overlay(ctx, content_y)
            ctx.list_view(
                "feed",
                self._build_rows(narrow=ctx.w <= NARROW_W),
                selected=self._sel,
                loading=self._loading,
                error=self._error,
                y=content_y,
                h=content_h,
            )

    def _draw_thread(self, ctx: RenderContext, content_y: float, content_h: float) -> None:
        if not self._thread:
            ctx.text(PAD, content_y + PAD, "Empty thread.",
                     size=14.0, color=ctx.theme.muted)
            return

        # Dynamic heights: measure text per post so long posts don't overlap images.
        base_avail_w = ctx.w - PAD * 2 - AVATAR_R * 2 - 6
        heights = [_thread_row_h(item["post"], base_avail_w) for item in self._thread]
        content_tot = sum(heights)

        ctx.begin_scroll("thread-list", 0.0, content_y, ctx.w, content_h, content_tot)

        y = content_y - self._t_scroll
        for idx, item in enumerate(self._thread):
            post   = item["post"]
            depth  = item["depth"]
            h      = heights[idx]
            indent = min(depth, 3) * 12.0

            if idx > 0:
                ctx.rect(PAD + indent, y, ctx.w - PAD - indent, 1,
                         fill=ctx.theme.highlight)

            did       = _did(post)
            av_handle = self._avatar_handles.get(did, "")
            av_cy     = y + _TH_TOP + HINT / 2
            if av_handle:
                ctx.avatar(av_handle, x=PAD + indent, y_center=av_cy, radius=AVATAR_R)
            else:
                ctx.circle(PAD + indent + AVATAR_R, av_cy, AVATAR_R,
                           fill=ctx.theme.surface)

            t_x      = PAD + indent + AVATAR_R * 2 + 6
            avail_w  = ctx.w - t_x - PAD
            ts       = _short_ts((post.get("record") or {}).get("createdAt", ""))
            ctx.text(t_x, y + _TH_TOP, _author(post),
                     size=HINT, color=ctx.theme.accent,
                     max_width=avail_w - 36)
            if ts:
                ctx.text(ctx.w - PAD - 32, y + _TH_TOP, ts, size=HINT, color=ctx.theme.muted)

            text_y = y + _TH_TOP + HINT + _TH_GAP
            ctx.markdown(t_x, text_y, avail_w, _text(post))

            text_h = _est_text_h(_text(post), avail_w)
            img_y  = text_y + text_h + _TH_IMGAP
            img_w  = max(120.0, avail_w)
            for thumb in _thumbs(post)[:2]:
                ctx.image(thumb, t_x, img_y, img_w, 104.0, fit="cover")
                img_y += IMG_H

            likes   = post.get("likeCount", 0)
            reposts = post.get("repostCount", 0)
            replies = post.get("replyCount", 0)
            ctx.text(t_x, y + h - HINT - _TH_BOT + 4,
                     f"♥ {likes}  ↺ {reposts}  ↩ {replies}",
                     size=HINT, color=ctx.theme.muted)

            y += h

        ctx.end_scroll()

    def _draw_input_overlay(self, ctx: RenderContext, content_y: float) -> None:
        pw = min(380.0, ctx.w - PAD * 4)
        px = (ctx.w - pw) / 2
        py = content_y + 36.0
        ctx.rect(px - 12, py - 12, pw + 24, 84.0,
                 fill=ctx.theme.surface, radius=8.0)
        ctx.text(px, py + 4, "Enter handle:", size=CAPTION, color=ctx.theme.muted)
        ctx.text_input("profile-handle", x=px,
                       y=py + 4 + CAPTION + 8,
                       w=pw, placeholder="e.g. jay.bsky.social")
        ctx.text(px, py + 60.0, "Esc to cancel", size=HINT, color=ctx.theme.muted)

    # ── input ─────────────────────────────────────────────────────────────────

    def on_escape(self) -> bool:
        if self._show_input:
            self._show_input = False
            self.emit.schedule_render()
            return True
        if self._view == self.VIEW_THREAD:
            self._view   = self.VIEW_FEED
            self._thread = []
            self.emit.info("bluesky: back to feed")
            self.emit.schedule_render()
            return True
        return False  # let framework close the app

    def on_key(self, key: str, _mods: dict) -> None:
        if self._loading:
            return

        if self._show_input:
            return  # input overlay handles its own text; Esc handled by on_escape

        if self._view == self.VIEW_FEED:
            if key == "p":
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
            if key == "o":
                self._open_browser(from_thread=True)

    def on_text_submitted(self, id: str, text: str) -> None:
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

    def on_list_select(self, _id: str, index: int) -> None:
        self._sel = index
        self.emit.schedule_render()

    def on_list_activate(self, _id: str, _index: int) -> None:
        self._open_thread()

    def on_scroll(self, id: str, offset_y: float) -> None:
        if id == "thread-list":
            self._t_scroll = offset_y

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
