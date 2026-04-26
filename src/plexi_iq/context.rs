//! Minimal tool-execution context for Plexi IQ.
//!
//! Only the two fields `loop.rs` / backends actually read today. Additional
//! fields (session state, app bus, subagent spawn handle) land when a tool
//! that needs them is introduced.

use std::path::PathBuf;

/// Everything a tool invocation needs. Passed by reference so tools never
/// own pane or session state.
#[derive(Debug, Clone)]
pub struct ToolContext {
    pub pane_id: crate::tiling::PaneId,
    pub directory_scope: PathBuf,
}

impl ToolContext {
    pub fn new(pane_id: crate::tiling::PaneId, directory_scope: PathBuf) -> Self {
        Self {
            pane_id,
            directory_scope,
        }
    }
}
