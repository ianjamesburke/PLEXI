//! App event timeline + undo checkpoints — `docs/prm/undo-and-app-events.md`.
//!
//! Apps own state. The host owns the timeline. Agents never own app state.
//!
//! This module is the host-owned record of:
//! - **Declared event streams**: apps must declare named streams (with a
//!   schema) before emitting on them.
//! - **App events**: every accepted `AppRequest::EmitEvent` enters the
//!   timeline. Malformed events are rejected and logged, never recorded.
//! - **Undo checkpoints**: events carrying rollback metadata
//!   (`rollback_token`) also create a reversible checkpoint.
//! - **Subscriptions**: actors subscribe to streams only through the broker
//!   (`TargetType::AppEventStream`); accepted events are routed into a
//!   delivery queue keyed by trigger mode.
//!
//! Phase C (agent runtime) consumes `drain_deliveries()` — the delivery
//! record carries the trigger mode (`never`/`conversation`/`ambient`/`ask`)
//! so the runtime knows whether to inject context, trigger a turn, run an
//! ambient workflow, or prompt first. Until then deliveries accumulate here.

use crate::app_protocol::{AppEventActor, EventStreamDecl, PayloadMode, TriggerMode};
use crate::broker::{ActorType, GrantDuration};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

// ── Validated event ──────────────────────────────────────────────────────────

/// A validated, host-accepted app event. Constructed only via
/// [`AppTimeline::record_event`] so every record in the timeline passed the
/// required-field checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppEventRecord {
    /// Host-assigned, unique within this timeline.
    pub event_id: u64,
    /// App that emitted the event (host-stamped, not app-supplied).
    pub app_id: String,
    /// Pane the emitting app instance lives in (host-stamped).
    pub pane_id: u64,
    /// Declared stream name, e.g. `move.played`.
    pub event: String,
    pub actor: AppEventActor,
    /// Actor identity. Defaults to the emitting app id when not supplied.
    pub actor_id: String,
    /// Causal tool caller (e.g. `"agent:chess-opponent"`) when this event
    /// was emitted while servicing a `ToolCall`; `None` for organic events.
    pub caused_by: Option<String>,
    pub summary: String,
    /// Document, game, pane, or app-instance id.
    pub resource_id: String,
    /// Scope class of `resource_id` (`pane` when the app didn't say).
    pub resource_scope: String,
    pub revision_after: String,
    pub payload: Option<serde_json::Value>,
    pub state_ref: Option<String>,
    pub revision_before: Option<String>,
    pub rollback_token: Option<String>,
    pub changed_resources: Vec<String>,
    /// App's hint; the subscription's own trigger mode always wins.
    pub suggested_trigger: Option<TriggerMode>,
    /// RFC 3339.
    pub created_at: String,
}

/// Unvalidated emit payload — the wire shape of `AppRequest::EmitEvent`
/// plus the host-stamped identity of the emitter.
#[derive(Debug, Clone)]
pub struct EmittedEvent {
    pub event: String,
    pub actor: AppEventActor,
    pub actor_id: Option<String>,
    pub caused_by: Option<String>,
    pub summary: String,
    pub resource_id: String,
    pub resource_scope: Option<String>,
    pub revision_after: String,
    pub payload: Option<serde_json::Value>,
    pub state_ref: Option<String>,
    pub revision_before: Option<String>,
    pub rollback_token: Option<String>,
    pub changed_resources: Vec<String>,
    pub suggested_trigger: Option<TriggerMode>,
}

// ── Undo checkpoints ─────────────────────────────────────────────────────────

/// Lifecycle of a reversible checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointStatus {
    /// Reversible — no rollback attempted.
    Active,
    /// Verification sent to the app, awaiting `RollbackVerifyResult`.
    Verifying,
    /// Rollback instruction (`RollbackApply`) was issued to the app.
    RolledBack,
    /// The app's current revision no longer matches `revision_after`;
    /// rollback is blocked pending conflict resolution.
    Conflict,
}

/// One reversible mutation in the host undo timeline. Field set matches the
/// spec's checkpoint metadata block exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoCheckpoint {
    pub checkpoint_id: String,
    pub actor_type: AppEventActor,
    pub actor_id: String,
    pub app_id: String,
    pub pane_id: u64,
    pub resource_scope: String,
    pub resource_id: String,
    pub revision_before: Option<String>,
    pub revision_after: String,
    pub rollback_token: String,
    pub changed_resources: Vec<String>,
    pub summary: String,
    /// RFC 3339.
    pub created_at: String,
    pub status: CheckpointStatus,
}

