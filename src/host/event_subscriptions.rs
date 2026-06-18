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
    ActorScope, ActorType, Decision, GrantDuration, GrantRecord, GrantSource, GrantStore,
    PermissionPosture, PermissionRequest, ResourceScope, TargetType,
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
    let decision = evaluate_subscription(
        grant_store,
        posture,
        workspace_root,
        publisher_app_id,
        subscriber_type,
        subscriber_id,
        &event_names,
    );
    if decision != Decision::Allow {
        log::info!(
            "event_subscriptions: subscription to '{publisher_app_id}' events for \
             {subscriber_type:?} '{subscriber_id}' blocked by broker ({})",
            decision.as_str()
        );
        return Err(decision);
    }
    Ok(record_subscription(
        timeline,
        publisher_app_id,
        subscriber_type,
        subscriber_id,
        event_names,
        payload_mode,
        trigger_mode,
        resource_id,
        duration,
    ))
}

/// The broker target ids a subscription touches: one `"<app>::<event>"` per
/// requested event, or a single `"<app>::*"` when subscribing to every stream.
fn subscription_targets(publisher_app_id: &str, event_names: &[String]) -> Vec<String> {
    if event_names.is_empty() {
        vec![format!("{publisher_app_id}::*")]
    } else {
        event_names
            .iter()
            .map(|n| format!("{publisher_app_id}::{n}"))
            .collect()
    }
}

