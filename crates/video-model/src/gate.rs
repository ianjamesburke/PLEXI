//! Video model release gate (stint 0523): a table-driven command matrix, a
//! long deterministic stress sequence, and seeded randomized command fuzzing
//! over the pure [`VideoModel`] state machine — with per-command invariant
//! checks and a machine-readable qualification artifact.
//!
//! Compiled only under `cfg(test)`: the gate is test infrastructure, never
//! product code. Mirrors the editor gate (`src/editor/gate.rs` in the host
//! crate) and the sibling `plexi-daw-model` gate in structure, seeds,
//! minimizer, and artifact schema shape.
#![cfg(test)]

use std::path::Path;
use std::time::Instant;

use crate::commands::{ApplyOutcome, VideoCommand};
use crate::history::MAX_GROUPS;
use crate::model::{ClipId, CutEntry, Fps, Project, SourceId, TrackKind, VideoModel};

/// Environment variable that pins `gate_randomized_commands` to one seed.
pub const GATE_SEED_ENV: &str = "PLEXI_VIDEO_GATE_SEED";

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
    pub command: VideoCommand,
    pub expect: Expect,
}

/// One named gate case: a step sequence applied to a fresh [`VideoModel`]
/// and a final-state check. Ids in the steps are deterministic: the model's
/// allocator starts at 1, so the Nth allocating command yields id N.
pub struct GateCase {
    pub name: &'static str,
    pub steps: Vec<GateStep>,
    pub check: fn(&VideoModel) -> Result<(), String>,
}

fn a(command: VideoCommand) -> GateStep {
    GateStep {
        command,
        expect: Expect::Applied,
    }
}

fn r(command: VideoCommand) -> GateStep {
    GateStep {
        command,
        expect: Expect::Rejected,
    }
}

fn n(command: VideoCommand) -> GateStep {
    GateStep {
        command,
        expect: Expect::NoOp,
    }
}

const FPS_30: Fps = Fps { num: 30, den: 1 };

fn source(path: &str, duration: u64) -> VideoCommand {
    VideoCommand::AddSource {
        path: path.to_string(),
        duration,
        fps: FPS_30,
    }
}

fn vclip(src: u64, source_in: u64, source_out: u64, position: u64) -> VideoCommand {
    VideoCommand::AddClip {
        track: TrackKind::Video,
        source: SourceId(src),
        source_in,
        source_out,
        position,
    }
}

fn aclip(src: u64, source_in: u64, source_out: u64, position: u64) -> VideoCommand {
    VideoCommand::AddClip {
        track: TrackKind::Audio,
        source: SourceId(src),
        source_in,
        source_out,
        position,
    }
}

fn sel(id: u64) -> VideoCommand {
    VideoCommand::Select {
        clip: Some(ClipId(id)),
    }
}

/// Fetches a clip by id or fails the check with a named error.
fn clip_by_id(model: &VideoModel, id: u64) -> Result<&crate::model::Clip, String> {
    let (track, index) = model
        .project()
        .find_clip(ClipId(id))
        .ok_or_else(|| format!("expected clip {id} to exist"))?;
    Ok(&model.project().track(track).clips[index])
}

