//! Snapshot-based grouped undo/redo, with mixer-set coalescing.
//!
//! Mirrors the editor's `EditHistory` group semantics (cap + coalescing +
//! `break_group`), but each undo group stores a full clone of the project
//! data instead of a transaction list — DAW edits are structural, and the
//! project is small enough that snapshots stay cheap.

use serde::{Deserialize, Serialize};

use crate::model::{Project, TrackId};

/// Maximum retained undo groups; the oldest group is dropped beyond this.
pub const MAX_GROUPS: usize = 500;

/// Identity of an open coalescing group: consecutive `Applied` mixer sets of
/// the same variant on the same track merge into one undo step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoalesceKey {
    Volume(TrackId),
    Pan(TrackId),
}

/// Undo/redo stacks of project snapshots. Each entry is the project state
/// *before* one user-visible undo step; recording clears the redo stack.
#[derive(Debug, Clone, Default)]
pub struct SnapshotHistory {
    undo_stack: Vec<Project>,
    redo_stack: Vec<Project>,
    /// When set, the top undo group may absorb the next record carrying the
    /// same key (the group's snapshot already predates the whole run).
    open_key: Option<CoalesceKey>,
}

impl SnapshotHistory {
    /// Records the pre-edit snapshot of one applied edit. When `key` matches
    /// the open group, the record coalesces: the existing snapshot already
    /// captures the state before the run of sets, so nothing is pushed. Any
    /// record invalidates the redo stack.
    pub fn record(&mut self, snapshot: Project, key: Option<CoalesceKey>) {
        self.redo_stack.clear();
        if key.is_some() && key == self.open_key {
            return;
        }
        self.undo_stack.push(snapshot);
        if self.undo_stack.len() > MAX_GROUPS {
            self.undo_stack.remove(0);
        }
        self.open_key = key;
    }

    /// Ends the current coalescing group; the next record starts a new undo
    /// step. Called by every applied non-coalescing command (including
    /// transport changes).
    pub fn break_group(&mut self) {
        self.open_key = None;
    }

    /// Pops the top undo snapshot, pushing `current` onto the redo stack.
    /// `None` when there is nothing to undo.
    pub fn undo(&mut self, current: &Project) -> Option<Project> {
        self.open_key = None;
        let snapshot = self.undo_stack.pop()?;
        self.redo_stack.push(current.clone());
        Some(snapshot)
    }

    /// Pops the top redo snapshot, pushing `current` onto the undo stack.
    /// `None` when there is nothing to redo.
    pub fn redo(&mut self, current: &Project) -> Option<Project> {
        self.open_key = None;
        let snapshot = self.redo_stack.pop()?;
        self.undo_stack.push(current.clone());
        Some(snapshot)
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
