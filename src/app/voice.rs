//! Host-owned voice session. Enable grants only the enumerated pane/app opens;
//! Jev has no ambient Assistant identity or executable command surface.
use super::PlexiApp;
use crate::voice::{
    self,
    decisions::{Action, Candidate},
    runtime::Worker,
    Origin, Session, Utterance,
};
use std::{
    sync::{atomic::Ordering, mpsc},
    time::Instant,
};

struct DecisionResult {
    utterance: Utterance,
    result: Result<Option<Action>, String>,
}

#[derive(Default)]
pub(crate) struct Voice {
    pub session: Session,
    #[cfg(test)]
    pub fixture: bool,
    worker: Option<Worker>,
    retired: Vec<Worker>,
    decision: Option<mpsc::Receiver<DecisionResult>>,
    decision_thread: Option<std::thread::JoinHandle<()>>,
    retired_decisions: Vec<std::thread::JoinHandle<()>>,
}

impl PlexiApp {
    fn voice_origin(&self) -> Option<Origin> {
        let window = self.windows.get(self.active_window)?;
        let pane = window
            .focused_pane
            .and_then(|tile| window.tree.tiles.get(tile))
            .and_then(|tile| {
                if let egui_tiles::Tile::Pane(id) = tile {
                    Some(*id)
                } else {
                    None
                }
            })?;
        let context = self
            .router
            .iter()
            .find(|c| c.context_id == window.context_id)?;
        Some(Origin {
            window: window.window_id,
            context: window.context_id,
            pane,
            workspace: context.root.clone(),
        })
    }

    fn voice_origin_exists(&self, origin: &Origin) -> bool {
        self.windows.iter().any(|window| {
            window.window_id == origin.window
                && window.context_id == origin.context
                && window.panes.contains_key(&origin.pane)
        }) && self
            .router
            .iter()
            .any(|c| c.context_id == origin.context && c.root == origin.workspace)
    }

    pub(crate) fn start_voice(&mut self) -> Result<(), String> {
        if self.voice.session.status.enabled {
            return Ok(());
        }
        #[cfg(test)]
        if self.voice.fixture {
            self.voice.session.start();
            self.voice.session.status.listening = true;
            self.voice.session.status.outcome = "Listening for one command at a time".into();
            return Ok(());
        }
        self.voice
            .retired
            .retain(|worker| !worker.thread.is_finished());
        self.voice
            .retired_decisions
            .retain(|worker| !worker.is_finished());
        if !self.voice.retired.is_empty() || !self.voice.retired_decisions.is_empty() {
            return Err("Previous voice model is still stopping; try again shortly".into());
        }
        self.config.voice.validate()?;
        if self.config.voice.model_path.is_none() {
            return Err(
                "Set voice.model_path to an extracted Parakeet v3 int8 model directory".into(),
            );
        }
        let origin = self
            .voice_origin()
            .ok_or("Open a pane before enabling voice mode")?;
        let candidates = self.voice_candidates(&origin)?;
        self.voice.session.start();
        self.voice.session.status.supported_apps = candidates
            .iter()
            .filter(|c| c.action.placement == "right")
            .map(|c| (c.action.app.clone(), c.action.name.clone()))
            .collect();
        match Worker::spawn(
            self.config.voice.clone(),
            self.voice.session.generation,
            origin,
            self.ctx.clone(),
        ) {
            Ok(worker) => self.voice.worker = Some(worker),
            Err(error) => {
                self.voice.session.stop();
                return Err(error);
            }
        }
        log::info!("voice: enabled generation={} authority=single_pane_app_opens transcript_destination=openrouter", self.voice.session.generation);
        self.ctx.request_repaint();
        Ok(())
    }

    pub(crate) fn stop_voice(&mut self) {
        self.voice.session.stop();
        self.voice.decision = None;
        if let Some(worker) = self.voice.decision_thread.take() {
            self.voice.retired_decisions.push(worker);
        }
        if let Some(worker) = self.voice.worker.take() {
            worker.cancel.store(true, Ordering::Release);
            self.voice.retired.push(worker);
        }
        log::info!("voice: disabled; pending utterances and decisions cancelled");
        self.ctx.request_repaint();
    }

    fn voice_candidates(&mut self, origin: &Origin) -> Result<Vec<Candidate>, String> {
        if !self.voice_origin_exists(origin) {
            return Err("Voice origin pane/context no longer exists".into());
        }
        let registry = self.registries.view_for_root(&origin.workspace);
        let mut apps = Vec::new();
        for id in &self.config.voice.apps {
            if id == "text-editor" {
                apps.push((id.clone(), "Notes".into()));
            } else {
                let app = registry.get(id).ok_or_else(|| {
                    format!("Configured voice app '{id}' is unavailable in this context")
                })?;
                if app.source == crate::app::registry::RegistrySource::LocalAgent {
                    return Err(format!("Voice cannot launch agent '{id}'"));
                }
                apps.push((id.clone(), app.manifest.name.clone()));
            }
        }
        voice::decisions::candidates(apps)
    }

