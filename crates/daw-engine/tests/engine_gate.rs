//! Engine gate: offline mixdown determinism as the CI-checkable proxy for
//! the RT path — extends the daw-model gate style (fixture matrix + seeded
//! randomized projects) to rendering. Fixtures render fractions of a second
//! deliberately; buffer sizes stay bounded.

use std::collections::BTreeMap;

use plexi_daw_engine::{pcm_hash, wav, Engine, EngineConfig, Note, SourceData};
use plexi_daw_model::{ApplyOutcome, DawCommand, DawModel, SourceId, TrackKind, TICKS_PER_BEAT};

const CONFIG: EngineConfig = EngineConfig { sample_rate: 44_100, channels: 2 };
const BEAT: u64 = TICKS_PER_BEAT;

/// SplitMix64 — same generator family as the daw-model gate fuzzer.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo)
    }
}

fn apply(model: &mut DawModel, cmd: DawCommand) {
    let outcome = model.apply(cmd.clone());
    assert!(
        matches!(outcome, ApplyOutcome::Applied),
        "fixture command {cmd:?} not applied: {outcome:?}"
    );
}

/// Deterministic quarter-second mono ramp+noise PCM derived from a seed.
fn seeded_pcm(seed: u64, rate: u32) -> SourceData {
    let mut rng = SplitMix64(seed);
    let frames = rate as usize / 4;
    let samples = (0..frames)
        .map(|_| (rng.next() % 2001) as f32 / 1000.0 - 1.0)
        .collect();
    SourceData::AudioPcm { sample_rate: rate, channels: 1, samples }
}

/// Two audio tracks (one seeded-noise, one from decoded WAV bytes) plus a
/// MIDI arp track — the canonical fixture project.
fn fixture() -> (DawModel, BTreeMap<SourceId, SourceData>) {
    let mut model = DawModel::new();
    let mut sources = BTreeMap::new();

    apply(&mut model, DawCommand::AddTrack { kind: TrackKind::Audio, name: "noise".into() });
    apply(&mut model, DawCommand::AddSource { kind: TrackKind::Audio, path: "noise.wav".into(), duration: 2 * BEAT });
    let noise = SourceId(2);
    sources.insert(noise, seeded_pcm(7, 22_050));
    apply(&mut model, DawCommand::AddClip { track: plexi_daw_model::TrackId(1), source: noise, position: 0, length: BEAT, source_offset: 0 });

    // Second audio source arrives as literal WAV bytes through the decoder,
    // exercising the same path the guest uses for SDK-read file bytes.
    apply(&mut model, DawCommand::AddTrack { kind: TrackKind::Audio, name: "tone".into() });
    apply(&mut model, DawCommand::AddSource { kind: TrackKind::Audio, path: "tone.wav".into(), duration: 2 * BEAT });
    let tone = SourceId(5);
    let tone_pcm: Vec<f32> = (0..11_025)
        .map(|i| (i as f32 * core::f32::consts::TAU * 220.0 / 44_100.0).sin() * 0.5)
        .collect();
    let bytes = wav::encode_f32(44_100, 1, &tone_pcm).expect("encode fixture tone");
    let decoded = wav::decode(&bytes).expect("decode fixture tone");
    sources.insert(tone, SourceData::AudioPcm {
        sample_rate: decoded.sample_rate,
        channels: decoded.channels,
        samples: decoded.samples,
    });
    apply(&mut model, DawCommand::AddClip { track: plexi_daw_model::TrackId(4), source: tone, position: BEAT / 2, length: BEAT, source_offset: BEAT / 4 });
    apply(&mut model, DawCommand::SetTrackVolume { track: plexi_daw_model::TrackId(4), volume: 0.8 });
    apply(&mut model, DawCommand::SetTrackPan { track: plexi_daw_model::TrackId(4), pan: -0.5 });

    apply(&mut model, DawCommand::AddTrack { kind: TrackKind::Midi, name: "keys".into() });
    apply(&mut model, DawCommand::AddSource { kind: TrackKind::Midi, path: "arp.mid".into(), duration: 2 * BEAT });
    let arp = SourceId(8);
    sources.insert(arp, SourceData::MidiNotes(vec![
        Note { key: 60, velocity: 100, start_ticks: 0, length_ticks: BEAT / 2 },
        Note { key: 64, velocity: 90, start_ticks: BEAT / 2, length_ticks: BEAT / 2 },
        Note { key: 67, velocity: 80, start_ticks: BEAT, length_ticks: BEAT / 2 },
    ]));
    apply(&mut model, DawCommand::AddClip { track: plexi_daw_model::TrackId(7), source: arp, position: 0, length: 2 * BEAT, source_offset: 0 });

    (model, sources)
}

