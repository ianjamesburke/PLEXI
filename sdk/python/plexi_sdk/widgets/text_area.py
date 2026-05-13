"""TextArea widget — TextBuffer + ScrollState + theme → DrawCommands.

Renders a multi-line text area with cursor, selection highlights, and
vertical scrolling. Emits PGAP-compatible DrawCommand dicts (same shape
as ctx.rect / ctx.text). All coordinates are absolute within the
render viewport passed to render().
"""

from dataclasses import dataclass


from plexi_sdk.widgets.scroll import ScrollState
from plexi_sdk.widgets.text_buffer import TextBuffer


@dataclass
class TextAreaTheme:
    """Color and spacing theme for TextArea widgets."""
    background: str = "#1e1e2e"
    foreground: str = "#cdd6f4"
    cursor: str = "#cdd6f4"
    selection: str = "#45475a"
    line_height: float = 20.0
    font_size: float = 14.0
    char_width: float = 8.4   # approximate monospace advance for font_size 14
    padding: float = 8.0
    monospace: bool = True


class TextArea:
    """TextBuffer + ScrollState + theme → DrawCommands.

    The widget owns geometry (position + size) at render time; the buffer and
    scroll are passed in so callers control state.
    """

    def __init__(
        self,
        buffer: TextBuffer,
        scroll: ScrollState | None = None,
        theme: TextAreaTheme | None = None,
    ) -> None:
        self.buffer = buffer
        self.theme = theme or TextAreaTheme()
        self.scroll = scroll or ScrollState(
            content_height=self._compute_content_height(),
            viewport_height=0.0,
        )

    # ── Geometry helpers ──────────────────────────────────────────────

    def _compute_content_height(self) -> float:
        return len(self.buffer.lines) * self.theme.line_height + 2 * self.theme.padding

    def _line_y_top(self, row: int) -> float:
        """Y of the top of row in *viewport* coordinates (after padding + scroll)."""
        return self.theme.padding + row * self.theme.line_height - self.scroll.offset

    def _col_x(self, col: int) -> float:
        """X of column (relative to widget left edge, before the widget's x offset)."""
        return self.theme.padding + col * self.theme.char_width

    # ── Input ─────────────────────────────────────────────────────────

    def on_key(self, key: str, shift: bool = False, ctrl: bool = False) -> None:
        """Handle a key event. Mutates buffer + keeps cursor visible via scroll."""
        buf = self.buffer

        # Named / special keys
        if key == "backspace":
            buf.delete_back()
        elif key == "delete":
            buf.delete_forward()
        elif key == "enter" or key == "return":
            buf.insert_newline()
        elif key == "left":
            buf.move_cursor("left", select=shift)
        elif key == "right":
            buf.move_cursor("right", select=shift)
        elif key == "up":
            buf.move_cursor("up", select=shift)
        elif key == "down":
            buf.move_cursor("down", select=shift)
        elif key == "home":
            buf.move_cursor("home", select=shift)
        elif key == "end":
            buf.move_cursor("end", select=shift)
        elif key == "a" and ctrl:
            buf.select_all()
        elif len(key) == 1:
            # Printable single character
            buf.insert(key)

        self.ensure_cursor_visible()

    def ensure_cursor_visible(self) -> None:
        self.scroll.content_height = self._compute_content_height()
        cur_top_y = self.theme.padding + self.buffer.cursor.row * self.theme.line_height
        cur_bottom_y = cur_top_y + self.theme.line_height
        self.scroll.ensure_visible(cur_top_y, margin=self.theme.padding)
        self.scroll.ensure_visible(cur_bottom_y, margin=self.theme.padding)

    # ── Rendering ─────────────────────────────────────────────────────

    def render(self, x: float, y: float, w: float, h: float) -> list[dict]:
        """Return DrawCommands for a TextArea at the given viewport rect.

        Updates scroll.viewport_height and scroll.content_height as a side
        effect (so it clamps correctly when the viewport changes size).
        All returned coordinates are absolute (include x, y offsets).
        """
        t = self.theme
        self.scroll.viewport_height = h
        self.scroll.content_height = self._compute_content_height()

        cmds: list[dict] = []

        # 1. Background rect fills the entire viewport.
        cmds.append({
            "type": "rect",
            "x": x, "y": y, "w": w, "h": h,
            "fill": t.background,
            "radius": 0,
        })

        lines = self.buffer.lines
        n_lines = len(lines)

        # Determine first and last visible rows.
        # A row is visible if its top edge is above (y + h) and bottom is below y.
        # row_top (viewport-relative) = padding + row * line_height - offset
        # visible when: row_top < h  AND  row_top + line_height > 0
        first_row = max(0, int((self.scroll.offset - t.padding) / t.line_height))
        last_row = min(
            n_lines - 1,
            int((self.scroll.offset + h - t.padding) / t.line_height),
        )

        # 2. Selection rects (one per involved visible line).
        sel = self.buffer.selection
        if sel is not None:
            start, end = sel.normalized()
            sel_first = max(first_row, start.row)
            sel_last = min(last_row, end.row)

            for row in range(sel_first, sel_last + 1):
                line_text = lines[row]
                line_len = len(line_text)

                if row == start.row and row == end.row:
                    # Selection entirely on this line.
                    sel_col_start = start.col
                    sel_col_end = end.col
                elif row == start.row:
                    sel_col_start = start.col
                    sel_col_end = line_len
                elif row == end.row:
                    sel_col_start = 0
                    sel_col_end = end.col
                else:
                    # Middle line: full line width (to viewport edge).
                    sel_col_start = 0
                    sel_col_end = line_len

                sel_x = x + self._col_x(sel_col_start)
                sel_w = (sel_col_end - sel_col_start) * t.char_width
                # Middle lines with no chars get full-width highlight.
                if row != start.row and row != end.row and line_len == 0:
                    sel_w = t.char_width  # show at least one char width on empty lines

                row_y_top = self._line_y_top(row)
                sel_y = y + row_y_top

                if sel_w > 0:
                    cmds.append({
                        "type": "rect",
                        "x": sel_x, "y": sel_y,
                        "w": sel_w, "h": t.line_height,
                        "fill": t.selection,
                        "radius": 0,
                    })

        # 3. Text: one text command per visible line.
        for row in range(first_row, last_row + 1):
            row_y_top = self._line_y_top(row)
            text_x = x + t.padding
            text_y = y + row_y_top

            cmds.append({
                "type": "text",
                "x": text_x,
                "y": text_y,
                "text": lines[row],
                "size": t.font_size,
                "color": t.foreground,
                "monospace": t.monospace,
                "bold": False,
                "align": "top_left",
                "max_width": None,
                "elide": True,
                "selectable": False,
            })

        # 4. Cursor rect: 2px wide, line_height tall.
        cursor = self.buffer.cursor
        if first_row <= cursor.row <= last_row:
            cur_row_y = self._line_y_top(cursor.row)
            cur_x = x + self._col_x(cursor.col)
            cur_y = y + cur_row_y
            cmds.append({
                "type": "rect",
                "x": cur_x, "y": cur_y,
                "w": 2.0, "h": t.line_height,
                "fill": t.cursor,
                "radius": 0,
            })

        return cmds
