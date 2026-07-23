//! DAW model release gate (stint 0514): a table-driven command matrix, a long
//! deterministic stress sequence, and seeded randomized command fuzzing over
//! the pure [`DawModel`] state machine — with per-command invariant checks
//! and a machine-readable qualification artifact.
//!
//! Compiled only under `cfg(test)`: the gate is test infrastructure, never
//! product code. Mirrors the editor gate (`src/editor/gate.rs` in the host
//! crate) in structure, seeds, minimizer, and artifact schema shape.
#![cfg(test)]

use std::path::Path;
use std::time::Instant;

use crate::commands::{ApplyOutcome, DawCommand};
use crate::history::MAX_GROUPS;
use crate::model::{
    ClipId, DawModel, Project, SourceId, TrackId, TrackKind, PAN_MAX, PAN_MIN, TEMPO_MAX,
    TEMPO_MIN, TICKS_PER_BEAT, VOLUME_MAX, VOLUME_MIN,
};

/// Environment variable that pins `gate_randomized_commands` to one seed.
pub const GATE_SEED_ENV: &str = "PLEXI_DAW_GATE_SEED";

const DEFAULT_SEEDS: [u64; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
/// Kept modest (vs the editor's 2000): snapshot history makes state clones
/// the dominant cost, and a sibling agent shares this machine's memory.
const RANDOM_COMMANDS_PER_SEED: usize = 1500;
/// Past this many clips the fuzzer biases hard toward shrinking commands.
const FUZZ_CLIP_SOFT_CAP: usize = 200;

// ─── Case table ──────────────────────────────────────────────────────────────

/// Expected outcome kind of one gate step (`Rejected` matches any reason).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    Applied,
    Rejected,
    NoOp,
}

fn outcome_kind(outcome: &ApplyOutcome) -> Expect {
    match outcome {
        ApplyOutcome::Applied => Expect::Applied,
        ApplyOutcome::Rejected(_) => Expect::Rejected,
        ApplyOutcome::NoOp => Expect::NoOp,
    }
}

/// One step of a gate case: the command and the outcome it must produce.
#[derive(Debug)]
pub struct GateStep {
    pub command: DawCommand,
    pub expect: Expect,
}

/// One named gate case: a step sequence applied to a fresh [`DawModel`] and
/// a final-state check. Ids in the steps are deterministic: the model's
/// allocator starts at 1, so the Nth allocating command yields id N.
pub struct GateCase {
    pub name: &'static str,
    pub steps: Vec<GateStep>,
    pub check: fn(&DawModel) -> Result<(), String>,
}

fn a(command: DawCommand) -> GateStep {
    GateStep {
        command,
        expect: Expect::Applied,
    }
}

fn r(command: DawCommand) -> GateStep {
    GateStep {
        command,
        expect: Expect::Rejected,
    }
}

fn n(command: DawCommand) -> GateStep {
    GateStep {
        command,
        expect: Expect::NoOp,
    }
}

fn audio_track(name: &str) -> DawCommand {
    DawCommand::AddTrack {
        kind: TrackKind::Audio,
        name: name.to_string(),
    }
}

fn midi_track(name: &str) -> DawCommand {
    DawCommand::AddTrack {
        kind: TrackKind::Midi,
        name: name.to_string(),
    }
}

fn audio_source(duration: u64) -> DawCommand {
    DawCommand::AddSource {
        kind: TrackKind::Audio,
        path: "samples/kick.wav".to_string(),
        duration,
    }
}

fn midi_source(duration: u64) -> DawCommand {
    DawCommand::AddSource {
        kind: TrackKind::Midi,
        path: "midi/lead.mid".to_string(),
        duration,
    }
}

fn clip(track: u64, source: u64, position: u64, length: u64, source_offset: u64) -> DawCommand {
    DawCommand::AddClip {
        track: TrackId(track),
        source: SourceId(source),
        position,
        length,
        source_offset,
    }
}

fn vol(track: u64, volume: f32) -> DawCommand {
    DawCommand::SetTrackVolume {
        track: TrackId(track),
        volume,
    }
}

fn pan(track: u64, value: f32) -> DawCommand {
    DawCommand::SetTrackPan {
        track: TrackId(track),
        pan: value,
    }
}

/// Steps shared by clip cases: one audio track (id 1) and one 4-beat audio
/// source (id 2); the next allocated id is 3.
fn setup_track_source() -> Vec<GateStep> {
    vec![a(audio_track("Audio 1")), a(audio_source(4 * TICKS_PER_BEAT))]
}

fn with_setup(extra: Vec<GateStep>) -> Vec<GateStep> {
    let mut steps = setup_track_source();
    steps.extend(extra);
    steps
}

fn first_track(model: &DawModel) -> Result<&crate::model::Track, String> {
    model
        .project()
        .tracks
        .first()
        .ok_or_else(|| "expected at least one track".to_string())
}

