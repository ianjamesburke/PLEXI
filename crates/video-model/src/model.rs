//! Video-editor project state and the pure `apply` state machine.
//!
//! [`Project`] is the serialized project data (sources, the two fixed
//! tracks, id counter). [`VideoModel`] wraps it with the playhead,
//! selection, revision counter, and snapshot history — none of which belong
//! in the serialized project or in undo snapshots.

use serde::{Deserialize, Serialize};

use crate::commands::{ApplyOutcome, VideoCommand};
use crate::history::SnapshotHistory;

/// Timeline resolution: all video-model times are `u64` ticks at this rate
/// (a millisecond timeline). Conversion to frames is the downstream
/// engine's concern.
pub const TICKS_PER_SECOND: u64 = 1000;

// ─── Validation ──────────────────────────────────────────────────────────────

/// One state-validity violation, typed per violation class.
///
/// This is the single validation path for persisted or foreign state:
/// [`VideoModel::from_parts`] (bundle loading) and the release gate's
/// invariant checks both run [`Project::validate`] /
/// [`VideoModel::validate`] — the same rules everywhere, never a second
/// hand-rolled checker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    ZeroFpsTerm { source: SourceId, num: u32, den: u32 },
    MissingClipSource { clip: ClipId, source: SourceId },
    EmptyClipWindow { clip: ClipId, source_in: u64, source_out: u64 },
    ClipWindowExceedsSource { clip: ClipId, source_out: u64, duration: u64 },
    ClipTimelineOverflow { clip: ClipId },
    OverlappingClips { track: TrackKind, first: ClipId, second: ClipId },
    DuplicateId { id: u64 },
    IdNotBelowNextId { id: u64, next_id: u64 },
    DanglingSelection { clip: ClipId },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroFpsTerm { source, num, den } => write!(
                f,
                "source {} fps {num}/{den} has a zero term",
                source.0
            ),
            Self::MissingClipSource { clip, source } => write!(
                f,
                "clip {} references missing source {}",
                clip.0, source.0
            ),
            Self::EmptyClipWindow {
                clip,
                source_in,
                source_out,
            } => write!(
                f,
                "clip {} window {source_in}..{source_out} is empty or inverted",
                clip.0
            ),
            Self::ClipWindowExceedsSource {
                clip,
                source_out,
                duration,
            } => write!(
                f,
                "clip {} window end {source_out} exceeds source duration {duration}",
                clip.0
            ),
            Self::ClipTimelineOverflow { clip } => {
                write!(f, "clip {} timeline end overflows u64", clip.0)
            }
            Self::OverlappingClips {
                track,
                first,
                second,
            } => write!(
                f,
                "{track:?} clips {} and {} overlap on the timeline",
                first.0, second.0
            ),
            Self::DuplicateId { id } => write!(f, "duplicate entity id {id}"),
            Self::IdNotBelowNextId { id, next_id } => write!(
                f,
                "id {id} >= next_id {next_id}; allocator lost monotonicity"
            ),
            Self::DanglingSelection { clip } => {
                write!(f, "selection points at missing clip {}", clip.0)
            }
        }
    }
}

impl std::error::Error for ValidationError {}

// ─── Ids ─────────────────────────────────────────────────────────────────────

/// Newtype ids allocated by [`Project::next_id`]; never reused, even across
/// undo (the counter survives snapshot restores).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ClipId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SourceId(pub u64);

// ─── Project data ────────────────────────────────────────────────────────────

/// v1 has exactly two fixed tracks; this addresses them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrackKind {
    Video,
    Audio,
}

/// Exact rational frame rate (e.g. 30000/1001). Both terms are > 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fps {
    pub num: u32,
    pub den: u32,
}

/// A media source. `duration` is in ticks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub id: SourceId,
    pub path: String,
    pub duration: u64,
    pub fps: Fps,
}

/// A clip on the timeline. Timeline length is `source_out - source_in`
/// (no retime in v1); `source_in < source_out <= source.duration`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub source: SourceId,
    pub source_in: u64,
    pub source_out: u64,
    /// Timeline start, in ticks.
    pub position: u64,
}

impl Clip {
    /// Timeline (and source-window) length; invariants keep `in < out`.
    #[must_use]
    pub fn length(&self) -> u64 {
        self.source_out.saturating_sub(self.source_in)
    }

