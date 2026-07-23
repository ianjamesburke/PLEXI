//! RT-contract proof: the render path performs zero heap allocation.
//!
//! A counting global allocator wraps the system allocator; after `prepare`,
//! any alloc or dealloc during `render_block`/`render_span` fails the test.
//! This is the debug-assertion counter the RT discipline calls for — it
//! lives in its own integration test binary so the counter is not disturbed
//! by unrelated tests.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use plexi_daw_engine::{Engine, EngineConfig, Note, SourceData};
use plexi_daw_model::{ApplyOutcome, DawCommand, DawModel, SourceId, TrackKind, TICKS_PER_BEAT};

static ALLOC_OPS: AtomicUsize = AtomicUsize::new(0);

struct CountingAlloc;

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_OPS.fetch_add(1, Ordering::SeqCst);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        ALLOC_OPS.fetch_add(1, Ordering::SeqCst);
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static ALLOCATOR: CountingAlloc = CountingAlloc;

#[test]
fn render_path_never_allocates() {
    let mut model = DawModel::new();
    let mut sources: BTreeMap<SourceId, SourceData> = BTreeMap::new();
    let beat = TICKS_PER_BEAT;

    let apply = |m: &mut DawModel, cmd: DawCommand| {
        assert!(matches!(m.apply(cmd), ApplyOutcome::Applied));
    };
    apply(&mut model, DawCommand::AddTrack { kind: TrackKind::Audio, name: "a".into() });
    apply(&mut model, DawCommand::AddSource { kind: TrackKind::Audio, path: "a.wav".into(), duration: 2 * beat });
    let pcm: Vec<f32> = (0..8_000).map(|i| ((i % 200) as f32 / 100.0) - 1.0).collect();
    sources.insert(SourceId(2), SourceData::AudioPcm { sample_rate: 16_000, channels: 1, samples: pcm });
    apply(&mut model, DawCommand::AddClip { track: plexi_daw_model::TrackId(1), source: SourceId(2), position: 0, length: 2 * beat, source_offset: 0 });

    apply(&mut model, DawCommand::AddTrack { kind: TrackKind::Midi, name: "m".into() });
    apply(&mut model, DawCommand::AddSource { kind: TrackKind::Midi, path: "m.mid".into(), duration: 2 * beat });
    sources.insert(SourceId(5), SourceData::MidiNotes(vec![
        Note { key: 57, velocity: 100, start_ticks: 0, length_ticks: beat },
        Note { key: 64, velocity: 100, start_ticks: beat / 2, length_ticks: beat },
    ]));
    apply(&mut model, DawCommand::AddClip { track: plexi_daw_model::TrackId(4), source: SourceId(5), position: 0, length: 2 * beat, source_offset: 0 });
    apply(&mut model, DawCommand::SetLoop { enabled: true, start: 0, end: beat });
    apply(&mut model, DawCommand::Play);

    let config = EngineConfig { sample_rate: 48_000, channels: 2 };
    let mut engine = Engine::prepare(model.project(), model.transport(), &sources, config)
        .expect("prepare");
    // The RT callback's buffer is preallocated by the caller.
    let mut block = vec![0.0f32; 512 * config.channels as usize];

    // Warm up once (first sin/log paths), then measure.
    engine.render_block(&mut block);

    let before = ALLOC_OPS.load(Ordering::SeqCst);
    for _ in 0..200 {
        engine.render_block(&mut block);
        engine.render_span(0, &mut block);
    }
    let after = ALLOC_OPS.load(Ordering::SeqCst);
    assert_eq!(
        after - before,
        0,
        "render path performed {} heap operations; the RT contract forbids any",
        after - before
    );
}
