#!/usr/bin/env python3
"""Wikipedia — search and read Wikipedia articles via the MediaWiki API.

L1 Reference Implementation
============================
Patterns demonstrated:
  - Fully L1: Column + AppBar + Section + SelectList + Scrollable + Label + FooterKeys
  - TextInput for search queries
  - net.http capability request (async, with CapabilityDeniedError handling)
  - Async networking via threading + emit.run_sync(emit.http_get(...))
  - Multi-view navigation: search -> results -> article (Esc to go back)
  - on_inject for test seam (seed mode/query/results/extract without network)
  - Scrollable wrapping Label for long article text
  - Loading state management with status_summary
"""

import json
import threading
import urllib.parse

from plexi_sdk import (
    App, CapabilityDeniedError,
)
from plexi_sdk.ui import (
    Column, AppBar, Spacer, Footer, FooterKeys,
    Label, Section, Scrollable, SelectList, TextInput, theme,
)

API = "https://en.wikipedia.org/w/api.php"
EXTRACT_API = "https://en.wikipedia.org/api/rest_v1/page/summary/"


class WikiApp(App):
    async def on_init(self) -> None:
        self._query = ""
        self._results: list[str] = []
        self._selected = 0
        self._extract = ""
        self._loading = False
        self._error_msg = ""
        self._mode = "search"  # search | results | article

        # Stateful widgets — created once here, reused across renders.
        self._search_input = TextInput(
            id="wiki_search",
            placeholder="Enter search query…",
        )
        self._results_list = SelectList(items=[], selected_idx=0)
        self._article_scroll = Scrollable(
            child=Label("", tone="body", max_lines=500),
        )

        self.emit.info("wikipedia: ready")
        try:
            await self.emit.capability_request("net.http")
        except CapabilityDeniedError:
            self._error_msg = "Network access denied. Search requires net.http."
            self.emit.warn("wikipedia: net.http capability denied")

    def on_inject(self, payload: dict) -> None:
        """Layer-1 test seam — seed mode/query/results/extract without network."""
        if isinstance(payload, dict):
            if "mode" in payload:
                self._mode = payload["mode"]
            if "query" in payload:
                self._query = payload["query"]
            if "results" in payload:
                self._results = list(payload["results"])
                self._selected = 0
                self._results_list.items = [{"name": r} for r in self._results]
                self._results_list.selected_idx = 0
            if "extract" in payload:
                self._extract = payload["extract"]
                self._article_scroll.child = Label(
                    self._extract, tone="body", max_lines=500,
                )

    def on_escape(self):
        if self._mode == "article":
            self._mode = "results"
            self.emit.schedule_render()
            return True
        if self._mode == "results":
            self._mode = "search"
            self.emit.schedule_render()
            return True
        return False

    def on_key(self, key: str, _mods: dict) -> None:
        if self._mode == "results":
            if self._results_list.handle_key(key):
                self._selected = self._results_list.selected_idx
                self.emit.schedule_render()
                return
            if key in ("return", "Enter"):
                if self._results:
                    self._fetch_article(self._results[self._selected])
                return
        elif self._mode == "article":
            if self._article_scroll.handle_key(key):
                self.emit.schedule_render()
                return

    def _fetch_search(self, query: str) -> None:
        self._loading = True
        self._error_msg = ""
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
                self._results_list.items = [{"name": r} for r in self._results]
                self._results_list.selected_idx = 0
                self._mode = "results"
                self.emit.info(f"wikipedia: search '{query}' -> {len(self._results)} results")
            except Exception as e:
                self.emit.error(f"wikipedia search failed: {e}")
                self._error_msg = str(e)
                self._results = []
            finally:
                self._loading = False
                self.emit.status_summary("")
        threading.Thread(target=run, daemon=True).start()

    def _fetch_article(self, title: str) -> None:
        self._loading = True
        self._error_msg = ""
        self.emit.status_summary(f"Loading {title}…")

        def run() -> None:
            try:
                url = EXTRACT_API + urllib.parse.quote(title)
                body = self.emit.run_sync(self.emit.http_get(url))
                data = json.loads(body)
                self._extract = data.get("extract", "No extract available.")
                self._article_scroll.child = Label(
                    self._extract, tone="body", max_lines=500,
                )
                self._article_scroll.scroll_offset = 0.0
                self._mode = "article"
                self.emit.info(f"wikipedia: article '{title}' loaded")
            except Exception as e:
                self.emit.error(f"wikipedia article fetch failed: {e}")
                self._error_msg = str(e)
                self._extract = ""
            finally:
                self._loading = False
                self.emit.status_summary("")
        threading.Thread(target=run, daemon=True).start()

    def _body_children(self) -> list:
        if self._mode == "search":
            children: list = [self._search_input]
            if self._error_msg:
                children.append(Label(f"Error: {self._error_msg}", tone="hint", color=theme.danger))
            else:
                children.append(Label("Type a query and press Enter", tone="hint"))
            return children

        if self._mode == "results":
            return [
                Section(f'Results for "{self._query}"'),
                self._results_list,
            ]

        # article mode
        title = self._results[self._selected] if self._results else ""
        return [
            Section(title),
            self._article_scroll,
        ]

    def _footer_component(self):
        if self._mode == "search":
            return Footer("Type a query · Enter to search")
        if self._mode == "results":
            return FooterKeys([
                (["↑", "↓"], "navigate"),
                ("↵", "open"),
                ("esc", "back"),
            ])
        return FooterKeys([
            (["j", "k"], "scroll"),
            ("esc", "back to results"),
        ])

    def on_render(self, ctx) -> None:
        # Check for search submission from TextInput
        if self._search_input.submitted is not None:
            query = self._search_input.submitted.strip()
            if query:
                self._query = query
                self._fetch_search(query)

        ctx.render(Column([
            AppBar(title="Wikipedia"),
            *self._body_children(),
            Spacer(grow=False),
            self._footer_component(),
        ]))


if __name__ == "__main__":
    WikiApp().run()
