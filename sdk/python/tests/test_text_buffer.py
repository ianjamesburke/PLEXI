"""Tests for TextBuffer — pure text model (no rendering)."""
from __future__ import annotations

import pytest
from plexi_sdk.widgets.text_buffer import Cursor, Selection, TextBuffer


# ---------------------------------------------------------------------------
# 1. Empty buffer
# ---------------------------------------------------------------------------

def test_empty_buffer_has_one_empty_line():
    buf = TextBuffer()
    assert buf.lines == [""]
    assert buf.cursor == Cursor(0, 0)
    assert buf.text() == ""


# ---------------------------------------------------------------------------
# 2. Initial text splits into lines correctly
# ---------------------------------------------------------------------------

def test_initial_text_splits_on_newlines():
    buf = TextBuffer("hello\nworld\nfoo")
    assert buf.lines == ["hello", "world", "foo"]
    assert buf.text() == "hello\nworld\nfoo"


def test_initial_text_single_line():
    buf = TextBuffer("abc")
    assert buf.lines == ["abc"]


def test_initial_text_trailing_newline():
    # A trailing \n produces an empty final line — same as split()
    buf = TextBuffer("abc\n")
    assert buf.lines == ["abc", ""]


# ---------------------------------------------------------------------------
# 3. insert("a") advances cursor by 1 col
# ---------------------------------------------------------------------------

def test_insert_single_char_advances_col():
    buf = TextBuffer()
    buf.insert("a")
    assert buf.cursor == Cursor(0, 1)
    assert buf.text() == "a"


# ---------------------------------------------------------------------------
# 4. insert("abc") advances cursor by 3
# ---------------------------------------------------------------------------

def test_insert_multi_char_advances_by_length():
    buf = TextBuffer()
    buf.insert("abc")
    assert buf.cursor == Cursor(0, 3)
    assert buf.text() == "abc"


# ---------------------------------------------------------------------------
# 5. insert("\n") splits line; cursor at (row+1, 0)
# ---------------------------------------------------------------------------

def test_insert_newline_splits_line():
    buf = TextBuffer()
    buf.insert("\n")
    assert buf.lines == ["", ""]
    assert buf.cursor == Cursor(1, 0)


def test_insert_newline_mid_word():
    buf = TextBuffer("hello")
    buf.set_cursor(0, 3)
    buf.insert("\n")
    assert buf.lines == ["hel", "lo"]
    assert buf.cursor == Cursor(1, 0)


# ---------------------------------------------------------------------------
# 6. Insert in middle of line splits correctly
# ---------------------------------------------------------------------------

def test_insert_in_middle_of_line():
    buf = TextBuffer("ac")
    buf.set_cursor(0, 1)
    buf.insert("b")
    assert buf.text() == "abc"
    assert buf.cursor == Cursor(0, 2)


def test_insert_multiline_string():
    buf = TextBuffer("ac")
    buf.set_cursor(0, 1)
    buf.insert("b\n")
    assert buf.lines == ["ab", "c"]
    assert buf.cursor == Cursor(1, 0)


# ---------------------------------------------------------------------------
# 7. delete_back at (0,0) is a no-op
# ---------------------------------------------------------------------------

def test_delete_back_at_origin_is_noop():
    buf = TextBuffer("hello")
    buf.set_cursor(0, 0)
    buf.delete_back()
    assert buf.text() == "hello"
    assert buf.cursor == Cursor(0, 0)


# ---------------------------------------------------------------------------
# 8. delete_back at start of line joins with previous
# ---------------------------------------------------------------------------

def test_delete_back_at_line_start_joins_previous():
    buf = TextBuffer("hello\nworld")
    buf.set_cursor(1, 0)
    buf.delete_back()
    assert buf.lines == ["helloworld"]
    assert buf.cursor == Cursor(0, 5)


# ---------------------------------------------------------------------------
# 9. delete_back deletes previous char
# ---------------------------------------------------------------------------

def test_delete_back_removes_previous_char():
    buf = TextBuffer("abc")
    buf.set_cursor(0, 3)
    buf.delete_back()
    assert buf.text() == "ab"
    assert buf.cursor == Cursor(0, 2)


# ---------------------------------------------------------------------------
# 10. delete_forward at end of line joins next line
# ---------------------------------------------------------------------------

