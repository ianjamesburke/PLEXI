//! Edit operations and transactions — the single mutation path.
//!
//! Adapted from Ferrite <https://github.com/OlaProeis/Ferrite>
//! @ 3ba085c561670342d72c560efbf6b0b92b5c0b46 (history.rs), MIT.

use super::buffer::TextBuffer;
use super::cursor::Selection;

/// A single reversible edit. Positions are char indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOperation {
    Insert { pos: usize, text: String },
    Delete { pos: usize, text: String },
}

impl EditOperation {
    /// The inverse operation (insert ↔ delete).
    #[must_use]
    pub fn inverse(&self) -> Self {
        match self {
            Self::Insert { pos, text } => Self::Delete {
                pos: *pos,
                text: text.clone(),
            },
            Self::Delete { pos, text } => Self::Insert {
                pos: *pos,
                text: text.clone(),
            },
        }
    }

    /// Applies this operation, validating bounds and (for deletes) that the
    /// buffer actually contains the recorded text.
    pub fn apply(&self, buffer: &mut TextBuffer) -> Result<(), String> {
        match self {
            Self::Insert { pos, text } => {
                if *pos > buffer.len() {
                    return Err(format!(
                        "insert at {pos} out of bounds (len {})",
                        buffer.len()
                    ));
                }
                buffer.insert(*pos, text);
                Ok(())
            }
            Self::Delete { pos, text } => {
                let n = text.chars().count();
                if pos + n > buffer.len() {
                    return Err(format!(
                        "delete {pos}..{} out of bounds (len {})",
                        pos + n,
                        buffer.len()
                    ));
                }
                let actual = buffer.slice(*pos, pos + n);
                if actual != *text {
                    return Err(format!(
                        "delete at {pos} mismatch: expected {text:?}, buffer has {actual:?}"
                    ));
                }
                buffer.remove(*pos, n);
                Ok(())
            }
        }
    }
}

/// One user-visible edit step: an ordered batch of operations plus the
/// selection to restore on undo (`selection_before`) and redo
/// (`selection_after`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub ops: Vec<EditOperation>,
    pub selection_before: Selection,
    pub selection_after: Selection,
}

impl Transaction {
    /// Applies all operations in order.
    pub fn apply(&self, buffer: &mut TextBuffer) -> Result<(), String> {
        for op in &self.ops {
            op.apply(buffer)?;
        }
        Ok(())
    }

    /// Applies the inverse of all operations in reverse order (undo).
    pub fn revert(&self, buffer: &mut TextBuffer) -> Result<(), String> {
        for op in self.ops.iter().rev() {
            op.inverse().apply(buffer)?;
        }
        Ok(())
    }
}

/// Minimal edit operations transforming `old` into `new` via prefix/suffix
/// matching: at most one Delete followed by one Insert. Char-indexed.
/// Test-only until a production caller lands.
#[cfg(test)]
#[must_use]
pub fn compute_edit_ops(old: &str, new: &str) -> Vec<EditOperation> {
    if old == new {
        return Vec::new();
    }

    let old_chars: Vec<char> = old.chars().collect();
    let new_chars: Vec<char> = new.chars().collect();

    let prefix = old_chars
        .iter()
        .zip(&new_chars)
        .take_while(|(a, b)| a == b)
        .count();

    let max_suffix = old_chars.len().min(new_chars.len()).saturating_sub(prefix);
    let suffix = old_chars
        .iter()
        .rev()
        .zip(new_chars.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();

    let del_end = old_chars.len() - suffix;
    let ins_end = new_chars.len() - suffix;

    let mut ops = Vec::new();
    if del_end > prefix {
        ops.push(EditOperation::Delete {
            pos: prefix,
            text: old_chars[prefix..del_end].iter().collect(),
        });
    }
    if ins_end > prefix {
        ops.push(EditOperation::Insert {
            pos: prefix,
            text: new_chars[prefix..ins_end].iter().collect(),
        });
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::cursor::{Cursor, Selection};

    fn tx(ops: Vec<EditOperation>) -> Transaction {
        Transaction {
            ops,
            selection_before: Selection::collapsed(Cursor::default()),
            selection_after: Selection::collapsed(Cursor::default()),
        }
    }

    #[test]
    fn inverse_round_trips() {
        let op = EditOperation::Insert {
            pos: 3,
            text: "abc".into(),
        };
        assert_eq!(op.inverse().inverse(), op);
    }

    #[test]
    fn transaction_apply_revert_round_trip() {
        let mut buffer = TextBuffer::from_string("Hello World");
        let t = tx(vec![
            EditOperation::Delete {
                pos: 5,
                text: " World".into(),
            },
            EditOperation::Insert {
                pos: 5,
                text: ", Plexi".into(),
            },
        ]);
        t.apply(&mut buffer).unwrap();
        assert_eq!(buffer.to_string(), "Hello, Plexi");
        t.revert(&mut buffer).unwrap();
        assert_eq!(buffer.to_string(), "Hello World");
    }

    #[test]
    fn unicode_transaction_round_trip() {
        let mut buffer = TextBuffer::from_string("héllo 🌍");
        let t = tx(vec![EditOperation::Delete {
            pos: 6,
            text: "🌍".into(),
        }]);
        t.apply(&mut buffer).unwrap();
        assert_eq!(buffer.to_string(), "héllo ");
        t.revert(&mut buffer).unwrap();
        assert_eq!(buffer.to_string(), "héllo 🌍");
    }

    #[test]
    fn apply_rejects_out_of_bounds_insert() {
        let mut buffer = TextBuffer::from_string("hi");
        let op = EditOperation::Insert {
            pos: 5,
            text: "x".into(),
        };
        assert!(op.apply(&mut buffer).is_err());
        assert_eq!(buffer.to_string(), "hi");
    }

    #[test]
    fn apply_rejects_mismatched_delete() {
        let mut buffer = TextBuffer::from_string("hello");
        let op = EditOperation::Delete {
            pos: 0,
            text: "world".into(),
        };
        assert!(op.apply(&mut buffer).is_err());
        assert_eq!(buffer.to_string(), "hello");
    }

    #[test]
    fn compute_edit_ops_replace() {
        let ops = compute_edit_ops("Hello World", "Hello Plexi");
        assert_eq!(
            ops,
            vec![
                EditOperation::Delete {
                    pos: 6,
                    text: "World".into()
                },
                EditOperation::Insert {
                    pos: 6,
                    text: "Plexi".into()
                },
            ]
        );
    }

    #[test]
    fn compute_edit_ops_identity_and_pure_edits() {
        assert!(compute_edit_ops("same", "same").is_empty());
        assert_eq!(
            compute_edit_ops("ab", "axb"),
            vec![EditOperation::Insert {
                pos: 1,
                text: "x".into()
            }]
        );
        assert_eq!(
            compute_edit_ops("axb", "ab"),
            vec![EditOperation::Delete {
                pos: 1,
                text: "x".into()
            }]
        );
    }

    #[test]
    fn compute_edit_ops_applies_to_produce_new() {
        let old = "line one\nline two\nline three";
        let new = "line one\nLINE 2\nline three";
        let mut buffer = TextBuffer::from_string(old);
        for op in compute_edit_ops(old, new) {
            op.apply(&mut buffer).unwrap();
        }
        assert_eq!(buffer.to_string(), new);
    }
}
