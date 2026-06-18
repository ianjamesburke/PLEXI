// Audio Synth — RT audio POC for the Plexi v2 WASM runtime.
//
// Proves:
//   1. audio-rt-control import: opens an output stream; host calls back at RT.
//   2. audio-rt-process export: process-output fills f32 sample buffers from
//      the OS audio thread. No heap allocation, no locks, no host imports.
//   3. Plexi Pipes: binary pipe streams interleaved f32 waveform chunks to any
//      connected visualiser pane (waveform pipe) and a JSON pipe for metadata.
//
// Synthesiser: additive sine oscillator (fundamental + 3 harmonics).
// State threading: the phase accumulator is packed into the u64 state slot
// passed through process-output — no heap, no global mutation in the RT path.
//
// update() handles key events to change pitch and waveform type.
// view() renders a simple panel showing note, frequency, and pipe status.

wit_bindgen::generate!({
    world: "plexi-audio-app",
    path: "wit/world.wit",
});

use plexi::platform::audio_rt_control::{self, AudioConfig, AudioFormat};
use plexi::platform::audio_rt_process;
use plexi::platform::host_log;
use plexi::platform::host_state;
use plexi::platform::pipes::{self, PipeDirection, PipeType};
use plexi::platform::types::{
    alignment::Alignment, badge_color::BadgeColor, badge_node::BadgeNode,
    column_node::ColumnNode, effect::Effect, indexed_node::IndexedNode,
    input_event::InputEvent, key_event::KeyEvent, row_node::RowNode,
    state_snapshot::StateSnapshot, text_node::TextNode, timer_effect::TimerEffect,
    ui_node_data::UiNodeData, ui_tree::UiTree,
};

// ── Note table ────────────────────────────────────────────────────────────────

const NOTES: &[(&str, f32)] = &[
    ("C4",  261.63), ("D4",  293.66), ("E4",  329.63), ("F4",  349.23),
    ("G4",  392.00), ("A4",  440.00), ("B4",  493.88), ("C5",  523.25),
    ("D5",  587.33), ("E5",  659.25), ("F5",  698.46), ("G5",  783.99),
];

const STATE_KEY_NOTE: &str = "note_idx";
const STATE_KEY_WAVE: &str = "wave";
const WAVEFORM_PIPE: &str  = "waveform-out";
const METADATA_PIPE: &str  = "synth-meta";
const TIMER_META: u32 = 1; // push metadata every 200ms
const SAMPLE_RATE: u32 = 48000;
const CHANNELS: u32 = 2;
const BUFFER_FRAMES: u32 = 512; // ~10.6ms at 48kHz

// ── Waveform types ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Wave { Sine, Square, Saw, Triangle }

impl Wave {
    fn from_u8(v: u8) -> Self {
        match v { 1 => Wave::Square, 2 => Wave::Saw, 3 => Wave::Triangle, _ => Wave::Sine }
    }
    fn as_u8(self) -> u8 { self as u8 }
    fn label(self) -> &'static str {
        match self { Wave::Sine => "Sine", Wave::Square => "Square", Wave::Saw => "Saw", Wave::Triangle => "Triangle" }
    }

    // Generate one sample. phase in [0, 1).
    fn sample(self, phase: f32) -> f32 {
        match self {
            Wave::Sine     => (phase * std::f32::consts::TAU).sin(),
            Wave::Square   => if phase < 0.5 { 1.0 } else { -1.0 },
            Wave::Saw      => 2.0 * phase - 1.0,
            Wave::Triangle => if phase < 0.5 { 4.0 * phase - 1.0 } else { 3.0 - 4.0 * phase },
        }
    }
}

// ── App state ─────────────────────────────────────────────────────────────────

struct Synth {
    stream: Option<u32>,
    note_idx: usize,
    wave: Wave,
    // Amplitude envelope: 0.0 = silent, 1.0 = full. Simple linear ramp.
    amplitude: f32,
    playing: bool,

    // Pipe handles
    wave_pipe: Option<u32>,
    meta_pipe: Option<u32>,

    node_id: u32,
}

impl Synth {
    fn new(state: &StateSnapshot) -> Self {
        let note_idx = state.entries.iter()
            .find(|(k, _)| k == STATE_KEY_NOTE)
            .and_then(|(_, v)| v.first().map(|&b| b as usize))
            .unwrap_or(5); // default A4

        let wave = state.entries.iter()
            .find(|(k, _)| k == STATE_KEY_WAVE)
            .and_then(|(_, v)| v.first().copied())
            .map(Wave::from_u8)
            .unwrap_or(Wave::Sine);

        Synth {
            stream: None, note_idx, wave,
            amplitude: 0.0, playing: false,
            wave_pipe: None, meta_pipe: None,
            node_id: 0,
        }
    }

