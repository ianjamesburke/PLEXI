//! Chess tools on the host tool registry, with scoped events on `AppTimeline`.
//!
//! `dispatch` goes through `ToolDispatcher::dispatch_call`. The actor is the
//! caller id the dispatcher stamps. Tool arguments cannot name an actor.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde_json::json;

use crate::broker::{ActorType, GrantDuration};
use crate::host::app_timeline::{AppTimeline, EmittedEvent, SubscriptionRecord};
use crate::host::scope::ScopeOrigin;
use crate::plexi_ai::tool_dispatch::{self, AppEventSender, ToolCallResult, ToolDispatcher};
use crate::protocol::{
    AiTool, AppEventActor, EventStreamDecl, PayloadMode, PlexiEvent, TriggerMode,
};

use super::chess::{
    op_key, ActorGrant, ChessError, ChessJournal, ChessStore, MoveReceipt, OpKind, PlayResult,
    StateView,
};
use super::contracts::{AppInstanceRecord, BackgroundPolicy, ViewState, SCHEMA_VERSION};

static NEXT_ID: AtomicU64 = AtomicU64::new(250_000);

struct ChessInner {
    pane_id: u64,
    context_id: u64,
    instance_id: String,
    journal_path: PathBuf,
    store: Mutex<ChessStore>,
    timeline: Mutex<AppTimeline>,
    running: AtomicBool,
    policy: Mutex<BackgroundPolicy>,
    view: Mutex<ViewState>,
    #[cfg(test)]
    skip_publish: AtomicBool,
    #[cfg(test)]
    fail_persist: AtomicBool,
}

pub struct ChessApp {
    inner: Arc<ChessInner>,
    pane_id: u64,
}

