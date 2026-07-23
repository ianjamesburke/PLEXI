//! Snapshot-based grouped undo/redo.
//!
//! Mirrors `plexi-daw-model`'s history minus coalescing: the video model
//! has no continuous controls, so every applied edit is one undo group.
//! Each group stores a full clone of the project data (never playhead or
//! selection).

use crate::model::Project;

/// Maximum retained undo groups; the oldest group is dropped beyond this.
pub const MAX_GROUPS: usize = 500;

/// Undo/redo stacks of project snapshots. Each entry is the project state
/// *before* one user-visible undo step; recording clears the redo stack.
#[derive(Debug, Clone, Default)]
pub struct SnapshotHistory {
    undo_stack: Vec<Project>,
    redo_stack: Vec<Project>,
}

impl SnapshotHistory {
    /// Records the pre-edit snapshot of one applied edit. Any record
    /// invalidates the redo stack.
    pub fn record(&mut self, snapshot: Project) {
        self.redo_stack.clear();
        self.undo_stack.push(snapshot);
        if self.undo_stack.len() > MAX_GROUPS {
            self.undo_stack.remove(0);
        }
    }

    /// Pops the top undo snapshot, pushing `current` onto the redo stack.
    /// `None` when there is nothing to undo.
    pub fn undo(&mut self, current: &Project) -> Option<Project> {
        let snapshot = self.undo_stack.pop()?;
        self.redo_stack.push(current.clone());
        Some(snapshot)
    }

    /// Pops the top redo snapshot, pushing `current` onto the undo stack.
    /// `None` when there is nothing to redo.
    pub fn redo(&mut self, current: &Project) -> Option<Project> {
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
