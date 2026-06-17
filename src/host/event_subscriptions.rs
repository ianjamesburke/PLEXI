//! Host-owned event subscription core — `docs/prm/undo-and-app-events.md`.
//!
//! One permission model, one delivery lifecycle, one schema. Both
//! agent-facing transports — the CLI NDJSON stream (`plexi events
//! subscribe`) and the host-level MCP server — wrap this module. They are
//! transports only, never second event buses.
//!
//! - [`evaluate_and_record_subscription`] is the single broker-gated path
//!   that turns a subscribe request into a [`SubscriptionRecord`]. Both the
//!   per-app `ProcessApp` (with its per-app grant store) and the host-level
//!   [`HostSubscriptionService`] (with the host grant store) call it, so the
//!   `TargetType::AppEventStream` grant rules live in exactly one place.
//! - [`HostSubscriptionService`] owns the host grant store + posture loaded
//!   from the profile dir and the global timeline handle. It serves non-app
//!   subscribers (CLI agents, host MCP clients) whose identity is derived
//!   from trusted host state, never spoofed from CLI arguments.

use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};

use crate::app_protocol::{PayloadMode, TriggerMode};
use crate::broker::{
    ActorType, Decision, GrantDuration, GrantStore, PermissionPosture, PermissionRequest,
    TargetType,
};
use crate::host::app_timeline::{AppTimeline, SubscriptionRecord};
use crate::host::event_log;

/// Evaluate broker grants for a subscription and, on a unanimous `Allow`,
/// record it in `timeline`. One `TargetType::AppEventStream` evaluation per
/// event name (`"<app_id>::<event>"`, or `"<app_id>::*"` for all streams);
/// the strictest non-allow decision short-circuits and nothing is recorded.
///
/// This is the shared seam between the per-app and host subscription paths —
/// keep the grant semantics here, not duplicated per caller.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_and_record_subscription(
    grant_store: &GrantStore,
    posture: Option<&PermissionPosture>,
    workspace_root: &Path,
    timeline: &Arc<Mutex<AppTimeline>>,
    publisher_app_id: &str,
    subscriber_type: ActorType,
    subscriber_id: &str,
    event_names: Vec<String>,
    payload_mode: PayloadMode,
    trigger_mode: TriggerMode,
    resource_id: Option<String>,
    duration: GrantDuration,
) -> Result<String, Decision> {
    let targets: Vec<String> = if event_names.is_empty() {
        vec![format!("{publisher_app_id}::*")]
    } else {
        event_names
            .iter()
            .map(|n| format!("{publisher_app_id}::{n}"))
            .collect()
    };
    let mut strictest = Decision::Allow;
    for target in &targets {
        let req = PermissionRequest::new(
            subscriber_type,
            subscriber_id,
            TargetType::AppEventStream,
            target,
            Some(workspace_root),
        );
        match grant_store.evaluate(&req, posture) {
            Decision::Allow => {}
            Decision::Deny => strictest = Decision::Deny,
            Decision::Ask => {
                if strictest != Decision::Deny {
                    strictest = Decision::Ask;
                }
            }
        }
    }
    if strictest != Decision::Allow {
        log::info!(
            "event_subscriptions: subscription to '{publisher_app_id}' events for \
             {subscriber_type:?} '{subscriber_id}' blocked by broker ({})",
            strictest.as_str()
        );
        return Err(strictest);
    }
    let subscription_id = format!("sub-{}", uuid::Uuid::new_v4());
    let record = SubscriptionRecord {
        subscription_id: subscription_id.clone(),
        subscriber_type,
        subscriber_id: subscriber_id.to_string(),
        app_id: publisher_app_id.to_string(),
        event_names,
        payload_mode,
        trigger_mode,
        resource_id,
        duration,
        created_at: event_log::now_timestamp(),
    };
    log::info!(
        "event_subscriptions: recorded subscription {subscription_id} for {subscriber_type:?} \
         '{subscriber_id}' -> '{publisher_app_id}' (payload={payload_mode:?}, trigger={trigger_mode:?})"
    );
    timeline.lock().unwrap().add_subscription(record);
    Ok(subscription_id)
}

