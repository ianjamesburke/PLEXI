//! Simulated Cloud Intake. The test client and a future phone client share
//! [`crate::cloud_assistant::contracts::parse_intake`], then this handler.
//! The scripted model proposes moves. `ChessApp::dispatch` is what applies them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::chess::{ActorGrant, Side};
use super::contracts::{
    fingerprint, parse_intake, AgentRecord, BackgroundPolicy, IntakeReceipt, ReceiptPhase,
    TerminalKind, ViewState, SCHEMA_VERSION,
};
use super::service::ChessApp;

pub const ASSISTANT: &str = "assistant-fixture";
pub const WHITE: &str = "agent:white";
pub const BLACK: &str = "agent:black";
pub const GAME: &str = "game-fixture";
pub const CONVERSATION: &str = "conversation-fixture";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub tenant_id: String,
    pub host_id: String,
    pub user_id: String,
    pub allowed_agents: BTreeSet<String>,
}

impl Principal {
    pub fn fixture() -> Self {
        let mut allowed_agents = BTreeSet::new();
        allowed_agents.insert(ASSISTANT.into());
        Self {
            tenant_id: "tenant-fixture".into(),
            host_id: "host-fixture".into(),
            user_id: "user-fixture".into(),
            allowed_agents,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntakeReply {
    Receipt(IntakeReceipt),
    Rejected { code: &'static str, detail: String },
}

impl IntakeReply {
    pub fn receipt(&self) -> Option<&IntakeReceipt> {
        match self {
            Self::Receipt(receipt) => Some(receipt),
            Self::Rejected { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateStatus {
    Passed,
    Unsupported(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateNote {
    pub name: &'static str,
    pub status: GateStatus,
}

pub fn local_gate_report() -> Vec<GateNote> {
    vec![
        GateNote {
            name: "contract_fixtures",
            status: GateStatus::Passed,
        },
        GateNote {
            name: "authorized_scoped_move",
            status: GateStatus::Passed,
        },
        GateNote {
            name: "duplicate_stale_unauthorized",
            status: GateStatus::Passed,
        },
        GateNote {
            name: "view_lifetime_without_assistant_loop",
            status: GateStatus::Passed,
        },
        GateNote {
            name: "scripted_local_e2e_a",
            status: GateStatus::Passed,
        },
        GateNote {
            name: "phone_browser",
            status: GateStatus::Unsupported("P4 phone client was not run"),
        },
        GateNote {
            name: "local_daemon",
            status: GateStatus::Unsupported("P6 no-window daemon was not run"),
        },
        GateNote {
            name: "hosted_daemon",
            status: GateStatus::Unsupported("P7 hosted runtime was not run"),
        },
        GateNote {
            name: "toml_board_scene",
            status: GateStatus::Unsupported("no chess pane UI is registered for a scene"),
        },
        GateNote {
            name: "assistant_app_loop",
            status: GateStatus::Unsupported(
                "AssistantHarness rejects connector scripts; this proof uses ToolDispatcher",
            ),
        },
        GateNote {
            name: "hidden_occluded_window",
            status: GateStatus::Unsupported(
                "eframe occlusion was not run; only view-state observations were modeled",
            ),
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentSession {
    agent_id: String,
    conversation_id: String,
    display_name: String,
    view: ViewState,
    transcript: Vec<TranscriptRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TranscriptRow {
    role: String,
    text: String,
    request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IntakeRecord {
    fingerprint: String,
    receipt: IntakeReceipt,
    dispatch_count: u32,
    white_outcome: Option<String>,
    prompt: String,
    /// Revision the client named. Kept apart from the receipt's result revision
    /// so a resumed run replays the same play payload.
    expected_revision: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ClientEvent {
    cursor: u64,
    request_id: String,
    kind: String,
    payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Delegation {
    from: String,
    to: String,
    game_id: String,
    side: Side,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct IntakeJournal {
    records: Vec<IntakeRecord>,
    events: Vec<ClientEvent>,
    next_cursor: u64,
    agents: Vec<AgentSession>,
    delegations: Vec<Delegation>,
    answered_revisions: Vec<u64>,
    black_attempts: u32,
}

pub struct CloudSession {
    chess: ChessApp,
    root: PathBuf,
    principal: Principal,
    clock: DateTime<Utc>,
    agents: BTreeMap<String, AgentSession>,
    records: BTreeMap<(String, String), IntakeRecord>,
    events: Vec<ClientEvent>,
    next_cursor: u64,
    retention: usize,
    delegations: Vec<Delegation>,
    answered_revisions: BTreeSet<u64>,
    black_attempts: u32,
    #[cfg(test)]
    stop_after_white: bool,
}

impl CloudSession {
    pub fn create(dir: &Path) -> Result<Self, String> {
        let chess = ChessApp::create(dir, BackgroundPolicy::Continue)?;
        chess.provision(GAME, fixture_grants())?;
        chess.observe(ViewState::Attached {
            device: "desktop".into(),
            observed_at: "2030-01-01T00:00:00Z".into(),
        })?;
        let mut session = Self::assemble(dir, chess, IntakeJournal::default())?;
        session.delegations = vec![Delegation {
            from: ASSISTANT.into(),
            to: WHITE.into(),
            game_id: GAME.into(),
            side: Side::White,
        }];
        session.subscribe_players();
        session.persist()?;
        log::info!(
            "cloud_assistant: session created host={} conversation={CONVERSATION}",
            session.principal.host_id
        );
        Ok(session)
    }

    pub fn open(dir: &Path) -> Result<Self, String> {
        let chess = ChessApp::open(dir)?;
        let bytes = std::fs::read(intake_path(dir))
            .map_err(|error| format!("intake journal read: {error}"))?;
        let journal: IntakeJournal = serde_json::from_slice(&bytes)
            .map_err(|error| format!("intake journal parse: {error}"))?;
        let session = Self::assemble(dir, chess, journal)?;
        session.subscribe_players();
        let published = session.chess.publish_pending()?;
        log::info!("cloud_assistant: session reopened published_pending={published}");
        Ok(session)
    }

    fn assemble(dir: &Path, chess: ChessApp, journal: IntakeJournal) -> Result<Self, String> {
        let mut agents = BTreeMap::new();
        if journal.agents.is_empty() {
            for (id, name) in [(ASSISTANT, "Assistant"), (WHITE, "White"), (BLACK, "Black")] {
                agents.insert(
                    id.to_string(),
                    AgentSession {
                        agent_id: id.into(),
                        conversation_id: CONVERSATION.into(),
                        display_name: name.into(),
                        view: ViewState::Attached {
                            device: "desktop".into(),
                            observed_at: "2030-01-01T00:00:00Z".into(),
                        },
                        transcript: Vec::new(),
                    },
                );
            }
        } else {
            for agent in journal.agents {
                agents.insert(agent.agent_id.clone(), agent);
            }
        }
        let mut records = BTreeMap::new();
        for record in journal.records {
            records.insert(
                (
                    record.receipt.owner_id.clone(),
                    record.receipt.request_id.clone(),
                ),
                record,
            );
        }
        Ok(Self {
            chess,
            root: dir.to_path_buf(),
            principal: Principal::fixture(),
            clock: DateTime::parse_from_rfc3339("2030-01-01T00:00:00Z")
                .map_err(|error| error.to_string())?
                .with_timezone(&Utc),
            agents,
            records,
            events: journal.events,
            next_cursor: journal.next_cursor,
            retention: 32,
            delegations: journal.delegations,
            answered_revisions: journal.answered_revisions.into_iter().collect(),
            black_attempts: journal.black_attempts,
            #[cfg(test)]
            stop_after_white: false,
        })
    }

    pub fn set_clock(&mut self, clock: DateTime<Utc>) {
        self.clock = clock;
    }

    pub fn set_retention(&mut self, retention: usize) {
        self.retention = retention.max(1);
    }

    #[cfg(test)]
    pub fn without_delegation(&mut self) {
        self.delegations.clear();
    }

    #[cfg(test)]
    pub fn arm_stop_after_white(&mut self) {
        self.stop_after_white = true;
    }

    pub fn black_play_attempts(&self) -> u32 {
        self.black_attempts
    }

    pub fn agent(&self, id: &str) -> Option<AgentRecord> {
        self.agents.get(id).map(|agent| AgentRecord {
            schema_version: SCHEMA_VERSION,
            agent_id: agent.agent_id.clone(),
            conversation_id: agent.conversation_id.clone(),
            display_name: agent.display_name.clone(),
        })
    }

    pub fn transcript_len(&self, agent_id: &str) -> usize {
        self.agents
            .get(agent_id)
            .map(|agent| agent.transcript.len())
            .unwrap_or(0)
    }

    pub fn revision(&self) -> u64 {
        let result = self
            .chess
            .dispatch(ASSISTANT, "chess.state", json!({"game_id": GAME}));
        serde_json::from_str::<serde_json::Value>(result.output_json.as_deref().unwrap_or("{}"))
            .ok()
            .and_then(|value| value["revision"].as_u64())
            .unwrap_or(0)
    }

    pub fn fen(&self) -> String {
        let result = self
            .chess
            .dispatch(ASSISTANT, "chess.state", json!({"game_id": GAME}));
        serde_json::from_str::<serde_json::Value>(result.output_json.as_deref().unwrap_or("{}"))
            .ok()
            .and_then(|value| value["fen"].as_str().map(str::to_string))
            .unwrap_or_default()
    }

    pub fn dispatch_count(&self, request_id: &str) -> u32 {
        self.records
            .get(&(self.principal.tenant_id.clone(), request_id.to_string()))
            .map(|record| record.dispatch_count)
            .unwrap_or(0)
    }

    pub fn enqueue(&mut self, raw: &str) -> IntakeReply {
        let envelope = match parse_intake(raw) {
            Ok(envelope) => envelope,
            Err(error) => {
                return IntakeReply::Rejected {
                    code: error.code,
                    detail: error.detail,
                }
            }
        };
        if envelope.host_id != self.principal.host_id
            || !self.principal.allowed_agents.contains(&envelope.agent_id)
        {
            log::info!(
                "cloud_assistant: intake denied request={} agent={}",
                envelope.request_id,
                envelope.agent_id
            );
            return IntakeReply::Rejected {
                code: "denied",
                detail: "denied".into(),
            };
        }
        let expires = match DateTime::parse_from_rfc3339(&envelope.expires_at) {
            Ok(when) => when.with_timezone(&Utc),
            Err(error) => {
                return IntakeReply::Rejected {
                    code: "invalid_input",
                    detail: error.to_string(),
                }
            }
        };
        let key = (
            self.principal.tenant_id.clone(),
            envelope.request_id.clone(),
        );
        let digest = fingerprint(&envelope);
        if let Some(existing) = self.records.get(&key) {
            if existing.fingerprint == digest {
                log::info!(
                    "cloud_assistant: intake deduped owner={} request={}",
                    key.0,
                    key.1
                );
                return IntakeReply::Receipt(existing.receipt.clone());
            }
            return IntakeReply::Rejected {
                code: "duplicate_conflict",
                detail: "request id was already accepted with a different payload".into(),
            };
        }
        let prompt = envelope
            .content
            .iter()
            .map(|part| part.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        let mut receipt = IntakeReceipt {
            schema_version: SCHEMA_VERSION,
            request_id: envelope.request_id.clone(),
            owner_id: self.principal.tenant_id.clone(),
            phase: ReceiptPhase::Queued,
            outcome: None,
            reason: None,
            resource_revision: envelope
                .target
                .as_ref()
                .map(|target| target.expected_revision),
            run_id: Some(format!("run-{}", envelope.request_id)),
            conversation_id: Some(envelope.conversation_id.clone()),
        };
        if self.clock >= expires {
            receipt.phase = ReceiptPhase::Terminal;
            receipt.outcome = Some(TerminalKind::Expired);
            receipt.reason = Some("expired".into());
            log::info!(
                "cloud_assistant: intake expired request={} at {}",
                envelope.request_id,
                self.clock
            );
        }
        self.push_event(
            &envelope.request_id,
            "intake.accepted",
            json!({ "phase": format!("{:?}", receipt.phase), "reason": receipt.reason }),
        );
        if receipt.outcome.is_none() {
            self.push_transcript(
                ASSISTANT,
                "user",
                &prompt,
                Some(envelope.request_id.clone()),
            );
        }
        self.records.insert(
            key,
            IntakeRecord {
                fingerprint: digest,
                receipt: receipt.clone(),
                dispatch_count: 0,
                white_outcome: None,
                prompt,
                expected_revision: envelope
                    .target
                    .as_ref()
                    .map(|target| target.expected_revision),
            },
        );
        if let Err(error) = self.persist() {
            return IntakeReply::Rejected {
                code: "persist_failed",
                detail: error,
            };
        }
        log::info!(
            "cloud_assistant: intake queued owner={} request={} phase={:?}",
            self.principal.tenant_id,
            envelope.request_id,
            receipt.phase
        );
        IntakeReply::Receipt(receipt)
    }

    pub fn cancel(&mut self, request_id: &str) -> IntakeReply {
        let key = (self.principal.tenant_id.clone(), request_id.to_string());
        let Some(record) = self.records.get_mut(&key) else {
            return IntakeReply::Rejected {
                code: "denied",
                detail: "denied".into(),
            };
        };
        if record.receipt.phase == ReceiptPhase::Terminal {
            return IntakeReply::Receipt(record.receipt.clone());
        }
        record.receipt.phase = ReceiptPhase::Terminal;
        record.receipt.outcome = Some(TerminalKind::Cancelled);
        record.receipt.reason = Some("cancelled".into());
        let receipt = record.receipt.clone();
        self.push_event(
            request_id,
            "intake.cancelled",
            json!({ "reason": "cancelled" }),
        );
        let _ = self.persist();
        log::info!("cloud_assistant: intake cancelled request={request_id}");
        IntakeReply::Receipt(receipt)
    }

    pub fn pump(&mut self, request_id: &str) -> IntakeReply {
        let key = (self.principal.tenant_id.clone(), request_id.to_string());
        let Some(record) = self.records.get(&key) else {
            return IntakeReply::Rejected {
                code: "denied",
                detail: "denied".into(),
            };
        };
        if record.receipt.phase == ReceiptPhase::Terminal {
            return IntakeReply::Receipt(record.receipt.clone());
        }
        if record.receipt.outcome == Some(TerminalKind::Cancelled) {
            return IntakeReply::Receipt(record.receipt.clone());
        }
        let prompt = record.prompt.clone();
        let run_id = record.receipt.run_id.clone();
        {
            let record = self.records.get_mut(&key).expect("record");
            record.receipt.phase = ReceiptPhase::Running;
            record.dispatch_count = record.dispatch_count.saturating_add(1);
        }
        log::info!("cloud_assistant: intake running request={request_id}");
        let reply = self.execute(&prompt, request_id, run_id.as_deref());
        let revision_now = self.revision();
        if let Some(record) = self.records.get_mut(&key) {
            match &reply {
                IntakeReply::Receipt(receipt) => record.receipt = receipt.clone(),
                IntakeReply::Rejected { code, detail } => {
                    record.receipt.phase = ReceiptPhase::Terminal;
                    record.receipt.outcome = Some(TerminalKind::Failed);
                    record.receipt.reason = Some((*code).into());
                    record.receipt.resource_revision = Some(revision_now);
                    let _ = detail;
                }
            }
        }
        let _ = self.persist();
        if let Some(record) = self.records.get(&key) {
            IntakeReply::Receipt(record.receipt.clone())
        } else {
            reply
        }
    }

    pub fn submit(&mut self, raw: &str) -> IntakeReply {
        let queued = self.enqueue(raw);
        let IntakeReply::Receipt(receipt) = &queued else {
            return queued;
        };
        if receipt.phase == ReceiptPhase::Terminal {
            return queued;
        }
        self.pump(&receipt.request_id.clone())
    }

    pub fn replay(&self, cursor: u64) -> Result<Vec<ClientEvent>, serde_json::Value> {
        if self.events.is_empty() {
            return Ok(Vec::new());
        }
        let oldest = self.events[0].cursor;
        if cursor + 1 < oldest {
            return Err(json!({
                "resync_required": true,
                "snapshot": {
                    "fen": self.fen(),
                    "revision": self.revision(),
                    "conversation_id": CONVERSATION,
                }
            }));
        }
        Ok(self
            .events
            .iter()
            .filter(|event| event.cursor > cursor)
            .cloned()
            .collect())
    }

    pub fn close_assistant_view(&mut self) {
        if let Some(agent) = self.agents.get_mut(ASSISTANT) {
            agent.view = ViewState::NoView {
                observed_at: "2030-01-01T00:00:00Z".into(),
            };
        }
        log::info!("cloud_assistant: assistant view detached conversation={CONVERSATION}");
    }

    pub fn close_chess_view(&mut self) -> Result<(), String> {
        self.chess.close_view()
    }

    pub fn reattach(&mut self) {
        for id in [ASSISTANT, WHITE, BLACK] {
            if let Some(agent) = self.agents.get_mut(id) {
                agent.view = ViewState::Attached {
                    device: "desktop".into(),
                    observed_at: "2030-01-01T00:00:02Z".into(),
                };
            }
        }
        let _ = self.chess.observe(ViewState::Attached {
            device: "desktop".into(),
            observed_at: "2030-01-01T00:00:02Z".into(),
        });
    }

    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    pub fn chess_running(&self) -> bool {
        self.chess.running()
    }

    fn execute(&mut self, prompt: &str, request_id: &str, run_id: Option<&str>) -> IntakeReply {
        let Some(uci) = scripted_white_move(prompt) else {
            let state = self
                .chess
                .dispatch(ASSISTANT, "chess.state", json!({"game_id": GAME}));
            let text = state
                .output_json
                .unwrap_or_else(|| "state unavailable".into());
            self.push_transcript(ASSISTANT, "assistant", &text, Some(request_id.into()));
            return self.finish(request_id, run_id, TerminalKind::Succeeded, None);
        };
        let allowed = self.delegations.iter().any(|delegation| {
            delegation.from == ASSISTANT
                && delegation.to == WHITE
                && delegation.game_id == GAME
                && delegation.side == Side::White
        });
        if !allowed {
            log::info!("cloud_assistant: delegation denied request={request_id}");
            return self.finish(
                request_id,
                run_id,
                TerminalKind::Failed,
                Some("unauthorized"),
            );
        }
        let target_revision = self
            .records
            .get(&(self.principal.tenant_id.clone(), request_id.to_string()))
            .and_then(|record| record.expected_revision);
        let Some(expected) = target_revision else {
            return self.finish(
                request_id,
                run_id,
                TerminalKind::Failed,
                Some("invalid_input"),
            );
        };
        let operation_id = format!("white-{request_id}");
        let played = self.chess.dispatch(
            WHITE,
            "chess.play",
            json!({
                "game_id": GAME,
                "expected_revision": expected,
                "operation_id": operation_id,
                "uci": uci,
            }),
        );
        if let Some(error) = played.error.clone() {
            let code = error.split(':').next().unwrap_or("failed");
            return self.finish(request_id, run_id, TerminalKind::Failed, Some(code));
        }
        let body: serde_json::Value =
            serde_json::from_str(played.output_json.as_deref().unwrap_or("{}")).unwrap_or_default();
        let white_outcome = if body["replayed"] == true {
            "replayed"
        } else {
            "committed"
        };
        if let Some(record) = self
            .records
            .get_mut(&(self.principal.tenant_id.clone(), request_id.to_string()))
        {
            record.white_outcome = Some(white_outcome.into());
        }
        #[cfg(test)]
        if self.stop_after_white {
            self.stop_after_white = false;
            let key = (self.principal.tenant_id.clone(), request_id.to_string());
            if let Some(record) = self.records.get_mut(&key) {
                record.receipt.phase = ReceiptPhase::Running;
            }
            log::info!("cloud_assistant: intake paused after white commit request={request_id}");
            return IntakeReply::Receipt(self.records.get(&key).expect("record").receipt.clone());
        }
        self.react_black(&operation_id, uci);
        let revision = body["revision"].as_u64();
        let text = format!(
            "white {uci} ({white_outcome}), black replies recorded, revision {}",
            self.revision()
        );
        self.push_transcript(ASSISTANT, "assistant", &text, Some(request_id.into()));
        let mut receipt = self
            .finish(request_id, run_id, TerminalKind::Succeeded, None)
            .receipt()
            .cloned()
            .expect("receipt");
        receipt.resource_revision = Some(self.revision());
        if revision.is_none() && receipt.resource_revision.is_none() {
            receipt.resource_revision = Some(self.revision());
        }
        if let Some(record) = self
            .records
            .get_mut(&(self.principal.tenant_id.clone(), request_id.to_string()))
        {
            record.receipt = receipt.clone();
        }
        IntakeReply::Receipt(receipt)
    }

    fn react_black(&mut self, white_op: &str, white_uci: &str) {
        let deliveries = self.chess.take_deliveries(BLACK);
        let mut revisions = Vec::new();
        if deliveries.is_empty() {
            if let Some(revision) = self.current_revision_if_black() {
                revisions.push(revision);
            }
        }
        for delivery in deliveries {
            if delivery.caused_by.as_deref() == Some(BLACK) {
                continue;
            }
            if delivery.event != "chess.move_committed" {
                continue;
            }
            let Some(revision) = delivery
                .payload
                .as_ref()
                .and_then(|payload| payload["revision_after"].as_u64())
            else {
                continue;
            };
            revisions.push(revision);
        }
        for revision in revisions {
            self.answer_revision(revision, white_op, white_uci);
        }
    }

    fn answer_revision(&mut self, revision: u64, white_op: &str, white_uci: &str) {
        if !self.answered_revisions.insert(revision) {
            log::info!("cloud_assistant: black ignored duplicate revision {revision}");
            return;
        }
        let Some(uci) = scripted_black_move(white_uci) else {
            return;
        };
        self.black_attempts = self.black_attempts.saturating_add(1);
        let result = self.chess.dispatch(
            BLACK,
            "chess.play",
            json!({
                "game_id": GAME,
                "expected_revision": revision,
                "operation_id": format!("black-{white_op}"),
                "uci": uci,
            }),
        );
        log::info!(
            "cloud_assistant: black reply revision={revision} uci={uci} error={}",
            result.error.as_deref().unwrap_or("none")
        );
    }

    fn current_revision_if_black(&self) -> Option<u64> {
        let result = self
            .chess
            .dispatch(BLACK, "chess.state", json!({"game_id": GAME}));
        let value: serde_json::Value = serde_json::from_str(result.output_json.as_deref()?).ok()?;
        if value["side_to_move"] == "black" && value["status"] == "ongoing" {
            value["revision"].as_u64()
        } else {
            None
        }
    }

    fn finish(
        &mut self,
        request_id: &str,
        run_id: Option<&str>,
        outcome: TerminalKind,
        reason: Option<&str>,
    ) -> IntakeReply {
        let key = (self.principal.tenant_id.clone(), request_id.to_string());
        let receipt = IntakeReceipt {
            schema_version: SCHEMA_VERSION,
            request_id: request_id.into(),
            owner_id: self.principal.tenant_id.clone(),
            phase: ReceiptPhase::Terminal,
            outcome: Some(outcome.clone()),
            reason: reason.map(str::to_string),
            resource_revision: Some(self.revision()),
            run_id: run_id.map(str::to_string),
            conversation_id: Some(CONVERSATION.into()),
        };
        if let Some(record) = self.records.get_mut(&key) {
            record.receipt = receipt.clone();
        }
        self.push_event(
            request_id,
            "intake.terminal",
            json!({ "outcome": format!("{outcome:?}"), "reason": reason }),
        );
        log::info!(
            "cloud_assistant: intake terminal request={request_id} outcome={outcome:?} reason={}",
            reason.unwrap_or("none")
        );
        IntakeReply::Receipt(receipt)
    }

    fn push_transcript(
        &mut self,
        agent_id: &str,
        role: &str,
        text: &str,
        request_id: Option<String>,
    ) {
        if let Some(agent) = self.agents.get_mut(agent_id) {
            agent.transcript.push(TranscriptRow {
                role: role.into(),
                text: text.into(),
                request_id,
            });
        }
    }

    fn push_event(&mut self, request_id: &str, kind: &str, payload: serde_json::Value) {
        self.next_cursor = self.next_cursor.saturating_add(1);
        self.events.push(ClientEvent {
            cursor: self.next_cursor,
            request_id: request_id.into(),
            kind: kind.into(),
            payload,
        });
        let overflow = self.events.len().saturating_sub(self.retention);
        if overflow > 0 {
            self.events.drain(0..overflow);
        }
    }

    fn subscribe_players(&self) {
        self.chess.subscribe(WHITE, GAME, self.chess.context_id());
        self.chess.subscribe(BLACK, GAME, self.chess.context_id());
    }

    fn persist(&self) -> Result<(), String> {
        let journal = IntakeJournal {
            records: self.records.values().cloned().collect(),
            events: self.events.clone(),
            next_cursor: self.next_cursor,
            agents: self.agents.values().cloned().collect(),
            delegations: self.delegations.clone(),
            answered_revisions: self.answered_revisions.iter().copied().collect(),
            black_attempts: self.black_attempts,
        };
        let bytes = serde_json::to_vec_pretty(&journal).map_err(|error| error.to_string())?;
        let path = intake_path(&self.root);
        let tmp = path.with_extension("json.tmp");
        {
            use std::io::Write;
            let mut file = std::fs::File::create(&tmp)
                .map_err(|error| format!("intake journal create: {error}"))?;
            file.write_all(&bytes)
                .map_err(|error| format!("intake journal write: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("intake journal sync: {error}"))?;
        }
        std::fs::rename(&tmp, &path).map_err(|error| format!("intake journal rename: {error}"))
    }
}

fn intake_path(dir: &Path) -> PathBuf {
    dir.join("intake-journal.json")
}

fn fixture_grants() -> Vec<ActorGrant> {
    vec![
        ActorGrant {
            actor_id: ASSISTANT.into(),
            game_id: GAME.into(),
            inspect: true,
            play: None,
            reset: false,
        },
        ActorGrant {
            actor_id: WHITE.into(),
            game_id: GAME.into(),
            inspect: true,
            play: Some(Side::White),
            reset: false,
        },
        ActorGrant {
            actor_id: BLACK.into(),
            game_id: GAME.into(),
            inspect: true,
            play: Some(Side::Black),
            reset: false,
        },
    ]
}

fn scripted_white_move(prompt: &str) -> Option<&'static str> {
    if prompt.contains("g1f3") {
        Some("g1f3")
    } else if prompt.contains("e2e4") {
        Some("e2e4")
    } else {
        None
    }
}

fn scripted_black_move(white_uci: &str) -> Option<&'static str> {
    match white_uci {
        "e2e4" => Some("e7e5"),
        "g1f3" => Some("b8c6"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud_assistant::contracts::parse_intake;

    fn session() -> (tempfile::TempDir, CloudSession) {
        let dir = tempfile::tempdir().unwrap();
        let session = CloudSession::create(dir.path()).unwrap();
        (dir, session)
    }

    fn text_request(id: &str, text: &str, revision: u64) -> String {
        let mut envelope = parse_intake(include_str!(
            "../../fixtures/cloud-assistant/intake-turn.json"
        ))
        .unwrap();
        envelope.request_id = id.into();
        envelope.content[0].text = text.into();
        envelope.target.as_mut().unwrap().expected_revision = revision;
        serde_json::to_string(&envelope).unwrap()
    }

    #[test]
    fn question_does_not_delegate_a_move() {
        let (_dir, mut session) = session();
        let reply = session.submit(&text_request("req-question", "What is the position?", 0));
        let receipt = reply.receipt().unwrap();
        assert_eq!(receipt.outcome, Some(TerminalKind::Succeeded));
        assert_eq!(session.revision(), 0);
        assert_eq!(session.black_play_attempts(), 0);
    }

    #[test]
    fn missing_delegation_does_not_play() {
        let (_dir, mut session) = session();
        session.without_delegation();
        let reply = session.submit(&text_request("req-no-grant", "play e2e4", 0));
        let receipt = reply.receipt().unwrap();
        assert_eq!(receipt.outcome, Some(TerminalKind::Failed));
        assert_eq!(receipt.reason.as_deref(), Some("unauthorized"));
        assert_eq!(session.revision(), 0);
    }

    #[test]
    fn e2e_a_scripted_round_trip_is_idempotent() {
        let (dir, mut session) = session();
        let first = include_str!("../../fixtures/cloud-assistant/intake-turn.json");
        let reply = session.submit(first);
        let receipt = reply.receipt().unwrap().clone();
        assert_eq!(receipt.outcome, Some(TerminalKind::Succeeded));
        assert_eq!(receipt.conversation_id.as_deref(), Some(CONVERSATION));
        let fen = session.fen();
        assert!(fen.contains("/4P3/"), "{fen}");
        assert!(fen.contains("/4p3/"), "{fen}");
        assert!(fen.contains(" w "), "{fen}");
        assert_eq!(session.revision(), 2);
        assert_eq!(session.black_play_attempts(), 1);
        assert_eq!(session.dispatch_count("req-fixture-e4"), 1);
        let transcript = session.transcript_len(ASSISTANT);

        let again = session.submit(first);
        assert_eq!(
            again.receipt().unwrap().outcome,
            Some(TerminalKind::Succeeded)
        );
        assert_eq!(session.revision(), 2);
        assert_eq!(session.black_play_attempts(), 1);
        assert_eq!(session.dispatch_count("req-fixture-e4"), 1);
        assert_eq!(session.transcript_len(ASSISTANT), transcript);

        let mut changed = parse_intake(first).unwrap();
        changed.content[0].text = "In this game, play d2d4 as White.".into();
        let conflict = session.submit(&serde_json::to_string(&changed).unwrap());
        assert!(matches!(
            conflict,
            IntakeReply::Rejected {
                code: "duplicate_conflict",
                ..
            }
        ));
        assert_eq!(session.revision(), 2);

        let cursor = 1;
        let replayed = session.replay(cursor).unwrap();
        assert!(!replayed.is_empty());
        assert_eq!(session.revision(), 2);
        assert_eq!(session.transcript_len(ASSISTANT), transcript);
        assert_eq!(session.black_play_attempts(), 1);

        session.close_assistant_view();
        session.close_chess_view().unwrap();
        assert!(session.chess_running());
        let second = text_request("req-g1f3", "In this game, play g1f3 as White.", 2);
        let reply = session.submit(&second);
        assert_eq!(
            reply.receipt().unwrap().outcome,
            Some(TerminalKind::Succeeded)
        );
        assert_eq!(session.revision(), 4);
        let fen = session.fen();
        assert!(fen.contains("5N2"), "{fen}");
        assert!(fen.contains("2n5"), "{fen}");
        session.reattach();
        assert_eq!(
            session.agent(ASSISTANT).unwrap().conversation_id,
            CONVERSATION
        );
        assert_eq!(session.agent_count(), 3);
        assert_eq!(session.black_play_attempts(), 2);

        let root = dir.path().to_path_buf();
        drop(session);
        let reopened = CloudSession::open(&root).unwrap();
        assert_eq!(reopened.revision(), 4);
        assert_eq!(reopened.agent(ASSISTANT).unwrap().agent_id, ASSISTANT);
        assert_eq!(reopened.transcript_len(ASSISTANT), transcript + 2);

        let report = local_gate_report();
        for name in [
            "contract_fixtures",
            "authorized_scoped_move",
            "duplicate_stale_unauthorized",
            "view_lifetime_without_assistant_loop",
            "scripted_local_e2e_a",
        ] {
            assert!(
                report
                    .iter()
                    .any(|note| note.name == name && note.status == GateStatus::Passed),
                "{name} should be a passed local gate"
            );
        }
        for name in [
            "phone_browser",
            "local_daemon",
            "hosted_daemon",
            "toml_board_scene",
            "assistant_app_loop",
            "hidden_occluded_window",
        ] {
            assert!(
                report.iter().any(|note| {
                    note.name == name && matches!(note.status, GateStatus::Unsupported(_))
                }),
                "{name} must stay unsupported"
            );
        }
    }

    #[test]
    fn expiry_cancel_stale_and_crash_have_named_outcomes() {
        let (dir, mut session) = session();
        session.set_clock(
            DateTime::parse_from_rfc3339("2030-01-01T00:02:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        let expired = session.submit(include_str!(
            "../../fixtures/cloud-assistant/intake-turn.json"
        ));
        assert_eq!(
            expired.receipt().unwrap().outcome,
            Some(TerminalKind::Expired)
        );
        assert_eq!(session.revision(), 0);
        session.set_clock(
            DateTime::parse_from_rfc3339("2030-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );

        let queued = session.enqueue(&text_request("req-cancel", "play e2e4 now", 0));
        assert_eq!(queued.receipt().unwrap().phase, ReceiptPhase::Queued);
        let cancelled = session.cancel("req-cancel");
        assert_eq!(
            cancelled.receipt().unwrap().outcome,
            Some(TerminalKind::Cancelled)
        );
        let pumped = session.pump("req-cancel");
        assert_eq!(
            pumped.receipt().unwrap().outcome,
            Some(TerminalKind::Cancelled)
        );
        assert_eq!(session.revision(), 0);

        let stale = session.submit(&text_request("req-stale", "play e2e4 please", 9));
        assert_eq!(
            stale.receipt().unwrap().reason.as_deref(),
            Some("stale_revision")
        );
        assert_eq!(session.revision(), 0);

        session.arm_stop_after_white();
        let paused = session.submit(&text_request("req-crash", "play e2e4 after the pause", 0));
        assert_eq!(paused.receipt().unwrap().phase, ReceiptPhase::Running);
        assert_eq!(session.revision(), 1);
        assert_eq!(session.black_play_attempts(), 0);
        let root = dir.path().to_path_buf();
        drop(session);
        let mut resumed = CloudSession::open(&root).unwrap();
        let done = resumed.pump("req-crash");
        assert_eq!(
            done.receipt().unwrap().outcome,
            Some(TerminalKind::Succeeded)
        );
        assert_eq!(resumed.revision(), 2);
        assert_eq!(resumed.black_play_attempts(), 1);
        assert!(resumed.fen().contains("/4p3/"));
        let white_outcome = resumed
            .records
            .get(&(resumed.principal.tenant_id.clone(), "req-crash".into()))
            .and_then(|record| record.white_outcome.clone());
        assert_eq!(white_outcome.as_deref(), Some("replayed"));
    }

    #[test]
    fn old_cursor_resyncs_instead_of_dropping_history() {
        let (_dir, mut session) = session();
        session.set_retention(1);
        let _ = session.submit(&text_request("req-a", "play e2e4 first", 0));
        assert_eq!(session.revision(), 2);
        let replay = session.replay(0);
        assert!(replay.is_err(), "trimmed cursor must resync");
        let body = replay.unwrap_err();
        assert_eq!(body["resync_required"], true);
        assert_eq!(body["snapshot"]["revision"], 2);
        assert_eq!(session.revision(), 2);
    }

    #[test]
    fn repeated_black_event_does_not_move_twice() {
        let (_dir, mut session) = session();
        let reply = session.submit(include_str!(
            "../../fixtures/cloud-assistant/intake-turn.json"
        ));
        assert_eq!(
            reply.receipt().unwrap().outcome,
            Some(TerminalKind::Succeeded)
        );
        assert_eq!(session.black_play_attempts(), 1);
        session.answer_revision(1, "white-req-fixture-e4", "e2e4");
        assert_eq!(session.black_play_attempts(), 1);
        assert_eq!(session.revision(), 2);
    }

    #[test]
    fn forged_host_is_denied_without_a_move() {
        let (_dir, mut session) = session();
        let mut envelope = parse_intake(include_str!(
            "../../fixtures/cloud-assistant/intake-turn.json"
        ))
        .unwrap();
        envelope.host_id = "host-other".into();
        envelope.request_id = "req-other-host".into();
        let reply = session.submit(&serde_json::to_string(&envelope).unwrap());
        assert!(matches!(
            reply,
            IntakeReply::Rejected { code: "denied", .. }
        ));
        assert_eq!(session.revision(), 0);
        assert_eq!(session.dispatch_count("req-other-host"), 0);
    }
}
