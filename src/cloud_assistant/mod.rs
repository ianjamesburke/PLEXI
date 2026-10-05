//! Local Cloud Assistant contracts: intake, chess tools, and scripted proof.
//!
//! The owning spec is `docs/cloud-assistant.md`. Authority rules stay in
//! `docs/assistant-authority-model.md`. This module does not open a public
//! relay, a phone client, or a headless daemon.

mod chess;
mod contracts;
mod service;
mod session;

/// Logs the linked intake schema. Referencing the session entry points keeps
/// the installable binary from treating the chess and intake paths as dead.
pub(crate) fn log_contract() {
    let passed = session::local_gate_report()
        .iter()
        .filter(|note| note.status == session::GateStatus::Passed)
        .count();
    log::info!(
        "cloud_assistant: schema {} local gates linked={passed}",
        contracts::SCHEMA_VERSION
    );
    link_entrypoints();
}

fn link_entrypoints() {
    link(
        session::CloudSession::create
            as fn(&std::path::Path) -> Result<session::CloudSession, String>,
    );
    link(
        session::CloudSession::open
            as fn(&std::path::Path) -> Result<session::CloudSession, String>,
    );
    link(
        session::CloudSession::enqueue
            as fn(&mut session::CloudSession, &str) -> session::IntakeReply,
    );
    link(
        session::CloudSession::cancel
            as fn(&mut session::CloudSession, &str) -> session::IntakeReply,
    );
    link(
        session::CloudSession::pump as fn(&mut session::CloudSession, &str) -> session::IntakeReply,
    );
    link(
        session::CloudSession::submit
            as fn(&mut session::CloudSession, &str) -> session::IntakeReply,
    );
    link(
        session::CloudSession::replay
            as fn(
                &session::CloudSession,
                u64,
            ) -> Result<Vec<session::ClientEvent>, serde_json::Value>,
    );
    link(session::CloudSession::close_assistant_view as fn(&mut session::CloudSession));
    link(
        session::CloudSession::close_chess_view
            as fn(&mut session::CloudSession) -> Result<(), String>,
    );
    link(session::CloudSession::reattach as fn(&mut session::CloudSession));
    link(
        session::CloudSession::set_clock
            as fn(&mut session::CloudSession, chrono::DateTime<chrono::Utc>),
    );
    link(session::CloudSession::set_retention as fn(&mut session::CloudSession, usize));
    link(session::CloudSession::agent_count as fn(&session::CloudSession) -> usize);
    link(session::CloudSession::chess_running as fn(&session::CloudSession) -> bool);
    link(session::CloudSession::black_play_attempts as fn(&session::CloudSession) -> u32);
    link(
        session::CloudSession::agent
            as fn(&session::CloudSession, &str) -> Option<contracts::AgentRecord>,
    );
    link(session::CloudSession::revision as fn(&session::CloudSession) -> u64);
    link(session::CloudSession::fen as fn(&session::CloudSession) -> String);
    link(session::CloudSession::dispatch_count as fn(&session::CloudSession, &str) -> u32);
    link(session::CloudSession::transcript_len as fn(&session::CloudSession, &str) -> usize);
    link(
        session::IntakeReply::receipt
            as fn(&session::IntakeReply) -> Option<&contracts::IntakeReceipt>,
    );
    link(
        service::ChessApp::dispatch_in_context
            as fn(
                &service::ChessApp,
                &str,
                u64,
                &str,
                serde_json::Value,
            ) -> crate::plexi_ai::tool_dispatch::ToolCallResult,
    );
    link(service::ChessApp::record as fn(&service::ChessApp) -> contracts::AppInstanceRecord);
    link(service::ChessApp::pane_id as fn(&service::ChessApp) -> u64);
    link(
        chess::ChessStore::history
            as for<'a> fn(&'a chess::ChessStore, &'a str) -> Option<&'a [String]>,
    );
    link(chess::ChessStore::fen as fn(&chess::ChessStore, &str) -> Option<String>);
    link(chess::ChessStore::revision as fn(&chess::ChessStore, &str) -> Option<u64>);
    link(chess::Position::side as fn(&chess::Position) -> chess::Side);
}

fn link<T>(value: T) {
    std::hint::black_box(value);
}
