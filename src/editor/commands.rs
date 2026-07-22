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
use super::markdown::{self, MarkdownPlan};
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
    /// One viewport page up; carries the page size in lines.
    PageUp(usize),
    /// One viewport page down; carries the page size in lines.
    PageDown(usize),
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
    /// Tab: indent every selected line (or insert one indent at the caret)
    /// as one undoable transaction.
    Indent,
    /// Shift-Tab: remove one indentation level from every selected line as
    /// one undoable transaction.
    Outdent,
    /// Markdown Tab: indent the current line or all selected lines (falls
    /// back to a plain caret indent inside fenced code blocks).
    MarkdownIndent,
    /// Markdown Shift-Tab: same as [`Self::Outdent`] (named for symmetric
    /// key wiring and logging).
    MarkdownOutdent,
    /// Markdown Enter: continue or exit list/task/quote structures; plain
    /// auto-indent newline everywhere else (including fenced code blocks).
    MarkdownNewline,
    /// Markdown Backspace: remove an empty structure marker; plain (smart)
    /// backspace everywhere else.
    MarkdownBackspace,
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
    /// Monotonic counter bumped on every buffer mutation (edit, undo, redo).
    /// Lets callers detect edits without diffing text.
    revision: u64,
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
            revision: 0,
        }
    }

    /// Monotonic revision counter: bumped on every buffer mutation.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
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
            EditorCommand::InsertNewline => self.insert_newline(),
            EditorCommand::Backspace => self.delete(true),
            EditorCommand::DeleteForward => self.delete(false),
            EditorCommand::Move { movement, extend } => self.do_move(movement, extend),
            EditorCommand::Indent => self.indent(),
            EditorCommand::Outdent => self.outdent(),
            EditorCommand::MarkdownIndent => {
                match markdown::plan_indent(&self.buffer, self.selection) {
                    Some(plan) => self.apply_markdown_plan("indent", plan),
                    // Collapsed caret inside a fenced code block: plain indent.
                    None => self.insert_text("    ", false),
                }
            }
            EditorCommand::MarkdownOutdent => {
                log::info!("editor: markdown command outdent");
                self.outdent();
            }
            EditorCommand::MarkdownNewline => {
                match markdown::plan_newline(&self.buffer, self.selection) {
                    Some(plan) => self.apply_markdown_plan("newline", plan),
                    None => self.insert_newline(),
                }
            }
            EditorCommand::MarkdownBackspace => {
                match markdown::plan_backspace(&self.buffer, self.selection) {
                    Some(plan) => self.apply_markdown_plan("backspace", plan),
                    None => self.delete(true),
                }
            }
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
                    self.revision += 1;
                }
                self.goal_column = None;
            }
            EditorCommand::Redo => {
                if let Some(sel) = self.history.redo(&mut self.buffer) {
                    self.selection = clamp_selection(&self.buffer, sel);
                    self.revision += 1;
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

    /// Auto-indent newline: carry the current line's leading whitespace onto
    /// the new line, capped at the caret column (an Enter at column 0 of an
    /// indented line must not duplicate the indent).
    fn insert_newline(&mut self) {
        let (start, _) = self.selection.ordered();
        let start = movement::clamp(&self.buffer, start);
        let indent: String = movement::line_text(&self.buffer, start.line)
            .chars()
            .take(start.column)
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        self.insert_text(&format!("\n{indent}"), false);
    }

    /// Commits a Markdown planner's ops as one transaction, logging the
    /// command name and edit ranges (never the text content).
    fn apply_markdown_plan(&mut self, name: &str, plan: MarkdownPlan) {
        let ranges: Vec<String> = plan
            .ops
            .iter()
            .map(|op| match op {
                EditOperation::Insert { pos, text } => {
                    format!("insert@{pos}+{}", text.chars().count())
                }
                EditOperation::Delete { pos, text } => {
                    format!("delete@{pos}-{}", text.chars().count())
                }
            })
            .collect();
        log::info!("editor: markdown command {name}: {}", ranges.join(","));
        let selection_before = self.selection;
        self.commit_with_selection(plan.ops, selection_before, plan.selection_after);
    }

    /// Number of spaces smart backspace removes at the caret: when the line
    /// prefix before the caret is pure spaces (2–4 of them), the whole
    /// indentation step deletes in one keypress.
    fn smart_backspace_spaces(&self) -> usize {
        if self.selection.is_range() {
            return 0;
        }
        let caret = movement::clamp(&self.buffer, self.selection.head);
        let prefix: String = movement::line_text(&self.buffer, caret.line)
            .chars()
            .take(caret.column)
            .collect();
        if !prefix.is_empty() && prefix.chars().all(|c| c == ' ') {
            prefix.chars().count().min(4)
        } else {
            0
        }
    }

    /// Deletes the selection, or one grapheme backward/forward of the caret.
    /// Backward deletion inside pure leading spaces removes one logical
    /// indentation level (smart backspace).
    fn delete(&mut self, backward: bool) {
        let selection_before = self.selection;
        if backward {
            let spaces = self.smart_backspace_spaces();
            if spaces > 1 {
                let caret = movement::clamp(&self.buffer, self.selection.head);
                let end_char = movement::cursor_to_char(&self.buffer, caret);
                let start_char = end_char - spaces;
                let ops = vec![EditOperation::Delete {
                    pos: start_char,
                    text: self.buffer.slice(start_char, end_char),
                }];
                self.commit(ops, selection_before, start_char, false);
                return;
            }
        }
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
        self.revision += 1;
        self.history.record(transaction, coalesce);
    }

    /// Like [`Self::commit`], but keeps an explicit (possibly ranged)
    /// selection after the edit — used by indent/outdent, which must leave
    /// the multi-line selection in place.
    fn commit_with_selection(
        &mut self,
        ops: Vec<EditOperation>,
        selection_before: Selection,
        selection_after: Selection,
    ) {
        let transaction = Transaction {
            ops,
            selection_before,
            selection_after,
        };
        if let Err(e) = transaction.apply(&mut self.buffer) {
            log::error!("editor: invalid transaction application: {e}");
            return;
        }
        self.selection = clamp_selection(&self.buffer, selection_after);
        self.goal_column = None;
        self.revision += 1;
        self.history.record(transaction, false);
    }

    /// The inclusive line range an indent/outdent operates on. A selection
    /// ending at column 0 of a later line excludes that line (platform
    /// convention).
    fn indent_line_range(&self) -> (usize, usize) {
        let (start, end) = self.selection.ordered();
        let last = if end.line > start.line && end.column == 0 {
            end.line - 1
        } else {
            end.line
        };
        (start.line, last)
    }

    /// Tab: with a multi-line selection, prefix every selected line with one
    /// indent step as one transaction; otherwise insert an indent at the caret.
    fn indent(&mut self) {
        const INDENT: &str = "    ";
        let selection_before = self.selection;
        let (first, last) = self.indent_line_range();
        if !selection_before.is_range() || first == last {
            self.insert_text(INDENT, false);
            return;
        }
        // Insert back-to-front so earlier positions stay valid as ops apply
        // in order.
        let ops: Vec<EditOperation> = (first..=last)
            .rev()
            .map(|line| EditOperation::Insert {
                pos: self.buffer.line_to_char(line),
                text: INDENT.to_string(),
            })
            .collect();
        let shift = |c: Cursor| {
            if c.line >= first && c.line <= last && c.column > 0 {
                Cursor::new(c.line, c.column + INDENT.len())
            } else if c.line >= first && c.line <= last {
                Cursor::new(c.line, INDENT.len())
            } else {
                c
            }
        };
        let after = Selection::new(shift(selection_before.anchor), shift(selection_before.head));
        self.commit_with_selection(ops, selection_before, after);
    }

    /// Shift-Tab: remove up to one indent step (4 spaces or one tab) from the
    /// start of every selected line as one transaction.
    fn outdent(&mut self) {
        let selection_before = self.selection;
        let (first, last) = self.indent_line_range();
        let mut ops = Vec::new();
        let mut removed_per_line = vec![0usize; last - first + 1];
        for line in (first..=last).rev() {
            let text = movement::line_text(&self.buffer, line);
            let removed = if text.starts_with('\t') {
                1
            } else {
                text.chars().take_while(|c| *c == ' ').count().min(4)
            };
            if removed > 0 {
                let pos = self.buffer.line_to_char(line);
                ops.push(EditOperation::Delete {
                    pos,
                    text: self.buffer.slice(pos, pos + removed),
                });
                removed_per_line[line - first] = removed;
            }
        }
        if ops.is_empty() {
            return;
        }
        let shift = |c: Cursor| {
            if c.line >= first && c.line <= last {
                Cursor::new(
                    c.line,
                    c.column.saturating_sub(removed_per_line[c.line - first]),
                )
            } else {
                c
            }
        };
        let after = Selection::new(shift(selection_before.anchor), shift(selection_before.head));
        self.commit_with_selection(ops, selection_before, after);
    }

    fn do_move(&mut self, m: Movement, extend: bool) {
        let head = self.selection.head;
        let vertical = matches!(
            m,
            Movement::Up | Movement::Down | Movement::PageUp(_) | Movement::PageDown(_)
        );
        let goal = if vertical {
            *self.goal_column.get_or_insert(head.column)
        } else {
            head.column
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
            Movement::PageUp(rows) => movement::page_up(&self.buffer, head, goal, rows),
            Movement::PageDown(rows) => movement::page_down(&self.buffer, head, goal, rows),
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
        if !vertical {
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
    fn tab_inserts_indent_at_caret_without_selection() {
        let mut d = doc("ab");
        d.apply(EditorCommand::SetCursor(Cursor::new(0, 1)));
        d.apply(EditorCommand::Indent);
        assert_eq!(d.text(), "a    b");
        assert_eq!(d.cursor(), Cursor::new(0, 5));
        d.apply(EditorCommand::Undo);
        assert_eq!(d.text(), "ab");
    }

    #[test]
    fn indent_multiline_selection_is_one_transaction() {
        let mut d = doc("one\ntwo\nthree");
        d.apply(EditorCommand::SetCursor(Cursor::new(0, 1)));
        d.apply(EditorCommand::ExtendTo(Cursor::new(2, 2)));
        d.apply(EditorCommand::Indent);
        assert_eq!(d.text(), "    one\n    two\n    three");
        // Selection survives, shifted by the indent.
        assert_eq!(d.selected_text(), "ne\n    two\n    th");
        d.apply(EditorCommand::Undo);
        assert_eq!(d.text(), "one\ntwo\nthree");
    }

    #[test]
    fn indent_excludes_line_when_selection_ends_at_its_start() {
        let mut d = doc("one\ntwo\nthree");
        d.apply(EditorCommand::SetCursor(Cursor::new(0, 0)));
        d.apply(EditorCommand::ExtendTo(Cursor::new(2, 0)));
        d.apply(EditorCommand::Indent);
        assert_eq!(d.text(), "    one\n    two\nthree");
    }

    #[test]
    fn outdent_removes_one_level_per_line_as_one_transaction() {
        let mut d = doc("    one\n  two\n\tthree\nfour");
        d.apply(EditorCommand::SelectAll);
        d.apply(EditorCommand::Outdent);
        assert_eq!(d.text(), "one\ntwo\nthree\nfour");
        d.apply(EditorCommand::Undo);
        assert_eq!(d.text(), "    one\n  two\n\tthree\nfour");
        // Fully outdented text is a no-op (no phantom transaction recorded).
        let mut d = doc("plain");
        d.apply(EditorCommand::Outdent);
        assert!(!d.semantic_state(0.0).can_undo);
    }

    #[test]
    fn smart_backspace_removes_pure_indent_block() {
        let mut d = doc("    x");
        d.apply(EditorCommand::SetCursor(Cursor::new(0, 4)));
        d.apply(EditorCommand::Backspace);
        assert_eq!(d.text(), "x");
        // Mixed prefix deletes one grapheme only.
        let mut d = doc("foo   ");
        d.apply(EditorCommand::Move {
            movement: Movement::DocEnd,
            extend: false,
        });
        d.apply(EditorCommand::Backspace);
        assert_eq!(d.text(), "foo  ");
        // A single leading space deletes normally.
        let mut d = doc(" x");
        d.apply(EditorCommand::SetCursor(Cursor::new(0, 1)));
        d.apply(EditorCommand::Backspace);
        assert_eq!(d.text(), "x");
    }

    #[test]
    fn enter_carries_leading_indent_capped_at_caret() {
        let mut d = doc("    item");
        d.apply(EditorCommand::Move {
            movement: Movement::LineEnd,
            extend: false,
        });
        d.apply(EditorCommand::InsertNewline);
        assert_eq!(d.text(), "    item\n    ");
        assert_eq!(d.cursor(), Cursor::new(1, 4));
        // Enter at column 0 of an indented line does not duplicate the indent.
        let mut d = doc("    item");
        d.apply(EditorCommand::SetCursor(Cursor::new(0, 0)));
        d.apply(EditorCommand::InsertNewline);
        assert_eq!(d.text(), "\n    item");
    }

    #[test]
    fn page_movement_moves_by_rows_and_clamps() {
        let text = (0..100).map(|i| format!("line{i}")).collect::<Vec<_>>();
        let mut d = doc(&text.join("\n"));
        d.apply(EditorCommand::Move {
            movement: Movement::PageDown(20),
            extend: false,
        });
        assert_eq!(d.cursor().line, 20);
        d.apply(EditorCommand::Move {
            movement: Movement::PageUp(50),
            extend: false,
        });
        assert_eq!(d.cursor().line, 0);
        d.apply(EditorCommand::Move {
            movement: Movement::PageDown(500),
            extend: true,
        });
        assert_eq!(d.cursor().line, 99);
        assert!(d.selection().is_range());
    }

    #[test]
    fn down_at_document_end_is_a_noop_never_an_append() {
        // Regression: the old TextEditorApp appended a newline when pressing
        // Down at the end of the document.
        let mut d = doc("last line");
        d.apply(EditorCommand::Move {
            movement: Movement::DocEnd,
            extend: false,
        });
        let before = d.revision();
        d.apply(EditorCommand::Move {
            movement: Movement::Down,
            extend: false,
        });
        assert_eq!(d.text(), "last line");
        assert_eq!(d.revision(), before);
    }

    #[test]
    fn revision_bumps_on_edit_undo_and_redo_only() {
        let mut d = doc("");
        assert_eq!(d.revision(), 0);
        d.apply(EditorCommand::Move {
            movement: Movement::Right,
            extend: false,
        });
        assert_eq!(d.revision(), 0);
        d.apply(EditorCommand::InsertText("a".into()));
        assert_eq!(d.revision(), 1);
        d.apply(EditorCommand::Undo);
        assert_eq!(d.revision(), 2);
        d.apply(EditorCommand::Redo);
        assert_eq!(d.revision(), 3);
        // Empty undo stack: no bump.
        d.apply(EditorCommand::Redo);
        assert_eq!(d.revision(), 3);
    }

    #[test]
    fn large_document_basic_ops() {
        let text = (0..10_000).map(|i| format!("line {i}")).collect::<Vec<_>>();
        let mut d = doc(&text.join("\n"));
        assert_eq!(d.buffer().line_count(), 10_000);
        d.apply(EditorCommand::Move {
            movement: Movement::DocEnd,
            extend: false,
        });
        d.apply(EditorCommand::InsertText("!".into()));
        assert!(d.text().ends_with("line 9999!"));
        d.apply(EditorCommand::SetCursor(Cursor::new(5000, 0)));
        d.apply(EditorCommand::SelectLineAt(5000));
        assert_eq!(d.selected_text(), "line 5000\n");
        d.apply(EditorCommand::Backspace);
        d.apply(EditorCommand::Undo);
        d.apply(EditorCommand::Undo);
        assert!(d.text().ends_with("line 9999"));
    }

    #[test]
    fn markdown_enter_continues_unordered_task_and_quote() {
        for (text, expected) in [
            ("- item", "- item\n- "),
            ("* item", "* item\n* "),
            ("+ item", "+ item\n+ "),
            ("  - nested", "  - nested\n  - "),
            ("- [x] done", "- [x] done\n- [ ] "),
            ("> quoted", "> quoted\n> "),
            ("> > deep", "> > deep\n> > "),
        ] {
            let mut d = doc(text);
            d.apply(EditorCommand::Move {
                movement: Movement::LineEnd,
                extend: false,
            });
            d.apply(EditorCommand::MarkdownNewline);
            assert_eq!(d.text(), expected, "continuation for {text:?}");
            let lines: Vec<&str> = expected.split('\n').collect();
            assert_eq!(
                d.cursor(),
                Cursor::new(1, lines[1].chars().count()),
                "caret sits after the new marker for {text:?}"
            );
            // One atomic undo.
            d.apply(EditorCommand::Undo);
            assert_eq!(d.text(), text);
        }
    }

    #[test]
    fn markdown_enter_mid_item_splits_with_marker() {
        let mut d = doc("- alpha beta");
        d.apply(EditorCommand::SetCursor(Cursor::new(0, 7)));
        d.apply(EditorCommand::MarkdownNewline);
        assert_eq!(d.text(), "- alpha\n- beta");
        assert_eq!(d.cursor(), Cursor::new(1, 2));
    }

    #[test]
    fn markdown_enter_on_empty_continuation_removes_marker() {
        let mut d = doc("- item\n- ");
        d.apply(EditorCommand::Move {
            movement: Movement::DocEnd,
            extend: false,
        });
        d.apply(EditorCommand::MarkdownNewline);
        assert_eq!(d.text(), "- item\n");
        assert_eq!(d.cursor(), Cursor::new(1, 0));
        d.apply(EditorCommand::Undo);
        assert_eq!(d.text(), "- item\n- ");

        // Indented empty continuation removes indent + marker in one step.
        let mut d = doc("  - [ ] ");
        d.apply(EditorCommand::Move {
            movement: Movement::LineEnd,
            extend: false,
        });
        d.apply(EditorCommand::MarkdownNewline);
        assert_eq!(d.text(), "");
    }

    #[test]
    fn markdown_enter_ordered_continues_and_renumbers_siblings() {
        let mut d = doc("1. one\n2. two\n3. three");
        d.apply(EditorCommand::SetCursor(Cursor::new(0, 6)));
        d.apply(EditorCommand::MarkdownNewline);
        assert_eq!(d.text(), "1. one\n2. \n3. two\n4. three");
        assert_eq!(d.cursor(), Cursor::new(1, 3));
        // The whole continuation + renumber is one undo step.
        d.apply(EditorCommand::Undo);
        assert_eq!(d.text(), "1. one\n2. two\n3. three");

        // Exiting an empty ordered item closes the numbering gap it leaves.
        let mut d = doc("1. one\n2. \n3. three");
        d.apply(EditorCommand::SetCursor(Cursor::new(1, 3)));
        d.apply(EditorCommand::MarkdownNewline);
        assert_eq!(d.text(), "1. one\n\n2. three");
        d.apply(EditorCommand::Undo);
        assert_eq!(d.text(), "1. one\n2. \n3. three");

        // Renumbering stops at a non-list line and skips different indents.
        let mut d = doc("1. a\n2. b\nplain\n3. c");
        d.apply(EditorCommand::SetCursor(Cursor::new(0, 4)));
        d.apply(EditorCommand::MarkdownNewline);
        assert_eq!(d.text(), "1. a\n2. \n3. b\nplain\n3. c");
    }

    #[test]
    fn markdown_enter_before_marker_and_on_plain_lines_is_plain_newline() {
        // Caret inside the marker prefix: plain newline, no continuation.
        let mut d = doc("- item");
        d.apply(EditorCommand::SetCursor(Cursor::new(0, 0)));
        d.apply(EditorCommand::MarkdownNewline);
        assert_eq!(d.text(), "\n- item");

        // Plain text: auto-indent newline as usual.
        let mut d = doc("    text");
        d.apply(EditorCommand::Move {
            movement: Movement::LineEnd,
            extend: false,
        });
        d.apply(EditorCommand::MarkdownNewline);
        assert_eq!(d.text(), "    text\n    ");

        // A selection replaces with a plain newline.
        let mut d = doc("- one\n- two");
        d.apply(EditorCommand::SetCursor(Cursor::new(0, 2)));
        d.apply(EditorCommand::ExtendTo(Cursor::new(1, 2)));
        d.apply(EditorCommand::MarkdownNewline);
        assert_eq!(d.text(), "- \ntwo");
    }

    #[test]
    fn markdown_no_list_behavior_inside_fenced_code_blocks() {
        let text = "```\n- not a list\n```";
        let mut d = doc(text);
        d.apply(EditorCommand::SetCursor(Cursor::new(1, 12)));
        d.apply(EditorCommand::MarkdownNewline);
        assert_eq!(d.text(), "```\n- not a list\n\n```", "plain newline in fence");

        // Tab in a fence is a plain caret indent, not a line indent.
        let mut d = doc(text);
        d.apply(EditorCommand::SetCursor(Cursor::new(1, 0)));
        d.apply(EditorCommand::MarkdownIndent);
        assert_eq!(d.text(), "```\n    - not a list\n```");
        assert_eq!(d.cursor(), Cursor::new(1, 4));

        // After the closing fence, Markdown behavior returns.
        let mut d = doc("```\ncode\n```\n- item");
        d.apply(EditorCommand::Move {
            movement: Movement::DocEnd,
            extend: false,
        });
        d.apply(EditorCommand::MarkdownNewline);
        assert_eq!(d.text(), "```\ncode\n```\n- item\n- ");
    }

    #[test]
    fn markdown_tab_indents_whole_line_at_any_caret_position() {
        let mut d = doc("- item");
        d.apply(EditorCommand::SetCursor(Cursor::new(0, 3)));
        d.apply(EditorCommand::MarkdownIndent);
        assert_eq!(d.text(), "    - item");
        assert_eq!(d.cursor(), Cursor::new(0, 7), "caret shifts with the line");
        d.apply(EditorCommand::MarkdownOutdent);
        assert_eq!(d.text(), "- item");
        assert_eq!(d.cursor(), Cursor::new(0, 3));
    }

    #[test]
    fn markdown_tab_uses_tab_unit_on_tab_indented_lines() {
        let mut d = doc("\t- item");
        d.apply(EditorCommand::SetCursor(Cursor::new(0, 3)));
        d.apply(EditorCommand::MarkdownIndent);
        assert_eq!(d.text(), "\t\t- item");
        assert_eq!(d.cursor(), Cursor::new(0, 4));
    }

    #[test]
    fn markdown_indent_mixed_selection_preserves_direction() {
        // Selection spans list and non-list lines, head above anchor.
        let mut d = doc("- one\nplain\n- two");
        d.apply(EditorCommand::SetCursor(Cursor::new(2, 3)));
        d.apply(EditorCommand::ExtendTo(Cursor::new(0, 1)));
        d.apply(EditorCommand::MarkdownIndent);
        assert_eq!(d.text(), "    - one\n    plain\n    - two");
        let sel = d.selection();
        assert_eq!(sel.anchor, Cursor::new(2, 7));
        assert_eq!(sel.head, Cursor::new(0, 5), "direction preserved");
        // One undo restores everything.
        d.apply(EditorCommand::Undo);
        assert_eq!(d.text(), "- one\nplain\n- two");
    }

    #[test]
    fn markdown_indent_excludes_line_when_selection_ends_at_its_start() {
        let mut d = doc("- one\n- two\n- three");
        d.apply(EditorCommand::SetCursor(Cursor::new(0, 0)));
        d.apply(EditorCommand::ExtendTo(Cursor::new(2, 0)));
        d.apply(EditorCommand::MarkdownIndent);
        assert_eq!(d.text(), "    - one\n    - two\n- three");
    }

    #[test]
    fn markdown_backspace_removes_empty_marker_then_indent_level() {
        let mut d = doc("    - ");
        d.apply(EditorCommand::Move {
            movement: Movement::LineEnd,
            extend: false,
        });
        d.apply(EditorCommand::MarkdownBackspace);
        assert_eq!(d.text(), "    ", "marker removed, indent kept");
        assert_eq!(d.cursor(), Cursor::new(0, 4));
        d.apply(EditorCommand::MarkdownBackspace);
        assert_eq!(d.text(), "", "smart backspace removes the indent level");
        d.apply(EditorCommand::Undo);
        d.apply(EditorCommand::Undo);
        assert_eq!(d.text(), "    - ");
    }

    #[test]
    fn markdown_backspace_is_plain_elsewhere() {
        // Mid-content: ordinary grapheme deletion.
        let mut d = doc("- ab");
        d.apply(EditorCommand::Move {
            movement: Movement::LineEnd,
            extend: false,
        });
        d.apply(EditorCommand::MarkdownBackspace);
        assert_eq!(d.text(), "- a");

        // Empty marker line inside a fence: plain deletion, not marker-aware.
        let mut d = doc("```\n- \n```");
        d.apply(EditorCommand::SetCursor(Cursor::new(1, 2)));
        d.apply(EditorCommand::MarkdownBackspace);
        assert_eq!(d.text(), "```\n-\n```");

        // Selection: deletes the selection.
        let mut d = doc("- one");
        d.apply(EditorCommand::SelectAll);
        d.apply(EditorCommand::MarkdownBackspace);
        assert_eq!(d.text(), "");
    }

    #[test]
    fn markdown_pasted_multiline_content_is_one_plain_transaction() {
        let mut d = doc("- item");
        d.apply(EditorCommand::Move {
            movement: Movement::LineEnd,
            extend: false,
        });
        d.apply(EditorCommand::InsertText("\nline a\nline b".into()));
        assert_eq!(d.text(), "- item\nline a\nline b");
        d.apply(EditorCommand::Undo);
        assert_eq!(d.text(), "- item");
    }

    #[test]
    fn markdown_newline_at_document_end_grapheme_safe() {
        let mut d = doc("- caf\u{65}\u{301}👨\u{200D}👩\u{200D}👧");
        d.apply(EditorCommand::Move {
            movement: Movement::DocEnd,
            extend: false,
        });
        d.apply(EditorCommand::MarkdownNewline);
        assert!(d.text().ends_with("\n- "));
        d.apply(EditorCommand::Undo);
        assert_eq!(d.text(), "- caf\u{65}\u{301}👨\u{200D}👩\u{200D}👧");
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