    fn execute_voice(&mut self, utterance: &Utterance, action: &Action) -> Result<String, String> {
        if !self.voice.session.status.enabled
            || self.voice.session.generation != utterance.generation
        {
            return Err("Voice session cancelled".into());
        }
        if !self
            .voice_candidates(&utterance.origin)?
            .iter()
            .any(|c| c.action == *action)
        {
            return Err("App is no longer available in the origin context".into());
        }
        use crate::broker::{
            ActorType, Decision, GrantStore, PermissionPosture, PermissionRequest, TargetType,
        };
        let request = PermissionRequest::new(
            ActorType::System,
            "voice",
            TargetType::HostTool,
            "host.panes.open",
            Some(&utterance.origin.workspace),
        );
        // The explicit enable action supplies a session posture only. Persisted
        // user/managed deny/ask records still win; nothing is saved as a grant.
        let posture = PermissionPosture {
            default_posture: Decision::Deny,
            allow: vec!["host.panes.open".into()],
            ask: vec![],
            deny: vec![],
        };
        let grants = GrantStore::load_or_default(&self.permission_store_dir);
        if grants.evaluate(&request, Some(&posture)) != Decision::Allow {
            return Err("Voice pane opening is blocked by permission policy".into());
        }
        let started = Instant::now();
        let layout = if action.placement == "down" {
            "split_down"
        } else {
            "split_right"
        };
        let pane = self.spawn_host_pane(
            utterance.origin.pane,
            utterance.origin.context,
            &action.app,
            Some(layout.into()),
            vec![],
            Some(utterance.origin.workspace.clone()),
            None,
        )?;
        let name = if action.app == "terminal" {
            "Terminal".to_string()
        } else {
            action.name.clone()
        };
        self.handle_pane_ipc_request(crate::protocol::AppRequest::SetPaneTitle {
            pane_id: pane,
            name: name.clone(),
        });
        log::info!("voice: executed generation={} utterance={} pane={} action={} placement={} host_ms={} end_to_action_ms={} utterance_age_ms={}",
            utterance.generation, utterance.id, pane, action.app, action.placement, started.elapsed().as_millis(),
            utterance.finalized.elapsed().as_millis(), utterance.started.elapsed().as_millis());
        Ok(format!("Opened {name}"))
    }

