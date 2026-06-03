#!/usr/bin/env python3
"""Kanban — keyboard-driven card board."""
from plexi_sdk import App, RenderContext
from plexi_sdk.ui import (
    AppBar, FooterKeys,
    TEXT_CAPTION, TEXT_BODY,
)

# ── Palette ───────────────────────────────────────────────────────────────────
APP_BG      = "#0d0d12"
COL_BG      = "#14141c"
COL_ACTIVE  = "#18182a"
DIVIDER     = "#252535"
CARD_BG     = "#1c1c28"
CARD_FOCUS  = "#21213a"
CARD_SEL    = "#282850"
TEXT        = "#dcdcf0"
TEXT_MID    = "#8888a8"
TEXT_DIM    = "#505068"
ACCENT      = "#7b9ef0"
TAG_BG      = "#22224a"
SEL_BAR     = "#5068d8"

BADGE_FILLS = ["#2e3060", "#1e3a70", "#1a4a36"]
BADGE_TEXT  = "#aab0e0"

# ── Sizes ─────────────────────────────────────────────────────────────────────
COL_GAP  = 8
COL_PAD  = 10
COL_R    = 10.0
CARD_H   = 68
CARD_GAP = 5
CARD_X   = 10
CARD_Y   = 44


class KanbanCard:
    def __init__(self, cid: int, title: str, tag: str = ""):
        self.id    = cid
        self.title = title
        self.tag   = tag