fn engine_of(model: &DawModel, sources: &BTreeMap<SourceId, SourceData>) -> Engine {
    Engine::prepare(model.project(), model.transport(), sources, CONFIG).expect("prepare")
}

#[test]
fn mixdown_is_deterministic_and_wav_bytes_identical() {
    let (model, sources) = fixture();
    let engine = engine_of(&model, &sources);
    let end = engine.project_end_frame();
    assert!(end > 0);
    let a = engine.mixdown(0, end).unwrap();
    let b = engine_of(&model, &sources).mixdown(0, end).unwrap();
    assert_eq!(a.samples, b.samples);
    let wav_a = wav::encode_f32(a.sample_rate, a.channels, &a.samples).unwrap();
    let wav_b = wav::encode_f32(b.sample_rate, b.channels, &b.samples).unwrap();
    assert_eq!(wav_a, wav_b, "same model must produce byte-identical WAV");
    assert!(a.samples.iter().any(|&s| s != 0.0), "fixture renders audible content");
    assert!(a.samples.iter().all(|s| s.is_finite() && (-1.0..=1.0).contains(s)));
}

#[test]
fn rt_block_path_equals_offline_mixdown() {
    let (mut model, sources) = fixture();
    apply(&mut model, DawCommand::Play);
    let mut engine = engine_of(&model, &sources);
    let end = engine.project_end_frame();
    let offline = engine.mixdown(0, end).unwrap();

    // Drive the RT entry point in odd-sized blocks; concatenation must match
    // the offline render sample-for-sample.
    let ch = CONFIG.channels as usize;
    let mut live: Vec<f32> = Vec::new();
    let mut block = vec![0.0f32; 160 * ch];
    while (live.len() / ch) < end as usize {
        engine.render_block(&mut block);
        live.extend_from_slice(&block);
    }
    live.truncate(offline.samples.len());
    assert_eq!(live, offline.samples, "RT path and offline path must be one render function");
}

#[test]
fn block_size_never_changes_output() {
    let (model, sources) = fixture();
    let engine = engine_of(&model, &sources);
    let end = engine.project_end_frame().min(CONFIG.sample_rate as u64 / 2);
    let ch = CONFIG.channels as usize;
    let whole = engine.mixdown(0, end).unwrap().samples;
    for chunk_frames in [1usize, 64, 480, 1000] {
        let mut chunked = vec![0.0f32; whole.len()];
        let mut frame = 0u64;
        for chunk in chunked.chunks_mut(chunk_frames * ch) {
            engine.render_span(frame, chunk);
            frame += (chunk.len() / ch) as u64;
        }
        assert_eq!(chunked, whole, "chunk size {chunk_frames} changed the render");
    }
}

#[test]
fn stopped_transport_renders_silence_and_holds_playhead() {
    let (model, sources) = fixture();
    let mut engine = engine_of(&model, &sources);
    assert!(!engine.playing());
    let mut block = vec![1.0f32; 256];
    engine.render_block(&mut block);
    assert!(block.iter().all(|&s| s == 0.0));
    assert_eq!(engine.playhead(), 0);
}