/// The release-gate matrix: every case is small, named, and independently
/// replayable through a fresh [`VideoModel`].
#[must_use]
pub fn gate_cases() -> Vec<GateCase> {
    vec![
        GateCase {
            name: "add_source_and_clip",
            steps: vec![a(source("a.mp4", 10_000)), a(vclip(1, 0, 1000, 0))],
            check: |m| {
                let c = clip_by_id(m, 2)?;
                if c.length() != 1000 || c.position != 0 || m.undo_depth() != 2 {
                    return Err(format!("unexpected clip {c:?} / depth {}", m.undo_depth()));
                }
                Ok(())
            },
        },
        GateCase {
            name: "fps_invalid_rejected",
            steps: vec![
                r(VideoCommand::AddSource {
                    path: "z.mp4".to_string(),
                    duration: 1000,
                    fps: Fps { num: 0, den: 1 },
                }),
                r(VideoCommand::AddSource {
                    path: "z.mp4".to_string(),
                    duration: 1000,
                    fps: Fps { num: 30, den: 0 },
                }),
            ],
            check: |m| {
                if !m.project().sources.is_empty() || m.undo_depth() != 0 || m.revision() != 0 {
                    return Err("invalid fps must leave state untouched".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "add_overlap_rejected_adjacent_ok",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(vclip(1, 0, 1000, 0)),
                r(vclip(1, 0, 1000, 500)),
                // Touching end-to-start is not an overlap.
                a(vclip(1, 0, 1000, 1000)),
            ],
            check: |m| {
                if m.project().video.clips.len() != 2 || m.undo_depth() != 3 {
                    return Err("overlap must reject; adjacency must apply".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "tracks_do_not_cross_check_overlap",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(vclip(1, 0, 1000, 0)),
                a(aclip(1, 0, 1000, 0)),
            ],
            check: |m| {
                let p = m.project();
                if p.video.clips.len() != 1 || p.audio.clips.len() != 1 {
                    return Err("same span on different tracks must coexist".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "add_clip_window_rejected",
            steps: vec![
                a(source("a.mp4", 10_000)),
                r(vclip(1, 1000, 1000, 0)),
                r(vclip(1, 2000, 1000, 0)),
                r(vclip(1, 0, 20_000, 0)),
                r(vclip(9, 0, 1000, 0)),
            ],
            check: |m| {
                if m.project().clip_count() != 0 || m.undo_depth() != 1 {
                    return Err("bad windows must leave state untouched".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "add_clip_position_overflow_rejected",
            steps: vec![a(source("a.mp4", 10_000)), r(vclip(1, 0, 1000, u64::MAX))],
            check: |m| {
                if m.project().clip_count() != 0 {
                    return Err("overflowing clip must be rejected".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "move_clip_and_overlap",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(vclip(1, 0, 1000, 0)),
                a(vclip(1, 0, 1000, 2000)),
                a(VideoCommand::MoveClip {
                    clip: ClipId(2),
                    position: 5000,
                }),
                n(VideoCommand::MoveClip {
                    clip: ClipId(2),
                    position: 5000,
                }),
                r(VideoCommand::MoveClip {
                    clip: ClipId(3),
                    position: 4500,
                }),
            ],
            check: |m| {
                if clip_by_id(m, 2)?.position != 5000 || clip_by_id(m, 3)?.position != 2000 {
                    return Err("move must land; overlapping move must reject".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "trim_in_shifts_position",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(vclip(1, 200, 1000, 5000)),
                // In point moves right by 200 → position shifts right by 200.
                a(VideoCommand::TrimIn {
                    clip: ClipId(2),
                    source_in: 400,
                }),
                // In point moves left by 300 → position shifts left by 300.
                a(VideoCommand::TrimIn {
                    clip: ClipId(2),
                    source_in: 100,
                }),
                n(VideoCommand::TrimIn {
                    clip: ClipId(2),
                    source_in: 100,
                }),
            ],
            check: |m| {
                let c = clip_by_id(m, 2)?;
                if c.source_in != 100 || c.source_out != 1000 || c.position != 4900 {
                    return Err(format!(
                        "TrimIn must keep content timeline-aligned; got {c:?}"
                    ));
                }
                Ok(())
            },
        },
        GateCase {
            name: "trim_in_underflow_rejected",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(vclip(1, 200, 1000, 0)),
                // Widening left would move the position before tick 0.
                r(VideoCommand::TrimIn {
                    clip: ClipId(2),
                    source_in: 100,
                }),
            ],
            check: |m| {
                let c = clip_by_id(m, 2)?;
                if c.source_in != 200 || c.position != 0 {
                    return Err("rejected trim must leave the clip untouched".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "trim_in_boundary_and_overlap_rejected",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(vclip(1, 0, 1000, 0)),
                a(vclip(1, 500, 1500, 1000)),
                // in == out is empty.
                r(VideoCommand::TrimIn {
                    clip: ClipId(3),
                    source_in: 1500,
                }),
                // Widening left by 500 would cover [500, 2000) over clip 2.
                r(VideoCommand::TrimIn {
                    clip: ClipId(3),
                    source_in: 0,
                }),
                r(VideoCommand::TrimIn {
                    clip: ClipId(99),
                    source_in: 0,
                }),
            ],
            check: |m| {
                let c = clip_by_id(m, 3)?;
                if c.source_in != 500 || c.position != 1000 {
                    return Err("boundary trims must reject in place".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "trim_out_moves_end",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(vclip(1, 0, 1000, 0)),
                a(vclip(1, 0, 1000, 2000)),
                a(VideoCommand::TrimOut {
                    clip: ClipId(2),
                    source_out: 1500,
                }),
                // Beyond the source duration.
                r(VideoCommand::TrimOut {
                    clip: ClipId(2),
                    source_out: 20_000,
                }),
                // out == in is empty.
                r(VideoCommand::TrimOut {
                    clip: ClipId(2),
                    source_out: 0,
                }),
                // Extending to 2500 ticks would cover clip 3 at 2000.
                r(VideoCommand::TrimOut {
                    clip: ClipId(2),
                    source_out: 2500,
                }),
            ],
            check: |m| {
                let c = clip_by_id(m, 2)?;
                if c.source_out != 1500 || c.position != 0 {
                    return Err("TrimOut must move only the end".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "split_fresh_ids_and_adjacency",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(vclip(1, 0, 1000, 3000)),
                a(VideoCommand::SplitAt {
                    clip: ClipId(2),
                    time: 3400,
                }),
            ],
            check: |m| {
                if m.project().find_clip(ClipId(2)).is_some() {
                    return Err("split must retire the original id".to_string());
                }
                let left = clip_by_id(m, 3)?;
                let right = clip_by_id(m, 4)?;
                if left.position != 3000 || left.source_in != 0 || left.source_out != 400 {
                    return Err(format!("bad left half {left:?}"));
                }
                if right.position != 3400 || right.source_in != 400 || right.source_out != 1000 {
                    return Err(format!("bad right half {right:?}"));
                }
                if left.timeline_end() != right.position {
                    return Err("halves must stay adjacent".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "split_at_edge_rejected",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(vclip(1, 0, 1000, 3000)),
                r(VideoCommand::SplitAt {
                    clip: ClipId(2),
                    time: 3000,
                }),
                r(VideoCommand::SplitAt {
                    clip: ClipId(2),
                    time: 4000,
                }),
                r(VideoCommand::SplitAt {
                    clip: ClipId(99),
                    time: 3500,
                }),
            ],
            check: |m| {
                if m.project().video.clips.len() != 1 || m.undo_depth() != 2 {
                    return Err("edge splits must reject in place".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "selection_follows_split",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(vclip(1, 0, 1000, 0)),
                a(sel(2)),
                a(VideoCommand::SplitAt {
                    clip: ClipId(2),
                    time: 500,
                }),
            ],
            check: |m| {
                if m.selection() != Some(ClipId(3)) {
                    return Err(format!(
                        "selection must move to the left half, got {:?}",
                        m.selection()
                    ));
                }
                Ok(())
            },
        },
        GateCase {
            name: "selection_cleared_on_remove",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(vclip(1, 0, 1000, 0)),
                a(sel(2)),
                a(VideoCommand::RemoveClip { clip: ClipId(2) }),
                n(VideoCommand::Select { clip: None }),
            ],
            check: |m| {
                if m.selection().is_some() {
                    return Err("removing the selected clip must clear selection".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "select_missing_rejected",
            steps: vec![r(sel(99))],
            check: |m| {
                if m.selection().is_some() || m.revision() != 0 {
                    return Err("selecting a missing clip must reject".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "ripple_delete_shifts_later_same_track",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(vclip(1, 0, 1000, 0)),
                a(vclip(1, 0, 1000, 1000)),
                a(vclip(1, 0, 1000, 3000)),
                a(aclip(1, 0, 1000, 1500)),
                a(VideoCommand::RippleDelete { clip: ClipId(3) }),
            ],
            check: |m| {
                if clip_by_id(m, 2)?.position != 0 {
                    return Err("earlier clip must not move".to_string());
                }
                if clip_by_id(m, 4)?.position != 2000 {
                    return Err("later same-track clip must shift left by 1000".to_string());
                }
                if clip_by_id(m, 5)?.position != 1500 {
                    return Err("other track must be untouched".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "ripple_delete_single_undo_group",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(vclip(1, 0, 1000, 0)),
                a(vclip(1, 0, 1000, 2000)),
                a(VideoCommand::RippleDelete { clip: ClipId(2) }),
                a(VideoCommand::Undo),
            ],
            check: |m| {
                // One undo restores both the removed clip and the shifts.
                if clip_by_id(m, 2)?.position != 0 || clip_by_id(m, 3)?.position != 2000 {
                    return Err("one undo must restore the whole ripple".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "remove_leaves_gap",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(vclip(1, 0, 1000, 0)),
                a(vclip(1, 0, 1000, 1000)),
                a(vclip(1, 0, 1000, 3000)),
                a(VideoCommand::RemoveClip { clip: ClipId(3) }),
            ],
            check: |m| {
                if clip_by_id(m, 2)?.position != 0 || clip_by_id(m, 4)?.position != 3000 {
                    return Err("plain remove must not shift neighbors".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "playhead_selection_not_undoable",
            steps: vec![
                a(VideoCommand::SetPlayhead { position: 500 }),
                n(VideoCommand::SetPlayhead { position: 500 }),
                n(VideoCommand::Undo),
                a(source("a.mp4", 10_000)),
                a(vclip(1, 0, 1000, 0)),
                a(sel(2)),
                a(VideoCommand::SetPlayhead { position: 700 }),
                // Undo pops the clip add; the playhead stays, and the now
                // dangling selection is cleared rather than restored.
                a(VideoCommand::Undo),
            ],
            check: |m| {
                if m.playhead() != 700 {
                    return Err("undo must not touch the playhead".to_string());
                }
                if m.selection().is_some() || m.project().clip_count() != 0 {
                    return Err("undoing the add must clear the dangling selection".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "remove_source_cascades_and_clears_selection",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(vclip(1, 0, 1000, 0)),
                a(aclip(1, 0, 1000, 0)),
                a(sel(2)),
                a(VideoCommand::RemoveSource { source: SourceId(1) }),
                r(VideoCommand::RemoveSource { source: SourceId(1) }),
            ],
            check: |m| {
                let p = m.project();
                if !p.sources.is_empty() || p.clip_count() != 0 || m.selection().is_some() {
                    return Err("cascade must remove both clips and the selection".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "remove_source_cascade_single_undo_group",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(vclip(1, 0, 1000, 0)),
                a(aclip(1, 0, 1000, 0)),
                a(VideoCommand::RemoveSource { source: SourceId(1) }),
                a(VideoCommand::Undo),
            ],
            check: |m| {
                let p = m.project();
                if p.sources.len() != 1 || p.clip_count() != 2 {
                    return Err("one undo must restore the source and both clips".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "cut_list_derivation",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(source("b.mp4", 5_000)),
                a(vclip(1, 0, 2000, 0)),
                a(vclip(2, 1000, 3000, 2000)),
                a(vclip(1, 5000, 6000, 5000)),
                // Trim the first clip's head: gap opens at [0, 500).
                a(VideoCommand::TrimIn {
                    clip: ClipId(3),
                    source_in: 500,
                }),
                // Split the b.mp4 clip at its midpoint → ids 6 (left) + 7.
                a(VideoCommand::SplitAt {
                    clip: ClipId(4),
                    time: 3000,
                }),
                // Ripple out the left half: id 7 and id 5 shift left 1000.
                a(VideoCommand::RippleDelete { clip: ClipId(6) }),
            ],
            check: |m| {
                let expected = vec![
                    CutEntry {
                        source_path: "a.mp4".to_string(),
                        source_in: 500,
                        source_out: 2000,
                        position: 500,
                    },
                    CutEntry {
                        source_path: "b.mp4".to_string(),
                        source_in: 2000,
                        source_out: 3000,
                        position: 2000,
                    },
                    CutEntry {
                        source_path: "a.mp4".to_string(),
                        source_in: 5000,
                        source_out: 6000,
                        position: 4000,
                    },
                ];
                let got = m.project().cut_list(TrackKind::Video);
                if got != expected {
                    return Err(format!("cut list mismatch:\nexpected {expected:#?}\ngot {got:#?}"));
                }
                if !m.project().cut_list(TrackKind::Audio).is_empty() {
                    return Err("audio cut list must be empty".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "undo_redo_round_trip",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(vclip(1, 0, 1000, 0)),
                a(VideoCommand::Undo),
                a(VideoCommand::Redo),
            ],
            check: |m| {
                if m.project().clip_count() != 1 || m.undo_depth() != 2 || m.can_redo() {
                    return Err("undo+redo must restore the clip".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "redo_invalidated_by_new_edit",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(VideoCommand::Undo),
                a(source("b.mp4", 5_000)),
            ],
            check: |m| {
                if m.can_redo() || m.project().sources.len() != 1 {
                    return Err("new edit after undo must clear redo".to_string());
                }
                if m.project().sources[0].path != "b.mp4" {
                    return Err("surviving source must be the new one".to_string());
                }
                Ok(())
            },
        },
        GateCase {
            name: "undo_to_empty",
            steps: vec![
                a(source("a.mp4", 10_000)),
                a(vclip(1, 0, 1000, 0)),
                a(VideoCommand::Undo),
                a(VideoCommand::Undo),
                n(VideoCommand::Undo),
            ],
            check: |m| {
                let p = m.project();
                if !p.sources.is_empty() || p.clip_count() != 0 || m.undo_depth() != 0 {
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
pub fn check_invariants(model: &VideoModel, prev_revision: u64) -> Result<(), String> {
    let project = model.project();
    for source in &project.sources {
        if source.fps.num == 0 || source.fps.den == 0 {
            return Err(format!(
                "source {} fps {}/{} has a zero term",
                source.id.0, source.fps.num, source.fps.den
            ));
        }
    }
    let mut ids: Vec<u64> = project.sources.iter().map(|s| s.id.0).collect();
    for kind in [TrackKind::Video, TrackKind::Audio] {
        let track = project.track(kind);
        for clip in &track.clips {
            ids.push(clip.id.0);
            let Some(source) = project.source(clip.source) else {
                return Err(format!(
                    "clip {} references missing source {}",
                    clip.id.0, clip.source.0
                ));
            };
            if clip.source_in >= clip.source_out {
                return Err(format!(
                    "clip {} window {}..{} is empty or inverted",
                    clip.id.0, clip.source_in, clip.source_out
                ));
            }
            if clip.source_out > source.duration {
                return Err(format!(
                    "clip {} window end {} exceeds source duration {}",
                    clip.id.0, clip.source_out, source.duration
                ));
            }
            if clip
                .position
                .checked_add(clip.source_out - clip.source_in)
                .is_none()
            {
                return Err(format!("clip {} timeline end overflows u64", clip.id.0));
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
                return Err(format!(
                    "{kind:?} clips {} and {} overlap: [{}, {}) vs [{}, {})",
                    pair[0].2, pair[1].2, pair[0].0, pair[0].1, pair[1].0, pair[1].1
                ));
            }
        }
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
    if let Some(selected) = model.selection() {
        if project.find_clip(selected).is_none() {
            return Err(format!("selection points at missing clip {}", selected.0));
        }
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
/// restore the exact serialized project. Playhead and selection are
/// deliberately outside the compare — they are not part of history.
pub fn check_history_round_trip(model: &mut VideoModel) -> Result<(), String> {
    if !model.can_undo() {
        return Ok(());
    }
    let before = model.project().to_json()?;
    model.apply(VideoCommand::Undo);
    model.apply(VideoCommand::Redo);
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
    let mut model = VideoModel::new();
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
    let mut model = VideoModel::new();
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
                    model.apply(VideoCommand::Undo);
                    check_invariants(&model, prev_revision)
                        .map_err(|e| format!("interleaved undo @{applied}: {e}"))?;
                    prev_revision = model.revision();
                }
                if applied % 13 == 0 {
                    model.apply(VideoCommand::Redo);
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
        model.apply(VideoCommand::Redo);
    }
    let final_json = model.project().to_json()?;
    let mut guard = 0usize;
    while model.can_undo() {
        model.apply(VideoCommand::Undo);
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
        model.apply(VideoCommand::Redo);
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

fn source_ids(project: &Project) -> Vec<u64> {
    project.sources.iter().map(|s| s.id.0).collect()
}

fn clip_ids(project: &Project) -> Vec<u64> {
    project
        .video
        .clips
        .iter()
        .chain(project.audio.clips.iter())
        .map(|c| c.id.0)
        .collect()
}

fn random_track(rng: &mut SplitMix64) -> TrackKind {
    if rng.below(2) == 0 {
        TrackKind::Video
    } else {
        TrackKind::Audio
    }
}

/// Fps values are valid most of the time; zero terms keep rejection hot.
fn random_fps(rng: &mut SplitMix64) -> Fps {
    match rng.below(12) {
        0 => Fps { num: 0, den: 1 },
        1 => Fps { num: 30, den: 0 },
        2 => Fps {
            num: 30_000,
            den: 1001,
        },
        _ => Fps {
            num: 24 + rng.below(40) as u32,
            den: 1,
        },
    }
}

/// Positions mix ordinary grid values with near-overflow extremes.
fn random_position(rng: &mut SplitMix64) -> u64 {
    if rng.below(16) == 0 {
        u64::MAX - rng.below(2000)
    } else {
        rng.below(64) * 500
    }
}

/// Windows include empty, inverted, and beyond-duration shapes.
fn random_window(rng: &mut SplitMix64) -> (u64, u64) {
    let source_in = rng.below(8) * 500;
    let source_out = match rng.below(8) {
        0 => source_in,
        1 => source_in.saturating_sub(500),
        2 => 1_000_000,
        _ => source_in + (1 + rng.below(6)) * 500,
    };
    (source_in, source_out)
}

/// Weighted random command generator over the full [`VideoCommand`] surface,
/// including deliberately invalid ids, geometry, and fps values.
pub fn random_command(rng: &mut SplitMix64, model: &VideoModel) -> VideoCommand {
    let project = model.project();
    let sources = source_ids(project);
    let clips = clip_ids(project);
    // Memory discipline: past the soft cap, bias hard toward shrinking
    // commands so fuzz state (and its history snapshots) stays bounded.
    if clips.len() > FUZZ_CLIP_SOFT_CAP {
        return match rng.below(10) {
            0..=3 => VideoCommand::RemoveClip {
                clip: ClipId(pick_id(rng, &clips)),
            },
            4..=5 => VideoCommand::RippleDelete {
                clip: ClipId(pick_id(rng, &clips)),
            },
            6..=7 => VideoCommand::Undo,
            8 => VideoCommand::RemoveSource {
                source: SourceId(pick_id(rng, &sources)),
            },
            _ => VideoCommand::RemoveClip {
                clip: ClipId(pick_id(rng, &clips)),
            },
        };
    }
    match rng.below(100) {
        0..=11 => VideoCommand::AddSource {
            path: format!("media/{}.mp4", rng.below(50)),
            duration: rng.below(20) * 1000,
            fps: random_fps(rng),
        },
        12..=14 => VideoCommand::RemoveSource {
            source: SourceId(pick_id(rng, &sources)),
        },
        15..=39 => {
            let (source_in, source_out) = random_window(rng);
            VideoCommand::AddClip {
                track: random_track(rng),
                source: SourceId(pick_id(rng, &sources)),
                source_in,
                source_out,
                position: random_position(rng),
            }
        }
        40..=45 => VideoCommand::RemoveClip {
            clip: ClipId(pick_id(rng, &clips)),
        },
        46..=50 => VideoCommand::RippleDelete {
            clip: ClipId(pick_id(rng, &clips)),
        },
        51..=57 => VideoCommand::MoveClip {
            clip: ClipId(pick_id(rng, &clips)),
            position: random_position(rng),
        },
        58..=64 => VideoCommand::TrimIn {
            clip: ClipId(pick_id(rng, &clips)),
            source_in: rng.below(10) * 500,
        },
        65..=71 => VideoCommand::TrimOut {
            clip: ClipId(pick_id(rng, &clips)),
            source_out: rng.below(12) * 500,
        },
        72..=77 => VideoCommand::SplitAt {
            clip: ClipId(pick_id(rng, &clips)),
            time: rng.below(40) * 250,
        },
        78..=80 => VideoCommand::SetPlayhead {
            position: random_position(rng),
        },
        81..=84 => VideoCommand::Select {
            clip: if rng.below(4) == 0 {
                None
            } else {
                Some(ClipId(pick_id(rng, &clips)))
            },
        },
        85..=92 => VideoCommand::Undo,
        _ => VideoCommand::Redo,
    }
}

/// Replays `commands` on a fresh model with per-command invariant checks
/// plus the final history round trip — the same predicates the seed run
/// applies, so any original failure mode reproduces during minimization.
/// Returns the failing command index and message, if any.
fn replay(commands: &[VideoCommand]) -> Result<(), (usize, String)> {
    let mut model = VideoModel::new();
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
fn minimize(mut commands: Vec<VideoCommand>) -> (Vec<VideoCommand>, String) {
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
    minimized_commands: Vec<VideoCommand>,
}

/// A rejected or no-op command must leave the model observationally
/// untouched. Sampled (not per-command) because it serializes the project.
fn frozen_state(model: &VideoModel) -> Result<String, String> {
    Ok(format!(
        "{}|{}|{:?}|{}|{}",
        model.project().to_json()?,
        model.playhead(),
        model.selection(),
        model.revision(),
        model.undo_depth()
    ))
}

/// Runs one randomized seed. On invariant failure, minimizes the failing
/// sequence and writes a replay bundle; returns the failure message.
fn run_random_seed(seed: u64, command_count: usize) -> Result<(), String> {
    let mut rng = SplitMix64::new(seed);
    let mut model = VideoModel::new();
    let mut prev_revision = model.revision();
    let mut applied: Vec<VideoCommand> = Vec::with_capacity(command_count);
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
    let bundle_path = std::env::temp_dir().join(format!("plexi-video-gate-failure-{seed}.json"));
    match serde_json::to_vec_pretty(&bundle)
        .map_err(|e| e.to_string())
        .and_then(|json| std::fs::write(&bundle_path, json).map_err(|e| e.to_string()))
    {
        Ok(()) => {}
        Err(e) => log::error!(
            "video_gate: failed to write replay bundle {}: {e}",
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
/// `video-gate-core.json` into `out_dir`, and returns the summary.
pub fn run_core_qualification(out_dir: &Path) -> GateSummary {
    let seeds = seeds_under_test();
    let cases = gate_cases();
    log::info!(
        "video_gate: core qualification started cases={} seeds={}",
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
        log::error!("video_gate: failed to create {}: {e}", out_dir.display());
    } else {
        let artifact = out_dir.join("video-gate-core.json");
        match serde_json::to_vec_pretty(&summary)
            .map_err(|e| e.to_string())
            .and_then(|json| std::fs::write(&artifact, json).map_err(|e| e.to_string()))
        {
            Ok(()) => log::info!("video_gate: wrote artifact {}", artifact.display()),
            Err(e) => log::error!(
                "video_gate: failed to write artifact {}: {e}",
                artifact.display()
            ),
        }
    }

    log::info!(
        "video_gate: core qualification finished passed={} failed={} duration_ms={}",
        summary.totals.passed,
        summary.totals.failed,
        summary.totals.duration_ms
    );
    println!(
        "video_gate: core qualification finished passed={} failed={} duration_ms={}",
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
                "video gate case {:?} failed ({} steps): {e}",
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
    let out_dir = std::env::temp_dir().join("plexi-video-gate");
    let summary = run_core_qualification(&out_dir);
    assert_eq!(
        summary.totals.failed,
        0,
        "core qualification reported failures; see {}",
        out_dir.join("video-gate-core.json").display()
    );
    assert!(out_dir.join("video-gate-core.json").is_file());
}

#[test]
fn gate_minimizer_shrinks_reproducible_failures() {
    // Sanity for the minimizer itself: a healthy sequence has no failure to
    // reproduce, so the greedy pass must return it unchanged and say so.
    let commands = vec![source("a.mp4", 1000), VideoCommand::Undo];
    let (kept, message) = minimize(commands.clone());
    assert_eq!(kept, commands, "healthy sequences must come back unchanged");
    assert!(message.contains("did not reproduce"));
}

#[test]
fn gate_serialization_round_trip() {
    // Nontrivial project built through commands only, then
    // serialize → deserialize → serialize must be byte-identical.
    let mut model = VideoModel::new();
    for command in [
        source("a.mp4", 10_000),
        VideoCommand::AddSource {
            path: "b.mp4".to_string(),
            duration: 5_000,
            fps: Fps {
                num: 30_000,
                den: 1001,
            },
        },
        vclip(1, 0, 2000, 0),
        vclip(2, 1000, 3000, 2000),
        aclip(1, 0, 4000, 0),
        VideoCommand::TrimIn {
            clip: ClipId(3),
            source_in: 500,
        },
        VideoCommand::SplitAt {
            clip: ClipId(4),
            time: 3000,
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
    // playhead/selection must carry the exact state, satisfy invariants,
    // clear a dangling selection, and accept further edits with fresh
    // history and monotonic id allocation.
    let mut model = VideoModel::new();
    for command in [
        source("a.mp4", 10_000),
        vclip(1, 0, 1000, 0),
        sel(2),
        VideoCommand::SetPlayhead { position: 700 },
    ] {
        assert_eq!(model.apply(command), ApplyOutcome::Applied);
    }
    let json = model.project().to_json().expect("serialize");
    let decoded = Project::from_json(&json).expect("deserialize");
    let mut restored = VideoModel::from_parts(decoded.clone(), model.playhead(), model.selection());
    assert_eq!(restored.project(), model.project());
    assert_eq!(restored.playhead(), 700);
    assert_eq!(restored.selection(), Some(ClipId(2)));
    assert_eq!(restored.undo_depth(), 0, "undo never spans sessions");
    check_invariants(&restored, 0).expect("restored model must satisfy invariants");
    assert_eq!(restored.apply(vclip(1, 0, 1000, 2000)), ApplyOutcome::Applied);
    assert!(
        restored.project().find_clip(ClipId(3)).is_some(),
        "id allocation resumes past restored ids"
    );
    // A persisted selection that no longer resolves is cleared on restore.
    let dangling = VideoModel::from_parts(decoded, 0, Some(ClipId(99)));
    assert_eq!(dangling.selection(), None);
}
