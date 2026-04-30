"""TextBuffer — pure text model for a text editor / text area widget.

Lines + cursor + selection. No rendering; that is the TextArea widget's job.
"""

from dataclasses import dataclass, field


@dataclass
class Cursor:
    row: int = 0
    col: int = 0

    def __le__(self, other: Cursor) -> bool:
        return (self.row, self.col) <= (other.row, other.col)

    def __lt__(self, other: Cursor) -> bool:
        return (self.row, self.col) < (other.row, other.col)

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, Cursor):
            return NotImplemented
        return self.row == other.row and self.col == other.col


@dataclass
class Selection:
    """Anchor is fixed; head follows the cursor.

    The selection is inclusive of anchor and exclusive of head, like most
    editors. Use normalized() to always get (start, end) in document order.
    """
    anchor: Cursor
    head: Cursor

    def is_empty(self) -> bool:
        return self.anchor == self.head

    def normalized(self) -> tuple[Cursor, Cursor]:
        """Return (start, end) where start <= end in document order."""
        if self.anchor <= self.head:
            return self.anchor, self.head
        return self.head, self.anchor


class TextBuffer:
    """Pure text model: lines, cursor, and optional selection.

    Internal representation: list of strings, no trailing newlines per line.
    An empty buffer contains exactly one empty string.
    """

    def __init__(self, text: str = "") -> None:
        self._lines: list[str] = text.split("\n") if text else [""]
        if not self._lines:
            self._lines = [""]
        self._cursor: Cursor = Cursor(0, 0)
        self._selection: Selection | None = None
        # Desired column for up/down navigation (preserved across short lines).
        self._desired_col: int = 0

    # ------------------------------------------------------------------
    # Properties
    # ------------------------------------------------------------------

    @property
    def lines(self) -> list[str]:
        """Read-only view of the line list."""
        return list(self._lines)

    @property
    def cursor(self) -> Cursor:
        return Cursor(self._cursor.row, self._cursor.col)

    @property
    def selection(self) -> Selection | None:
        if self._selection is None or self._selection.is_empty():
            return None
        return Selection(
            Cursor(self._selection.anchor.row, self._selection.anchor.col),
            Cursor(self._selection.head.row, self._selection.head.col),
        )

    # ------------------------------------------------------------------
    # Text accessors
    # ------------------------------------------------------------------

    def text(self) -> str:
        return "\n".join(self._lines)

    def selected_text(self) -> str:
        sel = self.selection
        if sel is None:
            return ""
        start, end = sel.normalized()
        if start.row == end.row:
            return self._lines[start.row][start.col : end.col]
        # First partial line
        parts = [self._lines[start.row][start.col :]]
        # Middle lines (whole)
        for r in range(start.row + 1, end.row):
            parts.append(self._lines[r])
        # Last partial line
        parts.append(self._lines[end.row][: end.col])
        return "\n".join(parts)

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------

    def _clamp_cursor(self, row: int, col: int) -> Cursor:
        row = max(0, min(row, len(self._lines) - 1))
        col = max(0, min(col, len(self._lines[row])))
        return Cursor(row, col)

    def _set_cursor_raw(self, row: int, col: int) -> None:
        self._cursor = self._clamp_cursor(row, col)
        self._desired_col = self._cursor.col

    def _delete_range(self, start: Cursor, end: Cursor) -> None:
        """Delete text from start (inclusive) to end (exclusive) in document order."""
        if start.row == end.row:
            line = self._lines[start.row]
            self._lines[start.row] = line[: start.col] + line[end.col :]
        else:
            first = self._lines[start.row][: start.col]
            last = self._lines[end.row][end.col :]
            self._lines[start.row : end.row + 1] = [first + last]
        self._cursor = Cursor(start.row, start.col)
        self._selection = None

    def _delete_selection_if_any(self) -> bool:
        """If a selection exists, delete it and return True."""
        sel = self.selection
        if sel is None:
            return False
        start, end = sel.normalized()
        self._delete_range(start, end)
        return True

    # ------------------------------------------------------------------
    # Mutation
    # ------------------------------------------------------------------

    def insert(self, ch: str) -> None:
        """Insert text at cursor, replacing any existing selection first."""
        self._delete_selection_if_any()
        if "\n" not in ch:
            # Fast path: single-line insert
            r, c = self._cursor.row, self._cursor.col
            line = self._lines[r]
            self._lines[r] = line[:c] + ch + line[c:]
            self._set_cursor_raw(r, c + len(ch))
            return
        # Multi-line insert: split on newlines
        parts = ch.split("\n")
        r, c = self._cursor.row, self._cursor.col
        tail = self._lines[r][c:]
        self._lines[r] = self._lines[r][:c] + parts[0]
        new_rows: list[str] = []
        for middle in parts[1:-1]:
            new_rows.append(middle)
        new_rows.append(parts[-1] + tail)
        self._lines[r + 1 : r + 1] = new_rows
        new_row = r + len(parts) - 1
        new_col = len(parts[-1])
        self._set_cursor_raw(new_row, new_col)

    def insert_newline(self) -> None:
        """Convenience wrapper; equivalent to insert('\\n')."""
        self.insert("\n")

    def delete_back(self) -> None:
        """Backspace. Deletes selection if one exists, else deletes char before cursor."""
        if self._delete_selection_if_any():
            return
        r, c = self._cursor.row, self._cursor.col
        if r == 0 and c == 0:
            return  # no-op at doc start
        if c == 0:
            # Join with previous line
            prev_len = len(self._lines[r - 1])
            self._lines[r - 1] = self._lines[r - 1] + self._lines[r]
            del self._lines[r]
            self._set_cursor_raw(r - 1, prev_len)
        else:
            line = self._lines[r]
            self._lines[r] = line[: c - 1] + line[c:]
            self._set_cursor_raw(r, c - 1)

    def delete_forward(self) -> None:
        """Forward delete. Symmetric to delete_back."""
        if self._delete_selection_if_any():
            return
        r, c = self._cursor.row, self._cursor.col
        last_row = len(self._lines) - 1
        if r == last_row and c == len(self._lines[r]):
            return  # no-op at doc end
        if c == len(self._lines[r]):
            # Join next line in; cursor stays
            self._lines[r] = self._lines[r] + self._lines[r + 1]
            del self._lines[r + 1]
        else:
            line = self._lines[r]
            self._lines[r] = line[:c] + line[c + 1 :]

    # ------------------------------------------------------------------
    # Cursor movement
    # ------------------------------------------------------------------

    def move_cursor(self, direction: str, select: bool = False) -> None:
        """Move cursor in the given direction.

        Valid directions: 'left', 'right', 'up', 'down',
                          'home', 'end', 'doc_start', 'doc_end'.

        If select=True the selection head is extended; otherwise any existing
        selection is cleared first (standard editor behaviour: pressing an
        arrow key without Shift collapses to the nearer edge).
        """
        if not select:
            # Collapse selection to the appropriate edge before moving
            sel = self.selection
            if sel is not None:
                start, end = sel.normalized()
                if direction in ("left", "up", "home", "doc_start"):
                    self._cursor = Cursor(start.row, start.col)
                else:
                    self._cursor = Cursor(end.row, end.col)
                self._selection = None
                # For left/right, collapsing IS the movement
                if direction in ("left", "right"):
                    self._desired_col = self._cursor.col
                    return

        r, c = self._cursor.row, self._cursor.col
        last_row = len(self._lines) - 1

        if direction == "left":
            if c > 0:
                c -= 1
            elif r > 0:
                r -= 1
                c = len(self._lines[r])
        elif direction == "right":
            if c < len(self._lines[r]):
                c += 1
            elif r < last_row:
                r += 1
                c = 0
        elif direction == "up":
            if r > 0:
                r -= 1
                c = min(self._desired_col, len(self._lines[r]))
            else:
                c = 0  # clamp to start of line when already on row 0
        elif direction == "down":
            if r < last_row:
                r += 1
                c = min(self._desired_col, len(self._lines[r]))
            else:
                c = len(self._lines[r])  # clamp to end of last line
        elif direction == "home":
            c = 0
        elif direction == "end":
            c = len(self._lines[r])
        elif direction == "doc_start":
            r, c = 0, 0
        elif direction == "doc_end":
            r = last_row
            c = len(self._lines[r])
        else:
            raise ValueError(f"Unknown direction: {direction!r}")

        new_cursor = self._clamp_cursor(r, c)

        if select:
            if self._selection is None:
                # Start a new selection anchored at the current position
                self._selection = Selection(
                    Cursor(self._cursor.row, self._cursor.col),
                    new_cursor,
                )
            else:
                self._selection.head = new_cursor
            self._cursor = new_cursor
            # Preserve desired_col only for vertical moves
            if direction not in ("up", "down"):
                self._desired_col = new_cursor.col
        else:
            self._cursor = new_cursor
            if direction not in ("up", "down"):
                self._desired_col = new_cursor.col
            self._selection = None

    def set_cursor(self, row: int, col: int, select: bool = False) -> None:
        """Jump cursor to (row, col), clamped to valid positions."""
        new_cursor = self._clamp_cursor(row, col)
        if select:
            if self._selection is None:
                self._selection = Selection(
                    Cursor(self._cursor.row, self._cursor.col),
                    new_cursor,
                )
            else:
                self._selection.head = new_cursor
        else:
            self._selection = None
        self._cursor = new_cursor
        self._desired_col = new_cursor.col

    def select_to(self, row: int, col: int) -> None:
        """Extend selection head to (row, col); creates selection if none."""
        new_head = self._clamp_cursor(row, col)
        if self._selection is None:
            self._selection = Selection(
                Cursor(self._cursor.row, self._cursor.col),
                new_head,
            )
        else:
            self._selection.head = new_head
        self._cursor = new_head
        self._desired_col = new_head.col

    def clear_selection(self) -> None:
        self._selection = None

    def select_all(self) -> None:
        last_row = len(self._lines) - 1
        self._selection = Selection(
            Cursor(0, 0),
            Cursor(last_row, len(self._lines[last_row])),
        )
        self._cursor = Cursor(last_row, len(self._lines[last_row]))
        self._desired_col = self._cursor.col
