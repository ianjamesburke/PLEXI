//! Pure video-editor edit model (stint 0523): commands in, state out.
//!
//! Zero egui, zero host types, zero I/O. The model owns project data
//! (sources plus exactly two fixed tracks — video and audio), a playhead
//! and selection, snapshot-based undo/redo, and a monotonic revision
//! counter. The export substrate is [`Project::cut_list`]. Unit conversion
//! to frames is the downstream engine's concern.
//!
//! Sibling crate: `plexi-daw-model` (stint 0514) shares the same module
//! shape and conventions.

pub mod commands;
pub mod history;
pub mod model;

mod gate;

pub use commands::{ApplyOutcome, VideoCommand};
pub use history::{SnapshotHistory, MAX_GROUPS};
pub use model::{
    Clip, ClipId, CutEntry, Fps, Project, Source, SourceId, Track, TrackKind, ValidationError,
    VideoModel, TICKS_PER_SECOND,
};
