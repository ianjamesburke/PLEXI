//! Program Status Protocol (OSC 7501): scan PTY bytes, keep per-pane records.
//!
//! The tap only observes. The terminal parser still sees every byte exactly
//! as before; the only bytes written back are the feature-query reply.
mod agent_state;
pub mod emit;
mod parser;
mod records;
#[cfg(test)]
mod tests;

use egui_term::PtyByteTap;
use std::sync::{Arc, Mutex};

pub use agent_state::{records_json, reported_agent_state, root_or_best_agent_state};
pub use parser::{parse_report, Osc7501Scanner, ScanEvent, FEATURE_REPLY};
pub use records::{ProgramStatusStore, State};

/// Store shared between the PTY reader thread (tap) and the pane.
pub type SharedProgramStatus = Arc<Mutex<ProgramStatusStore>>;

pub struct ProgramStatusTap {
    pane_id: u64,
    scanner: Osc7501Scanner,
    store: SharedProgramStatus,
}

impl ProgramStatusTap {
    pub fn new(pane_id: u64, store: SharedProgramStatus) -> Self {
        Self {
            pane_id,
            scanner: Osc7501Scanner::new(),
            store,
        }
    }
}

impl PtyByteTap for ProgramStatusTap {
    fn on_bytes(&mut self, bytes: &[u8]) -> Vec<u8> {
        let events = self.scanner.feed(bytes);
        let mut reply = Vec::new();
        if events.is_empty() {
            return reply;
        }
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        for event in events {
            match event {
                ScanEvent::Report(payload) => match parse_report(&payload) {
                    Some(report) => {
                        let (id, state) = match &report {
                            records::Report::Set { id, record } => {
                                (id.clone(), Some(record.state))
                            }
                            records::Report::Clear { id } => (id.clone(), None),
                        };
                        let previous = store.apply(report);
                        if previous != state {
                            log::info!(
                                "program_status: pane {} id={id:?} state={:?}",
                                self.pane_id,
                                state.map(records::State::as_str).unwrap_or("clear"),
                            );
                        }
                    }
                    None => log::debug!(
                        "program_status: pane {} dropped report ({} bytes)",
                        self.pane_id,
                        payload.len()
                    ),
                },
                ScanEvent::Query => {
                    log::info!("program_status: pane {} feature query", self.pane_id);
                    reply.extend_from_slice(FEATURE_REPLY);
                }
                ScanEvent::PromptStart => store.on_prompt_start(),
                ScanEvent::Reset => store.reset(),
            }
        }
        reply
    }
}
