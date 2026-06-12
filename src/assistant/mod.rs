//! Host Assistant — Phase 1 of `docs/prm/assistant-host-app.md`.
//!
//! The Assistant is a first-party host app: a `Pane::App(AppRuntime::Builtin)`
//! pane, not a PGAP process. It is split into pure state (`model`), slash
//! command parsing (`commands`), disk persistence (`store`), and egui
//! rendering (`render`); this module is the pane shell that wires those to
//! the `App` trait and runs model turns on worker threads — the same
//! dispatch-on-worker / outcome-channel pattern as `crate::agent::AgentHost`.

pub mod commands;
pub mod model;
pub mod render;
pub mod store;

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use crate::app::app_trait::{App, AppRenderContext};
use crate::app_protocol::{AiMessage, ModelTier};
use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest, StreamDelta, StreamSink};

use model::{AssistantEffect, AssistantModel, TurnRole};
use render::{AssistantRenderer, ComposerEvent};
use store::AssistantStore;

const ASSISTANT_SYSTEM_PROMPT: &str = "You are the Plexi Assistant, the workspace \
operator inside the Plexi terminal environment. Answer concisely. Workspace \
tools, pane control, and app connectors arrive in a later phase — when asked \
to act on panes or apps, explain that those tools are not wired up yet.";

/// Outcome of one completed Assistant turn, sent back from the worker thread.
struct TurnOutcome {
    conversation_id: String,
    text: Option<String>,
    error: Option<String>,
}

/// The host Assistant pane: model + store + broker wiring.
pub struct AssistantApp {
    model: AssistantModel,
    store: AssistantStore,
    broker: Arc<dyn AiBroker>,
    workspace_root: PathBuf,
    /// How many of `model.turns` are already on disk for the active
    /// conversation. Reset when the conversation id changes.
    persisted_turns: usize,
    persisted_conversation: String,
    outcome_tx: Sender<TurnOutcome>,
    outcome_rx: Receiver<TurnOutcome>,
    /// Live deltas from the in-flight turn's worker thread.
    delta_rx: Option<Receiver<StreamDelta>>,
}

impl AssistantApp {
    /// Open the Assistant for a workspace: resume the persisted active
    /// conversation, or create a fresh one.
    pub fn new(workspace_root: PathBuf, broker: Arc<dyn AiBroker>) -> Self {
        let store = AssistantStore::new(&workspace_root);
        let model = match store.active_conversation() {
            Some(id) => {
                let turns = store.load_turns(&id);
                log::info!(
                    "assistant: resuming conversation {id} ({} turn(s)) for {}",
                    turns.len(),
                    workspace_root.display()
                );
                AssistantModel::resume(id, turns)
            }
            None => {
                let model = AssistantModel::fresh();
                log::info!(
                    "assistant: created conversation {} for {}",
                    model.conversation_id,
                    workspace_root.display()
                );
                model
            }
        };
        let persisted_turns = model.turns.len();
        let persisted_conversation = model.conversation_id.clone();
        let (outcome_tx, outcome_rx) = std::sync::mpsc::channel();
        let mut app = Self {
            model,
            store,
            broker,
            workspace_root,
            persisted_turns,
            persisted_conversation,
            outcome_tx,
            outcome_rx,
            delta_rx: None,
        };
        // Persist the active id immediately so close-then-reopen resumes
        // this conversation even before the first turn.
        app.session_write();
        app
    }

    /// Execute the effects a model transition returned.
    fn execute_effects(&mut self, effects: Vec<AssistantEffect>) {
        for effect in effects {
            match effect {
                AssistantEffect::AiQuery {
                    conversation_id,
                    prompt,
                } => self.start_turn(conversation_id, prompt),
                AssistantEffect::SessionWrite { .. } => self.session_write(),
                // Phase 2 stubs: correctly shaped, logged, never panic.
                AssistantEffect::ToolCall { tool, .. } => {
                    log::info!("assistant: ToolCall '{tool}' not yet implemented (Phase 2)");
                }
                AssistantEffect::PaneAction { action } => {
                    log::info!("assistant: PaneAction '{action}' not yet implemented (Phase 2)");
                }
                AssistantEffect::PermissionPrompt { target } => {
                    log::info!(
                        "assistant: PermissionPrompt '{target}' not yet implemented (Phase 2)"
                    );
                }
            }
        }
    }