/// Run the broker over every target a subscription touches and return the
/// strictest decision (`Deny` > `Ask` > `Allow`). Records nothing — callers
/// decide what to do with the verdict (record on `Allow`, prompt on `Ask`).
pub fn evaluate_subscription(
    grant_store: &GrantStore,
    posture: Option<&PermissionPosture>,
    workspace_root: &Path,
    publisher_app_id: &str,
    subscriber_type: ActorType,
    subscriber_id: &str,
    event_names: &[String],
) -> Decision {
    let mut strictest = Decision::Allow;
    for target in &subscription_targets(publisher_app_id, event_names) {
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
    strictest
}

/// Add a subscription to `timeline` and return its id. Performs no broker check
/// — the caller has already established consent (a unanimous `Allow` grant or an
/// interactive user approval).
#[allow(clippy::too_many_arguments)]
pub fn record_subscription(
    timeline: &Arc<Mutex<AppTimeline>>,
    publisher_app_id: &str,
    subscriber_type: ActorType,
    subscriber_id: &str,
    event_names: Vec<String>,
    payload_mode: PayloadMode,
    trigger_mode: TriggerMode,
    resource_id: Option<String>,
    duration: GrantDuration,
) -> String {
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
    subscription_id
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
    /// Transport-supplied subscriber id, used instead of the pane-derived one.
    /// Set only by trusted host transport code (e.g. the host MCP server uses
    /// `mcp:host`), never sourced from an untrusted client argument.
    pub subscriber_override: Option<String>,
    pub reply: SyncSender<HostSubscribeReply>,
}

/// A subscribe request the broker answered with `Ask`: it needs an explicit
/// user decision before the subscription is recorded. The UI thread parks one
/// of these (holding the transport's live `reply` channel) and surfaces a host
/// consent modal; [`HostSubscriptionService::resolve_consent`] answers it.
///
/// Identity is already host-stamped here — the modal shows it, it is never
/// taken from the user's click.
pub struct PendingEventConsent {
    pub subscriber_type: ActorType,
    pub subscriber_id: String,
    pub publisher_app_id: String,
    /// Empty = all of the app's declared streams.
    pub event_names: Vec<String>,
    pub payload_mode: PayloadMode,
    pub trigger_mode: TriggerMode,
    pub resource_id: Option<String>,
    reply: SyncSender<HostSubscribeReply>,
}

impl PendingEventConsent {
    /// Human-readable target for the consent modal: `"<app> :: <events>"`.
    pub fn target_label(&self) -> String {
        let streams = if self.event_names.is_empty() {
            "* (all streams)".to_string()
        } else {
            self.event_names.join(", ")
        };
        format!("{} :: {streams}", self.publisher_app_id)
    }
}

/// The user's answer to a [`PendingEventConsent`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsentChoice {
    /// Record the subscription for this connection only; ask again next time.
    AllowOnce,
    /// Record a persistent `Allow` grant, then subscribe.
    AllowAlways,
    /// Refuse; the transport gets a permission-denied error.
    Deny,
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

    /// Construct a service with an explicit grant store + timeline for tests,
    /// bypassing disk loading.
    #[cfg(test)]
    pub fn new_for_test(grant_store: GrantStore, timeline: Arc<Mutex<AppTimeline>>) -> Self {
        Self {
            grant_store,
            posture: None,
            workspace_root: PathBuf::from("/tmp/ws"),
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

    /// Resolve host-stamped identity, validate the requested streams are
    /// declared, and run the broker check. The single host entry point both
    /// transports' UI-thread drain calls. Consumes the request so a deferred
    /// `Ask` can move the live `reply` channel into the returned
    /// [`PendingEventConsent`].
    ///
    /// - `Allow` → record the subscription and send `Ok` on `req.reply` now.
    /// - `Deny` / undeclared stream → send `Err` on `req.reply` now.
    /// - `Ask` → record nothing, send nothing; return `Some(consent)` for the
    ///   caller to park and surface a consent modal. The reply fires later from
    ///   [`Self::resolve_consent`].
    ///
    /// Returns `None` whenever the request was answered immediately.
    pub fn classify_subscribe_request(
        &self,
        req: HostSubscribeRequest,
    ) -> Option<PendingEventConsent> {
        let (subscriber_type, subscriber_id) = match &req.subscriber_override {
            Some(id) => (ActorType::Agent, id.clone()),
            None => Self::resolve_cli_subscriber(req.from_pane_id),
        };
        // Reject undeclared streams before involving the broker so the client
        // gets a precise error instead of a generic permission denial.
        let undeclared: Vec<String> = req
            .event_names
            .iter()
            .filter(|n| !self.stream_is_declared(&req.publisher_app_id, n))
            .cloned()
            .collect();
        if !undeclared.is_empty() {
            let _ = req.reply.send(HostSubscribeReply::Err {
                message: format!(
                    "app '{}' has not declared stream(s): {}",
                    req.publisher_app_id,
                    undeclared.join(", ")
                ),
            });
            return None;
        }
        let decision = evaluate_subscription(
            &self.grant_store,
            self.posture.as_ref(),
            &self.workspace_root,
            &req.publisher_app_id,
            subscriber_type,
            &subscriber_id,
            &req.event_names,
        );
        match decision {
            Decision::Allow => {
                let subscription_id = record_subscription(
                    &self.timeline,
                    &req.publisher_app_id,
                    subscriber_type,
                    &subscriber_id,
                    req.event_names,
                    req.payload_mode,
                    req.trigger_mode,
                    req.resource_id,
                    GrantDuration::Session,
                );
                let _ = req.reply.send(HostSubscribeReply::Ok {
                    subscription_id,
                    subscriber_type,
                    subscriber_id,
                });
                None
            }
            Decision::Deny => {
                let _ = req.reply.send(HostSubscribeReply::Err {
                    message: "blocked by broker: deny".to_string(),
                });
                None
            }
            Decision::Ask => {
                log::info!(
                    "event_subscriptions: subscription to '{}' for {subscriber_type:?} \
                     '{subscriber_id}' awaiting user consent",
                    req.publisher_app_id
                );
                Some(PendingEventConsent {
                    subscriber_type,
                    subscriber_id,
                    publisher_app_id: req.publisher_app_id,
                    event_names: req.event_names,
                    payload_mode: req.payload_mode,
                    trigger_mode: req.trigger_mode,
                    resource_id: req.resource_id,
                    reply: req.reply,
                })
            }
        }
    }

    /// Answer a parked [`PendingEventConsent`] with the user's decision. On
    /// `AllowAlways`, persist an `Allow` grant for every target so future
    /// subscribes pass without prompting; on `AllowOnce`, record only the
    /// session subscription. Fires the transport's reply either way. If the
    /// transport already gave up (its `reply` receiver dropped), the freshly
    /// recorded subscription is rolled back so no orphan accumulates deliveries.
    pub fn resolve_consent(
        &mut self,
        consent: PendingEventConsent,
        choice: ConsentChoice,
        config_dir: &Path,
    ) {
        if choice == ConsentChoice::Deny {
            log::info!(
                "event_subscriptions: user denied subscription for {:?} '{}' -> '{}'",
                consent.subscriber_type,
                consent.subscriber_id,
                consent.publisher_app_id
            );
            let _ = consent.reply.send(HostSubscribeReply::Err {
                message: "subscription denied by user".to_string(),
            });
            return;
        }

        if choice == ConsentChoice::AllowAlways {
            let created_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            for target in subscription_targets(&consent.publisher_app_id, &consent.event_names) {
                self.grant_store.record(GrantRecord {
                    actor_type: consent.subscriber_type,
                    actor_id: consent.subscriber_id.clone(),
                    actor_scope: ActorScope::User,
                    workspace_root: None,
                    target_type: TargetType::AppEventStream,
                    target_id: target,
                    resource_scope: ResourceScope::Global,
                    resource_id: None,
                    decision: Decision::Allow,
                    duration: GrantDuration::Always,
                    source: GrantSource::User,
                    created_at,
                    expires_at: None,
                });
            }
            self.grant_store.save();
            // Keep the in-memory store consistent with disk for later evals.
            self.reload(config_dir);
            log::info!(
                "event_subscriptions: user granted ALWAYS to {:?} '{}' -> '{}'",
                consent.subscriber_type,
                consent.subscriber_id,
                consent.publisher_app_id
            );
        }

        let subscription_id = record_subscription(
            &self.timeline,
            &consent.publisher_app_id,
            consent.subscriber_type,
            &consent.subscriber_id,
            consent.event_names.clone(),
            consent.payload_mode,
            consent.trigger_mode,
            consent.resource_id.clone(),
            GrantDuration::Session,
        );
        if consent
            .reply
            .send(HostSubscribeReply::Ok {
                subscription_id,
                subscriber_type: consent.subscriber_type,
                subscriber_id: consent.subscriber_id.clone(),
            })
            .is_err()
        {
            let (subs, drops) = self
                .timeline
                .lock()
                .unwrap()
                .clear_subscriber(consent.subscriber_type, &consent.subscriber_id);
            log::warn!(
                "event_subscriptions: consented subscriber '{}' already disconnected; \
                 rolled back {subs} subscription(s), {drops} queued delivery(ies)",
                consent.subscriber_id
            );
        } else {
            log::info!(
                "event_subscriptions: user allowed ({}) subscription for {:?} '{}' -> '{}'",
                if choice == ConsentChoice::AllowAlways {
                    "always"
                } else {
                    "once"
                },
                consent.subscriber_type,
                consent.subscriber_id,
                consent.publisher_app_id
            );
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
    use crate::app_protocol::{AppEventActor, EventStreamDecl};
    use crate::broker::{ActorScope, Decision, GrantRecord, GrantSource, ResourceScope};
    use crate::host::app_timeline::EmittedEvent;

    fn allow_grant(actor_id: &str, target_id: &str) -> GrantRecord {
        GrantRecord {
            actor_type: ActorType::Agent,
            actor_id: actor_id.to_string(),
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
        store.record(allow_grant("pane:7", "event-probe::probe.tick"));
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

    fn granted_service(timeline: Arc<Mutex<AppTimeline>>) -> HostSubscriptionService {
        let mut store = GrantStore::default();
        // Grant both the pane-derived (CLI) and override (MCP) identities.
        store.record(allow_grant("pane:7", "event-probe::probe.tick"));
        store.record(allow_grant("mcp:host", "event-probe::probe.tick"));
        HostSubscriptionService::new_for_test(store, timeline)
    }

    fn subscribe_request(
        event_names: Vec<String>,
        payload_mode: PayloadMode,
        override_id: Option<&str>,
    ) -> (HostSubscribeRequest, std::sync::mpsc::Receiver<HostSubscribeReply>) {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let req = HostSubscribeRequest {
            publisher_app_id: "event-probe".to_string(),
            event_names,
            payload_mode,
            trigger_mode: TriggerMode::Conversation,
            resource_id: None,
            from_pane_id: Some(7),
            subscriber_override: override_id.map(String::from),
            reply: tx,
        };
        (req, rx)
    }

    fn probe_event(count: u64) -> EmittedEvent {
        EmittedEvent {
            event: "probe.tick".to_string(),
            actor: AppEventActor::User,
            actor_id: None,
            caused_by: None,
            summary: format!("Probe tick {count}"),
            resource_id: "probe-session".to_string(),
            resource_scope: Some("document".to_string()),
            revision_after: format!("tick-{count}"),
            payload: Some(serde_json::json!({ "count": count })),
            state_ref: None,
            revision_before: None,
            rollback_token: None,
            changed_resources: vec![],
            suggested_trigger: Some(TriggerMode::Conversation),
        }
    }

    /// `classify_subscribe_request` rejects a stream the app never declared,
    /// before the broker is consulted.
    #[test]
    fn handle_request_rejects_undeclared_stream() {
        let timeline = timeline_with_stream();
        let svc = granted_service(Arc::clone(&timeline));
        let (req, rx) = subscribe_request(vec!["nope.stream".to_string()], PayloadMode::Full, None);
        assert!(svc.classify_subscribe_request(req).is_none());
        match rx.recv().unwrap() {
            HostSubscribeReply::Err { message } => assert!(message.contains("not declared")),
            HostSubscribeReply::Ok { .. } => panic!("undeclared stream must be refused"),
        }
        assert!(timeline.lock().unwrap().subscriptions().is_empty());
    }

    /// A CLI subscriber (no override) is identified by its pane.
    #[test]
    fn handle_request_uses_pane_identity() {
        let timeline = timeline_with_stream();
        let svc = granted_service(Arc::clone(&timeline));
        let (req, rx) = subscribe_request(vec!["probe.tick".to_string()], PayloadMode::Full, None);
        assert!(svc.classify_subscribe_request(req).is_none());
        match rx.recv().unwrap() {
            HostSubscribeReply::Ok { subscriber_id, .. } => assert_eq!(subscriber_id, "pane:7"),
            HostSubscribeReply::Err { message } => panic!("should subscribe: {message}"),
        }
    }

    /// The MCP transport's host-stamped override identity is honoured.
    #[test]
    fn handle_request_honours_override_identity() {
        let timeline = timeline_with_stream();
        let svc = granted_service(Arc::clone(&timeline));
        let (req, rx) =
            subscribe_request(vec!["probe.tick".to_string()], PayloadMode::Full, Some("mcp:host"));
        assert!(svc.classify_subscribe_request(req).is_none());
        match rx.recv().unwrap() {
            HostSubscribeReply::Ok { subscriber_id, .. } => assert_eq!(subscriber_id, "mcp:host"),
            HostSubscribeReply::Err { message } => panic!("should subscribe: {message}"),
        }
    }

    /// Default `Ask` posture parks a consent instead of recording or erroring.
    /// `AllowOnce` then records the subscription and replies `Ok` with the
    /// host-stamped identity — no grant is persisted.
    #[test]
    fn consent_allow_once_records_session_subscription() {
        let timeline = timeline_with_stream();
        // No grant + no posture → broker default Ask.
        let mut svc =
            HostSubscriptionService::new_for_test(GrantStore::default(), Arc::clone(&timeline));
        let (req, rx) = subscribe_request(vec!["probe.tick".to_string()], PayloadMode::Full, None);
        let consent = svc
            .classify_subscribe_request(req)
            .expect("Ask must park a consent, not answer inline");
        assert_eq!(consent.subscriber_id, "pane:7");
        assert!(timeline.lock().unwrap().subscriptions().is_empty());

        svc.resolve_consent(consent, ConsentChoice::AllowOnce, Path::new("/tmp/ws"));
        match rx.recv().unwrap() {
            HostSubscribeReply::Ok { subscriber_id, .. } => assert_eq!(subscriber_id, "pane:7"),
            HostSubscribeReply::Err { message } => panic!("allow-once should subscribe: {message}"),
        }
        assert_eq!(timeline.lock().unwrap().subscriptions().len(), 1);
    }

    /// `Deny` answers the transport with a permission error and records nothing.
    #[test]
    fn consent_deny_refuses_and_records_nothing() {
        let timeline = timeline_with_stream();
        let mut svc =
            HostSubscriptionService::new_for_test(GrantStore::default(), Arc::clone(&timeline));
        let (req, rx) = subscribe_request(vec!["probe.tick".to_string()], PayloadMode::Full, None);
        let consent = svc.classify_subscribe_request(req).expect("Ask must park a consent");
        svc.resolve_consent(consent, ConsentChoice::Deny, Path::new("/tmp/ws"));
        match rx.recv().unwrap() {
            HostSubscribeReply::Err { message } => assert!(message.contains("denied by user")),
            HostSubscribeReply::Ok { .. } => panic!("deny must refuse"),
        }
        assert!(timeline.lock().unwrap().subscriptions().is_empty());
    }

    /// `AllowAlways` persists an `Allow` grant: a fresh service over the same
    /// profile dir subscribes without prompting (classify answers inline).
    #[test]
    fn consent_allow_always_persists_grant() {
        let dir = tempfile::tempdir().unwrap();
        let timeline = timeline_with_stream();
        let mut svc = HostSubscriptionService::new(
            dir.path(),
            PathBuf::from("/tmp/ws"),
            Arc::clone(&timeline),
        );
        let (req, rx) = subscribe_request(vec!["probe.tick".to_string()], PayloadMode::Full, None);
        let consent = svc.classify_subscribe_request(req).expect("Ask must park a consent");
        svc.resolve_consent(consent, ConsentChoice::AllowAlways, dir.path());
        assert!(matches!(rx.recv().unwrap(), HostSubscribeReply::Ok { .. }));

        // Fresh service over the same profile dir: the persisted grant makes the
        // next subscribe pass inline (no parked consent).
        let svc2 = HostSubscriptionService::new(
            dir.path(),
            PathBuf::from("/tmp/ws"),
            Arc::clone(&timeline),
        );
        let (req2, rx2) =
            subscribe_request(vec!["probe.tick".to_string()], PayloadMode::Full, None);
        assert!(
            svc2.classify_subscribe_request(req2).is_none(),
            "persisted ALWAYS grant must answer inline, not re-prompt"
        );
        assert!(matches!(rx2.recv().unwrap(), HostSubscribeReply::Ok { .. }));
    }

    /// End-to-end: subscribe (full payload) → emit → the delivery carries the
    /// structured payload, then disconnect cleanup drops the subscription and
    /// any queued deliveries.
    #[test]
    fn delivery_full_payload_then_disconnect_cleanup() {
        let timeline = timeline_with_stream();
        let svc = granted_service(Arc::clone(&timeline));
        let (req, rx) = subscribe_request(vec!["probe.tick".to_string()], PayloadMode::Full, None);
        assert!(svc.classify_subscribe_request(req).is_none());
        let (stype, sid) = match rx.recv().unwrap() {
            HostSubscribeReply::Ok {
                subscriber_type,
                subscriber_id,
                ..
            } => (subscriber_type, subscriber_id),
            HostSubscribeReply::Err { message } => panic!("subscribe failed: {message}"),
        };

        timeline
            .lock()
            .unwrap()
            .record_event("event-probe", 7, probe_event(3))
            .expect("emit should record");

        let deliveries = timeline.lock().unwrap().take_deliveries_for(stype, &sid);
        assert_eq!(deliveries.len(), 1);
        let d = &deliveries[0];
        assert_eq!(d.event, "probe.tick");
        assert_eq!(d.summary.as_deref(), Some("Probe tick 3"));
        assert_eq!(d.payload, Some(serde_json::json!({ "count": 3 })));

        // Disconnect: a second event re-queues, then clear_subscriber wipes it.
        timeline
            .lock()
            .unwrap()
            .record_event("event-probe", 7, probe_event(4))
            .expect("emit should record");
        assert_eq!(timeline.lock().unwrap().pending_delivery_count(), 1);
        let (subs, drops) = timeline.lock().unwrap().clear_subscriber(stype, &sid);
        assert_eq!(subs, 1);
        assert_eq!(drops, 1);
        assert!(timeline.lock().unwrap().subscriptions().is_empty());
    }

    /// `PayloadMode::Summary` delivers the summary but withholds the payload.
    #[test]
    fn delivery_summary_mode_withholds_payload() {
        let timeline = timeline_with_stream();
        let svc = granted_service(Arc::clone(&timeline));
        let (req, rx) =
            subscribe_request(vec!["probe.tick".to_string()], PayloadMode::Summary, None);
        assert!(svc.classify_subscribe_request(req).is_none());
        let (stype, sid) = match rx.recv().unwrap() {
            HostSubscribeReply::Ok {
                subscriber_type,
                subscriber_id,
                ..
            } => (subscriber_type, subscriber_id),
            HostSubscribeReply::Err { message } => panic!("subscribe failed: {message}"),
        };
        timeline
            .lock()
            .unwrap()
            .record_event("event-probe", 7, probe_event(1))
            .expect("emit should record");
        let deliveries = timeline.lock().unwrap().take_deliveries_for(stype, &sid);
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].summary.as_deref(), Some("Probe tick 1"));
        assert_eq!(deliveries[0].payload, None);
    }
}