/// Outcome of recording an event.
#[derive(Debug, Clone, PartialEq)]
pub struct EventOutcome {
    pub event_id: u64,
    /// Set when the event carried rollback metadata.
    pub checkpoint_id: Option<String>,
    /// Number of subscription deliveries queued for this event.
    pub deliveries_queued: usize,
}

/// Why a rollback request did not proceed.
#[derive(Debug, Clone, PartialEq)]
pub enum RollbackError {
    UnknownCheckpoint(String),
    AlreadyRolledBack(String),
    Conflict(String),
    /// A verification round-trip for this checkpoint is already in flight.
    VerifyInFlight(String),
}

impl std::fmt::Display for RollbackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownCheckpoint(id) => write!(f, "unknown checkpoint '{id}'"),
            Self::AlreadyRolledBack(id) => write!(f, "checkpoint '{id}' already rolled back"),
            Self::Conflict(id) => write!(f, "checkpoint '{id}' is in conflict"),
            Self::VerifyInFlight(id) => write!(f, "checkpoint '{id}' verification in flight"),
        }
    }
}

/// What the host should send the app to start a rollback: a revision
/// verification question.
#[derive(Debug, Clone, PartialEq)]
pub struct RollbackVerifyRequest {
    pub checkpoint_id: String,
    pub resource_id: String,
    pub expected_revision: String,
}

/// The host's decision after the app reports its current revision.
#[derive(Debug, Clone, PartialEq)]
pub enum RollbackVerdict {
    /// Revisions match — instruct the app to roll back.
    Apply {
        checkpoint_id: String,
        resource_id: String,
        rollback_token: String,
    },
    /// Revisions diverged — rollback blocked, checkpoint marked conflict.
    Blocked {
        checkpoint_id: String,
        expected_revision: String,
        current_revision: String,
    },
}

// ── Subscriptions ────────────────────────────────────────────────────────────

/// A broker-granted subscription of one actor to one app's event stream(s).
/// Created only through [`AppTimeline::add_subscription`], which callers must
/// gate behind a `TargetType::AppEventStream` broker decision (see
/// `ProcessApp::subscribe_event_stream`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionRecord {
    pub subscription_id: String,
    pub subscriber_type: ActorType,
    pub subscriber_id: String,
    /// App package identity whose events are subscribed.
    pub app_id: String,
    /// Declared stream names. Empty = all streams the app declares.
    pub event_names: Vec<String>,
    pub payload_mode: PayloadMode,
    pub trigger_mode: TriggerMode,
    /// `None` = any resource the app emits on; `Some` = only that resource.
    pub resource_id: Option<String>,
    pub duration: GrantDuration,
    /// RFC 3339.
    pub created_at: String,
}

impl SubscriptionRecord {
    fn matches(&self, app_id: &str, record: &AppEventRecord) -> bool {
        if self.app_id != app_id {
            return false;
        }
        if !self.event_names.is_empty() && !self.event_names.contains(&record.event) {
            return false;
        }
        match &self.resource_id {
            None => true,
            Some(rid) => *rid == record.resource_id,
        }
    }
}

/// One routed event awaiting consumption by the agent runtime (Phase C).
/// The payload is already shaped per the subscription's `payload_mode`, so
/// the runtime never sees more than the grant allows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventDelivery {
    pub delivery_id: u64,
    pub subscription_id: String,
    pub subscriber_type: ActorType,
    pub subscriber_id: String,
    pub trigger_mode: TriggerMode,
    pub app_id: String,
    pub event: String,
    pub event_id: u64,
    pub resource_id: String,
    pub actor: AppEventActor,
    pub actor_id: String,
    /// Causal tool caller — agent runtimes must not trigger on deliveries
    /// the subscriber itself caused.
    pub caused_by: Option<String>,
    /// Always delivered — every mode but `off` includes the summary.
    pub summary: Option<String>,
    /// Present only for `payload_mode = full`.
    pub payload: Option<serde_json::Value>,
    /// Present only for `payload_mode = state_ref`.
    pub state_ref: Option<String>,
    /// RFC 3339.
    pub created_at: String,
}

// ── AppTimeline ──────────────────────────────────────────────────────────────

/// Host-owned timeline of app events, undo checkpoints, subscriptions, and
/// pending deliveries. Shared as `Arc<Mutex<AppTimeline>>`: production panes
/// all point at [`global()`], harness tests construct isolated instances.
#[derive(Debug, Default)]
pub struct AppTimeline {
    /// `app_id -> stream name -> declaration`.
    streams: HashMap<String, HashMap<String, EventStreamDecl>>,
    events: Vec<AppEventRecord>,
    checkpoints: Vec<UndoCheckpoint>,
    subscriptions: Vec<SubscriptionRecord>,
    deliveries: VecDeque<EventDelivery>,
    next_event_id: u64,
    next_delivery_id: u64,
    next_checkpoint_seq: u64,
}