    /// Persist the active conversation id and any turns not yet on disk.
    fn session_write(&mut self) {
        if self.persisted_conversation != self.model.conversation_id {
            self.persisted_conversation = self.model.conversation_id.clone();
            self.persisted_turns = 0;
        }
        if let Err(e) = self
            .store
            .set_active_conversation(&self.model.conversation_id)
        {
            log::error!("assistant: failed to persist active conversation: {e}");
        }
        for turn in &self.model.turns[self.persisted_turns.min(self.model.turns.len())..] {
            if let Err(e) = self.store.append_turn(&self.model.conversation_id, turn) {
                log::error!(
                    "assistant[{}]: failed to persist turn: {e}",
                    self.model.conversation_id
                );
            }
        }
        self.persisted_turns = self.model.turns.len();
    }

    /// Conversation history for the broker: user/assistant turns only.
    fn history_messages(&self) -> Vec<AiMessage> {
        self.model
            .turns
            .iter()
            .filter_map(|turn| match turn.role {
                TurnRole::User => Some(AiMessage {
                    role: "user".to_string(),
                    content: turn.text.clone(),
                }),
                TurnRole::Assistant => Some(AiMessage {
                    role: "assistant".to_string(),
                    content: turn.text.clone(),
                }),
                TurnRole::Tool | TurnRole::Error => None,
            })
            .collect()
    }

    /// Run one model turn on a worker thread (the broker blocks on network).
    fn start_turn(&mut self, conversation_id: String, _prompt: String) {
        let (delta_tx, delta_rx) = std::sync::mpsc::channel();
        self.delta_rx = Some(delta_rx);
        let request = AiBrokerRequest {
            app_id: "assistant".to_string(),
            model_tier: ModelTier::Medium,
            system: ASSISTANT_SYSTEM_PROMPT.to_string(),
            messages: self.history_messages(),
            tools: Vec::new(),
            workspace_root: Some(self.workspace_root.clone()),
            open_panes: crate::plexi_ai::broker::get_pane_snapshot(),
            tool_dispatcher: None,
            stream_sink: Some(StreamSink(delta_tx)),
        };
        log::info!(
            "assistant[{conversation_id}]: dispatching turn ({} message(s))",
            request.messages.len()
        );
        let broker = Arc::clone(&self.broker);
        let outcome_tx = self.outcome_tx.clone();
        let spawn = std::thread::Builder::new()
            .name("assistant-turn".to_string())
            .spawn(move || {
                let resp = broker.dispatch(request);
                let _ = outcome_tx.send(TurnOutcome {
                    conversation_id,
                    text: resp.content,
                    error: resp.error,
                });
            });
        if let Err(e) = spawn {
            log::error!("assistant: failed to spawn turn thread: {e}");
            let effects = self.model.finish_turn(
                &self.model.conversation_id.clone(),
                Err(format!("failed to spawn turn thread: {e}")),
            );
            self.execute_effects(effects);
        }
    }

    /// Per-frame pump: apply live stream deltas, then collect finished turns.
    fn pump_turn_io(&mut self) {
        if let Some(rx) = &self.delta_rx {
            let deltas: Vec<StreamDelta> = rx.try_iter().collect();
            for delta in deltas {
                match delta {
                    StreamDelta::Answer(chunk) => self.model.apply_answer_delta(&chunk),
                    StreamDelta::Reasoning(chunk) => self.model.apply_reasoning_delta(&chunk),
                }
            }
        }
        while let Ok(outcome) = self.outcome_rx.try_recv() {
            self.delta_rx = None;
            let result = match (outcome.text, outcome.error) {
                (Some(text), _) => Ok(text),
                (None, Some(error)) => Err(error),
                (None, None) => Err("broker returned neither content nor error".to_string()),
            };
            let effects = self.model.finish_turn(&outcome.conversation_id, result);
            self.execute_effects(effects);
        }
    }
}

