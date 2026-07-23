//! Pure DAW edit model (stint 0514): commands in, state out.
//!
//! Zero egui, zero host types, zero I/O. The model owns project data
//! (tracks, clips, sources, mixer, tempo), a transport, snapshot-based
//! undo/redo, and a monotonic revision counter. Downstream engines convert
//! ticks to samples; the model never does.
//!
//! Sibling crate: `plexi-video-model` (stint 0523) shares the same module
//! shape and conventions.

pub mod commands;
pub mod history;
pub mod model;

mod gate;

pub use commands::{ApplyOutcome, DawCommand};
pub use history::{CoalesceKey, SnapshotHistory, MAX_GROUPS};
pub use model::{
    Clip, ClipId, DawModel, Mixer, Project, Source, SourceId, Track, TrackId, TrackKind,
    Transport, ValidationError, PAN_MAX, PAN_MIN, TEMPO_MAX, TEMPO_MIN, TICKS_PER_BEAT,
    VOLUME_MAX, VOLUME_MIN,
};
