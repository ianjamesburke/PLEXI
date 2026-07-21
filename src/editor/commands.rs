//! `Document` state machine and its `EditorCommand` dispatch.
//!
//! Adapted from Ferrite <https://github.com/OlaProeis/Ferrite>
//! @ 3ba085c561670342d72c560efbf6b0b92b5c0b46 (editor.rs command handling),
//! MIT. Diverges from upstream: the 4k-line orchestrator is replaced by this
//! explicit command API; every buffer mutation is a recorded [`Transaction`].

use super::buffer::TextBuffer;
use super::cursor::{Cursor, Selection};
use super::history::EditHistory;
use super::ime::ImeState;
use super::movement;
use super::selection;
use super::transaction::{EditOperation, Transaction};
use super::EditorSemanticState;

/// Directional caret movements. `extend: true` moves only the head
/// (shift-selection).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Movement {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
}

/// Everything the widget (or a test) can ask a [`Document`] to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorCommand {
    /// Insert text at the caret, replacing any selection.
    InsertText(String),
    InsertNewline,
    /// Delete selection, or one grapheme left of the caret.
    Backspace,
    /// Delete selection, or one grapheme right of the caret.
    DeleteForward,
    Move { movement: Movement, extend: bool },
    SetCursor(Cursor),
    /// Move the head only (mouse drag / shift-click).
    ExtendTo(Cursor),
    SelectAll,
    /// Double-click word selection at a position.
    SelectWordAt(Cursor),
    /// Triple-click line selection.
    SelectLineAt(usize),
    Undo,
    Redo,
    /// IME composition text changed (not inserted into the buffer).
    ImePreedit(String),
    /// IME composition committed: inserts the text as one transaction.
    ImeCommit(String),
    /// IME composition cancelled.
    ImeCancel,
}

/// An editable document: rope buffer, selection, transaction history, and
/// IME state. All mutation flows through [`Document::apply`].
#[derive(Debug, Clone)]
pub struct Document {
    buffer: TextBuffer,
    selection: Selection,
    history: EditHistory,
    ime: ImeState,
    /// Column the caret aims for during vertical movement across short lines.
    goal_column: Option<usize>,
}

impl Document {
    #[must_use]
    pub fn new(text: &str) -> Self {
        let buffer = TextBuffer::from_string(text);
        log::info!(
            "editor: document constructed ({} chars, {} lines)",
            buffer.len(),
            buffer.line_count()
        );
        Self {
            buffer,
            selection: Selection::default(),
            history: EditHistory::default(),
            ime: ImeState::default(),
            goal_column: None,
        }
    }

    #[must_use]
    pub fn buffer(&self) -> &TextBuffer {
        &self.buffer
    }

    #[must_use]
    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// The caret position (selection head).
    #[must_use]
    pub fn cursor(&self) -> Cursor {
        self.selection.head
    }

    #[must_use]
    pub fn text(&self) -> String {
        self.buffer.to_string()
    }

    /// The currently selected text (empty when collapsed).
    #[must_use]
    pub fn selected_text(&self) -> String {
        let (start, end) = self.selection.ordered();
        self.buffer.slice(
            movement::cursor_to_char(&self.buffer, start),
            movement::cursor_to_char(&self.buffer, end),
        )
    }

    #[must_use]
    pub fn ime(&self) -> &ImeState {
        &self.ime
    }

    /// Semantic snapshot for host inspection and tests. `scroll_y` comes from
    /// the caller's view state.
    #[must_use]
    pub fn semantic_state(&self, scroll_y: f32) -> EditorSemanticState {
        EditorSemanticState {
            text: self.text(),
            selection: self.selection,
            cursor: self.cursor(),
            undo_depth: self.history.undo_depth(),
            can_undo: self.history.can_undo(),
            can_redo: self.history.can_redo(),
            ime_composition: self.ime.preedit().map(str::to_string),
            scroll_y,
        }
    }