impl ChessApp {
    pub fn create(dir: &Path, policy: BackgroundPolicy) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|error| format!("chess journal dir: {error}"))?;
        let path = journal_path(dir);
        if path.exists() {
            return Err("chess journal already exists".into());
        }
        let context_id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        let instance_id = format!("chess-instance-{context_id}");
        let app = Self::boot(
            dir,
            context_id,
            instance_id,
            ChessStore::new(),
            policy,
            ViewState::NoView {
                observed_at: "2030-01-01T00:00:00Z".into(),
            },
            true,
        )?;
        app.persist_loaded()?;
        Ok(app)
    }

    pub fn open(dir: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(journal_path(dir))
            .map_err(|error| format!("chess journal read: {error}"))?;
        let saved: SavedJournal = serde_json::from_slice(&bytes)
            .map_err(|error| format!("chess journal parse: {error}"))?;
        let store = ChessStore::from_journal(saved.chess)?;
        let app = Self::boot(
            dir,
            saved.context_id,
            saved.instance_id,
            store,
            saved.policy,
            saved.view,
            saved.running,
        )?;
        log::info!(
            "cloud_assistant: chess reopened instance={} context={} unpublished={}",
            app.instance_id(),
            app.context_id(),
            app.inner.store.lock().expect("store").unpublished().len()
        );
        Ok(app)
    }

    fn boot(
        dir: &Path,
        context_id: u64,
        instance_id: String,
        store: ChessStore,
        policy: BackgroundPolicy,
        view: ViewState,
        running: bool,
    ) -> Result<Self, String> {
        let pane_id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        let inner = Arc::new(ChessInner {
            pane_id,
            context_id,
            instance_id,
            journal_path: journal_path(dir),
            store: Mutex::new(store),
            timeline: Mutex::new(AppTimeline::default()),
            running: AtomicBool::new(running),
            policy: Mutex::new(policy),
            view: Mutex::new(view),
            #[cfg(test)]
            skip_publish: AtomicBool::new(false),
            #[cfg(test)]
            fail_persist: AtomicBool::new(false),
        });
        declare_streams(&inner);
        let app = Self { inner, pane_id };
        app.reregister();
        log::info!(
            "cloud_assistant: chess instance {} registered pane={pane_id} context={context_id}",
            app.instance_id()
        );
        Ok(app)
    }

    pub fn context_id(&self) -> u64 {
        self.inner.context_id
    }

    pub fn pane_id(&self) -> u64 {
        self.pane_id
    }

    pub fn instance_id(&self) -> String {
        self.inner.instance_id.clone()
    }

    pub fn running(&self) -> bool {
        self.inner.running.load(Ordering::SeqCst)
    }

    pub fn view(&self) -> ViewState {
        self.inner.view.lock().expect("view").clone()
    }

    pub fn provision(&self, game_id: &str, grants: Vec<ActorGrant>) -> Result<(), String> {
        let journal = {
            let mut store = self.inner.store.lock().expect("store");
            store.provision(game_id, grants);
            store.to_journal()
        };
        write_journal(&self.inner.journal_head(), &journal)
    }

    pub fn observe(&self, view: ViewState) -> Result<(), String> {
        *self.inner.view.lock().expect("view") = view;
        log::info!(
            "cloud_assistant: chess view observed instance={} running={}",
            self.instance_id(),
            self.running()
        );
        self.persist_loaded()
    }

    pub fn close_view(&self) -> Result<(), String> {
        let policy = *self.inner.policy.lock().expect("policy");
        *self.inner.view.lock().expect("view") = ViewState::NoView {
            observed_at: "2030-01-01T00:00:00Z".into(),
        };
        if policy == BackgroundPolicy::StopOnLastViewClose {
            self.inner.running.store(false, Ordering::SeqCst);
            log::info!(
                "cloud_assistant: chess instance {} stopped because its last view closed",
                self.instance_id()
            );
        } else {
            log::info!(
                "cloud_assistant: chess instance {} detached its view and kept running",
                self.instance_id()
            );
        }
        self.reregister();
        self.persist_loaded()
    }

    pub fn dispatch(&self, actor_id: &str, tool: &str, args: serde_json::Value) -> ToolCallResult {
        let dispatcher = ToolDispatcher::from_registry(
            self.pane_id,
            actor_id.to_string(),
            self.inner.context_id,
        );
        let input = args.to_string();
        dispatcher.dispatch_call(format!("call-{}", uuid::Uuid::new_v4()), tool, input)
    }

    pub fn dispatch_in_context(
        &self,
        actor_id: &str,
        viewer_context: u64,
        tool: &str,
        args: serde_json::Value,
    ) -> ToolCallResult {
        let dispatcher =
            ToolDispatcher::from_registry(self.pane_id, actor_id.to_string(), viewer_context);
        dispatcher.dispatch_call(
            format!("call-{}", uuid::Uuid::new_v4()),
            tool,
            args.to_string(),
        )
    }

    pub fn subscribe(&self, subscriber_id: &str, game_id: &str, subscriber_context: u64) {
        self.inner
            .timeline
            .lock()
            .expect("timeline")
            .add_subscription(SubscriptionRecord {
                subscription_id: format!("sub-{subscriber_id}-{game_id}"),
                subscriber_type: ActorType::Agent,
                subscriber_id: subscriber_id.to_string(),
                app_id: "chess".into(),
                event_names: vec!["chess.move_committed".into(), "chess.game_reset".into()],
                payload_mode: PayloadMode::Full,
                trigger_mode: TriggerMode::Ambient,
                resource_id: Some(game_id.to_string()),
                duration: GrantDuration::Game,
                subscriber_context_id: subscriber_context,
                created_at: "2030-01-01T00:00:00Z".into(),
            });
    }

    pub fn take_deliveries(
        &self,
        subscriber_id: &str,
    ) -> Vec<crate::host::app_timeline::EventDelivery> {
        self.inner
            .timeline
            .lock()
            .expect("timeline")
            .take_deliveries_for(ActorType::Agent, subscriber_id)
    }

    pub fn publish_pending(&self) -> Result<usize, String> {
        let pending: Vec<(String, MoveReceipt)> = {
            let store = self.inner.store.lock().expect("store");
            store
                .unpublished()
                .iter()
                .filter_map(|key| {
                    store
                        .receipt(key)
                        .cloned()
                        .map(|receipt| (key.clone(), receipt))
                })
                .collect()
        };
        let mut published = 0usize;
        for (key, receipt) in pending {
            self.emit(&receipt)?;
            let journal = {
                let mut store = self.inner.store.lock().expect("store");
                store.mark_published(&key);
                store.to_journal()
            };
            write_journal(&self.inner.journal_head(), &journal)?;
            published += 1;
        }
        if published > 0 {
            log::info!(
                "cloud_assistant: chess published {published} pending event(s) instance={}",
                self.instance_id()
            );
        }
        Ok(published)
    }

    pub fn record(&self) -> AppInstanceRecord {
        AppInstanceRecord {
            schema_version: SCHEMA_VERSION,
            instance_id: self.instance_id(),
            app_id: "chess".into(),
            resource_id: "chess".into(),
            background_policy: *self.inner.policy.lock().expect("policy"),
            view: self.view(),
        }
    }

    #[cfg(test)]
    pub fn arm_skip_publish(&self) {
        self.inner.skip_publish.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub fn arm_persist_failure(&self) {
        self.inner.fail_persist.store(true, Ordering::SeqCst);
    }

    fn reregister(&self) {
        let running = self.running();
        let tools = if running { tool_defs() } else { Vec::new() };
        let inner = Arc::clone(&self.inner);
        let sender = AppEventSender::InProcess(Arc::new(move |event| {
            inner.handle(event);
            Ok(())
        }));
        let root = self
            .inner
            .journal_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        tool_dispatch::register(
            self.pane_id,
            "chess".into(),
            tools,
            sender,
            ScopeOrigin {
                context_id: self.inner.context_id,
                context_root: root,
                window_id: 1,
                pane_id: self.pane_id,
                app_id: Some("chess".into()),
            },
        );
    }

    fn emit(&self, receipt: &MoveReceipt) -> Result<(), String> {
        let event_name = match receipt.kind {
            OpKind::Play => "chess.move_committed",
            OpKind::Reset => "chess.game_reset",
        };
        let payload = json!({
            "game_id": receipt.game_id,
            "revision_before": receipt.revision_before,
            "revision_after": receipt.revision_after,
            "uci": receipt.uci,
            "actor_id": receipt.actor_id,
            "operation_id": receipt.operation_id,
            "side_to_move": receipt.side_to_move.as_str(),
            "status": receipt.status.as_str(),
        });
        let summary = match receipt.kind {
            OpKind::Play => format!(
                "{} played {} on {} at revision {}",
                receipt.actor_id, receipt.uci, receipt.game_id, receipt.revision_after
            ),
            OpKind::Reset => format!(
                "{} reset {} at revision {}",
                receipt.actor_id, receipt.game_id, receipt.revision_after
            ),
        };
        self.inner
            .timeline
            .lock()
            .expect("timeline")
            .record_event(
                self.inner.context_id,
                "chess",
                self.pane_id,
                EmittedEvent {
                    event: event_name.into(),
                    actor: AppEventActor::Agent,
                    actor_id: Some(receipt.actor_id.clone()),
                    caused_by: Some(receipt.actor_id.clone()),
                    summary,
                    resource_id: receipt.game_id.clone(),
                    resource_scope: Some("game".into()),
                    revision_after: receipt.revision_after.to_string(),
                    payload: Some(payload),
                    state_ref: Some(format!(
                        "chess://game/{}/rev/{}",
                        receipt.game_id, receipt.revision_after
                    )),
                    revision_before: Some(receipt.revision_before.to_string()),
                    rollback_token: None,
                    changed_resources: vec![receipt.game_id.clone()],
                    suggested_trigger: Some(TriggerMode::Ambient),
                },
            )
            .map(|_| ())
    }

    fn persist_loaded(&self) -> Result<(), String> {
        let journal = self.inner.store.lock().expect("store").to_journal();
        write_journal(&self.inner.journal_head(), &journal)
    }
}

