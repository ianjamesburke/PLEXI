#!/usr/bin/env python3
"""AI Query Test — POC for #284.

Drives the host's `ai.query` broker across the three model tiers
(Low / Medium / High → Haiku / Sonnet / Opus). Type a prompt, hit
the tier you want, see the response and token counts.

Keys:
  l — Send the typed prompt at Low tier (Haiku)
  m — Send the typed prompt at Medium tier (Sonnet)
  h — Send the typed prompt at High tier (Opus)
  c — Clear the response

The host requires `ai.query` declared in manifest.toml. This app
declares it; see `ai-query-denied-test/` for the gate-denied path.
"""

import threading

from plexi_sdk import App, RenderContext, CapabilityDeniedError, AiResponse
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


SYSTEM_PROMPT = "You are a helpful assistant. Reply concisely (one or two sentences)."


class AiQueryTestApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._prompt: str = ""
        self._tier_in_flight: str | None = None
        # `_result` is one of:
        #   None        — no query sent yet
        #   ("ok", tier, content, tokens_in, tokens_out)
        #   ("err", tier, message)
        self._result: tuple | None = None
        ctx.status_summary("AI Query — type a prompt, then press l/m/h")
        self.emit.info("AI Query Test started")

    def _dispatch(self, tier: str) -> None:
        if not self._prompt:
            self.emit.info("ai_query: prompt is empty — type something first")
            return
        if self._tier_in_flight is not None:
            self.emit.info(f"ai_query: {self._tier_in_flight} call still in flight — wait")
            return

        self._tier_in_flight = tier
        prompt_snapshot = self._prompt
        self.emit.schedule_render(after_ms=20)

        def runner() -> None:
            try:
                resp: AiResponse = self.emit.run_sync(self.emit.ai_query(
                    model_tier=tier,
                    system=SYSTEM_PROMPT,
                    messages=[{"role": "user", "content": prompt_snapshot}],
                ))
                self._result = (
                    "ok", tier, resp.content, resp.tokens_in, resp.tokens_out,
                )
                self.emit.info(
                    f"ai_query[{tier}]: {resp.tokens_in}/{resp.tokens_out} tokens"
                )
            except CapabilityDeniedError as e:
                self._result = ("err", tier, f"capability denied: {e}")
                self.emit.warn(f"ai_query[{tier}] denied: {e}")
            except Exception as e:
                self._result = ("err", tier, str(e))
                self.emit.warn(f"ai_query[{tier}] failed: {e}")
            finally:
                self._tier_in_flight = None
                self.emit.schedule_render(after_ms=20)

        threading.Thread(target=runner, daemon=True).start()

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        k = key.lower()
        if k == "l":
            self._dispatch("low")
        elif k == "m":
            self._dispatch("medium")
        elif k == "h":
            self._dispatch("high")
        elif k == "c":
            self._result = None
            self.emit.schedule_render(after_ms=20)

    def _result_card(self) -> Card | None:
        if self._tier_in_flight:
            return Card([
                Section(f"In flight: {self._tier_in_flight}"),
                Label("Waiting for the host's AI broker…"),
            ])
        if self._result is None:
            return None
        kind = self._result[0]
        if kind == "ok":
            _, tier, content, tin, tout = self._result
            return Card([
                Section(f"Response ({tier})"),
                Label(content),
                Label(f"tokens: {tin} in / {tout} out"),
            ])
        # Error path: render in red so the failure is obvious.
        _, tier, message = self._result
        return Card([
            Section(f"Error ({tier})"),
            Label(f"  {message}", color="#f38ba8"),
            Label("(set OPENROUTER_API_KEY in your shell environment)"),
        ])

    def on_render(self, ctx: RenderContext) -> None:
        ox, oy, ow = ctx.x, ctx.y, ctx.w

        # Single-line text input, host-owned buffer. Submit (Enter) sends to
        # Low tier by default — buttons l/m/h pick a specific tier explicitly.
        submitted = ctx.text_input(
            "ai-prompt",
            x=ox + 16, y=oy + 96, w=ow - 32,
            placeholder="Ask anything (e.g. 'What is 2+2?'), then press l, m, or h",
        )
        if submitted is not None:
            # Treat Enter as "send at Low tier" — the cheapest path. Apps that
            # want Enter to dispatch differently can swap tiers here.
            self._prompt = submitted
            self._dispatch("low")

        # Mirror submitted text into our prompt buffer so subsequent l/m/h
        # presses use the most recent submission. Apps that want live-editing
        # need #283's per-keystroke text-input event (out of scope here).
        if submitted is not None:
            self._prompt = submitted

        result = self._result_card()
        children = [
            AppBar(title="AI Query Test"),
            Label("v3.3 broker — `ai.query` capability."),
            Section("Prompt (host-owned input below)"),
            # The text_input itself is positioned absolutely above; this
            # section header just labels the area visually for the user.
            Spacer(grow=False),
            Section("Tiers"),
            Card([
                KeyRow("l", "Low tier — Haiku"),
                KeyRow("m", "Medium tier — Sonnet"),
                KeyRow("h", "High tier — Opus"),
                KeyRow("c", "Clear response"),
            ]),
        ]
        if result is not None:
            children.append(result)
        children.append(Spacer(grow=True))
        children.append(Footer(
            f"latest prompt: {self._prompt or '(none yet)'}"
        ))

        ctx.render(Column(children))


AiQueryTestApp().run()
