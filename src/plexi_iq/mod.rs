//! Plexi IQ — in-process agent harness.
//!
//! Stage 0 scaffolding. See `docs/specs/plexi-iq.md` §3 for the module
//! layout and §9 for the staging plan. Nothing in this module is wired
//! into the running application yet — `AgentMode` still shells out via
//! the existing `claude -p --resume` path. This tree exists purely so
//! Stage 1 has a clean foundation to start building on.
//!
//! Re-exports below expose the public types (`PlexiIq`, `PlexiIqConfig`,
//! `PlexiIqInstance`) that Stage 1 will flesh out.

#![allow(dead_code)] // Stage 0: stubs only.

pub mod backend;
pub mod context;
#[path = "loop.rs"]
pub mod turn_loop;
pub mod prompt;
pub mod tools;

pub use backend::{BillingModel, LlmBackend};
pub use context::ToolContext;
pub use tools::{Tool, ToolRegistry, ToolResult};

use std::path::PathBuf;

/// Top-level Plexi IQ handle — owns shared configuration and spawns per-pane
/// `PlexiIqInstance`s. Stage 1 will give this real fields (backend factory,
/// global budget ledger handle, MCP client pool, etc.). For now it is a
/// zero-sized marker so downstream code can name the type.
#[derive(Debug, Default)]
pub struct PlexiIq {}

/// Configuration passed when constructing a `PlexiIq`. See spec §3.6 for the
/// backend-selection logic and §10 for budget fields that will land here.
#[derive(Debug, Clone, Default)]
pub struct PlexiIqConfig {
    /// Optional override for the directory scope used when bootstrapping
    /// instances. When `None`, instances will inherit the pane's cwd.
    pub default_directory_scope: Option<PathBuf>,
}

/// Per-pane agent instance. Owns its own conversation, tool registry, and
/// session state. Stage 1 will add the turn loop driver and the channels that
/// connect it to the pane's `AgentMode` state machine.
#[derive(Debug, Default)]
pub struct PlexiIqInstance {}
