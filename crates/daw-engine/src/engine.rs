//! The render engine: prepared project → interleaved f32 PCM.
//!
//! [`Engine::prepare`] snapshots everything the render path needs from a
//! [`DawModel`]'s project + transport (clip windows in frames, per-clip
//! channel gains with mute/solo folded in, resampled source PCM, MIDI
//! voices) and allocates every buffer up front. After `prepare`, the render
//! path — [`Engine::render_span`] and its wrappers — performs no heap
//! allocation, takes no locks, and calls no I/O.
//!
//! Per-frame synthesis is stateless: an audio clip sample is a direct index
//! into prepared PCM, and a MIDI voice's phase/envelope are pure functions
//! of the absolute frame. Output therefore never depends on how the
//! timeline is chunked into blocks, which is what makes the offline mixdown
//! a byte-exact proxy for the real-time path.

use std::collections::BTreeMap;

use plexi_daw_model::{Project, SourceId, TrackKind, Transport, TICKS_PER_BEAT};

/// Hard ceiling on frames a single [`Engine::mixdown`] call may render
/// (~23 minutes at 48 kHz). Deliberate memory bound: a stereo f32 mixdown at
/// this cap is ~536 MB. Longer exports must chunk through
/// [`Engine::render_span`] directly.
pub const MAX_MIXDOWN_FRAMES: u64 = 1 << 26;

/// Master gain applied to every MIDI voice before velocity scaling, leaving
/// headroom when several voices overlap.
const MIDI_VOICE_GAIN: f32 = 0.35;
const ATTACK_SECS: f64 = 0.005;
const RELEASE_SECS: f64 = 0.010;

/// One parsed MIDI note, in model ticks ([`TICKS_PER_BEAT`] per beat)
/// relative to the start of its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Note {
    /// MIDI key number (69 = A4 = 440 Hz).
    pub key: u8,
    /// 1..=127; scales voice amplitude.
    pub velocity: u8,
    pub start_ticks: u64,
    pub length_ticks: u64,
}

