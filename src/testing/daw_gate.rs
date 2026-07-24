//! DAW release gate, host side (stint 0519): the same tiers the notes editor
//! gate runs (`src/editor/gate.rs`, `src/testing/harness_tests.rs`), cloned
//! onto the DAW so the whole app is agent-validatable under one command,
//! `cargo test --bin plexi`.
//!
//! The DAW's pure model lives in a sibling crate (`plexi-daw-model`), unlike
//! the in-binary notes editor, so its release gate — the 0514 `GateCase`
//! table plus the seeded command fuzzer — is compiled into this host test
//! build through that crate's `gate` feature (a dev-dependency). The named
//! cases defined once in `plexi_daw_model::gate::gate_cases()` are the single
//! source reused across every tier here:
//!
//! - Tier 1 (fuzz): the pure-model matrix, deterministic long sequence, and
//!   seeded randomized fuzzer, with the same minimizer + replay bundles.
//! - Tier 2 (harness): every named case replayed through the offline render
//!   engine (`plexi-daw-engine`) — the audio runtime an installed host drives
//!   — asserting each produces a valid, deterministic mixdown; plus a real
//!   DAW WASM pane driven through named-key input, asserting its semantic
//!   tree reacts (the keyboard-complete, node-addressable contract).
//! - Determinism: a fixture project → offline mixdown → PCM-hash equality,
//!   block-size independence, and byte-identical `.wav` export.
//!
//! Tiers 3 (TOML scenes, `tests/scenes/daw-*.toml`) and 4 (installed host,
//! `scripts/daw-gate.sh`) drive the same WASM pane out of process.

use std::collections::BTreeMap;

use plexi_daw_engine::{pcm_hash, wav, Engine, EngineConfig, Note, SourceData};
use plexi_daw_model::gate;
use plexi_daw_model::{
    ApplyOutcome, DawCommand, DawModel, Project, Source, SourceId, TrackId, TrackKind,
    TICKS_PER_BEAT,
};

use crate::host::wasm_app::{
    InputEvent, KeyEvent, Modifiers, StateSnapshot, StateStore, UiNodeData, UiTree, WasmApp,
};

// ─── Tier 1: pure-model fuzz, wired under `cargo test --bin plexi` ────────────

/// The 0514 case matrix, replayed here so a model regression fails the host
/// gate command — not only `cargo test -p plexi-daw-model`.
#[test]
fn daw_gate_core_matrix() {
    for case in gate::gate_cases() {
        if let Err(e) = gate::run_case(&case) {
            panic!("daw gate case {:?} failed: {e}", case.name);
        }
    }
}

/// Seeded randomized command fuzzing over the pure model. A failing seed
/// minimizes to a replay bundle and prints `PLEXI_DAW_GATE_SEED=<seed>`; the
/// env var pins the run to one seed for reproduction.
#[test]
fn daw_gate_randomized_commands() {
    for seed in gate::seeds_under_test() {
        if let Err(e) = gate::run_random_seed(seed, gate::RANDOM_COMMANDS_PER_SEED) {
            panic!("{e}");
        }
    }
}

/// The full core qualification (matrix + long sequence + default seeds) with
/// its machine-readable `daw-gate-core.json` artifact — the artifact
/// `scripts/daw-gate.sh` collects as the installed-host gate's Tier 1 record.
#[test]
fn daw_gate_core_qualification_artifact() {
    let out_dir = std::env::temp_dir().join("plexi-daw-gate");
    let summary = gate::run_core_qualification(&out_dir);
    assert_eq!(
        summary.totals.failed,
        0,
        "core qualification reported failures; see {}",
        out_dir.join("daw-gate-core.json").display()
    );
    assert!(out_dir.join("daw-gate-core.json").is_file());
}

// ─── Tier 2a: harness matrix — same cases through the render engine ───────────

const ENGINE_CONFIG: EngineConfig = EngineConfig {
    sample_rate: 48_000,
    channels: 2,
};

/// Deterministic decoded content for one model source, keyed on its kind — a
/// short synthesized buffer for audio, one held note for MIDI. The bytes are
/// fixed, so any nondeterminism a mixdown exposes is the engine's, not the
/// fixture's.
fn source_data_for(source: &Source) -> SourceData {
    match source.kind {
        TrackKind::Audio => {
            let samples: Vec<f32> = (0..ENGINE_CONFIG.sample_rate)
                .map(|i| {
                    let t = i as f32 / ENGINE_CONFIG.sample_rate as f32;
                    (t * 220.0 * std::f32::consts::TAU).sin() * 0.25
                })
                .collect();
            SourceData::AudioPcm {
                sample_rate: ENGINE_CONFIG.sample_rate,
                channels: 1,
                samples,
            }
        }
        TrackKind::Midi => SourceData::MidiNotes(vec![Note {
            key: 69,
            velocity: 100,
            start_ticks: 0,
            length_ticks: source.duration.max(1),
        }]),
    }
}