impl Drop for ChessApp {
    fn drop(&mut self) {
        // The in-process sender holds one extra `Arc`. Unregister when this
        // value and that sender are the only owners, so an `Arc<ChessApp>`
        // clone can finish a call already in flight.
        if Arc::strong_count(&self.inner) <= 2 {
            tool_dispatch::unregister(self.pane_id);
        }
    }
}

impl ChessInner {
    fn handle(&self, event: &PlexiEvent) {
        let PlexiEvent::ToolCall {
            call_id,
            name,
            input_json,
            caller_id,
        } = event
        else {
            return;
        };
        let result = self.handle_tool(caller_id, name, input_json);
        tool_dispatch::resolve_pending(call_id, result);
    }

    fn handle_tool(&self, actor_id: &str, name: &str, input_json: &str) -> ToolCallResult {
        if !self.running.load(Ordering::SeqCst) {
            log::info!("cloud_assistant: chess tool {name} rejected actor={actor_id} reason=instance_stopped");
            return ToolCallResult::err(ChessError::InstanceStopped.message());
        }
        let outcome = match name {
            "chess.state" => self.tool_state(actor_id, input_json),
            "chess.legal_moves" => self.tool_legal(actor_id, input_json),
            "chess.play" => self.tool_play(actor_id, input_json),
            "chess.new_game" => self.tool_reset(actor_id, input_json),
            other => Err(format!("invalid_input: {other}")),
        };
        match outcome {
            Ok(value) => ToolCallResult::ok_value(value),
            Err(message) => {
                log::info!(
                    "cloud_assistant: chess tool {name} rejected actor={actor_id} reason={message}"
                );
                ToolCallResult::err(message)
            }
        }
    }

    fn tool_state(&self, actor_id: &str, input_json: &str) -> Result<serde_json::Value, String> {
        let args: GameArgs = parse_args(input_json)?;
        let store = self.store.lock().expect("store");
        let view = store
            .state_for(actor_id, &args.game_id)
            .map_err(|error| error.message())?;
        Ok(state_json(&view))
    }

    fn tool_legal(&self, actor_id: &str, input_json: &str) -> Result<serde_json::Value, String> {
        let args: LegalArgs = parse_args(input_json)?;
        let store = self.store.lock().expect("store");
        let moves = store
            .legal_for(actor_id, &args.game_id, args.revision)
            .map_err(|error| error.message())?;
        Ok(json!({ "game_id": args.game_id, "revision": args.revision, "moves": moves }))
    }