/// Decoded content for one model source, loaded outside the RT path (file
/// bytes via the SDK read effect, or generated). The model's `Source` stays
/// a path + duration; this is the actual media behind it.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceData {
    /// Interleaved f32 PCM at the source's native rate/channel count.
    AudioPcm {
        sample_rate: u32,
        channels: u32,
        samples: Vec<f32>,
    },
    /// Notes synthesized by the engine's built-in oscillator voice.
    MidiNotes(Vec<Note>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineConfig {
    pub sample_rate: u32,
    /// 1 (mono) or 2 (stereo interleaved).
    pub channels: u32,
}

/// Offline render output: the WAV-export substrate and CI assertion payload.
#[derive(Debug, Clone, PartialEq)]
pub struct MixdownResult {
    pub sample_rate: u32,
    pub channels: u32,
    /// Interleaved; `frames * channels` samples, clamped to `[-1.0, 1.0]`.
    pub samples: Vec<f32>,
}

/// Source PCM resampled to the engine rate and channel layout at prepare
/// time, so the RT path is a direct index.
#[derive(Debug)]
struct AudioBuf {
    frames: u64,
    /// Interleaved at the engine channel count.
    samples: Vec<f32>,
}

/// One MIDI note instantiated on the timeline, clipped to its clip window.
#[derive(Debug, Clone, Copy)]
struct Voice {
    /// Absolute timeline frames.
    start: u64,
    end: u64,
    freq: f64,
    amp: f32,
}

#[derive(Debug)]
enum ClipData {
    Audio {
        /// Index into `Engine::audio`.
        buf: usize,
        /// Source-side skip (`source_offset`) in frames.
        skip: u64,
    },
    Midi {
        /// Index range into `Engine::voices`.
        voices: core::ops::Range<usize>,
    },
}

#[derive(Debug)]
struct PreparedClip {
    /// Absolute timeline frames.
    start: u64,
    end: u64,
    /// Track volume, pan, and mute/solo audibility folded into per-channel
    /// gains (`gain_r` unused when the engine is mono).
    gain_l: f32,
    gain_r: f32,
    data: ClipData,
}

/// The prepared render state. Derives entirely from the model at
/// [`prepare`](Engine::prepare) time; it holds no path back into the model —
/// [`plexi_daw_model::DawCommand`] via `DawModel::apply` remains the only
/// mutation surface, and edits reach the engine by re-preparing.
#[derive(Debug)]
pub struct Engine {
    config: EngineConfig,
    playing: bool,
    /// Absolute timeline frame the next rendered block starts at. Engine-
    /// local playback state (advanced by [`render_block`](Self::render_block)),
    /// seeded from the model transport at prepare.
    playhead: u64,
    /// `(start, end)` in frames; present only when the transport loop is
    /// enabled (`start < end` guaranteed by model validation).
    loop_region: Option<(u64, u64)>,
    end_frame: u64,
    audio: Vec<AudioBuf>,
    voices: Vec<Voice>,
    clips: Vec<PreparedClip>,
    missing: Vec<SourceId>,
    attack_frames: f32,
    release_frames: f32,
}

impl Engine {
    /// Builds the prepared render state. All allocation happens here.
    ///
    /// `sources` maps model sources to their decoded content; sources absent
    /// from the map (not yet loaded, dangling path) render silence and are
    /// reported by [`missing_sources`](Self::missing_sources). A kind
    /// mismatch between a model source and its data is an error — that is a
    /// caller bug, not a loading gap.
    pub fn prepare(
        project: &Project,
        transport: &Transport,
        sources: &BTreeMap<SourceId, SourceData>,
        config: EngineConfig,
    ) -> Result<Self, String> {
        if config.sample_rate == 0 {
            return Err("engine prepare: sample rate must be > 0".to_string());
        }
        if !(1..=2).contains(&config.channels) {
            return Err(format!(
                "engine prepare: unsupported channel count {} (1 or 2)",
                config.channels
            ));
        }
        let fpt = frames_per_tick(config.sample_rate, project.tempo_bpm);
        let to_frame = |ticks: u64| tick_to_frame(ticks, fpt);

        // Resample audio sources once; clips index the shared buffer.
        let mut audio: Vec<AudioBuf> = Vec::new();
        let mut audio_index: BTreeMap<SourceId, usize> = BTreeMap::new();
        let mut midi_notes: BTreeMap<SourceId, &Vec<Note>> = BTreeMap::new();
        let mut missing: Vec<SourceId> = Vec::new();
        for source in &project.sources {
            match (source.kind, sources.get(&source.id)) {
                (TrackKind::Audio, Some(SourceData::AudioPcm { sample_rate, channels, samples })) => {
                    let buf = resample(*sample_rate, *channels, samples, &config)
                        .map_err(|e| format!("engine prepare: source {} ({}): {e}", source.id.0, source.path))?;
                    audio_index.insert(source.id, audio.len());
                    audio.push(buf);
                }
                (TrackKind::Midi, Some(SourceData::MidiNotes(notes))) => {
                    midi_notes.insert(source.id, notes);
                }
                (kind, Some(_)) => {
                    return Err(format!(
                        "engine prepare: source {} ({}) is {kind:?} but its data is the other kind",
                        source.id.0, source.path
                    ));
                }
                (_, None) => missing.push(source.id),
            }
        }

        let solo_active = project.tracks.iter().any(|t| t.mixer.solo);
        let mut clips: Vec<PreparedClip> = Vec::new();
        let mut voices: Vec<Voice> = Vec::new();
        let mut end_frame: u64 = 0;
        for track in &project.tracks {
            let audible = !track.mixer.mute && (!solo_active || track.mixer.solo);
            for clip in &track.clips {
                let start = to_frame(clip.position);
                let end = to_frame(clip.position.saturating_add(clip.length));
                end_frame = end_frame.max(end);
                if !audible || end <= start {
                    continue;
                }
                let (gain_l, gain_r) = channel_gains(track.mixer.volume, track.mixer.pan, config.channels);
                let data = match track.kind {
                    TrackKind::Audio => {
                        let Some(&buf) = audio_index.get(&clip.source) else {
                            continue; // missing source: silence
                        };
                        ClipData::Audio { buf, skip: to_frame(clip.source_offset) }
                    }
                    TrackKind::Midi => {
                        let Some(&notes) = midi_notes.get(&clip.source) else {
                            continue;
                        };
                        let first = voices.len();
                        instantiate_voices(notes, clip.source_offset, clip.length, clip.position, fpt, &mut voices);
                        ClipData::Midi { voices: first..voices.len() }
                    }
                };
                clips.push(PreparedClip { start, end, gain_l, gain_r, data });
            }
        }

        Ok(Self {
            config,
            playing: transport.playing,
            playhead: to_frame(transport.position),
            loop_region: (transport.loop_enabled && transport.loop_start < transport.loop_end)
                .then(|| (to_frame(transport.loop_start), to_frame(transport.loop_end)))
                .filter(|(s, e)| s < e),
            end_frame,
            audio,
            voices,
            clips,
            missing,
            attack_frames: (ATTACK_SECS * f64::from(config.sample_rate)).max(1.0) as f32,
            release_frames: (RELEASE_SECS * f64::from(config.sample_rate)).max(1.0) as f32,
        })
    }

    #[must_use]
    pub fn config(&self) -> EngineConfig {
        self.config
    }

    /// Sources present in the model but absent from the prepare-time data
    /// map; their clips render silence until re-prepared with data.
    #[must_use]
    pub fn missing_sources(&self) -> &[SourceId] {
        &self.missing
    }

    /// Last frame any clip reaches — the natural mixdown end.
    #[must_use]
    pub fn project_end_frame(&self) -> u64 {
        self.end_frame
    }

    #[must_use]
    pub fn playing(&self) -> bool {
        self.playing
    }

    #[must_use]
    pub fn playhead(&self) -> u64 {
        self.playhead
    }

    /// Restores engine-local playback position — used by callers that
    /// re-prepare mid-playback (an edit landed) and want playback to
    /// continue where it was rather than jump to the model transport tick.
    pub fn set_playhead(&mut self, frame: u64) {
        self.playhead = frame;
    }

    /// THE render function. Mixes every audible clip overlapping
    /// `[start_frame, start_frame + out.len() / channels)` into `out`
    /// (interleaved, zeroed first, clamped to `[-1.0, 1.0]`). Stateless:
    /// same span in, same samples out, regardless of chunking. RT-safe: no
    /// allocation, no locks, no I/O.
    pub fn render_span(&self, start_frame: u64, out: &mut [f32]) {
        out.fill(0.0);
        let ch = self.config.channels as usize;
        let frames = (out.len() / ch) as u64;
        let block_end = start_frame.saturating_add(frames);
        let sr = f64::from(self.config.sample_rate);

        for clip in &self.clips {
            let lo = clip.start.max(start_frame);
            let hi = clip.end.min(block_end);
            if lo >= hi {
                continue;
            }
            match &clip.data {
                ClipData::Audio { buf, skip } => {
                    let buf = &self.audio[*buf];
                    for gf in lo..hi {
                        let src = skip.saturating_add(gf - clip.start);
                        if src >= buf.frames {
                            break;
                        }
                        let si = (src as usize) * ch;
                        let oi = ((gf - start_frame) as usize) * ch;
                        out[oi] += buf.samples[si] * clip.gain_l;
                        if ch == 2 {
                            out[oi + 1] += buf.samples[si + 1] * clip.gain_r;
                        }
                    }
                }
                ClipData::Midi { voices } => {
                    for v in &self.voices[voices.clone()] {
                        let vlo = lo.max(v.start);
                        let vhi = hi.min(v.end);
                        if vlo >= vhi {
                            continue;
                        }
                        let len = (v.end - v.start) as f32;
                        for gf in vlo..vhi {
                            let rel = gf - v.start;
                            let phase = (rel as f64 * v.freq / sr).fract() as f32;
                            let env = envelope(rel as f32, len, self.attack_frames, self.release_frames);
                            let s = oscillator(phase) * env * v.amp;
                            let oi = ((gf - start_frame) as usize) * ch;
                            out[oi] += s * clip.gain_l;
                            if ch == 2 {
                                out[oi + 1] += s * clip.gain_r;
                            }
                        }
                    }
                }
            }
        }
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }

    /// Real-time policy over [`render_span`](Self::render_span): silence
    /// while the transport is stopped, loop wrap at the loop end, playhead
    /// advance. This is what the WASM guest's `process-output` calls.
    /// RT-safe like the span itself.
    pub fn render_block(&mut self, out: &mut [f32]) {
        if !self.playing {
            out.fill(0.0);
            return;
        }
        let ch = self.config.channels as usize;
        let frames = out.len() / ch;
        let mut done = 0usize;
        while done < frames {
            if let Some((ls, le)) = self.loop_region {
                if self.playhead >= le {
                    self.playhead = ls;
                }
            }
            let remaining = frames - done;
            let span = match self.loop_region {
                Some((_, le)) if self.playhead < le => {
                    remaining.min((le - self.playhead) as usize)
                }
                _ => remaining,
            };
            self.render_span(self.playhead, &mut out[done * ch..(done + span) * ch]);
            self.playhead = self.playhead.saturating_add(span as u64);
            done += span;
        }
    }

    /// Deterministic offline render of `[start_frame, end_frame)` through
    /// the same span function the RT path uses — the WAV-export substrate
    /// and the CI assertion mechanism. Ignores transport play/loop state.
    pub fn mixdown(&self, start_frame: u64, end_frame: u64) -> Result<MixdownResult, String> {
        if start_frame >= end_frame {
            return Err(format!(
                "mixdown: start frame {start_frame} must be < end frame {end_frame}"
            ));
        }
        let frames = end_frame - start_frame;
        if frames > MAX_MIXDOWN_FRAMES {
            return Err(format!(
                "mixdown: {frames} frames exceeds MAX_MIXDOWN_FRAMES {MAX_MIXDOWN_FRAMES}; \
                 chunk longer renders through render_span"
            ));
        }
        let mut samples = vec![0.0f32; (frames as usize) * self.config.channels as usize];
        self.render_span(start_frame, &mut samples);
        log::info!(
            "daw_engine: mixdown rendered frames={frames} rate={} channels={} hash={:016x}",
            self.config.sample_rate,
            self.config.channels,
            pcm_hash(&samples)
        );
        Ok(MixdownResult {
            sample_rate: self.config.sample_rate,
            channels: self.config.channels,
            samples,
        })
    }
}

/// FNV-1a 64 over the little-endian sample bytes — the fingerprint mixdown
/// determinism tests and the guest's mixdown tool report.
#[must_use]
pub fn pcm_hash(samples: &[f32]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for s in samples {
        for b in s.to_le_bytes() {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

fn frames_per_tick(sample_rate: u32, tempo_bpm: f64) -> f64 {
    f64::from(sample_rate) * 60.0 / (tempo_bpm * TICKS_PER_BEAT as f64)
}

fn tick_to_frame(ticks: u64, fpt: f64) -> u64 {
    let f = (ticks as f64 * fpt).round();
    if f >= u64::MAX as f64 {
        u64::MAX
    } else {
        f as u64
    }
}

/// Folds volume + constant-power pan into per-channel gains. Mono engines
/// ignore pan.
fn channel_gains(volume: f32, pan: f32, channels: u32) -> (f32, f32) {
    if channels == 1 {
        return (volume, volume);
    }
    let angle = (pan + 1.0) * core::f32::consts::FRAC_PI_4;
    (volume * angle.cos(), volume * angle.sin())
}

/// Instantiates the notes of one MIDI clip window onto the timeline.
/// The window `[source_offset, source_offset + length)` selects and clips
/// notes; window ticks map to timeline ticks at the clip position.
fn instantiate_voices(
    notes: &[Note],
    source_offset: u64,
    length: u64,
    clip_position: u64,
    fpt: f64,
    voices: &mut Vec<Voice>,
) {
    let window_end = source_offset.saturating_add(length);
    for note in notes {
        if note.velocity == 0 || note.length_ticks == 0 {
            continue;
        }
        let note_end = note.start_ticks.saturating_add(note.length_ticks);
        let lo = note.start_ticks.max(source_offset);
        let hi = note_end.min(window_end);
        if lo >= hi {
            continue;
        }
        let timeline_start = clip_position.saturating_add(lo - source_offset);
        let timeline_end = clip_position.saturating_add(hi - source_offset);
        let start = tick_to_frame(timeline_start, fpt);
        let end = tick_to_frame(timeline_end, fpt);
        if end <= start {
            continue;
        }
        voices.push(Voice {
            start,
            end,
            freq: 440.0 * f64::powf(2.0, (f64::from(note.key) - 69.0) / 12.0),
            amp: MIDI_VOICE_GAIN * f32::from(note.velocity) / 127.0,
        });
    }
}

/// Additive oscillator voice: fundamental + two harmonics, matching the
/// audio-synth POC pattern. `phase` in `[0, 1)`.
fn oscillator(phase: f32) -> f32 {
    let t = phase * core::f32::consts::TAU;
    (t.sin() + 0.25 * (2.0 * t).sin() + 0.125 * (3.0 * t).sin()) / 1.375
}

/// Linear attack/release envelope, a pure function of the note-relative
/// frame — stateless so chunking never changes the output.
fn envelope(rel: f32, len: f32, attack: f32, release: f32) -> f32 {
    let a = (rel / attack).min(1.0);
    let r = ((len - rel) / release).min(1.0);
    a.min(r).clamp(0.0, 1.0)
}

/// Converts source PCM to the engine rate and channel layout (linear
/// interpolation; mono↔stereo up/down-mix). All allocation for a source
/// happens here, at prepare time.
fn resample(
    src_rate: u32,
    src_channels: u32,
    samples: &[f32],
    config: &EngineConfig,
) -> Result<AudioBuf, String> {
    if src_rate == 0 {
        return Err("source sample rate is zero".to_string());
    }
    if !(1..=2).contains(&src_channels) {
        return Err(format!("unsupported source channel count {src_channels} (1 or 2)"));
    }
    if !samples.len().is_multiple_of(src_channels as usize) {
        return Err(format!(
            "source sample count {} is not a multiple of {src_channels} channels",
            samples.len()
        ));
    }
    let src_frames = (samples.len() / src_channels as usize) as u64;
    let dst_frames = if src_rate == config.sample_rate {
        src_frames
    } else {
        (src_frames as f64 * f64::from(config.sample_rate) / f64::from(src_rate)).floor() as u64
    };
    let sch = src_channels as usize;
    let dch = config.channels as usize;
    let mut out = vec![0.0f32; (dst_frames as usize) * dch];
    let ratio = f64::from(src_rate) / f64::from(config.sample_rate);
    for df in 0..dst_frames as usize {
        let pos = df as f64 * ratio;
        let i0 = pos.floor() as usize;
        let frac = (pos - pos.floor()) as f32;
        let i1 = (i0 + 1).min(src_frames.saturating_sub(1) as usize);
        let mut frame = [0.0f32; 2];
        for (c, slot) in frame.iter_mut().enumerate().take(sch) {
            let a = samples[i0 * sch + c];
            let b = samples[i1 * sch + c];
            *slot = a + (b - a) * frac;
        }
        // Sanitize here so the render path's output contract (finite,
        // clamped) holds even for hostile float32 WAV input — clamp alone
        // would pass NaN through.
        for slot in frame.iter_mut() {
            if !slot.is_finite() {
                *slot = 0.0;
            }
        }
        match (sch, dch) {
            (1, 1) => out[df] = frame[0],
            (1, 2) => {
                out[df * 2] = frame[0];
                out[df * 2 + 1] = frame[0];
            }
            (2, 2) => {
                out[df * 2] = frame[0];
                out[df * 2 + 1] = frame[1];
            }
            (2, 1) => out[df] = (frame[0] + frame[1]) * 0.5,
            _ => unreachable!("channel counts validated above"),
        }
    }
    Ok(AudioBuf { frames: dst_frames, samples: out })
}