def test_delete_forward_at_end_of_line_joins_next():
    buf = TextBuffer("hello\nworld")
    buf.set_cursor(0, 5)
    buf.delete_forward()
    assert buf.lines == ["helloworld"]
    assert buf.cursor == Cursor(0, 5)


def test_delete_forward_removes_next_char():
    buf = TextBuffer("abc")
    buf.set_cursor(0, 1)
    buf.delete_forward()
    assert buf.text() == "ac"
    assert buf.cursor == Cursor(0, 1)


def test_delete_forward_at_doc_end_is_noop():
    buf = TextBuffer("abc")
    buf.set_cursor(0, 3)
    buf.delete_forward()
    assert buf.text() == "abc"


# ---------------------------------------------------------------------------
# 11. move_cursor('right') at end of line wraps to next line start
# ---------------------------------------------------------------------------

def test_move_right_wraps_to_next_line():
    buf = TextBuffer("hello\nworld")
    buf.set_cursor(0, 5)
    buf.move_cursor("right")
    assert buf.cursor == Cursor(1, 0)


def test_move_right_at_doc_end_stays():
    buf = TextBuffer("abc")
    buf.set_cursor(0, 3)
    buf.move_cursor("right")
    assert buf.cursor == Cursor(0, 3)


# ---------------------------------------------------------------------------
# 12. move_cursor('left') at start of line wraps to previous line end
# ---------------------------------------------------------------------------

def test_move_left_wraps_to_previous_line_end():
    buf = TextBuffer("hello\nworld")
    buf.set_cursor(1, 0)
    buf.move_cursor("left")
    assert buf.cursor == Cursor(0, 5)


def test_move_left_at_doc_start_stays():
    buf = TextBuffer("abc")
    buf.set_cursor(0, 0)
    buf.move_cursor("left")
    assert buf.cursor == Cursor(0, 0)


# ---------------------------------------------------------------------------
# 13. move_cursor('up') from row 0 clamps to col 0
# ---------------------------------------------------------------------------

def test_move_up_from_row_0_clamps_col_to_zero():
    buf = TextBuffer("hello")
    buf.set_cursor(0, 3)
    buf.move_cursor("up")
    assert buf.cursor == Cursor(0, 0)


def test_move_up_from_row_1_goes_to_row_0():
    buf = TextBuffer("hello\nworld")
    buf.set_cursor(1, 3)
    buf.move_cursor("up")
    assert buf.cursor == Cursor(0, 3)


# ---------------------------------------------------------------------------
# 14. move_cursor('end') goes to end of current line
# ---------------------------------------------------------------------------

def test_move_end_goes_to_line_end():
    buf = TextBuffer("hello\nworld")
    buf.set_cursor(0, 0)
    buf.move_cursor("end")
    assert buf.cursor == Cursor(0, 5)


def test_move_home_goes_to_line_start():
    buf = TextBuffer("hello")
    buf.set_cursor(0, 4)
    buf.move_cursor("home")
    assert buf.cursor == Cursor(0, 0)


def test_doc_end_goes_to_last_line_end():
    buf = TextBuffer("hello\nworld")
    buf.set_cursor(0, 0)
    buf.move_cursor("doc_end")
    assert buf.cursor == Cursor(1, 5)


def test_doc_start_goes_to_beginning():
    buf = TextBuffer("hello\nworld")
    buf.set_cursor(1, 3)
    buf.move_cursor("doc_start")
    assert buf.cursor == Cursor(0, 0)


# ---------------------------------------------------------------------------
# 15. select_to creates selection; selected_text returns correct substring
# ---------------------------------------------------------------------------

def test_select_to_creates_selection_and_text():
    buf = TextBuffer("hello world")
    buf.set_cursor(0, 0)
    buf.select_to(0, 5)
    assert buf.selected_text() == "hello"


def test_select_to_backward_creates_selection():
    buf = TextBuffer("hello world")
    buf.set_cursor(0, 5)
    buf.select_to(0, 0)
    assert buf.selected_text() == "hello"


# ---------------------------------------------------------------------------
# 16. Insert replaces selection
# ---------------------------------------------------------------------------

def test_insert_replaces_selection():
    buf = TextBuffer("hello world")
    buf.set_cursor(0, 6)
    buf.select_to(0, 11)
    buf.insert("there")
    assert buf.text() == "hello there"
    assert buf.selection is None


