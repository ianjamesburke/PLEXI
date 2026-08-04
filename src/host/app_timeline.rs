//! App event timeline + undo checkpoints.
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
    /// Host-established context the emitting app instance lives in (stint
    /// 0724 Phase D) — the owning `Scope::AppInstance` for reachability.
    /// Never app-supplied; resolved by the caller from
    /// `PlexiApp::origin_for_pane` at the point the event was recorded.
    pub owner_context_id: u64,
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
#[cfg(test)]
pub enum RollbackError {
    UnknownCheckpoint(String),
    AlreadyRolledBack(String),
    Conflict(String),
    /// A verification round-trip for this checkpoint is already in flight.
    VerifyInFlight(String),
}

#[cfg(test)]
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
#[cfg(test)]
pub struct RollbackVerifyRequest {
    pub checkpoint_id: String,
    pub resource_id: String,
    pub expected_revision: String,
}

/// The host's decision after the app reports its current revision.
#[derive(Debug, Clone, PartialEq)]
#[cfg(test)]
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
/// `WASM app runtime::subscribe_event_stream`).
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
    /// Host-established context the subscriber lives/acts in (stint 0724
    /// Phase D) — the viewer side of `evaluate_reach`. Never client-supplied;
    /// resolved by the caller from `PlexiApp::origin_for_pane` (or the
    /// equivalent host-trusted identity for non-pane subscribers) at the
    /// point the subscription was recorded.
    pub subscriber_context_id: u64,
    /// RFC 3339.
    pub created_at: String,
}

