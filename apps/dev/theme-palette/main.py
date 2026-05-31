#!/usr/bin/env python3
"""Theme Palette POC — demonstrates AppPalette and ctx.theme.is_dark (#1636)."""
from plexi_sdk import App, AppPalette, RenderContext
from plexi_sdk._constants import BODY, CAPTION, HINT, PAD

# App-defined palette — tokens auto-switch with the host theme.
PALETTE = AppPalette(
    dark={
        "card":    "#1e1e2e",
        "card_hi": "#313244",
        "label":   "#cdd6f4",
        "sub":     "#6c7086",
        "accent":  "#7aa2f7",
        "ok":      "#a6e3a1",
        "warn":    "#f9e2af",
    },
    light={
        "card":    "#e6e9ef",
        "card_hi": "#ccd0da",
        "label":   "#4c4f69",
        "sub":     "#8c8fa1",
        "accent":  "#1e66f5",
        "ok":      "#40a02b",
        "warn":    "#df8e1d",
    },
)

SWATCH_W  = 80.0
SWATCH_H  = 56.0
SWATCH_GAP = 8.0
ROW_PAD   = 16.0


class ThemePaletteApp(App):
    async def on_init(self, ctx: RenderContext) -> None:
        self.emit.info(f"theme-palette ready; is_dark={ctx.theme.is_dark}")
        ctx.status_summary("Theme Palette POC")

    def on_render(self, ctx: RenderContext) -> None:
        c = PALETTE.resolve(ctx.theme)
        t = ctx.theme

        ctx.clear(t.bg)

        # Header
        mode = "dark" if t.is_dark else "light"
        ctx.text(PAD, PAD, "Theme Palette POC", size=BODY, color=t.fg, bold=True)
        ctx.text(PAD, PAD + 22, f"ctx.theme.is_dark = {t.is_dark}  ({mode} mode)", size=CAPTION, color=t.muted)
        ctx.rect(PAD, PAD + 40, ctx.w - PAD * 2, 1.0, t.highlight)

        # Host theme swatches
        y = PAD + 56
        ctx.text(PAD, y, "Host theme roles  (ctx.theme.*)", size=HINT, color=t.muted, bold=True)
        y += 18

        host_swatches = [
            ("bg",        t.bg),
            ("surface",   t.surface),
            ("fg",        t.fg),
            ("accent",    t.accent),
            ("danger",    t.danger),
            ("success",   t.success),
        ]
        x = PAD
        for label, color in host_swatches:
            ctx.rect(x, y, SWATCH_W, SWATCH_H, color, radius=6.0)
            ctx.text(x + 4, y + SWATCH_H - 14, label, size=9.0, color=t.bg if _is_dark_color(color) else t.fg)
            x += SWATCH_W + SWATCH_GAP

        # App palette swatches
        y += SWATCH_H + 24
        ctx.text(PAD, y, "App palette  (PALETTE.resolve(ctx.theme))", size=HINT, color=t.muted, bold=True)
        y += 18

        app_swatches = [
            ("card",    c["card"]),
            ("card_hi", c["card_hi"]),
            ("label",   c["label"]),
            ("accent",  c["accent"]),
            ("ok",      c["ok"]),
            ("warn",    c["warn"]),
        ]
        x = PAD
        for label, color in app_swatches:
            ctx.rect(x, y, SWATCH_W, SWATCH_H, color, radius=6.0)
            ctx.text(x + 4, y + SWATCH_H - 14, label, size=9.0, color=t.bg if _is_dark_color(color) else "#1e1e2e")
            x += SWATCH_W + SWATCH_GAP

        # Sample card using palette
        y += SWATCH_H + 24
        ctx.text(PAD, y, "Sample card using palette tokens", size=HINT, color=t.muted, bold=True)
        y += 18
        card_w = ctx.w - PAD * 2
        ctx.rect(PAD, y, card_w, 72, c["card"], radius=8.0)
        ctx.rect(PAD + 12, y + 14, 4, 44, c["accent"], radius=2.0)
        ctx.text(PAD + 24, y + 14, "AppPalette token: card", size=CAPTION, color=c["label"], bold=True)
        ctx.text(PAD + 24, y + 33, f"Switches automatically with host theme — no if/else needed.", size=HINT, color=c["sub"])
        ctx.text(PAD + 24, y + 50, f"Current mode: {mode}", size=HINT, color=c["accent"])

        # Footer
        ctx.text(PAD, ctx.h - 20, "Switch themes in Plexi config to see colors update.", size=HINT, color=t.muted)


def _is_dark_color(hex_color: str) -> bool:
    h = hex_color.lstrip("#")[:6]
    if len(h) < 6:
        return True
    r, g, b = int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16)
    return (0.299 * r + 0.587 * g + 0.114 * b) / 255.0 < 0.5


ThemePaletteApp().run()