    /// Exclusive timeline end; invariants keep this overflow-free.
    #[must_use]
    pub fn timeline_end(&self) -> u64 {
        self.position.saturating_add(self.length())
    }
}

/// One fixed track. v1 has exactly two ([`Project::video`],
/// [`Project::audio`]) — a struct field each, never a growable list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub clips: Vec<Clip>,
}

/// One entry of the export cut list — the substrate for `media.export`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutEntry {
    pub source_path: String,
    pub source_in: u64,
    pub source_out: u64,
    pub position: u64,
}

/// Serialized project data — exactly what undo snapshots clone and what
/// [`Project::to_json`] emits. Playhead, selection, revision, and history
/// live on [`VideoModel`], outside this struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub sources: Vec<Source>,
    pub video: Track,
    pub audio: Track,
    /// Monotonic id allocator; part of serialized state, deliberately NOT
    /// restored by undo/redo. Restoring it would let a post-undo edit mint
    /// an id aliasing one already handed to external holders (assistant
    /// tools, UI) from the undone branch — a stale reference would silently
    /// rebind to a new object instead of failing as dangling. Monotonic ids
    /// keep staleness detectable, and `next_id` stays a deterministic
    /// function of the applied command history, so bundle serialization
    /// stays deterministic.
    pub next_id: u64,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            video: Track::default(),
            audio: Track::default(),
            next_id: 1,
        }
    }
}