/// An in-memory subscribe request handed from a transport's connection thread
/// to the host UI thread. The UI thread owns the [`HostSubscriptionService`]
/// (grant store + pane→identity trust), so identity resolution and the broker
/// check happen there; the connection thread streams deliveries afterward by
/// polling the global timeline directly. Not serialized — `reply` is a live
/// channel.
pub struct HostSubscribeRequest {
    pub publisher_app_id: String,
    /// Empty = subscribe to all of the app's declared streams.
    pub event_names: Vec<String>,
    pub payload_mode: PayloadMode,
    pub trigger_mode: TriggerMode,
    pub resource_id: Option<String>,
    /// Pane id stamped by the host PTY env (`PLEXI_PANE_ID`), forwarded by the
    /// transport. Host-trusted: the CLI cannot pass a subscriber identity flag.
    pub from_pane_id: Option<u64>,
    pub reply: SyncSender<HostSubscribeReply>,
}

/// The UI thread's answer to a [`HostSubscribeRequest`].
pub enum HostSubscribeReply {
    Ok {
        subscription_id: String,
        subscriber_type: ActorType,
        subscriber_id: String,
    },
    Err {
        message: String,
    },
}

/// Host-level subscription service for non-app actors (CLI agents reading
/// subprocess stdout, host MCP clients). Owns its own grant store + posture
/// loaded from the profile dir and a clone of the global timeline handle.
///
/// Every transport routes through this so the host has one event permission
/// model, one delivery queue, and one disconnect-cleanup path. The service
/// stamps subscriber identity from host-trusted state; callers never pass an
/// identity sourced from untrusted CLI arguments.
pub struct HostSubscriptionService {
    grant_store: GrantStore,
    posture: Option<PermissionPosture>,
    workspace_root: PathBuf,
    timeline: Arc<Mutex<AppTimeline>>,
}

impl HostSubscriptionService {
    /// Load the host grant store + posture from `config_dir` and bind the
    /// service to `timeline` (the global timeline in production).
    pub fn new(config_dir: &Path, workspace_root: PathBuf, timeline: Arc<Mutex<AppTimeline>>) -> Self {
        let grant_store = GrantStore::load_or_default(config_dir);
        let posture = PermissionPosture::load_from_config(config_dir);
        log::info!(
            "HostSubscriptionService: loaded {} grant record(s), posture={}",
            grant_store.records().len(),
            posture.is_some()
        );
        Self {
            grant_store,
            posture,
            workspace_root,
            timeline,
        }
    }

    /// Reload grants + posture from disk. Call before evaluating a fresh
    /// subscription so newly-granted permissions take effect without a host
    /// restart.
    pub fn reload(&mut self, config_dir: &Path) {
        self.grant_store = GrantStore::load_or_default(config_dir);
        self.posture = PermissionPosture::load_from_config(config_dir);
    }

    /// Subscribe a non-app actor to `publisher_app_id`'s streams. Broker-gated
    /// exactly like the app path. Session-scoped: the subscription is dropped
    /// when the transport disconnects (via [`Self::clear_subscriber`]).
    #[allow(clippy::too_many_arguments)]
    pub fn subscribe(
        &self,
        publisher_app_id: &str,
        subscriber_type: ActorType,
        subscriber_id: &str,
        event_names: Vec<String>,
        payload_mode: PayloadMode,
        trigger_mode: TriggerMode,
        resource_id: Option<String>,
    ) -> Result<String, Decision> {
        evaluate_and_record_subscription(
            &self.grant_store,
            self.posture.as_ref(),
            &self.workspace_root,
            &self.timeline,
            publisher_app_id,
            subscriber_type,
            subscriber_id,
            event_names,
            payload_mode,
            trigger_mode,
            resource_id,
            GrantDuration::Session,
        )
    }

    /// Derive the trusted subscriber identity for a transport connection from
    /// the host-stamped pane id. CLI/MCP agents are `Agent`-typed; the id is
    /// namespaced by pane so it never collides with the assistant
    /// (`agent:assistant`) or a `ProcessApp` (its `type_id`). A connection with
    /// no pane context (rare) gets a stable anonymous id.
    pub fn resolve_cli_subscriber(from_pane_id: Option<u64>) -> (ActorType, String) {
        let id = match from_pane_id {
            Some(p) => format!("pane:{p}"),
            None => "cli:anon".to_string(),
        };
        (ActorType::Agent, id)
    }

