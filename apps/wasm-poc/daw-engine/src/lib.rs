// DAW Engine POC — real-time playback of the plexi-daw-model edit model
// (stint 0515), with the timeline / mixer / transport UI (stint 0516).
//
// Proves:
//   1. The shared plexi-daw-engine render path runs under the RT contract:
//      process-output calls Engine::render_block from the host audio thread
//      (prepared buffers only; the sole allocation is the ABI-required
//      return list).
//   2. The SAME path renders offline: the `daw_mixdown` tool renders
//      transport-start→project-end deterministically and reports the PCM
//      hash; with `out_path` it writes a float32 WAV via the file-write
//      effect — the 0518 export substrate.
//   3. The model mutates only through DawCommand: keys, pointer drags, the
//      view's buttons, and the `daw_command`/`daw.*` tools all route through
//      DawModel::apply; the engine re-prepares from the model after every
//      applied edit.
//   4. WAV/MIDI clip sources load outside the RT path via the SDK
//      file-read effect (binary I/O from 0509), gated by fs:read
//      capability requests; `demo:` sources are synthesized in-guest so the
//      POC is audible with zero grants.
//
// The UI (stint 0516) is keyboard-complete: every operation — transport,
// track/clip selection, clip move/trim/split, add track, and the whole mixer
// strip — has a key. Buttons expose the same operations as node-addressable
// controls (so `plexi pane click --node` and `expect`/`node_changes` reach
// them), and pointer drag on the timeline canvas moves and end-trims clips
// through the 0510 drag seam. No control is pointer-only.

wit_bindgen::generate!({
    world: "plexi-audio-app",
    path: "wit/world.wit",
});

use std::collections::{BTreeMap, VecDeque};

use exports::plexi::platform::audio_rt_process;
use exports::plexi::platform::lifecycle::Guest;
use plexi::platform::audio_rt_control::{self, AudioConfig, AudioFormat};
use plexi::platform::host_log;
use plexi::platform::types::{
    Alignment, BadgeColor, BadgeNode, ButtonNode, ButtonStyle, CanvasCommand, CanvasLine,
    CanvasNode, CanvasRect, CanvasText, Color, ColumnNode, DeclareToolsEffect, Effect,
    FileReadEffect, FileWriteEffect, FooterKeyEntry, FooterKeysNode, IndexedNode, InputEvent,
    KeyEvent, MouseButton, MouseEvent, PaddingNode, RowNode, StateSnapshot, TextNode,
    ToolCallEvent, ToolDecl, ToolResultEffect, UiActionEvent, UiNodeData, UiTree,
};

use plexi_daw_engine::{midi, pcm_hash, wav, Engine, EngineConfig, Note, SourceData};
use plexi_daw_model::{
    ApplyOutcome, Clip, ClipId, DawCommand, DawModel, SourceId, TrackId, TrackKind, TICKS_PER_BEAT,
};

/// Requested stream shape; the engine prepares at whatever the host
/// actually negotiates (queried via `stream-config` after open).
const REQUESTED_SAMPLE_RATE: u32 = 48_000;
const REQUESTED_CHANNELS: u32 = 2;
const BUFFER_FRAMES: u32 = 512;

const TOOL_MIXDOWN: &str = "daw_mixdown";
const TOOL_COMMAND: &str = "daw_command";

// ── Timeline layout (canvas-logical units) ───────────────────────────────────

/// One beat, in ticks — the keyboard nudge/seek quantum.
const BEAT: u64 = TICKS_PER_BEAT;
/// Snap resolution for pointer edits: a sixteenth note.
const GRID_TICKS: u64 = TICKS_PER_BEAT / 4;
/// Beats per bar (fixed 4/4 for the POC).
const BEATS_PER_BAR: u64 = 4;

/// Canvas logical width; at the 1280-wide scene the pane is wider than this,
/// so the Fill transform resolves to an identity (1 logical unit = 1 px) and
/// pointer coordinates map back exactly.
const CANVAS_W: f32 = 1120.0;
const RULER_H: f32 = 26.0;
const LANE_H: f32 = 54.0;
/// Left gutter that carries the track name inside each lane.
const LABEL_W: f32 = 96.0;
const CLIP_PAD: f32 = 6.0;
/// Pointer distance (logical px) from a clip's right edge that starts a trim
/// instead of a move.
const EDGE_GRAB: f32 = 10.0;
const DEFAULT_PX_PER_BEAT: f32 = 48.0;
const MIN_PX_PER_BEAT: f32 = 12.0;
const MAX_PX_PER_BEAT: f32 = 200.0;

const BPM_STEP: f64 = 5.0;
const VOLUME_STEP: f32 = 0.1;
const PAN_STEP: f32 = 0.1;

/// Load state of one model source's media bytes.
enum LoadState {
    Loaded,
    Pending,
    Failed(String),
}

/// What a live pointer drag is doing to `clip`. The preview geometry lives on
/// the drag until release, when exactly one command commits it — so a drag is
/// a single undo entry, not one per moved frame.
#[derive(Clone, Copy)]
enum DragMode {
    Move,
    TrimEnd,
}

struct Drag {
    clip: ClipId,
    track: TrackId,
    mode: DragMode,
    /// Tick under the pointer at press.
    grab_ticks: u64,
    orig_position: u64,
    orig_length: u64,
    source_offset: u64,
    /// Source duration, the trim ceiling for `source_offset + length`.
    source_duration: u64,
    cur_position: u64,
    cur_length: u64,
}

struct App {
    model: DawModel,
    /// Negotiated stream shape the engine prepares and renders at.
    config: EngineConfig,
    sources: BTreeMap<SourceId, SourceData>,
    load_states: BTreeMap<SourceId, LoadState>,
    engine: Option<Engine>,
    stream: Option<u32>,
    /// File reads are single-flight: the result event carries no request id,
    /// so the head of this queue owns the next FileReadResult.
    read_queue: VecDeque<SourceId>,
    inflight: Option<SourceId>,
    /// Parent dirs already requested via fs:read capability.
    requested_roots: Vec<String>,
    /// Parent dirs the host has granted fs:read for; reads only issue for
    /// sources under a granted root.
    granted_roots: Vec<String>,
    /// Encoded mixdown WAV waiting for the fs:write grant of exactly its
    /// own parent root.
    pending_write: Option<PendingWrite>,
    last_status: String,
    node_id: u32,

    // ── UI state (stint 0516) ────────────────────────────────────────────────
    /// Selected track lane, clamped to the track list on read.
    sel_track: usize,
    /// Selected clip. Ids never rebind, so a deleted clip's id simply reads
    /// as "no live selection".
    sel_clip: Option<ClipId>,
    /// Left edge of the visible timeline window, in ticks.
    view_start_ticks: u64,
    /// Timeline zoom: logical px per beat.
    px_per_beat: f32,
    /// In-progress pointer drag; commits one command on release.
    drag: Option<Drag>,
}

struct Component;
static mut APP: Option<App> = None;
fn app() -> &'static mut App {
    unsafe { (*core::ptr::addr_of_mut!(APP)).as_mut().unwrap() }
}

// ── Demo content (audible with zero capability grants) ───────────────────────

fn demo_source_data(path: &str, sample_rate: u32) -> Option<SourceData> {
    match path {
        "demo:pluck" => {
            // Half a second of a decaying 220 Hz partial stack.
            let frames = (sample_rate / 2) as usize;
            let samples = (0..frames)
                .map(|i| {
                    let t = i as f32 / sample_rate as f32;
                    let decay = (1.0 - i as f32 / frames as f32).powi(2);
                    (t * core::f32::consts::TAU * 220.0).sin() * 0.6 * decay
                })
                .collect();
            Some(SourceData::AudioPcm { sample_rate, channels: 1, samples })
        }
        "demo:arp" => Some(SourceData::MidiNotes(vec![
            Note { key: 60, velocity: 100, start_ticks: 0, length_ticks: TICKS_PER_BEAT / 2 },
            Note { key: 64, velocity: 92, start_ticks: TICKS_PER_BEAT / 2, length_ticks: TICKS_PER_BEAT / 2 },
            Note { key: 67, velocity: 84, start_ticks: TICKS_PER_BEAT, length_ticks: TICKS_PER_BEAT / 2 },
            Note { key: 72, velocity: 76, start_ticks: TICKS_PER_BEAT * 3 / 2, length_ticks: TICKS_PER_BEAT / 2 },
        ])),
        _ => None,
    }
}

fn seed_demo_project(model: &mut DawModel) {
    let beat = TICKS_PER_BEAT;
    // Ids are allocated deterministically from an empty model in apply order:
    // Pluck=T1, pluck source=S2, its clips=C3..C5, Arp=T6, arp source=S7,
    // its clips=C8..C9.
    let cmds = [
        DawCommand::AddTrack { kind: TrackKind::Audio, name: "Pluck".into() },
        DawCommand::AddSource { kind: TrackKind::Audio, path: "demo:pluck".into(), duration: 2 * beat },
        DawCommand::AddClip { track: TrackId(1), source: SourceId(2), position: 0, length: beat, source_offset: 0 },
        DawCommand::AddClip { track: TrackId(1), source: SourceId(2), position: 2 * beat, length: beat, source_offset: 0 },
        DawCommand::AddClip { track: TrackId(1), source: SourceId(2), position: 4 * beat, length: beat, source_offset: 0 },
        DawCommand::AddTrack { kind: TrackKind::Midi, name: "Arp".into() },
        DawCommand::AddSource { kind: TrackKind::Midi, path: "demo:arp".into(), duration: 2 * beat },
        DawCommand::AddClip { track: TrackId(6), source: SourceId(7), position: 0, length: 2 * beat, source_offset: 0 },
        DawCommand::AddClip { track: TrackId(6), source: SourceId(7), position: 4 * beat, length: 2 * beat, source_offset: 0 },
        DawCommand::SetLoop { enabled: true, start: 0, end: 4 * beat },
    ];
    for cmd in cmds {
        let outcome = model.apply(cmd.clone());
        if !matches!(outcome, ApplyOutcome::Applied) {
            host_log::error(&format!("daw-engine-poc: demo seed {cmd:?} -> {outcome:?}"));
        }
    }
}

