#!/usr/bin/env python3
"""CSV Viewer — browse and inspect CSV files in the launch directory."""

import csv
import sys
from pathlib import Path

from plexi_sdk import (  # type: ignore[attr-defined]
    App, RenderContext,
    FG, MUTED, ACCENT, BG,
    BODY, CAPTION,
)

PAD = 16.0
ROW_H = 24.0
LIST_ITEM_H = 36.0
COL_W = 130.0
CELL_PAD = 8.0
STRIPE = "#0d0d0d"
HEADER_BG = "#1a1a2e"


class CsvViewer(App):
    def on_init(self, ctx: RenderContext) -> None:
        launch_dir = Path(ctx.workspace_root) if ctx.workspace_root else Path.cwd()
        self._files: list[Path] = sorted(
            launch_dir.glob("*.csv"), key=lambda p: p.name.lower()
        )
        self._dir = launch_dir
        self._selected = 0
        self._mode = "list"
        self._headers: list[str] = []
        self._rows: list[list[str]] = []
        self._v_scroll = 0
        self._h_scroll = 0
        # Cache file sizes once at init to avoid stat() calls inside on_render
        self._file_hints: dict[Path, str] = {}
        for p in self._files:
            try:
                kb = p.stat().st_size / 1024
                self._file_hints[p] = f"{kb:.1f} KB" if kb < 1024 else f"{kb / 1024:.1f} MB"
            except OSError:
                self._file_hints[p] = ""
        ctx.info(f"csv_viewer: {len(self._files)} CSV files in {launch_dir}")

    def on_inject(self, _ctx: RenderContext, payload: dict) -> None:
        if "mode" in payload:
            self._mode = payload["mode"]
        if "headers" in payload:
            self._headers = list(payload["headers"])
        if "rows" in payload:
            self._rows = [list(r) for r in payload["rows"]]
        if "selected" in payload:
            self._selected = int(payload["selected"])

    def on_key(self, ctx: RenderContext, key: str, _mods: dict) -> None:
        if self._mode == "list":
            if key in ("up", "k"):
                self._selected = max(0, self._selected - 1)
            elif key in ("down", "j"):
                if self._files:
                    self._selected = min(len(self._files) - 1, self._selected + 1)
            elif key == "Enter":
                if self._files:
                    self._load_csv(self._files[self._selected])
                    self._mode = "detail"
                    ctx.info(f"csv_viewer: opened {self._files[self._selected].name}")
            elif key == "Escape":
                ctx.info("csv_viewer: exit via Escape")
                sys.exit(0)
        else:
            if key in ("up", "k"):
                self._v_scroll = max(0, self._v_scroll - 1)
            elif key in ("down", "j"):
                self._v_scroll += 1  # clamped to viewport in _draw_detail
            elif key in ("left", "h"):
                self._h_scroll = max(0, self._h_scroll - 1)
            elif key in ("right", "l"):
                max_h = max(0, len(self._headers) - 1)
                self._h_scroll = min(max_h, self._h_scroll + 1)
            elif key == "Escape":
                self._mode = "list"
                ctx.info("csv_viewer: back to list")

    def _load_csv(self, path: Path) -> None:
        self._headers = []
        self._rows = []
        self._v_scroll = 0
        self._h_scroll = 0
        try:
            with open(path, newline="", errors="replace") as f:
                reader = csv.reader(f)
                all_rows = list(reader)
            if all_rows:
                self._headers = all_rows[0]
                self._rows = all_rows[1:]
        except OSError as e:
            self._headers = [f"Error: {e}"]

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)
        if self._mode == "list":
            self._draw_list(ctx)
        else:
            self._draw_detail(ctx)

    def _draw_list(self, ctx: RenderContext) -> None:
        w, h = ctx.w, ctx.h
        y = PAD

        ctx.text(PAD, y, str(self._dir), size=CAPTION, color=MUTED, monospace=True)
        y += 22.0

        if not self._files:
            ctx.text(PAD, y, "No CSV files found.", size=BODY, color=MUTED)
            ctx.text(PAD, h - 20, "esc  exit", size=CAPTION, color=MUTED)
            return

        label = f"{len(self._files)} CSV file{'s' if len(self._files) != 1 else ''}"
        ctx.text(PAD, y, label, size=BODY, color=FG)
        y += ROW_H + 4

        items = [
            {"label": p.name, "secondary": self._file_hints.get(p, "") or None}
            for p in self._files
        ]
        ctx.list_view(items, selected=self._selected, item_height=LIST_ITEM_H,
                      x=0, y=y, w=w, h=h - y - 30)

        ctx.text(PAD, h - 20, "↑↓ / jk  navigate   ↵  open   esc  exit", size=CAPTION, color=MUTED)

    def _draw_detail(self, ctx: RenderContext) -> None:
        w, h = ctx.w, ctx.h
        if not self._files:
            return
        path = self._files[self._selected]
        y = PAD

        ctx.text(PAD, y, path.name, size=BODY, color=FG, bold=True)
        y += 22.0
        ctx.text(PAD, y, f"{len(self._rows)} rows × {len(self._headers)} columns", size=CAPTION, color=MUTED)
        y += 22.0

        visible_cols = max(1, int((w - PAD) // COL_W))
        col_start = self._h_scroll
        col_end = min(len(self._headers), col_start + visible_cols)

        # Header row
        ctx.rect(0, y - 2, w, ROW_H, fill=HEADER_BG, radius=0.0)
        for ci, col_idx in enumerate(range(col_start, col_end)):
            x = PAD + ci * COL_W
            label = self._headers[col_idx] if col_idx < len(self._headers) else ""
            ctx.text(x + CELL_PAD, y + 4, label, size=CAPTION, color=ACCENT, bold=True,
                     max_width=COL_W - CELL_PAD * 2)
        y += ROW_H + 2

        # Data rows
        footer_h = 28.0
        visible_rows = max(1, int((h - y - footer_h) // ROW_H))
        max_v = max(0, len(self._rows) - visible_rows)
        if self._v_scroll > max_v:
            self._v_scroll = max_v
        row_start = self._v_scroll
        row_end = min(len(self._rows), row_start + visible_rows)

        for ri, row_idx in enumerate(range(row_start, row_end)):
            row_y = y + ri * ROW_H
            if ri % 2 == 1:
                ctx.rect(0, row_y - 2, w, ROW_H, fill=STRIPE, radius=0.0)
            row = self._rows[row_idx]
            for ci, col_idx in enumerate(range(col_start, col_end)):
                x = PAD + ci * COL_W
                cell = row[col_idx] if col_idx < len(row) else ""
                ctx.text(x + CELL_PAD, row_y + 4, cell, size=CAPTION, color=FG,
                         max_width=COL_W - CELL_PAD * 2)

        # Footer
        if self._rows:
            pct = min(100, int(100 * row_end / max(1, len(self._rows))))
            info = f"row {row_start + 1}–{row_end} of {len(self._rows)}  ({pct}%)"
        else:
            info = "empty"
        ctx.text(PAD, h - 20, f"{info}   ↑↓/jk scroll   ←→/hl columns   esc  back", size=CAPTION, color=MUTED)


if __name__ == "__main__":
    CsvViewer().run()
