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
from pathlib import Path
import urllib.parse
import webbrowser

from plexi_sdk import (
    App, RenderContext, CapabilityDeniedError, Arg,
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
NARROW_W  = 400    # pane width threshold for compact layout
IMG_MIN_H = 96.0
IMG_MAX_H = 200.0
IMG_FRAC  = 0.36
THREAD_TEXT_SIZE = 14.0
# Thread row layout constants
_TH_TOP   = 6.0    # padding above author line
_TH_GAP   = 4.0    # gap between author line and text
_TH_BOT   = 8.0    # padding below stats line
_TH_IMGAP = 8.0    # gap between text and first image
_TH_IMG_GAP = 12.0 # gap after each image


# ── helpers ───────────────────────────────────────────────────────────────────

def _short_ts(ts: str) -> str:
    try:
        return ts[5:10].replace("-", "/")
    except Exception:
        return ""


def _strip_unrendered_symbols(text: str) -> str:
    """Drop emoji-style codepoints that currently render as placeholder boxes."""
    out: list[str] = []
    for ch in text:
        cp = ord(ch)
        if (
            0x1F000 <= cp <= 0x1FAFF
            or 0xFE00 <= cp <= 0xFE0F
            or cp == 0x200D
        ):
            continue
        out.append(ch)
    return "".join(out)


def _author(post: dict) -> str:
    a    = post.get("author") or {}
    name = _strip_unrendered_symbols(a.get("displayName") or "")
    hand = a.get("handle") or "?"
    return f"{name} @{hand}" if name and name != hand else f"@{hand}"


def _avatar_url(post: dict) -> str:
    return (post.get("author") or {}).get("avatar") or ""


def _did(post: dict) -> str:
    return (post.get("author") or {}).get("did") or ""


def _text(post: dict) -> str:
    return _strip_unrendered_symbols((post.get("record") or {}).get("text", ""))


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
    """Fallback only; live thread rows use host text measurement."""
    chars_per_line = max(20, avail_w / 7.5)
    lines = max(1, -(-len(text) // int(chars_per_line)))  # ceil
    return lines * (THREAD_TEXT_SIZE + 5.0)


def _image_h(pane_w: float) -> float:
    return max(IMG_MIN_H, min(IMG_MAX_H, pane_w * IMG_FRAC))


def _fmt_count(value: int) -> str:
    try:
        value = int(value)
    except Exception:
        value = 0
    if value >= 1_000_000:
        return f"{value / 1_000_000:.1f}m".replace(".0m", "m")
    if value >= 1_000:
        return f"{value / 1_000:.1f}k".replace(".0k", "k")
    return str(value)


def _thread_row_h(post: dict, text_h: float, image_h: float) -> float:
    n_imgs = min(len(_thumbs(post)), 2)
    image_block_h = (_TH_IMGAP + n_imgs * (image_h + _TH_IMG_GAP)) if n_imgs else 0.0
    return _TH_TOP + HINT + _TH_GAP + text_h + image_block_h + HINT + _TH_BOT


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
    fixture: Arg[str | None] = Arg("--fixture", default=None)

    def on_init(self) -> None:
        self._view    = self.VIEW_FEED
        self._feed    : list[dict] = []
        self._sel     = 0
        self._loading = True
        self._error   : str | None = None
        self._has_net_http = False

        self._cursors  : list[str | None] = [None]
        self._page_idx = 0
        self._next_cur : str | None = None

        self._feed_label    = "Discover"
        self._author_handle : str | None = None
        self._show_input    = False

        self._thread   : list[dict] = []
        self._thread_heights: list[float] = []
        self._thread_measure_w = 0.0
        self._thread_measure_pending_w: float | None = None
        self._thread_measure_pending_generation: int | None = None
        self._thread_generation = 0
        self._t_scroll = 0.0

        self._avatar_handles: dict[str, str] = {}

        # Headless render: --state JSON lands in self._app_state before on_init.
        seeded = self._app_state
        if self.fixture:
            fixture_path = Path(__file__).with_name(self.fixture)
            try:
                seeded = json.loads(fixture_path.read_text())
                self.emit.info(f"bluesky: loaded fixture {fixture_path.name}")
            except Exception as exc:
                seeded = {"_loading": False, "_error": f"Fixture load failed: {exc}"}
                self.emit.warn(f"bluesky: fixture load failed {fixture_path}: {exc}")
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
        self._has_net_http = True
        self.emit.info(f"bluesky: net.http granted capabilities={self.capabilities}")
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
        if not self._has_net_http and "net.http" not in self.capabilities:
            self.emit.warn("bluesky: skip avatar load; net.http not granted")
            return
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
            self._thread_generation += 1
            self._thread_heights = []
            self._thread_measure_w = 0.0
            self._thread_measure_pending_w = None
            self._thread_measure_pending_generation = None
            self._t_scroll = 0.0
            self._loading  = False
            self.emit.info(f"bluesky: thread loaded {len(posts)} nodes uri={uri!r}")
            self.emit.schedule_render()
            asyncio.create_task(self._measure_thread_heights())
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

    def _thread_text_w(self, pane_w: float, depth: int) -> float:
        indent = min(depth, 3) * 12.0
        t_x = PAD + indent + AVATAR_R * 2 + 6
        return max(80.0, pane_w - t_x - PAD)

    async def _measure_thread_heights(self) -> None:
        pane_w = float(self._rect.get("w", 800.0))
        await self._measure_thread_heights_for_width(pane_w, self._thread_generation)

    async def _measure_thread_heights_for_width(self, pane_w: float, generation: int) -> None:
        self._thread_measure_pending_w = pane_w
        self._thread_measure_pending_generation = generation
        heights: list[float] = []
        try:
            thread_snapshot = list(self._thread)
            for item in thread_snapshot:
                post = item["post"]
                avail_w = self._thread_text_w(pane_w, int(item.get("depth", 0)))
                try:
                    text_h = await self.emit.measure_text_wrapped(_text(post), THREAD_TEXT_SIZE, avail_w)
                except Exception as exc:
                    self.emit.warn(f"bluesky: thread text measure failed: {exc}")
                    text_h = _est_text_h(_text(post), avail_w)
                heights.append(text_h)
            if generation != self._thread_generation or thread_snapshot != self._thread:
                self.emit.info("bluesky: discarded stale thread measurement")
                return
            self._thread_heights = heights
            self._thread_measure_w = pane_w
            self.emit.info(f"bluesky: measured thread rows={len(heights)} width={pane_w:.0f}")
            self.emit.schedule_render()
        finally:
            if (
                self._thread_measure_pending_generation == generation
                and self._thread_measure_pending_w == pane_w
            ):
                self._thread_measure_pending_w = None
                self._thread_measure_pending_generation = None

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
            name      = _strip_unrendered_symbols(a.get("displayName") or "")
            hand      = a.get("handle") or "?"
            text      = _text(post)

            # primary: @handle  secondary: Name · text; stats stay visible in trailing.
            parts = [p for p in [name if name != hand else "", text] if p]
            secondary = "  ·  ".join(parts) if parts else None
            trailing_parts = [f"like {_fmt_count(likes)}", f"repost {_fmt_count(reposts)}"]
            if ts:
                trailing_parts.append(ts)

            leading = (
                LeadingIcon("@") if (narrow or not av_handle)
                else LeadingAvatar(av_handle)
            )

            rows.append(ListRow(
                id=f"post-{i}",
                leading=leading,
                primary=f"@{hand}",
                secondary=secondary,
                chips=[],
                trailing="  ".join(trailing_parts),
            ).to_dict())
        return rows

    def on_render(self, ctx: RenderContext) -> None:
        label = "Thread" if self._view == self.VIEW_THREAD else self._feed_label
        subtitle = f"page {self._page_idx + 1}" if (self._view == self.VIEW_FEED and self._page_idx > 0) else None
        appbar = AppBar(title=label, subtitle=subtitle, accent=ctx.theme.accent)
        appbar_h = appbar.measure(ctx.w)

        feed_keys = [
            (["j", "k"], "nav"), ("enter", "thread"),
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

        image_h = _image_h(ctx.w)
        text_heights = list(self._thread_heights)
        if len(text_heights) != len(self._thread) or abs(self._thread_measure_w - ctx.w) > 1.0:
            if (
                self._thread_measure_pending_generation != self._thread_generation
                or self._thread_measure_pending_w is None
                or abs(self._thread_measure_pending_w - ctx.w) > 1.0
            ):
                asyncio.create_task(self._measure_thread_heights_for_width(ctx.w, self._thread_generation))
            text_heights = [
                _est_text_h(_text(item["post"]), self._thread_text_w(ctx.w, int(item.get("depth", 0))))
                for item in self._thread
            ]
        heights = [
            _thread_row_h(item["post"], text_heights[idx], image_h)
            for idx, item in enumerate(self._thread)
        ]
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
            ctx.markdown(t_x, text_y, avail_w, _text(post), base_size=THREAD_TEXT_SIZE)

            text_h = text_heights[idx]
            img_y  = text_y + text_h + _TH_IMGAP
            img_w  = max(120.0, avail_w)
            for thumb in _thumbs(post)[:2]:
                ctx.image(thumb, t_x, img_y, img_w, image_h, fit="cover")
                img_y += image_h + _TH_IMG_GAP

            likes   = post.get("likeCount", 0)
            reposts = post.get("repostCount", 0)
            replies = post.get("replyCount", 0)
            ctx.text(t_x, y + h - HINT - _TH_BOT + 4,
                     f"like {likes}  repost {reposts}  reply {replies}",
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
        self._thread_generation += 1
        self._thread_heights = []
        self._thread_measure_w = 0.0
        self._thread_measure_pending_w = None
        self._thread_measure_pending_generation = None
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
