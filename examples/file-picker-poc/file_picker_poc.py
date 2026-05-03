#!/usr/bin/env python3
"""File Picker POC — demonstrates the fs.pick capability (#514).

Shows a button to open a native macOS file picker. The selected path
is displayed in the pane after the dialog closes.
"""

from plexi_sdk import App, RenderContext, FG, MUTED, SURFACE, ACCENT, BG, BODY, CAPTION  # type: ignore[attr-defined]

import uuid

BUTTON_X = 20.0
BUTTON_Y = 80.0   # pad(20) + title(32) + subtitle(28)
BUTTON_W = 200.0
BUTTON_H = 36.0


class FilePickerApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._selected_paths: list[str] = []
        self._status = "No file selected."
        self._pending = False

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)
        w, h = ctx.w, ctx.h
        pad = 20.0
        y = pad

        ctx.text(pad, y, "File Picker POC", size=16.0, color=FG, bold=True)
        y += 32.0

        ctx.text(pad, y, "Click the button to open a native macOS file picker.", size=BODY, color=MUTED)
        y += 28.0

        btn_fill = MUTED if self._pending else ACCENT
        ctx.rect(BUTTON_X, BUTTON_Y, BUTTON_W, BUTTON_H, fill=btn_fill, radius=6.0)
        label = "Picking…" if self._pending else "Open File Picker"
        ctx.text(BUTTON_X + 12, BUTTON_Y + 10, label, size=BODY, color=FG)

        y = BUTTON_Y + BUTTON_H + 20.0
        ctx.rect(pad, y, w - pad * 2, 1, fill=SURFACE, radius=0.0)
        y += 12.0

        ctx.text(pad, y, "Result:", size=CAPTION, color=MUTED)
        y += 20.0
        ctx.text(pad, y, self._status, size=BODY, color=FG)

        if self._selected_paths:
            y += 22.0
            for path in self._selected_paths:
                ctx.text(pad + 8, y, path, size=CAPTION, color=ACCENT, monospace=True)
                y += 18.0

    def on_click(self, ctx: RenderContext, x: float, y: float, button: str) -> None:
        if button != "primary" or self._pending:
            return
        if BUTTON_X <= x <= BUTTON_X + BUTTON_W and BUTTON_Y <= y <= BUTTON_Y + BUTTON_H:
            self._pending = True
            self._status = "Waiting for picker…"
            ctx.emit.open_file_picker(str(uuid.uuid4()), filter=[], multiple=False)

    def on_file_picked(self, ctx: RenderContext, request_id: str, paths: list[str]) -> None:
        self._pending = False
        self._selected_paths = paths
        self._status = f"Selected {len(paths)} file(s):"

    def on_file_pick_cancelled(self, ctx: RenderContext, request_id: str) -> None:
        self._pending = False
        self._status = "Picker cancelled."
        self._selected_paths = []


if __name__ == "__main__":
    FilePickerApp().run()