impl AppTimeline {
    // ── Stream declaration ──────────────────────────────────────────────────

    /// Register an app's declared event streams. Replaces any previous
    /// declaration for the same stream name (hot-reload friendly). Returns
    /// the names accepted; invalid declarations are rejected with a reason.
    pub fn declare_streams(
        &mut self,
        app_id: &str,
        decls: Vec<EventStreamDecl>,
    ) -> Result<Vec<String>, String> {
        if decls.is_empty() {
            return Err("declare_event_streams: empty stream list".to_string());
        }
        let mut accepted = Vec::with_capacity(decls.len());
        for decl in &decls {
            if decl.name.trim().is_empty() {
                return Err("declare_event_streams: stream name must be non-empty".to_string());
            }
            if !decl.schema.is_object() {
                return Err(format!(
                    "declare_event_streams: schema for '{}' must be a JSON object",
                    decl.name
                ));
            }
        }
        let entry = self.streams.entry(app_id.to_string()).or_default();
        for decl in decls {
            accepted.push(decl.name.clone());
            entry.insert(decl.name.clone(), decl);
        }
        log::info!("app_timeline: '{app_id}' declared event streams {accepted:?}");
        Ok(accepted)
    }

    /// Every declared `(app_id, stream_name)` pair, sorted — the discovery
    /// surface for actors that subscribe at runtime (Phase D Assistant).
    pub fn all_declared_streams(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .streams
            .iter()
            .flat_map(|(app, m)| m.keys().map(move |name| (app.clone(), name.clone())))
            .collect();
        out.sort();
        out
    }

    /// Declared streams for an app (empty when none declared).
    pub fn declared_streams(&self, app_id: &str) -> Vec<&EventStreamDecl> {
        self.streams
            .get(app_id)
            .map(|m| m.values().collect())
            .unwrap_or_default()
    }

    // ── Event recording ─────────────────────────────────────────────────────

    /// Validate and record an emitted event. On success the event enters the
    /// timeline, creates a checkpoint when rollback metadata is present, and
    /// is routed to matching subscriptions. On failure nothing is recorded
    /// and the reason names the missing/invalid field.
    pub fn record_event(
        &mut self,
        app_id: &str,
        pane_id: u64,
        emitted: EmittedEvent,
    ) -> Result<EventOutcome, String> {
        // Required-field validation (spec: event name, actor, summary,
        // resource id, revision after).
        if emitted.event.trim().is_empty() {
            return Err("emit_event: 'event' name must be non-empty".to_string());
        }
        if emitted.summary.trim().is_empty() {
            return Err("emit_event: 'summary' must be non-empty".to_string());
        }
        if emitted.resource_id.trim().is_empty() {
            return Err("emit_event: 'resource_id' must be non-empty".to_string());
        }
        if emitted.revision_after.trim().is_empty() {
            return Err("emit_event: 'revision_after' must be non-empty".to_string());
        }
        // Streams must be declared with a schema before use.
        let declared = self
            .streams
            .get(app_id)
            .is_some_and(|m| m.contains_key(&emitted.event));
        if !declared {
            return Err(format!(
                "emit_event: '{}' is not a declared event stream for app '{app_id}' \
                 — declare it via declare_event_streams first",
                emitted.event
            ));
        }
        if let Some(scope) = &emitted.resource_scope {
            if scope.trim().is_empty() {
                return Err("emit_event: 'resource_scope' must be non-empty when set".to_string());
            }
        }

        self.next_event_id += 1;
        let record = AppEventRecord {
            event_id: self.next_event_id,
            app_id: app_id.to_string(),
            pane_id,
            event: emitted.event,
            actor: emitted.actor,
            actor_id: emitted.actor_id.unwrap_or_else(|| app_id.to_string()),
            caused_by: emitted.caused_by,
            summary: emitted.summary,
            resource_id: emitted.resource_id,
            resource_scope: emitted.resource_scope.unwrap_or_else(|| "pane".to_string()),
            revision_after: emitted.revision_after,
            payload: emitted.payload,
            state_ref: emitted.state_ref,
            revision_before: emitted.revision_before,
            rollback_token: emitted.rollback_token,
            changed_resources: emitted.changed_resources,
            suggested_trigger: emitted.suggested_trigger,
            created_at: crate::host::event_log::now_timestamp(),
        };

        // Rollback metadata → undo checkpoint.
        let checkpoint_id = record.rollback_token.clone().map(|token| {
            self.next_checkpoint_seq += 1;
            let checkpoint_id = format!("ckpt-{}", self.next_checkpoint_seq);
            let checkpoint = UndoCheckpoint {
                checkpoint_id: checkpoint_id.clone(),
                actor_type: record.actor,
                actor_id: record.actor_id.clone(),
                app_id: record.app_id.clone(),
                pane_id: record.pane_id,
                resource_scope: record.resource_scope.clone(),
                resource_id: record.resource_id.clone(),
                revision_before: record.revision_before.clone(),
                revision_after: record.revision_after.clone(),
                rollback_token: token,
                changed_resources: record.changed_resources.clone(),
                summary: record.summary.clone(),
                created_at: record.created_at.clone(),
                status: CheckpointStatus::Active,
            };
            log::info!(
                "app_timeline: undo checkpoint {checkpoint_id} for '{}' ({} -> {})",
                record.app_id,
                record.revision_before.as_deref().unwrap_or("?"),
                record.revision_after
            );
            self.checkpoints.push(checkpoint);
            checkpoint_id
        });

        // Route to matching subscriptions.
        let deliveries_queued = self.route_to_subscriptions(app_id, &record);

        log::info!(
            "app_timeline: recorded '{}' event {} from '{app_id}' (resource={}, rev_after={}, \
             checkpoint={:?}, deliveries={deliveries_queued})",
            record.event,
            record.event_id,
            record.resource_id,
            record.revision_after,
            checkpoint_id,
        );
        let outcome = EventOutcome {
            event_id: record.event_id,
            checkpoint_id,
            deliveries_queued,
        };
        self.events.push(record);
        Ok(outcome)
    }