fn sources_for(project: &Project) -> BTreeMap<SourceId, SourceData> {
    project
        .sources
        .iter()
        .map(|s| (s.id, source_data_for(s)))
        .collect()
}

/// Prepares the engine for `model`'s final state and, when the project is
/// audible, mixes it down twice — asserting byte-for-byte agreement. Returns
/// the mixdown PCM hash for projects that render, `None` otherwise.
fn render_and_hash(model: &DawModel) -> Result<Option<u64>, String> {
    let project = model.project();
    let sources = sources_for(project);
    let engine = Engine::prepare(project, model.transport(), &sources, ENGINE_CONFIG)?;
    let end = engine.project_end_frame();
    if end == 0 {
        return Ok(None);
    }
    let first = engine.mixdown(0, end)?;
    let second = engine.mixdown(0, end)?;
    if first.samples != second.samples {
        return Err("two mixdowns of one prepared engine diverged".to_string());
    }
    Ok(Some(pcm_hash(&first.samples)))
}

/// Every named model case, built through commands only, must prepare and
/// render on the real engine without error and deterministically — the same
/// cases the pure-model tier fuzzes, now carried into the audio runtime.
#[test]
fn daw_gate_engine_matrix() {
    for case in gate::gate_cases() {
        let mut model = DawModel::new();
        for step in &case.steps {
            model.apply(step.command.clone());
        }
        render_and_hash(&model)
            .unwrap_or_else(|e| panic!("engine matrix case {:?}: {e}", case.name));
    }
}

// ─── Tier 2b: real DAW WASM pane driven through named-key input ───────────────

fn daw_fixture() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/daw-engine.wasm")
}

fn named_key(k: &str) -> InputEvent {
    InputEvent::Key(KeyEvent {
        key: k.to_string(),
        modifiers: Modifiers {
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
        },
        pressed: true,
    })
}