impl Project {
    /// Pretty JSON of the project data. Errors name the failure.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| format!("video project serialization failed: {e}"))
    }

    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json)
            .map_err(|e| format!("video project deserialization failed: {e}"))
    }

    #[must_use]
    pub fn track(&self, kind: TrackKind) -> &Track {
        match kind {
            TrackKind::Video => &self.video,
            TrackKind::Audio => &self.audio,
        }
    }

    fn track_mut(&mut self, kind: TrackKind) -> &mut Track {
        match kind {
            TrackKind::Video => &mut self.video,
            TrackKind::Audio => &mut self.audio,
        }
    }

    #[must_use]
    pub fn source(&self, id: SourceId) -> Option<&Source> {
        self.sources.iter().find(|s| s.id == id)
    }

    /// Locates a clip as `(track, clip index)`.
    #[must_use]
    pub fn find_clip(&self, id: ClipId) -> Option<(TrackKind, usize)> {
        for kind in [TrackKind::Video, TrackKind::Audio] {
            if let Some(index) = self.track(kind).clips.iter().position(|c| c.id == id) {
                return Some((kind, index));
            }
        }
        None
    }

    /// Total clips across both tracks.
    #[must_use]
    pub fn clip_count(&self) -> usize {
        self.video.clips.len() + self.audio.clips.len()
    }

    /// The export cut list for one track: every clip resolved to its source
    /// path and window, sorted by timeline position. Pure derivation — the
    /// substrate for `media.export`.
    #[must_use]
    pub fn cut_list(&self, track: TrackKind) -> Vec<CutEntry> {
        let mut entries: Vec<CutEntry> = self
            .track(track)
            .clips
            .iter()
            .filter_map(|clip| match self.source(clip.source) {
                Some(source) => Some(CutEntry {
                    source_path: source.path.clone(),
                    source_in: clip.source_in,
                    source_out: clip.source_out,
                    position: clip.position,
                }),
                None => {
                    // Unreachable while invariants hold; never silently
                    // fabricate an entry for a dangling reference.
                    log::error!(
                        "cut_list: clip {} references missing source {}",
                        clip.id.0,
                        clip.source.0
                    );
                    None
                }
            })
            .collect();
        entries.sort_by_key(|e| e.position);
        entries
    }

    /// Full state-validity check over the project data: per-track overlap
    /// freedom, source references, window and overflow geometry, fps
    /// terms, id uniqueness and allocator monotonicity. The single shared
    /// path for bundle loading ([`VideoModel::from_parts`]) and the
    /// release gate's invariants.
    pub fn validate(&self) -> Result<(), ValidationError> {
        for source in &self.sources {
            if source.fps.num == 0 || source.fps.den == 0 {
                return Err(ValidationError::ZeroFpsTerm {
                    source: source.id,
                    num: source.fps.num,
                    den: source.fps.den,
                });
            }
        }
        let mut ids: Vec<u64> = self.sources.iter().map(|s| s.id.0).collect();
        for kind in [TrackKind::Video, TrackKind::Audio] {
            let track = self.track(kind);
            for clip in &track.clips {
                ids.push(clip.id.0);
                let Some(source) = self.source(clip.source) else {
                    return Err(ValidationError::MissingClipSource {
                        clip: clip.id,
                        source: clip.source,
                    });
                };
                if clip.source_in >= clip.source_out {
                    return Err(ValidationError::EmptyClipWindow {
                        clip: clip.id,
                        source_in: clip.source_in,
                        source_out: clip.source_out,
                    });
                }
                if clip.source_out > source.duration {
                    return Err(ValidationError::ClipWindowExceedsSource {
                        clip: clip.id,
                        source_out: clip.source_out,
                        duration: source.duration,
                    });
                }
                if clip
                    .position
                    .checked_add(clip.source_out - clip.source_in)
                    .is_none()
                {
                    return Err(ValidationError::ClipTimelineOverflow { clip: clip.id });
                }
            }
            // No overlaps within a track: sort by position, check neighbors.
            let mut spans: Vec<(u64, u64, u64)> = track
                .clips
                .iter()
                .map(|c| (c.position, c.timeline_end(), c.id.0))
                .collect();
            spans.sort_unstable();
            for pair in spans.windows(2) {
                if pair[1].0 < pair[0].1 {
                    return Err(ValidationError::OverlappingClips {
                        track: kind,
                        first: ClipId(pair[0].2),
                        second: ClipId(pair[1].2),
                    });
                }
            }
        }
        ids.sort_unstable();
        if let Some(pair) = ids.windows(2).find(|w| w[0] == w[1]) {
            return Err(ValidationError::DuplicateId { id: pair[0] });
        }
        if let Some(&max) = ids.last() {
            if max >= self.next_id {
                return Err(ValidationError::IdNotBelowNextId {
                    id: max,
                    next_id: self.next_id,
                });
            }
        }
        Ok(())
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Returns the exclusive timeline end of `[position, position+length)`
    /// after checking overflow and overlap against every clip on `kind`
    /// other than `exclude`.
    fn check_placement(
        &self,
        what: &str,
        kind: TrackKind,
        position: u64,
        length: u64,
        exclude: Option<ClipId>,
    ) -> Result<u64, String> {
        let end = position.checked_add(length).ok_or_else(|| {
            format!("{what}: position {position} + length {length} overflows u64")
        })?;
        for clip in &self.track(kind).clips {
            if Some(clip.id) == exclude {
                continue;
            }
            if position < clip.timeline_end() && clip.position < end {
                return Err(format!(
                    "{what}: [{position}, {end}) overlaps clip {} at [{}, {})",
                    clip.id.0,
                    clip.position,
                    clip.timeline_end()
                ));
            }
        }
        Ok(end)
    }
}

// ─── Model ───────────────────────────────────────────────────────────────────

/// The video edit model: project data plus playhead, selection, revision,
/// and history. Commands in ([`VideoCommand`]), state out — no I/O, no
/// rendering.
#[derive(Debug, Clone, Default)]
pub struct VideoModel {
    project: Project,
    /// Not undoable, never snapshotted; changes bump the revision.
    playhead: u64,
    /// Not undoable, never snapshotted. Always points at an existing clip
    /// (restores that would dangle it clear it instead).
    selection: Option<ClipId>,
    /// Monotonic change counter, bumped on every `Applied` outcome
    /// (including undo/redo and playhead/selection changes). Starts at 0.
    revision: u64,
    history: SnapshotHistory,
}

impl VideoModel {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Restores a model from decoded state — the load half of persistence
    /// ([`Project::from_json`] plus the caller-persisted playhead and
    /// selection). Rejects invariant-violating state (corrupted or
    /// hand-edited bundles) through the shared [`Project::validate`] path;
    /// a persisted selection that does not resolve to a clip is likewise
    /// rejected ([`ValidationError::DanglingSelection`]) — the constructor
    /// never repairs invalid input (silent pruning is reserved for live
    /// transitions like undo, where the model itself made the clip
    /// disappear). History starts empty and the revision at 0: undo never
    /// spans sessions, and the revision counts changes within one session
    /// only.
    pub fn from_parts(
        project: Project,
        playhead: u64,
        selection: Option<ClipId>,
    ) -> Result<Self, ValidationError> {
        let model = Self {
            project,
            playhead,
            selection,
            revision: 0,
            history: SnapshotHistory::default(),
        };
        model.validate()?;
        Ok(model)
    }

