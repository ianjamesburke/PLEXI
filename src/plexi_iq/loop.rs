//! Turn loop driver — stream → collect tool_use → dispatch → reply.
//!
//! Stage 0: stub only. The real implementation lands in Stage 1; see
//! `docs/specs/plexi-iq.md` §3.2 for the target pseudocode and §3.6 for
//! the `supports_tool_dispatch()` branch that decides whether this loop
//! owns tool dispatch or treats the backend call as opaque prompting.
//!
//! NOTE: the file is named `loop.rs` to match the spec, but `loop` is a
//! Rust keyword, so `mod.rs` mounts it as `pub mod turn_loop` via
//! `#[path = "loop.rs"]`.

use crate::plexi_iq::backend::LlmBackend;
use crate::plexi_iq::context::ToolContext;

/// Run a single user turn through the agent loop. Stage 1 will implement
/// the streaming/tool-dispatch logic; for now this is a `todo!()` so the
/// trait surface and call sites can be sketched without behavior.
pub async fn run_turn<B: LlmBackend + ?Sized>(
    _backend: &B,
    _ctx: &ToolContext,
    _user_input: &str,
) -> ! {
    todo!("Plexi IQ Stage 1: implement the streaming turn loop (spec §3.2).")
}