class Kanban(App):
    async def on_init(self, ctx: RenderContext) -> None:
        self.columns: list[tuple[str, list[KanbanCard]]] = [
            ("Todo", [
                KanbanCard(1, "Write SDK docs",        "docs"),
                KanbanCard(2, "Add dark mode support", "feat"),
                KanbanCard(3, "Audit accessibility",   "a11y"),
                KanbanCard(4, "Refactor render loop",  "perf"),
            ]),
            ("In Progress", [
                KanbanCard(5, "Kanban drag & drop",    "feat"),
                KanbanCard(6, "Bootstrap scaffold",    "infra"),
                KanbanCard(7, "Notify API",            "feat"),
            ]),
            ("Done", [
                KanbanCard(8, "Design system tokens",  "design"),
                KanbanCard(9, "Hot reload watcher",    "infra"),
            ]),
        ]
        self._col  = 0
        self._card = 0
        self._clamp()

        # L1 chrome components — created once in on_init as required for stateful widgets
        total = sum(len(c[1]) for c in self.columns)
        self._appbar = AppBar(title="Kanban", subtitle=f"{total} cards")
        self._footer = FooterKeys([
            ("↑↓",  "navigate"),
            ("←→",  "column"),
            ("⇧↑↓", "reorder"),
            ("⇧←→", "move"),
        ])

        self._update_status(ctx)
        self.emit.info("Kanban initialized")

    # ── helpers ───────────────────────────────────────────────────────────────

    def _clamp(self) -> None:
        self._col = max(0, min(self._col, len(self.columns) - 1))
        cards = self.columns[self._col][1]
        self._card = max(0, min(self._card, len(cards) - 1)) if cards else 0

    def _col_rect(self, ctx: RenderContext, i: int) -> tuple[float, float, float, float]:
        n = len(self.columns)
        w = (ctx.w - COL_PAD * 2 - COL_GAP * (n - 1)) / n
        x = COL_PAD + i * (w + COL_GAP)
        # top = AppBar height, bottom = FooterKeys height (approximated to 44px)
        appbar_h = self._appbar.measure(ctx.w)
        footer_h = self._footer.measure(ctx.w)
        return x, appbar_h, w, ctx.h - appbar_h - footer_h

    def _update_status(self, ctx: RenderContext) -> None:
        col_name, cards = self.columns[self._col]
        title = cards[self._card].title if cards else ""
        ctx.status_summary(f"{col_name} · {title}" if title else col_name)

    def _rebuild_chrome(self) -> None:
        """Refresh AppBar subtitle after card moves change the total."""
        total = sum(len(c[1]) for c in self.columns)
        self._appbar = AppBar(title="Kanban", subtitle=f"{total} cards")

    # ── render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        # Clear background
        ctx.clear(APP_BG)

        # Measure chrome heights so column layout is consistent
        appbar_h = self._appbar.measure(ctx.w)
        footer_h = self._footer.measure(ctx.w)

        # Render AppBar via L1
        self._appbar.render(ctx, 0.0, 0.0, ctx.w, appbar_h)

        # Render FooterKeys via L1 at bottom
        footer_y = ctx.h - footer_h
        self._footer.render(ctx, 0.0, footer_y, ctx.w, footer_h)

        # Kanban columns — pixel layout (no horizontal container in L1)
        for i, (col_name, cards) in enumerate(self.columns):
            cx, cy, cw, ch = self._col_rect(ctx, i)
            focused = (i == self._col)

            ctx.rect(cx, cy, cw, ch, fill=COL_ACTIVE if focused else COL_BG, radius=COL_R)

            # column header
            hdr_bg = "#1e1e38" if focused else "#161622"
            ctx.rect(cx, cy, cw, 36, fill=hdr_bg, radius=COL_R)
            ctx.rect(cx, cy + 28, cw, 8, fill=hdr_bg)  # square off bottom corners
            ctx.rect(cx, cy + 35, cw, 1, fill=DIVIDER)
            ctx.text(cx + CARD_X, cy + 18, col_name, size=TEXT_BODY, color=ACCENT if focused else TEXT_MID,
                     bold=True, align="left_center")

            # count badge
            bw, bh = 22, 16
            bx = cx + cw - CARD_X - bw
            by = cy + 10
            ctx.rect(bx, by, bw, bh, fill=BADGE_FILLS[i % len(BADGE_FILLS)], radius=4.0)
            ctx.text(bx + bw / 2, by + bh / 2, str(len(cards)), size=TEXT_CAPTION, color=BADGE_TEXT, bold=True, align="center")

            # cards
            for idx, card in enumerate(cards):
                kx = cx + CARD_X
                ky = cy + CARD_Y + idx * (CARD_H + CARD_GAP)
                kw = cw - CARD_X * 2
                kh = CARD_H
                is_sel = focused and idx == self._card

                bg = CARD_SEL if is_sel else (CARD_FOCUS if focused else CARD_BG)
                ctx.rect(kx, ky, kw, kh, fill=bg, radius=6.0)

                if is_sel:
                    ctx.rect(kx, ky + 4, 3, kh - 8, fill=SEL_BAR, radius=2.0)

                text_x = kx + (14 if is_sel else 10)
                ctx.text(text_x, ky + 14, card.title, size=TEXT_BODY, color=TEXT if is_sel else TEXT_MID,
                         max_width=kw - (14 if is_sel else 10) - 6)

                if card.tag:
                    tw = len(card.tag) * 6 + 10
                    tx = kx + (14 if is_sel else 10)
                    ctx.rect(tx, ky + 44, tw, 14, fill=TAG_BG, radius=3.0)
                    ctx.text(tx + tw / 2, ky + 51, card.tag, size=TEXT_CAPTION,
                             color=ACCENT if is_sel else TEXT_DIM, align="center")

    # ── keyboard ──────────────────────────────────────────────────────────────

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        shift = mods.get("shift", False)
        _, cards = self.columns[self._col]

        if key in ("up", "k", "K"):
            moving = shift or key == "K"
            if moving and cards and self._card > 0:
                c = self._card
                cards[c], cards[c - 1] = cards[c - 1], cards[c]
                self._card -= 1
            elif not moving and cards:
                self._card = max(0, self._card - 1)

        elif key in ("down", "j", "J"):
            moving = shift or key == "J"
            if moving and cards and self._card < len(cards) - 1:
                c = self._card
                cards[c], cards[c + 1] = cards[c + 1], cards[c]
                self._card += 1
            elif not moving and cards:
                self._card = min(len(cards) - 1, self._card + 1)

        elif key in ("left", "h", "H"):
            moving = shift or key == "H"
            if moving and cards and self._col > 0:
                card = cards.pop(self._card)
                self._col -= 1
                dest = self.columns[self._col][1]
                dest.append(card)
                self._card = len(dest) - 1
                self._rebuild_chrome()
            elif not moving:
                self._col = max(0, self._col - 1)
                self._clamp()

        elif key in ("right", "l", "L"):
            moving = shift or key == "L"
            if moving and cards and self._col < len(self.columns) - 1:
                card = cards.pop(self._card)
                self._col += 1
                dest = self.columns[self._col][1]
                dest.append(card)
                self._card = len(dest) - 1
                self._rebuild_chrome()
            elif not moving:
                self._col = min(len(self.columns) - 1, self._col + 1)
                self._clamp()

        self._update_status(ctx)


Kanban().run()