#[test]
fn loop_region_wraps_the_rt_path() {
    let (mut model, sources) = fixture();
    apply(&mut model, DawCommand::Play);
    apply(&mut model, DawCommand::SetLoop { enabled: true, start: 0, end: BEAT / 2 });
    let mut engine = engine_of(&model, &sources);
    // Half a beat at the default 120 BPM is exactly a quarter second.
    let loop_frames = (CONFIG.sample_rate as usize) / 4;
    let ch = CONFIG.channels as usize;
    // Render exactly two loop lengths through the RT path.
    let mut two_loops = vec![0.0f32; loop_frames * 2 * ch];
    engine.render_block(&mut two_loops);
    let (first, second) = two_loops.split_at(loop_frames * ch);
    assert_eq!(first, second, "second pass of the loop must repeat the first");
    // The wrap happens on entry to the next block, so the playhead rests at
    // the loop end after finishing a pass.
    assert_eq!(engine.playhead(), loop_frames as u64);
}

#[test]
fn mute_and_solo_gate_tracks() {
    let (mut model, sources) = fixture();
    let end = engine_of(&model, &sources).project_end_frame();
    let full = engine_of(&model, &sources).mixdown(0, end).unwrap().samples;

    apply(&mut model, DawCommand::SetTrackMute { track: plexi_daw_model::TrackId(1), mute: true });
    apply(&mut model, DawCommand::SetTrackMute { track: plexi_daw_model::TrackId(4), mute: true });
    apply(&mut model, DawCommand::SetTrackMute { track: plexi_daw_model::TrackId(7), mute: true });
    let muted = engine_of(&model, &sources).mixdown(0, end).unwrap().samples;
    assert!(muted.iter().all(|&s| s == 0.0), "all tracks muted must render silence");
    assert_ne!(full, muted);

    apply(&mut model, DawCommand::SetTrackMute { track: plexi_daw_model::TrackId(1), mute: false });
    apply(&mut model, DawCommand::SetTrackMute { track: plexi_daw_model::TrackId(4), mute: false });
    apply(&mut model, DawCommand::SetTrackMute { track: plexi_daw_model::TrackId(7), mute: false });
    apply(&mut model, DawCommand::SetTrackSolo { track: plexi_daw_model::TrackId(7), solo: true });
    let solo = engine_of(&model, &sources).mixdown(0, end).unwrap().samples;
    let midi_only = {
        let mut m = model.clone();
        apply(&mut m, DawCommand::SetTrackSolo { track: plexi_daw_model::TrackId(7), solo: false });
        apply(&mut m, DawCommand::SetTrackMute { track: plexi_daw_model::TrackId(1), mute: true });
        apply(&mut m, DawCommand::SetTrackMute { track: plexi_daw_model::TrackId(4), mute: true });
        engine_of(&m, &sources).mixdown(0, end).unwrap().samples
    };
    assert_eq!(solo, midi_only, "solo must equal muting every other track");
}

#[test]
fn hard_left_pan_silences_right_channel() {
    let mut model = DawModel::new();
    let mut sources = BTreeMap::new();
    apply(&mut model, DawCommand::AddTrack { kind: TrackKind::Audio, name: "left".into() });
    apply(&mut model, DawCommand::AddSource { kind: TrackKind::Audio, path: "n.wav".into(), duration: BEAT });
    sources.insert(SourceId(2), seeded_pcm(3, 44_100));
    apply(&mut model, DawCommand::AddClip { track: plexi_daw_model::TrackId(1), source: SourceId(2), position: 0, length: BEAT / 4, source_offset: 0 });
    apply(&mut model, DawCommand::SetTrackPan { track: plexi_daw_model::TrackId(1), pan: -1.0 });
    let engine = engine_of(&model, &sources);
    let out = engine.mixdown(0, engine.project_end_frame()).unwrap().samples;
    let right_energy: f32 = out.iter().skip(1).step_by(2).map(|s| s.abs()).sum();
    let left_energy: f32 = out.iter().step_by(2).map(|s| s.abs()).sum();
    assert!(left_energy > 0.0);
    assert!(right_energy < 1e-3, "hard-left pan leaked {right_energy} into the right channel");
}

