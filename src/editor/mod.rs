//! Shared editor core, derived from Ferrite.
//!
//! Upstream: <https://github.com/OlaProeis/Ferrite>
//! Commit: 3ba085c561670342d72c560efbf6b0b92b5c0b46
//! License: MIT (see `src/editor/LICENSE-ferrite`, © 2024-2025 OlaProeis).
//!
//! Divergence from upstream:
//! - Only the pure editing core is ported (buffer, cursor/selection, grapheme
//!   boundaries, movement, transaction-grouped undo/redo, viewport math).
//!   Syntax highlighting, folding, minimap, vim mode, search/replace, line
//!   numbers, gutter, outline, and stats were dropped.
//! - Ferrite's 4k-line `editor.rs` orchestrator is replaced by a small
//!   `Document` state machine ([`commands`]) with an explicit
//!   [`EditorCommand`] API; all mutation routes through recorded
//!   [`Transaction`]s ([`transaction`], [`history`]).
//! - IME preedit text is never inserted into the buffer; it lives in
//!   [`ime::ImeState`] and is painted as an overlay, so the buffer only ever
//!   changes through transactions.
//! - `widget.rs` is the only egui-dependent file.

pub mod buffer;
pub mod commands;
pub mod gate;
pub mod cursor;
pub mod grapheme;
pub mod highlight;
pub mod history;
pub mod ime;
pub mod markdown;
pub mod mode;
pub mod movement;
pub mod preview;
pub mod selection;
pub mod transaction;
pub mod view;
pub mod widget;

pub use commands::{Document, EditorCommand};
pub use cursor::{Cursor, Selection};
pub use highlight::SyntaxHighlighter;
pub use mode::EditorMode;
pub use view::ViewState;

/// Snapshot of the editor's semantic state, for host inspection and tests.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct EditorSemanticState {
    /// Full document text.
    pub text: String,
    /// Current selection (anchor/head).
    pub selection: Selection,
    /// Caret position (selection head).
    pub cursor: Cursor,
    /// Number of undoable groups.
    pub undo_depth: usize,
    pub can_undo: bool,
    pub can_redo: bool,
    /// In-progress IME composition text, if any.
    pub ime_composition: Option<String>,
    /// Vertical scroll offset in points.
    pub scroll_y: f32,
}