impl SubscriptionRecord {
    /// A subscription matches an event when: the publisher app-id matches,
    /// the event's owning scope is reachable from this subscriber's context
    /// (stint 0724 Phase D — same context only, since no cross-context grant
    /// is ever issued yet), the event name is one of the subscribed names
    /// (or the subscription covers all of the app's streams), and the
    /// resource filter (if any) matches.
    fn matches(&self, app_id: &str, record: &AppEventRecord) -> bool {
        if self.app_id != app_id {
            return false;
        }
        let owner = crate::host::scope::Scope::AppInstance {
            pane_id: record.pane_id,
            app_id: app_id.to_string(),
            context_id: record.owner_context_id,
        };
        if crate::host::scope::evaluate_reach(&owner, self.subscriber_context_id, None)
            != crate::host::scope::Reach::Allowed
        {
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
    /// `(context_id, app_id) -> stream name -> declaration`. Context-keyed
    /// (stint 0724 Phase D) so two contexts running the same `app_id` never
    /// share a stream namespace or clobber each other's schema — the
    /// cross-context bleed this phase fixes.
    streams: HashMap<(u64, String), HashMap<String, EventStreamDecl>>,
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

    /// Register an app's declared event streams, namespaced under the
    /// declaring instance's host-established `context_id` (stint 0724 Phase
    /// D) so the same `app_id` running in two different contexts never
    /// shares a stream namespace. Replaces any previous declaration for the
    /// same `(context_id, app_id, stream name)` (hot-reload friendly).
    /// Returns the names accepted; invalid declarations are rejected with a
    /// reason.
    pub fn declare_streams(
        &mut self,
        context_id: u64,
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
        let entry = self
            .streams
            .entry((context_id, app_id.to_string()))
            .or_default();
        for decl in decls {
            accepted.push(decl.name.clone());
            entry.insert(decl.name.clone(), decl);
        }
        log::info!(
            "app_timeline: '{app_id}' (context={context_id}) declared event streams {accepted:?}"
        );
        Ok(accepted)
    }

    /// Every declared `(context_id, app_id, stream_name)` triple, sorted —
    /// the discovery surface for actors that subscribe at runtime (Phase D
    /// Assistant). Context-qualified since the same `app_id` may declare
    /// different streams/schemas in different contexts.
    pub fn all_declared_streams(&self) -> Vec<(u64, String, String)> {
        let mut out: Vec<(u64, String, String)> = self
            .streams
            .iter()
            .flat_map(|((context_id, app), m)| {
                m.keys()
                    .map(move |name| (*context_id, app.clone(), name.clone()))
            })
            .collect();
        out.sort();
        out
    }

    /// Declared streams for an app in one context (empty when none declared).
    #[cfg(test)]
    pub fn declared_streams(&self, context_id: u64, app_id: &str) -> Vec<&EventStreamDecl> {
        self.streams
            .get(&(context_id, app_id.to_string()))
            .map(|m| m.values().collect())
            .unwrap_or_default()
    }

    /// Whether `app_id` in `context_id` has declared a stream named `name` —
    /// an O(1) lookup (mirrors `record_event`'s declared-stream check), for
    /// callers that only need existence and not the full `EventStreamDecl`
    /// list.
    pub fn has_stream(&self, context_id: u64, app_id: &str, name: &str) -> bool {
        self.streams
            .get(&(context_id, app_id.to_string()))
            .is_some_and(|m| m.contains_key(name))
    }

    // ── Event recording ─────────────────────────────────────────────────────

    /// Validate and record an emitted event. On success the event enters the
    /// timeline, creates a checkpoint when rollback metadata is present, and
    /// is routed to matching subscriptions. On failure nothing is recorded
    /// and the reason names the missing/invalid field.
    pub fn record_event(
        &mut self,
        context_id: u64,
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
        // Streams must be declared (in this context, under this app_id) with
        // a schema before use.
        let declared = self
            .streams
            .get(&(context_id, app_id.to_string()))
            .is_some_and(|m| m.contains_key(&emitted.event));
        if !declared {
            return Err(format!(
                "emit_event: '{}' is not a declared event stream for app '{app_id}' \
                 in this context — declare it via declare_event_streams first",
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
            owner_context_id: context_id,
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
        for sub in self
            .subscriptions
            .iter()
            .filter(|s| s.matches(app_id, record))
        {
            // `never` = record in the timeline only — no delivery.
            if sub.trigger_mode == TriggerMode::Never {
                continue;
            }
            self.next_delivery_id += 1;
            let (summary, payload, state_ref) = match sub.payload_mode {
                PayloadMode::Off => (None, None, None),
                PayloadMode::Summary => (Some(record.summary.clone()), None, None),
                PayloadMode::Full => (Some(record.summary.clone()), record.payload.clone(), None),
                PayloadMode::StateRef => {
                    (Some(record.summary.clone()), None, record.state_ref.clone())
                }
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
    #[cfg(test)]
    pub fn events(&self) -> &[AppEventRecord] {
        &self.events
    }

    // ── Undo timeline ───────────────────────────────────────────────────────

    /// All undo checkpoints (read-only, oldest first).
    #[cfg(test)]
    pub fn checkpoints(&self) -> &[UndoCheckpoint] {
        &self.checkpoints
    }

    /// Checkpoints for one app, newest first — the listing API for history UI.
    #[cfg(test)]
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
    /// see `WASM app runtime::request_rollback`.
    #[cfg(test)]
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
    #[cfg(test)]
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
    #[cfg(test)]
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
        self.subscriptions.retain(|s| {
            !(s.subscriber_type == subscriber_type && s.subscriber_id == subscriber_id)
        });
        let dels_before = self.deliveries.len();
        self.deliveries.retain(|d| {
            !(d.subscriber_type == subscriber_type && d.subscriber_id == subscriber_id)
        });
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

    /// Remove one subscription only when it belongs to the requesting actor.
    pub fn remove_subscription_for(
        &mut self,
        subscriber_type: ActorType,
        subscriber_id: &str,
        subscription_id: &str,
    ) -> Result<bool, String> {
        let Some(owner) = self
            .subscriptions
            .iter()
            .find(|subscription| subscription.subscription_id == subscription_id)
        else {
            return Ok(false);
        };
        if owner.subscriber_type != subscriber_type || owner.subscriber_id != subscriber_id {
            return Err(format!(
                "subscription '{subscription_id}' is owned by another actor"
            ));
        }
        Ok(self.remove_subscription(subscription_id))
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

/// The host-wide shared timeline. Production `WASM app runtime`s all clone this
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

    /// Default owning context for tests that don't care about cross-context
    /// behavior — one fixed value keeps every existing same-context
    /// assertion unchanged.
    const CTX: u64 = 1;
    /// A second, distinct context — used only by the cross-context tests.
    const OTHER_CTX: u64 = 2;

    fn timeline_with_stream() -> AppTimeline {
        let mut t = AppTimeline::default();
        t.declare_streams(CTX, "chess", vec![decl("move.played")])
            .unwrap();
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
            subscriber_context_id: CTX,
            created_at: "2026-06-11T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn declare_rejects_empty_name_and_non_object_schema() {
        let mut t = AppTimeline::default();
        assert!(t.declare_streams(CTX, "a", vec![]).is_err());
        assert!(t.declare_streams(CTX, "a", vec![decl(" ")]).is_err());
        let bad = EventStreamDecl {
            name: "x".to_string(),
            schema: serde_json::json!("not an object"),
            description: None,
        };
        assert!(t.declare_streams(CTX, "a", vec![bad]).is_err());
        assert!(
            t.declared_streams(CTX, "a").is_empty(),
            "nothing partial recorded"
        );
    }

    #[test]
    fn undeclared_stream_is_rejected() {
        let mut t = AppTimeline::default();
        let err = t
            .record_event(CTX, "chess", 1, emitted("move.played"))
            .unwrap_err();
        assert!(err.contains("not a declared event stream"), "{err}");
        assert!(t.events().is_empty());
    }

    #[test]
    fn same_app_id_in_different_context_needs_its_own_declaration() {
        // Declaring "move.played" for "chess" in CTX must not make it usable
        // for the same app_id running in OTHER_CTX — stream namespace is
        // (context_id, app_id), not app_id alone.
        let mut t = timeline_with_stream();
        let err = t
            .record_event(OTHER_CTX, "chess", 99, emitted("move.played"))
            .unwrap_err();
        assert!(err.contains("not a declared event stream"), "{err}");
        assert!(t.declared_streams(OTHER_CTX, "chess").is_empty());
        assert!(!t.declared_streams(CTX, "chess").is_empty());
    }

    #[test]
    fn required_fields_are_enforced() {
        let mut t = timeline_with_stream();
        for (field, mutate) in [
            (
                "event",
                Box::new(|e: &mut EmittedEvent| e.event = String::new())
                    as Box<dyn Fn(&mut EmittedEvent)>,
            ),
            (
                "summary",
                Box::new(|e: &mut EmittedEvent| e.summary = "  ".to_string()),
            ),
            (
                "resource_id",
                Box::new(|e: &mut EmittedEvent| e.resource_id = String::new()),
            ),
            (
                "revision_after",
                Box::new(|e: &mut EmittedEvent| e.revision_after = String::new()),
            ),
        ] {
            let mut e = emitted("move.played");
            mutate(&mut e);
            let err = t.record_event(CTX, "chess", 1, e).unwrap_err();
            assert!(
                err.contains(field),
                "expected '{field}' in error, got: {err}"
            );
        }
        assert!(
            t.events().is_empty(),
            "rejected events must not be recorded"
        );
    }

    #[test]
    fn accepted_event_enters_timeline_without_checkpoint() {
        let mut t = timeline_with_stream();
        let out = t
            .record_event(CTX, "chess", 7, emitted("move.played"))
            .unwrap();
        assert_eq!(
            out.checkpoint_id, None,
            "no rollback metadata = no checkpoint"
        );
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
        let out = t.record_event(CTX, "chess", 7, e).unwrap();
        let ckpt_id = out
            .checkpoint_id
            .expect("rollback metadata must checkpoint");
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
        let ckpt_id = t
            .record_event(CTX, "chess", 7, e)
            .unwrap()
            .checkpoint_id
            .unwrap();

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
        let ckpt_id = t
            .record_event(CTX, "chess", 7, e)
            .unwrap()
            .checkpoint_id
            .unwrap();

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
        let ckpt_id = t
            .record_event(CTX, "chess", 7, e)
            .unwrap()
            .checkpoint_id
            .unwrap();
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
        t.declare_streams(CTX, "chess", vec![decl("game.ended")])
            .unwrap();
        let mut sub = subscription(PayloadMode::Summary, TriggerMode::Conversation);
        sub.event_names = vec!["game.ended".to_string()];
        t.add_subscription(sub);

        // Non-matching event name → no delivery.
        let out = t
            .record_event(CTX, "chess", 1, emitted("move.played"))
            .unwrap();
        assert_eq!(out.deliveries_queued, 0);

        // Matching event name → delivery.
        let out = t
            .record_event(CTX, "chess", 1, emitted("game.ended"))
            .unwrap();
        assert_eq!(out.deliveries_queued, 1);

        // Resource-scoped subscription only matches its resource.
        let mut sub2 = subscription(PayloadMode::Summary, TriggerMode::Ask);
        sub2.subscription_id = "sub-2".to_string();
        sub2.resource_id = Some("game-other".to_string());
        t.add_subscription(sub2);
        let out = t
            .record_event(CTX, "chess", 1, emitted("move.played"))
            .unwrap();
        assert_eq!(
            out.deliveries_queued, 0,
            "resource mismatch must not deliver"
        );

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
        let out = t
            .record_event(CTX, "chess", 1, emitted("move.played"))
            .unwrap();
        assert_eq!(
            out.deliveries_queued, 0,
            "never = timeline only, no delivery"
        );
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
        let out = t
            .record_event(CTX, "chess", 1, emitted("move.played"))
            .unwrap();
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
        let out = t
            .record_event(CTX, "chess", 1, emitted("move.played"))
            .unwrap();
        assert_eq!(out.deliveries_queued, 0);
    }

    #[test]
    fn remove_subscription_requires_matching_owner() {
        let mut timeline = timeline_with_stream();
        timeline.add_subscription(subscription(
            PayloadMode::Summary,
            TriggerMode::Conversation,
        ));

        let denied = timeline.remove_subscription_for(ActorType::App, "other", "sub-1");
        assert!(denied.is_err());
        assert_eq!(timeline.subscriptions().len(), 1);

        let removed = timeline
            .remove_subscription_for(ActorType::Agent, "chess-opponent", "sub-1")
            .unwrap();
        assert!(removed);
        assert!(timeline.subscriptions().is_empty());
    }

    #[test]
    fn app_subscriptions_deliver_across_python_and_wasm_publishers() {
        let mut timeline = AppTimeline::default();
        timeline
            .declare_streams(CTX, "python-notes", vec![decl("note.saved")])
            .unwrap();
        timeline
            .declare_streams(CTX, "wasm-counter", vec![decl("count.changed")])
            .unwrap();

        let mut wasm_subscriber = subscription(PayloadMode::Full, TriggerMode::Conversation);
        wasm_subscriber.subscription_id = "wasm-sub".to_string();
        wasm_subscriber.subscriber_type = ActorType::App;
        wasm_subscriber.subscriber_id = "app-pane:41".to_string();
        wasm_subscriber.app_id = "python-notes".to_string();
        wasm_subscriber.event_names = vec!["note.saved".to_string()];
        timeline.add_subscription(wasm_subscriber);

        let mut python_subscriber = subscription(PayloadMode::Full, TriggerMode::Conversation);
        python_subscriber.subscription_id = "python-sub".to_string();
        python_subscriber.subscriber_type = ActorType::App;
        python_subscriber.subscriber_id = "app-pane:42".to_string();
        python_subscriber.app_id = "wasm-counter".to_string();
        python_subscriber.event_names = vec!["count.changed".to_string()];
        timeline.add_subscription(python_subscriber);

        let mut python_event = emitted("note.saved");
        python_event.resource_id = "note-1".to_string();
        assert_eq!(
            timeline
                .record_event(CTX, "python-notes", 11, python_event)
                .unwrap()
                .deliveries_queued,
            1
        );
        let wasm_delivery = timeline.take_deliveries_for(ActorType::App, "app-pane:41");
        assert_eq!(wasm_delivery.len(), 1);
        assert_eq!(wasm_delivery[0].event, "note.saved");

        let mut wasm_event = emitted("count.changed");
        wasm_event.resource_id = "counter-1".to_string();
        assert_eq!(
            timeline
                .record_event(CTX, "wasm-counter", 12, wasm_event)
                .unwrap()
                .deliveries_queued,
            1
        );
        let python_delivery = timeline.take_deliveries_for(ActorType::App, "app-pane:42");
        assert_eq!(python_delivery.len(), 1);
        assert_eq!(python_delivery[0].event, "count.changed");

        assert!(timeline
            .remove_subscription_for(ActorType::App, "app-pane:41", "wasm-sub")
            .unwrap());
        assert_eq!(
            timeline
                .record_event(CTX, "python-notes", 11, emitted("note.saved"))
                .unwrap()
                .deliveries_queued,
            0
        );
    }

    // ── Phase D: context-scoped stream ownership / reachability ─────────────

    /// The falsifying regression this phase fixes: the SAME `app_id`
    /// ("chess") runs once in `CTX` and once in `OTHER_CTX`. A subscriber
    /// whose own context is `CTX` must receive ONLY `CTX`'s emitted events —
    /// never `OTHER_CTX`'s — even though both declare identical stream names
    /// under the identical app_id. Before this phase, `SubscriptionRecord`
    /// carried no context dimension at all, so this delivery would have
    /// bled across contexts.
    #[test]
    fn cross_context_same_app_id_does_not_bleed_into_subscriber() {
        let mut t = AppTimeline::default();
        t.declare_streams(CTX, "chess", vec![decl("move.played")])
            .unwrap();
        t.declare_streams(OTHER_CTX, "chess", vec![decl("move.played")])
            .unwrap();

        // Subscriber lives in CTX.
        let mut sub = subscription(PayloadMode::Full, TriggerMode::Conversation);
        sub.subscriber_context_id = CTX;
        t.add_subscription(sub);

        // CTX's own "chess" instance emits — must deliver.
        let out = t
            .record_event(CTX, "chess", 1, emitted("move.played"))
            .unwrap();
        assert_eq!(out.deliveries_queued, 1, "same-context event must deliver");

        // OTHER_CTX's "chess" instance (identical app_id, identical stream
        // name) emits — must NOT deliver to the CTX subscriber.
        let out = t
            .record_event(OTHER_CTX, "chess", 99, emitted("move.played"))
            .unwrap();
        assert_eq!(
            out.deliveries_queued, 0,
            "a different context's same-app_id event must never reach a subscriber \
             scoped to another context"
        );

        // Exactly the one CTX delivery is queued — never the OTHER_CTX one.
        let deliveries = t.take_deliveries_for(ActorType::Agent, "chess-opponent");
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].event_id, out.event_id - 1);
    }

    /// A subscriber scoped to `OTHER_CTX` is the symmetric case: it receives
    /// only `OTHER_CTX`'s events, never `CTX`'s.
    #[test]
    fn subscriber_in_other_context_only_sees_its_own_context_events() {
        let mut t = AppTimeline::default();
        t.declare_streams(CTX, "chess", vec![decl("move.played")])
            .unwrap();
        t.declare_streams(OTHER_CTX, "chess", vec![decl("move.played")])
            .unwrap();

        let mut sub = subscription(PayloadMode::Full, TriggerMode::Conversation);
        sub.subscriber_context_id = OTHER_CTX;
        t.add_subscription(sub);

        let out = t
            .record_event(CTX, "chess", 1, emitted("move.played"))
            .unwrap();
        assert_eq!(
            out.deliveries_queued, 0,
            "CTX event must not reach OTHER_CTX subscriber"
        );

        let out = t
            .record_event(OTHER_CTX, "chess", 99, emitted("move.played"))
            .unwrap();
        assert_eq!(
            out.deliveries_queued, 1,
            "OTHER_CTX event must reach its own subscriber"
        );
    }

    /// Direct unit coverage of `SubscriptionRecord::matches`'s new
    /// `evaluate_reach` gate, independent of the higher-level `record_event`
    /// flow above — mirrors Phase C's `tool_dispatch.rs` test style.
    #[test]
    fn matches_rejects_cross_context_even_with_identical_app_and_event_name() {
        let mut same_context = subscription(PayloadMode::Full, TriggerMode::Conversation);
        same_context.subscriber_context_id = CTX;
        let mut cross_context = subscription(PayloadMode::Full, TriggerMode::Conversation);
        cross_context.subscriber_context_id = OTHER_CTX;

        let record = AppEventRecord {
            event_id: 1,
            app_id: "chess".to_string(),
            owner_context_id: CTX,
            pane_id: 7,
            event: "move.played".to_string(),
            actor: AppEventActor::User,
            actor_id: "chess".to_string(),
            caused_by: None,
            summary: "White played e4".to_string(),
            resource_id: "game-abc".to_string(),
            resource_scope: "game".to_string(),
            revision_after: "rev-13".to_string(),
            payload: None,
            state_ref: None,
            revision_before: None,
            rollback_token: None,
            changed_resources: vec![],
            suggested_trigger: None,
            created_at: "2026-06-11T00:00:00Z".to_string(),
        };

        assert!(same_context.matches("chess", &record));
        assert!(!cross_context.matches("chess", &record));
    }
}