// ── App impl ─────────────────────────────────────────────────────────────────

/// A mixdown export waiting for its write grant; `root` must match the
/// granted `fs:write:` capability exactly before the write is issued.
struct PendingWrite {
    root: String,
    path: String,
    bytes: Vec<u8>,
}

/// Parent directory of an absolute source path, for capability scoping.
fn parent_root(path: &str) -> Option<String> {
    match path.rfind('/') {
        Some(i) if i > 0 => Some(path[..i].to_string()),
        _ => None,
    }
}

impl App {
    fn new(config: EngineConfig) -> Self {
        let mut model = DawModel::new();
        seed_demo_project(&mut model);
        let sel_clip = first_clip_id(&model);
        App {
            model,
            config,
            sources: BTreeMap::new(),
            load_states: BTreeMap::new(),
            engine: None,
            stream: None,
            read_queue: VecDeque::new(),
            inflight: None,
            requested_roots: Vec::new(),
            granted_roots: Vec::new(),
            pending_write: None,
            last_status: "ready".into(),
            node_id: 0,
            sel_track: 0,
            sel_clip,
            view_start_ticks: 0,
            px_per_beat: DEFAULT_PX_PER_BEAT,
            drag: None,
        }
    }

    /// Queues loading for every model source without data: `demo:` paths
    /// synthesize immediately, real paths go through fs:read capability +
    /// file-read effects.
    fn ensure_source_data(&mut self, effects: &mut Vec<Effect>) {
        let sources: Vec<(SourceId, String)> = self
            .model
            .project()
            .sources
            .iter()
            .map(|s| (s.id, s.path.clone()))
            .collect();
        for (id, path) in sources {
            if self.sources.contains_key(&id) || self.load_states.contains_key(&id) {
                continue;
            }
            if let Some(data) = demo_source_data(&path, self.config.sample_rate) {
                self.sources.insert(id, data);
                self.load_states.insert(id, LoadState::Loaded);
                continue;
            }
            let Some(root) = parent_root(&path) else {
                self.load_states.insert(
                    id,
                    LoadState::Failed(format!("source path {path} is not absolute")),
                );
                continue;
            };
            if !self.requested_roots.contains(&root) {
                self.requested_roots.push(root.clone());
                effects.push(Effect::RequestCapability(format!("fs:read:{root}")));
            }
            self.load_states.insert(id, LoadState::Pending);
            self.read_queue.push_back(id);
        }
        self.pump_reads(effects);
    }

    /// Issues the next queued read whose parent dir has been granted; one at
    /// a time because the result event carries no request id. Sources under
    /// still-pending roots stay queued.
    fn pump_reads(&mut self, effects: &mut Vec<Effect>) {
        if self.inflight.is_some() {
            return;
        }
        let mut i = 0;
        while i < self.read_queue.len() {
            let id = self.read_queue[i];
            let Some(path) = self.model.project().source(id).map(|s| s.path.clone()) else {
                // Source deleted while queued; ids never rebind, so drop it.
                self.read_queue.remove(i);
                self.load_states.remove(&id);
                continue;
            };
            let granted = parent_root(&path)
                .is_some_and(|root| self.granted_roots.contains(&root));
            if !granted {
                i += 1;
                continue;
            }
            self.read_queue.remove(i);
            host_log::info(&format!("daw-engine-poc: loading source {} from {path}", id.0));
            self.inflight = Some(id);
            effects.push(Effect::FileRead(FileReadEffect { path }));
            return;
        }
    }

    fn finish_read(&mut self, result: Result<Vec<u8>, String>, effects: &mut Vec<Effect>) {
        let Some(id) = self.inflight.take() else {
            host_log::warn("daw-engine-poc: file-read result with no read in flight");
            return;
        };
        let kind = self.model.project().source(id).map(|s| s.kind);
        let decoded = result.and_then(|bytes| match kind {
            Some(TrackKind::Audio) => wav::decode(&bytes).map(|w| SourceData::AudioPcm {
                sample_rate: w.sample_rate,
                channels: w.channels,
                samples: w.samples,
            }),
            Some(TrackKind::Midi) => midi::parse_smf(&bytes).map(SourceData::MidiNotes),
            None => Err("source removed while loading".to_string()),
        });
        match decoded {
            Ok(data) => {
                self.sources.insert(id, data);
                self.load_states.insert(id, LoadState::Loaded);
                // A source load never moves the transport; keep the playhead.
                self.reprepare(self.model.transport().position);
            }
            Err(e) => {
                host_log::error(&format!("daw-engine-poc: source {} load failed: {e}", id.0));
                self.load_states.insert(id, LoadState::Failed(e));
            }
        }
        self.pump_reads(effects);
    }

    /// Rebuilds the engine from the model — the only way edits reach the
    /// render path. `prev_tick` is the transport position before the edit:
    /// when the edit left the transport tick unchanged (a mid-playback
    /// mixer/clip edit), the live playhead carries over instead of
    /// rewinding; a `Seek` changes the tick, so the new position wins.
    fn reprepare(&mut self, prev_tick: u64) {
        let carried = self
            .engine
            .as_ref()
            .filter(|_| self.model.transport().position == prev_tick)
            .map(Engine::playhead);
        match Engine::prepare(
            self.model.project(),
            self.model.transport(),
            &self.sources,
            self.config,
        ) {
            Ok(mut engine) => {
                if let Some(playhead) = carried {
                    engine.set_playhead(playhead);
                }
                self.engine = Some(engine);
            }
            Err(e) => {
                host_log::error(&format!("daw-engine-poc: prepare failed: {e}"));
                self.last_status = format!("prepare failed: {e}");
                self.engine = None;
            }
        }
    }

    fn apply(&mut self, cmd: DawCommand, effects: &mut Vec<Effect>) -> ApplyOutcome {
        let prev_tick = self.model.transport().position;
        let outcome = self.model.apply(cmd);
        if matches!(outcome, ApplyOutcome::Applied) {
            self.ensure_source_data(effects);
            self.reprepare(prev_tick);
        }
        outcome
    }

    // ── Selection / view helpers ──────────────────────────────────────────────

    fn tracks_len(&self) -> usize {
        self.model.project().tracks.len()
    }

    /// Selected lane index clamped into the current track list.
    fn sel_track_idx(&self) -> usize {
        let n = self.tracks_len();
        if n == 0 {
            0
        } else {
            self.sel_track.min(n - 1)
        }
    }

    fn sel_track_id(&self) -> Option<TrackId> {
        self.model.project().tracks.get(self.sel_track_idx()).map(|t| t.id)
    }

    /// The selected clip resolved against the live model, plus its owning
    /// track id — `None` once the clip has been removed.
    fn sel_clip_ref(&self) -> Option<(TrackId, Clip)> {
        let id = self.sel_clip?;
        let (ti, ci) = self.model.project().find_clip(id)?;
        let track = &self.model.project().tracks[ti];
        Some((track.id, track.clips[ci].clone()))
    }

    /// Clips on the selected track, ordered left-to-right by position — the
    /// order `[` / `]` and the clip-select buttons walk.
    fn sel_track_clips_sorted(&self) -> Vec<ClipId> {
        let Some(track) = self.model.project().tracks.get(self.sel_track_idx()) else {
            return Vec::new();
        };
        let mut clips: Vec<&Clip> = track.clips.iter().collect();
        clips.sort_by_key(|c| (c.position, c.id.0));
        clips.iter().map(|c| c.id).collect()
    }

    fn ticks_to_x(&self, tick: u64) -> f32 {
        let beats = (tick as f64 - self.view_start_ticks as f64) / BEAT as f64;
        LABEL_W + beats as f32 * self.px_per_beat
    }

    fn x_to_ticks(&self, x: f32) -> u64 {
        let beats = ((x - LABEL_W) / self.px_per_beat) as f64;
        let ticks = self.view_start_ticks as f64 + beats * BEAT as f64;
        ticks.max(0.0) as u64
    }

    fn timeline_height(&self) -> f32 {
        RULER_H + self.tracks_len().max(1) as f32 * LANE_H
    }

    /// Ticks visible between the gutter and the right edge at the current zoom.
    fn view_span_ticks(&self) -> u64 {
        let beats = ((CANVAS_W - LABEL_W) / self.px_per_beat).max(1.0) as f64;
        (beats * BEAT as f64) as u64
    }

    /// Scrolls the view so the playhead stays on screen after a transport or
    /// selection move.
    fn ensure_playhead_visible(&mut self) {
        let p = self.model.transport().position;
        let span = self.view_span_ticks();
        if p < self.view_start_ticks {
            self.view_start_ticks = p.saturating_sub(BEAT);
        } else if p >= self.view_start_ticks + span {
            self.view_start_ticks = p.saturating_sub(span / 2);
        }
    }

    fn snap(ticks: u64) -> u64 {
        ((ticks + GRID_TICKS / 2) / GRID_TICKS) * GRID_TICKS
    }

    // ── Operations (each routes through DawCommand) ───────────────────────────

    fn op_toggle_play(&mut self, effects: &mut Vec<Effect>) {
        let cmd = if self.model.transport().playing {
            DawCommand::Stop
        } else {
            DawCommand::Play
        };
        self.apply(cmd, effects);
    }

    fn op_seek_beats(&mut self, delta: i64, effects: &mut Vec<Effect>) {
        let pos = self.model.transport().position;
        let target = (pos as i64 + delta * BEAT as i64).max(0) as u64;
        self.apply(DawCommand::Seek { position: target }, effects);
        self.ensure_playhead_visible();
    }

    fn op_seek_ticks(&mut self, ticks: u64, effects: &mut Vec<Effect>) {
        self.apply(DawCommand::Seek { position: ticks }, effects);
        self.ensure_playhead_visible();
    }

