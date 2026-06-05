//! Plexi external app protocol — PGAP v3 (newline-delimited JSON over stdin/stdout).
//!
//! # Protocol overview
//!
//! Binary data (audio PCM, video frames, raw bytes) travels on typed pipes — not stdio.
//! The PGAP wire carries only JSON control/draw messages.

pub mod commands;
pub mod events;
pub mod primitives;
pub mod ui_nodes;
pub mod view;

pub use commands::*;
pub use events::*;
pub use primitives::*;
pub use ui_nodes::*;
