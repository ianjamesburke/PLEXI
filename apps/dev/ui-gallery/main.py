"""UI Gallery — reference implementation of every SDK component.

Renders all 9 text align values on a grid with crosshair markers,
plus a complete set of UI components in a scrollable view.
"""
from plexi_sdk import App, BODY, CAPTION, HINT
from plexi_sdk.ui import (
    AppBar, Heading, Label, Spacer, Divider,
    Column, Canvas, FooterKeys,
)

_ALIGNS = [
    ("left_top",      0, 0),
    ("center_top",    1, 0),
    ("right_top",     2, 0),
    ("left_center",   0, 1),
    ("center_center", 1, 1),
    ("right_center",  2, 1),
    ("left_bottom",   0, 2),
    ("center_bottom", 1, 2),
    ("right_bottom",  2, 2),
]


def draw_align_grid(ctx, x, y, w, h):
    cell_w = w / 3
    cell_h = h / 3
    for align, col, row in _ALIGNS:
        cx = x + (col + 0.5) * cell_w
        cy = y + (row + 0.5) * cell_h
        # Grid cell border
        ctx.rect(x + col * cell_w, y + row * cell_h, cell_w, cell_h,
                 "#1e1e2e", radius=0.0)
        ctx.rect(x + col * cell_w, y + row * cell_h, cell_w, 1.0, "#313244")
        ctx.rect(x + col * cell_w, y + row * cell_h, 1.0, cell_h, "#313244")
        # Crosshair
        ctx.line(cx - 6, cy, cx + 6, cy, "#585b70", width=1.0)
        ctx.line(cx, cy - 6, cx, cy + 6, "#585b70", width=1.0)
        # Anchored label
        ctx.text(cx, cy, align, size=HINT, color=ctx.theme.fg, align=align)


class UIGallery(App):
    def on_render(self, ctx):
        self.emit.info("gallery render")
        root = Column(children=[
            AppBar(title="UI Gallery", subtitle="SDK component reference"),
            Heading("Text Align — 9 Anchors", level=2),
            Label("Each label is anchored exactly at the crosshair. "
                  "All 9 egui Align2 values are covered."),
            Canvas(draw=draw_align_grid, grow=False, height=240.0),
            Divider(),
            Heading("Typography", level=2),
            Heading("Heading level 1", level=1),
            Heading("Heading level 2", level=2),
            Heading("Heading level 3", level=3),
            Label("Label — body tone (default)", tone="body"),
            Label("Label — caption tone", tone="caption"),
            Label("Label — hint tone (muted)", tone="hint"),
            Spacer(size=8.0),
            Divider(),
            Heading("Footer", level=2),
            FooterKeys(shortcuts=[
                ("q", "quit"),
                (["j", "k"], "scroll"),
            ]),
        ])
        root.render_into(ctx, 0, 0, ctx.w)


UIGallery().run()
