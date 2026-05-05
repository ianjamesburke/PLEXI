#!/usr/bin/env python3
from __future__ import annotations

import threading
from dataclasses import dataclass

from plexi_sdk import App, RenderContext, CapabilityDeniedError, AiResponse
from plexi_sdk.ui import (
    AppBar,
    ChatBubble,
    Column,
    Divider,
    Label,
    Scrollable,
    Spacer,
    TextInput,
    SPACE_XL,
    SPACE_SM,
    SPACE_MD,
)

SYSTEM_PROMPT = (
    "You are a helpful, concise assistant. "
    "Answer clearly without unnecessary padding."
)

# ── Layout constants (match ui.py defaults) ──────────────────────────────────
# Used to compute copy-button positions after ctx.render() without modifying
# the component tree. These mirror the Column/Scrollable/AppBar constants in
# ui.py — update here if those change.
_COL_PAD: float = SPACE_XL          # 24 — main column left/right/bottom padding
_COL_PAD_TOP: float = SPACE_SM      # 8  — main column top padding
_COL_GAP: float = SPACE_MD          # 12 — gap between column children
_APPBAR_H: float = 37.0             # AppBar.BAND_H (36) + AppBar.DIVIDER_H (1)
_DIVIDER_H: float = 17.0            # Divider 1px + margin_top 8 + margin_bottom 8
_HIST_PAD_TOP: float = SPACE_SM     # 8  — history column pad_top (padding=0, pad_top default)
_SCROLLBAR_W: float = 5.0           # Scrollable._SCROLLBAR_W (3) + gutter (2)
_COPY_BTN_W: float = 30.0
_COPY_BTN_H: float = 13.0


@dataclass
class Turn:
    role: str   # "user" | "assistant" | "error"
    text: str


class ChatApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._turns: list[Turn] = []
        self._in_flight: bool = False
        self._scroll = Scrollable(child=Column([]))
        self._input = TextInput("chat-input", multiline=True, height=64.0)
        self._scroll_to_bottom_next: bool = False
        # (turn_idx, x1, y1, x2, y2) — rebuilt each on_render frame
        self._copy_hitboxes: list[tuple[int, float, float, float, float]] = []
        self._copy_flashing: int = -1  # turn_idx whose copy button shows "✓"
        ctx.status_summary("Chat")
        self.emit.info("chat-poc started")

    def _maybe_scroll_to_bottom(self) -> None:
        if not self._scroll_to_bottom_next:
            return
        self._scroll_to_bottom_next = False
        self._scroll.scroll_offset = 1_000_000.0

    def _send(self, text: str) -> None:
        if not text.strip() or self._in_flight:
            return

        self._turns.append(Turn("user", text))
        self._in_flight = True
        self._scroll_to_bottom_next = True
        self.emit.schedule_render(after_ms=20)

        history = [{"role": t.role, "content": t.text}
                   for t in self._turns if t.role in ("user", "assistant")]

        def runner() -> None:
            try:
                resp: AiResponse = self.emit.run_sync(self.emit.ai_query(
                    model_tier="low",
                    system=SYSTEM_PROMPT,
                    messages=history,
                ))
                self._turns.append(Turn("assistant", resp.content or ""))
                self.emit.info(f"chat: {resp.tokens_in}/{resp.tokens_out} tokens")
            except CapabilityDeniedError as e:
                self._turns.append(Turn("error", f"Capability denied: {e}"))
                self.emit.warn(f"chat: capability denied: {e}")
            except Exception as e:
                self._turns.append(Turn("error", str(e)))
                self.emit.warn(f"chat: query failed: {e}")
            finally:
                self._in_flight = False
                self._scroll_to_bottom_next = True
                self.emit.schedule_render(after_ms=20)

        threading.Thread(target=runner, daemon=True).start()

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        self._scroll.handle_key(key)

    def on_click(self, _ctx: RenderContext, x: float, y: float, _button: str) -> None:
        for tidx, x1, y1, x2, y2 in self._copy_hitboxes:
            if x1 <= x <= x2 and y1 <= y <= y2:
                if tidx < len(self._turns):
                    self.emit.copy_to_clipboard(self._turns[tidx].text)
                    self.emit.info(f"chat: copied turn {tidx}")
                    self._copy_flashing = tidx
                    self.emit.schedule_render(after_ms=10)

                    def _clear_flash(idx: int = tidx) -> None:
                        if self._copy_flashing == idx:
                            self._copy_flashing = -1
                            self.emit.schedule_render(after_ms=10)

                    threading.Timer(1.5, _clear_flash).start()
                return

    def _build_history(self) -> Column:
        children: list = []
        for turn in self._turns:
            children.append(ChatBubble(turn.text, role=turn.role))
            children.append(Spacer(size=6.0))
        if self._in_flight:
            children.append(Label("thinking…", tone="hint"))
        if not children:
            children.append(Label("Type a message below to start chatting.", tone="hint"))
        return Column(children, padding=0, gap=0)

    def _update_copy_hitboxes(self, content_w: float, scroll_top_y: float) -> None:
        """Recompute copy-button rects for the current scroll state."""
        self._copy_hitboxes = []
        cursor = _HIST_PAD_TOP  # history column pad_top (SPACE_SM, padding=0 default)
        for i, turn in enumerate(self._turns):
            bh = ChatBubble(turn.text, role=turn.role).measure(content_w)
            if turn.role == "assistant":
                abs_y = scroll_top_y - self._scroll.scroll_offset + cursor
                btn_x = _COL_PAD + content_w - _COPY_BTN_W
                btn_y = abs_y + bh - _COPY_BTN_H - 3
                self._copy_hitboxes.append((i, btn_x, btn_y, btn_x + _COPY_BTN_W, btn_y + _COPY_BTN_H))
            cursor += bh + 6.0  # ChatBubble + Spacer(size=6.0)

    def on_render(self, ctx: RenderContext) -> None:
        self._scroll.child = self._build_history()
        self._input.placeholder = (
            "Waiting for response…" if self._in_flight
            else "Type a message… (Shift+Enter for newline)"
        )
        self._maybe_scroll_to_bottom()
        ctx.render(Column([
            AppBar(title="Chat"),
            self._scroll,
            Divider(),
            self._input,
        ]))

        if self._input.submitted is not None and not self._in_flight:
            self._send(self._input.submitted)

        # ── Post-render overlays ─────────────────────────────────────────────
        # Content width inside the scrollable (excl. scrollbar gutter).
        content_w = ctx.w - 2 * _COL_PAD - _SCROLLBAR_W
        # Y of the scrollable's top edge (below AppBar + gaps).
        scroll_top_y = _COL_PAD_TOP + _APPBAR_H + _COL_GAP
        # Height of the scrollable region.
        scroll_h = (ctx.h - _COL_PAD_TOP - _COL_PAD
                    - _APPBAR_H - _DIVIDER_H - self._input.height
                    - 3 * _COL_GAP)

        # Copy buttons — one per assistant bubble, clipped to scroll area.
        self._update_copy_hitboxes(content_w, scroll_top_y)
        if self._copy_hitboxes:
            ctx.push_clip(_COL_PAD, scroll_top_y, content_w + _SCROLLBAR_W, scroll_h)
            for tidx, x1, y1, _x2, _y2 in self._copy_hitboxes:
                if scroll_top_y <= y1 <= scroll_top_y + scroll_h:
                    flashing = tidx == self._copy_flashing
                    color = "#a6e3a1" if flashing else "#45475a"
                    label = "✓" if flashing else "copy"
                    ctx.text(x1, y1, label, size=10.0, color=color, monospace=True)
            ctx.pop_clip()

        # Disabled-input overlay while waiting for a response.
        if self._in_flight:
            input_y = ctx.h - _COL_PAD - self._input.height
            ctx.rect(_COL_PAD, input_y, ctx.w - 2 * _COL_PAD, self._input.height,
                     fill="#1e1e2e99", radius=4.0)


ChatApp().run()