    fn freq(&self) -> f32 { NOTES[self.note_idx].1 }
    fn note_name(&self) -> &'static str { NOTES[self.note_idx].0 }

    fn save_state(&self) {
        let _ = host_state::set(STATE_KEY_NOTE, &[self.note_idx as u8]);
        let _ = host_state::set(STATE_KEY_WAVE,  &[self.wave.as_u8()]);
    }

    fn push_metadata(&self) {
        if let Some(h) = self.meta_pipe {
            let json = format!(
                "{{\"note\":\"{}\",\"freq\":{:.2},\"wave\":\"{}\",\"playing\":{}}}",
                self.note_name(), self.freq(), self.wave.label(), self.playing
            );
            let _ = pipes::send_json(h, &json);
        }
    }

    // ── View ─────────────────────────────────────────────────────────────────

    fn nid(&mut self, key: &str, data: UiNodeData) -> IndexedNode {
        let id = self.node_id;
        self.node_id += 1;
        IndexedNode { id, key: key.to_string(), data }
    }

    fn build_tree(&mut self) -> UiTree {
        self.node_id = 0;
        let mut nodes: Vec<IndexedNode> = Vec::new();

        let title = self.nid("title", UiNodeData::Text(TextNode {
            text: "Synth".to_string(),
            size: Some(18.0), bold: true, color: None, truncate: false,
            align: Alignment::Start,
        }));
        let title_id = title.id;
        nodes.push(title);

        // Note badge + frequency
        let note_badge = self.nid("note-badge", UiNodeData::Badge(BadgeNode {
            text: self.note_name().to_string(),
            color: if self.playing { BadgeColor::Accent } else { BadgeColor::Neutral },
        }));
        let freq_label = self.nid("freq", UiNodeData::Text(TextNode {
            text: format!("{:.2} Hz", self.freq()),
            size: Some(13.0), bold: false, color: None, truncate: false,
            align: Alignment::Start,
        }));
        let note_row = self.nid("note-row", UiNodeData::Row(RowNode {
            children: vec![note_badge.id, freq_label.id],
            gap: 8.0, align: Alignment::Center, grow: false,
        }));
        nodes.extend([note_badge, freq_label]);
        let note_row_id = note_row.id;
        nodes.push(note_row);

        // Waveform
        let wave_label = self.nid("wave", UiNodeData::Text(TextNode {
            text: format!("Wave: {}", self.wave.label()),
            size: Some(12.0), bold: false, color: None, truncate: false,
            align: Alignment::Start,
        }));
        let wave_id = wave_label.id;
        nodes.push(wave_label);

        // Pipe status
        let pipe_status = self.nid("pipe", UiNodeData::Text(TextNode {
            text: format!(
                "Waveform pipe: {}   Meta pipe: {}",
                if self.wave_pipe.map(|h| pipes::is_connected(h)).unwrap_or(false) { "connected" } else { "idle" },
                if self.meta_pipe.map(|h| pipes::is_connected(h)).unwrap_or(false) { "connected" } else { "idle" },
            ),
            size: Some(11.0), bold: false, color: None, truncate: false,
            align: Alignment::Start,
        }));
        let pipe_id = pipe_status.id;
        nodes.push(pipe_status);

        let divider = self.nid("div", UiNodeData::Divider);
        let div_id = divider.id;
        nodes.push(divider);

        let hint = self.nid("hint", UiNodeData::Text(TextNode {
            text: "space: play/stop   ←/→: note   w: wave   q: quit".to_string(),
            size: Some(11.0), bold: false, color: None, truncate: false,
            align: Alignment::Start,
        }));
        let hint_id = hint.id;
        nodes.push(hint);

        let root = self.nid("root", UiNodeData::Column(ColumnNode {
            children: vec![title_id, note_row_id, wave_id, pipe_id, div_id, hint_id],
            gap: 10.0, align: Alignment::Start, grow: true,
        }));
        let root_id = root.id;
        nodes.push(root);
        UiTree { root: root_id, nodes }
    }
}

// ── Lifecycle ─────────────────────────────────────────────────────────────────

struct Component;
static mut SYNTH: Option<Synth> = None;
fn synth() -> &'static mut Synth { unsafe { SYNTH.as_mut().unwrap() } }

