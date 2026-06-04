//! Host runtime model and deterministic harness.
//!
//! This module is intentionally egui-free so keyboard and pane orchestration
//! can be regression-tested without the GUI event loop.

pub mod command;
pub mod effect;
pub mod model;
pub mod services;
pub mod context;
pub mod event_log;
pub mod hot_reload;
pub mod launch_failed;
pub mod pane;
pub mod runs;
pub mod scheduler;
pub mod shell;
pub mod typed_pipes;
pub mod anchor;
pub mod context_state;
pub mod keys;