    fn route_to_subscriptions(&mut self, app_id: &str, record: &AppEventRecord) -> usize {
        let mut queued = 0;
        let mut new_deliveries = Vec::new();
        for sub in self.subscriptions.iter().filter(|s| s.matches(app_id, record)) {
            // `never` = record in the timeline only — no delivery.
            if sub.trigger_mode == TriggerMode::Never {
                continue;
            }
            self.next_delivery_id += 1;
            let (summary, payload, state_ref) = match sub.payload_mode {
                PayloadMode::Off => (None, None, None),
                PayloadMode::Summary => (Some(record.summary.clone()), None, None),
                PayloadMode::Full => (
                    Some(record.summary.clone()),
                    record.payload.clone(),
                    None,
                ),
                PayloadMode::StateRef => (
                    Some(record.summary.clone()),
                    None,
                    record.state_ref.clone(),
                ),
            };
            new_deliveries.push(EventDelivery {
                delivery_id: self.next_delivery_id,
                subscription_id: sub.subscription_id.clone(),
                subscriber_type: sub.subscriber_type,
                subscriber_id: sub.subscriber_id.clone(),
                trigger_mode: sub.trigger_mode,
                app_id: app_id.to_string(),
                event: record.event.clone(),
                event_id: record.event_id,
                resource_id: record.resource_id.clone(),
                actor: record.actor,
                actor_id: record.actor_id.clone(),
                caused_by: record.caused_by.clone(),
                summary,
                payload,
                state_ref,
                created_at: record.created_at.clone(),
            });
            queued += 1;
        }
        for d in new_deliveries {
            log::info!(
                "app_timeline: queued delivery {} of '{}' to {:?} '{}' (trigger={:?})",
                d.delivery_id,
                d.event,
                d.subscriber_type,
                d.subscriber_id,
                d.trigger_mode
            );
            self.deliveries.push_back(d);
        }
        queued
    }

    /// All recorded events (read-only, oldest first).
    pub fn events(&self) -> &[AppEventRecord] {
        &self.events
    }

    // ── Undo timeline ───────────────────────────────────────────────────────

    /// All undo checkpoints (read-only, oldest first).
    pub fn checkpoints(&self) -> &[UndoCheckpoint] {
        &self.checkpoints
    }

    /// Checkpoints for one app, newest first — the listing API for history UI.
    pub fn checkpoints_for_app(&self, app_id: &str) -> Vec<&UndoCheckpoint> {
        let mut out: Vec<&UndoCheckpoint> = self
            .checkpoints
            .iter()
            .filter(|c| c.app_id == app_id)
            .collect();
        out.reverse();
        out
    }