    fn op_seek_end(&mut self, effects: &mut Vec<Effect>) {
        let end = self
            .model
            .project()
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.position + c.length)
            .max()
            .unwrap_or(0);
        self.apply(DawCommand::Seek { position: end }, effects);
        self.ensure_playhead_visible();
    }

    fn op_select_track(&mut self, delta: i64) {
        let n = self.tracks_len();
        if n == 0 {
            return;
        }
        let cur = self.sel_track_idx() as i64;
        self.sel_track = (cur + delta).rem_euclid(n as i64) as usize;
        // Keep a live clip selection on the newly focused track.
        let clips = self.sel_track_clips_sorted();
        if !self.sel_clip.is_some_and(|id| clips.contains(&id)) {
            self.sel_clip = clips.first().copied();
        }
    }

    fn op_select_clip(&mut self, delta: i64) {
        let clips = self.sel_track_clips_sorted();
        if clips.is_empty() {
            self.sel_clip = None;
            return;
        }
        let cur = self
            .sel_clip
            .and_then(|id| clips.iter().position(|c| *c == id))
            .unwrap_or(0) as i64;
        let next = (cur + delta).rem_euclid(clips.len() as i64) as usize;
        self.sel_clip = Some(clips[next]);
    }

    fn op_move_clip(&mut self, delta_beats: i64, effects: &mut Vec<Effect>) {
        let Some((track, clip)) = self.sel_clip_ref() else {
            return;
        };
        let target = (clip.position as i64 + delta_beats * BEAT as i64).max(0) as u64;
        self.apply(
            DawCommand::MoveClip { clip: clip.id, track, position: Self::snap(target) },
            effects,
        );
    }

    fn op_trim_clip_end(&mut self, delta_beats: i64, effects: &mut Vec<Effect>) {
        let Some((_, clip)) = self.sel_clip_ref() else {
            return;
        };
        let Some(source) = self.model.project().source(clip.source).cloned() else {
            return;
        };
        let max_len = source.duration.saturating_sub(clip.source_offset);
        let target = (clip.length as i64 + delta_beats * BEAT as i64).max(GRID_TICKS as i64) as u64;
        let length = Self::snap(target).clamp(GRID_TICKS, max_len.max(GRID_TICKS));
        self.apply(
            DawCommand::ResizeClip { clip: clip.id, length, source_offset: clip.source_offset },
            effects,
        );
    }

    /// Splits the selected clip at the playhead into two abutting clips.
    /// Left part shrinks in place (ResizeClip); the right part is a new clip
    /// with an advanced source offset (AddClip). The new clip becomes the
    /// selection.
    fn op_split_clip(&mut self, effects: &mut Vec<Effect>) {
        let Some((track, clip)) = self.sel_clip_ref() else {
            return;
        };
        let p = self.model.transport().position;
        if p <= clip.position || p >= clip.position + clip.length {
            self.last_status = "split: playhead is not inside the selected clip".into();
            return;
        }
        let left_len = p - clip.position;
        let right_len = clip.position + clip.length - p;
        let right_offset = clip.source_offset + left_len;
        if !matches!(
            self.apply(
                DawCommand::ResizeClip { clip: clip.id, length: left_len, source_offset: clip.source_offset },
                effects,
            ),
            ApplyOutcome::Applied
        ) {
            return;
        }
        if matches!(
            self.apply(
                DawCommand::AddClip {
                    track,
                    source: clip.source,
                    position: p,
                    length: right_len,
                    source_offset: right_offset,
                },
                effects,
            ),
            ApplyOutcome::Applied
        ) {
            // The new clip took the next id; select it.
            self.sel_clip = Some(ClipId(self.model.project().next_id - 1));
        }
    }

    fn op_delete_clip(&mut self, effects: &mut Vec<Effect>) {
        let Some(id) = self.sel_clip else {
            return;
        };
        if matches!(self.apply(DawCommand::RemoveClip { clip: id }, effects), ApplyOutcome::Applied) {
            self.sel_clip = self.sel_track_clips_sorted().first().copied();
        }
    }

    fn op_add_track(&mut self, kind: TrackKind, effects: &mut Vec<Effect>) {
        let name = format!("Track {}", self.tracks_len() + 1);
        if matches!(self.apply(DawCommand::AddTrack { kind, name }, effects), ApplyOutcome::Applied) {
            self.sel_track = self.tracks_len().saturating_sub(1);
            self.sel_clip = None;
        }
    }

    fn op_toggle_mute(&mut self, track: TrackId, effects: &mut Vec<Effect>) {
        let Some(t) = self.model.project().track(track) else {
            return;
        };
        let mute = !t.mixer.mute;
        self.apply(DawCommand::SetTrackMute { track, mute }, effects);
    }

    fn op_toggle_solo(&mut self, track: TrackId, effects: &mut Vec<Effect>) {
        let Some(t) = self.model.project().track(track) else {
            return;
        };
        let solo = !t.mixer.solo;
        self.apply(DawCommand::SetTrackSolo { track, solo }, effects);
    }

    fn op_adjust_volume(&mut self, track: TrackId, delta: f32, effects: &mut Vec<Effect>) {
        let Some(t) = self.model.project().track(track) else {
            return;
        };
        let volume = t.mixer.volume + delta;
        self.apply(DawCommand::SetTrackVolume { track, volume }, effects);
    }

    fn op_adjust_pan(&mut self, track: TrackId, delta: f32, effects: &mut Vec<Effect>) {
        let Some(t) = self.model.project().track(track) else {
            return;
        };
        let pan = t.mixer.pan + delta;
        self.apply(DawCommand::SetTrackPan { track, pan }, effects);
    }

    fn op_adjust_tempo(&mut self, delta: f64, effects: &mut Vec<Effect>) {
        let bpm = self.model.project().tempo_bpm + delta;
        self.apply(DawCommand::SetTempo { bpm }, effects);
    }

    fn op_toggle_loop(&mut self, effects: &mut Vec<Effect>) {
        let t = *self.model.transport();
        let cmd = if t.loop_enabled {
            DawCommand::SetLoop { enabled: false, start: t.loop_start, end: t.loop_end }
        } else if t.loop_start < t.loop_end {
            DawCommand::SetLoop { enabled: true, start: t.loop_start, end: t.loop_end }
        } else {
            DawCommand::SetLoop { enabled: true, start: 0, end: 4 * BEAT }
        };
        self.apply(cmd, effects);
    }

    fn op_zoom(&mut self, factor: f32) {
        self.px_per_beat = (self.px_per_beat * factor).clamp(MIN_PX_PER_BEAT, MAX_PX_PER_BEAT);
        self.ensure_playhead_visible();
    }

    // ── Input routing ─────────────────────────────────────────────────────────

    /// Keyboard map — every operation the scope names has a key here. See the
    /// footer node for the human-facing legend.
    fn on_key(&mut self, key: &str, effects: &mut Vec<Effect>) {
        match key {
            // Transport
            "space" => self.op_toggle_play(effects),
            "left" => self.op_seek_beats(-1, effects),
            "right" => self.op_seek_beats(1, effects),
            "home" => self.op_seek_ticks(0, effects),
            "end" => self.op_seek_end(effects),
            "\\" => self.op_toggle_loop(effects),
            "-" => self.op_adjust_tempo(-BPM_STEP, effects),
            "=" => self.op_adjust_tempo(BPM_STEP, effects),
            // Timeline view
            "pageup" => self.op_zoom(1.0 / 1.25),
            "pagedown" => self.op_zoom(1.25),
            // Navigation
            "up" => self.op_select_track(-1),
            "down" => self.op_select_track(1),
            "[" => self.op_select_clip(-1),
            "]" => self.op_select_clip(1),
            // Clip editing
            "," => self.op_move_clip(-1, effects),
            "." => self.op_move_clip(1, effects),
            ";" => self.op_trim_clip_end(-1, effects),
            "'" => self.op_trim_clip_end(1, effects),
            "s" => self.op_split_clip(effects),
            "x" | "delete" | "backspace" => self.op_delete_clip(effects),
            // Track / mixer
            "t" => self.op_add_track(TrackKind::Audio, effects),
            "y" => self.op_add_track(TrackKind::Midi, effects),
            "m" => {
                if let Some(track) = self.sel_track_id() {
                    self.op_toggle_mute(track, effects);
                }
            }
            "n" => {
                if let Some(track) = self.sel_track_id() {
                    self.op_toggle_solo(track, effects);
                }
            }
            "9" => {
                if let Some(track) = self.sel_track_id() {
                    self.op_adjust_volume(track, -VOLUME_STEP, effects);
                }
            }
            "0" => {
                if let Some(track) = self.sel_track_id() {
                    self.op_adjust_volume(track, VOLUME_STEP, effects);
                }
            }
            "k" => {
                if let Some(track) = self.sel_track_id() {
                    self.op_adjust_pan(track, -PAN_STEP, effects);
                }
            }
            "l" => {
                if let Some(track) = self.sel_track_id() {
                    self.op_adjust_pan(track, PAN_STEP, effects);
                }
            }
            // History
            "z" => {
                self.apply(DawCommand::Undo, effects);
            }
            "r" => {
                self.apply(DawCommand::Redo, effects);
            }
            "q" | "escape" => effects.push(Effect::CloseSelf),
            _ => {}
        }
    }

    /// Button / clip-select handlers. `handler_id` encodes op and, where
    /// needed, its target id — self-contained so dispatch never reads ambient
    /// state (the command-handler-data rule).
    fn on_action(&mut self, handler: &str, effects: &mut Vec<Effect>) {
        let (verb, arg) = handler.split_once(':').map_or((handler, None), |(v, a)| (v, Some(a)));
        let track_arg = || arg.and_then(|a| a.parse::<u64>().ok()).map(TrackId);
        match verb {
            "toggle-play" => self.op_toggle_play(effects),
            "stop" => {
                self.apply(DawCommand::Stop, effects);
            }
            "loop" => self.op_toggle_loop(effects),
            "bpm-" => self.op_adjust_tempo(-BPM_STEP, effects),
            "bpm+" => self.op_adjust_tempo(BPM_STEP, effects),
            "undo" => {
                self.apply(DawCommand::Undo, effects);
            }
            "redo" => {
                self.apply(DawCommand::Redo, effects);
            }
            "add-track" => self.op_add_track(TrackKind::Audio, effects),
            "split" => self.op_split_clip(effects),
            "del-clip" => self.op_delete_clip(effects),
            "mute" => {
                if let Some(track) = track_arg() {
                    self.op_toggle_mute(track, effects);
                }
            }
            "solo" => {
                if let Some(track) = track_arg() {
                    self.op_toggle_solo(track, effects);
                }
            }
            "vol-" => {
                if let Some(track) = track_arg() {
                    self.op_adjust_volume(track, -VOLUME_STEP, effects);
                }
            }
            "vol+" => {
                if let Some(track) = track_arg() {
                    self.op_adjust_volume(track, VOLUME_STEP, effects);
                }
            }
            "pan-" => {
                if let Some(track) = track_arg() {
                    self.op_adjust_pan(track, -PAN_STEP, effects);
                }
            }
            "pan+" => {
                if let Some(track) = track_arg() {
                    self.op_adjust_pan(track, PAN_STEP, effects);
                }
            }
            "clip" => {
                if let Some(id) = arg.and_then(|a| a.parse::<u64>().ok()) {
                    let clip = ClipId(id);
                    if let Some((ti, _)) = self.model.project().find_clip(clip) {
                        self.sel_track = ti;
                        self.sel_clip = Some(clip);
                    }
                }
            }
            other => host_log::info(&format!("daw-engine-poc: unknown action {other}")),
        }
    }

    /// Pointer down: seek from the ruler, or grab a clip for move/trim.
    fn on_pointer_press(&mut self, x: f32, y: f32, effects: &mut Vec<Effect>) {
        if x < LABEL_W {
            return;
        }
        if y < RULER_H {
            let ticks = Self::snap(self.x_to_ticks(x));
            self.op_seek_ticks(ticks, effects);
            return;
        }
        let ti = ((y - RULER_H) / LANE_H) as usize;
        let Some(track) = self.model.project().tracks.get(ti) else {
            return;
        };
        let track_id = track.id;
        // Topmost clip wins: scan the draw order (vec order) back-to-front.
        let hit = track.clips.iter().rev().find_map(|clip| {
            let x0 = self.ticks_to_x(clip.position);
            let x1 = self.ticks_to_x(clip.position + clip.length);
            (x >= x0 && x <= x1).then_some((clip.clone(), x1))
        });
        let Some((clip, x1)) = hit else {
            return;
        };
        let source_duration = self
            .model
            .project()
            .source(clip.source)
            .map(|s| s.duration)
            .unwrap_or(clip.source_offset + clip.length);
        let mode = if (x1 - x).abs() <= EDGE_GRAB {
            DragMode::TrimEnd
        } else {
            DragMode::Move
        };
        self.sel_track = ti;
        self.sel_clip = Some(clip.id);
        self.drag = Some(Drag {
            clip: clip.id,
            track: track_id,
            mode,
            grab_ticks: self.x_to_ticks(x),
            orig_position: clip.position,
            orig_length: clip.length,
            source_offset: clip.source_offset,
            source_duration,
            cur_position: clip.position,
            cur_length: clip.length,
        });
    }

    /// Pointer move: update the drag preview only. Nothing reaches the model
    /// until release, so the whole gesture is one undo entry.
    fn on_pointer_move(&mut self, x: f32) {
        let ptr = self.x_to_ticks(x);
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        let delta = ptr as i64 - drag.grab_ticks as i64;
        match drag.mode {
            DragMode::Move => {
                let target = (drag.orig_position as i64 + delta).max(0) as u64;
                drag.cur_position = Self::snap(target);
            }
            DragMode::TrimEnd => {
                let max_len = drag.source_duration.saturating_sub(drag.source_offset).max(GRID_TICKS);
                let target = (drag.orig_length as i64 + delta).max(GRID_TICKS as i64) as u64;
                drag.cur_length = Self::snap(target).clamp(GRID_TICKS, max_len);
            }
        }
    }

    /// Pointer up: commit the drag preview as exactly one command.
    fn on_pointer_release(&mut self, effects: &mut Vec<Effect>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        match drag.mode {
            DragMode::Move => {
                self.apply(
                    DawCommand::MoveClip { clip: drag.clip, track: drag.track, position: drag.cur_position },
                    effects,
                );
            }
            DragMode::TrimEnd => {
                self.apply(
                    DawCommand::ResizeClip {
                        clip: drag.clip,
                        length: drag.cur_length,
                        source_offset: drag.source_offset,
                    },
                    effects,
                );
            }
        }
    }

    fn on_mouse(&mut self, event: MouseEvent, effects: &mut Vec<Effect>) {
        match (event.button, event.pressed) {
            (Some(MouseButton::Left), true) => {
                self.on_pointer_press(event.position.x, event.position.y, effects)
            }
            (None, false) => self.on_pointer_move(event.position.x),
            (Some(_), false) => self.on_pointer_release(effects),
            _ => {}
        }
    }

    // ── Tools ────────────────────────────────────────────────────────────────

    fn tool_decls() -> Vec<ToolDecl> {
        // The daw.* connector surface (stint 0517): specs live in
        // plexi-daw-tools so the schemas and the dispatch that enforces them
        // can never drift apart.
        let mut decls: Vec<ToolDecl> = plexi_daw_tools::tools()
            .into_iter()
            .map(|spec| ToolDecl {
                name: spec.name.into(),
                description: spec.description,
                input_schema_json: spec.input_schema.to_string(),
                output_schema_json: spec.output_schema.to_string(),
                timeout_ms: None,
                read_only: spec.read_only,
            })
            .collect();
        decls.extend([
            ToolDecl {
                name: TOOL_MIXDOWN.into(),
                description: "Deterministic offline mixdown of the project (transport start to \
                              project end) through the same render path as playback. Returns \
                              frames, sample rate, channels, and the FNV-1a PCM hash; writes a \
                              float32 WAV when out_path is given."
                    .into(),
                input_schema_json: r#"{"type":"object","properties":{"out_path":{"type":"string","description":"Absolute path to write a float32 WAV to (requires fs write grant)"}}}"#.into(),
                output_schema_json: r#"{"type":"object","properties":{"frames":{"type":"integer"},"sample_rate":{"type":"integer"},"channels":{"type":"integer"},"pcm_hash":{"type":"string"},"write_queued":{"type":"string"}}}"#.into(),
                timeout_ms: None,
                // Not read-only: out_path makes this write a file.
                read_only: false,
            },
            ToolDecl {
                name: TOOL_COMMAND.into(),
                description: "Apply one DawCommand (serde JSON, e.g. {\"Play\":null} or \
                              {\"SetTempo\":{\"bpm\":140.0}}) to the edit model. The command enum \
                              is the only mutation path; the engine re-prepares from the model."
                    .into(),
                input_schema_json: r#"{"type":"object","properties":{"command":{"type":"object","description":"DawCommand in serde external-tag form"}},"required":["command"]}"#.into(),
                output_schema_json: r#"{"type":"object","properties":{"outcome":{"type":"string"},"revision":{"type":"integer"}}}"#.into(),
                timeout_ms: None,
                read_only: false,
            },
        ]);
        decls
    }

    /// Routes one `daw.*` call through the pure connector crate, then
    /// re-prepares the engine when the model's revision moved — the same
    /// follow-the-model contract as key handling. Returns `None` for names
    /// outside the `daw.*` surface.
    fn daw_tool(
        &mut self,
        name: &str,
        input_json: &str,
        effects: &mut Vec<Effect>,
    ) -> Option<Result<String, String>> {
        let prev_tick = self.model.transport().position;
        let prev_revision = self.model.revision();
        let result = plexi_daw_tools::dispatch(&mut self.model, name, input_json)?;
        if self.model.revision() != prev_revision {
            self.ensure_source_data(effects);
            self.reprepare(prev_tick);
            self.last_status = format!("{name} applied");
        }
        match &result {
            Ok(_) => host_log::info(&format!("daw-engine-poc: tool {name} ok")),
            Err(e) => host_log::info(&format!("daw-engine-poc: tool {name} error: {e}")),
        }
        Some(result.and_then(|value| {
            serde_json::to_string(&value).map_err(|e| format!("{name} result: {e}"))
        }))
    }

    fn handle_tool_call(&mut self, call: ToolCallEvent, effects: &mut Vec<Effect>) {
        let result = match self.daw_tool(&call.name, &call.input_json, effects) {
            Some(result) => result,
            None => match call.name.as_str() {
                TOOL_MIXDOWN => self.tool_mixdown(&call.input_json, effects),
                TOOL_COMMAND => self.tool_command(&call.input_json, effects),
                other => Err(format!("unknown tool {other}")),
            },
        };
        let (output_json, error) = match result {
            Ok(json) => (Some(json), None),
            Err(e) => (None, Some(e)),
        };
        effects.push(Effect::ToolResult(ToolResultEffect {
            call_id: call.call_id,
            output_json,
            error,
        }));
    }

    fn tool_mixdown(&mut self, input_json: &str, effects: &mut Vec<Effect>) -> Result<String, String> {
        let input: serde_json::Value = if input_json.trim().is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_str(input_json).map_err(|e| format!("mixdown input: {e}"))?
        };
        let engine = self.engine.as_ref().ok_or("engine not prepared")?;
        let fpt = f64::from(self.config.sample_rate) * 60.0
            / (self.model.project().tempo_bpm * TICKS_PER_BEAT as f64);
        let start = (self.model.transport().position as f64 * fpt).round() as u64;
        let end = engine.project_end_frame();
        if end <= start {
            return Err(format!(
                "nothing to render: transport frame {start} is at or past project end {end}"
            ));
        }
        let mix = engine.mixdown(start, end)?;
        let hash = pcm_hash(&mix.samples);
        let frames = mix.samples.len() as u32 / mix.channels;
        let mut out = serde_json::json!({
            "frames": frames,
            "sample_rate": mix.sample_rate,
            "channels": mix.channels,
            "pcm_hash": format!("{hash:016x}"),
        });
        if let Some(path) = input.get("out_path").and_then(|v| v.as_str()) {
            let root = parent_root(path)
                .ok_or_else(|| format!("out_path {path} is not an absolute file path"))?;
            let bytes = wav::encode_f32(mix.sample_rate, mix.channels, &mix.samples)?;
            if self.pending_write.is_some() {
                host_log::warn("daw-engine-poc: replacing a mixdown write still awaiting its grant");
            }
            // The write waits for its fs:write grant; completion is reported
            // by the FileWriteResult handler, never here.
            effects.push(Effect::RequestCapability(format!("fs:write:{root}")));
            self.pending_write = Some(PendingWrite { root, path: path.to_string(), bytes });
            out["write_queued"] = serde_json::Value::String(path.to_string());
        }
        self.last_status = format!("mixdown {frames} frames hash {hash:016x}");
        host_log::info(&format!("daw-engine-poc: {}", self.last_status));
        serde_json::to_string(&out).map_err(|e| format!("mixdown result: {e}"))
    }

    fn tool_command(
        &mut self,
        input_json: &str,
        effects: &mut Vec<Effect>,
    ) -> Result<String, String> {
        let input: serde_json::Value =
            serde_json::from_str(input_json).map_err(|e| format!("daw_command input: {e}"))?;
        let cmd_value = input
            .get("command")
            .ok_or("daw_command input missing \"command\"")?;
        let cmd: DawCommand = serde_json::from_value(cmd_value.clone())
            .map_err(|e| format!("not a DawCommand: {e}"))?;
        // Effects flow to the caller: an AddSource with a real path queues
        // its capability request and file read right here.
        let outcome = self.apply(cmd, effects);
        let outcome_str = match &outcome {
            ApplyOutcome::Applied => "Applied".to_string(),
            ApplyOutcome::NoOp => "NoOp".to_string(),
            ApplyOutcome::Rejected(reason) => format!("Rejected: {reason}"),
        };
        self.last_status = format!("daw_command -> {outcome_str}");
        serde_json::to_string(&serde_json::json!({
            "outcome": outcome_str,
            "revision": self.model.revision(),
        }))
        .map_err(|e| format!("daw_command result: {e}"))
    }

    // ── View ─────────────────────────────────────────────────────────────────

    fn nid(&mut self, key: &str, data: UiNodeData) -> IndexedNode {
        let id = self.node_id;
        self.node_id += 1;
        IndexedNode { id, key: key.to_string(), data }
    }

    fn text(&mut self, key: &str, text: String, size: f32, bold: bool) -> IndexedNode {
        self.nid(
            key,
            UiNodeData::Text(TextNode {
                text,
                size: Some(size),
                bold,
                color: None,
                truncate: false,
                align: Alignment::Start,
            }),
        )
    }

    fn button(
        &mut self,
        key: &str,
        label: &str,
        handler: &str,
        style: ButtonStyle,
        disabled: bool,
    ) -> IndexedNode {
        self.nid(
            key,
            UiNodeData::Button(ButtonNode {
                label: label.to_string(),
                on_click: handler.to_string(),
                style,
                disabled,
            }),
        )
    }

    fn build_tree(&mut self) -> UiTree {
        self.node_id = 0;
        let mut nodes: Vec<IndexedNode> = Vec::new();

        let transport = self.transport_row(&mut nodes);
        let timeline = self.timeline_node(&mut nodes);
        let mixer = self.mixer_row(&mut nodes);
        let inspector = self.inspector_row(&mut nodes);

        let status_line = match self.first_load_failure() {
            Some(reason) => format!("{} · source load failed: {reason}", self.last_status),
            None => self.last_status.clone(),
        };
        let status = self.text("status", status_line, 11.0, false);
        let status_id = status.id;
        nodes.push(status);

        let footer = self.nid(
            "footer",
            UiNodeData::FooterKeys(FooterKeysNode {
                divider: true,
                entries: vec![
                    FooterKeyEntry { keys: vec!["space".into()], description: "play/stop".into() },
                    FooterKeyEntry { keys: vec!["<-".into(), "->".into()], description: "seek".into() },
                    FooterKeyEntry { keys: vec!["up".into(), "down".into()], description: "track".into() },
                    FooterKeyEntry { keys: vec!["[".into(), "]".into()], description: "clip".into() },
                    FooterKeyEntry { keys: vec![",".into(), ".".into()], description: "move".into() },
                    FooterKeyEntry { keys: vec![";".into(), "'".into()], description: "trim".into() },
                    FooterKeyEntry { keys: vec!["s".into()], description: "split".into() },
                    FooterKeyEntry { keys: vec!["x".into()], description: "delete".into() },
                    FooterKeyEntry { keys: vec!["t".into()], description: "add track".into() },
                    FooterKeyEntry { keys: vec!["m".into(), "n".into()], description: "mute/solo".into() },
                    FooterKeyEntry { keys: vec!["9".into(), "0".into()], description: "vol".into() },
                    FooterKeyEntry { keys: vec!["k".into(), "l".into()], description: "pan".into() },
                    FooterKeyEntry { keys: vec!["-".into(), "=".into()], description: "bpm".into() },
                    FooterKeyEntry { keys: vec!["\\".into()], description: "loop".into() },
                    FooterKeyEntry { keys: vec!["z".into(), "r".into()], description: "undo/redo".into() },
                ],
            }),
        );
        let footer_id = footer.id;
        nodes.push(footer);

        let root = self.nid(
            "root",
            UiNodeData::Column(ColumnNode {
                children: vec![transport, timeline, mixer, inspector, status_id, footer_id],
                gap: 8.0,
                align: Alignment::Stretch,
                grow: true,
            }),
        );
        let root_id = root.id;
        nodes.push(root);
        UiTree { root: root_id, nodes }
    }

    fn transport_row(&mut self, nodes: &mut Vec<IndexedNode>) -> u32 {
        let transport = *self.model.transport();
        let playing = transport.playing;
        let beat = transport.position / BEAT;
        let bar = beat / BEATS_PER_BAR + 1;
        let beat_in_bar = beat % BEATS_PER_BAR + 1;
        let tempo = self.model.project().tempo_bpm;
        let rev = self.model.revision();

        let title = self.text("title", "DAW".into(), 18.0, true);
        let title_id = title.id;
        nodes.push(title);

        let play = self.button(
            "toggle-play",
            if playing { "Pause" } else { "Play" },
            "toggle-play",
            if playing { ButtonStyle::Secondary } else { ButtonStyle::Primary },
            false,
        );
        let play_id = play.id;
        nodes.push(play);

        let stop = self.button("stop", "Stop", "stop", ButtonStyle::Ghost, !playing);
        let stop_id = stop.id;
        nodes.push(stop);

        let loop_btn = self.button(
            "loop",
            if transport.loop_enabled { "Loop On" } else { "Loop Off" },
            "loop",
            if transport.loop_enabled { ButtonStyle::Primary } else { ButtonStyle::Ghost },
            false,
        );
        let loop_id = loop_btn.id;
        nodes.push(loop_btn);

        let pos = self.text(
            "position",
            format!("bar {bar} · beat {beat_in_bar}"),
            13.0,
            true,
        );
        let pos_id = pos.id;
        nodes.push(pos);

        let bpm_down = self.button("bpm-down", "-", "bpm-", ButtonStyle::Ghost, false);
        let bpm_down_id = bpm_down.id;
        nodes.push(bpm_down);

        let bpm = self.text("bpm", format!("{tempo:.0} BPM"), 13.0, false);
        let bpm_id = bpm.id;
        nodes.push(bpm);

        let bpm_up = self.button("bpm-up", "+", "bpm+", ButtonStyle::Ghost, false);
        let bpm_up_id = bpm_up.id;
        nodes.push(bpm_up);

        let add = self.button("add-track", "+ Track", "add-track", ButtonStyle::Secondary, false);
        let add_id = add.id;
        nodes.push(add);

        let undo = self.button("undo", "Undo", "undo", ButtonStyle::Ghost, !self.model.can_undo());
        let undo_id = undo.id;
        nodes.push(undo);

        let redo = self.button("redo", "Redo", "redo", ButtonStyle::Ghost, !self.model.can_redo());
        let redo_id = redo.id;
        nodes.push(redo);

        let badge = self.nid(
            "transport-state",
            UiNodeData::Badge(BadgeNode {
                text: if playing {
                    format!("PLAYING · rev {rev}")
                } else {
                    format!("STOPPED · rev {rev}")
                },
                color: if playing { BadgeColor::Accent } else { BadgeColor::Neutral },
            }),
        );
        let badge_id = badge.id;
        nodes.push(badge);

        let row = self.nid(
            "transport-row",
            UiNodeData::Row(RowNode {
                children: vec![
                    title_id, play_id, stop_id, loop_id, pos_id, bpm_down_id, bpm_id, bpm_up_id,
                    add_id, undo_id, redo_id, badge_id,
                ],
                gap: 8.0,
                align: Alignment::Center,
                grow: false,
            }),
        );
        let row_id = row.id;
        nodes.push(row);
        row_id
    }

    /// The reason string of the first source whose media failed to load, for
    /// the status line — capability denials and decode errors surface here.
    fn first_load_failure(&self) -> Option<String> {
        self.load_states.values().find_map(|s| match s {
            LoadState::Failed(reason) => Some(reason.clone()),
            _ => None,
        })
    }

    fn timeline_node(&mut self, nodes: &mut Vec<IndexedNode>) -> u32 {
        let canvas = self.timeline_canvas();
        let canvas_node = self.nid("timeline", UiNodeData::Canvas(canvas));
        let canvas_id = canvas_node.id;
        nodes.push(canvas_node);
        canvas_id
    }

    fn timeline_canvas(&self) -> CanvasNode {
        let height = self.timeline_height();
        let tracks = &self.model.project().tracks;
        let transport = self.model.transport();
        let sel = self.sel_track_idx();
        let mut cmds: Vec<CanvasCommand> = Vec::new();

        // 1. Backdrop.
        push_rect(&mut cmds, 0.0, 0.0, CANVAS_W, height, col(18, 18, 22, 255), 0.0);
        // 2. Ruler strip.
        push_rect(&mut cmds, 0.0, 0.0, CANVAS_W, RULER_H, col(30, 30, 38, 255), 0.0);

        // 3. Lanes + gutter labels.
        for (ti, track) in tracks.iter().enumerate() {
            let y = RULER_H + ti as f32 * LANE_H;
            let lane = if ti == sel {
                col(42, 46, 62, 255)
            } else if ti % 2 == 0 {
                col(26, 26, 32, 255)
            } else {
                col(22, 22, 28, 255)
            };
            push_rect(&mut cmds, 0.0, y, CANVAS_W, LANE_H, lane, 0.0);
            // Gutter divider.
            push_line(&mut cmds, LABEL_W, y, LABEL_W, y + LANE_H, col(55, 55, 66, 255), 1.0);
            let kind_tag = match track.kind {
                TrackKind::Audio => "AUD",
                TrackKind::Midi => "MIDI",
            };
            push_text(&mut cmds, 6.0, y + 8.0, track.name.clone(), 13.0, col(232, 236, 245, 255), true);
            let mut flags = format!("{kind_tag} {:.2}", track.mixer.volume);
            if track.mixer.mute {
                flags.push_str(" M");
            }
            if track.mixer.solo {
                flags.push_str(" S");
            }
            push_text(&mut cmds, 6.0, y + 28.0, flags, 10.0, col(150, 156, 172, 255), false);
        }

        // 4. Loop region shade (lanes area).
        if transport.loop_enabled {
            let x0 = self.ticks_to_x(transport.loop_start).max(LABEL_W);
            let x1 = self.ticks_to_x(transport.loop_end).min(CANVAS_W);
            if x1 > x0 {
                push_rect(&mut cmds, x0, RULER_H, x1 - x0, height - RULER_H, col(90, 150, 100, 40), 0.0);
            }
        }

        // 5. Beat / bar grid over the whole height.
        let first_beat = self.view_start_ticks / BEAT;
        let last_beat = self.x_to_ticks(CANVAS_W) / BEAT + 1;
        for b in first_beat..=last_beat {
            let x = self.ticks_to_x(b * BEAT);
            if x < LABEL_W || x > CANVAS_W {
                continue;
            }
            let is_bar = b % BEATS_PER_BAR == 0;
            let color = if is_bar { col(78, 80, 96, 255) } else { col(46, 46, 56, 255) };
            push_line(&mut cmds, x, 0.0, x, height, color, if is_bar { 1.5 } else { 1.0 });
            if is_bar {
                push_text(&mut cmds, x + 3.0, 4.0, format!("{}", b / BEATS_PER_BAR + 1), 11.0, col(176, 182, 198, 255), false);
            }
        }

        // 6. Clips.
        for (ti, track) in tracks.iter().enumerate() {
            let lane_y = RULER_H + ti as f32 * LANE_H;
            for clip in &track.clips {
                let (position, length) = self.clip_geom(clip);
                let x0 = self.ticks_to_x(position);
                let x1 = self.ticks_to_x(position + length);
                if x1 <= LABEL_W || x0 >= CANVAS_W {
                    continue;
                }
                let cx = x0.max(LABEL_W);
                let cw = (x1.min(CANVAS_W) - cx).max(2.0);
                let cy = lane_y + CLIP_PAD;
                let ch = LANE_H - 2.0 * CLIP_PAD;
                let selected = self.sel_clip == Some(clip.id);
                let mut fill = match track.kind {
                    TrackKind::Audio => col(56, 122, 142, 255),
                    TrackKind::Midi => col(122, 84, 162, 255),
                };
                if track.mixer.mute {
                    fill = dim(fill);
                }
                push_rect(&mut cmds, cx, cy, cw, ch, fill, 4.0);
                if selected {
                    let border = col(240, 226, 150, 255);
                    push_line(&mut cmds, cx, cy, cx + cw, cy, border, 2.0);
                    push_line(&mut cmds, cx, cy + ch, cx + cw, cy + ch, border, 2.0);
                    push_line(&mut cmds, cx, cy, cx, cy + ch, border, 2.0);
                    push_line(&mut cmds, cx + cw, cy, cx + cw, cy + ch, border, 2.0);
                }
                push_text(&mut cmds, cx + 5.0, cy + 4.0, source_label(&self.model, clip.source), 11.0, col(240, 244, 250, 255), false);
            }
        }

        // 7. Playhead.
        let px = self.ticks_to_x(transport.position);
        if px >= LABEL_W && px <= CANVAS_W {
            push_line(&mut cmds, px, 0.0, px, height, col(242, 120, 92, 255), 2.0);
        }

        CanvasNode { width: CANVAS_W, height, grow: false, commands: cmds }
    }

    /// Geometry to draw a clip at — the live drag preview when this clip is
    /// the one being dragged, otherwise its committed model geometry.
    fn clip_geom(&self, clip: &Clip) -> (u64, u64) {
        if let Some(drag) = &self.drag {
            if drag.clip == clip.id {
                return (drag.cur_position, drag.cur_length);
            }
        }
        (clip.position, clip.length)
    }

    fn mixer_row(&mut self, nodes: &mut Vec<IndexedNode>) -> u32 {
        let strips: Vec<(TrackId, String, f32, f32, bool, bool, bool)> = self
            .model
            .project()
            .tracks
            .iter()
            .enumerate()
            .map(|(ti, t)| {
                (t.id, t.name.clone(), t.mixer.volume, t.mixer.pan, t.mixer.mute, t.mixer.solo, ti == self.sel_track_idx())
            })
            .collect();

        let mut strip_ids = Vec::new();
        for (track, name, volume, pan, mute, solo, selected) in strips {
            let tid = track.0;
            let head = self.text(
                &format!("mix-name-{tid}"),
                if selected { format!("> {name}") } else { name.clone() },
                12.0,
                selected,
            );
            let head_id = head.id;
            nodes.push(head);

            let readout = self.text(
                &format!("mix-read-{tid}"),
                format!("vol {volume:.2} · pan {pan:+.2}"),
                10.0,
                false,
            );
            let readout_id = readout.id;
            nodes.push(readout);

            let mute_btn = self.button(
                &format!("mix-mute-{tid}"),
                "M",
                &format!("mute:{tid}"),
                if mute { ButtonStyle::Danger } else { ButtonStyle::Ghost },
                false,
            );
            let mute_id = mute_btn.id;
            nodes.push(mute_btn);

            let solo_btn = self.button(
                &format!("mix-solo-{tid}"),
                "S",
                &format!("solo:{tid}"),
                if solo { ButtonStyle::Primary } else { ButtonStyle::Ghost },
                false,
            );
            let solo_id = solo_btn.id;
            nodes.push(solo_btn);

            let vdn = self.button(&format!("mix-vol-down-{tid}"), "v-", &format!("vol-:{tid}"), ButtonStyle::Ghost, false);
            let vdn_id = vdn.id;
            nodes.push(vdn);
            let vup = self.button(&format!("mix-vol-up-{tid}"), "v+", &format!("vol+:{tid}"), ButtonStyle::Ghost, false);
            let vup_id = vup.id;
            nodes.push(vup);
            let pdn = self.button(&format!("mix-pan-down-{tid}"), "p-", &format!("pan-:{tid}"), ButtonStyle::Ghost, false);
            let pdn_id = pdn.id;
            nodes.push(pdn);
            let pup = self.button(&format!("mix-pan-up-{tid}"), "p+", &format!("pan+:{tid}"), ButtonStyle::Ghost, false);
            let pup_id = pup.id;
            nodes.push(pup);

            let controls = self.nid(
                &format!("mix-controls-{tid}"),
                UiNodeData::Row(RowNode {
                    children: vec![mute_id, solo_id, vdn_id, vup_id, pdn_id, pup_id],
                    gap: 4.0,
                    align: Alignment::Center,
                    grow: false,
                }),
            );
            let controls_id = controls.id;
            nodes.push(controls);

            let strip = self.nid(
                &format!("mix-strip-{tid}"),
                UiNodeData::Column(ColumnNode {
                    children: vec![head_id, readout_id, controls_id],
                    gap: 4.0,
                    align: Alignment::Start,
                    grow: false,
                }),
            );
            let strip_id = strip.id;
            nodes.push(strip);
            strip_ids.push(strip_id);
        }

        let row = self.nid(
            "mixer-row",
            UiNodeData::Row(RowNode { children: strip_ids, gap: 16.0, align: Alignment::Start, grow: false }),
        );
        let row_id = row.id;
        let padded = self.nid(
            "mixer",
            UiNodeData::Padding(PaddingNode { child: row_id, top: 4.0, right: 0.0, bottom: 4.0, left: 0.0 }),
        );
        let padded_id = padded.id;
        nodes.push(row);
        nodes.push(padded);
        padded_id
    }

    fn inspector_row(&mut self, nodes: &mut Vec<IndexedNode>) -> u32 {
        let summary = match self.sel_clip_ref() {
            Some((_, clip)) => format!(
                "Selected clip {} · start beat {} · len {} beat(s)",
                clip.id.0,
                clip.position / BEAT,
                (clip.length as f64 / BEAT as f64 * 100.0).round() / 100.0
            ),
            None => "No clip selected".to_string(),
        };
        let label = self.text("inspector", summary, 12.0, false);
        let label_id = label.id;
        nodes.push(label);

        // Node-addressable clip selection for the focused track: one button
        // per clip, so `expect`/`--node` can drive selection without pointers.
        let clips = self.sel_track_clips_sorted();
        let mut children = vec![label_id];
        for (i, clip_id) in clips.iter().enumerate() {
            let selected = self.sel_clip == Some(*clip_id);
            let btn = self.button(
                &format!("clip-{}", clip_id.0),
                &format!("clip {}", i + 1),
                &format!("clip:{}", clip_id.0),
                if selected { ButtonStyle::Primary } else { ButtonStyle::Ghost },
                false,
            );
            children.push(btn.id);
            nodes.push(btn);
        }
        let split = self.button("split", "Split", "split", ButtonStyle::Secondary, self.sel_clip.is_none());
        children.push(split.id);
        nodes.push(split);
        let del = self.button("del-clip", "Delete", "del-clip", ButtonStyle::Danger, self.sel_clip.is_none());
        children.push(del.id);
        nodes.push(del);

        let row = self.nid(
            "inspector-row",
            UiNodeData::Row(RowNode { children, gap: 8.0, align: Alignment::Center, grow: false }),
        );
        let row_id = row.id;
        nodes.push(row);
        row_id
    }
}