    fn tool_play(&self, actor_id: &str, input_json: &str) -> Result<serde_json::Value, String> {
        let args: PlayArgs = parse_args(input_json)?;
        let head = self.journal_head();
        let (value, fresh) = {
            let mut store = self.store.lock().expect("store");
            let backup = store.clone();
            let played = store
                .play(
                    actor_id,
                    &args.game_id,
                    args.expected_revision,
                    &args.operation_id,
                    &args.uci,
                )
                .map_err(|error| error.message())?;
            if played.is_fresh() {
                if let Err(error) = write_journal(&head, &store.to_journal()) {
                    *store = backup;
                    return Err(error);
                }
            }
            (
                receipt_json(&played),
                played.is_fresh().then(|| played.receipt().clone()),
            )
        };
        if let Some(receipt) = fresh {
            self.publish_fresh(actor_id, &args.operation_id, &receipt);
            log::info!(
                "cloud_assistant: chess.play actor={actor_id} game={} op={} revision {}->{} uci={}",
                args.game_id,
                args.operation_id,
                receipt.revision_before,
                receipt.revision_after,
                args.uci
            );
        } else {
            log::info!(
                "cloud_assistant: chess.play replayed actor={actor_id} game={} op={}",
                args.game_id,
                args.operation_id
            );
        }
        Ok(value)
    }

    fn tool_reset(&self, actor_id: &str, input_json: &str) -> Result<serde_json::Value, String> {
        let args: ResetArgs = parse_args(input_json)?;
        let head = self.journal_head();
        let (value, fresh) = {
            let mut store = self.store.lock().expect("store");
            let backup = store.clone();
            let played = store
                .reset(
                    actor_id,
                    &args.game_id,
                    &args.operation_id,
                    args.fen.as_deref(),
                )
                .map_err(|error| error.message())?;
            if played.is_fresh() {
                if let Err(error) = write_journal(&head, &store.to_journal()) {
                    *store = backup;
                    return Err(error);
                }
            }
            (
                receipt_json(&played),
                played.is_fresh().then(|| played.receipt().clone()),
            )
        };
        if let Some(receipt) = fresh {
            self.publish_fresh(actor_id, &args.operation_id, &receipt);
            log::info!(
                "cloud_assistant: chess.new_game actor={actor_id} game={} op={} revision {}",
                args.game_id,
                args.operation_id,
                receipt.revision_after
            );
        }
        Ok(value)
    }

    fn publish_fresh(&self, actor_id: &str, operation_id: &str, receipt: &MoveReceipt) {
        if !self.publish_now() {
            return;
        }
        if let Err(error) = emit_of(self, receipt) {
            log::info!(
                "cloud_assistant: chess move committed but event deferred op={} error={error}",
                receipt.operation_id
            );
            return;
        }
        let journal = {
            let mut store = self.store.lock().expect("store");
            store.mark_published(&op_key(actor_id, operation_id));
            store.to_journal()
        };
        if let Err(error) = write_journal(&self.journal_head(), &journal) {
            log::info!(
                "cloud_assistant: chess event published but journal update failed op={operation_id} error={error}"
            );
        }
    }

    fn journal_head(&self) -> JournalHead {
        JournalHead {
            path: self.journal_path.clone(),
            instance_id: self.instance_id.clone(),
            context_id: self.context_id,
            policy: *self.policy.lock().expect("policy"),
            view: self.view.lock().expect("view").clone(),
            running: self.running.load(Ordering::SeqCst),
            #[cfg(test)]
            fail: self.fail_persist.load(Ordering::SeqCst),
        }
    }

    fn publish_now(&self) -> bool {
        #[cfg(test)]
        {
            !self.skip_publish.load(Ordering::SeqCst)
        }
        #[cfg(not(test))]
        {
            true
        }
    }
}

struct JournalHead {
    path: PathBuf,
    instance_id: String,
    context_id: u64,
    policy: BackgroundPolicy,
    view: ViewState,
    running: bool,
    #[cfg(test)]
    fail: bool,
}

fn write_journal(head: &JournalHead, chess: &ChessJournal) -> Result<(), String> {
    #[cfg(test)]
    if head.fail {
        return Err("persist_failed".into());
    }
    let saved = SavedJournal {
        instance_id: head.instance_id.clone(),
        context_id: head.context_id,
        policy: head.policy,
        view: head.view.clone(),
        running: head.running,
        chess: chess.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&saved).map_err(|error| error.to_string())?;
    write_atomic(&head.path, &bytes)
}

