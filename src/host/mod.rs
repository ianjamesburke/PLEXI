//! Host runtime model and deterministic harness.
//!
//! This module is intentionally egui-free so keyboard and pane orchestration
//! can be regression-tested without the GUI event loop.

/// Hard ceiling on a single app file read or write, enforced host-side in both
/// app runtimes (CPython bridge and WASM component). Keeps one runaway
/// `file_write`/`file_read` from ballooning the JSON bridge or host memory;
/// larger media transfers need a streaming seam, not a bigger cap.
pub const MAX_FILE_IO_BYTES: usize = 64 * 1024 * 1024;

pub mod anchor;
pub mod app_timeline;
pub mod command;
pub mod context;
pub mod context_state;
pub mod effect;
pub mod event_log;
pub mod event_subscriptions;
pub mod hot_reload;
pub mod keys;
pub mod mcp_client;
pub mod launch_failed;
pub mod model;
pub mod pane;
pub mod scheduler;
pub mod scope;
pub mod services;
pub mod shell;
pub mod state_scope;
pub mod state_watch;
pub mod typed_pipes;
pub mod wasm_app;
pub mod wasm_frame;
pub mod wasm_gpu;
pub mod wasm_pane;
pub mod wasm_python;
pub mod wasm_render;
