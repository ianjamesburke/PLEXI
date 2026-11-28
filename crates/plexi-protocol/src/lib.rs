//! Plexi external plugin protocol — PGAP (version: `PROTOCOL_VERSION`; newline-delimited JSON over stdin/stdout).
//!
//! # Protocol overview
//!
//! Binary data (audio PCM, video frames, raw bytes) travels on typed pipes — not stdio.
//! The PGAP wire carries only JSON control/draw messages.

pub mod commands;
pub mod events;
pub mod primitives;

/// The PGAP version this host speaks. It is sent in every Python `init`, and
/// the SDK refuses to start against another version. Bump it with any wire
/// change an older SDK cannot read; the schema and the SDK's
/// `PROTOCOL_VERSION` are generated from it.
pub const PROTOCOL_VERSION: &str = "pgap/5";

pub use commands::*;
pub use events::*;
pub use primitives::*;
