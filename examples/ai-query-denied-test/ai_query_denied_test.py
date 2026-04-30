#!/usr/bin/env python3
"""AI Query Denied Test — POC for #284 (gate-denied path).

Manifest does NOT declare `ai.query`. The host must therefore return
`capability denied` immediately for every ai_query call, without ever
reaching the LLM backend. The SDK turns that into a CapabilityDeniedError.

Press `s` to send a test query and watch the gate fire.
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


class AiQueryDeniedTestApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        # `_state` is one of:
        #   None             — no attempt yet
        #   "in_flight"
        #   ("denied", str)  — got CapabilityDeniedError (the happy path here)
        #   ("ok", str)      — unexpected; the gate should have blocked us
        #   ("err", str)     — some other error
        self._state: str | tuple[str, str] | None = None
        ctx.status_summary("AI Query Denied — press `s` to verify the gate")
        self.emit.info("AI Query Denied Test started")

    def _try_call(self) -> None:
        if self._state == "in_flight":
            return
        self._state = "in_flight"
        self.emit.schedule_render(after_ms=20)

        def runner() -> None:
            try:
                resp: AiResponse = self.emit.run_sync(self.emit.ai_query(
                    model_tier="low",
                    system="You are a helpful assistant.",
                    messages=[{"role": "user", "content": "Hello?"}],
                ))
                # Reached only if the gate is broken.
                self._state = ("ok", resp.content)
                self.emit.error(
                    "GATE BUG: ai_query returned content despite missing capability"
                )
            except CapabilityDeniedError as e:
                self._state = ("denied", str(e))
                self.emit.info(f"gate fired (expected): {e}")
            except Exception as e:
                self._state = ("err", str(e))
                self.emit.warn(f"unexpected error from ai_query: {e}")
            finally:
                self.emit.schedule_render(after_ms=20)

        threading.Thread(target=runner, daemon=True).start()

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        if key.lower() == "s":
            self._try_call()

    def _state_card(self) -> Card | None:
        if self._state is None:
            return None
        if self._state == "in_flight":
            return Card([
                Section("In flight"),
                Label("Awaiting response from the host…"),
            ])
        kind = self._state[0] if isinstance(self._state, tuple) else None
        if kind == "denied":
            _, msg = self._state
            return Card([
                Section("Gate fired (expected)"),
                Label(msg, color="#a6e3a1"),
                Label(
                    "The host refused the call because manifest.toml does"
                    " not declare `ai.query`. This is the correct behaviour."
                ),
            ])
        if kind == "ok":
            _, content = self._state
            return Card([
                Section("UNEXPECTED success"),
                Label(content, color="#f38ba8"),
                Label("The gate did NOT fire — this is a bug.", color="#f38ba8"),
            ])
        if kind == "err":
            _, msg = self._state
            return Card([
                Section("Other error"),
                Label(msg, color="#f9e2af"),
            ])
        return None

    def on_render(self, ctx: RenderContext) -> None:
        children = [
            AppBar(title="AI Query Denied Test"),
            Label(
                "This app's manifest does NOT declare `ai.query`. Every call"
                " should be refused at the host's capability gate."
            ),
            Section("Actions"),
            Card([
                KeyRow("s", "Send a test query (expect capability denied)"),
            ]),
        ]
        card = self._state_card()
        if card is not None:
            children.append(card)
        children.append(Spacer(grow=True))
        children.append(Footer("manifest declares: no capabilities"))
        ctx.render(Column(children))


AiQueryDeniedTestApp().run()