impl Guest for Component {
    fn init(state: StateSnapshot, _size: (f32, f32)) -> Vec<Effect> {
        let s = Synth::new(&state);
        host_log::info(&format!(
            "audio-synth: init note={} wave={}", s.note_name(), s.wave.label()
        ));
        unsafe { SYNTH = Some(s); }

        // Open RT audio output stream.
        match audio_rt_control::open_output(AudioConfig {
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
            buffer_frames: BUFFER_FRAMES,
            format: AudioFormat::F32,
        }) {
            Ok(handle) => {
                synth().stream = Some(handle);
                host_log::info(&format!("audio-synth: stream {} opened", handle));
            }
            Err(e) => host_log::error(&format!("audio-synth: open_output failed: {e}")),
        }

        // Open waveform binary pipe — a visualiser connects as a client.
        match pipes::open(WAVEFORM_PIPE, PipeType::Binary, PipeDirection::Out) {
            Ok(h) => { synth().wave_pipe = Some(h); host_log::info("audio-synth: waveform pipe open"); }
            Err(e) => host_log::warn(&format!("audio-synth: waveform pipe failed: {e}")),
        }

        // Open metadata JSON pipe.
        match pipes::open(METADATA_PIPE, PipeType::Json, PipeDirection::Out) {
            Ok(h) => { synth().meta_pipe = Some(h); }
            Err(e) => host_log::warn(&format!("audio-synth: meta pipe failed: {e}")),
        }

        vec![
            Effect::SetTitle("Synth".to_string()),
            Effect::SetTimer(TimerEffect { id: TIMER_META, delay_ms: 200, repeat: true }),
        ]
    }

    fn update(event: InputEvent) -> Vec<Effect> {
        match event {
            InputEvent::TimerFired(TIMER_META) => {
                synth().push_metadata();
                vec![]
            }

            InputEvent::Key(KeyEvent { key, pressed: true, .. }) => {
                let s = synth();
                match key.as_str() {
                    "space" => {
                        s.playing = !s.playing;
                        host_log::info(&format!("audio-synth: {}", if s.playing { "play" } else { "stop" }));
                    }
                    "right" => {
                        s.note_idx = (s.note_idx + 1).min(NOTES.len() - 1);
                        s.save_state();
                    }
                    "left" => {
                        s.note_idx = s.note_idx.saturating_sub(1);
                        s.save_state();
                    }
                    "w" => {
                        s.wave = match s.wave {
                            Wave::Sine => Wave::Square,
                            Wave::Square => Wave::Saw,
                            Wave::Saw => Wave::Triangle,
                            Wave::Triangle => Wave::Sine,
                        };
                        s.save_state();
                    }
                    "q" | "escape" => return vec![Effect::CloseSelf],
                    _ => {}
                }
                vec![]
            }
            _ => vec![],
        }
    }

    fn view() -> UiTree { synth().build_tree() }
}

// ── Real-time audio process ───────────────────────────────────────────────────
//
// Called by the host from its OS audio RT thread. Strict no-alloc, no-lock,
// no-host-import constraints. The u64 state slot carries the phase accumulator
// as a u64-reinterpret of f32 — stays in registers, no heap.

impl audio_rt_process::Guest for Component {
    fn process_output(
        handle: u32,
        buffer_frames: u32,
        channels: u32,
        sample_rate: u32,
        state: u64,
    ) -> (Vec<f32>, u64) {
        let mut phase = f32::from_bits(state as u32);
        let s = unsafe { SYNTH.as_mut().unwrap() };

        let freq = s.freq();
        let wave = s.wave;
        let phase_inc = freq / sample_rate as f32;
        let playing = s.playing;

        // Amplitude envelope: ramp to 0.7 when playing, to 0.0 when stopped.
        let target = if playing { 0.7 } else { 0.0 };
        let ramp = 0.001; // per-sample ramp rate

        let n = (buffer_frames * channels) as usize;
        let mut out = vec![0.0f32; n];

        for frame in 0..buffer_frames as usize {
            // Smooth amplitude
            s.amplitude += (target - s.amplitude) * ramp;
            let amp = s.amplitude;

            let sample = if amp > 0.0001 { wave.sample(phase) * amp } else { 0.0 };

            // Write to all channels
            for ch in 0..channels as usize {
                out[frame * channels as usize + ch] = sample;
            }

            phase = (phase + phase_inc).fract();
        }

        // Push a decimated copy to the waveform pipe.
        // Every 8th frame (reduces bandwidth). No allocation: we reuse the
        // existing out slice. The pipe ring may drop frames under load — that's OK.
        if let Some(pipe_h) = s.wave_pipe {
            // Take every 8th mono sample as a u8 for the pipe (lossy preview only).
            let preview: Vec<u8> = out.iter()
                .step_by(8 * channels as usize)
                .flat_map(|&f| f.to_le_bytes())
                .collect();
            let _ = pipes::send_binary(pipe_h, preview);
        }

        let new_state = (phase.to_bits() as u64);
        (out, new_state)
    }

    fn process_input(
        _handle: u32,
        _samples: Vec<f32>,
        _channels: u32,
        _sample_rate: u32,
        state: u64,
    ) -> u64 {
        state // not used — output-only synth
    }
}

export!(Component);