// ── Free helpers ──────────────────────────────────────────────────────────────

fn col(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color { r, g, b, a }
}

/// Darkens a fill for a muted track without changing its hue.
fn dim(c: Color) -> Color {
    Color { r: c.r / 2, g: c.g / 2, b: c.b / 2, a: c.a }
}

fn push_rect(cmds: &mut Vec<CanvasCommand>, x: f32, y: f32, width: f32, height: f32, fill: Color, radius: f32) {
    cmds.push(CanvasCommand::Rect(CanvasRect { x, y, width, height, fill, radius }));
}

fn push_line(cmds: &mut Vec<CanvasCommand>, x1: f32, y1: f32, x2: f32, y2: f32, color: Color, width: f32) {
    cmds.push(CanvasCommand::Line(CanvasLine { x1, y1, x2, y2, width, color }));
}

fn push_text(cmds: &mut Vec<CanvasCommand>, x: f32, y: f32, text: String, size: f32, color: Color, bold: bool) {
    cmds.push(CanvasCommand::Text(CanvasText { x, y, text, size, color, bold, align: Alignment::Start }));
}

/// Short human label for a clip: the source path's trailing segment.
fn source_label(model: &DawModel, source: SourceId) -> String {
    model
        .project()
        .source(source)
        .map(|s| s.path.rsplit([':', '/']).next().unwrap_or(&s.path).to_string())
        .unwrap_or_else(|| format!("src {}", source.0))
}

