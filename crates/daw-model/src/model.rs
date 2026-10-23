//! DAW project state and the pure `apply` state machine.
//!
//! [`Project`] is the serialized project data (tracks, clips, sources,
//! tempo, id counter). [`DawModel`] wraps it with the transport, the
//! revision counter, and the snapshot history — none of which belong in the
//! serialized project or in undo snapshots.

use serde::{Deserialize, Serialize};

use crate::commands::{ApplyOutcome, DawCommand};
use crate::history::{CoalesceKey, SnapshotHistory};

/// Musical resolution: all DAW times are `u64` ticks at this rate.
/// Conversion to samples/frames is the downstream engine's concern.
pub const TICKS_PER_BEAT: u64 = 960;

pub const TEMPO_MIN: f64 = 20.0;
pub const TEMPO_MAX: f64 = 999.0;
/// Linear gain range for [`Mixer::volume`].
pub const VOLUME_MIN: f32 = 0.0;
pub const VOLUME_MAX: f32 = 2.0;
pub const PAN_MIN: f32 = -1.0;
pub const PAN_MAX: f32 = 1.0;

// ─── Validation ──────────────────────────────────────────────────────────────

/// One state-validity violation, typed per violation class.
///
/// This is the single validation path for persisted or foreign state:
/// [`DawModel::from_parts`] (bundle loading) and the release gate's
/// invariant checks both run [`Project::validate`] / [`Transport::validate`]
/// — the same rules everywhere, never a second hand-rolled checker.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    TempoOutOfRange { tempo_bpm: f64 },
    VolumeOutOfRange { track: TrackId, volume: f32 },
    PanOutOfRange { track: TrackId, pan: f32 },
    MissingClipSource { clip: ClipId, source: SourceId },
    ClipKindMismatch { clip: ClipId, track: TrackId },
    ZeroLengthClip { clip: ClipId },
    ClipWindowExceedsSource { clip: ClipId, source_offset: u64, length: u64, duration: u64 },
    ClipTimelineOverflow { clip: ClipId },
    DuplicateId { id: u64 },
    IdNotBelowNextId { id: u64, next_id: u64 },
    LoopInverted { start: u64, end: u64 },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TempoOutOfRange { tempo_bpm } => write!(
                f,
                "tempo {tempo_bpm} is not finite or outside {TEMPO_MIN}..={TEMPO_MAX}"
            ),
            Self::VolumeOutOfRange { track, volume } => write!(
                f,
                "track {} volume {volume} is not finite or outside {VOLUME_MIN}..={VOLUME_MAX}",
                track.0
            ),
            Self::PanOutOfRange { track, pan } => write!(
                f,
                "track {} pan {pan} is not finite or outside {PAN_MIN}..={PAN_MAX}",
                track.0
            ),
            Self::MissingClipSource { clip, source } => write!(
                f,
                "clip {} references missing source {}",
                clip.0, source.0
            ),
            Self::ClipKindMismatch { clip, track } => write!(
                f,
                "clip {} source kind does not match track {} kind",
                clip.0, track.0
            ),
            Self::ZeroLengthClip { clip } => write!(f, "clip {} has zero length", clip.0),
            Self::ClipWindowExceedsSource {
                clip,
                source_offset,
                length,
                duration,
            } => write!(
                f,
                "clip {} window {source_offset}+{length} exceeds source duration {duration}",
                clip.0
            ),
            Self::ClipTimelineOverflow { clip } => {
                write!(f, "clip {} timeline end overflows u64", clip.0)
            }
            Self::DuplicateId { id } => write!(f, "duplicate entity id {id}"),
            Self::IdNotBelowNextId { id, next_id } => write!(
                f,
                "id {id} >= next_id {next_id}; allocator lost monotonicity"
            ),
            Self::LoopInverted { start, end } => {
                write!(f, "loop enabled with start {start} >= end {end}")
            }
        }
    }
}

impl std::error::Error for ValidationError {}

// ─── Ids ─────────────────────────────────────────────────────────────────────

/// Newtype ids allocated by [`Project::next_id`]; never reused, even across
/// undo (the counter survives snapshot restores).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TrackId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ClipId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SourceId(pub u64);

// ─── Project data ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackKind {
    Audio,
    Midi,
}