    /// Begin a rollback: transition the checkpoint to `Verifying` and return
    /// the revision-verification question the host must send the app.
    /// Broker gating (`TargetType::UndoCheckpoint`) is the caller's duty —
    /// see `ProcessApp::request_rollback`.
    pub fn begin_rollback(
        &mut self,
        checkpoint_id: &str,
    ) -> Result<RollbackVerifyRequest, RollbackError> {
        let Some(ckpt) = self
            .checkpoints
            .iter_mut()
            .find(|c| c.checkpoint_id == checkpoint_id)
        else {
            return Err(RollbackError::UnknownCheckpoint(checkpoint_id.to_string()));
        };
        match ckpt.status {
            CheckpointStatus::RolledBack => {
                return Err(RollbackError::AlreadyRolledBack(checkpoint_id.to_string()))
            }
            CheckpointStatus::Conflict => {
                return Err(RollbackError::Conflict(checkpoint_id.to_string()))
            }
            CheckpointStatus::Verifying => {
                return Err(RollbackError::VerifyInFlight(checkpoint_id.to_string()))
            }
            CheckpointStatus::Active => {}
        }
        ckpt.status = CheckpointStatus::Verifying;
        log::info!(
            "app_timeline: rollback verification started for {checkpoint_id} \
             (expect rev '{}')",
            ckpt.revision_after
        );
        Ok(RollbackVerifyRequest {
            checkpoint_id: ckpt.checkpoint_id.clone(),
            resource_id: ckpt.resource_id.clone(),
            expected_revision: ckpt.revision_after.clone(),
        })
    }

    /// Resolve the app's answer to a rollback verification. Revision match →
    /// `Apply` (checkpoint marked `RolledBack`); mismatch → `Blocked`
    /// (checkpoint marked `Conflict`).
    pub fn resolve_rollback_verify(
        &mut self,
        checkpoint_id: &str,
        current_revision: &str,
    ) -> Result<RollbackVerdict, RollbackError> {
        let Some(ckpt) = self
            .checkpoints
            .iter_mut()
            .find(|c| c.checkpoint_id == checkpoint_id)
        else {
            return Err(RollbackError::UnknownCheckpoint(checkpoint_id.to_string()));
        };
        if ckpt.status != CheckpointStatus::Verifying {
            return Err(RollbackError::UnknownCheckpoint(format!(
                "{checkpoint_id} (no verification in flight)"
            )));
        }
        if ckpt.revision_after == current_revision {
            ckpt.status = CheckpointStatus::RolledBack;
            log::info!(
                "app_timeline: rollback verified for {checkpoint_id} — issuing apply \
                 (token '{}')",
                ckpt.rollback_token
            );
            Ok(RollbackVerdict::Apply {
                checkpoint_id: ckpt.checkpoint_id.clone(),
                resource_id: ckpt.resource_id.clone(),
                rollback_token: ckpt.rollback_token.clone(),
            })
        } else {
            ckpt.status = CheckpointStatus::Conflict;
            log::warn!(
                "app_timeline: rollback blocked for {checkpoint_id} — app at rev \
                 '{current_revision}', checkpoint expects '{}'",
                ckpt.revision_after
            );
            Ok(RollbackVerdict::Blocked {
                checkpoint_id: ckpt.checkpoint_id.clone(),
                expected_revision: ckpt.revision_after.clone(),
                current_revision: current_revision.to_string(),
            })
        }
    }

    // ── Subscriptions ───────────────────────────────────────────────────────

    /// Add a broker-approved subscription. Callers MUST have evaluated a
    /// `TargetType::AppEventStream` request to `Allow` first.
    pub fn add_subscription(&mut self, sub: SubscriptionRecord) {
        log::info!(
            "app_timeline: subscription {} — {:?} '{}' -> '{}' events {:?} \
             (payload={:?}, trigger={:?})",
            sub.subscription_id,
            sub.subscriber_type,
            sub.subscriber_id,
            sub.app_id,
            sub.event_names,
            sub.payload_mode,
            sub.trigger_mode
        );
        self.subscriptions.push(sub);
    }

    /// Active subscriptions (read-only).
    pub fn subscriptions(&self) -> &[SubscriptionRecord] {
        &self.subscriptions
    }

    /// Remove every subscription owned by one subscriber and drop its queued
    /// deliveries. Returns `(subscriptions_removed, deliveries_dropped)`.
    /// Used when a subscriber instance re-registers from persisted grants
    /// (e.g. the Assistant pane reopening) so subscriptions leaked by a
    /// previous instance never duplicate deliveries.
    pub fn clear_subscriber(
        &mut self,
        subscriber_type: ActorType,
        subscriber_id: &str,
    ) -> (usize, usize) {
        let subs_before = self.subscriptions.len();
        self.subscriptions
            .retain(|s| !(s.subscriber_type == subscriber_type && s.subscriber_id == subscriber_id));
        let dels_before = self.deliveries.len();
        self.deliveries
            .retain(|d| !(d.subscriber_type == subscriber_type && d.subscriber_id == subscriber_id));
        let removed = (
            subs_before - self.subscriptions.len(),
            dels_before - self.deliveries.len(),
        );
        if removed != (0, 0) {
            log::info!(
                "app_timeline: cleared subscriber {subscriber_type:?} '{subscriber_id}' — \
                 {} subscription(s) removed, {} queued delivery(ies) dropped",
                removed.0,
                removed.1
            );
        }
        removed
    }

