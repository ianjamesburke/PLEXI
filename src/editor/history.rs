//! Grouped undo/redo over transactions, with typing coalescing.
//!
//! Adapted from Ferrite <https://github.com/OlaProeis/Ferrite>
//! @ 3ba085c561670342d72c560efbf6b0b92b5c0b46 (history.rs), MIT.
//! Diverges from upstream: groups hold [`Transaction`]s (which carry
//! selections), and consecutive coalescible records merge into the open group
//! until [`EditHistory::break_group`] is called.

use super::buffer::TextBuffer;
use super::cursor::Selection;
use super::transaction::Transaction;

/// Maximum retained undo groups; operations are small so this can be large.
const MAX_GROUPS: usize = 500;

#[derive(Debug, Clone)]
struct Group {
    transactions: Vec<Transaction>,
}

/// Undo/redo stacks of transaction groups. Each group is one user-visible
/// undo step; recording clears the redo stack.
#[derive(Debug, Clone, Default)]
pub struct EditHistory {
    undo_stack: Vec<Group>,
    redo_stack: Vec<Group>,
    /// When true, the top undo group may absorb the next coalescible record.
    group_open: bool,
}

impl EditHistory {
    /// Records a transaction. When `coalesce` is true and the current group is
    /// open, the transaction merges into the top group (rapid typing = one
    /// undo step). Any record invalidates the redo stack.
    pub fn record(&mut self, transaction: Transaction, coalesce: bool) {
        self.redo_stack.clear();
        if coalesce && self.group_open {
            if let Some(top) = self.undo_stack.last_mut() {
                top.transactions.push(transaction);
                return;
            }
        }
        self.undo_stack.push(Group {
            transactions: vec![transaction],
        });
        if self.undo_stack.len() > MAX_GROUPS {
            self.undo_stack.remove(0);
        }
        self.group_open = coalesce;
    }

    /// Ends the current coalescing group; the next record starts a new
    /// undo step. Called on cursor movement, clicks, and focus changes.
    pub fn break_group(&mut self) {
        self.group_open = false;
    }

    /// Undoes the top group. Returns the selection to restore, or `None` when
    /// the undo stack is empty or the buffer no longer matches the recorded
    /// operations (the group is dropped and an error logged).
    pub fn undo(&mut self, buffer: &mut TextBuffer) -> Option<Selection> {
        self.group_open = false;
        let group = self.undo_stack.pop()?;
        for t in group.transactions.iter().rev() {
            if let Err(e) = t.revert(buffer) {
                log::error!("editor undo: invalid transaction application: {e}");
                return None;
            }
        }
        let selection = group.transactions.first().map(|t| t.selection_before);
        self.redo_stack.push(group);
        selection
    }

    /// Redoes the last undone group. Returns the selection to restore, or
    /// `None` when empty or invalid (see [`Self::undo`]).
    pub fn redo(&mut self, buffer: &mut TextBuffer) -> Option<Selection> {
        self.group_open = false;
        let group = self.redo_stack.pop()?;
        for t in &group.transactions {
            if let Err(e) = t.apply(buffer) {
                log::error!("editor redo: invalid transaction application: {e}");
                return None;
            }
        }
        let selection = group.transactions.last().map(|t| t.selection_after);
        self.undo_stack.push(group);
        selection
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Number of undoable groups.
    #[must_use]
    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::cursor::{Cursor, Selection};
    use crate::editor::transaction::EditOperation;

    fn insert_tx(pos: usize, text: &str) -> Transaction {
        Transaction {
            ops: vec![EditOperation::Insert {
                pos,
                text: text.into(),
            }],
            selection_before: Selection::collapsed(Cursor::new(0, pos)),
            selection_after: Selection::collapsed(Cursor::new(0, pos + text.chars().count())),
        }
    }

    #[test]
    fn undo_redo_round_trip() {
        let mut buffer = TextBuffer::from_string("Hello");
        let mut history = EditHistory::default();
        let t = insert_tx(5, " World");
        t.apply(&mut buffer).unwrap();
        history.record(t, false);

        let sel = history.undo(&mut buffer).unwrap();
        assert_eq!(buffer.to_string(), "Hello");
        assert_eq!(sel.head, Cursor::new(0, 5));
        assert!(history.can_redo());

        let sel = history.redo(&mut buffer).unwrap();
        assert_eq!(buffer.to_string(), "Hello World");
        assert_eq!(sel.head, Cursor::new(0, 11));
    }

    #[test]
    fn typing_coalesces_into_one_undo_step() {
        let mut buffer = TextBuffer::default();
        let mut history = EditHistory::default();
        for (i, ch) in ["a", "b", "c"].iter().enumerate() {
            let t = insert_tx(i, ch);
            t.apply(&mut buffer).unwrap();
            history.record(t, true);
        }
        assert_eq!(buffer.to_string(), "abc");
        assert_eq!(history.undo_depth(), 1);
        history.undo(&mut buffer);
        assert_eq!(buffer.to_string(), "");
    }

    #[test]
    fn break_group_splits_undo_steps() {
        let mut buffer = TextBuffer::default();
        let mut history = EditHistory::default();
        let t = insert_tx(0, "a");
        t.apply(&mut buffer).unwrap();
        history.record(t, true);
        history.break_group();
        let t = insert_tx(1, "b");
        t.apply(&mut buffer).unwrap();
        history.record(t, true);

        assert_eq!(history.undo_depth(), 2);
        history.undo(&mut buffer);
        assert_eq!(buffer.to_string(), "a");
        history.undo(&mut buffer);
        assert_eq!(buffer.to_string(), "");
    }

    #[test]
    fn new_edit_invalidates_redo() {
        let mut buffer = TextBuffer::default();
        let mut history = EditHistory::default();
        let t = insert_tx(0, "a");
        t.apply(&mut buffer).unwrap();
        history.record(t, false);
        history.undo(&mut buffer);
        assert!(history.can_redo());

        let t = insert_tx(0, "z");
        t.apply(&mut buffer).unwrap();
        history.record(t, false);
        assert!(!history.can_redo());
        assert_eq!(buffer.to_string(), "z");
    }

    #[test]
    fn undo_on_empty_stack_is_none() {
        let mut buffer = TextBuffer::default();
        let mut history = EditHistory::default();
        assert!(history.undo(&mut buffer).is_none());
        assert!(history.redo(&mut buffer).is_none());
        assert!(!history.can_undo());
    }
}
