//! The DAW command surface: the API downstream agents build against.
//!
//! Every command is self-contained — all data a handler needs is in the
//! command's own fields, never looked up from ambient state.

use serde::{Deserialize, Serialize};

use crate::model::{ClipId, SourceId, TrackId, TrackKind};

/// Result of [`crate::DawModel::apply`].
///
/// - `Applied`: state changed; revision bumped; undoable edits recorded.
/// - `Rejected`: missing id, invalid geometry, or non-finite float — state
///   untouched, no history entry, no revision bump; the reason names what
///   failed.
/// - `NoOp`: valid but changed nothing — no history entry, no revision bump.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplyOutcome {
    Applied,
    Rejected(String),
    NoOp,
}

/// Every edit, mixer, transport, and history command the DAW model accepts.
///
/// Edits to project data (tracks, clips, sources, mixer, tempo) are
/// undoable. Transport commands (`Play`, `Stop`, `Seek`, `SetLoop`) are not
/// undoable and are never restored by `Undo`, but they do bump the revision
/// when they change state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DawCommand {
    /// Set the project tempo. Non-finite is rejected; the value clamps to
    /// [`crate::TEMPO_MIN`]..=[`crate::TEMPO_MAX`].
    SetTempo { bpm: f64 },

    AddTrack { kind: TrackKind, name: String },
    RemoveTrack { track: TrackId },
    RenameTrack { track: TrackId, name: String },

    /// Register a media source. `duration` is in ticks.
    AddSource { kind: TrackKind, path: String, duration: u64 },
    /// Remove a source and every clip referencing it, in one undo group.
    RemoveSource { source: SourceId },

    /// Place a clip on a track. The source kind must match the track kind;
    /// `source_offset + length` must fit inside the source. Clips may
    /// overlap on a DAW track.
    AddClip {
        track: TrackId,
        source: SourceId,
        position: u64,
        length: u64,
        source_offset: u64,
    },
    RemoveClip { clip: ClipId },
    /// Move a clip, possibly to another track when the kinds match.
    MoveClip { clip: ClipId, track: TrackId, position: u64 },
    ResizeClip { clip: ClipId, length: u64, source_offset: u64 },

    /// Mixer sets validate finiteness up front and clamp on apply.
    /// Consecutive sets of the same variant on the same track coalesce into
    /// one undo group until any other command breaks it.
    SetTrackVolume { track: TrackId, volume: f32 },
    SetTrackPan { track: TrackId, pan: f32 },
    SetTrackMute { track: TrackId, mute: bool },
    SetTrackSolo { track: TrackId, solo: bool },

    Play,
    /// Stops playback only; the position is untouched (use `Seek`).
    Stop,
    Seek { position: u64 },
    /// `start < end` is required whenever `enabled` is true.
    SetLoop { enabled: bool, start: u64, end: u64 },

    Undo,
    Redo,
}
