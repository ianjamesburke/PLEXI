//! The video-editor command surface: the API downstream agents build
//! against.
//!
//! Every command is self-contained — all data a handler needs is in the
//! command's own fields, never looked up from ambient state.

use serde::{Deserialize, Serialize};

use crate::model::{ClipId, Fps, SourceId, TrackKind};

/// Result of [`crate::VideoModel::apply`].
///
/// - `Applied`: state changed; revision bumped; undoable edits recorded.
/// - `Rejected`: missing id, invalid geometry (overlap, `in >= out`,
///   overflow), or invalid fps — state untouched, no history entry, no
///   revision bump; the reason names what failed.
/// - `NoOp`: valid but changed nothing — no history entry, no revision bump.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplyOutcome {
    Applied,
    Rejected(String),
    NoOp,
}

/// Every edit, playhead, selection, and history command the video model
/// accepts.
///
/// Edits to project data (sources, clips) are undoable. Playhead and
/// selection changes are not undoable and are never restored by `Undo`, but
/// they do bump the revision when they change state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoCommand {
    /// Register a media source. `duration` is in ticks (milliseconds).
    AddSource { path: String, duration: u64, fps: Fps },
    /// Remove a source and every clip referencing it on both tracks, in one
    /// undo group. Clears the selection if it pointed at a removed clip.
    RemoveSource { source: SourceId },

    /// Place a clip on the fixed video or audio track. The timeline length
    /// is `source_out - source_in` (no retime in v1). Rejected if it would
    /// overlap an existing clip on that track — video-editor tracks never
    /// overlap.
    AddClip {
        track: TrackKind,
        source: SourceId,
        source_in: u64,
        source_out: u64,
        position: u64,
    },
    /// Plain lift — leaves a gap.
    RemoveClip { clip: ClipId },
    /// Removes the clip and shifts every later clip on the same track left
    /// by the removed clip's length, in one undo group.
    RippleDelete { clip: ClipId },
    /// Same-track move; rejected on overlap.
    MoveClip { clip: ClipId, position: u64 },
    /// Trim the clip's in point. Content stays timeline-aligned: the
    /// position shifts by `new_in - old_in` (checked arithmetic).
    TrimIn { clip: ClipId, source_in: u64 },
    /// Trim the clip's out point. The position is fixed; the clip end moves.
    TrimOut { clip: ClipId, source_out: u64 },
    /// Split at a time strictly inside the clip's timeline span. Both
    /// halves get fresh ids (ids are never aliased); the selection moves to
    /// the left half if the split clip was selected.
    SplitAt { clip: ClipId, time: u64 },

    SetPlayhead { position: u64 },
    /// `Some(id)` is rejected when the clip does not exist.
    Select { clip: Option<ClipId> },

    Undo,
    Redo,
}