def test_insert_replaces_selection_with_empty():
    buf = TextBuffer("hello world")
    buf.set_cursor(0, 5)
    buf.select_to(0, 11)
    buf.insert("")
    assert buf.text() == "hello"


# ---------------------------------------------------------------------------
# 17. Selection across multiple lines; selected_text includes newlines
# ---------------------------------------------------------------------------

def test_multiline_selected_text():
    buf = TextBuffer("hello\nworld\nfoo")
    buf.set_cursor(0, 3)
    buf.select_to(2, 2)
    assert buf.selected_text() == "lo\nworld\nfo"


def test_multiline_selection_includes_whole_lines():
    buf = TextBuffer("aaa\nbbb\nccc")
    buf.set_cursor(0, 0)
    buf.select_to(2, 3)
    assert buf.selected_text() == "aaa\nbbb\nccc"


# ---------------------------------------------------------------------------
# 18. move_cursor('right', select=True) extends selection
# ---------------------------------------------------------------------------

def test_move_right_with_select_extends_selection():
    buf = TextBuffer("hello")
    buf.set_cursor(0, 0)
    buf.move_cursor("right", select=True)
    buf.move_cursor("right", select=True)
    assert buf.selected_text() == "he"
    assert buf.cursor == Cursor(0, 2)


def test_move_without_select_clears_selection():
    buf = TextBuffer("hello")
    buf.set_cursor(0, 0)
    buf.move_cursor("right", select=True)
    buf.move_cursor("right", select=True)
    buf.move_cursor("right")  # no select — should collapse
    assert buf.selection is None


# ---------------------------------------------------------------------------
# 19. select_all selects everything
# ---------------------------------------------------------------------------

def test_select_all_single_line():
    buf = TextBuffer("hello")
    buf.select_all()
    assert buf.selected_text() == "hello"
    start, end = buf.selection.normalized()
    assert start == Cursor(0, 0)
    assert end == Cursor(0, 5)


def test_select_all_multiline():
    buf = TextBuffer("hello\nworld\nfoo")
    buf.select_all()
    assert buf.selected_text() == "hello\nworld\nfoo"


def test_select_all_then_insert_replaces_all():
    buf = TextBuffer("hello\nworld")
    buf.select_all()
    buf.insert("new")
    assert buf.text() == "new"
    assert buf.lines == ["new"]


# ---------------------------------------------------------------------------
# Bonus: cursor clamping, set_cursor with select, desired_col
# ---------------------------------------------------------------------------

def test_set_cursor_clamped_to_valid_range():
    buf = TextBuffer("hi")
    buf.set_cursor(99, 99)
    assert buf.cursor == Cursor(0, 2)


def test_set_cursor_with_select():
    buf = TextBuffer("hello world")
    buf.set_cursor(0, 0)
    buf.set_cursor(0, 5, select=True)
    assert buf.selected_text() == "hello"


def test_desired_col_preserved_across_short_line():
    buf = TextBuffer("hello\nhi\nworld")
    buf.set_cursor(0, 5)  # end of "hello"
    buf.move_cursor("down")  # row 1 "hi" len=2 → clamp to 2
    assert buf.cursor == Cursor(1, 2)
    buf.move_cursor("down")  # row 2 "world" len=5 → desired_col=5, land at 5
    assert buf.cursor == Cursor(2, 5)


def test_delete_back_with_selection_deletes_selection():
    buf = TextBuffer("hello world")
    buf.set_cursor(0, 0)
    buf.select_to(0, 5)
    buf.delete_back()
    assert buf.text() == " world"
    assert buf.selection is None


def test_delete_forward_with_selection_deletes_selection():
    buf = TextBuffer("hello world")
    buf.set_cursor(0, 6)
    buf.select_to(0, 11)
    buf.delete_forward()
    assert buf.text() == "hello "
    assert buf.selection is None


def test_clear_selection():
    buf = TextBuffer("hello")
    buf.select_all()
    buf.clear_selection()
    assert buf.selection is None
    # Cursor stays where select_all left it
    assert buf.cursor == Cursor(0, 5)


def test_insert_multiline_replaces_selection():
    buf = TextBuffer("start end")
    buf.set_cursor(0, 6)
    buf.select_to(0, 9)
    buf.insert("new\ntext")
    assert buf.text() == "start new\ntext"
    assert buf.cursor == Cursor(1, 4)
