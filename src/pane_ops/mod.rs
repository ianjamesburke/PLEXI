//! Pane-operation methods on `PlexiApp`, decomposed by concern:
//!
//! - [`create`] — pane / tile / app / agent creation, plus the app-launch
//!   entry points used by the command palette and HostCommand routing.
//! - [`layout`] — splits, tabs, close, navigation, zoom-free tree
//!   manipulation on already-created panes.
//! - [`workspace`] — multi-context management and on-disk workspace
//!   serialization.
//!
//! Each submodule attaches `impl PlexiApp { ... }` blocks. Methods stay on
//! `PlexiApp` regardless of file, so call sites elsewhere in the crate are
//! unchanged.

mod create;
mod layout;
mod workspace;

pub(crate) use layout::SwapResult;
