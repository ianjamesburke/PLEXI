from __future__ import annotations
from dataclasses import dataclass
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from plexi_sdk._render_context import RenderContext

_FG = "#cdd6f4"
_BG = "#313244"
_HOVER = "#45475a"
_ACTIVE = "#585b70"


@dataclass
class ButtonStyle:
    """Visual style for Button widgets — fill, hover, active colors and font size."""
    fill: str = _BG
    hover_fill: str = _HOVER
    active_fill: str = _ACTIVE
    text_color: str = _FG
    font_size: float = 13.0
    radius: float = 6.0


class Button:
    """Stateful button widget. Tracks its own hover/click state across frames.

    Usage::

        btn = Button("ok", x=20, y=40, w=80, h=32, label="OK")

        def on_render(self, ctx):
            if btn.render(ctx):
                handle_ok()
    """

    def __init__(self, id: str, x: float, y: float, w: float, h: float,
                 label: str, style: "ButtonStyle | None" = None) -> None:
        self.id = id
        self.x = x
        self.y = y
        self.w = w
        self.h = h
        self.label = label
        self.style = style or ButtonStyle()

    def render(self, ctx: "RenderContext") -> bool:
        """Draw the button and return True if clicked this frame."""
        s = self.style
        return ctx.button(
            id=self.id,
            x=self.x, y=self.y, w=self.w, h=self.h,
            label=self.label,
            fill=s.fill,
            hover_fill=s.hover_fill,
            active_fill=s.active_fill,
            text_color=s.text_color,
            font_size=s.font_size,
            radius=s.radius,
        )
