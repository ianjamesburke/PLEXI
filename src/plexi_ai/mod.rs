//! Plexi AI — agent runtime + brokered LLM capability.
//!
//! Public surface today (#284):
//!   - `backend` — `AiBackend` trait + concrete `OpenRouterBackend` + `OllamaBackend`.
//!   - `broker`  — `AiBroker` trait + `LiveAiBroker`. The host-side bridge
//!     that fields `DrawCommand::AiQuery`, gates on the `ai.query`
//!     capability, dispatches to a backend, and writes the ledger.
//!   - `ledger`  — append-only JSONL row of every brokered call.
//!   - `turn_loop` — single-turn driver consumed by the broker.

pub mod backend;
pub mod broker;
pub mod ledger;
#[path = "loop.rs"]
pub mod turn_loop;