    /// Remove a subscription by id. Returns true when one was removed.
    pub fn remove_subscription(&mut self, subscription_id: &str) -> bool {
        let before = self.subscriptions.len();
        self.subscriptions
            .retain(|s| s.subscription_id != subscription_id);
        let removed = self.subscriptions.len() != before;
        if removed {
            log::info!("app_timeline: removed subscription {subscription_id}");
        }
        removed
    }

    // ── Delivery queue (Phase C seam) ───────────────────────────────────────

    /// Take all queued deliveries addressed to one subscriber. Subscriber
    /// panes call this each frame to receive `PlexiEvent::AppEvent`s; the
    /// Phase C agent runtime consumes the same queue — `trigger_mode` tells
    /// it whether to inject and trigger a turn (`conversation`), run a
    /// bounded workflow (`ambient`), or prompt the user first (`ask`).
    pub fn take_deliveries_for(
        &mut self,
        subscriber_type: ActorType,
        subscriber_id: &str,
    ) -> Vec<EventDelivery> {
        let mut taken = Vec::new();
        let mut kept = VecDeque::with_capacity(self.deliveries.len());
        for d in self.deliveries.drain(..) {
            if d.subscriber_type == subscriber_type && d.subscriber_id == subscriber_id {
                taken.push(d);
            } else {
                kept.push_back(d);
            }
        }
        self.deliveries = kept;
        taken
    }

    /// Number of deliveries waiting without draining them.
    pub fn pending_delivery_count(&self) -> usize {
        self.deliveries.len()
    }
}

// ── Global shared instance ───────────────────────────────────────────────────

static GLOBAL_TIMELINE: OnceLock<Arc<Mutex<AppTimeline>>> = OnceLock::new();

