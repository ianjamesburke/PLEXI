//! Map OSC 7501 records onto Plexi's [`PaneAgentState`].
use super::records::{Kind, Record, State};
use super::{ProgramStatusStore, SharedProgramStatus};
use crate::protocol::{AgentState, PaneAgentState};

/// Fallback `PaneAgentState::agent` when no record or ancestor names an app.
const DEFAULT_APP: &str = "program";

fn rank(state: State) -> u8 {
    match state {
        State::Blocked => 4,
        State::Error => 3,
        State::Working => 2,
        State::Done => 1,
        State::Idle => 0,
    }
}

fn kind_str(kind: Kind) -> &'static str {
    match kind {
        Kind::Permission => "permission",
        Kind::Question => "question",
        Kind::Auth => "auth",
    }
}

fn detail(record: &Record) -> Option<String> {
    let msg = record.msg.as_deref().filter(|m| !m.is_empty());
    if record.state != State::Blocked {
        return msg.map(str::to_owned);
    }
    match (record.kind, msg) {
        (Some(kind), Some(msg)) => Some(format!("{}: {msg}", kind_str(kind))),
        (None, Some(msg)) => Some(msg.to_owned()),
        (Some(kind), None) => Some(kind_str(kind).to_owned()),
        (None, None) => None,
    }
}

fn agent_state(state: State) -> AgentState {
    match state {
        State::Working => AgentState::Working,
        State::Blocked => AgentState::NeedsYou,
        State::Idle => AgentState::Idle,
        State::Done => AgentState::Completed,
        State::Error => AgentState::Failed,
    }
}

/// The record that speaks for the pane: blocked > error > working > done >
/// idle, ties to the most recently updated. A blocked child surfaces even
/// when the root says working. `None` when the store is empty.
pub fn root_or_best_agent_state(
    store: &ProgramStatusStore,
    pane_id: u64,
) -> Option<PaneAgentState> {
    let (id, record) = store
        .iter()
        .max_by_key(|(_, r)| (rank(r.state), r.updated_at))?;
    Some(PaneAgentState {
        pane_id,
        state: agent_state(record.state),
        agent: store
            .effective_app(id)
            .unwrap_or(DEFAULT_APP)
            .to_owned(),
        detail: detail(record),
        session_id: None,
    })
}

/// Lock-and-map convenience for the host tick.
pub fn reported_agent_state(
    pane_id: u64,
    store: &SharedProgramStatus,
) -> Option<PaneAgentState> {
    let guard = store.lock().unwrap_or_else(|e| e.into_inner());
    root_or_best_agent_state(&guard, pane_id)
}

/// `{"records": [...], "source": "osc7501"}` for `plexi pane status`.
/// The root record has id `""`; `app` is the inherited one.
pub fn records_json(store: &ProgramStatusStore) -> serde_json::Value {
    let records: Vec<serde_json::Value> = store
        .iter()
        .map(|(id, r)| {
            serde_json::json!({
                "id": id,
                "state": r.state.as_str(),
                "kind": r.kind.map(kind_str),
                "progress": r.progress,
                "msg": r.msg,
                "title": r.title,
                "app": store.effective_app(id),
                "seen": r.seen,
            })
        })
        .collect();
    serde_json::json!({ "records": records, "source": "osc7501" })
}
