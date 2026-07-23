// Jukebox POC (stint 0513) — the first agent-drivable media app and the
// integration proof for sprint s4's risky primitives, all through one small
// real consumer:
//
//   1. File picker (0508): the `o` key opens a multi-select audio pick via
//      `OpenFilePicker`; each picked path arrives as a pane-scoped fs grant.
//   2. Binary file I/O (0509): picked files load through the `file-read`
//      effect and decode in-guest (WAV → interleaved f32).
//   3. Host audio (audio.playback): a real-time `audio-rt-control` output
//      stream; `process-output` pulls the current track's samples from the
//      pure `Jukebox` model on the host RT thread.
//   4. ExposeTools (0517): the `jukebox.*` connector surface lets the
//      assistant list, inspect, and drive transport end to end, with truthful
//      read-only flags so read tools auto-grant and mutations prompt.
//
// A demo playlist is synthesized in-guest at init, so the app is audible and
// listable with zero capability grants — the picker only adds the user's own
// tracks on top.

wit_bindgen::generate!({
    world: "plexi-audio-app",
    path: "wit/world.wit",
});

mod model;
mod tools;

use std::collections::VecDeque;

use exports::plexi::platform::audio_rt_process;
use exports::plexi::platform::lifecycle::Guest;
use plexi::platform::audio_rt_control::{self, AudioConfig, AudioFormat};
use plexi::platform::host_log;
use plexi::platform::types::{
    Alignment, BadgeColor, BadgeNode, ColumnNode, DeclareToolsEffect, Effect, FilePickerMode,
    FileReadEffect, IndexedNode, InputEvent, KeyEvent, OpenFilePickerEffect, StateSnapshot,
    TextNode, TimerEffect, ToolCallEvent, ToolDecl, ToolResultEffect, UiNodeData, UiTree,
};

use model::Jukebox;

const REQUESTED_SAMPLE_RATE: u32 = 48_000;
const REQUESTED_CHANNELS: u32 = 2;
const BUFFER_FRAMES: u32 = 512;

/// Capability that gates the file picker.
const CAP_FS_PICK: &str = "fs.pick";
/// Repaint cadence so the playhead in the view tracks the RT cursor.
const TIMER_REFRESH: u32 = 1;

struct App {
    jb: Jukebox,
    /// Picked tracks awaiting their bytes. `file-read` results carry no
    /// request id, so the head of this queue owns the next `FileReadResult`.
    read_queue: VecDeque<usize>,
    inflight: Option<usize>,
    /// An `o` press requested `fs.pick` and is waiting on the grant to open
    /// the dialog.
    pending_open: bool,
    last_status: String,
    node_id: u32,
}

struct Component;
static mut APP: Option<App> = None;
fn app() -> &'static mut App {
    unsafe { (*core::ptr::addr_of_mut!(APP)).as_mut().unwrap() }
}

// ── Demo content (audible + listable with zero grants) ───────────────────────

/// A short tone track synthesized at the stream layout: `seconds` of a
/// `freq` Hz sine with a gentle decay, interleaved across `channels`.
fn synth_tone(freq: f32, seconds: f32, sample_rate: u32, channels: u32) -> Vec<f32> {
    let frames = (seconds * sample_rate as f32) as usize;
    let channels = channels.max(1) as usize;
    let mut out = Vec::with_capacity(frames * channels);
    for i in 0..frames {
        let t = i as f32 / sample_rate as f32;
        let decay = 1.0 - (i as f32 / frames as f32);
        let sample = (t * core::f32::consts::TAU * freq).sin() * 0.3 * decay;
        for _ in 0..channels {
            out.push(sample);
        }
    }
    out
}

fn seed_demo_playlist(jb: &mut Jukebox) {
    let (rate, channels) = (jb.stream_rate(), jb.stream_channels());
    for (name, freq) in [
        ("Demo: Sine A4", 440.0_f32),
        ("Demo: Sine C5", 523.25),
        ("Demo: Sine E5", 659.25),
    ] {
        jb.add_loaded(name, "demo:sine", synth_tone(freq, 1.5, rate, channels));
    }
}

// ── App impl ─────────────────────────────────────────────────────────────────