/// Per-track mixer strip. Values clamp to the documented ranges on apply and
/// are re-checked by the gate invariants.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Mixer {
    /// Linear gain, [`VOLUME_MIN`]..=[`VOLUME_MAX`].
    pub volume: f32,
    /// [`PAN_MIN`] (hard left)..=[`PAN_MAX`] (hard right).
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
}

impl Default for Mixer {
    fn default() -> Self {
        Self {
            volume: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
        }
    }
}

/// A media source. `duration` is in ticks; a clip's window
/// (`source_offset + length`) must fit inside it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub id: SourceId,
    pub kind: TrackKind,
    pub path: String,
    pub duration: u64,
}

/// A clip placed on a track. DAW clips may overlap on a track; the clip's
/// source kind must match its track kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub source: SourceId,
    /// Timeline start, in ticks.
    pub position: u64,
    /// Timeline (and source-window) length, in ticks; always > 0.
    pub length: u64,
    /// Offset into the source where the clip's window starts.
    pub source_offset: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub name: String,
    pub mixer: Mixer,
    pub clips: Vec<Clip>,
}

/// Serialized project data — exactly what undo snapshots clone and what
/// [`Project::to_json`] emits. Transport, revision, and history live on
/// [`DawModel`], outside this struct.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// Clamped to [`TEMPO_MIN`]..=[`TEMPO_MAX`].
    pub tempo_bpm: f64,
    pub tracks: Vec<Track>,
    pub sources: Vec<Source>,
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
            tempo_bpm: 120.0,
            tracks: Vec::new(),
            sources: Vec::new(),
            next_id: 1,
        }
    }
}

impl Project {
    /// Pretty JSON of the project data. Errors name the failure.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| format!("daw project serialization failed: {e}"))
    }

    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json)
            .map_err(|e| format!("daw project deserialization failed: {e}"))
    }

    #[must_use]
    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    #[must_use]
    pub fn source(&self, id: SourceId) -> Option<&Source> {
        self.sources.iter().find(|s| s.id == id)
    }

    /// Locates a clip as `(track index, clip index)`.
    #[must_use]
    pub fn find_clip(&self, id: ClipId) -> Option<(usize, usize)> {
        self.tracks.iter().enumerate().find_map(|(ti, track)| {
            track
                .clips
                .iter()
                .position(|c| c.id == id)
                .map(|ci| (ti, ci))
        })
    }

    /// Total clips across all tracks.
    #[must_use]
    pub fn clip_count(&self) -> usize {
        self.tracks.iter().map(|t| t.clips.len()).sum()
    }

    /// Full state-validity check over the project data: clip geometry,
    /// source references and kind matches, mixer/tempo ranges and
    /// finiteness, id uniqueness and allocator monotonicity. The single
    /// shared path for bundle loading ([`DawModel::from_parts`]) and the
    /// release gate's invariants.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if !self.tempo_bpm.is_finite() || !(TEMPO_MIN..=TEMPO_MAX).contains(&self.tempo_bpm) {
            return Err(ValidationError::TempoOutOfRange {
                tempo_bpm: self.tempo_bpm,
            });
        }
        let mut ids: Vec<u64> = Vec::new();
        for track in &self.tracks {
            ids.push(track.id.0);
            let mixer = track.mixer;
            if !mixer.volume.is_finite() || !(VOLUME_MIN..=VOLUME_MAX).contains(&mixer.volume) {
                return Err(ValidationError::VolumeOutOfRange {
                    track: track.id,
                    volume: mixer.volume,
                });
            }
            if !mixer.pan.is_finite() || !(PAN_MIN..=PAN_MAX).contains(&mixer.pan) {
                return Err(ValidationError::PanOutOfRange {
                    track: track.id,
                    pan: mixer.pan,
                });
            }
            for clip in &track.clips {
                ids.push(clip.id.0);
                let Some(source) = self.source(clip.source) else {
                    return Err(ValidationError::MissingClipSource {
                        clip: clip.id,
                        source: clip.source,
                    });
                };
                if source.kind != track.kind {
                    return Err(ValidationError::ClipKindMismatch {
                        clip: clip.id,
                        track: track.id,
                    });
                }
                if clip.length == 0 {
                    return Err(ValidationError::ZeroLengthClip { clip: clip.id });
                }
                match clip.source_offset.checked_add(clip.length) {
                    Some(end) if end <= source.duration => {}
                    _ => {
                        return Err(ValidationError::ClipWindowExceedsSource {
                            clip: clip.id,
                            source_offset: clip.source_offset,
                            length: clip.length,
                            duration: source.duration,
                        });
                    }
                }
                if clip.position.checked_add(clip.length).is_none() {
                    return Err(ValidationError::ClipTimelineOverflow { clip: clip.id });
                }
            }
        }
        for source in &self.sources {
            ids.push(source.id.0);
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

    fn track_index(&self, id: TrackId) -> Option<usize> {
        self.tracks.iter().position(|t| t.id == id)
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

// ─── Transport ───────────────────────────────────────────────────────────────

/// Playback state. Not undoable and never part of undo snapshots, but
/// changes bump the model revision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transport {
    pub playing: bool,
    /// Playhead position in ticks.
    pub position: u64,
    pub loop_enabled: bool,
    /// `loop_start < loop_end` holds whenever `loop_enabled` is true.
    pub loop_start: u64,
    pub loop_end: u64,
}

impl Transport {
    /// State-validity check for transport values; part of the shared
    /// validation path (see [`Project::validate`]).
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.loop_enabled && self.loop_start >= self.loop_end {
            return Err(ValidationError::LoopInverted {
                start: self.loop_start,
                end: self.loop_end,
            });
        }
        Ok(())
    }
}