/// First clip id in the seeded project, by ascending id.
fn first_clip_id(model: &DawModel) -> Option<ClipId> {
    model
        .project()
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .map(|c| c.id)
        .min_by_key(|id| id.0)
}

// ── Lifecycle ─────────────────────────────────────────────────────────────────

impl Guest for Component {
    fn init(_state: StateSnapshot, _size: (f32, f32), _args: Vec<String>) -> Vec<Effect> {
        // Open the stream first: the engine must prepare at the negotiated
        // rate/channels, not the requested ones, or playback pitch drifts.
        let mut config = EngineConfig {
            sample_rate: REQUESTED_SAMPLE_RATE,
            channels: REQUESTED_CHANNELS,
        };
        let stream = match audio_rt_control::open_output(AudioConfig {
            sample_rate: REQUESTED_SAMPLE_RATE,
            channels: REQUESTED_CHANNELS,
            buffer_frames: BUFFER_FRAMES,
            format: AudioFormat::Float32,
        }) {
            Ok(handle) => {
                match audio_rt_control::stream_config(handle) {
                    Ok(negotiated) => {
                        if !(1..=2).contains(&negotiated.channels) {
                            // The engine renders mono/stereo only; the RT
                            // guard below keeps such a stream silent.
                            host_log::error(&format!(
                                "daw-engine-poc: negotiated {} channels unsupported; output muted",
                                negotiated.channels
                            ));
                        }
                        config = EngineConfig {
                            sample_rate: negotiated.sample_rate,
                            channels: negotiated.channels,
                        };
                    }
                    Err(e) => host_log::error(&format!(
                        "daw-engine-poc: stream-config query failed ({e}); assuming requested config"
                    )),
                }
                host_log::info(&format!(
                    "daw-engine-poc: stream {handle} opened at {} Hz / {} ch",
                    config.sample_rate, config.channels
                ));
                Some(handle)
            }
            Err(e) => {
                // Offline mixdown still works without a live stream.
                host_log::error(&format!("daw-engine-poc: open_output failed: {e}"));
                None
            }
        };

        let mut app_state = App::new(EngineConfig {
            sample_rate: config.sample_rate,
            channels: config.channels.clamp(1, 2),
        });
        app_state.stream = stream;
        let mut effects: Vec<Effect> = Vec::new();
        app_state.ensure_source_data(&mut effects);
        app_state.reprepare(app_state.model.transport().position);
        host_log::info(&format!(
            "daw-engine-poc: init tracks={} sources={} end_frame={}",
            app_state.model.project().tracks.len(),
            app_state.model.project().sources.len(),
            app_state.engine.as_ref().map(Engine::project_end_frame).unwrap_or(0)
        ));
        unsafe { APP = Some(app_state) }

        effects.push(Effect::SetTitle("DAW Engine".to_string()));
        effects.push(Effect::DeclareTools(DeclareToolsEffect { tools: App::tool_decls() }));
        effects
    }

