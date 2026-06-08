#!/usr/bin/env python3
"""CSV Viewer — browse and inspect CSV files in the launch directory."""

import csv
from pathlib import Path

from plexi_sdk import App, Arg
from plexi_sdk.ui import (
    AppBar, Column, FooterKeys, Label, Section,
    SelectList, Spacer,
)

COL_W = 130.0
CELL_PAD = 8.0
ROW_H = 24.0


class CsvViewer(App):
    file: Arg[str | None] = Arg(positional=True, default=None)

    def on_init(self) -> None:
        launch_dir = Path(self.workspace_root) if self.workspace_root else Path.cwd()

        # If a file path was passed as a launch argument, open it directly
        if self.file:
            target = Path(self.file)
            if not target.is_absolute():
                target = launch_dir / target
            if target.is_file():
                self._files: list[Path] = [target]
                self._dir = target.parent
                self._selected = 0
                self._mode = "detail"
                self._headers: list[str] = []
                self._rows: list[list[str]] = []
                self._v_scroll = 0
                self._h_scroll = 0
                self._file_hints: dict[Path, str] = {}
                self._file_list = SelectList([{"name": target.name}])
                self._load_csv(target)
                self.emit.info(f"csv_viewer: opened {target} via launch arg")
                return

        self._files = sorted(
            launch_dir.glob("*.csv"), key=lambda p: p.name.lower()
        )
        self._dir = launch_dir
        self._selected = 0
        self._mode = "list"
        self._headers = []
        self._rows = []
        self._v_scroll = 0
        self._h_scroll = 0
        # Cache file sizes once at init to avoid stat() calls inside on_render
        self._file_hints = {}
        for p in self._files:
            try:
                kb = p.stat().st_size / 1024
                self._file_hints[p] = f"{kb:.1f} KB" if kb < 1024 else f"{kb / 1024:.1f} MB"
            except OSError:
                self._file_hints[p] = ""
        self._file_list = SelectList(
            [{"name": p.name, "description": self._file_hints.get(p) or None} for p in self._files]
        )
        self.emit.info(f"csv_viewer: {len(self._files)} CSV files in {launch_dir}")

    def on_inject(self, payload: dict) -> None:
        if "mode" in payload:
            self._mode = payload["mode"]
        if "headers" in payload:
            self._headers = list(payload["headers"])
        if "rows" in payload:
            self._rows = [list(r) for r in payload["rows"]]
        if "selected" in payload:
            self._selected = max(0, min(len(self._files) - 1, int(payload["selected"]))) if self._files else 0
            self._file_list.selected_idx = self._selected

    def on_escape(self):
        if self._mode == "detail":
            self._mode = "list"
            return True
        return False

    def on_key(self, key: str, _mods: dict) -> None:
        if self._mode == "list":
            if key in ("up", "k", "down", "j"):
                self._file_list.handle_key(key)
                self._selected = self._file_list.selected_idx
            elif key == "return":
                if self._files:
                    self._load_csv(self._files[self._selected])
                    self._mode = "detail"
                    self.emit.info(f"csv_viewer: opened {self._files[self._selected].name}")
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

    def on_render(self, ctx) -> None:
        if self._mode == "list":
            self._draw_list(ctx)
        else:
            self._draw_detail(ctx)

    def _draw_list(self, ctx) -> None:
        n = len(self._files)
        label = f"{n} CSV file{'s' if n != 1 else ''}" if self._files else "No CSV files found."
        subtitle = str(self._dir)

        if not self._files:
            ctx.render(Column([
                AppBar("CSV Viewer", subtitle=subtitle),
                Label(label, tone="hint"),
                Spacer(grow=True),
                FooterKeys([("esc", "exit")]),
            ], padding=0.0, padding_top=0, gap=0))
            return

        self._file_list.selected_idx = self._selected

        ctx.render(Column([
            AppBar("CSV Viewer", subtitle=subtitle),
            Section(label),
            self._file_list,
            FooterKeys([
                (["↑", "k"], "up"),
                (["↓", "j"], "down"),
                ("↵", "open"),
                ("esc", "exit"),
            ]),
        ], padding=0.0, padding_top=0, gap=0))

    def _draw_detail(self, ctx) -> None:
        if not self._files:
            return
        path = self._files[self._selected]
        w, h = ctx.w, ctx.h

        appbar = AppBar(path.name, subtitle=f"{len(self._rows)} rows × {len(self._headers)} columns")
        footer = FooterKeys([
            (["↑", "k"], "up"),
            (["↓", "j"], "down"),
            (["←", "h"], "col left"),
            (["→", "l"], "col right"),
            ("esc", "back"),
        ])

        appbar_h = appbar.measure(w)
        footer_h = footer.measure(w)

        ctx.clear(ctx.theme.bg)
        appbar.render(ctx, 0.0, 0.0, w, appbar_h)
        footer.render(ctx, 0.0, h - footer_h, w, footer_h)

        # CSV grid occupies the space between bar and footer
        y = appbar_h
        grid_h = h - appbar_h - footer_h

        visible_cols = max(1, int((w - 0) // COL_W))
        col_start = self._h_scroll
        col_end = min(len(self._headers), col_start + visible_cols)

        # Header row
        ctx.rect(0, y, w, ROW_H, fill=ctx.theme.surface, radius=0.0)
        for ci, col_idx in enumerate(range(col_start, col_end)):
            x = ci * COL_W + CELL_PAD
            label = self._headers[col_idx] if col_idx < len(self._headers) else ""
            ctx.text(x, y + 4, label, size=12, color=ctx.theme.accent, bold=True,
                     max_width=COL_W - CELL_PAD * 2)
        y += ROW_H + 2

        # Data rows
        visible_rows = max(1, int((grid_h - ROW_H - 2) // ROW_H))
        max_v = max(0, len(self._rows) - visible_rows)
        if self._v_scroll > max_v:
            self._v_scroll = max_v
        row_start = self._v_scroll
        row_end = min(len(self._rows), row_start + visible_rows)

        for ri, row_idx in enumerate(range(row_start, row_end)):
            row_y = y + ri * ROW_H
            if ri % 2 == 1:
                ctx.rect(0, row_y, w, ROW_H, fill=ctx.theme.bg_darkest, radius=0.0)
            row = self._rows[row_idx]
            for ci, col_idx in enumerate(range(col_start, col_end)):
                x = ci * COL_W + CELL_PAD
                cell = row[col_idx] if col_idx < len(row) else ""
                ctx.text(x, row_y + 4, cell, size=12, color=ctx.theme.fg,
                         max_width=COL_W - CELL_PAD * 2)


if __name__ == "__main__":
    CsvViewer().run()
