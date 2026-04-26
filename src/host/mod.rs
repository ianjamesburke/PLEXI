//! Host runtime model and deterministic harness.
//!
//! This module is intentionally egui-free so keyboard and pane orchestration
//! can be regression-tested without the GUI event loop.

pub mod command;
pub mod effect;
pub mod model;
pub mod services;
