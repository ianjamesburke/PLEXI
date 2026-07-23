//! DAW audio engine (stint 0515): renders the `plexi-daw-model` edit model
//! to interleaved f32 PCM.
//!
//! One render path serves both consumers. [`Engine::render_span`] is the
//! single mixing/synthesis function — stateless per frame, so output never
//! depends on block size. [`Engine::render_block`] wraps it with real-time
//! transport policy (play gating, loop wrap, playhead advance) for the WASM
//! `process-output` callback; [`Engine::mixdown`] runs it offline for
//! deterministic export and CI assertion. Same model in, byte-identical
//! samples out.
//!
//! The engine derives everything from the model at [`Engine::prepare`] time
//! and holds no mutation path into it — `DawCommand` through
//! `DawModel::apply` stays the only way project state changes. All
//! allocation happens in `prepare`; the render path allocates nothing,
//! takes no locks, and does no I/O (guarded by the `rt_alloc` test).

pub mod engine;
pub mod midi;
pub mod wav;

pub use engine::{
    pcm_hash, Engine, EngineConfig, MixdownResult, Note, SourceData, MAX_MIXDOWN_FRAMES,
};