    /// Resolve identity, validate the requested streams are declared, run the
    /// broker check, and record the subscription. The single host entry point
    /// both transports' UI-thread handlers call.
    pub fn handle_subscribe_request(&self, req: &HostSubscribeRequest) -> HostSubscribeReply {
        let (subscriber_type, subscriber_id) = Self::resolve_cli_subscriber(req.from_pane_id);
        // Reject undeclared streams before involving the broker so the client
        // gets a precise error instead of a generic permission denial.
        let undeclared: Vec<&String> = req
            .event_names
            .iter()
            .filter(|n| !self.stream_is_declared(&req.publisher_app_id, n))
            .collect();
        if !undeclared.is_empty() {
            return HostSubscribeReply::Err {
                message: format!(
                    "app '{}' has not declared stream(s): {}",
                    req.publisher_app_id,
                    undeclared
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
        }
        match self.subscribe(
            &req.publisher_app_id,
            subscriber_type,
            &subscriber_id,
            req.event_names.clone(),
            req.payload_mode,
            req.trigger_mode,
            req.resource_id.clone(),
        ) {
            Ok(subscription_id) => HostSubscribeReply::Ok {
                subscription_id,
                subscriber_type,
                subscriber_id,
            },
            Err(decision) => HostSubscribeReply::Err {
                message: format!("blocked by broker: {}", decision.as_str()),
            },
        }
    }

    /// Whether `app_id` has declared `stream_name`. Used to reject subscribe
    /// requests for undeclared streams before they reach the broker.
    pub fn stream_is_declared(&self, app_id: &str, stream_name: &str) -> bool {
        self.timeline
            .lock()
            .unwrap()
            .declared_streams(app_id)
            .iter()
            .any(|d| d.name == stream_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_protocol::EventStreamDecl;
    use crate::broker::{ActorScope, Decision, GrantRecord, GrantSource, ResourceScope};

    fn allow_grant(target_id: &str) -> GrantRecord {
        GrantRecord {
            actor_type: ActorType::Agent,
            actor_id: "pane:7".to_string(),
            actor_scope: ActorScope::User,
            workspace_root: None,
            target_type: TargetType::AppEventStream,
            target_id: target_id.to_string(),
            resource_scope: ResourceScope::Global,
            resource_id: None,
            decision: Decision::Allow,
            duration: GrantDuration::Session,
            source: GrantSource::Session,
            created_at: 0,
            expires_at: None,
        }
    }

    fn timeline_with_stream() -> Arc<Mutex<AppTimeline>> {
        let mut t = AppTimeline::default();
        t.declare_streams(
            "event-probe",
            vec![EventStreamDecl {
                name: "probe.tick".to_string(),
                schema: serde_json::json!({"type": "object"}),
                description: None,
            }],
        )
        .unwrap();
        Arc::new(Mutex::new(t))
    }

    /// No grant on file → broker default is `Ask`, which is not `Allow`, so
    /// the subscription is refused and nothing is recorded.
    #[test]
    fn subscribe_refused_without_grant() {
        let timeline = timeline_with_stream();
        let store = GrantStore::default();
        let res = evaluate_and_record_subscription(
            &store,
            None,
            Path::new("/tmp/ws"),
            &timeline,
            "event-probe",
            ActorType::Agent,
            "pane:7",
            vec!["probe.tick".to_string()],
            PayloadMode::Full,
            TriggerMode::Conversation,
            None,
            GrantDuration::Session,
        );
        assert_eq!(res, Err(Decision::Ask));
        assert!(timeline.lock().unwrap().subscriptions().is_empty());
    }

    /// An explicit allow grant for the stream → subscription recorded.
    #[test]
    fn subscribe_succeeds_with_grant() {
        let timeline = timeline_with_stream();
        let mut store = GrantStore::default();
        store.record(allow_grant("event-probe::probe.tick"));
        let res = evaluate_and_record_subscription(
            &store,
            None,
            Path::new("/tmp/ws"),
            &timeline,
            "event-probe",
            ActorType::Agent,
            "pane:7",
            vec!["probe.tick".to_string()],
            PayloadMode::Full,
            TriggerMode::Conversation,
            None,
            GrantDuration::Session,
        );
        let sub_id = res.expect("grant present, subscription should record");
        assert!(sub_id.starts_with("sub-"));
        assert_eq!(timeline.lock().unwrap().subscriptions().len(), 1);
    }
}
