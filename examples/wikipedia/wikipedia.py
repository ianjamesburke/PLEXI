#!/usr/bin/env python3
"""Wikipedia — net.http + text render example for PGAP v3."""

import json
import threading
import urllib.parse

from plexi_sdk import (
    App, RenderContext,
    FG, MUTED, ACCENT, SURFACE, BODY, CAPTION,
)
from plexi_sdk.ui import (
    Column, AppBar, Spacer, Footer, FooterKeys, Component,
)

API = "https://en.wikipedia.org/w/api.php"
EXTRACT_API = "https://en.wikipedia.org/api/rest_v1/page/summary/"


class WikiApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._query = ""
        self._results: list[str] = []
        self._selected = 0
        self._extract = ""
        self._loading = False
        self._mode = "search"  # search | results | article

    def on_inject(self, ctx: RenderContext, payload: dict) -> None:
        """Layer-1 test seam — seed mode/query/results/extract without network."""
        if isinstance(payload, dict):
            if "mode" in payload:
                self._mode = payload["mode"]
            if "query" in payload:
                self._query = payload["query"]
            if "results" in payload:
                self._results = list(payload["results"])
                self._selected = 0
            if "extract" in payload:
                self._extract = payload["extract"]

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if self._mode == "search":
            if key == "Enter":
                if self._query:
                    self._fetch_search(self._query)
            elif key == "Backspace":
                self._query = self._query[:-1]
            elif len(key) == 1:
                self._query += key
        elif self._mode == "results":
            if key == "up" or key == "k":
                self._selected = max(0, self._selected - 1)
            elif key == "down" or key == "j":
                self._selected = min(len(self._results) - 1, self._selected + 1)
            elif key == "Enter":
                if self._results:
                    self._fetch_article(self._results[self._selected])
            elif key == "Escape":
                self._mode = "search"
        elif self._mode == "article":
            if key == "Escape":
                self._mode = "results"

    def _fetch_search(self, query: str) -> None:
        self._loading = True
        self.emit.status_summary("Searching…")
        def run() -> None:
            try:
                params = urllib.parse.urlencode({
                    "action": "opensearch", "search": query,
                    "limit": 10, "format": "json",
                })
                body = self.emit.run_sync(self.emit.http_get(f"{API}?{params}"))
                data = json.loads(body)
                self._results = data[1] if len(data) > 1 else []
                self._selected = 0
                self._mode = "results"
            except Exception as e:
                self.emit.error(f"wikipedia search failed: {e}")
                self._results = []
            finally:
                self._loading = False
                self.emit.status_summary("")
        threading.Thread(target=run, daemon=True).start()

    def _fetch_article(self, title: str) -> None:
        self._loading = True
        self.emit.status_summary(f"Loading {title}…")
        def run() -> None:
            try:
                url = EXTRACT_API + urllib.parse.quote(title)
                body = self.emit.run_sync(self.emit.http_get(url))
                data = json.loads(body)
                self._extract = data.get("extract", "No extract available.")
                self._mode = "article"
            except Exception as e:
                self.emit.error(f"wikipedia article fetch failed: {e}")
                self._extract = f"Error: {e}"
            finally:
                self._loading = False
                self.emit.status_summary("")
        threading.Thread(target=run, daemon=True).start()

    # ── Mode body — functional surface, stays primitive ─────────────────────
    class _Body(Component):
        """Renders the search box, results list, or article text into whatever
        vertical space the Column hands us."""
        def __init__(self, app: "WikiApp") -> None:
            self._app = app

        def measure(self, avail_w: float) -> float:
            return 0.0

        def is_grow(self) -> bool:
            return True

        def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
            app = self._app
            if app._mode == "search":
                ctx.text(x, y, "Search:", size=BODY, color=FG)
                ctx.rect(x, y + 24, w, 32, fill=SURFACE, radius=4.0)
                ctx.text(x + 8, y + 32, app._query + "▌",
                         size=BODY, color=FG, monospace=True)
                ctx.text(x, y + 72, "Type a query and press Enter",
                         size=12, color=MUTED)
            elif app._mode == "results":
                ctx.text(x, y, f'Results for "{app._query}":', size=BODY, color=FG)
                items = [{"label": r, "secondary": None} for r in app._results]
                ctx.list_view(items, selected=app._selected, item_height=40.0,
                         x=x, y=y + 28, w=w, h=max(0.0, h - 28))
            elif app._mode == "article":
                title = app._results[app._selected] if app._results else ""
                ctx.text(x, y, title, size=18.0, color=ACCENT, bold=True)
                # Word-wrap extract into lines (rough 8px/char estimate).
                words = app._extract.split()
                line, lines = "", []
                for word in words:
                    candidate = (line + " " + word).strip()
                    if len(candidate) * 8 > w:
                        lines.append(line)
                        line = word
                    else:
                        line = candidate
                if line:
                    lines.append(line)
                ty = y + 30
                for ln in lines[: int(max(0.0, (h - 30)) / 20)]:
                    ctx.text(x, ty, ln, size=CAPTION, color=FG)
                    ty += 20

    def _footer_component(self):
        if self._mode == "search":
            return Footer("Type a query · Enter to search")
        if self._mode == "results":
            return FooterKeys([
                (["↑", "↓"], "navigate"),
                ("↵", "open"),
                ("esc", "back"),
            ])
        return FooterKeys([("esc", "back to results")])

    def on_render(self, ctx: RenderContext) -> None:
        ctx.render(Column([
            AppBar(title="Wikipedia"),
            self._Body(self),
            Spacer(grow=False),
            self._footer_component(),
        ]))


if __name__ == "__main__":
    WikiApp().run()