    /// Applies a command. The single entry point for all document mutation.
    pub fn apply(&mut self, command: EditorCommand) {
        match command {
            EditorCommand::InsertText(text) => self.insert_text(&text, true),
            EditorCommand::InsertNewline => self.insert_text("\n", false),
            EditorCommand::Backspace => self.delete(true),
            EditorCommand::DeleteForward => self.delete(false),
            EditorCommand::Move { movement, extend } => self.do_move(movement, extend),
            EditorCommand::SetCursor(cursor) => {
                self.set_selection(Selection::collapsed(movement::clamp(&self.buffer, cursor)));
            }
            EditorCommand::ExtendTo(cursor) => {
                let head = movement::clamp(&self.buffer, cursor);
                self.set_selection(Selection::new(self.selection.anchor, head));
            }
            EditorCommand::SelectAll => self.set_selection(selection::select_all(&self.buffer)),
            EditorCommand::SelectWordAt(cursor) => {
                let cursor = movement::clamp(&self.buffer, cursor);
                self.set_selection(selection::word_boundaries(&self.buffer, cursor));
            }
            EditorCommand::SelectLineAt(line) => {
                self.set_selection(selection::line_selection(&self.buffer, line));
            }
            EditorCommand::Undo => {
                if let Some(sel) = self.history.undo(&mut self.buffer) {
                    self.selection = clamp_selection(&self.buffer, sel);
                }
                self.goal_column = None;
            }
            EditorCommand::Redo => {
                if let Some(sel) = self.history.redo(&mut self.buffer) {
                    self.selection = clamp_selection(&self.buffer, sel);
                }
                self.goal_column = None;
            }
            EditorCommand::ImePreedit(text) => self.ime.set_preedit(text),
            EditorCommand::ImeCommit(text) => {
                self.ime.clear();
                if !text.is_empty() {
                    self.insert_text(&text, false);
                }
            }
            EditorCommand::ImeCancel => self.ime.clear(),
        }
    }

    fn set_selection(&mut self, selection: Selection) {
        self.selection = selection;
        self.goal_column = None;
        self.history.break_group();
    }

    /// Inserts `text` at the caret (replacing any selection) as one
    /// transaction. `coalesce` groups rapid typing into one undo step.
    fn insert_text(&mut self, text: &str, coalesce: bool) {
        let selection_before = self.selection;
        let mut ops = Vec::new();
        let (start, end) = self.selection.ordered();
        let start_char = movement::cursor_to_char(&self.buffer, start);
        if self.selection.is_range() {
            let end_char = movement::cursor_to_char(&self.buffer, end);
            ops.push(EditOperation::Delete {
                pos: start_char,
                text: self.buffer.slice(start_char, end_char),
            });
        }
        ops.push(EditOperation::Insert {
            pos: start_char,
            text: text.to_string(),
        });
        let caret_after = start_char + text.chars().count();
        self.commit(
            ops,
            selection_before,
            caret_after,
            coalesce && !selection_before.is_range(),
        );
    }

    /// Deletes the selection, or one grapheme backward/forward of the caret.
    fn delete(&mut self, backward: bool) {
        let selection_before = self.selection;
        let (start_char, end_char) = if self.selection.is_range() {
            let (start, end) = self.selection.ordered();
            (
                movement::cursor_to_char(&self.buffer, start),
                movement::cursor_to_char(&self.buffer, end),
            )
        } else {
            let caret = movement::clamp(&self.buffer, self.selection.head);
            let other = if backward {
                movement::left(&self.buffer, caret)
            } else {
                movement::right(&self.buffer, caret)
            };
            let a = movement::cursor_to_char(&self.buffer, other);
            let b = movement::cursor_to_char(&self.buffer, caret);
            (a.min(b), a.max(b))
        };
        if start_char == end_char {
            return;
        }
        let ops = vec![EditOperation::Delete {
            pos: start_char,
            text: self.buffer.slice(start_char, end_char),
        }];
        self.commit(ops, selection_before, start_char, false);
    }

    /// Applies `ops` as one transaction, records it, and moves the caret to
    /// `caret_after` (a char index). Invalid transactions are logged and
    /// dropped without mutating anything.
    fn commit(
        &mut self,
        ops: Vec<EditOperation>,
        selection_before: Selection,
        caret_after: usize,
        coalesce: bool,
    ) {
        let mut transaction = Transaction {
            ops,
            selection_before,
            selection_after: selection_before,
        };
        if let Err(e) = transaction.apply(&mut self.buffer) {
            log::error!("editor: invalid transaction application: {e}");
            return;
        }
        let caret = movement::char_to_cursor(&self.buffer, caret_after);
        transaction.selection_after = Selection::collapsed(caret);
        self.selection = transaction.selection_after;
        self.goal_column = None;
        self.history.record(transaction, coalesce);
    }

