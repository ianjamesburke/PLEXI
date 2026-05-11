#!/usr/bin/env python3
"""Kanban — keyboard-driven card board."""
from plexi_sdk import App, RenderContext, CAPTION, BODY, TITLE

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
HEADER_H = 44
STATUS_H = 30
COL_GAP  = 8
COL_PAD  = 10
COL_R    = 10.0
CARD_H   = 68
CARD_GAP = 5
CARD_X   = 10
CARD_Y   = 44


class Card:
    def __init__(self, cid: int, title: str, tag: str = ""):
        self.id    = cid
        self.title = title
        self.tag   = tag


class Kanban(App):
    async def on_init(self, ctx: RenderContext) -> None:
        self.columns: list[tuple[str, list[Card]]] = [
            ("Todo", [
                Card(1, "Write SDK docs",        "docs"),
                Card(2, "Add dark mode support", "feat"),
                Card(3, "Audit accessibility",   "a11y"),
                Card(4, "Refactor render loop",  "perf"),
            ]),
            ("In Progress", [
                Card(5, "Kanban drag & drop",    "feat"),
                Card(6, "Bootstrap scaffold",    "infra"),
                Card(7, "Notify API",            "feat"),
            ]),
            ("Done", [
                Card(8, "Design system tokens",  "design"),
                Card(9, "Hot reload watcher",    "infra"),
            ]),
        ]
        self._col  = 0
        self._card = 0
        self._clamp()
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
        return x, float(HEADER_H), w, ctx.h - HEADER_H - STATUS_H

    def _update_status(self, ctx: RenderContext) -> None:
        col_name, cards = self.columns[self._col]
        title = cards[self._card].title if cards else ""
        ctx.status_summary(f"{col_name} · {title}" if title else col_name)

    # ── render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(APP_BG)

        # header
        ctx.rect(0, 0, ctx.w, HEADER_H, fill=COL_BG)
        ctx.rect(0, HEADER_H - 1, ctx.w, 1, fill=DIVIDER)
        ctx.text(COL_PAD + 2, HEADER_H / 2, "Kanban", size=TITLE, color=TEXT, bold=True, align="left_center")
        total = sum(len(c[1]) for c in self.columns)
        ctx.text(ctx.w - COL_PAD, HEADER_H / 2, f"{total} cards", size=CAPTION, color=TEXT_DIM, align="right_center")

        for i, (col_name, cards) in enumerate(self.columns):
            cx, cy, cw, ch = self._col_rect(ctx, i)
            focused = (i == self._col)

            ctx.rect(cx, cy, cw, ch, fill=COL_ACTIVE if focused else COL_BG, radius=COL_R)

            # column header
            hdr_bg = "#1e1e38" if focused else "#161622"
            ctx.rect(cx, cy, cw, 36, fill=hdr_bg, radius=COL_R)
            ctx.rect(cx, cy + 28, cw, 8, fill=hdr_bg)  # square off bottom corners
            ctx.rect(cx, cy + 35, cw, 1, fill=DIVIDER)
            ctx.text(cx + CARD_X, cy + 18, col_name, size=BODY, color=ACCENT if focused else TEXT_MID,
                     bold=True, align="left_center")

            # count badge — align="center" fixes the 3/4 centering issue
            bw, bh = 22, 16
            bx = cx + cw - CARD_X - bw
            by = cy + 10
            ctx.rect(bx, by, bw, bh, fill=BADGE_FILLS[i % len(BADGE_FILLS)], radius=4.0)
            ctx.text(bx + bw / 2, by + bh / 2, str(len(cards)), size=CAPTION, color=BADGE_TEXT, bold=True, align="center")

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
                ctx.text(text_x, ky + 14, card.title, size=BODY, color=TEXT if is_sel else TEXT_MID,
                         max_width=kw - (14 if is_sel else 10) - 6)

                if card.tag:
                    tw = len(card.tag) * 6 + 10
                    tx = kx + (14 if is_sel else 10)
                    ctx.rect(tx, ky + 44, tw, 14, fill=TAG_BG, radius=3.0)
                    ctx.text(tx + tw / 2, ky + 51, card.tag, size=CAPTION,
                             color=ACCENT if is_sel else TEXT_DIM, align="center")

        # status bar
        ctx.rect(0, ctx.h - STATUS_H, ctx.w, STATUS_H, fill=COL_BG)
        ctx.rect(0, ctx.h - STATUS_H, ctx.w, 1, fill=DIVIDER)
        ctx.text(ctx.w / 2, ctx.h - STATUS_H / 2,
                 "↑↓  navigate   ←→  column   ⇧↑↓  reorder   ⇧←→  move",
                 size=CAPTION, color=TEXT_DIM, align="center")

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
            elif not moving:
                self._col = min(len(self.columns) - 1, self._col + 1)
                self._clamp()

        self._update_status(ctx)


Kanban().run()
