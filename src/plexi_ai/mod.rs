//! Plexi AI — agent runtime + brokered LLM capability.
//!
//! Public surface today (#284):
//!   - `backend` — `AiBackend` trait + concrete `OpenRouterBackend`.
//!   - `broker`  — `AiBroker` trait + `LiveAiBroker`. The host-side bridge
//!     that fields `DrawCommand::AiQuery`, gates on the `ai.query`
//!     capability, dispatches to a backend, and writes the ledger.
//!   - `ledger`  — append-only JSONL row of every brokered call.
//!   - `spend`   — admission: reserves budget before dispatch, bounds active calls.
//!   - `turn_loop` — single-turn driver consumed by the broker.

pub mod backend;
pub mod broker;
pub mod ledger;
pub mod spend;
pub mod tool_dispatch;
#[path = "loop.rs"]
pub mod turn_loop;

mod cancel;
pub use cancel::CancelToken;