    /// Called from the off-paint preamble. No model, audio-device, or HTTP work
    /// occurs here. A pending decision never prevents draining new speech.
    pub(crate) fn service_voice(&mut self) {
        self.voice
            .retired
            .retain(|worker| !worker.thread.is_finished());
        self.voice
            .retired_decisions
            .retain(|worker| !worker.is_finished());
        let origin = self.voice_origin();
        if let Some(worker) = &self.voice.worker {
            if let Ok(mut shared) = worker.mailbox.try_lock() {
                // Origin is sampled at VAD onset against the latest host pass.
                shared.origin = origin;
                let status = &mut self.voice.session.status;
                if shared.listening && !status.listening {
                    status.outcome = "Listening for one command at a time".into();
                }
                status.listening = shared.listening;
                status.microphone = shared.microphone.clone();
                status.partial.clone_from(&shared.partial);
                status.rejected += std::mem::take(&mut shared.rejected);
                if let Some(error) = shared.error.take() {
                    status.outcome = error;
                }
                for utterance in shared.finals.drain(..) {
                    if let Err(error) = self.voice.session.enqueue(utterance) {
                        self.voice.session.status.outcome = error.into();
                    }
                }
            } else {
                self.ctx
                    .request_repaint_after(std::time::Duration::from_millis(1));
            }
        }
        if self
            .voice
            .worker
            .as_ref()
            .is_some_and(|worker| worker.thread.is_finished())
        {
            let error = self.voice.session.status.outcome.clone();
            self.stop_voice();
            self.voice.session.status.outcome = error;
        }
        let completed =
            self.voice
                .decision
                .as_ref()
                .and_then(|receiver| match receiver.try_recv() {
                    Ok(result) => Some(Ok(result)),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err("Jev worker ended without a response".to_string()))
                    }
                    Err(mpsc::TryRecvError::Empty) => None,
                });
        if let Some(completed) = completed {
            self.voice.decision = None;
            match completed {
                Ok(completed) => {
                    let outcome = match completed.result {
                        Ok(Some(action)) => self.execute_voice(&completed.utterance, &action),
                        Ok(None) => Ok("No command".into()),
                        Err(error) => Err(error),
                    };
                    if let Err(error) = &outcome {
                        log::info!(
                            "voice: refused utterance={} reason={error}",
                            completed.utterance.id
                        );
                    }
                    self.voice.session.complete(
                        completed.utterance.generation,
                        completed.utterance.id,
                        outcome.unwrap_or_else(|e| e),
                    );
                }
                Err(error) => {
                    self.voice.session.status.processing = None;
                    self.voice.session.status.outcome = error;
                }
            }
        }
        if let Some(utterance) = self.voice.session.next() {
            let candidates = match self.voice_candidates(&utterance.origin) {
                Ok(candidates) => candidates,
                Err(error) => {
                    self.voice
                        .session
                        .complete(utterance.generation, utterance.id, error);
                    self.ctx
                        .request_repaint_after(std::time::Duration::from_millis(1));
                    return;
                }
            };
            let threshold = self.config.voice.confidence_threshold;
            let key_env = self
                .config
                .ai
                .as_ref()
                .and_then(|ai| ai.openrouter.as_ref())
                .and_then(|or| or.api_key_env.clone())
                .unwrap_or_else(|| "OPENROUTER_API_KEY".into());
            let (sender, receiver) = mpsc::sync_channel(1);
            let wake = self.ctx.clone();
            let cancel = self.voice.worker.as_ref().map(|w| w.cancel.clone());
            let generation = utterance.generation;
            let id = utterance.id;
            match std::thread::Builder::new()
                .name("plexi-voice-jev".into())
                .spawn(move || {
                    let started = Instant::now();
                    let result = crate::plexi_ai::broker::resolve_openrouter_api_key(
                        &key_env,
                        Some(&utterance.origin.workspace),
                    )
                    .and_then(|key| {
                        if cancel.as_ref().is_some_and(|c| c.load(Ordering::Acquire)) {
                            return Err("Voice cancelled".into());
                        }
                        voice::decisions::decide(&utterance.text, &candidates, threshold, &key)
                    });
                    log::info!(
                        "voice: interpreted generation={generation} utterance={id} jev_ms={}",
                        started.elapsed().as_millis()
                    );
                    let _ = sender.send(DecisionResult { utterance, result });
                    wake.request_repaint();
                }) {
                Ok(worker) => {
                    self.voice.decision = Some(receiver);
                    self.voice.decision_thread = Some(worker);
                }
                Err(error) => {
                    self.voice.session.complete(
                        generation,
                        id,
                        format!("Start Jev worker: {error}"),
                    );
                    self.ctx
                        .request_repaint_after(std::time::Duration::from_millis(1));
                }
            }
        }
    }

    pub(crate) fn draw_voice_status(&mut self, ui: &mut egui::Ui) {
        if !self.voice.session.status.enabled && self.voice.session.status.outcome.is_empty() {
            return;
        }
        let mut stop = false;
        egui::Panel::bottom("voice_status").resizable(false).show_inside(ui, |ui| {
            let status = &self.voice.session.status;
            ui.horizontal_wrapped(|ui| {
                crate::ui::typography::body_strong(ui, if status.listening { "Voice · Listening" } else if status.enabled { "Voice · Starting" } else { "Voice · Off" }, &self.colors);
                if status.processing.is_some() { crate::ui::typography::body(ui, "Interpreting", &self.colors); }
                if status.queued > 0 { crate::ui::typography::caption(ui, format!("{} queued", status.queued), &self.colors); }
                if let Some(input) = &status.microphone { crate::ui::typography::caption(ui, input, &self.colors); }
                if status.enabled { stop = crate::ui::button::chrome_button(ui, "Stop listening", crate::ui::button::ButtonKind::Primary, &self.colors, 0.0).clicked(); }
                else if crate::ui::button::chrome_button(ui, "Dismiss", crate::ui::button::ButtonKind::Secondary, &self.colors, 0.0).clicked() { stop = true; }
            });
            if !status.partial.is_empty() { crate::ui::typography::body(ui, &status.partial, &self.colors); }
            crate::ui::typography::caption(ui, &status.outcome, &self.colors);
            if status.enabled { crate::ui::typography::caption(ui, "Audio stays on this device · Finalized speech is sent to OpenRouter · Commands run automatically", &self.colors); }
        });
        if stop {
            if self.voice.session.status.enabled {
                self.stop_voice();
            } else {
                self.voice.session.status.outcome.clear();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_placement_matches_rendered_right_and_below_geometry() {
        for placement in ["right", "down"] {
            let mut harness = crate::testing::HostHarness::new();
            let pane = harness.add_test_pane();
            harness.app.pane_navigate(pane);
            let origin = harness.app.voice_origin().unwrap();
            harness.app.voice.session.start();
            let utterance = Utterance {
                generation: 1,
                id: 1,
                origin,
                text: "Open Notes".into(),
                started: Instant::now(),
                finalized: Instant::now(),
            };
            harness
                .app
                .execute_voice(
                    &utterance,
                    &Action {
                        app: "text-editor".into(),
                        name: "Notes".into(),
                        placement,
                    },
                )
                .unwrap();
            harness.run_frames(2);
            let (window, tile) = harness.app.find_pane_in_any_window(pane).unwrap();
            let state = &harness.app.windows[window];
            let origin_rect = state.tree.tiles.rect(tile).unwrap();
            let new_tile = state
                .tree
                .tiles
                .iter()
                .find_map(|(id, tile)| {
                    matches!(tile, egui_tiles::Tile::Pane(id) if *id != pane).then_some(*id)
                })
                .unwrap();
            let new_rect = state.tree.tiles.rect(new_tile).unwrap();
            if placement == "right" {
                assert!(
                    new_rect.left() >= origin_rect.right(),
                    "right: origin={origin_rect:?}, new={new_rect:?}"
                );
                assert!((new_rect.top() - origin_rect.top()).abs() < 1.0);
            } else {
                assert!(
                    new_rect.top() >= origin_rect.bottom(),
                    "below: origin={origin_rect:?}, new={new_rect:?}"
                );
                assert!((new_rect.left() - origin_rect.left()).abs() < 1.0);
            }
        }
    }

    #[test]
    fn voice_completion_dispatches_without_a_visible_frame() {
        let mut harness = crate::testing::HostHarness::new();
        let pane = harness.add_test_pane();
        harness.app.pane_navigate(pane);
        let origin = harness.app.voice_origin().unwrap();
        harness.app.voice.session.start();
        harness.app.voice.session.status.processing = Some(1);
        let utterance = Utterance {
            generation: 1,
            id: 1,
            origin: origin.clone(),
            text: "Open Notes".into(),
            started: Instant::now(),
            finalized: Instant::now(),
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        harness.app.voice.decision = Some(receiver);
        sender
            .send(DecisionResult {
                utterance,
                result: Ok(Some(Action {
                    app: "text-editor".into(),
                    name: "Notes".into(),
                    placement: "right",
                })),
            })
            .unwrap();
        harness.hidden_frame();
        assert!(harness.app.voice.session.status.processing.is_none());
        assert_eq!(harness.app.voice.session.status.outcome, "Opened Notes");
    }

    #[test]
    fn stopped_session_and_stale_origins_refuse_mutation() {
        let mut harness = crate::testing::HostHarness::new();
        let pane = harness.add_test_pane();
        harness.app.pane_navigate(pane);
        let mut origin = harness.app.voice_origin().unwrap();
        harness.app.voice.session.start();
        origin.pane += 1000;
        let utterance = Utterance {
            generation: 1,
            id: 1,
            origin,
            text: "Open Notes".into(),
            started: Instant::now(),
            finalized: Instant::now(),
        };
        let action = Action {
            app: "text-editor".into(),
            name: "Notes".into(),
            placement: "right",
        };
        assert!(harness.app.execute_voice(&utterance, &action).is_err());
        harness.app.stop_voice();
        assert!(harness.app.execute_voice(&utterance, &action).is_err());
    }

    #[test]
    fn voice_start_is_idempotent_and_does_not_restart_microphone() {
        let mut harness = crate::testing::HostHarness::new();
        harness.app.voice.session.start();
        harness.app.voice.session.status.processing = Some(7);
        assert!(harness.app.start_voice().is_ok());
        assert_eq!(harness.app.voice.session.generation, 1);
        assert_eq!(harness.app.voice.session.status.processing, Some(7));
    }

    #[test]
    fn stale_queued_origin_schedules_progress_for_the_next_command() {
        let mut harness = crate::testing::HostHarness::new();
        let pane = harness.add_test_pane();
        harness.app.pane_navigate(pane);
        let origin = harness.app.voice_origin().unwrap();
        harness.app.voice.session.start();
        let mut stale = origin.clone();
        stale.pane += 1000;
        for (id, origin) in [(1, stale), (2, origin)] {
            harness
                .app
                .voice
                .session
                .enqueue(Utterance {
                    generation: 1,
                    id,
                    origin,
                    text: "Open Notes".into(),
                    started: Instant::now(),
                    finalized: Instant::now(),
                })
                .unwrap();
        }
        harness.hidden_frame();
        assert_eq!(harness.app.voice.session.status.queued, 1);
        assert!(harness.app.ctx.has_requested_repaint());
        assert_eq!(harness.app.voice.session.next().unwrap().id, 2);
    }

    #[test]
    fn voice_missing_model_fails_without_starting_capture() {
        let mut harness = crate::testing::HostHarness::new();
        harness.add_test_pane();
        assert!(harness
            .app
            .start_voice()
            .unwrap_err()
            .contains("model_path"));
        assert!(!harness.app.voice.session.status.enabled);
        assert!(harness.app.voice.worker.is_none());
    }
}