    fn update(event: InputEvent) -> Vec<Effect> {
        let mut effects: Vec<Effect> = Vec::new();
        let a = app();
        match event {
            InputEvent::Key(KeyEvent { key, pressed: true, .. }) => a.on_key(&key, &mut effects),
            InputEvent::UiAction(UiActionEvent { handler_id }) => a.on_action(&handler_id, &mut effects),
            InputEvent::Mouse(mouse) => a.on_mouse(mouse, &mut effects),
            InputEvent::FileReadResult(result) => a.finish_read(result, &mut effects),
            InputEvent::FileWriteResult(Ok(())) => {
                a.last_status = "mixdown WAV written".into();
                host_log::info("daw-engine-poc: mixdown WAV written");
            }
            InputEvent::FileWriteResult(Err(e)) => {
                a.last_status = format!("mixdown WAV write failed: {e}");
                host_log::error(&format!("daw-engine-poc: mixdown write failed: {e}"));
            }
            InputEvent::CapabilityGranted(cap) => {
                host_log::info(&format!("daw-engine-poc: capability granted: {cap}"));
                if let Some(root) = cap.strip_prefix("fs:read:") {
                    if !a.granted_roots.contains(&root.to_string()) {
                        a.granted_roots.push(root.to_string());
                    }
                    a.pump_reads(&mut effects);
                } else if let Some(root) = cap.strip_prefix("fs:write:") {
                    // Only the grant for exactly this export's root releases
                    // it; a grant for some other directory must not.
                    if a.pending_write.as_ref().is_some_and(|w| w.root == root) {
                        let w = a.pending_write.take().expect("checked above");
                        host_log::info(&format!("daw-engine-poc: writing mixdown WAV to {}", w.path));
                        effects.push(Effect::FileWrite(FileWriteEffect { path: w.path, content: w.bytes }));
                    }
                }
            }
            InputEvent::CapabilityDenied(cap) => {
                if let Some(root) = cap.strip_prefix("fs:read:") {
                    // Fail only the sources under the denied root; reads for
                    // other roots stay queued for their own grants.
                    let denied: Vec<SourceId> = a
                        .read_queue
                        .iter()
                        .copied()
                        .filter(|id| {
                            a.model
                                .project()
                                .source(*id)
                                .and_then(|s| parent_root(&s.path))
                                .is_some_and(|r| r == root)
                        })
                        .collect();
                    a.read_queue.retain(|id| !denied.contains(id));
                    for id in denied {
                        a.load_states
                            .insert(id, LoadState::Failed(format!("capability denied: {cap}")));
                    }
                    a.last_status = format!("file access denied: {cap}");
                    host_log::warn(&format!("daw-engine-poc: {}", a.last_status));
                } else if let Some(root) = cap.strip_prefix("fs:write:") {
                    if a.pending_write.as_ref().is_some_and(|w| w.root == root) {
                        a.pending_write = None;
                        a.last_status = format!("mixdown write denied: {cap}");
                        host_log::warn(&format!("daw-engine-poc: {}", a.last_status));
                    }
                }
            }
            InputEvent::ToolCall(call) => a.handle_tool_call(call, &mut effects),
            InputEvent::DeclareToolsResult(Ok(names)) => {
                host_log::info(&format!("daw-engine-poc: tools declared: {names:?}"));
            }
            InputEvent::DeclareToolsResult(Err(e)) => {
                host_log::error(&format!("daw-engine-poc: declare-tools failed: {e}"));
            }
            _ => {}
        }
        effects
    }