/// The release-gate matrix: every case is small, named, and independently
/// replayable through a fresh [`DawModel`].
#[must_use]
pub fn gate_cases() -> Vec<GateCase> {
    vec![
        GateCase {
            name: "add_track",
            steps: vec![a(audio_track("Drums"))],
            check: |m| {
                let t = first_track(m)?;
                if t.name != "Drums" || t.kind != TrackKind::Audio || t.id != TrackId(1) {
                    return Err(format!("unexpected track {t:?}"));
                }
                if m.undo_depth() != 1 || m.revision() != 1 {
                    return Err(format!(
                        "expected depth 1 / revision 1, got {} / {}",
                        m.undo_depth(),
                        m.revision()
                    ));
                }
                Ok(())
            },
        },
        GateCase {
            name: "remove_track",
            steps: vec![
                a(audio_track("Drums")),
                a(DawCommand::RemoveTrack { track: TrackId(1) }),
            ],
            check: |m| {
                if !m.project().tracks.is_empty() || m.undo_depth() != 2 {
                    return Err(format!(
                        "expected no tracks / depth 2, got {} tracks / depth {}",
                        m.project().tracks.len(),
                        m.undo_depth()
                    ));
                }
                Ok(())
            },
        },
        GateCase {
            name: "remove_missing_track_rejected",
            steps: vec![r(DawCommand::RemoveTrack { track: TrackId(42) })],
            check: |m| {
                if m.undo_depth() != 0 || m.revision() != 0 {
                    return Err("rejection must leave history and revision untouched".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "rename_track_and_noop",
            steps: vec![
                a(audio_track("A")),
                a(DawCommand::RenameTrack {
                    track: TrackId(1),
                    name: "Lead".to_string(),
                }),
                n(DawCommand::RenameTrack {
                    track: TrackId(1),
                    name: "Lead".to_string(),
                }),
            ],
            check: |m| {
                if first_track(m)?.name != "Lead" || m.undo_depth() != 2 {
                    return Err("expected renamed track and depth 2".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "add_source_and_clip",
            steps: with_setup(vec![a(clip(1, 2, 0, TICKS_PER_BEAT, 0))]),
            check: |m| {
                let t = first_track(m)?;
                let c = t
                    .clips
                    .first()
                    .ok_or_else(|| "expected one clip".to_string())?;
                if c.id != ClipId(3) || c.length != TICKS_PER_BEAT || m.undo_depth() != 3 {
                    return Err(format!("unexpected clip {c:?} / depth {}", m.undo_depth()));
                }
                Ok(())
            },
        },
        GateCase {
            name: "clip_offset_beyond_source_rejected",
            steps: with_setup(vec![r(clip(1, 2, 0, TICKS_PER_BEAT, 4 * TICKS_PER_BEAT))]),
            check: |m| {
                if m.project().clip_count() != 0 || m.undo_depth() != 2 {
                    return Err("rejected clip must not exist or record history".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "clip_zero_length_rejected",
            steps: with_setup(vec![r(clip(1, 2, 0, 0, 0))]),
            check: |m| {
                if m.project().clip_count() != 0 || m.undo_depth() != 2 {
                    return Err("zero-length clip must be rejected without history".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "clip_missing_ids_rejected",
            steps: with_setup(vec![
                r(clip(9, 2, 0, TICKS_PER_BEAT, 0)),
                r(clip(1, 9, 0, TICKS_PER_BEAT, 0)),
            ]),
            check: |m| {
                if m.project().clip_count() != 0 || m.undo_depth() != 2 || m.revision() != 2 {
                    return Err("missing-id clips must leave state untouched".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "clip_kind_mismatch_rejected",
            steps: vec![
                a(audio_track("A")),
                a(midi_source(4 * TICKS_PER_BEAT)),
                r(clip(1, 2, 0, TICKS_PER_BEAT, 0)),
            ],
            check: |m| {
                if m.project().clip_count() != 0 {
                    return Err("MIDI source must not land on an audio track".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "clip_position_overflow_rejected",
            steps: with_setup(vec![r(clip(1, 2, u64::MAX, TICKS_PER_BEAT, 0))]),
            check: |m| {
                if m.project().clip_count() != 0 {
                    return Err("overflowing clip must be rejected".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "move_clip_same_track",
            steps: with_setup(vec![
                a(clip(1, 2, 0, TICKS_PER_BEAT, 0)),
                a(DawCommand::MoveClip {
                    clip: ClipId(3),
                    track: TrackId(1),
                    position: 2 * TICKS_PER_BEAT,
                }),
                n(DawCommand::MoveClip {
                    clip: ClipId(3),
                    track: TrackId(1),
                    position: 2 * TICKS_PER_BEAT,
                }),
            ]),
            check: |m| {
                let c = &first_track(m)?.clips[0];
                if c.position != 2 * TICKS_PER_BEAT || m.undo_depth() != 4 {
                    return Err(format!("unexpected position {} after move", c.position));
                }
                Ok(())
            },
        },
        GateCase {
            name: "move_clip_cross_track",
            steps: vec![
                a(audio_track("A")),
                a(audio_track("B")),
                a(audio_source(4 * TICKS_PER_BEAT)),
                a(clip(1, 3, 0, TICKS_PER_BEAT, 0)),
                a(DawCommand::MoveClip {
                    clip: ClipId(4),
                    track: TrackId(2),
                    position: TICKS_PER_BEAT,
                }),
            ],
            check: |m| {
                let p = m.project();
                if !p.tracks[0].clips.is_empty() || p.tracks[1].clips.len() != 1 {
                    return Err("clip must have moved from track 1 to track 2".to_string());
                }
                if p.tracks[1].clips[0].position != TICKS_PER_BEAT {
                    return Err("moved clip has wrong position".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "move_clip_kind_mismatch_rejected",
            steps: vec![
                a(audio_track("A")),
                a(midi_track("M")),
                a(audio_source(4 * TICKS_PER_BEAT)),
                a(clip(1, 3, 0, TICKS_PER_BEAT, 0)),
                r(DawCommand::MoveClip {
                    clip: ClipId(4),
                    track: TrackId(2),
                    position: 0,
                }),
            ],
            check: |m| {
                let p = m.project();
                if p.tracks[0].clips.len() != 1 || !p.tracks[1].clips.is_empty() {
                    return Err("cross-kind move must be rejected in place".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "resize_clip",
            steps: with_setup(vec![
                a(clip(1, 2, 0, TICKS_PER_BEAT, 0)),
                a(DawCommand::ResizeClip {
                    clip: ClipId(3),
                    length: 2 * TICKS_PER_BEAT,
                    source_offset: TICKS_PER_BEAT,
                }),
                // 960 + 3840 exceeds the 3840-tick source.
                r(DawCommand::ResizeClip {
                    clip: ClipId(3),
                    length: 4 * TICKS_PER_BEAT,
                    source_offset: TICKS_PER_BEAT,
                }),
                n(DawCommand::ResizeClip {
                    clip: ClipId(3),
                    length: 2 * TICKS_PER_BEAT,
                    source_offset: TICKS_PER_BEAT,
                }),
            ]),
            check: |m| {
                let c = &first_track(m)?.clips[0];
                if c.length != 2 * TICKS_PER_BEAT || c.source_offset != TICKS_PER_BEAT {
                    return Err(format!("unexpected clip window {c:?}"));
                }
                Ok(())
            },
        },
        GateCase {
            name: "volume_set_and_clamp",
            steps: vec![
                a(audio_track("A")),
                a(vol(1, 5.0)),
                r(vol(1, f32::NAN)),
                r(vol(9, 0.5)),
            ],
            check: |m| {
                let v = first_track(m)?.mixer.volume;
                if v != VOLUME_MAX || m.undo_depth() != 2 {
                    return Err(format!("expected clamped volume {VOLUME_MAX}, got {v}"));
                }
                Ok(())
            },
        },
        GateCase {
            name: "volume_coalesces_one_group",
            steps: vec![a(audio_track("A")), a(vol(1, 0.5)), a(vol(1, 0.7))],
            check: |m| {
                // Two consecutive volume sets on the same track = one group.
                if m.undo_depth() != 2 || first_track(m)?.mixer.volume != 0.7 {
                    return Err(format!(
                        "expected depth 2 / volume 0.7, got {} / {}",
                        m.undo_depth(),
                        first_track(m)?.mixer.volume
                    ));
                }
                Ok(())
            },
        },
        GateCase {
            name: "volume_coalesced_undo_restores_original",
            steps: vec![
                a(audio_track("A")),
                a(vol(1, 0.5)),
                a(vol(1, 0.7)),
                a(DawCommand::Undo),
            ],
            check: |m| {
                if first_track(m)?.mixer.volume != 1.0 || !m.can_redo() {
                    return Err("one undo must restore the pre-run volume".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "coalescing_broken_by_other_edit",
            steps: vec![
                a(audio_track("A")),
                a(vol(1, 0.5)),
                a(DawCommand::SetTrackMute {
                    track: TrackId(1),
                    mute: true,
                }),
                a(vol(1, 0.7)),
            ],
            check: |m| {
                if m.undo_depth() != 4 {
                    return Err(format!(
                        "mute must split the volume groups; depth {}",
                        m.undo_depth()
                    ));
                }
                Ok(())
            },
        },
        GateCase {
            name: "coalescing_broken_by_transport",
            steps: vec![
                a(audio_track("A")),
                a(vol(1, 0.5)),
                a(DawCommand::Seek {
                    position: TICKS_PER_BEAT,
                }),
                a(vol(1, 0.7)),
            ],
            check: |m| {
                // Track add + two volume groups; the seek records nothing.
                if m.undo_depth() != 3 {
                    return Err(format!(
                        "seek must break the volume group; depth {}",
                        m.undo_depth()
                    ));
                }
                Ok(())
            },
        },
        GateCase {
            name: "pan_clamp_and_coalesce",
            steps: vec![
                a(audio_track("A")),
                a(pan(1, -3.0)),
                a(pan(1, 0.25)),
                r(pan(1, f32::INFINITY)),
            ],
            check: |m| {
                let p = first_track(m)?.mixer.pan;
                if p != 0.25 || m.undo_depth() != 2 {
                    return Err(format!("expected coalesced pan 0.25, got {p}"));
                }
                Ok(())
            },
        },
        GateCase {
            name: "mute_solo_noop",
            steps: vec![
                a(audio_track("A")),
                a(DawCommand::SetTrackMute {
                    track: TrackId(1),
                    mute: true,
                }),
                n(DawCommand::SetTrackMute {
                    track: TrackId(1),
                    mute: true,
                }),
                a(DawCommand::SetTrackSolo {
                    track: TrackId(1),
                    solo: true,
                }),
                n(DawCommand::SetTrackSolo {
                    track: TrackId(1),
                    solo: true,
                }),
            ],
            check: |m| {
                let mixer = first_track(m)?.mixer;
                if !mixer.mute || !mixer.solo || m.undo_depth() != 3 {
                    return Err("mute/solo must stick; repeats must be NoOp".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "tempo_set_clamp_noop",
            steps: vec![
                a(DawCommand::SetTempo { bpm: 150.0 }),
                n(DawCommand::SetTempo { bpm: 150.0 }),
                a(DawCommand::SetTempo { bpm: 5.0 }),
                r(DawCommand::SetTempo { bpm: f64::NAN }),
            ],
            check: |m| {
                if m.project().tempo_bpm != TEMPO_MIN || m.undo_depth() != 2 {
                    return Err(format!(
                        "expected tempo clamped to {TEMPO_MIN}, got {}",
                        m.project().tempo_bpm
                    ));
                }
                Ok(())
            },
        },
        GateCase {
            name: "remove_source_cascades_one_group",
            steps: with_setup(vec![
                a(clip(1, 2, 0, TICKS_PER_BEAT, 0)),
                a(clip(1, 2, 2 * TICKS_PER_BEAT, TICKS_PER_BEAT, 0)),
                a(DawCommand::RemoveSource { source: SourceId(2) }),
                a(DawCommand::Undo),
            ]),
            check: |m| {
                // One undo restores the source and both cascaded clips.
                let p = m.project();
                if p.sources.len() != 1 || p.clip_count() != 2 {
                    return Err(format!(
                        "cascade undo must restore source + 2 clips, got {} sources / {} clips",
                        p.sources.len(),
                        p.clip_count()
                    ));
                }
                Ok(())
            },
        },
        GateCase {
            name: "transport_not_undoable_empty_history",
            steps: vec![
                a(DawCommand::Play),
                a(DawCommand::Seek {
                    position: TICKS_PER_BEAT,
                }),
                n(DawCommand::Undo),
            ],
            check: |m| {
                let t = m.transport();
                if !t.playing || t.position != TICKS_PER_BEAT {
                    return Err("undo with empty history must not touch transport".to_string());
                }
                if m.undo_depth() != 0 || m.revision() != 2 {
                    return Err("transport commands must not record history".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "transport_survives_undo",
            steps: vec![
                a(audio_track("A")),
                a(DawCommand::Play),
                a(DawCommand::Seek { position: 480 }),
                a(DawCommand::Undo),
            ],
            check: |m| {
                let t = m.transport();
                if !m.project().tracks.is_empty() {
                    return Err("undo must remove the track".to_string());
                }
                if !t.playing || t.position != 480 {
                    return Err("undo must not restore transport state".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "loop_validation",
            steps: vec![
                r(DawCommand::SetLoop {
                    enabled: true,
                    start: 100,
                    end: 100,
                }),
                r(DawCommand::SetLoop {
                    enabled: true,
                    start: 200,
                    end: 100,
                }),
                a(DawCommand::SetLoop {
                    enabled: true,
                    start: 0,
                    end: 4 * TICKS_PER_BEAT,
                }),
                n(DawCommand::SetLoop {
                    enabled: true,
                    start: 0,
                    end: 4 * TICKS_PER_BEAT,
                }),
                a(DawCommand::SetLoop {
                    enabled: false,
                    start: 0,
                    end: 4 * TICKS_PER_BEAT,
                }),
            ],
            check: |m| {
                if m.transport().loop_enabled {
                    return Err("loop must end disabled".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "stop_does_not_rewind",
            steps: vec![
                a(DawCommand::Play),
                a(DawCommand::Seek { position: 500 }),
                a(DawCommand::Stop),
                n(DawCommand::Stop),
            ],
            check: |m| {
                let t = m.transport();
                if t.playing || t.position != 500 {
                    return Err("stop must clear playing and keep the position".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "undo_redo_round_trip",
            steps: vec![
                a(audio_track("A")),
                a(DawCommand::RenameTrack {
                    track: TrackId(1),
                    name: "B".to_string(),
                }),
                a(DawCommand::Undo),
                a(DawCommand::Redo),
            ],
            check: |m| {
                if first_track(m)?.name != "B" || m.undo_depth() != 2 || m.can_redo() {
                    return Err("undo+redo must restore the rename".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "redo_invalidated_by_new_edit",
            steps: vec![
                a(audio_track("A")),
                a(DawCommand::Undo),
                a(midi_track("M")),
            ],
            check: |m| {
                if m.can_redo() || m.project().tracks.len() != 1 {
                    return Err("new edit after undo must clear redo".to_string());
                }
                if first_track(m)?.kind != TrackKind::Midi {
                    return Err("surviving track must be the new MIDI track".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "undo_to_empty",
            steps: vec![
                a(audio_track("A")),
                a(audio_source(TICKS_PER_BEAT)),
                a(DawCommand::Undo),
                a(DawCommand::Undo),
                n(DawCommand::Undo),
            ],
            check: |m| {
                let p = m.project();
                if !p.tracks.is_empty() || !p.sources.is_empty() || m.undo_depth() != 0 {
                    return Err("full undo must restore the empty project".to_string());
                }
                if !m.can_redo() {
                    return Err("redo must be available after full undo".to_string());
                }
                Ok(())
            },
        },
    ]
}

// ─── Invariants ──────────────────────────────────────────────────────────────

/// Read-only invariants that must hold after every applied command.
/// `prev_revision` is the revision observed before the command.
pub fn check_invariants(model: &DawModel, prev_revision: u64) -> Result<(), String> {
    let project = model.project();
    if !project.tempo_bpm.is_finite() || !(TEMPO_MIN..=TEMPO_MAX).contains(&project.tempo_bpm) {
        return Err(format!("tempo {} out of range", project.tempo_bpm));
    }
    let mut ids: Vec<u64> = Vec::new();
    for track in &project.tracks {
        ids.push(track.id.0);
        let mixer = track.mixer;
        if !mixer.volume.is_finite() || !(VOLUME_MIN..=VOLUME_MAX).contains(&mixer.volume) {
            return Err(format!("track {} volume {} out of range", track.id.0, mixer.volume));
        }
        if !mixer.pan.is_finite() || !(PAN_MIN..=PAN_MAX).contains(&mixer.pan) {
            return Err(format!("track {} pan {} out of range", track.id.0, mixer.pan));
        }
        for clip in &track.clips {
            ids.push(clip.id.0);
            let Some(source) = project.source(clip.source) else {
                return Err(format!(
                    "clip {} references missing source {}",
                    clip.id.0, clip.source.0
                ));
            };
            if source.kind != track.kind {
                return Err(format!(
                    "clip {} source kind {:?} mismatches track kind {:?}",
                    clip.id.0, source.kind, track.kind
                ));
            }
            if clip.length == 0 {
                return Err(format!("clip {} has zero length", clip.id.0));
            }
            match clip.source_offset.checked_add(clip.length) {
                Some(end) if end <= source.duration => {}
                _ => {
                    return Err(format!(
                        "clip {} window {}+{} exceeds source duration {}",
                        clip.id.0, clip.source_offset, clip.length, source.duration
                    ));
                }
            }
            if clip.position.checked_add(clip.length).is_none() {
                return Err(format!("clip {} timeline end overflows u64", clip.id.0));
            }
        }
    }
    for source in &project.sources {
        ids.push(source.id.0);
    }
    ids.sort_unstable();
    if ids.windows(2).any(|w| w[0] == w[1]) {
        return Err("duplicate entity id".to_string());
    }
    if ids.last().is_some_and(|&max| max >= project.next_id) {
        return Err(format!(
            "id {} >= next_id {}; allocator lost monotonicity",
            ids.last().unwrap(),
            project.next_id
        ));
    }
    let transport = model.transport();
    if transport.loop_enabled && transport.loop_start >= transport.loop_end {
        return Err(format!(
            "loop enabled with start {} >= end {}",
            transport.loop_start, transport.loop_end
        ));
    }
    if model.revision() < prev_revision {
        return Err(format!(
            "revision went backwards: {} -> {}",
            prev_revision,
            model.revision()
        ));
    }
    if model.can_undo() != (model.undo_depth() > 0) {
        return Err(format!(
            "undo_depth {} inconsistent with can_undo {}",
            model.undo_depth(),
            model.can_undo()
        ));
    }
    Ok(())
}

/// History round-trip invariant: when undo is available, Undo then Redo must
/// restore the exact serialized project. Mutates only history bookkeeping
/// (breaks the open coalescing group), so callers run it where coalescing no
/// longer matters. Transport is deliberately outside the compare — it is not
/// part of history.
pub fn check_history_round_trip(model: &mut DawModel) -> Result<(), String> {
    if !model.can_undo() {
        return Ok(());
    }
    let before = model.project().to_json()?;
    model.apply(DawCommand::Undo);
    model.apply(DawCommand::Redo);
    let after = model.project().to_json()?;
    if before != after {
        return Err(format!(
            "undo/redo round trip did not restore the project:\nbefore: {before}\nafter: {after}"
        ));
    }
    Ok(())
}

// ─── Case runner ─────────────────────────────────────────────────────────────

/// Final-state summary recorded in the qualification artifact.
#[derive(Debug, serde::Serialize)]
pub struct GateFinalState {
    pub revision: u64,
    pub undo_depth: usize,
    pub can_redo: bool,
}

fn run_case(case: &GateCase) -> Result<GateFinalState, String> {
    let mut model = DawModel::new();
    let mut prev_revision = model.revision();
    for (index, step) in case.steps.iter().enumerate() {
        let outcome = model.apply(step.command.clone());
        if outcome_kind(&outcome) != step.expect {
            return Err(format!(
                "step[{index}] {:?}: expected {:?}, got {outcome:?}",
                step.command, step.expect
            ));
        }
        check_invariants(&model, prev_revision)
            .map_err(|e| format!("step[{index}] {:?}: {e}", step.command))?;
        prev_revision = model.revision();
    }
    (case.check)(&model)?;
    check_history_round_trip(&mut model)?;
    Ok(GateFinalState {
        revision: model.revision(),
        undo_depth: model.undo_depth(),
        can_redo: model.can_redo(),
    })
}

// ─── Deterministic long sequence ─────────────────────────────────────────────

const LONG_SEQUENCE_REPS: usize = 10;
/// Guard band under [`MAX_GROUPS`]: the full-undo-to-empty assertion is only
/// valid while no undo group was ever evicted.
const MAX_OBSERVED_DEPTH: usize = MAX_GROUPS - 20;

fn run_long_sequence() -> Result<usize, String> {
    let mut model = DawModel::new();
    let mut prev_revision = model.revision();
    let mut applied = 0usize;
    let mut max_depth = 0usize;
    let cases = gate_cases();
    for rep in 0..LONG_SEQUENCE_REPS {
        for case in &cases {
            for step in &case.steps {
                // Outcomes differ in the shared model (ids drift); only the
                // invariants and history behavior are under test here.
                model.apply(step.command.clone());
                applied += 1;
                check_invariants(&model, prev_revision)
                    .map_err(|e| format!("rep {rep} case {} @{applied}: {e}", case.name))?;
                prev_revision = model.revision();
                max_depth = max_depth.max(model.undo_depth());
                // Interleaved undo/redo passes keep the history exercised
                // (and pruned: an edit after an undo drops the redone group).
                if applied % 5 == 0 {
                    model.apply(DawCommand::Undo);
                    check_invariants(&model, prev_revision)
                        .map_err(|e| format!("interleaved undo @{applied}: {e}"))?;
                    prev_revision = model.revision();
                }
                if applied % 13 == 0 {
                    model.apply(DawCommand::Redo);
                    check_invariants(&model, prev_revision)
                        .map_err(|e| format!("interleaved redo @{applied}: {e}"))?;
                    prev_revision = model.revision();
                }
                if applied % 97 == 0 {
                    check_history_round_trip(&mut model)
                        .map_err(|e| format!("round trip @{applied}: {e}"))?;
                }
            }
        }
    }
    if applied < 1000 {
        return Err(format!(
            "long sequence too short ({applied} commands) to qualify as a stress pass"
        ));
    }
    if max_depth >= MAX_OBSERVED_DEPTH {
        return Err(format!(
            "undo depth reached {max_depth}; retention cap invalidates full-undo assertion"
        ));
    }
    // Settle any leftover interleaved-undo residue first so "final" is
    // well-defined, then: full undo walks back to the default project (the
    // id allocator alone survives); full redo restores the exact final JSON.
    while model.can_redo() {
        model.apply(DawCommand::Redo);
    }
    let final_json = model.project().to_json()?;
    let mut guard = 0usize;
    while model.can_undo() {
        model.apply(DawCommand::Undo);
        guard += 1;
        if guard > 100_000 {
            return Err("full undo did not terminate".to_string());
        }
    }
    let empty = Project {
        next_id: model.project().next_id,
        ..Project::default()
    };
    if model.project() != &empty {
        return Err(format!(
            "full undo did not restore the default project; residue: {:?}",
            model.project()
        ));
    }
    while model.can_redo() {
        model.apply(DawCommand::Redo);
    }
    if model.project().to_json()? != final_json {
        return Err("full redo did not restore the final project".to_string());
    }
    Ok(applied)
}

// ─── Deterministic PRNG + randomized commands ────────────────────────────────

/// SplitMix64: tiny, deterministic, dependency-free PRNG.
pub struct SplitMix64(u64);

impl SplitMix64 {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform value in `0..n` (`n > 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

/// Existing id, or (1-in-8, and always when empty) a deliberately bogus one.
fn pick_id(rng: &mut SplitMix64, ids: &[u64]) -> u64 {
    if ids.is_empty() || rng.below(8) == 0 {
        1_000_000 + rng.below(1000)
    } else {
        ids[rng.below(ids.len() as u64) as usize]
    }
}

fn track_ids(project: &Project) -> Vec<u64> {
    project.tracks.iter().map(|t| t.id.0).collect()
}

fn source_ids(project: &Project) -> Vec<u64> {
    project.sources.iter().map(|s| s.id.0).collect()
}

fn clip_ids(project: &Project) -> Vec<u64> {
    project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(|c| c.id.0))
        .collect()
}

fn random_kind(rng: &mut SplitMix64) -> TrackKind {
    if rng.below(2) == 0 {
        TrackKind::Audio
    } else {
        TrackKind::Midi
    }
}

/// Positions mix ordinary grid values with near-overflow extremes.
fn random_position(rng: &mut SplitMix64) -> u64 {
    if rng.below(16) == 0 {
        u64::MAX - rng.below(2 * TICKS_PER_BEAT)
    } else {
        rng.below(64) * (TICKS_PER_BEAT / 2)
    }
}

/// Lengths include zero and absurd values so geometry rejection stays hot.
fn random_length(rng: &mut SplitMix64) -> u64 {
    match rng.below(8) {
        0 => 0,
        1 => 100 * TICKS_PER_BEAT,
        _ => (1 + rng.below(8)) * (TICKS_PER_BEAT / 2),
    }
}

/// Finite values spanning past both clamp edges, plus NaN/±inf injections.
fn random_f32(rng: &mut SplitMix64) -> f32 {
    match rng.below(16) {
        0 => f32::NAN,
        1 => f32::INFINITY,
        2 => f32::NEG_INFINITY,
        _ => (rng.below(600) as f32) / 100.0 - 2.0,
    }
}

fn random_tempo(rng: &mut SplitMix64) -> f64 {
    match rng.below(16) {
        0 => f64::NAN,
        1 => f64::INFINITY,
        _ => rng.below(1200) as f64,
    }
}

/// Weighted random command generator over the full [`DawCommand`] surface,
/// including deliberately invalid ids, geometry, and non-finite floats.
pub fn random_command(rng: &mut SplitMix64, model: &DawModel) -> DawCommand {
    let project = model.project();
    let tracks = track_ids(project);
    let sources = source_ids(project);
    let clips = clip_ids(project);
    // Memory discipline: past the soft cap, bias hard toward shrinking
    // commands so fuzz state (and its history snapshots) stays bounded.
    if clips.len() > FUZZ_CLIP_SOFT_CAP {
        return match rng.below(10) {
            0..=4 => DawCommand::RemoveClip {
                clip: ClipId(pick_id(rng, &clips)),
            },
            5..=6 => DawCommand::Undo,
            7 => DawCommand::RemoveTrack {
                track: TrackId(pick_id(rng, &tracks)),
            },
            8 => DawCommand::RemoveSource {
                source: SourceId(pick_id(rng, &sources)),
            },
            _ => DawCommand::RemoveClip {
                clip: ClipId(pick_id(rng, &clips)),
            },
        };
    }
    match rng.below(100) {
        0..=7 => DawCommand::AddTrack {
            kind: random_kind(rng),
            name: format!("Track {}", rng.below(1000)),
        },
        8..=10 => DawCommand::RemoveTrack {
            track: TrackId(pick_id(rng, &tracks)),
        },
        11..=13 => DawCommand::RenameTrack {
            track: TrackId(pick_id(rng, &tracks)),
            name: format!("Renamed {}", rng.below(100)),
        },
        14..=21 => DawCommand::AddSource {
            kind: random_kind(rng),
            path: format!("media/{}.wav", rng.below(50)),
            duration: rng.below(16) * TICKS_PER_BEAT,
        },
        22..=24 => DawCommand::RemoveSource {
            source: SourceId(pick_id(rng, &sources)),
        },
        25..=42 => DawCommand::AddClip {
            track: TrackId(pick_id(rng, &tracks)),
            source: SourceId(pick_id(rng, &sources)),
            position: random_position(rng),
            length: random_length(rng),
            source_offset: rng.below(8) * TICKS_PER_BEAT,
        },
        43..=47 => DawCommand::RemoveClip {
            clip: ClipId(pick_id(rng, &clips)),
        },
        48..=53 => DawCommand::MoveClip {
            clip: ClipId(pick_id(rng, &clips)),
            track: TrackId(pick_id(rng, &tracks)),
            position: random_position(rng),
        },
        54..=59 => DawCommand::ResizeClip {
            clip: ClipId(pick_id(rng, &clips)),
            length: random_length(rng),
            source_offset: rng.below(8) * TICKS_PER_BEAT,
        },
        60..=65 => DawCommand::SetTrackVolume {
            track: TrackId(pick_id(rng, &tracks)),
            volume: random_f32(rng),
        },
        66..=70 => DawCommand::SetTrackPan {
            track: TrackId(pick_id(rng, &tracks)),
            pan: random_f32(rng),
        },
        71..=73 => DawCommand::SetTrackMute {
            track: TrackId(pick_id(rng, &tracks)),
            mute: rng.below(2) == 0,
        },
        74..=75 => DawCommand::SetTrackSolo {
            track: TrackId(pick_id(rng, &tracks)),
            solo: rng.below(2) == 0,
        },
        76..=78 => DawCommand::SetTempo {
            bpm: random_tempo(rng),
        },
        79 => DawCommand::Play,
        80 => DawCommand::Stop,
        81..=82 => DawCommand::Seek {
            position: random_position(rng),
        },
        83..=85 => {
            // Sometimes inverted so loop validation stays exercised.
            let a = rng.below(8) * TICKS_PER_BEAT;
            let b = rng.below(8) * TICKS_PER_BEAT;
            DawCommand::SetLoop {
                enabled: rng.below(2) == 0,
                start: a,
                end: b,
            }
        }
        86..=92 => DawCommand::Undo,
        _ => DawCommand::Redo,
    }
}

/// Replays `commands` on a fresh model with per-command invariant checks
/// plus the final history round trip — the same predicates the seed run
/// applies, so any original failure mode reproduces during minimization.
/// Returns the failing command index and message, if any.
fn replay(commands: &[DawCommand]) -> Result<(), (usize, String)> {
    let mut model = DawModel::new();
    let mut prev_revision = model.revision();
    for (index, command) in commands.iter().enumerate() {
        model.apply(command.clone());
        if let Err(e) = check_invariants(&model, prev_revision) {
            return Err((index, e));
        }
        prev_revision = model.revision();
    }
    check_history_round_trip(&mut model)
        .map_err(|e| (commands.len().saturating_sub(1), e))
}

/// Greedy sequence minimization: repeatedly drop commands while the failure
/// still reproduces. Bounded passes keep worst-case cost predictable.
fn minimize(mut commands: Vec<DawCommand>) -> (Vec<DawCommand>, String) {
    let mut message = match replay(&commands) {
        Err((index, message)) => {
            commands.truncate(index + 1);
            message
        }
        Ok(()) => return (commands, "failure did not reproduce on replay".to_string()),
    };
    for _pass in 0..6 {
        let mut removed_any = false;
        let mut i = commands.len();
        while i > 0 {
            i -= 1;
            let mut candidate = commands.clone();
            candidate.remove(i);
            if let Err((_, m)) = replay(&candidate) {
                commands = candidate;
                message = m;
                removed_any = true;
            }
        }
        if !removed_any {
            break;
        }
    }
    (commands, message)
}

#[derive(serde::Serialize)]
struct ReplayBundle {
    seed: u64,
    command_count: usize,
    invariant_message: String,
    minimized_commands: Vec<DawCommand>,
}

/// A rejected or no-op command must leave the model observationally
/// untouched. Sampled (not per-command) because it serializes the project.
fn frozen_state(model: &DawModel) -> Result<String, String> {
    let transport = serde_json::to_string(model.transport())
        .map_err(|e| format!("transport serialization failed: {e}"))?;
    Ok(format!(
        "{}|{}|{}|{}",
        model.project().to_json()?,
        transport,
        model.revision(),
        model.undo_depth()
    ))
}

/// Runs one randomized seed. On invariant failure, minimizes the failing
/// sequence and writes a replay bundle; returns the failure message.
fn run_random_seed(seed: u64, command_count: usize) -> Result<(), String> {
    let mut rng = SplitMix64::new(seed);
    let mut model = DawModel::new();
    let mut prev_revision = model.revision();
    let mut applied: Vec<DawCommand> = Vec::with_capacity(command_count);
    let mut failure: Option<String> = None;
    for index in 0..command_count {
        let command = random_command(&mut rng, &model);
        // Sampled no-mutation check for Rejected/NoOp outcomes.
        let watch = index % 16 == 0;
        let before = if watch {
            match frozen_state(&model) {
                Ok(s) => Some(s),
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        } else {
            None
        };
        applied.push(command.clone());
        let outcome = model.apply(command);
        if let (Some(before), ApplyOutcome::Rejected(_) | ApplyOutcome::NoOp) = (before, &outcome)
        {
            match frozen_state(&model) {
                Ok(after) if after != before => {
                    failure = Some(format!("{outcome:?} outcome mutated state"));
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
        if let Err(e) = check_invariants(&model, prev_revision) {
            failure = Some(e);
            break;
        }
        prev_revision = model.revision();
        if (index + 1) % 250 == 0 {
            if let Err(e) = check_history_round_trip(&mut model) {
                failure = Some(format!("round trip @{index}: {e}"));
                break;
            }
        }
    }
    if failure.is_none() {
        if let Err(e) = check_history_round_trip(&mut model) {
            failure = Some(e);
        }
    }
    let Some(original_message) = failure else {
        return Ok(());
    };
    let (minimized, message) = minimize(applied);
    let bundle = ReplayBundle {
        seed,
        command_count: minimized.len(),
        invariant_message: message.clone(),
        minimized_commands: minimized,
    };
    let bundle_path = std::env::temp_dir().join(format!("plexi-daw-gate-failure-{seed}.json"));
    match serde_json::to_vec_pretty(&bundle)
        .map_err(|e| e.to_string())
        .and_then(|json| std::fs::write(&bundle_path, json).map_err(|e| e.to_string()))
    {
        Ok(()) => {}
        Err(e) => log::error!(
            "daw_gate: failed to write replay bundle {}: {e}",
            bundle_path.display()
        ),
    }
    Err(format!(
        "seed {seed} invariant failure: {original_message} (minimized: {message}). \
         Replay bundle: {}. Reproduce with {GATE_SEED_ENV}={seed}",
        bundle_path.display()
    ))
}

fn seeds_under_test() -> Vec<u64> {
    match std::env::var(GATE_SEED_ENV) {
        Ok(raw) => {
            let seed: u64 = raw
                .parse()
                .unwrap_or_else(|e| panic!("{GATE_SEED_ENV}={raw:?} is not a u64: {e}"));
            vec![seed]
        }
        Err(_) => DEFAULT_SEEDS.to_vec(),
    }
}

// ─── Qualification summary + artifact ────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct GateCaseResult {
    pub name: String,
    pub passed: bool,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_state: Option<GateFinalState>,
}

#[derive(serde::Serialize)]
pub struct GateRandomizedResult {
    pub seed: u64,
    pub commands: usize,
    pub passed: bool,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(serde::Serialize)]
pub struct GateLongSequenceResult {
    pub passed: bool,
    pub commands: usize,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(serde::Serialize)]
pub struct GateTotals {
    pub sections: usize,
    pub passed: usize,
    pub failed: usize,
    pub duration_ms: u64,
}

#[derive(serde::Serialize)]
pub struct GateSummary {
    pub schema_version: u32,
    pub cases: Vec<GateCaseResult>,
    pub long_sequence: GateLongSequenceResult,
    pub randomized: Vec<GateRandomizedResult>,
    pub totals: GateTotals,
}

/// Runs the whole core gate (matrix + long sequence + default seeds), writes
/// `daw-gate-core.json` into `out_dir`, and returns the summary.
pub fn run_core_qualification(out_dir: &Path) -> GateSummary {
    let seeds = seeds_under_test();
    let cases = gate_cases();
    log::info!(
        "daw_gate: core qualification started cases={} seeds={}",
        cases.len(),
        seeds.len()
    );
    let start = Instant::now();

    let case_results: Vec<GateCaseResult> = cases
        .iter()
        .map(|case| {
            let t = Instant::now();
            let result = run_case(case);
            GateCaseResult {
                name: case.name.to_string(),
                passed: result.is_ok(),
                duration_ms: t.elapsed().as_millis() as u64,
                error: result.as_ref().err().cloned(),
                final_state: result.ok(),
            }
        })
        .collect();

    let t = Instant::now();
    let long_result = run_long_sequence();
    let long_sequence = GateLongSequenceResult {
        passed: long_result.is_ok(),
        commands: *long_result.as_ref().unwrap_or(&0),
        duration_ms: t.elapsed().as_millis() as u64,
        error: long_result.err(),
    };

    let randomized: Vec<GateRandomizedResult> = seeds
        .iter()
        .map(|&seed| {
            let t = Instant::now();
            let result = run_random_seed(seed, RANDOM_COMMANDS_PER_SEED);
            GateRandomizedResult {
                seed,
                commands: RANDOM_COMMANDS_PER_SEED,
                passed: result.is_ok(),
                duration_ms: t.elapsed().as_millis() as u64,
                error: result.err(),
            }
        })
        .collect();

    let sections = case_results.len() + 1 + randomized.len();
    let failed = case_results.iter().filter(|c| !c.passed).count()
        + usize::from(!long_sequence.passed)
        + randomized.iter().filter(|r| !r.passed).count();
    let summary = GateSummary {
        schema_version: 1,
        cases: case_results,
        long_sequence,
        randomized,
        totals: GateTotals {
            sections,
            passed: sections - failed,
            failed,
            duration_ms: start.elapsed().as_millis() as u64,
        },
    };

    if let Err(e) = std::fs::create_dir_all(out_dir) {
        log::error!("daw_gate: failed to create {}: {e}", out_dir.display());
    } else {
        let artifact = out_dir.join("daw-gate-core.json");
        match serde_json::to_vec_pretty(&summary)
            .map_err(|e| e.to_string())
            .and_then(|json| std::fs::write(&artifact, json).map_err(|e| e.to_string()))
        {
            Ok(()) => log::info!("daw_gate: wrote artifact {}", artifact.display()),
            Err(e) => log::error!(
                "daw_gate: failed to write artifact {}: {e}",
                artifact.display()
            ),
        }
    }

    log::info!(
        "daw_gate: core qualification finished passed={} failed={} duration_ms={}",
        summary.totals.passed,
        summary.totals.failed,
        summary.totals.duration_ms
    );
    println!(
        "daw_gate: core qualification finished passed={} failed={} duration_ms={}",
        summary.totals.passed, summary.totals.failed, summary.totals.duration_ms
    );
    summary
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[test]
fn gate_core_matrix() {
    for case in gate_cases() {
        if let Err(e) = run_case(&case) {
            panic!(
                "daw gate case {:?} failed ({} steps): {e}",
                case.name,
                case.steps.len()
            );
        }
    }
}

#[test]
fn gate_deterministic_long_sequence() {
    let applied = run_long_sequence().unwrap_or_else(|e| panic!("long sequence failed: {e}"));
    assert!(applied >= 1000, "sequence must span thousands of commands");
}

#[test]
fn gate_randomized_commands() {
    // Seeds run sequentially, each on a fresh model: bounded memory.
    for seed in seeds_under_test() {
        if let Err(e) = run_random_seed(seed, RANDOM_COMMANDS_PER_SEED) {
            panic!("{e}");
        }
    }
}

#[test]
fn gate_core_qualification_artifact() {
    let out_dir = std::env::temp_dir().join("plexi-daw-gate");
    let summary = run_core_qualification(&out_dir);
    assert_eq!(
        summary.totals.failed,
        0,
        "core qualification reported failures; see {}",
        out_dir.join("daw-gate-core.json").display()
    );
    assert!(out_dir.join("daw-gate-core.json").is_file());
}

#[test]
fn gate_minimizer_shrinks_reproducible_failures() {
    // Sanity for the minimizer itself: a healthy sequence has no failure to
    // reproduce, so the greedy pass must return it unchanged and say so.
    let commands = vec![
        DawCommand::AddTrack {
            kind: TrackKind::Audio,
            name: "A".to_string(),
        },
        DawCommand::Undo,
    ];
    let (kept, message) = minimize(commands.clone());
    assert_eq!(kept, commands, "healthy sequences must come back unchanged");
    assert!(message.contains("did not reproduce"));
}

#[test]
fn gate_serialization_round_trip() {
    // Nontrivial project built through commands only, then
    // serialize → deserialize → serialize must be byte-identical.
    let mut model = DawModel::new();
    for command in [
        DawCommand::SetTempo { bpm: 133.7 },
        audio_track("Drums"),
        midi_track("Keys"),
        audio_source(8 * TICKS_PER_BEAT),
        midi_source(16 * TICKS_PER_BEAT),
        clip(1, 3, 0, 2 * TICKS_PER_BEAT, TICKS_PER_BEAT),
        DawCommand::AddClip {
            track: TrackId(2),
            source: SourceId(4),
            position: 4 * TICKS_PER_BEAT,
            length: 8 * TICKS_PER_BEAT,
            source_offset: 0,
        },
        vol(1, 0.35),
        pan(2, -0.5),
        DawCommand::SetTrackMute {
            track: TrackId(1),
            mute: true,
        },
    ] {
        assert_eq!(model.apply(command), ApplyOutcome::Applied);
    }
    let json1 = model.project().to_json().expect("serialize");
    let restored = Project::from_json(&json1).expect("deserialize");
    assert_eq!(&restored, model.project(), "semantic equality after round trip");
    let json2 = restored.to_json().expect("re-serialize");
    assert_eq!(json1, json2, "round trip must be byte-identical");
}

#[test]
fn gate_restore_from_parts() {
    // The load half of persistence: a model rebuilt from decoded project +
    // transport must carry the exact state, satisfy invariants, keep the id
    // allocator monotonic, and accept further edits with fresh history.
    let mut model = DawModel::new();
    for command in [
        audio_track("Drums"),
        audio_source(4 * TICKS_PER_BEAT),
        clip(1, 2, 0, TICKS_PER_BEAT, 0),
        DawCommand::Play,
        DawCommand::Seek { position: 480 },
    ] {
        assert_eq!(model.apply(command), ApplyOutcome::Applied);
    }
    let json = model.project().to_json().expect("serialize");
    let mut restored = DawModel::from_parts(
        Project::from_json(&json).expect("deserialize"),
        *model.transport(),
    );
    assert_eq!(restored.project(), model.project());
    assert_eq!(restored.transport(), model.transport());
    assert_eq!(restored.undo_depth(), 0, "undo never spans sessions");
    check_invariants(&restored, 0).expect("restored model must satisfy invariants");
    assert_eq!(restored.apply(midi_track("Keys")), ApplyOutcome::Applied);
    let new_track = restored.project().tracks.last().expect("new track");
    assert_eq!(new_track.id, TrackId(4), "id allocation resumes past restored ids");
}