/// The host-wide shared timeline. Production `ProcessApp`s all clone this
/// handle; tests construct isolated `Arc<Mutex<AppTimeline>>`s instead.
pub fn global() -> Arc<Mutex<AppTimeline>> {
    Arc::clone(GLOBAL_TIMELINE.get_or_init(|| Arc::new(Mutex::new(AppTimeline::default()))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(name: &str) -> EventStreamDecl {
        EventStreamDecl {
            name: name.to_string(),
            schema: serde_json::json!({"type": "object"}),
            description: None,
        }
    }

    fn emitted(event: &str) -> EmittedEvent {
        EmittedEvent {
            event: event.to_string(),
            actor: AppEventActor::User,
            actor_id: None,
            caused_by: None,
            summary: "White played e4".to_string(),
            resource_id: "game-abc".to_string(),
            resource_scope: Some("game".to_string()),
            revision_after: "rev-13".to_string(),
            payload: Some(serde_json::json!({"san": "e4"})),
            state_ref: Some("chess://game/abc/rev/13".to_string()),
            revision_before: Some("rev-12".to_string()),
            rollback_token: None,
            changed_resources: vec![],
            suggested_trigger: None,
        }
    }

    fn timeline_with_stream() -> AppTimeline {
        let mut t = AppTimeline::default();
        t.declare_streams("chess", vec![decl("move.played")]).unwrap();
        t
    }

    fn subscription(payload_mode: PayloadMode, trigger_mode: TriggerMode) -> SubscriptionRecord {
        SubscriptionRecord {
            subscription_id: "sub-1".to_string(),
            subscriber_type: ActorType::Agent,
            subscriber_id: "chess-opponent".to_string(),
            app_id: "chess".to_string(),
            event_names: vec!["move.played".to_string()],
            payload_mode,
            trigger_mode,
            resource_id: None,
            duration: GrantDuration::Session,
            created_at: "2026-06-11T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn declare_rejects_empty_name_and_non_object_schema() {
        let mut t = AppTimeline::default();
        assert!(t.declare_streams("a", vec![]).is_err());
        assert!(t.declare_streams("a", vec![decl(" ")]).is_err());
        let bad = EventStreamDecl {
            name: "x".to_string(),
            schema: serde_json::json!("not an object"),
            description: None,
        };
        assert!(t.declare_streams("a", vec![bad]).is_err());
        assert!(t.declared_streams("a").is_empty(), "nothing partial recorded");
    }

    #[test]
    fn undeclared_stream_is_rejected() {
        let mut t = AppTimeline::default();
        let err = t.record_event("chess", 1, emitted("move.played")).unwrap_err();
        assert!(err.contains("not a declared event stream"), "{err}");
        assert!(t.events().is_empty());
    }

    #[test]
    fn required_fields_are_enforced() {
        let mut t = timeline_with_stream();
        for (field, mutate) in [
            ("event", Box::new(|e: &mut EmittedEvent| e.event = String::new())
                as Box<dyn Fn(&mut EmittedEvent)>),
            ("summary", Box::new(|e: &mut EmittedEvent| e.summary = "  ".to_string())),
            ("resource_id", Box::new(|e: &mut EmittedEvent| e.resource_id = String::new())),
            ("revision_after", Box::new(|e: &mut EmittedEvent| e.revision_after = String::new())),
        ] {
            let mut e = emitted("move.played");
            mutate(&mut e);
            let err = t.record_event("chess", 1, e).unwrap_err();
            assert!(err.contains(field), "expected '{field}' in error, got: {err}");
        }
        assert!(t.events().is_empty(), "rejected events must not be recorded");
    }

    #[test]
    fn accepted_event_enters_timeline_without_checkpoint() {
        let mut t = timeline_with_stream();
        let out = t.record_event("chess", 7, emitted("move.played")).unwrap();
        assert_eq!(out.checkpoint_id, None, "no rollback metadata = no checkpoint");
        assert_eq!(t.events().len(), 1);
        let rec = &t.events()[0];
        assert_eq!(rec.app_id, "chess");
        assert_eq!(rec.pane_id, 7);
        assert_eq!(rec.actor_id, "chess", "actor_id defaults to app_id");
        assert_eq!(rec.resource_scope, "game");
        assert!(t.checkpoints().is_empty());
    }

    #[test]
    fn rollback_metadata_creates_checkpoint() {
        let mut t = timeline_with_stream();
        let mut e = emitted("move.played");
        e.rollback_token = Some("undo-abc".to_string());
        e.changed_resources = vec!["game-abc".to_string()];
        let out = t.record_event("chess", 7, e).unwrap();
        let ckpt_id = out.checkpoint_id.expect("rollback metadata must checkpoint");
        let ckpt = &t.checkpoints()[0];
        assert_eq!(ckpt.checkpoint_id, ckpt_id);
        assert_eq!(ckpt.rollback_token, "undo-abc");
        assert_eq!(ckpt.revision_before.as_deref(), Some("rev-12"));
        assert_eq!(ckpt.revision_after, "rev-13");
        assert_eq!(ckpt.resource_id, "game-abc");
        assert_eq!(ckpt.status, CheckpointStatus::Active);
        assert_eq!(t.checkpoints_for_app("chess").len(), 1);
        assert!(t.checkpoints_for_app("other").is_empty());
    }

    #[test]
    fn rollback_verify_match_applies_and_marks_rolled_back() {
        let mut t = timeline_with_stream();
        let mut e = emitted("move.played");
        e.rollback_token = Some("undo-abc".to_string());
        let ckpt_id = t.record_event("chess", 7, e).unwrap().checkpoint_id.unwrap();

        let verify = t.begin_rollback(&ckpt_id).unwrap();
        assert_eq!(verify.expected_revision, "rev-13");
        assert_eq!(verify.resource_id, "game-abc");

        let verdict = t.resolve_rollback_verify(&ckpt_id, "rev-13").unwrap();
        match verdict {
            RollbackVerdict::Apply { rollback_token, .. } => {
                assert_eq!(rollback_token, "undo-abc")
            }
            other => panic!("expected Apply, got {other:?}"),
        }
        assert_eq!(t.checkpoints()[0].status, CheckpointStatus::RolledBack);
        assert!(matches!(
            t.begin_rollback(&ckpt_id),
            Err(RollbackError::AlreadyRolledBack(_))
        ));
    }

    #[test]
    fn rollback_verify_mismatch_blocks_and_marks_conflict() {
        let mut t = timeline_with_stream();
        let mut e = emitted("move.played");
        e.rollback_token = Some("undo-abc".to_string());
        let ckpt_id = t.record_event("chess", 7, e).unwrap().checkpoint_id.unwrap();

        t.begin_rollback(&ckpt_id).unwrap();
        let verdict = t.resolve_rollback_verify(&ckpt_id, "rev-99").unwrap();
        assert!(matches!(verdict, RollbackVerdict::Blocked { .. }));
        assert_eq!(t.checkpoints()[0].status, CheckpointStatus::Conflict);
        assert!(matches!(
            t.begin_rollback(&ckpt_id),
            Err(RollbackError::Conflict(_))
        ));
    }

    #[test]
    fn begin_rollback_guards_unknown_and_in_flight() {
        let mut t = timeline_with_stream();
        assert!(matches!(
            t.begin_rollback("nope"),
            Err(RollbackError::UnknownCheckpoint(_))
        ));
        let mut e = emitted("move.played");
        e.rollback_token = Some("undo-abc".to_string());
        let ckpt_id = t.record_event("chess", 7, e).unwrap().checkpoint_id.unwrap();
        t.begin_rollback(&ckpt_id).unwrap();
        assert!(matches!(
            t.begin_rollback(&ckpt_id),
            Err(RollbackError::VerifyInFlight(_))
        ));
        // Resolving without a verification in flight is also rejected.
        let mut t2 = timeline_with_stream();
        assert!(t2.resolve_rollback_verify("nope", "rev-1").is_err());
    }

    #[test]
    fn subscription_routing_filters_event_name_and_resource() {
        let mut t = timeline_with_stream();
        t.declare_streams("chess", vec![decl("game.ended")]).unwrap();
        let mut sub = subscription(PayloadMode::Summary, TriggerMode::Conversation);
        sub.event_names = vec!["game.ended".to_string()];
        t.add_subscription(sub);

        // Non-matching event name → no delivery.
        let out = t.record_event("chess", 1, emitted("move.played")).unwrap();
        assert_eq!(out.deliveries_queued, 0);

        // Matching event name → delivery.
        let out = t.record_event("chess", 1, emitted("game.ended")).unwrap();
        assert_eq!(out.deliveries_queued, 1);

        // Resource-scoped subscription only matches its resource.
        let mut sub2 = subscription(PayloadMode::Summary, TriggerMode::Ask);
        sub2.subscription_id = "sub-2".to_string();
        sub2.resource_id = Some("game-other".to_string());
        t.add_subscription(sub2);
        let out = t.record_event("chess", 1, emitted("move.played")).unwrap();
        assert_eq!(out.deliveries_queued, 0, "resource mismatch must not deliver");

        // Deliveries for someone else stay queued.
        assert!(t
            .take_deliveries_for(ActorType::Agent, "someone-else")
            .is_empty());
        assert_eq!(t.pending_delivery_count(), 1);

        let deliveries = t.take_deliveries_for(ActorType::Agent, "chess-opponent");
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].trigger_mode, TriggerMode::Conversation);
        assert_eq!(deliveries[0].subscriber_id, "chess-opponent");
        assert_eq!(t.pending_delivery_count(), 0);
    }

    #[test]
    fn never_trigger_records_timeline_only() {
        let mut t = timeline_with_stream();
        t.add_subscription(subscription(PayloadMode::Full, TriggerMode::Never));
        let out = t.record_event("chess", 1, emitted("move.played")).unwrap();
        assert_eq!(out.deliveries_queued, 0, "never = timeline only, no delivery");
        assert_eq!(t.events().len(), 1);
        assert_eq!(t.pending_delivery_count(), 0);
    }

    #[test]
    fn payload_modes_shape_delivery() {
        let mut t = timeline_with_stream();
        for (i, mode) in [
            PayloadMode::Off,
            PayloadMode::Summary,
            PayloadMode::Full,
            PayloadMode::StateRef,
        ]
        .into_iter()
        .enumerate()
        {
            let mut sub = subscription(mode, TriggerMode::Ask);
            sub.subscription_id = format!("sub-{i}");
            t.add_subscription(sub);
        }
        let out = t.record_event("chess", 1, emitted("move.played")).unwrap();
        assert_eq!(out.deliveries_queued, 4);
        let d = t.take_deliveries_for(ActorType::Agent, "chess-opponent");
        // Off: nothing.
        assert!(d[0].summary.is_none() && d[0].payload.is_none() && d[0].state_ref.is_none());
        // Summary: summary only.
        assert!(d[1].summary.is_some() && d[1].payload.is_none() && d[1].state_ref.is_none());
        // Full: summary + payload.
        assert!(d[2].summary.is_some() && d[2].payload.is_some() && d[2].state_ref.is_none());
        // StateRef: summary + state_ref.
        assert!(d[3].summary.is_some() && d[3].payload.is_none() && d[3].state_ref.is_some());
    }

    #[test]
    fn remove_subscription_stops_routing() {
        let mut t = timeline_with_stream();
        t.add_subscription(subscription(PayloadMode::Summary, TriggerMode::Ambient));
        assert!(t.remove_subscription("sub-1"));
        assert!(!t.remove_subscription("sub-1"));
        let out = t.record_event("chess", 1, emitted("move.played")).unwrap();
        assert_eq!(out.deliveries_queued, 0);
    }
}
