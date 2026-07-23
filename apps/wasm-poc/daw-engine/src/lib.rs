// DAW Engine POC — real-time playback of the plexi-daw-model edit model
// (stint 0515).
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
//   3. The model mutates only through DawCommand: keys and the
//      `daw_command` tool both route through DawModel::apply; the engine
//      re-prepares from the model after every applied edit.
//   4. WAV/MIDI clip sources load outside the RT path via the SDK
//      file-read effect (binary I/O from 0509), gated by fs:read
//      capability requests; `demo:` sources are synthesized in-guest so the
//      POC is audible with zero grants.

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
    Alignment, BadgeColor, BadgeNode, ColumnNode, DeclareToolsEffect, Effect, FileReadEffect,
    FileWriteEffect, IndexedNode, InputEvent, KeyEvent, StateSnapshot, TextNode, ToolCallEvent,
    ToolDecl, ToolResultEffect, UiNodeData, UiTree,
};

use plexi_daw_engine::{midi, pcm_hash, wav, Engine, EngineConfig, Note, SourceData};
use plexi_daw_model::{
    ApplyOutcome, DawCommand, DawModel, SourceId, TrackKind, TICKS_PER_BEAT,
};

/// Requested stream shape; the engine prepares at whatever the host
/// actually negotiates (queried via `stream-config` after open).
const REQUESTED_SAMPLE_RATE: u32 = 48_000;
const REQUESTED_CHANNELS: u32 = 2;
const BUFFER_FRAMES: u32 = 512;

const TOOL_MIXDOWN: &str = "daw_mixdown";
const TOOL_COMMAND: &str = "daw_command";

/// Load state of one model source's media bytes.
enum LoadState {
    Loaded,
    Pending,
    Failed(String),
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
    let cmds = [
        DawCommand::AddTrack { kind: TrackKind::Audio, name: "Pluck".into() },
        DawCommand::AddSource { kind: TrackKind::Audio, path: "demo:pluck".into(), duration: 2 * beat },
        DawCommand::AddClip {
            track: plexi_daw_model::TrackId(1),
            source: SourceId(2),
            position: 0,
            length: beat,
            source_offset: 0,
        },
        DawCommand::AddTrack { kind: TrackKind::Midi, name: "Arp".into() },
        DawCommand::AddSource { kind: TrackKind::Midi, path: "demo:arp".into(), duration: 2 * beat },
        DawCommand::AddClip {
            track: plexi_daw_model::TrackId(4),
            source: SourceId(5),
            position: 0,
            length: 2 * beat,
            source_offset: 0,
        },
        DawCommand::SetLoop { enabled: true, start: 0, end: 2 * beat },
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

    // ── Tools ────────────────────────────────────────────────────────────────

    fn tool_decls() -> Vec<ToolDecl> {
        vec![
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
        ]
    }

    fn handle_tool_call(&mut self, call: ToolCallEvent, effects: &mut Vec<Effect>) {
        let result = match call.name.as_str() {
            TOOL_MIXDOWN => self.tool_mixdown(&call.input_json, effects),
            TOOL_COMMAND => self.tool_command(&call.input_json, effects),
            other => Err(format!("unknown tool {other}")),
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

    fn build_tree(&mut self) -> UiTree {
        self.node_id = 0;
        let mut nodes: Vec<IndexedNode> = Vec::new();
        let mut children: Vec<u32> = Vec::new();

        let title = self.text("title", "DAW Engine".into(), 18.0, true);
        children.push(title.id);
        nodes.push(title);

        let transport = self.model.transport();
        let playing = transport.playing;
        let beat = transport.position / TICKS_PER_BEAT;
        let badge = self.nid(
            "transport",
            UiNodeData::Badge(BadgeNode {
                text: if playing {
                    format!("PLAYING · beat {beat}")
                } else {
                    format!("STOPPED · beat {beat}")
                },
                color: if playing { BadgeColor::Accent } else { BadgeColor::Neutral },
            }),
        );
        children.push(badge.id);
        nodes.push(badge);

        let tempo = self.model.project().tempo_bpm;
        let rev = self.model.revision();
        let info = self.text("info", format!("{tempo:.1} BPM · revision {rev}"), 12.0, false);
        children.push(info.id);
        nodes.push(info);

        let track_lines: Vec<String> = self
            .model
            .project()
            .tracks
            .iter()
            .map(|t| {
                format!(
                    "{} [{:?}] vol {:.2} pan {:+.2}{}{} · {} clip(s)",
                    t.name,
                    t.kind,
                    t.mixer.volume,
                    t.mixer.pan,
                    if t.mixer.mute { " MUTE" } else { "" },
                    if t.mixer.solo { " SOLO" } else { "" },
                    t.clips.len()
                )
            })
            .collect();
        for (i, line) in track_lines.into_iter().enumerate() {
            let node = self.text(&format!("track-{i}"), line, 12.0, false);
            children.push(node.id);
            nodes.push(node);
        }

        let (loaded, pending, failed) = self.load_states.values().fold((0, 0, 0), |acc, s| match s {
            LoadState::Loaded => (acc.0 + 1, acc.1, acc.2),
            LoadState::Pending => (acc.0, acc.1 + 1, acc.2),
            LoadState::Failed(_) => (acc.0, acc.1, acc.2 + 1),
        });
        let first_failure = self.load_states.values().find_map(|s| match s {
            LoadState::Failed(reason) => Some(format!(" · {reason}")),
            _ => None,
        });
        let sources = self.text(
            "sources",
            format!(
                "sources: {loaded} loaded · {pending} pending · {failed} failed{}",
                first_failure.unwrap_or_default()
            ),
            11.0,
            false,
        );
        children.push(sources.id);
        nodes.push(sources);

        let status_line = self.last_status.clone();
        let status = self.text("status", status_line, 11.0, false);
        children.push(status.id);
        nodes.push(status);

        let div = self.nid("div", UiNodeData::Divider);
        children.push(div.id);
        nodes.push(div);

        let hint = self.text(
            "hint",
            "space: play/stop   ←/→: seek 1 beat   q: quit".into(),
            11.0,
            false,
        );
        children.push(hint.id);
        nodes.push(hint);

        let root = self.nid(
            "root",
            UiNodeData::Column(ColumnNode { children, gap: 8.0, align: Alignment::Start, grow: true }),
        );
        let root_id = root.id;
        nodes.push(root);
        UiTree { root: root_id, nodes }
    }
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
            InputEvent::Key(KeyEvent { key, pressed: true, .. }) => match key.as_str() {
                "space" => {
                    let cmd = if a.model.transport().playing { DawCommand::Stop } else { DawCommand::Play };
                    a.apply(cmd, &mut effects);
                }
                "right" => {
                    let pos = a.model.transport().position + TICKS_PER_BEAT;
                    a.apply(DawCommand::Seek { position: pos }, &mut effects);
                }
                "left" => {
                    let pos = a.model.transport().position.saturating_sub(TICKS_PER_BEAT);
                    a.apply(DawCommand::Seek { position: pos }, &mut effects);
                }
                "q" | "escape" => effects.push(Effect::CloseSelf),
                _ => {}
            },
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