    fn view() -> UiTree {
        app().build_tree()
    }
}

// ── Real-time audio process ───────────────────────────────────────────────────
//
// Called from the host's OS audio thread. The engine's render_block is the
// same span function the offline mixdown uses; everything it touches was
// allocated at prepare time. The returned Vec is the ABI-required sample
// list — the only allocation on this path.

impl audio_rt_process::Guest for Component {
    fn process_output(
        _handle: u32,
        buffer_frames: u32,
        channels: u32,
        sample_rate: u32,
        state: u64,
    ) -> (Vec<f32>, u64) {
        let mut out = vec![0.0f32; (buffer_frames * channels) as usize];
        let a = unsafe { (*core::ptr::addr_of_mut!(APP)).as_mut() };
        if let Some(engine) = a.and_then(|s| s.engine.as_mut()) {
            // Render only when the callback's layout matches what the engine
            // prepared for (queried from stream-config at init); a mismatch
            // would scramble channel stride or shift pitch.
            let cfg = engine.config();
            if channels == cfg.channels && sample_rate == cfg.sample_rate {
                engine.render_block(&mut out);
            }
        }
        (out, state)
    }

    fn process_input(
        _handle: u32,
        _samples: Vec<f32>,
        _channels: u32,
        _sample_rate: u32,
        state: u64,
    ) -> u64 {
        state
    }
}

