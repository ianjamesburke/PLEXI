#!/usr/bin/env python3
"""Secrets Routing Tester — POC for #322.

Declares two canonical secrets in manifest.toml and lets the user fetch each
via ctx.get_secret(...). The host resolves them through the workspace
.plexi/secrets.toml router. Values are masked in the UI (first 8 chars + "…")
and never logged.

Keys:
  o — Fetch OPENAI_API_KEY (required)
  g — Fetch GITHUB_TOKEN (optional)
  r — Refresh both
"""
from __future__ import annotations

import threading

from plexi_sdk import App, RenderContext
from plexi_sdk.ui import (
    AppBar,
    Card,
    Column,
    Footer,
    KeyRow,
    Label,
    Section,
    Spacer,
)


CANONICAL_SECRETS = ("OPENAI_API_KEY", "GITHUB_TOKEN")


def _mask(value: str) -> str:
    """Mask a secret for display. Show first 8 chars + ellipsis if long enough,
    otherwise show middle-dots so we never reveal a short value's content."""
    if not value:
        return "(empty)"
    if len(value) > 8:
        return value[:8] + "…"
    return "·" * len(value)


class SecretsRoutingTesterApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        # status[name] is one of:
        #   None         — not yet fetched
        #   "fetching"   — request in flight
        #   ("ok", masked_preview)
        #   ("missing",) — host returned None (denied / hard-missing / prompt)
        self._status: dict[str, object] = {n: None for n in CANONICAL_SECRETS}
        self._workspace_root = ctx.workspace_root or ""
        ctx.status_summary("Secrets Routing Demo — ready")
        self.emit.info("Secrets Routing Tester started")

    def _fetch(self, name: str) -> None:
        """Background-thread fetch — uses emit.run_sync to await secret_get
        without blocking the event loop. Never logs the resolved value."""
        self._status[name] = "fetching"
        self.emit.schedule_render(after_ms=20)

        def runner() -> None:
            try:
                value = self.emit.run_sync(self.emit.get_secret(name))
            except Exception as e:
                self.emit.error(f"get_secret({name}) raised: {e}")
                self._status[name] = ("missing",)
                self.emit.schedule_render(after_ms=20)
                return
            if value is None:
                self._status[name] = ("missing",)
                self.emit.info(f"{name}: missing (denied / hard-missing / prompt)")
            else:
                self._status[name] = ("ok", _mask(value))
                # NB: we deliberately log only the length, never the value.
                self.emit.info(f"{name}: resolved (len={len(value)})")
            self.emit.schedule_render(after_ms=20)

        threading.Thread(target=runner, daemon=True).start()

    def _refresh_all(self) -> None:
        for name in CANONICAL_SECRETS:
            self._fetch(name)

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        k = key.lower()
        if k == "o":
            self._fetch("OPENAI_API_KEY")
        elif k == "g":
            self._fetch("GITHUB_TOKEN")
        elif k == "r":
            self._refresh_all()

    def _row_status_text(self, name: str) -> str:
        st = self._status[name]
        if st is None:
            return "not fetched"
        if st == "fetching":
            return "fetching…"
        if isinstance(st, tuple) and st and st[0] == "ok":
            return f"value: {st[1]}"
        if isinstance(st, tuple) and st and st[0] == "missing":
            return "missing — check .plexi/secrets.toml route or Keychain entry"
        return "unknown"

    def on_render(self, ctx: RenderContext) -> None:
        footer_ws = self._workspace_root or "(no workspace)"
        ctx.render(Column([
            AppBar(title="Secrets Routing Demo"),
            Label("Resolves declared secrets via .plexi/secrets.toml routing."),
            Section("Declared secrets"),
            Card([
                KeyRow("o", "Fetch OPENAI_API_KEY (required)"),
                Label(f"  → {self._row_status_text('OPENAI_API_KEY')}"),
                KeyRow("g", "Fetch GITHUB_TOKEN (optional)"),
                Label(f"  → {self._row_status_text('GITHUB_TOKEN')}"),
            ]),
            Section("Actions"),
            Card([
                KeyRow("r", "Refresh both (re-resolve after editing secrets.toml)"),
            ]),
            Spacer(grow=True),
            Footer(f"workspace: {footer_ws}"),
        ]))


SecretsRoutingTesterApp().run()
