#!/usr/bin/env python3
"""Wikipedia — net.http + text render example for PGAP v3."""
from __future__ import annotations

import sys
import os
sys.path.insert(0, os.path.dirname(__file__))

import json
import threading
import urllib.parse
from plexi_sdk import App, RenderContext, BG, FG, MUTED, ACCENT, SURFACE, HIGHLIGHT, BODY, CAPTION, HINT

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
            if key == "ArrowUp" or key == "k":
                self._selected = max(0, self._selected - 1)
            elif key == "ArrowDown" or key == "j":
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
                body = self.emit.http_get(f"{API}?{params}")
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
                body = self.emit.http_get(url)
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

    def on_render(self, ctx: RenderContext) -> None:
        ctx.rect(0, 0, ctx.w, ctx.h, fill=BG)
        # Header
        ctx.rect(0, 0, ctx.w, 44, fill=SURFACE)
        ctx.text(16, 14, "Wikipedia", size=18.0, color=ACCENT, bold=True)
        if self._loading:
            ctx.text(ctx.w - 100, 14, "Loading…", size=12.0, color=MUTED)

        y = 60.0
        if self._mode == "search":
            ctx.text(16, y, "Search:", size=BODY, color=FG)
            ctx.rect(16, y + 24, ctx.w - 32, 32, fill=SURFACE, radius=4.0)
            ctx.text(24, y + 32, self._query + "▌", size=BODY, color=FG, monospace=True)
            ctx.text(16, y + 72, "Type a query and press Enter", size=HINT, color=MUTED)
        elif self._mode == "results":
            ctx.text(16, y, f'Results for "{self._query}":', size=BODY, color=FG)
            items = [{"label": r, "secondary": None} for r in self._results]
            ctx.list(items, selected=self._selected, item_height=40.0)
            ctx.text(16, ctx.h - 28, "↑↓ navigate · Enter open · Esc back", size=HINT, color=MUTED)
        elif self._mode == "article":
            title = self._results[self._selected] if self._results else ""
            ctx.text(16, y, title, size=18.0, color=ACCENT, bold=True)
            # Word-wrap extract into lines
            words = self._extract.split()
            line, lines = "", []
            for w in words:
                candidate = (line + " " + w).strip()
                if len(candidate) * 8 > ctx.w - 32:
                    lines.append(line)
                    line = w
                else:
                    line = candidate
            if line:
                lines.append(line)
            ty = y + 30
            for l in lines[:int((ctx.h - ty - 40) / 20)]:
                ctx.text(16, ty, l, size=CAPTION, color=FG)
                ty += 20
            ctx.text(16, ctx.h - 28, "Esc back to results", size=HINT, color=MUTED)


if __name__ == "__main__":
    WikiApp().run()