    /// Full state-validity check over project data plus the selection —
    /// the same rules [`from_parts`](Self::from_parts) enforces on load
    /// and the release gate re-checks after every command.
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.project.validate()?;
        if let Some(selected) = self.selection {
            if self.project.find_clip(selected).is_none() {
                return Err(ValidationError::DanglingSelection { clip: selected });
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn project(&self) -> &Project {
        &self.project
    }

    #[must_use]
    pub fn playhead(&self) -> u64 {
        self.playhead
    }

    #[must_use]
    pub fn selection(&self) -> Option<ClipId> {
        self.selection
    }

    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    #[must_use]
    pub fn undo_depth(&self) -> usize {
        self.history.undo_depth()
    }

    /// Applies one command. See [`ApplyOutcome`] for the contract; on
    /// `Rejected` the model is untouched.
    pub fn apply(&mut self, cmd: VideoCommand) -> ApplyOutcome {
        match cmd {
            VideoCommand::AddSource { path, duration, fps } => {
                self.add_source(path, duration, fps)
            }
            VideoCommand::RemoveSource { source } => self.remove_source(source),
            VideoCommand::AddClip {
                track,
                source,
                source_in,
                source_out,
                position,
            } => self.add_clip(track, source, source_in, source_out, position),
            VideoCommand::RemoveClip { clip } => self.remove_clip(clip),
            VideoCommand::RippleDelete { clip } => self.ripple_delete(clip),
            VideoCommand::MoveClip { clip, position } => self.move_clip(clip, position),
            VideoCommand::TrimIn { clip, source_in } => self.trim_in(clip, source_in),
            VideoCommand::TrimOut { clip, source_out } => self.trim_out(clip, source_out),
            VideoCommand::SplitAt { clip, time } => self.split_at(clip, time),
            VideoCommand::SetPlayhead { position } => self.set_playhead(position),
            VideoCommand::Select { clip } => self.select(clip),
            VideoCommand::Undo => self.undo(),
            VideoCommand::Redo => self.redo(),
        }
    }

    // ── Internals ────────────────────────────────────────────────────────────

    /// Records the pre-edit snapshot for one undoable edit.
    fn record(&mut self) {
        self.history.record(self.project.clone());
    }

    fn applied(&mut self) -> ApplyOutcome {
        self.revision += 1;
        ApplyOutcome::Applied
    }

    /// Clears the selection when it no longer points at an existing clip.
    fn prune_dangling_selection(&mut self) {
        if let Some(id) = self.selection {
            if self.project.find_clip(id).is_none() {
                self.selection = None;
            }
        }
    }

    fn add_source(&mut self, path: String, duration: u64, fps: Fps) -> ApplyOutcome {
        if fps.num == 0 || fps.den == 0 {
            return ApplyOutcome::Rejected(format!(
                "AddSource: fps {}/{} must have both terms > 0",
                fps.num, fps.den
            ));
        }
        self.record();
        let id = SourceId(self.project.alloc_id());
        self.project.sources.push(Source {
            id,
            path,
            duration,
            fps,
        });
        self.applied()
    }

    /// Removes the source and every clip referencing it on both tracks —
    /// one undo group behind a single pre-edit snapshot.
    fn remove_source(&mut self, source: SourceId) -> ApplyOutcome {
        let Some(index) = self.project.sources.iter().position(|s| s.id == source) else {
            return ApplyOutcome::Rejected(format!("RemoveSource: no source {}", source.0));
        };
        self.record();
        self.project.sources.remove(index);
        self.project.video.clips.retain(|c| c.source != source);
        self.project.audio.clips.retain(|c| c.source != source);
        self.prune_dangling_selection();
        self.applied()
    }

    /// Validates one source window: `source_in < source_out <= duration`.
    fn check_window(
        what: &str,
        source: &Source,
        source_in: u64,
        source_out: u64,
    ) -> Result<(), ApplyOutcome> {
        if source_in >= source_out {
            return Err(ApplyOutcome::Rejected(format!(
                "{what}: source_in {source_in} must be < source_out {source_out}"
            )));
        }
        if source_out > source.duration {
            return Err(ApplyOutcome::Rejected(format!(
                "{what}: source_out {source_out} exceeds source {} duration {}",
                source.id.0, source.duration
            )));
        }
        Ok(())
    }

    fn add_clip(
        &mut self,
        track: TrackKind,
        source: SourceId,
        source_in: u64,
        source_out: u64,
        position: u64,
    ) -> ApplyOutcome {
        let Some(src) = self.project.source(source) else {
            return ApplyOutcome::Rejected(format!("AddClip: no source {}", source.0));
        };
        if let Err(rejected) = Self::check_window("AddClip", src, source_in, source_out) {
            return rejected;
        }
        if let Err(e) =
            self.project
                .check_placement("AddClip", track, position, source_out - source_in, None)
        {
            return ApplyOutcome::Rejected(e);
        }
        self.record();
        let id = ClipId(self.project.alloc_id());
        self.project.track_mut(track).clips.push(Clip {
            id,
            source,
            source_in,
            source_out,
            position,
        });
        self.applied()
    }

    fn remove_clip(&mut self, clip: ClipId) -> ApplyOutcome {
        let Some((track, index)) = self.project.find_clip(clip) else {
            return ApplyOutcome::Rejected(format!("RemoveClip: no clip {}", clip.0));
        };
        self.record();
        self.project.track_mut(track).clips.remove(index);
        self.prune_dangling_selection();
        self.applied()
    }

    fn ripple_delete(&mut self, clip: ClipId) -> ApplyOutcome {
        let Some((track, index)) = self.project.find_clip(clip) else {
            return ApplyOutcome::Rejected(format!("RippleDelete: no clip {}", clip.0));
        };
        let removed = self.project.track(track).clips[index].clone();
        let length = removed.length();
        // Defensive pre-check: with no overlaps, every later clip starts at
        // or after the removed clip's end, so the shift can never
        // underflow. Reject loudly instead of wrapping if that ever breaks.
        for later in &self.project.track(track).clips {
            if later.id != removed.id
                && later.position > removed.position
                && later.position.checked_sub(length).is_none()
            {
                return ApplyOutcome::Rejected(format!(
                    "RippleDelete: shifting clip {} by {length} would underflow",
                    later.id.0
                ));
            }
        }
        self.record();
        let clips = &mut self.project.track_mut(track).clips;
        clips.remove(index);
        for later in clips.iter_mut() {
            if later.position > removed.position {
                later.position -= length;
            }
        }
        self.prune_dangling_selection();
        self.applied()
    }

    fn move_clip(&mut self, clip: ClipId, position: u64) -> ApplyOutcome {
        let Some((track, index)) = self.project.find_clip(clip) else {
            return ApplyOutcome::Rejected(format!("MoveClip: no clip {}", clip.0));
        };
        let current = self.project.track(track).clips[index].clone();
        if current.position == position {
            return ApplyOutcome::NoOp;
        }
        if let Err(e) = self.project.check_placement(
            "MoveClip",
            track,
            position,
            current.length(),
            Some(clip),
        ) {
            return ApplyOutcome::Rejected(e);
        }
        self.record();
        self.project.track_mut(track).clips[index].position = position;
        self.applied()
    }

    fn trim_in(&mut self, clip: ClipId, source_in: u64) -> ApplyOutcome {
        let Some((track, index)) = self.project.find_clip(clip) else {
            return ApplyOutcome::Rejected(format!("TrimIn: no clip {}", clip.0));
        };
        let current = self.project.track(track).clips[index].clone();
        if source_in == current.source_in {
            return ApplyOutcome::NoOp;
        }
        if source_in >= current.source_out {
            return ApplyOutcome::Rejected(format!(
                "TrimIn: source_in {source_in} must be < source_out {}",
                current.source_out
            ));
        }
        // Content stays timeline-aligned: the position shifts by the same
        // delta as the in point.
        let new_position = if source_in >= current.source_in {
            current.position.checked_add(source_in - current.source_in)
        } else {
            current.position.checked_sub(current.source_in - source_in)
        };
        let Some(new_position) = new_position else {
            return ApplyOutcome::Rejected(format!(
                "TrimIn: shifting position {} by the in-point delta under/overflows",
                current.position
            ));
        };
        let new_length = current.source_out - source_in;
        if let Err(e) = self.project.check_placement(
            "TrimIn",
            track,
            new_position,
            new_length,
            Some(clip),
        ) {
            return ApplyOutcome::Rejected(e);
        }
        self.record();
        let target = &mut self.project.track_mut(track).clips[index];
        target.source_in = source_in;
        target.position = new_position;
        self.applied()
    }

    fn trim_out(&mut self, clip: ClipId, source_out: u64) -> ApplyOutcome {
        let Some((track, index)) = self.project.find_clip(clip) else {
            return ApplyOutcome::Rejected(format!("TrimOut: no clip {}", clip.0));
        };
        let current = self.project.track(track).clips[index].clone();
        if source_out == current.source_out {
            return ApplyOutcome::NoOp;
        }
        let Some(src) = self.project.source(current.source) else {
            return ApplyOutcome::Rejected(format!(
                "TrimOut: clip {} references missing source {}",
                clip.0, current.source.0
            ));
        };
        if let Err(rejected) = Self::check_window("TrimOut", src, current.source_in, source_out) {
            return rejected;
        }
        if let Err(e) = self.project.check_placement(
            "TrimOut",
            track,
            current.position,
            source_out - current.source_in,
            Some(clip),
        ) {
            return ApplyOutcome::Rejected(e);
        }
        self.record();
        self.project.track_mut(track).clips[index].source_out = source_out;
        self.applied()
    }

    fn split_at(&mut self, clip: ClipId, time: u64) -> ApplyOutcome {
        let Some((track, index)) = self.project.find_clip(clip) else {
            return ApplyOutcome::Rejected(format!("SplitAt: no clip {}", clip.0));
        };
        let current = self.project.track(track).clips[index].clone();
        let end = current.timeline_end();
        if time <= current.position || time >= end {
            return ApplyOutcome::Rejected(format!(
                "SplitAt: time {time} is not strictly inside [{}, {end})",
                current.position
            ));
        }
        let offset = time - current.position;
        self.record();
        // Both halves get fresh ids: ids are never aliased across edits.
        let left_id = ClipId(self.project.alloc_id());
        let right_id = ClipId(self.project.alloc_id());
        let left = Clip {
            id: left_id,
            source: current.source,
            source_in: current.source_in,
            source_out: current.source_in + offset,
            position: current.position,
        };
        let right = Clip {
            id: right_id,
            source: current.source,
            source_in: current.source_in + offset,
            source_out: current.source_out,
            position: time,
        };
        let clips = &mut self.project.track_mut(track).clips;
        clips[index] = left;
        clips.insert(index + 1, right);
        if self.selection == Some(clip) {
            self.selection = Some(left_id);
        }
        self.applied()
    }

    /// Playhead and selection bump the revision but never touch history.
    fn set_playhead(&mut self, position: u64) -> ApplyOutcome {
        if self.playhead == position {
            return ApplyOutcome::NoOp;
        }
        self.playhead = position;
        self.applied()
    }

    fn select(&mut self, clip: Option<ClipId>) -> ApplyOutcome {
        if let Some(id) = clip {
            if self.project.find_clip(id).is_none() {
                return ApplyOutcome::Rejected(format!("Select: no clip {}", id.0));
            }
        }
        if self.selection == clip {
            return ApplyOutcome::NoOp;
        }
        self.selection = clip;
        self.applied()
    }

    fn undo(&mut self) -> ApplyOutcome {
        let Some(mut snapshot) = self.history.undo(&self.project) else {
            return ApplyOutcome::NoOp;
        };
        // The id allocator is monotonic across undo: ids are never reused.
        snapshot.next_id = self.project.next_id;
        self.project = snapshot;
        self.prune_dangling_selection();
        self.applied()
    }

    fn redo(&mut self) -> ApplyOutcome {
        let Some(mut snapshot) = self.history.redo(&self.project) else {
            return ApplyOutcome::NoOp;
        };
        snapshot.next_id = self.project.next_id;
        self.project = snapshot;
        self.prune_dangling_selection();
        self.applied()
    }
}