#[test]
fn undo_redo_round_trip_renders_identically() {
    let (mut model, sources) = fixture();
    let end = engine_of(&model, &sources).project_end_frame();
    let before = pcm_hash(&engine_of(&model, &sources).mixdown(0, end).unwrap().samples);

    apply(&mut model, DawCommand::SetTrackVolume { track: plexi_daw_model::TrackId(1), volume: 0.2 });
    apply(&mut model, DawCommand::RemoveClip { clip: plexi_daw_model::ClipId(9) });
    let edited = pcm_hash(&engine_of(&model, &sources).mixdown(0, end).unwrap().samples);
    assert_ne!(before, edited);

    apply(&mut model, DawCommand::Undo);
    apply(&mut model, DawCommand::Undo);
    let undone = pcm_hash(&engine_of(&model, &sources).mixdown(0, end).unwrap().samples);
    assert_eq!(before, undone, "undo must restore the exact rendered output");

    apply(&mut model, DawCommand::Redo);
    apply(&mut model, DawCommand::Redo);
    let redone = pcm_hash(&engine_of(&model, &sources).mixdown(0, end).unwrap().samples);
    assert_eq!(edited, redone, "redo must restore the exact edited output");
}

#[test]
fn missing_source_renders_silence_and_is_reported() {
    let (model, mut sources) = fixture();
    sources.remove(&SourceId(2));
    let engine = engine_of(&model, &sources);
    assert_eq!(engine.missing_sources(), &[SourceId(2)]);
    // Still renders (other clips audible), no panic.
    let out = engine.mixdown(0, engine.project_end_frame()).unwrap();
    assert!(out.samples.iter().any(|&s| s != 0.0));
}

#[test]
fn non_finite_source_samples_never_reach_the_output() {
    let mut model = DawModel::new();
    let mut sources = BTreeMap::new();
    apply(&mut model, DawCommand::AddTrack { kind: TrackKind::Audio, name: "hostile".into() });
    apply(&mut model, DawCommand::AddSource { kind: TrackKind::Audio, path: "h.wav".into(), duration: BEAT });
    sources.insert(SourceId(2), SourceData::AudioPcm {
        sample_rate: 44_100,
        channels: 1,
        samples: vec![0.5, f32::NAN, f32::INFINITY, -0.5, f32::NEG_INFINITY, 0.25],
    });
    apply(&mut model, DawCommand::AddClip { track: plexi_daw_model::TrackId(1), source: SourceId(2), position: 0, length: BEAT / 8, source_offset: 0 });
    let engine = engine_of(&model, &sources);
    let out = engine.mixdown(0, engine.project_end_frame()).unwrap().samples;
    assert!(
        out.iter().all(|s| s.is_finite() && (-1.0..=1.0).contains(s)),
        "non-finite source samples leaked into the render"
    );
    assert!(out.iter().any(|&s| s != 0.0), "finite samples still render");
}

#[test]
fn kind_mismatched_source_data_is_rejected() {
    let (model, mut sources) = fixture();
    sources.insert(SourceId(2), SourceData::MidiNotes(vec![]));
    let err = Engine::prepare(model.project(), model.transport(), &sources, CONFIG).unwrap_err();
    assert!(err.contains("other kind"), "{err}");
}