// ─── Model ───────────────────────────────────────────────────────────────────

/// The DAW edit model: project data plus transport, revision, and history.
/// Commands in ([`DawCommand`]), state out — no I/O, no rendering.
#[derive(Debug, Clone, Default)]
pub struct DawModel {
    project: Project,
    transport: Transport,
    /// Monotonic change counter, bumped on every `Applied` outcome
    /// (including undo/redo and transport changes). Starts at 0.
    revision: u64,
    history: SnapshotHistory,
}

impl DawModel {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Restores a model from decoded state — the load half of persistence
    /// ([`Project::from_json`] plus the caller-persisted [`Transport`]).
    /// Rejects invariant-violating state (corrupted or hand-edited
    /// bundles) through the shared [`Project::validate`] path. History
    /// starts empty and the revision at 0: undo never spans sessions, and
    /// the revision counts changes within one session only.
    pub fn from_parts(project: Project, transport: Transport) -> Result<Self, ValidationError> {
        let model = Self {
            project,
            transport,
            revision: 0,
            history: SnapshotHistory::default(),
        };
        model.validate()?;
        Ok(model)
    }

    /// Full state-validity check over project data and transport — the
    /// same rules [`from_parts`](Self::from_parts) enforces on load and the
    /// release gate re-checks after every command.
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.project.validate()?;
        self.transport.validate()
    }

    #[must_use]
    pub fn project(&self) -> &Project {
        &self.project
    }

    #[must_use]
    pub fn transport(&self) -> &Transport {
        &self.transport
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
    pub fn apply(&mut self, cmd: DawCommand) -> ApplyOutcome {
        match cmd {
            DawCommand::SetTempo { bpm } => self.set_tempo(bpm),
            DawCommand::AddTrack { kind, name } => self.add_track(kind, name),
            DawCommand::RemoveTrack { track } => self.remove_track(track),
            DawCommand::RenameTrack { track, name } => self.rename_track(track, name),
            DawCommand::AddSource { kind, path, duration } => self.add_source(kind, path, duration),
            DawCommand::RemoveSource { source } => self.remove_source(source),
            DawCommand::AddClip {
                track,
                source,
                position,
                length,
                source_offset,
            } => self.add_clip(track, source, position, length, source_offset),
            DawCommand::RemoveClip { clip } => self.remove_clip(clip),
            DawCommand::MoveClip {
                clip,
                track,
                position,
            } => self.move_clip(clip, track, position),
            DawCommand::ResizeClip {
                clip,
                length,
                source_offset,
            } => self.resize_clip(clip, length, source_offset),
            DawCommand::SetTrackVolume { track, volume } => self.set_track_volume(track, volume),
            DawCommand::SetTrackPan { track, pan } => self.set_track_pan(track, pan),
            DawCommand::SetTrackMute { track, mute } => self.set_track_mute(track, mute),
            DawCommand::SetTrackSolo { track, solo } => self.set_track_solo(track, solo),
            DawCommand::Play => self.set_playing(true),
            DawCommand::Stop => self.set_playing(false),
            DawCommand::Seek { position } => self.seek(position),
            DawCommand::SetLoop { enabled, start, end } => self.set_loop(enabled, start, end),
            DawCommand::Undo => self.undo(),
            DawCommand::Redo => self.redo(),
        }
    }

    // ── Internals ────────────────────────────────────────────────────────────

    /// Records the pre-edit snapshot for one undoable edit.
    fn record(&mut self, key: Option<CoalesceKey>) {
        self.history.record(self.project.clone(), key);
    }

    fn applied(&mut self) -> ApplyOutcome {
        self.revision += 1;
        ApplyOutcome::Applied
    }

    fn set_tempo(&mut self, bpm: f64) -> ApplyOutcome {
        if !bpm.is_finite() {
            return ApplyOutcome::Rejected(format!("SetTempo: bpm {bpm:?} is not finite"));
        }
        let clamped = bpm.clamp(TEMPO_MIN, TEMPO_MAX);
        if clamped == self.project.tempo_bpm {
            return ApplyOutcome::NoOp;
        }
        self.record(None);
        self.project.tempo_bpm = clamped;
        self.applied()
    }

    fn add_track(&mut self, kind: TrackKind, name: String) -> ApplyOutcome {
        self.record(None);
        let id = TrackId(self.project.alloc_id());
        self.project.tracks.push(Track {
            id,
            kind,
            name,
            mixer: Mixer::default(),
            clips: Vec::new(),
        });
        self.applied()
    }

    fn remove_track(&mut self, track: TrackId) -> ApplyOutcome {
        let Some(index) = self.project.track_index(track) else {
            return ApplyOutcome::Rejected(format!("RemoveTrack: no track {}", track.0));
        };
        self.record(None);
        self.project.tracks.remove(index);
        self.applied()
    }

    fn rename_track(&mut self, track: TrackId, name: String) -> ApplyOutcome {
        let Some(index) = self.project.track_index(track) else {
            return ApplyOutcome::Rejected(format!("RenameTrack: no track {}", track.0));
        };
        if self.project.tracks[index].name == name {
            return ApplyOutcome::NoOp;
        }
        self.record(None);
        self.project.tracks[index].name = name;
        self.applied()
    }

    fn add_source(&mut self, kind: TrackKind, path: String, duration: u64) -> ApplyOutcome {
        self.record(None);
        let id = SourceId(self.project.alloc_id());
        self.project.sources.push(Source {
            id,
            kind,
            path,
            duration,
        });
        self.applied()
    }

    /// Removes the source and every clip referencing it — one undo group,
    /// because the whole cascade sits behind a single pre-edit snapshot.
    fn remove_source(&mut self, source: SourceId) -> ApplyOutcome {
        let Some(index) = self.project.sources.iter().position(|s| s.id == source) else {
            return ApplyOutcome::Rejected(format!("RemoveSource: no source {}", source.0));
        };
        self.record(None);
        self.project.sources.remove(index);
        for track in &mut self.project.tracks {
            track.clips.retain(|c| c.source != source);
        }
        self.applied()
    }

    /// Validates one clip window against its source and the timeline:
    /// `length > 0`, `source_offset + length <= duration`, and
    /// `position + length` must not overflow.
    fn check_clip_geometry(
        what: &str,
        source: &Source,
        position: u64,
        length: u64,
        source_offset: u64,
    ) -> Result<(), ApplyOutcome> {
        if length == 0 {
            return Err(ApplyOutcome::Rejected(format!("{what}: length must be > 0")));
        }
        match source_offset.checked_add(length) {
            Some(end) if end <= source.duration => {}
            _ => {
                return Err(ApplyOutcome::Rejected(format!(
                    "{what}: source window {source_offset}+{length} exceeds source {} duration {}",
                    source.id.0, source.duration
                )));
            }
        }
        if position.checked_add(length).is_none() {
            return Err(ApplyOutcome::Rejected(format!(
                "{what}: position {position} + length {length} overflows u64"
            )));
        }
        Ok(())
    }

    fn add_clip(
        &mut self,
        track: TrackId,
        source: SourceId,
        position: u64,
        length: u64,
        source_offset: u64,
    ) -> ApplyOutcome {
        let Some(track_index) = self.project.track_index(track) else {
            return ApplyOutcome::Rejected(format!("AddClip: no track {}", track.0));
        };
        let Some(src) = self.project.source(source) else {
            return ApplyOutcome::Rejected(format!("AddClip: no source {}", source.0));
        };
        if src.kind != self.project.tracks[track_index].kind {
            return ApplyOutcome::Rejected(format!(
                "AddClip: source {} kind {:?} does not match track {} kind {:?}",
                source.0, src.kind, track.0, self.project.tracks[track_index].kind
            ));
        }
        if let Err(rejected) =
            Self::check_clip_geometry("AddClip", src, position, length, source_offset)
        {
            return rejected;
        }
        self.record(None);
        let id = ClipId(self.project.alloc_id());
        self.project.tracks[track_index].clips.push(Clip {
            id,
            source,
            position,
            length,
            source_offset,
        });
        self.applied()
    }

    fn remove_clip(&mut self, clip: ClipId) -> ApplyOutcome {
        let Some((track_index, clip_index)) = self.project.find_clip(clip) else {
            return ApplyOutcome::Rejected(format!("RemoveClip: no clip {}", clip.0));
        };
        self.record(None);
        self.project.tracks[track_index].clips.remove(clip_index);
        self.applied()
    }

    fn move_clip(&mut self, clip: ClipId, track: TrackId, position: u64) -> ApplyOutcome {
        let Some((from_track, clip_index)) = self.project.find_clip(clip) else {
            return ApplyOutcome::Rejected(format!("MoveClip: no clip {}", clip.0));
        };
        let Some(to_track) = self.project.track_index(track) else {
            return ApplyOutcome::Rejected(format!("MoveClip: no track {}", track.0));
        };
        let moving = self.project.tracks[from_track].clips[clip_index].clone();
        let Some(src) = self.project.source(moving.source) else {
            return ApplyOutcome::Rejected(format!(
                "MoveClip: clip {} references missing source {}",
                clip.0, moving.source.0
            ));
        };
        if src.kind != self.project.tracks[to_track].kind {
            return ApplyOutcome::Rejected(format!(
                "MoveClip: clip {} kind {:?} does not match track {} kind {:?}",
                clip.0, src.kind, track.0, self.project.tracks[to_track].kind
            ));
        }
        if position.checked_add(moving.length).is_none() {
            return ApplyOutcome::Rejected(format!(
                "MoveClip: position {position} + length {} overflows u64",
                moving.length
            ));
        }
        if from_track == to_track && moving.position == position {
            return ApplyOutcome::NoOp;
        }
        self.record(None);
        self.project.tracks[from_track].clips.remove(clip_index);
        let mut moved = moving;
        moved.position = position;
        self.project.tracks[to_track].clips.push(moved);
        self.applied()
    }

    fn resize_clip(&mut self, clip: ClipId, length: u64, source_offset: u64) -> ApplyOutcome {
        let Some((track_index, clip_index)) = self.project.find_clip(clip) else {
            return ApplyOutcome::Rejected(format!("ResizeClip: no clip {}", clip.0));
        };
        let current = self.project.tracks[track_index].clips[clip_index].clone();
        let Some(src) = self.project.source(current.source) else {
            return ApplyOutcome::Rejected(format!(
                "ResizeClip: clip {} references missing source {}",
                clip.0, current.source.0
            ));
        };
        if let Err(rejected) =
            Self::check_clip_geometry("ResizeClip", src, current.position, length, source_offset)
        {
            return rejected;
        }
        if current.length == length && current.source_offset == source_offset {
            return ApplyOutcome::NoOp;
        }
        self.record(None);
        let target = &mut self.project.tracks[track_index].clips[clip_index];
        target.length = length;
        target.source_offset = source_offset;
        self.applied()
    }

    fn set_track_volume(&mut self, track: TrackId, volume: f32) -> ApplyOutcome {
        if !volume.is_finite() {
            return ApplyOutcome::Rejected(format!(
                "SetTrackVolume: volume {volume:?} is not finite"
            ));
        }
        let Some(index) = self.project.track_index(track) else {
            return ApplyOutcome::Rejected(format!("SetTrackVolume: no track {}", track.0));
        };
        let clamped = volume.clamp(VOLUME_MIN, VOLUME_MAX);
        if clamped == self.project.tracks[index].mixer.volume {
            return ApplyOutcome::NoOp;
        }
        self.record(Some(CoalesceKey::Volume(track)));
        self.project.tracks[index].mixer.volume = clamped;
        self.applied()
    }

    fn set_track_pan(&mut self, track: TrackId, pan: f32) -> ApplyOutcome {
        if !pan.is_finite() {
            return ApplyOutcome::Rejected(format!("SetTrackPan: pan {pan:?} is not finite"));
        }
        let Some(index) = self.project.track_index(track) else {
            return ApplyOutcome::Rejected(format!("SetTrackPan: no track {}", track.0));
        };
        let clamped = pan.clamp(PAN_MIN, PAN_MAX);
        if clamped == self.project.tracks[index].mixer.pan {
            return ApplyOutcome::NoOp;
        }
        self.record(Some(CoalesceKey::Pan(track)));
        self.project.tracks[index].mixer.pan = clamped;
        self.applied()
    }

    fn set_track_mute(&mut self, track: TrackId, mute: bool) -> ApplyOutcome {
        let Some(index) = self.project.track_index(track) else {
            return ApplyOutcome::Rejected(format!("SetTrackMute: no track {}", track.0));
        };
        if self.project.tracks[index].mixer.mute == mute {
            return ApplyOutcome::NoOp;
        }
        self.record(None);
        self.project.tracks[index].mixer.mute = mute;
        self.applied()
    }

    fn set_track_solo(&mut self, track: TrackId, solo: bool) -> ApplyOutcome {
        let Some(index) = self.project.track_index(track) else {
            return ApplyOutcome::Rejected(format!("SetTrackSolo: no track {}", track.0));
        };
        if self.project.tracks[index].mixer.solo == solo {
            return ApplyOutcome::NoOp;
        }
        self.record(None);
        self.project.tracks[index].mixer.solo = solo;
        self.applied()
    }

    /// Transport changes bump the revision but never touch history —
    /// except that they break an open mixer coalescing group, mirroring
    /// how cursor movement breaks typing coalescing in the editor.
    fn set_playing(&mut self, playing: bool) -> ApplyOutcome {
        if self.transport.playing == playing {
            return ApplyOutcome::NoOp;
        }
        self.history.break_group();
        self.transport.playing = playing;
        self.applied()
    }

    fn seek(&mut self, position: u64) -> ApplyOutcome {
        if self.transport.position == position {
            return ApplyOutcome::NoOp;
        }
        self.history.break_group();
        self.transport.position = position;
        self.applied()
    }

    fn set_loop(&mut self, enabled: bool, start: u64, end: u64) -> ApplyOutcome {
        if enabled && start >= end {
            return ApplyOutcome::Rejected(format!(
                "SetLoop: start {start} must be < end {end} when enabled"
            ));
        }
        if self.transport.loop_enabled == enabled
            && self.transport.loop_start == start
            && self.transport.loop_end == end
        {
            return ApplyOutcome::NoOp;
        }
        self.history.break_group();
        self.transport.loop_enabled = enabled;
        self.transport.loop_start = start;
        self.transport.loop_end = end;
        self.applied()
    }

    fn undo(&mut self) -> ApplyOutcome {
        let Some(mut snapshot) = self.history.undo(&self.project) else {
            return ApplyOutcome::NoOp;
        };
        // The id allocator is monotonic across undo: ids are never reused.
        snapshot.next_id = self.project.next_id;
        self.project = snapshot;
        self.applied()
    }

    fn redo(&mut self) -> ApplyOutcome {
        let Some(mut snapshot) = self.history.redo(&self.project) else {
            return ApplyOutcome::NoOp;
        };
        snapshot.next_id = self.project.next_id;
        self.project = snapshot;
        self.applied()
    }
}