/// File-stem display name for a picked path.
fn file_stem_name(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    let base = trimmed.rsplit('/').next().unwrap_or(trimmed);
    let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
    if stem.is_empty() {
        base.to_string()
    } else {
        stem.to_string()
    }
}

impl App {
    fn tool_decls() -> Vec<ToolDecl> {
        tools::tools()
            .into_iter()
            .map(|spec| ToolDecl {
                name: spec.name.to_string(),
                description: spec.description.to_string(),
                input_schema_json: spec.input_schema.to_string(),
                output_schema_json: spec.output_schema.to_string(),
                timeout_ms: Some(2_000),
                read_only: spec.read_only,
            })
            .collect()
    }

    /// Enqueues picked paths as pending tracks and starts loading them.
    fn accept_picked(&mut self, paths: Vec<String>, effects: &mut Vec<Effect>) {
        for path in paths {
            let name = file_stem_name(&path);
            let index = self.jb.add_pending(name, path.clone());
            host_log::info(&format!("jukebox: queued picked track {index} from {path}"));
            self.read_queue.push_back(index);
        }
        self.last_status = format!("added {} track(s) from picker", self.jb.track_count());
        self.pump_reads(effects);
    }

    /// Issues the next queued read; one at a time because `FileReadResult`
    /// carries no request id.
    fn pump_reads(&mut self, effects: &mut Vec<Effect>) {
        if self.inflight.is_some() {
            return;
        }
        while let Some(index) = self.read_queue.pop_front() {
            let Some(path) = self.jb.source_of(index).map(str::to_string) else {
                continue;
            };
            host_log::info(&format!("jukebox: reading track {index} bytes from {path}"));
            self.inflight = Some(index);
            effects.push(Effect::FileRead(FileReadEffect { path }));
            return;
        }
    }

    fn finish_read(&mut self, result: Result<Vec<u8>, String>, effects: &mut Vec<Effect>) {
        let Some(index) = self.inflight.take() else {
            host_log::warn("jukebox: file-read result with no read in flight");
            return;
        };
        match result.and_then(|bytes| plexi_daw_engine::wav::decode(&bytes)) {
            Ok(wav) => {
                self.jb
                    .load_track(index, &wav.samples, wav.sample_rate, wav.channels);
                host_log::info(&format!(
                    "jukebox: loaded track {index} ({} Hz, {} ch, {} frames)",
                    wav.sample_rate,
                    wav.channels,
                    wav.frames()
                ));
                self.last_status = "track loaded".into();
            }
            Err(e) => {
                host_log::error(&format!("jukebox: track {index} load failed: {e}"));
                self.jb.fail_track(index, e);
                self.last_status = "a track failed to load".into();
            }
        }
        self.pump_reads(effects);
    }