export!(Component);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plexi::platform::types::Point;

    fn seeded() -> App {
        let mut app = App::new(EngineConfig { sample_rate: 48_000, channels: 2 });
        let mut fx = Vec::new();
        app.ensure_source_data(&mut fx);
        app
    }

    fn key(app: &mut App, k: &str) {
        let mut fx = Vec::new();
        app.on_key(k, &mut fx);
    }

    fn action(app: &mut App, handler: &str) {
        let mut fx = Vec::new();
        app.on_action(handler, &mut fx);
    }

    #[test]
    fn demo_seed_has_two_tracks_and_five_clips() {
        let app = seeded();
        assert_eq!(app.tracks_len(), 2);
        assert_eq!(app.model.project().clip_count(), 5);
        assert!(app.sel_clip.is_some());
    }

    #[test]
    fn space_toggles_transport() {
        let mut app = seeded();
        assert!(!app.model.transport().playing);
        key(&mut app, "space");
        assert!(app.model.transport().playing);
        key(&mut app, "space");
        assert!(!app.model.transport().playing);
    }

    #[test]
    fn arrows_seek_one_beat() {
        let mut app = seeded();
        key(&mut app, "right");
        assert_eq!(app.model.transport().position, BEAT);
        key(&mut app, "left");
        assert_eq!(app.model.transport().position, 0);
        // Never seeks below zero.
        key(&mut app, "left");
        assert_eq!(app.model.transport().position, 0);
    }

    #[test]
    fn down_up_cycles_track_selection() {
        let mut app = seeded();
        assert_eq!(app.sel_track_idx(), 0);
        key(&mut app, "down");
        assert_eq!(app.sel_track_idx(), 1);
        key(&mut app, "down");
        assert_eq!(app.sel_track_idx(), 0); // wraps
    }

    #[test]
    fn bracket_keys_cycle_clip_selection() {
        let mut app = seeded();
        let clips = app.sel_track_clips_sorted();
        assert_eq!(clips.len(), 3);
        assert_eq!(app.sel_clip, Some(clips[0]));
        key(&mut app, "]");
        assert_eq!(app.sel_clip, Some(clips[1]));
        key(&mut app, "[");
        assert_eq!(app.sel_clip, Some(clips[0]));
    }

    #[test]
    fn move_clip_advances_position_by_one_beat() {
        let mut app = seeded();
        let (_, clip) = app.sel_clip_ref().unwrap();
        let start = clip.position;
        key(&mut app, ".");
        let (_, moved) = app.sel_clip_ref().unwrap();
        assert_eq!(moved.position, start + BEAT);
        key(&mut app, ",");
        let (_, back) = app.sel_clip_ref().unwrap();
        assert_eq!(back.position, start);
    }

    #[test]
    fn trim_end_shrinks_and_grows_within_source() {
        let mut app = seeded();
        // The 2-beat Arp clip has room to shrink and grow a whole beat, so
        // the round trip is exact (a 1-beat clip would floor at the grid).
        key(&mut app, "down");
        let clips = app.sel_track_clips_sorted();
        app.sel_clip = Some(clips[0]);
        let (_, clip) = app.sel_clip_ref().unwrap();
        let start_len = clip.length;
        key(&mut app, ";");
        let (_, shorter) = app.sel_clip_ref().unwrap();
        assert!(shorter.length < start_len);
        key(&mut app, "'");
        let (_, longer) = app.sel_clip_ref().unwrap();
        assert_eq!(longer.length, start_len);
    }

    #[test]
    fn split_at_playhead_makes_two_clips() {
        let mut app = seeded();
        // Select the 2-beat Arp clip on track 2 and split at beat 1.
        key(&mut app, "down");
        let clips = app.sel_track_clips_sorted();
        app.sel_clip = Some(clips[0]);
        let before = app.model.project().clip_count();
        let mut fx = Vec::new();
        app.op_seek_ticks(BEAT, &mut fx);
        key(&mut app, "s");
        assert_eq!(app.model.project().clip_count(), before + 1);
    }

    #[test]
    fn delete_removes_selected_clip() {
        let mut app = seeded();
        let before = app.model.project().clip_count();
        key(&mut app, "x");
        assert_eq!(app.model.project().clip_count(), before - 1);
    }

    #[test]
    fn t_adds_track_and_focuses_it() {
        let mut app = seeded();
        key(&mut app, "t");
        assert_eq!(app.tracks_len(), 3);
        assert_eq!(app.sel_track_idx(), 2);
    }

    #[test]
    fn mute_solo_volume_pan_keys_drive_mixer() {
        let mut app = seeded();
        let track = app.sel_track_id().unwrap();
        assert!(!app.model.project().track(track).unwrap().mixer.mute);
        key(&mut app, "m");
        assert!(app.model.project().track(track).unwrap().mixer.mute);
        key(&mut app, "n");
        assert!(app.model.project().track(track).unwrap().mixer.solo);

        let v0 = app.model.project().track(track).unwrap().mixer.volume;
        key(&mut app, "9");
        assert!(app.model.project().track(track).unwrap().mixer.volume < v0);
        let p0 = app.model.project().track(track).unwrap().mixer.pan;
        key(&mut app, "l");
        assert!(app.model.project().track(track).unwrap().mixer.pan > p0);
    }

    #[test]
    fn tempo_and_loop_keys() {
        let mut app = seeded();
        let t0 = app.model.project().tempo_bpm;
        key(&mut app, "=");
        assert_eq!(app.model.project().tempo_bpm, t0 + BPM_STEP);
        // Loop starts enabled in the demo; toggle off then on.
        assert!(app.model.transport().loop_enabled);
        key(&mut app, "\\");
        assert!(!app.model.transport().loop_enabled);
        key(&mut app, "\\");
        assert!(app.model.transport().loop_enabled);
    }

    #[test]
    fn action_handlers_match_the_keyboard_ops() {
        let mut app = seeded();
        let track = app.sel_track_id().unwrap();
        action(&mut app, &format!("mute:{}", track.0));
        assert!(app.model.project().track(track).unwrap().mixer.mute);
        action(&mut app, "toggle-play");
        assert!(app.model.transport().playing);
        action(&mut app, "add-track");
        assert_eq!(app.tracks_len(), 3);
    }

    #[test]
    fn clip_action_selects_that_clip_and_track() {
        let mut app = seeded();
        // A clip on track 2.
        let arp_clip = app.model.project().tracks[1].clips[0].id;
        action(&mut app, &format!("clip:{}", arp_clip.0));
        assert_eq!(app.sel_clip, Some(arp_clip));
        assert_eq!(app.sel_track_idx(), 1);
    }

    #[test]
    fn pointer_drag_moves_a_clip_through_one_command() {
        let mut app = seeded();
        // First clip on track 1 starts at position 0; its body sits a little
        // right of the gutter at the default zoom.
        let clip = app.model.project().tracks[0].clips[0].id;
        let start = app.model.project().tracks[0].clips[0].position;
        let press_x = app.ticks_to_x(BEAT / 2); // inside the first (1-beat) clip
        let press_y = RULER_H + LANE_H / 2.0;
        let mut fx = Vec::new();
        app.on_mouse(
            MouseEvent { position: Point { x: press_x, y: press_y }, button: Some(MouseButton::Left), pressed: true, scroll_delta: None },
            &mut fx,
        );
        assert!(app.drag.is_some());
        assert_eq!(app.sel_clip, Some(clip));
        let rev_before = app.model.revision();
        // Drag two beats to the right.
        let move_x = app.ticks_to_x(BEAT / 2 + 2 * BEAT);
        app.on_mouse(
            MouseEvent { position: Point { x: move_x, y: press_y }, button: None, pressed: false, scroll_delta: None },
            &mut fx,
        );
        // Preview only — the model has not changed yet.
        assert_eq!(app.model.revision(), rev_before);
        app.on_mouse(
            MouseEvent { position: Point { x: move_x, y: press_y }, button: Some(MouseButton::Left), pressed: false, scroll_delta: None },
            &mut fx,
        );
        assert!(app.drag.is_none());
        let (ti, ci) = app.model.project().find_clip(clip).unwrap();
        assert_eq!(app.model.project().tracks[ti].clips[ci].position, start + 2 * BEAT);
        // Exactly one command (one revision bump) for the whole gesture.
        assert_eq!(app.model.revision(), rev_before + 1);
    }

    #[test]
    fn undo_redo_keys_round_trip_an_edit() {
        let mut app = seeded();
        let start = app.sel_clip_ref().unwrap().1.position;
        key(&mut app, ".");
        assert_ne!(app.sel_clip_ref().unwrap().1.position, start);
        key(&mut app, "z");
        assert_eq!(app.sel_clip_ref().unwrap().1.position, start);
        key(&mut app, "r");
        assert_ne!(app.sel_clip_ref().unwrap().1.position, start);
    }

    #[test]
    fn view_tree_exposes_transport_mixer_and_timeline_nodes() {
        let mut app = seeded();
        let tree = app.build_tree();
        let has_button = |label: &str| {
            tree.nodes.iter().any(|n| matches!(&n.data, UiNodeData::Button(b) if b.label == label))
        };
        assert!(has_button("Play"));
        assert!(has_button("M"));
        assert!(has_button("+ Track"));
        assert!(tree.nodes.iter().any(|n| matches!(&n.data, UiNodeData::Canvas(_))));
        // Transport text is a real Text node so node_changes can see it.
        assert!(tree.nodes.iter().any(|n| matches!(&n.data, UiNodeData::Text(t) if t.text.starts_with("bar "))));
    }
}
