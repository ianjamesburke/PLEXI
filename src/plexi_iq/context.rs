//! Tool execution context.
//!
//! Stage 0: stub fields for pane ID and directory scope. Stage 1 will
//! add the rest per spec §3.3:
//!
//! - `session: &SessionState` — Read-before-Edit guard lives here
//! - `app_bus: Option<&AppBus>` — gates app-protocol tools when the
//!   pane has a companion app
//! - `plexi_ctx: &PlexiCtx` — handle for the subagent `Task` tool to
//!   spawn new panes and look up the pane tree
//!
//! These are all left as comments for now because the concrete types
//! (`SessionState`, `AppBus`, `PlexiCtx`) haven't been introduced yet.

use std::path::PathBuf;

/// Opaque pane identifier. Stage 1 will wire this to the real `PaneId`
/// type from `src/pane.rs`; for now it's a newtype wrapper so the
/// signature of `ToolContext` can settle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PaneId(pub u64);

/// Everything a `Tool::run` call needs. Passed by reference so tools
/// never own pane/session state.
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// Which pane this agent instance is bound to.
    pub pane_id: PaneId,

    /// Filesystem scope for file-touching tools. Tools must refuse
    /// absolute paths that escape this directory (Stage 1 enforcement).
    pub directory_scope: PathBuf,
    // TODO (Stage 1, spec §3.3):
    //   pub session: &'a SessionState,
    //   pub app_bus: Option<&'a AppBus>,
    //   pub plexi_ctx: &'a PlexiCtx,
    //
    // These require types that don't exist yet in this worktree. They
    // land in Stage 1 alongside the turn-loop driver in `loop.rs`.
}

impl ToolContext {
    /// Construct a minimal context. Stage 1 will replace this with a
    /// real builder once the session/app_bus/plexi_ctx fields land.
    pub fn new(pane_id: PaneId, directory_scope: PathBuf) -> Self {
        Self {
            pane_id,
            directory_scope,
        }
    }
}