    fn handle_tool_call(&mut self, call: ToolCallEvent, effects: &mut Vec<Effect>) {
        let result = match tools::dispatch(&mut self.jb, &call.name, &call.input_json) {
            Some(Ok(value)) => {
                host_log::info(&format!("jukebox: tool {} ok", call.name));
                self.last_status = format!("assistant: {}", call.name);
                serde_json::to_string(&value)
                    .map_err(|e| format!("{} result serialize: {e}", call.name))
            }
            Some(Err(e)) => {
                host_log::info(&format!("jukebox: tool {} error: {e}", call.name));
                Err(e)
            }
            None => Err(format!("unknown tool {}", call.name)),
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

        let title = self.text("title", "Jukebox".into(), 18.0, true);
        children.push(title.id);
        nodes.push(title);

        // Now-playing badge.
        let playing = self.jb.is_playing();
        let current = self.jb.current_index();
        let now = self
            .jb
            .tracks()
            .get(current)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| "—".to_string());
        let badge = self.nid(
            "now",
            UiNodeData::Badge(BadgeNode {
                text: if playing {
                    format!("PLAYING · {now}")
                } else {
                    format!("STOPPED · {now}")
                },
                color: if playing { BadgeColor::Accent } else { BadgeColor::Neutral },
            }),
        );
        children.push(badge.id);
        nodes.push(badge);

        let meta = self.text(
            "meta",
            format!(
                "{} / {} · vol {:.2} · {} Hz {} ch",
                self.jb.position_ms(),
                self.jb.now_playing_value()["duration_ms"].as_u64().unwrap_or(0),
                self.jb.volume(),
                self.jb.stream_rate(),
                self.jb.stream_channels(),
            ),
            12.0,
            false,
        );
        children.push(meta.id);
        nodes.push(meta);

        let div = self.nid("div", UiNodeData::Divider);
        children.push(div.id);
        nodes.push(div);

        // Playlist rows: "> 1. Name [state]" with a marker on the current one.
        let rows: Vec<(String, bool)> = self
            .jb
            .tracks()
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let marker = if i == current { "▶" } else { " " };
                let state = match &t.load {
                    model::Load::Loaded => String::new(),
                    model::Load::Pending => "  (loading…)".to_string(),
                    model::Load::Failed(reason) => format!("  (failed: {reason})"),
                };
                (format!("{marker} {}. {}{state}", i + 1, t.name), i == current)
            })
            .collect();
        if rows.is_empty() {
            let empty = self.text("empty", "No tracks. Press o to add audio files.".into(), 12.0, false);
            children.push(empty.id);
            nodes.push(empty);
        } else {
            for (i, (line, bold)) in rows.into_iter().enumerate() {
                let row = self.text(&format!("track-{i}"), line, 12.0, bold);
                children.push(row.id);
                nodes.push(row);
            }
        }

        let status = self.text("status", self.last_status.clone(), 11.0, false);
        children.push(status.id);
        nodes.push(status);

        let hint = self.text(
            "hint",
            "space: play/pause   n/p: next/prev   ↑/↓: volume   o: open files   q: quit".into(),
            11.0,
            false,
        );
        children.push(hint.id);
        nodes.push(hint);

        let root = self.nid(
            "root",
            UiNodeData::Column(ColumnNode { children, gap: 6.0, align: Alignment::Start, grow: true }),
        );
        let root_id = root.id;
        nodes.push(root);
        UiTree { root: root_id, nodes }
    }
}

// ── Lifecycle ─────────────────────────────────────────────────────────────────

impl Guest for Component {
    fn init(_state: StateSnapshot, _size: (f32, f32), _args: Vec<String>) -> Vec<Effect> {
        // Open the audio stream first so the model is built at the negotiated
        // rate/channels, not the requested ones.
        let (mut rate, mut channels) = (REQUESTED_SAMPLE_RATE, REQUESTED_CHANNELS);
        match audio_rt_control::open_output(AudioConfig {
            sample_rate: REQUESTED_SAMPLE_RATE,
            channels: REQUESTED_CHANNELS,
            buffer_frames: BUFFER_FRAMES,
            format: AudioFormat::Float32,
        }) {
            Ok(handle) => {
                if let Ok(negotiated) = audio_rt_control::stream_config(handle) {
                    rate = negotiated.sample_rate;
                    channels = negotiated.channels.clamp(1, 2);
                }
                host_log::info(&format!(
                    "jukebox: audio stream {handle} opened at {rate} Hz / {channels} ch"
                ));
            }
            Err(e) => host_log::error(&format!("jukebox: open_output failed: {e}")),
        }

        let mut jb = Jukebox::new(rate, channels);
        seed_demo_playlist(&mut jb);
        host_log::info(&format!(
            "jukebox: init with {} demo track(s)",
            jb.track_count()
        ));

        unsafe {
            APP = Some(App {
                jb,
                read_queue: VecDeque::new(),
                inflight: None,
                pending_open: false,
                last_status: "ready · demo playlist loaded".into(),
                node_id: 0,
            });
        }

        vec![
            Effect::SetTitle("Jukebox".to_string()),
            Effect::DeclareTools(DeclareToolsEffect { tools: App::tool_decls() }),
            Effect::SetTimer(TimerEffect { id: TIMER_REFRESH, delay_ms: 200, repeat: true }),
        ]
    }

