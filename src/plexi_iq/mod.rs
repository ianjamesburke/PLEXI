//! Plexi IQ — agent runtime + brokered LLM capability.
//!
//! Public surface today (#284):
//!   - `backend` — `LlmBackend` trait + concrete `AnthropicApiBackend`. The
//!     proxied `claude_cli` backend will land with #285 (agent-as-app).
//!   - `broker`  — `IqBroker` trait + `LiveIqBroker`. The host-side bridge
//!     that fields `DrawCommand::IqQuery`, gates on the `iq.query`
//!     capability, dispatches to a backend, and writes to the ledger.
//!   - `ledger`  — append-only JSONL row of every brokered call.
//!   - `turn_loop` — single-turn driver consumed by the broker.
//!
//! The deeper agent runtime (`PlexiIq` / `PlexiIqInstance` / `ToolContext` /
//! per-pane sessions) lives behind the v3.3 #285 milestone — not this PR.

pub mod backend;
pub mod broker;
pub mod ledger;
#[path = "loop.rs"]
pub mod turn_loop;