#[test]
fn clip_window_beyond_pcm_data_is_silent_not_panicking() {
    let mut model = DawModel::new();
    let mut sources = BTreeMap::new();
    apply(&mut model, DawCommand::AddTrack { kind: TrackKind::Audio, name: "short".into() });
    // Model says 4 beats, but the PCM is only a quarter second.
    apply(&mut model, DawCommand::AddSource { kind: TrackKind::Audio, path: "s.wav".into(), duration: 4 * BEAT });
    sources.insert(SourceId(2), seeded_pcm(11, 44_100));
    apply(&mut model, DawCommand::AddClip { track: plexi_daw_model::TrackId(1), source: SourceId(2), position: 0, length: 4 * BEAT, source_offset: 0 });
    let engine = engine_of(&model, &sources);
    let out = engine.mixdown(0, engine.project_end_frame()).unwrap();
    let tail_start = out.samples.len() - 100;
    assert!(out.samples[tail_start..].iter().all(|&s| s == 0.0), "beyond PCM must be silence");
}

#[test]
fn mixdown_bounds_are_enforced() {
    let (model, sources) = fixture();
    let engine = engine_of(&model, &sources);
    assert!(engine.mixdown(10, 10).unwrap_err().contains("must be <"));
    assert!(engine
        .mixdown(0, plexi_daw_engine::MAX_MIXDOWN_FRAMES + 1)
        .unwrap_err()
        .contains("MAX_MIXDOWN_FRAMES"));
}

/// Seeded randomized projects: build via commands only, render twice,
/// assert byte-identical output and clamped finite samples. Renders are
/// capped to half a second per seed.
#[test]
fn seeded_random_projects_render_deterministically() {
    for seed in [1u64, 2, 3, 4, 42, 0xDEAD] {
        let mut rng = SplitMix64(seed);
        let mut model = DawModel::new();
        let mut sources: BTreeMap<SourceId, SourceData> = BTreeMap::new();
        let track_count = rng.range(1, 4);
        for _ in 0..track_count {
            let kind = if rng.next().is_multiple_of(2) { TrackKind::Audio } else { TrackKind::Midi };
            apply(&mut model, DawCommand::AddTrack { kind, name: format!("t{}", rng.next() % 100) });
            let track = plexi_daw_model::TrackId(model.project().next_id - 1);
            let duration = rng.range(BEAT / 2, 4 * BEAT);
            apply(&mut model, DawCommand::AddSource { kind, path: format!("s{seed}.dat"), duration });
            let source = SourceId(model.project().next_id - 1);
            match kind {
                TrackKind::Audio => {
                    sources.insert(source, seeded_pcm(rng.next(), 22_050));
                }
                TrackKind::Midi => {
                    let notes = (0..rng.range(1, 5))
                        .map(|_| Note {
                            key: rng.range(40, 90) as u8,
                            velocity: rng.range(20, 128) as u8,
                            start_ticks: rng.range(0, duration),
                            length_ticks: rng.range(1, BEAT),
                        })
                        .collect();
                    sources.insert(source, SourceData::MidiNotes(notes));
                }
            }
            for _ in 0..rng.range(1, 3) {
                let length = rng.range(1, duration + 1);
                let source_offset = rng.range(0, duration - length + 1);
                apply(&mut model, DawCommand::AddClip {
                    track,
                    source,
                    position: rng.range(0, 2 * BEAT),
                    length,
                    source_offset,
                });
            }
            if rng.next().is_multiple_of(3) {
                let volume = (rng.next() % 2000) as f32 / 1000.0;
                apply(&mut model, DawCommand::SetTrackVolume { track, volume });
            }
        }
        let engine = engine_of(&model, &sources);
        let end = engine.project_end_frame().min(CONFIG.sample_rate as u64 / 2);
        if end == 0 {
            continue;
        }
        let a = engine.mixdown(0, end).unwrap().samples;
        let b = engine_of(&model, &sources).mixdown(0, end).unwrap().samples;
        assert_eq!(pcm_hash(&a), pcm_hash(&b), "seed {seed} rendered non-deterministically");
        assert!(
            a.iter().all(|s| s.is_finite() && (-1.0..=1.0).contains(s)),
            "seed {seed} produced out-of-range samples"
        );
    }
}