    fn do_move(&mut self, m: Movement, extend: bool) {
        let head = self.selection.head;
        let goal = match m {
            Movement::Up | Movement::Down => *self.goal_column.get_or_insert(head.column),
            _ => head.column,
        };
        let new_head = match m {
            Movement::Left => movement::left(&self.buffer, head),
            Movement::Right => movement::right(&self.buffer, head),
            Movement::Up => movement::up(&self.buffer, head, goal),
            Movement::Down => movement::down(&self.buffer, head, goal),
            Movement::WordLeft => movement::word_left(&self.buffer, head),
            Movement::WordRight => movement::word_right(&self.buffer, head),
            Movement::LineStart => movement::line_start(head),
            Movement::LineEnd => movement::line_end(&self.buffer, head),
            Movement::DocStart => movement::doc_start(),
            Movement::DocEnd => movement::doc_end(&self.buffer),
        };
        // Collapsing a selection with plain left/right jumps to its edge.
        let new_head = if !extend
            && self.selection.is_range()
            && matches!(m, Movement::Left | Movement::Right)
        {
            let (start, end) = self.selection.ordered();
            if m == Movement::Left {
                start
            } else {
                end
            }
        } else {
            new_head
        };
        self.selection = if extend {
            Selection::new(self.selection.anchor, new_head)
        } else {
            Selection::collapsed(new_head)
        };
        if !matches!(m, Movement::Up | Movement::Down) {
            self.goal_column = None;
        }
        self.history.break_group();
    }
}