fn emit_of(inner: &ChessInner, receipt: &MoveReceipt) -> Result<(), String> {
    let event_name = match receipt.kind {
        OpKind::Play => "chess.move_committed",
        OpKind::Reset => "chess.game_reset",
    };
    inner
        .timeline
        .lock()
        .expect("timeline")
        .record_event(
            inner.context_id,
            "chess",
            inner.pane_id,
            EmittedEvent {
                event: event_name.into(),
                actor: AppEventActor::Agent,
                actor_id: Some(receipt.actor_id.clone()),
                caused_by: Some(receipt.actor_id.clone()),
                summary: format!(
                    "{} {} {} revision {}",
                    receipt.actor_id, event_name, receipt.game_id, receipt.revision_after
                ),
                resource_id: receipt.game_id.clone(),
                resource_scope: Some("game".into()),
                revision_after: receipt.revision_after.to_string(),
                payload: Some(json!({
                    "game_id": receipt.game_id,
                    "revision_before": receipt.revision_before,
                    "revision_after": receipt.revision_after,
                    "uci": receipt.uci,
                    "actor_id": receipt.actor_id,
                    "operation_id": receipt.operation_id,
                    "side_to_move": receipt.side_to_move.as_str(),
                    "status": receipt.status.as_str(),
                })),
                state_ref: Some(format!(
                    "chess://game/{}/rev/{}",
                    receipt.game_id, receipt.revision_after
                )),
                revision_before: Some(receipt.revision_before.to_string()),
                rollback_token: None,
                changed_resources: vec![receipt.game_id.clone()],
                suggested_trigger: Some(TriggerMode::Ambient),
            },
        )
        .map(|_| ())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GameArgs {
    game_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegalArgs {
    game_id: String,
    revision: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlayArgs {
    game_id: String,
    expected_revision: u64,
    operation_id: String,
    uci: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetArgs {
    game_id: String,
    operation_id: String,
    #[serde(default)]
    fen: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedJournal {
    instance_id: String,
    context_id: u64,
    policy: BackgroundPolicy,
    view: ViewState,
    running: bool,
    chess: ChessJournal,
}

fn parse_args<T: for<'de> Deserialize<'de>>(input_json: &str) -> Result<T, String> {
    serde_json::from_str(input_json).map_err(|error| format!("invalid_input: {error}"))
}

fn state_json(view: &StateView) -> serde_json::Value {
    json!({
        "game_id": view.game_id,
        "revision": view.revision,
        "generation": view.generation,
        "fen": view.fen,
        "side_to_move": view.side_to_move.as_str(),
        "status": view.status.as_str(),
        "history": view.history,
        "legal_moves": view.legal_moves,
    })
}

fn receipt_json(result: &PlayResult) -> serde_json::Value {
    let receipt = result.receipt();
    json!({
        "game_id": receipt.game_id,
        "revision": receipt.revision_after,
        "revision_before": receipt.revision_before,
        "operation_id": receipt.operation_id,
        "actor_id": receipt.actor_id,
        "uci": receipt.uci,
        "replayed": !result.is_fresh(),
        "side_to_move": receipt.side_to_move.as_str(),
        "status": receipt.status.as_str(),
        "fen": receipt.fen_after,
    })
}

fn journal_path(dir: &Path) -> PathBuf {
    dir.join("chess-journal.json")
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = std::fs::File::create(&tmp)
            .map_err(|error| format!("chess journal create: {error}"))?;
        use std::io::Write;
        file.write_all(bytes)
            .map_err(|error| format!("chess journal write: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("chess journal sync: {error}"))?;
    }
    std::fs::rename(&tmp, path).map_err(|error| format!("chess journal rename: {error}"))
}

fn declare_streams(inner: &ChessInner) {
    let schema = json!({
        "type": "object",
        "required": ["game_id", "revision_before", "revision_after", "actor_id", "operation_id", "side_to_move", "status"]
    });
    let _ = inner.timeline.lock().expect("timeline").declare_streams(
        inner.context_id,
        "chess",
        vec![
            EventStreamDecl {
                name: "chess.move_committed".into(),
                schema: schema.clone(),
                description: Some("A chess move committed in the app store".into()),
            },
            EventStreamDecl {
                name: "chess.game_reset".into(),
                schema,
                description: Some("A privileged chess reset committed in the app store".into()),
            },
        ],
    );
}

fn tool_defs() -> Vec<AiTool> {
    vec![
        tool(
            "chess.state",
            "Return game identity, revision, position, history, side to move, and status.",
            json!({"type": "object", "required": ["game_id"], "properties": {"game_id": {"type": "string"}}, "additionalProperties": false}),
            true,
        ),
        tool(
            "chess.legal_moves",
            "Return legal moves for one game revision.",
            json!({"type": "object", "required": ["game_id", "revision"], "properties": {"game_id": {"type": "string"}, "revision": {"type": "integer"}}, "additionalProperties": false}),
            true,
        ),
        tool(
            "chess.play",
            "Commit one UCI move when the actor, revision, and operation id allow it.",
            json!({"type": "object", "required": ["game_id", "expected_revision", "operation_id", "uci"], "properties": {"game_id": {"type": "string"}, "expected_revision": {"type": "integer"}, "operation_id": {"type": "string"}, "uci": {"type": "string"}}, "additionalProperties": false}),
            false,
        ),
        tool(
            "chess.new_game",
            "Privileged reset of one game. Ordinary players are not granted this.",
            json!({"type": "object", "required": ["game_id", "operation_id"], "properties": {"game_id": {"type": "string"}, "operation_id": {"type": "string"}, "fen": {"type": "string"}}, "additionalProperties": false}),
            false,
        ),
    ]
}

fn tool(name: &str, description: &str, input_schema: serde_json::Value, read_only: bool) -> AiTool {
    AiTool {
        name: name.into(),
        description: description.into(),
        input_schema,
        output_schema: json!({"type": "object"}),
        timeout_ms: Some(2_000),
        read_only,
    }
}

#[cfg(test)]
mod tests {
    use super::super::chess::Side;
    use super::*;
    use crate::plexi_ai::tool_dispatch::ToolDispatcher;

    fn white_grant() -> ActorGrant {
        ActorGrant {
            actor_id: "agent:white".into(),
            game_id: "game-fixture".into(),
            inspect: true,
            play: Some(Side::White),
            reset: false,
        }
    }

    fn black_grant() -> ActorGrant {
        ActorGrant {
            actor_id: "agent:black".into(),
            game_id: "game-fixture".into(),
            inspect: true,
            play: Some(Side::Black),
            reset: false,
        }
    }

    fn admin_grant() -> ActorGrant {
        ActorGrant {
            actor_id: "agent:admin".into(),
            game_id: "game-fixture".into(),
            inspect: true,
            play: None,
            reset: true,
        }
    }

    fn app(policy: BackgroundPolicy) -> (tempfile::TempDir, ChessApp) {
        let dir = tempfile::tempdir().unwrap();
        let chess = ChessApp::create(dir.path(), policy).unwrap();
        chess
            .provision(
                "game-fixture",
                vec![white_grant(), black_grant(), admin_grant()],
            )
            .unwrap();
        (dir, chess)
    }

    fn play_args(revision: u64, op: &str, uci: &str) -> serde_json::Value {
        json!({
            "game_id": "game-fixture",
            "expected_revision": revision,
            "operation_id": op,
            "uci": uci
        })
    }

    fn fen_of(result: &ToolCallResult) -> String {
        let value: serde_json::Value =
            serde_json::from_str(result.output_json.as_deref().unwrap()).unwrap();
        value["fen"].as_str().unwrap().to_string()
    }

    #[test]
    fn authorized_move_publishes_one_scoped_event() {
        let (_dir, chess) = app(BackgroundPolicy::Continue);
        chess.subscribe("agent:black", "game-fixture", chess.context_id());
        chess.subscribe("agent:other", "other-game", chess.context_id());
        chess.subscribe("agent:cross", "game-fixture", chess.context_id() + 1);
        let result = chess.dispatch("agent:white", "chess.play", play_args(0, "op-e4", "e2e4"));
        assert!(result.error.is_none(), "{:?}", result.error);
        let body: serde_json::Value =
            serde_json::from_str(result.output_json.unwrap().as_str()).unwrap();
        assert_eq!(body["replayed"], false);
        assert_eq!(body["revision"], 1);
        assert_eq!(body["actor_id"], "agent:white");
        assert!(body["fen"].as_str().unwrap().contains("/4P3/"));
        let black = chess.take_deliveries("agent:black");
        assert_eq!(black.len(), 1);
        assert_eq!(black[0].event, "chess.move_committed");
        assert_eq!(black[0].payload.as_ref().unwrap()["revision_after"], 1);
        assert_eq!(black[0].payload.as_ref().unwrap()["uci"], "e2e4");
        assert!(chess.take_deliveries("agent:other").is_empty());
        assert!(chess.take_deliveries("agent:cross").is_empty());
    }

    #[test]
    fn idempotency_grant_and_stale_revision_hold_through_dispatch() {
        let (_dir, chess) = app(BackgroundPolicy::Continue);
        let first = chess.dispatch("agent:white", "chess.play", play_args(0, "op-e4", "e2e4"));
        assert!(first.error.is_none(), "{:?}", first.error);
        let replay = chess.dispatch("agent:white", "chess.play", play_args(0, "op-e4", "e2e4"));
        assert!(replay.error.is_none(), "{:?}", replay.error);
        let replay_body: serde_json::Value =
            serde_json::from_str(replay.output_json.unwrap().as_str()).unwrap();
        assert_eq!(replay_body["replayed"], true);
        assert_eq!(replay_body["revision"], 1);
        let state = chess.dispatch(
            "agent:white",
            "chess.state",
            json!({"game_id": "game-fixture"}),
        );
        let state_body: serde_json::Value =
            serde_json::from_str(state.output_json.unwrap().as_str()).unwrap();
        assert_eq!(state_body["history"], json!(["e2e4"]));

        let conflict = chess.dispatch("agent:white", "chess.play", play_args(0, "op-e4", "d2d4"));
        assert!(conflict.error.unwrap().starts_with("duplicate_conflict"));
        assert_eq!(state_body["revision"], 1);

        let stranger = chess.dispatch("agent:stranger", "chess.play", play_args(1, "op-x", "e7e5"));
        assert!(stranger.error.unwrap().starts_with("denied"));
        let forged = chess.dispatch(
            "agent:black",
            "chess.play",
            json!({
                "game_id": "game-fixture",
                "expected_revision": 1,
                "operation_id": "op-forged",
                "uci": "e7e5",
                "actor_id": "agent:white"
            }),
        );
        assert!(forged.error.is_some());
        let wrong_side =
            chess.dispatch("agent:white", "chess.play", play_args(1, "op-side", "e7e5"));
        assert!(wrong_side.error.unwrap().starts_with("unauthorized"));
        let stale = chess.dispatch(
            "agent:black",
            "chess.play",
            play_args(0, "op-stale", "e7e5"),
        );
        assert!(stale.error.unwrap().starts_with("stale_revision"));
        let illegal = chess.dispatch("agent:black", "chess.play", play_args(1, "op-bad", "e7e4"));
        assert!(illegal.error.unwrap().starts_with("illegal_move"));
        let after = chess.dispatch(
            "agent:black",
            "chess.state",
            json!({"game_id": "game-fixture"}),
        );
        let after_body: serde_json::Value =
            serde_json::from_str(after.output_json.unwrap().as_str()).unwrap();
        assert_eq!(after_body["revision"], 1);
        assert_eq!(after_body["history"], json!(["e2e4"]));
    }

    #[test]
    fn namespaced_mcp_call_uses_stamped_caller_not_argument_actor() {
        let (_dir, chess) = app(BackgroundPolicy::Continue);
        let dispatcher =
            ToolDispatcher::from_namespaced_registry(chess.pane_id(), chess.context_id());
        let names: Vec<_> = dispatcher
            .all_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert!(names.iter().any(|name| name == "chess__chess.play"));
        let result = dispatcher.dispatch_call(
            "mcp-call".into(),
            "chess__chess.play",
            play_args(0, "op-mcp", "e2e4").to_string(),
        );
        assert!(result.error.unwrap().starts_with("denied"));
        let state = chess.dispatch(
            "agent:white",
            "chess.state",
            json!({"game_id": "game-fixture"}),
        );
        let body: serde_json::Value =
            serde_json::from_str(state.output_json.unwrap().as_str()).unwrap();
        assert_eq!(body["revision"], 0);
    }

    #[test]
    fn crash_between_commit_and_publish_keeps_one_mutation() {
        let (dir, chess) = app(BackgroundPolicy::Continue);
        chess.arm_skip_publish();
        let result = chess.dispatch("agent:white", "chess.play", play_args(0, "op-e4", "e2e4"));
        assert!(result.error.is_none(), "{:?}", result.error);
        chess.subscribe("agent:black", "game-fixture", chess.context_id());
        assert!(chess.take_deliveries("agent:black").is_empty());
        drop(chess);
        let reopened = ChessApp::open(dir.path()).unwrap();
        reopened.subscribe("agent:black", "game-fixture", reopened.context_id());
        assert_eq!(reopened.publish_pending().unwrap(), 1);
        let deliveries = reopened.take_deliveries("agent:black");
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].payload.as_ref().unwrap()["uci"], "e2e4");
        drop(reopened);
        let again = ChessApp::open(dir.path()).unwrap();
        again.subscribe("agent:black", "game-fixture", again.context_id());
        assert_eq!(again.publish_pending().unwrap(), 0);
        assert!(again.take_deliveries("agent:black").is_empty());
        let state = again.dispatch(
            "agent:white",
            "chess.state",
            json!({"game_id": "game-fixture"}),
        );
        let body: serde_json::Value =
            serde_json::from_str(state.output_json.unwrap().as_str()).unwrap();
        assert_eq!(body["revision"], 1);
        assert_eq!(body["history"], json!(["e2e4"]));
    }

    #[test]
    fn persist_failure_does_not_mutate() {
        let (_dir, chess) = app(BackgroundPolicy::Continue);
        chess.arm_persist_failure();
        let result = chess.dispatch("agent:white", "chess.play", play_args(0, "op-e4", "e2e4"));
        assert!(result.error.unwrap().contains("persist_failed"));
        let state = chess.dispatch(
            "agent:white",
            "chess.state",
            json!({"game_id": "game-fixture"}),
        );
        let body: serde_json::Value =
            serde_json::from_str(state.output_json.unwrap().as_str()).unwrap();
        assert_eq!(body["revision"], 0);
    }

    #[test]
    fn reset_is_privileged_and_old_operation_does_not_apply_again() {
        let (_dir, chess) = app(BackgroundPolicy::Continue);
        chess
            .dispatch("agent:white", "chess.play", play_args(0, "op-e4", "e2e4"))
            .error
            .is_none()
            .then_some(())
            .unwrap();
        let denied = chess.dispatch(
            "agent:white",
            "chess.new_game",
            json!({"game_id": "game-fixture", "operation_id": "reset-1"}),
        );
        assert!(denied.error.unwrap().starts_with("unauthorized"));
        let reset = chess.dispatch(
            "agent:admin",
            "chess.new_game",
            json!({"game_id": "game-fixture", "operation_id": "reset-1"}),
        );
        assert!(reset.error.is_none(), "{:?}", reset.error);
        let replay = chess.dispatch("agent:white", "chess.play", play_args(0, "op-e4", "e2e4"));
        assert!(replay.error.is_none(), "{:?}", replay.error);
        let body: serde_json::Value =
            serde_json::from_str(replay.output_json.unwrap().as_str()).unwrap();
        assert_eq!(body["replayed"], true);
        let state = chess.dispatch(
            "agent:white",
            "chess.state",
            json!({"game_id": "game-fixture"}),
        );
        let state_body: serde_json::Value =
            serde_json::from_str(state.output_json.unwrap().as_str()).unwrap();
        assert!(state_body["fen"]
            .as_str()
            .unwrap()
            .starts_with("rnbqkbnr/pppppppp/"));
        assert!(state_body["history"].as_array().unwrap().is_empty());
    }

    #[test]
    fn background_policy_and_cached_tool_do_not_grant_authority() {
        let (_dir, chess) = app(BackgroundPolicy::StopOnLastViewClose);
        let cached = ToolDispatcher::from_registry(9, "agent:white".into(), chess.context_id());
        chess
            .observe(ViewState::Hidden {
                device: "desktop".into(),
                observed_at: "2030-01-01T00:00:00Z".into(),
            })
            .unwrap();
        assert!(chess.running());
        let hidden = chess.dispatch("agent:white", "chess.play", play_args(0, "op-e4", "e2e4"));
        assert!(hidden.error.is_none(), "{:?}", hidden.error);
        chess.close_view().unwrap();
        assert!(!chess.running());
        assert!(matches!(chess.view(), ViewState::NoView { .. }));
        let stopped = cached.dispatch_call(
            "cached".into(),
            "chess.play",
            play_args(1, "op-next", "d2d4").to_string(),
        );
        assert!(stopped.error.unwrap().starts_with("instance_stopped"));
        let fresh = ToolDispatcher::from_registry(9, "agent:white".into(), chess.context_id());
        assert!(fresh
            .all_tools()
            .iter()
            .all(|tool| tool.name != "chess.play"));
        let state = chess.dispatch(
            "agent:white",
            "chess.state",
            json!({"game_id": "game-fixture"}),
        );
        assert!(state.error.unwrap().starts_with("tool_not_found"));
        assert!(fen_of(&hidden).contains("/4P3/"));
    }

    #[test]
    fn continue_policy_survives_hidden_inactive_and_detached_views() {
        let (dir, chess) = app(BackgroundPolicy::Continue);
        let context = chess.context_id();
        let instance = chess.instance_id();
        chess
            .observe(ViewState::Hidden {
                device: "phone".into(),
                observed_at: "2030-01-01T00:00:00Z".into(),
            })
            .unwrap();
        chess
            .observe(ViewState::InactiveContext {
                device: "desktop".into(),
                observed_at: "2030-01-01T00:00:01Z".into(),
            })
            .unwrap();
        chess.close_view().unwrap();
        assert!(chess.running());
        let other = chess.dispatch_in_context(
            "agent:white",
            context + 7,
            "chess.play",
            play_args(0, "op-cross", "e2e4"),
        );
        assert!(other.error.unwrap().starts_with("tool_not_found"));
        let played = chess.dispatch("agent:white", "chess.play", play_args(0, "op-e4", "e2e4"));
        assert!(played.error.is_none(), "{:?}", played.error);
        drop(chess);
        let reopened = ChessApp::open(dir.path()).unwrap();
        assert_eq!(reopened.instance_id(), instance);
        assert_eq!(reopened.context_id(), context);
        assert!(reopened.running());
        let state = reopened.dispatch(
            "agent:white",
            "chess.state",
            json!({"game_id": "game-fixture"}),
        );
        let body: serde_json::Value =
            serde_json::from_str(state.output_json.unwrap().as_str()).unwrap();
        assert_eq!(body["revision"], 1);
        let recorded = reopened.record();
        assert_eq!(recorded.instance_id, instance);
        assert_eq!(recorded.background_policy, BackgroundPolicy::Continue);
        assert!(matches!(recorded.view, ViewState::NoView { .. }));
    }

    #[test]
    fn concurrent_same_revision_admits_one_move() {
        let (_dir, chess) = app(BackgroundPolicy::Continue);
        let chess = Arc::new(chess);
        let mut joins = Vec::new();
        for (op, uci) in [("op-e", "e2e4"), ("op-d", "d2d4")] {
            let chess = Arc::clone(&chess);
            joins.push(std::thread::spawn(move || {
                chess.dispatch("agent:white", "chess.play", play_args(0, op, uci))
            }));
        }
        let results: Vec<_> = joins.into_iter().map(|join| join.join().unwrap()).collect();
        let wins = results
            .iter()
            .filter(|result| result.error.is_none())
            .count();
        let losses = results
            .iter()
            .filter(|result| result.error.is_some())
            .count();
        assert_eq!(wins, 1);
        assert_eq!(losses, 1);
        assert!(results.iter().any(|result| {
            result
                .error
                .as_deref()
                .is_some_and(|error| error.starts_with("stale_revision"))
        }));
        let state = chess.dispatch(
            "agent:white",
            "chess.state",
            json!({"game_id": "game-fixture"}),
        );
        let body: serde_json::Value =
            serde_json::from_str(state.output_json.unwrap().as_str()).unwrap();
        assert_eq!(body["revision"], 1);
        assert_eq!(body["history"].as_array().unwrap().len(), 1);
    }
}