    fn update(event: InputEvent) -> Vec<Effect> {
        let mut effects: Vec<Effect> = Vec::new();
        let a = app();
        match event {
            InputEvent::TimerFired(TIMER_REFRESH) => {
                // Repaint so the playhead advances in the view; no state change.
            }
            InputEvent::Key(KeyEvent { key, pressed: true, .. }) => {
                // Two-second nudge for seeks, in frames at the stream rate.
                let seek_step = a.jb.stream_rate() as u64 * 2;
                let transport_changed = match key.as_str() {
                    "space" => {
                        a.jb.toggle();
                        true
                    }
                    "n" => {
                        a.jb.next();
                        true
                    }
                    "p" => {
                        a.jb.prev();
                        true
                    }
                    "right" => {
                        a.jb.seek(a.jb.cursor() + seek_step);
                        true
                    }
                    "left" => {
                        a.jb.seek(a.jb.cursor().saturating_sub(seek_step));
                        true
                    }
                    "up" => {
                        a.jb.set_volume(a.jb.volume() + 0.1);
                        true
                    }
                    "down" => {
                        a.jb.set_volume(a.jb.volume() - 0.1);
                        true
                    }
                    "o" => {
                        // Ask for fs.pick; the dialog opens once the grant lands.
                        a.pending_open = true;
                        a.last_status = "requesting file access…".into();
                        host_log::info("jukebox: requesting fs.pick for file open");
                        effects.push(Effect::RequestCapability(CAP_FS_PICK.to_string()));
                        false
                    }
                    "q" | "escape" => {
                        effects.push(Effect::CloseSelf);
                        false
                    }
                    _ => false,
                };
                if transport_changed {
                    host_log::info(&format!("jukebox: key {key} -> {}", a.jb.serialize_state()));
                }
            }
            InputEvent::CapabilityGranted(cap) if cap == CAP_FS_PICK => {
                if a.pending_open {
                    a.pending_open = false;
                    host_log::info("jukebox: fs.pick granted; opening picker");
                    effects.push(Effect::OpenFilePicker(OpenFilePickerEffect {
                        request_id: "jukebox-open".to_string(),
                        filter: vec![
                            "wav".to_string(),
                            "mp3".to_string(),
                            "flac".to_string(),
                            "m4a".to_string(),
                            "ogg".to_string(),
                        ],
                        multiple: true,
                        mode: FilePickerMode::Open,
                    }));
                }
            }
            InputEvent::CapabilityDenied(cap) if cap == CAP_FS_PICK => {
                a.pending_open = false;
                a.last_status = "file access denied".into();
                host_log::warn("jukebox: fs.pick denied");
            }
            InputEvent::FilePicked(picked) => {
                host_log::info(&format!(
                    "jukebox: picker returned {} path(s)",
                    picked.paths.len()
                ));
                a.accept_picked(picked.paths, &mut effects);
            }
            InputEvent::FilePickCancelled(request_id) => {
                a.last_status = "picker cancelled".into();
                host_log::info(&format!("jukebox: pick {request_id} cancelled"));
            }
            InputEvent::FileReadResult(result) => a.finish_read(result, &mut effects),
            InputEvent::ToolCall(call) => a.handle_tool_call(call, &mut effects),
            InputEvent::DeclareToolsResult(Ok(names)) => {
                host_log::info(&format!("jukebox: tools declared: {names:?}"));
            }
            InputEvent::DeclareToolsResult(Err(e)) => {
                host_log::error(&format!("jukebox: declare-tools failed: {e}"));
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
// Called from the host's OS audio thread. `Jukebox::fill_output` writes into
// the prepared buffer only — no allocation beyond the ABI-required return list,
// no host imports.

impl audio_rt_process::Guest for Component {
    fn process_output(
        _handle: u32,
        buffer_frames: u32,
        channels: u32,
        _sample_rate: u32,
        state: u64,
    ) -> (Vec<f32>, u64) {
        let mut out = vec![0.0f32; (buffer_frames * channels) as usize];
        let a = unsafe { (*core::ptr::addr_of_mut!(APP)).as_mut() };
        if let Some(a) = a {
            // Render only when the callback layout matches what the model was
            // built for; a mismatch would scramble channel stride or pitch.
            if channels == a.jb.stream_channels() {
                a.jb.fill_output(&mut out);
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