impl App for AssistantApp {
    fn type_id(&self) -> &'static str {
        "assistant"
    }

    fn display_name(&self) -> String {
        "Assistant".to_string()
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        self.pump_turn_io();
        if self.model.streaming.in_flight {
            // Keep frames coming while a worker thread streams a turn.
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }
        let event = AssistantRenderer::draw(ui, &mut self.model, ctx.colors, ctx.is_focused);
        if let Some(ComposerEvent::Submit) = event {
            let effects = self.model.submit();
            self.execute_effects(effects);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plexi_ai::broker::AiBrokerResponse;

    /// Test broker: echoes a canned reply and streams deltas into the sink.
    struct EchoBroker;

    impl AiBroker for EchoBroker {
        fn dispatch(&self, request: AiBrokerRequest) -> AiBrokerResponse {
            if let Some(sink) = &request.stream_sink {
                let _ = sink.0.send(StreamDelta::Reasoning("pondering".to_string()));
                let _ = sink.0.send(StreamDelta::Answer("echo: ".to_string()));
                let _ = sink.0.send(StreamDelta::Answer("ok".to_string()));
            }
            AiBrokerResponse::ok_with_deltas(
                "echo: ok".to_string(),
                1,
                1,
                vec!["echo: ".to_string(), "ok".to_string()],
            )
        }
    }

    fn wait_for_turn(app: &mut AssistantApp) {
        let start = std::time::Instant::now();
        while app.model.streaming.in_flight {
            app.pump_turn_io();
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "turn never completed"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn full_turn_streams_deltas_and_persists() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = AssistantApp::new(ws.path().to_path_buf(), Arc::new(EchoBroker));
        let conversation_id = app.model.conversation_id.clone();

        app.model.composer = "hello".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        wait_for_turn(&mut app);

        assert_eq!(app.model.turns.len(), 2);
        assert_eq!(app.model.turns[1].role, TurnRole::Assistant);
        assert_eq!(app.model.turns[1].text, "echo: ok");

        // Close-and-reopen resumes the same conversation from disk.
        drop(app);
        let reopened = AssistantApp::new(ws.path().to_path_buf(), Arc::new(EchoBroker));
        assert_eq!(reopened.model.conversation_id, conversation_id);
        assert_eq!(reopened.model.turns.len(), 2);
        assert_eq!(reopened.model.turns[1].text, "echo: ok");
    }

    #[test]
    fn phase2_effect_stubs_execute_without_panicking() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = AssistantApp::new(ws.path().to_path_buf(), Arc::new(EchoBroker));
        app.execute_effects(vec![
            AssistantEffect::ToolCall {
                conversation_id: "c".to_string(),
                tool: "csv.read_range".to_string(),
                input_json: "{}".to_string(),
            },
            AssistantEffect::PaneAction {
                action: "focus:1".to_string(),
            },
            AssistantEffect::PermissionPrompt {
                target: "app_connector:app.csv.write_range".to_string(),
            },
        ]);
        assert!(!app.model.streaming.in_flight);
    }

    #[test]
    fn clear_keeps_prior_transcript_resumable_on_disk() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = AssistantApp::new(ws.path().to_path_buf(), Arc::new(EchoBroker));
        let first_id = app.model.conversation_id.clone();

        app.model.composer = "first question".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        wait_for_turn(&mut app);

        app.model.composer = "/clear".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        let second_id = app.model.conversation_id.clone();
        assert_ne!(second_id, first_id);

        // The prior conversation is still fully on disk.
        let prior = app.store.load_turns(&first_id);
        assert_eq!(prior.len(), 2);
        // The new conversation is the persisted active one.
        assert_eq!(app.store.active_conversation().as_deref(), Some(second_id.as_str()));
    }

    /// The Tool row role renders through the same Turn shape (Phase 2 seam).
    #[test]
    fn store_api_uses_model_turn_type() {
        let turn = crate::assistant::model::Turn::now(TurnRole::Tool, "tool output");
        assert_eq!(turn.role, TurnRole::Tool);
    }
}