/// Every human-readable label in the tree — text, button, and badge nodes —
/// so a substring search matches the same content the out-of-process scene's
/// `tree_contains` finds in the pane's semantic serialization.
fn tree_text(tree: &UiTree) -> String {
    tree.nodes
        .iter()
        .filter_map(|n| match &n.data {
            UiNodeData::Text(t) => Some(t.text.clone()),
            UiNodeData::Button(b) => Some(b.label.clone()),
            UiNodeData::Badge(b) => Some(b.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Drives the committed DAW fixture in-process through the real WASM runtime,
/// asserting the seeded project renders and that each named transport/edit key
/// moves the semantic tree — the keyboard-complete, node-addressable contract
/// the out-of-process scenes and installed-host gate also assert. Only named
/// keys are delivered: the WASM key channel swallows bare printable chars
/// (they arrive as `Event::Text`), exactly as in `tests/scenes/daw-*.toml`.
#[test]
fn daw_gate_pane_drive() -> wasmtime::Result<()> {
    let mut app =
        WasmApp::load_ephemeral_run("daw-gate", &daw_fixture(), StateStore::ephemeral())?;
    app.init(&StateSnapshot { entries: vec![] }, (1280.0, 800.0), &[])?;

    // The seeded demo project renders both tracks, transport, and readout.
    let seeded = tree_text(&app.view()?);
    for expected in ["DAW", "Pluck", "Arp", "Play", "bar 1 · beat 1"] {
        assert!(
            seeded.contains(expected),
            "seeded DAW tree missing {expected:?}; tree=\n{seeded}"
        );
    }

    // Named-key drives, each asserting the tree reacts. These mirror the
    // proven `tests/scenes/daw-timeline.toml` steps against the same fixture.
    for (key, expected) in [
        ("space", "PLAYING"),
        ("right", "beat 2"),
        ("down", "Selected clip"),
        ("delete", "Selected clip"),
        ("home", "bar 1 · beat 1"),
    ] {
        app.update(&named_key(key))?;
        let tree = tree_text(&app.view()?);
        assert!(
            tree.contains(expected),
            "after key {key:?} the DAW tree lacks {expected:?}; tree=\n{tree}"
        );
    }
    Ok(())
}

// ─── Determinism: fixture project → mixdown → hash equality ───────────────────

/// A nontrivial fixture built through commands only: two audio tracks with
/// clips plus one MIDI track, a set tempo, and mixer moves — enough to
/// exercise summing, panning, and MIDI synthesis in one render.
fn determinism_fixture() -> (DawModel, BTreeMap<SourceId, SourceData>) {
    let beat = TICKS_PER_BEAT;
    let mut model = DawModel::new();
    let cmds = [
        DawCommand::SetTempo { bpm: 128.0 },
        DawCommand::AddTrack {
            kind: TrackKind::Audio,
            name: "Kick".into(),
        },
        DawCommand::AddSource {
            kind: TrackKind::Audio,
            path: "samples/kick.wav".into(),
            duration: 2 * beat,
        },
        DawCommand::AddClip {
            track: TrackId(1),
            source: SourceId(2),
            position: 0,
            length: beat,
            source_offset: 0,
        },
        DawCommand::AddClip {
            track: TrackId(1),
            source: SourceId(2),
            position: 2 * beat,
            length: beat,
            source_offset: 0,
        },
        // Ids are allocated monotonically in apply order: Kick=T1, its
        // source=S2, its two clips=C3/C4, so this track is T5 and its
        // source S6.
        DawCommand::AddTrack {
            kind: TrackKind::Midi,
            name: "Lead".into(),
        },
        DawCommand::AddSource {
            kind: TrackKind::Midi,
            path: "midi/lead.mid".into(),
            duration: 4 * beat,
        },
        DawCommand::AddClip {
            track: TrackId(5),
            source: SourceId(6),
            position: 0,
            length: 4 * beat,
            source_offset: 0,
        },
        DawCommand::SetTrackVolume {
            track: TrackId(1),
            volume: 0.8,
        },
        DawCommand::SetTrackPan {
            track: TrackId(5),
            pan: -0.5,
        },
    ];
    for cmd in cmds {
        assert_eq!(
            model.apply(cmd.clone()),
            ApplyOutcome::Applied,
            "determinism fixture command {cmd:?} must apply"
        );
    }
    let sources = sources_for(model.project());
    (model, sources)
}

/// Offline mixdown is deterministic: the same model and decoded sources render
/// to a byte-identical PCM hash across two independently prepared engines.
#[test]
fn daw_gate_mixdown_determinism() {
    let (model, sources) = determinism_fixture();
    let render = || {
        let engine = Engine::prepare(model.project(), model.transport(), &sources, ENGINE_CONFIG)
            .expect("prepare");
        let end = engine.project_end_frame();
        assert!(end > 0, "fixture must produce audible frames");
        engine.mixdown(0, end).expect("mixdown").samples
    };
    let a = render();
    let b = render();
    assert_eq!(
        pcm_hash(&a),
        pcm_hash(&b),
        "independent mixdowns of one fixture must hash identically"
    );
    assert_eq!(a, b, "PCM samples must be byte-identical, not just hash-equal");
}

/// Block-size independence: rendering the whole span at once equals rendering
/// it in arbitrary chunks through `render_span` — the property that lets the
/// RT callback and the offline export share one mixer.
#[test]
fn daw_gate_render_is_block_size_independent() {
    let (model, sources) = determinism_fixture();
    let engine = Engine::prepare(model.project(), model.transport(), &sources, ENGINE_CONFIG)
        .expect("prepare");
    let end = engine.project_end_frame();
    let channels = ENGINE_CONFIG.channels as usize;

    let mut whole = vec![0.0f32; end as usize * channels];
    engine.render_span(0, &mut whole);

    for chunk_frames in [1u64, 17, 512, 4096] {
        let mut chunked = vec![0.0f32; end as usize * channels];
        let mut start = 0u64;
        while start < end {
            let span = chunk_frames.min(end - start);
            let from = start as usize * channels;
            let to = from + span as usize * channels;
            engine.render_span(start, &mut chunked[from..to]);
            start += span;
        }
        assert_eq!(
            whole, chunked,
            "render output changed with chunk size {chunk_frames}"
        );
    }
}

/// WAV export is the mixdown substrate: encoding the same samples twice yields
/// byte-identical files (equal file hash), so an exported `.wav` is
/// reproducible from a project.
#[test]
fn daw_gate_wav_export_is_byte_identical() {
    let (model, sources) = determinism_fixture();
    let engine = Engine::prepare(model.project(), model.transport(), &sources, ENGINE_CONFIG)
        .expect("prepare");
    let end = engine.project_end_frame();
    let mix = engine.mixdown(0, end).expect("mixdown");
    let a = wav::encode_f32(mix.sample_rate, mix.channels, &mix.samples).expect("encode a");
    let b = wav::encode_f32(mix.sample_rate, mix.channels, &mix.samples).expect("encode b");
    assert_eq!(a, b, "WAV export must be byte-identical for identical samples");
    assert_eq!(
        pcm_hash_bytes(&a),
        pcm_hash_bytes(&b),
        "exported .wav file hashes must match"
    );
}

/// FNV-1a over raw bytes — a file fingerprint for the export equality check.
fn pcm_hash_bytes(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