fn clamp_selection(buffer: &TextBuffer, sel: Selection) -> Selection {
    Selection::new(
        movement::clamp(buffer, sel.anchor),
        movement::clamp(buffer, sel.head),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(s: &str) -> Document {
        Document::new(s)
    }

    #[test]
    fn typing_inserts_and_moves_caret() {
        let mut d = doc("");
        d.apply(EditorCommand::InsertText("héllo".into()));
        assert_eq!(d.text(), "héllo");
        assert_eq!(d.cursor(), Cursor::new(0, 5));
    }

    #[test]
    fn insert_replaces_selection_atomically() {
        let mut d = doc("Hello World");
        d.apply(EditorCommand::SetCursor(Cursor::new(0, 6)));
        d.apply(EditorCommand::ExtendTo(Cursor::new(0, 11)));
        d.apply(EditorCommand::InsertText("Plexi".into()));
        assert_eq!(d.text(), "Hello Plexi");
        // Single undo restores both delete and insert.
        d.apply(EditorCommand::Undo);
        assert_eq!(d.text(), "Hello World");
        assert_eq!(d.selection().ordered().1, Cursor::new(0, 11));
    }

    #[test]
    fn backspace_deletes_grapheme_cluster() {
        let mut d = doc("a👨\u{200D}👩\u{200D}👧");
        d.apply(EditorCommand::Move {
            movement: Movement::DocEnd,
            extend: false,
        });
        d.apply(EditorCommand::Backspace);
        assert_eq!(d.text(), "a");
        d.apply(EditorCommand::Undo);
        assert_eq!(d.text(), "a👨\u{200D}👩\u{200D}👧");
    }

    #[test]
    fn backspace_at_line_start_joins_lines() {
        let mut d = doc("ab\ncd");
        d.apply(EditorCommand::SetCursor(Cursor::new(1, 0)));
        d.apply(EditorCommand::Backspace);
        assert_eq!(d.text(), "abcd");
        assert_eq!(d.cursor(), Cursor::new(0, 2));
    }

    #[test]
    fn delete_forward_on_empty_doc_is_noop() {
        let mut d = doc("");
        d.apply(EditorCommand::DeleteForward);
        d.apply(EditorCommand::Backspace);
        assert_eq!(d.text(), "");
        assert!(!d.semantic_state(0.0).can_undo);
    }

    #[test]
    fn typing_coalesces_but_movement_breaks_group() {
        let mut d = doc("");
        d.apply(EditorCommand::InsertText("a".into()));
        d.apply(EditorCommand::InsertText("b".into()));
        d.apply(EditorCommand::Move {
            movement: Movement::Left,
            extend: false,
        });
        d.apply(EditorCommand::InsertText("c".into()));
        assert_eq!(d.text(), "acb");
        d.apply(EditorCommand::Undo);
        assert_eq!(d.text(), "ab");
        d.apply(EditorCommand::Undo);
        assert_eq!(d.text(), "");
    }

    #[test]
    fn redo_invalidated_by_new_edit() {
        let mut d = doc("");
        d.apply(EditorCommand::InsertText("a".into()));
        d.apply(EditorCommand::Undo);
        assert!(d.semantic_state(0.0).can_redo);
        d.apply(EditorCommand::InsertText("z".into()));
        let state = d.semantic_state(0.0);
        assert!(!state.can_redo);
        assert_eq!(state.text, "z");
    }

    #[test]
    fn multiline_selection_delete() {
        let mut d = doc("one\ntwo\nthree");
        d.apply(EditorCommand::SetCursor(Cursor::new(0, 2)));
        d.apply(EditorCommand::ExtendTo(Cursor::new(2, 3)));
        assert_eq!(d.selected_text(), "e\ntwo\nthr");
        d.apply(EditorCommand::Backspace);
        assert_eq!(d.text(), "onee");
        d.apply(EditorCommand::Undo);
        assert_eq!(d.text(), "one\ntwo\nthree");
    }

    #[test]
    fn select_all_and_word_and_line() {
        let mut d = doc("foo bar\nbaz");
        d.apply(EditorCommand::SelectAll);
        assert_eq!(d.selected_text(), "foo bar\nbaz");
        d.apply(EditorCommand::SelectWordAt(Cursor::new(0, 5)));
        assert_eq!(d.selected_text(), "bar");
        d.apply(EditorCommand::SelectLineAt(0));
        assert_eq!(d.selected_text(), "foo bar\n");
    }

    #[test]
    fn vertical_movement_keeps_goal_column() {
        let mut d = doc("longest line\nab\nanother long");
        d.apply(EditorCommand::SetCursor(Cursor::new(0, 8)));
        d.apply(EditorCommand::Move {
            movement: Movement::Down,
            extend: false,
        });
        assert_eq!(d.cursor(), Cursor::new(1, 2));
        d.apply(EditorCommand::Move {
            movement: Movement::Down,
            extend: false,
        });
        assert_eq!(d.cursor(), Cursor::new(2, 8));
    }

    #[test]
    fn shift_movement_extends_selection() {
        let mut d = doc("hello world");
        d.apply(EditorCommand::Move {
            movement: Movement::WordRight,
            extend: true,
        });
        assert_eq!(d.selected_text(), "hello");
        // Plain right collapses to selection end.
        d.apply(EditorCommand::Move {
            movement: Movement::Right,
            extend: false,
        });
        assert!(!d.selection().is_range());
        assert_eq!(d.cursor(), Cursor::new(0, 5));
    }

    #[test]
    fn ime_commit_and_cancel() {
        let mut d = doc("x");
        d.apply(EditorCommand::SetCursor(Cursor::new(0, 1)));
        d.apply(EditorCommand::ImePreedit("かん".into()));
        // Preedit never touches the buffer.
        assert_eq!(d.text(), "x");
        assert_eq!(
            d.semantic_state(0.0).ime_composition,
            Some("かん".to_string())
        );
        d.apply(EditorCommand::ImeCommit("漢字".into()));
        assert_eq!(d.text(), "x漢字");
        assert_eq!(d.semantic_state(0.0).ime_composition, None);

        d.apply(EditorCommand::ImePreedit("あ".into()));
        d.apply(EditorCommand::ImeCancel);
        assert_eq!(d.text(), "x漢字");
        assert!(!d.ime().is_composing());
        // Commit is undoable like any transaction.
        d.apply(EditorCommand::Undo);
        assert_eq!(d.text(), "x");
    }

    #[test]
    fn undo_restores_selection() {
        let mut d = doc("abc");
        d.apply(EditorCommand::SelectAll);
        d.apply(EditorCommand::InsertText("z".into()));
        assert_eq!(d.text(), "z");
        d.apply(EditorCommand::Undo);
        assert_eq!(d.text(), "abc");
        assert_eq!(d.selected_text(), "abc");
        d.apply(EditorCommand::Redo);
        assert_eq!(d.text(), "z");
        assert_eq!(d.cursor(), Cursor::new(0, 1));
    }
}
