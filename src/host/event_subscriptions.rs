//! Host-owned event subscription + publish core.
//!
//! One permission model, one delivery lifecycle, one schema. Both
//! agent-facing transports — the CLI NDJSON stream (`plexi events
//! subscribe`/`emit`/`declare`) and the host-level MCP server — wrap this
//! module. They are transports only, never second event buses.
//!
//! CLI/agent *publish* (declare + emit) is gated symmetrically to subscribe:
//! a foreign identity is broker-checked on `TargetType::AppEventStream` the
//! first time and, on `AllowAlways`, its grant persists — see
//! [`HostSubscriptionService::classify_publish_request`]. Read and write use
//! distinct grant targets (publish targets carry a `publish:` prefix), so an
//! Allow-Always granted on a read prompt never authorizes an emit, nor the
//! reverse.
//!
//! - [`HostSubscriptionService`] is the single broker-gated path that turns a
//!   subscribe request into a [`SubscriptionRecord`]. Python and Rust WASM app
//!   runtimes use it alongside CLI and host MCP transports.
//! - The service owns the host grant store, posture, and global timeline. Every
//!   caller supplies a host-stamped delivery identity; app runtimes also supply
//!   their host-known app identity and workspace for authorization.

use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};

use crate::app_protocol::{EventStreamDecl, PayloadMode, TriggerMode};
use crate::broker::{
    ActorScope, ActorType, Decision, GrantDuration, GrantRecord, GrantSource, GrantStore,
    PermissionPosture, PermissionRequest, ResourceScope, TargetType,
};
use crate::host::app_timeline::{AppTimeline, EmittedEvent, SubscriptionRecord};
use crate::host::event_log;

pub fn app_subscriber_id(pane_id: u64) -> String {
    format!("app-pane:{pane_id}")
}

/// Evaluate broker grants for a subscription and, on a unanimous `Allow`,
/// record it in `timeline`. One `TargetType::AppEventStream` evaluation per
/// event name (`"<app_id>::<event>"`, or `"<app_id>::*"` for all streams);
/// the strictest non-allow decision short-circuits and nothing is recorded.
///
/// Retained as a focused policy test seam. Production callers use
/// [`HostSubscriptionService::classify_subscribe_request`] so `Ask` decisions
/// can enter the shared consent UI.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub fn evaluate_and_record_subscription(
    grant_store: &GrantStore,
    posture: Option<&PermissionPosture>,
    workspace_root: Option<&Path>,
    timeline: &Arc<Mutex<AppTimeline>>,
    publisher_app_id: &str,
    subscriber_type: ActorType,
    subscriber_id: &str,
    subscriber_context_id: u64,
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
        subscriber_context_id,
        event_names,
        payload_mode,
        trigger_mode,
        resource_id,
        duration,
    ))
}

/// The broker target ids a subscription (read) touches: one `"<app>::<event>"`
/// per requested event, or a single `"<app>::*"` when subscribing to every
/// stream.
///
/// Read and write grants must never satisfy each other: an Allow-Always the user
/// granted on a read prompt must not silently authorize an emit, and vice versa.
/// Subscribe targets are the bare `"<app>::<event>"` form; publish targets carry
/// a `publish:` prefix ([`publish_targets`]) so a grant matches only the
/// operation it was granted for.
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

/// The broker target ids a publish (declare/emit) touches — the subscribe
/// targets with a `publish:` prefix so publish grants and subscribe grants are
/// distinct and can never match each other (see [`subscription_targets`]).
fn publish_targets(app_id: &str, event_names: &[String]) -> Vec<String> {
    subscription_targets(app_id, event_names)
        .into_iter()
        .map(|t| format!("publish:{t}"))
        .collect()
}

/// Run the broker over every target a subscription touches and return the
/// strictest decision (`Deny` > `Ask` > `Allow`). Records nothing — callers
/// decide what to do with the verdict (record on `Allow`, prompt on `Ask`).
pub fn evaluate_subscription(
    grant_store: &GrantStore,
    posture: Option<&PermissionPosture>,
    workspace_root: Option<&Path>,
    publisher_app_id: &str,
    subscriber_type: ActorType,
    subscriber_id: &str,
    event_names: &[String],
) -> Decision {
    evaluate_targets(
        grant_store,
        posture,
        workspace_root,
        subscriber_type,
        subscriber_id,
        &subscription_targets(publisher_app_id, event_names),
    )
}

/// Publish twin of [`evaluate_subscription`]: evaluate the broker over the
/// `publish:`-prefixed targets so a subscribe grant never authorizes a publish.
fn evaluate_publish(
    grant_store: &GrantStore,
    posture: Option<&PermissionPosture>,
    workspace_root: Option<&Path>,
    app_id: &str,
    actor_type: ActorType,
    actor_id: &str,
    event_names: &[String],
) -> Decision {
    evaluate_targets(
        grant_store,
        posture,
        workspace_root,
        actor_type,
        actor_id,
        &publish_targets(app_id, event_names),
    )
}

/// Run the broker over an explicit target-id list and return the strictest
/// decision (`Deny` > `Ask` > `Allow`). The shared core of
/// [`evaluate_subscription`] and [`evaluate_publish`] — they differ only in how
/// they build the target ids.
fn evaluate_targets(
    grant_store: &GrantStore,
    posture: Option<&PermissionPosture>,
    workspace_root: Option<&Path>,
    actor_type: ActorType,
    actor_id: &str,
    targets: &[String],
) -> Decision {
    let mut strictest = Decision::Allow;
    for target in targets {
        let req = PermissionRequest::new(
            actor_type,
            actor_id,
            TargetType::AppEventStream,
            target,
            workspace_root,
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
    subscriber_context_id: u64,
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
        subscriber_context_id,
        created_at: event_log::now_timestamp(),
    };
    log::info!(
        "event_subscriptions: recorded subscription {subscription_id} for {subscriber_type:?} \
         '{subscriber_id}' (context={subscriber_context_id}) -> '{publisher_app_id}' \
         (payload={payload_mode:?}, trigger={trigger_mode:?})"
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
    /// Transport-supplied subscriber id — the *delivery routing key*: where this
    /// specific in-flight wait's events are queued and torn down. Must be unique
    /// per concurrent waiter (e.g. the host MCP server mints `mcp:host:<uuid>`),
    /// never sourced from an untrusted client argument.
    pub subscriber_override: Option<String>,
    /// Host-stamped actor type for non-CLI subscribers. App runtimes set this
    /// to `App`; transports leave it unset and resolve as `Agent`.
    pub subscriber_type_override: Option<ActorType>,
    /// Transport-supplied *authorization identity*: the stable actor id the
    /// broker checks grants against, decoupled from the per-call routing key
    /// above (#2290). When `None`, the broker actor is the routing id itself
    /// (the CLI case, where `pane:N` is already both stable and unique). The
    /// host MCP server sets this to the stable `mcp:host` so one "Always" grant
    /// covers every concurrent connection.
    pub broker_actor_override: Option<String>,
    /// Workspace used for broker posture evaluation, resolved by the caller
    /// from the subscriber's host-established `ScopeOrigin` (stint 0724
    /// Phase D — there is no ambient service-level fallback). `None` only
    /// when the caller has no resolvable pane (e.g. an outside-pane CLI
    /// connection), in which case the broker check runs with no workspace
    /// context.
    pub workspace_root_override: Option<PathBuf>,
    /// The subscriber's host-established context id, resolved the same way
    /// as `workspace_root_override` — the viewer side of `evaluate_reach`
    /// for every stream this subscription will ever match against. `None`
    /// only when the caller has no resolvable pane; resolved to `0` (matches
    /// no real context, mirroring `AgentHost`'s pre-attach sentinel) at the
    /// point the request is classified.
    pub context_id_override: Option<u64>,
    /// Host-captured kernel ancestor-pid chain of the socket peer (stint
    /// 0636's `capture_peer_ancestry`, threaded the same way as
    /// `Notify`/`DismissNotification`'s `peer_pid`). `Some` only for the raw
    /// CLI-socket transport; when present it independently re-derives
    /// `from_pane_id` instead of trusting the client-forwarded
    /// `PLEXI_PANE_ID` value, closing the gap where any local process could
    /// otherwise claim to be calling from an arbitrary pane. `None` for
    /// transports that already establish identity another way (host MCP's
    /// bearer credential, a WASM/Python app's own host-known pane id).
    pub peer_ancestry: Option<Vec<u32>>,
    pub reply: SyncSender<HostSubscribeReply>,
}

/// A subscribe or publish request the broker answered with `Ask`: it needs an
/// explicit user decision before the effect is applied. The UI thread parks one
/// of these (holding the transport's live `reply` channel inside `action`) and
/// surfaces a host consent modal; [`HostSubscriptionService::resolve_consent`]
/// answers it.
///
/// Identity is already host-stamped here — the modal shows it, it is never
/// taken from the user's click.
pub struct PendingEventConsent {
    pub subscriber_type: ActorType,
    /// Delivery routing key — unique per waiter; the recorded subscription and
    /// its teardown key on this.
    pub subscriber_id: String,
    /// Authorization identity — stable actor the broker grant is persisted
    /// against on `AllowAlways` (#2290). Equals `subscriber_id` unless the
    /// transport decoupled them (host MCP: `mcp:host`).
    pub broker_actor_id: String,
    /// App-id namespace the streams live under.
    pub app_id: String,
    /// Stream names touched (empty = all of the app's declared streams). Drives
    /// both the consent label and the `AllowAlways` grant targets.
    pub event_names: Vec<String>,
    /// The subscriber/publisher's host-established context id, resolved once
    /// at classify time and carried in the parked struct — never re-resolved
    /// when the user later answers the modal (command-handler data must be
    /// self-contained; by the time `resolve_consent` runs, the pane that
    /// requested this may have closed).
    pub subscriber_context_id: u64,
    /// The effect to apply once allowed — owns the transport's reply channel.
    action: ConsentAction,
}

/// The terminal effect a parked [`PendingEventConsent`] performs once the user
/// allows. Each variant owns the transport's live `reply` channel, fired from
/// [`HostSubscriptionService::resolve_consent`].
enum ConsentAction {
    Subscribe {
        payload_mode: PayloadMode,
        trigger_mode: TriggerMode,
        resource_id: Option<String>,
        reply: SyncSender<HostSubscribeReply>,
    },
    Publish {
        publish: PublishAction,
        /// Pane the emitting agent lives in (host-stamped); recorded on emitted
        /// events. `0` when the connection had no pane context.
        pane_id: u64,
        reply: SyncSender<HostPublishReply>,
    },
}

impl PendingEventConsent {
    /// Human-readable target for the consent modal: `"<app> :: <events>"`.
    pub fn target_label(&self) -> String {
        let streams = if self.event_names.is_empty() {
            "* (all streams)".to_string()
        } else {
            self.event_names.join(", ")
        };
        format!("{} :: {streams}", self.app_id)
    }

    /// Verb for the consent modal copy: readers subscribe, publishers emit.
    pub fn action_verb(&self) -> &'static str {
        match self.action {
            ConsentAction::Subscribe { .. } => "read",
            ConsentAction::Publish { .. } => "publish to",
        }
    }
}

/// A CLI/agent request to *publish* to the event bus: declare stream schemas
/// under an app-id namespace, or emit an event on a declared stream. Handed
/// from a transport connection thread to the UI thread, which owns the grant
/// store and broker gate. Identity is host-stamped from the pane, never taken
/// from a CLI argument (anti-spoof, symmetrical to [`HostSubscribeRequest`]).
pub struct HostPublishRequest {
    /// App-id namespace to declare/emit under.
    pub app_id: String,
    pub action: PublishAction,
    /// Pane id stamped by the host PTY env (`PLEXI_PANE_ID`).
    pub from_pane_id: Option<u64>,
    /// Trusted host-transport override for the actor id (e.g. the host MCP
    /// server). Never sourced from an untrusted client argument.
    pub subscriber_override: Option<String>,
    /// Workspace for broker posture evaluation — see
    /// [`HostSubscribeRequest::workspace_root_override`]; same contract,
    /// publish twin.
    pub workspace_root_override: Option<PathBuf>,
    /// Context id under which the declared/emitted stream is owned — see
    /// [`HostSubscribeRequest::context_id_override`]; same contract, publish
    /// twin. Stamped onto every `AppEventRecord`/stream declaration this
    /// request produces.
    pub context_id_override: Option<u64>,
    /// See [`HostSubscribeRequest::peer_ancestry`]; same contract, publish
    /// twin.
    pub peer_ancestry: Option<Vec<u32>>,
    pub reply: SyncSender<HostPublishReply>,
}

/// What a [`HostPublishRequest`] does to the timeline once allowed.
pub enum PublishAction {
    /// Register stream schemas — routes to [`AppTimeline::declare_streams`].
    Declare(Vec<EventStreamDecl>),
    /// Record + fan out one event — routes to [`AppTimeline::record_event`].
    Emit(Box<EmittedEvent>),
}

/// The UI thread's answer to a [`HostPublishRequest`].
pub enum HostPublishReply {
    Ok {
        /// Human-readable result, e.g. `"declared stream(s): probe.tick"`.
        detail: String,
        /// The recorded event id (emit only; `None` for declare).
        event_id: Option<u64>,
    },
    Err {
        message: String,
    },
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
    timeline: Arc<Mutex<AppTimeline>>,
}

impl HostSubscriptionService {
    /// Load the host grant store + posture from `config_dir` and bind the
    /// service to `timeline` (the global timeline in production).
    ///
    /// Deliberately takes no workspace/context — stint 0724 Phase D removed
    /// the service-wide `workspace_root` field that used to be captured once
    /// at host startup (`active_workspace_root()`/cwd) and silently applied
    /// to every request regardless of which context the caller actually
    /// lives in. Every request now carries its own resolved
    /// `workspace_root_override`/`context_id_override` (or, for the raw CLI
    /// socket path, is resolved per-request by the caller from
    /// `PlexiApp::origin_for_pane` before it reaches this service) — there is
    /// no ambient fallback.
    pub fn new(config_dir: &Path, timeline: Arc<Mutex<AppTimeline>>) -> Self {
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
    /// (`agent:assistant`) or a `WASM app runtime` (its `type_id`). A connection with
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
            Some(id) => (
                req.subscriber_type_override.unwrap_or(ActorType::Agent),
                id.clone(),
            ),
            None => Self::resolve_cli_subscriber(req.from_pane_id),
        };
        // Authorization identity: the stable actor the broker grants are keyed
        // to. Defaults to the routing id (CLI: `pane:N` is both), overridden by
        // the host MCP server to `mcp:host` so grants outlive any one connection.
        let broker_actor_id = req
            .broker_actor_override
            .clone()
            .unwrap_or_else(|| subscriber_id.clone());
        // Viewer context for reachability and stream-declaration scoping —
        // stint 0724 Phase D. `0` matches no real context (mirrors
        // `AgentHost`'s pre-attach sentinel) and is only reached when the
        // caller has no resolvable pane at all.
        let subscriber_context_id = req.context_id_override.unwrap_or(0);
        // Reject undeclared streams before involving the broker so the client
        // gets a precise error instead of a generic permission denial. Scoped
        // to the subscriber's own context: a stream declared only in a
        // different context is, correctly, "not declared" from here — no
        // cross-context grant exists yet to make it visible.
        let undeclared: Vec<String> = req
            .event_names
            .iter()
            .filter(|n| !self.stream_is_declared(subscriber_context_id, &req.publisher_app_id, n))
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
            req.workspace_root_override.as_deref(),
            &req.publisher_app_id,
            subscriber_type,
            &broker_actor_id,
            &req.event_names,
        );
        match decision {
            Decision::Allow => {
                let subscription_id = record_subscription(
                    &self.timeline,
                    &req.publisher_app_id,
                    subscriber_type,
                    &subscriber_id,
                    subscriber_context_id,
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
                    broker_actor_id,
                    app_id: req.publisher_app_id,
                    event_names: req.event_names,
                    subscriber_context_id,
                    action: ConsentAction::Subscribe {
                        payload_mode: req.payload_mode,
                        trigger_mode: req.trigger_mode,
                        resource_id: req.resource_id,
                        reply: req.reply,
                    },
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
        let PendingEventConsent {
            subscriber_type,
            subscriber_id,
            broker_actor_id,
            app_id,
            event_names,
            subscriber_context_id,
            action,
        } = consent;

        if choice == ConsentChoice::AllowAlways {
            // Persist against the same operation-qualified targets the matching
            // evaluation uses: subscribe grants the bare `<app>::<stream>`
            // targets, publish grants the `publish:`-prefixed ones. A read grant
            // must never authorize a write, so the persisted target must match
            // the operation being consented to, not just the app + streams.
            let targets = match action {
                ConsentAction::Subscribe { .. } => subscription_targets(&app_id, &event_names),
                ConsentAction::Publish { .. } => publish_targets(&app_id, &event_names),
            };
            self.persist_always_grants(
                subscriber_type,
                &broker_actor_id,
                &app_id,
                &targets,
                config_dir,
            );
        }
        let scope = if choice == ConsentChoice::AllowAlways {
            "always"
        } else {
            "once"
        };

        match action {
            ConsentAction::Subscribe {
                payload_mode,
                trigger_mode,
                resource_id,
                reply,
            } => {
                if choice == ConsentChoice::Deny {
                    log::info!(
                        "event_subscriptions: user denied subscription for {subscriber_type:?} \
                         '{subscriber_id}' -> '{app_id}'"
                    );
                    let _ = reply.send(HostSubscribeReply::Err {
                        message: "subscription denied by user".to_string(),
                    });
                    return;
                }
                let subscription_id = record_subscription(
                    &self.timeline,
                    &app_id,
                    subscriber_type,
                    &subscriber_id,
                    subscriber_context_id,
                    event_names,
                    payload_mode,
                    trigger_mode,
                    resource_id,
                    GrantDuration::Session,
                );
                if reply
                    .send(HostSubscribeReply::Ok {
                        subscription_id,
                        subscriber_type,
                        subscriber_id: subscriber_id.clone(),
                    })
                    .is_err()
                {
                    let (subs, drops) = self
                        .timeline
                        .lock()
                        .unwrap()
                        .clear_subscriber(subscriber_type, &subscriber_id);
                    log::warn!(
                        "event_subscriptions: consented subscriber '{subscriber_id}' already \
                         disconnected; rolled back {subs} subscription(s), {drops} queued \
                         delivery(ies)"
                    );
                } else {
                    log::info!(
                        "event_subscriptions: user allowed ({scope}) subscription for \
                         {subscriber_type:?} '{subscriber_id}' -> '{app_id}'"
                    );
                }
            }
            ConsentAction::Publish {
                publish,
                pane_id,
                reply,
            } => {
                if choice == ConsentChoice::Deny {
                    log::info!(
                        "event_subscriptions: user denied publish for {subscriber_type:?} \
                         '{subscriber_id}' -> '{app_id}'"
                    );
                    let _ = reply.send(HostPublishReply::Err {
                        message: "publish denied by user".to_string(),
                    });
                    return;
                }
                let outcome = Self::perform_publish(
                    &self.timeline,
                    subscriber_context_id,
                    &app_id,
                    pane_id,
                    publish,
                );
                let summary = match &outcome {
                    HostPublishReply::Ok { detail, .. } => detail.clone(),
                    HostPublishReply::Err { message } => format!("failed: {message}"),
                };
                // A dropped receiver means the transport already gave up (e.g.
                // the CLI's 120s consent wait expired). An emit is irreversible
                // once recorded, so unlike the subscribe path there is nothing
                // to roll back — surface it so a publish applied after the client
                // disconnected is not silent.
                if reply.send(outcome).is_err() {
                    log::warn!(
                        "event_subscriptions: consented publisher '{subscriber_id}' already \
                         disconnected; the {scope} publish to '{app_id}' was still applied \
                         ({summary})"
                    );
                } else {
                    log::info!(
                        "event_subscriptions: user allowed ({scope}) publish for \
                         {subscriber_type:?} '{subscriber_id}' -> '{app_id}' ({summary})"
                    );
                }
            }
        }
    }

    /// Persist an `Always` `Allow` grant for every operation-qualified target
    /// the consent touched (built by the caller via [`subscription_targets`] or
    /// [`publish_targets`]), then reload so later evaluations see it. Shared by
    /// the subscribe and publish consent paths.
    fn persist_always_grants(
        &mut self,
        actor_type: ActorType,
        actor_id: &str,
        app_id: &str,
        targets: &[String],
        config_dir: &Path,
    ) {
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        for target in targets {
            self.grant_store.record(GrantRecord {
                actor_type,
                actor_id: actor_id.to_string(),
                actor_scope: ActorScope::User,
                workspace_root: None,
                target_type: TargetType::AppEventStream,
                target_id: target.clone(),
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
            "event_subscriptions: user granted ALWAYS to {actor_type:?} '{actor_id}' -> '{app_id}'"
        );
    }

    /// Resolve host-stamped identity, (for emit) reject an undeclared stream,
    /// broker-gate on `TargetType::AppEventStream`, and — on `Allow` — apply the
    /// publish to the timeline now. The publish twin of
    /// [`Self::classify_subscribe_request`].
    ///
    /// - `Allow` → declare/emit now and send the reply on `req.reply`.
    /// - `Deny` / undeclared stream → send `Err` on `req.reply` now.
    /// - `Ask` → apply nothing; return `Some(consent)` for the caller to park.
    ///
    /// Returns `None` whenever the request was answered immediately.
    pub fn classify_publish_request(
        &self,
        mut req: HostPublishRequest,
    ) -> Option<PendingEventConsent> {
        let (actor_type, actor_id) = match &req.subscriber_override {
            Some(id) => (ActorType::Agent, id.clone()),
            None => Self::resolve_cli_subscriber(req.from_pane_id),
        };
        // Host-stamp the emitter identity so an event's provenance is the
        // agent/pane, never the app-id namespace it publishes under.
        if let PublishAction::Emit(ev) = &mut req.action {
            ev.actor_id = Some(actor_id.clone());
        }
        // Owning context for the declared/emitted stream — stint 0724 Phase
        // D. `0` matches no real context, reached only when the caller has
        // no resolvable pane.
        let context_id = req.context_id_override.unwrap_or(0);
        // Stream names this request touches — the broker targets and the label.
        let event_names: Vec<String> = match &req.action {
            PublishAction::Declare(decls) => decls.iter().map(|d| d.name.clone()).collect(),
            PublishAction::Emit(ev) => vec![ev.event.clone()],
        };
        // Reject an emit on an undeclared stream before the broker so the client
        // gets a precise error and the user is never prompted for a doomed emit
        // (mirrors the subscribe undeclared-stream check). Scoped to this
        // request's own context, matching what `record_event` will enforce.
        if let PublishAction::Emit(ev) = &req.action {
            if !self.stream_is_declared(context_id, &req.app_id, &ev.event) {
                let _ = req.reply.send(HostPublishReply::Err {
                    message: format!(
                        "app '{}' has not declared stream '{}' in this context — declare it first",
                        req.app_id, ev.event
                    ),
                });
                return None;
            }
        }
        let decision = evaluate_publish(
            &self.grant_store,
            self.posture.as_ref(),
            req.workspace_root_override.as_deref(),
            &req.app_id,
            actor_type,
            &actor_id,
            &event_names,
        );
        let pane_id = req.from_pane_id.unwrap_or(0);
        match decision {
            Decision::Allow => {
                let outcome = Self::perform_publish(
                    &self.timeline,
                    context_id,
                    &req.app_id,
                    pane_id,
                    req.action,
                );
                let _ = req.reply.send(outcome);
                None
            }
            Decision::Deny => {
                let _ = req.reply.send(HostPublishReply::Err {
                    message: "blocked by broker: deny".to_string(),
                });
                None
            }
            Decision::Ask => {
                log::info!(
                    "event_subscriptions: publish to '{}' for {actor_type:?} '{actor_id}' \
                     awaiting user consent",
                    req.app_id
                );
                Some(PendingEventConsent {
                    subscriber_type: actor_type,
                    // Publish is one-shot with no long-poll, so routing and
                    // authorization identity are the same emitter id.
                    broker_actor_id: actor_id.clone(),
                    subscriber_id: actor_id,
                    app_id: req.app_id,
                    event_names,
                    subscriber_context_id: context_id,
                    action: ConsentAction::Publish {
                        publish: req.action,
                        pane_id,
                        reply: req.reply,
                    },
                })
            }
        }
    }

    /// Apply a [`PublishAction`] to the timeline, translating the timeline's
    /// `Result` into a [`HostPublishReply`]. Performs no broker check — the
    /// caller has already established consent.
    fn perform_publish(
        timeline: &Arc<Mutex<AppTimeline>>,
        context_id: u64,
        app_id: &str,
        pane_id: u64,
        action: PublishAction,
    ) -> HostPublishReply {
        let mut timeline = timeline.lock().unwrap();
        match action {
            PublishAction::Declare(decls) => {
                match timeline.declare_streams(context_id, app_id, decls) {
                    Ok(names) => HostPublishReply::Ok {
                        detail: format!("declared stream(s): {}", names.join(", ")),
                        event_id: None,
                    },
                    Err(message) => HostPublishReply::Err { message },
                }
            }
            PublishAction::Emit(ev) => {
                match timeline.record_event(context_id, app_id, pane_id, *ev) {
                    Ok(outcome) => HostPublishReply::Ok {
                        detail: format!(
                            "recorded event {} ({} delivery(ies) queued)",
                            outcome.event_id, outcome.deliveries_queued
                        ),
                        event_id: Some(outcome.event_id),
                    },
                    Err(message) => HostPublishReply::Err { message },
                }
            }
        }
    }

    /// Whether `app_id` has declared `stream_name`. Used to reject subscribe
    /// requests for undeclared streams before they reach the broker.
    pub fn stream_is_declared(&self, context_id: u64, app_id: &str, stream_name: &str) -> bool {
        self.timeline
            .lock()
            .unwrap()
            .has_stream(context_id, app_id, stream_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_protocol::{AppEventActor, EventStreamDecl};
    use crate::broker::{ActorScope, Decision, GrantRecord, GrantSource, ResourceScope};
    use crate::host::app_timeline::EmittedEvent;

    /// Fixed owning/viewer context for tests that don't exercise
    /// cross-context behavior (that's `app_timeline.rs`'s job) — keeps every
    /// existing same-context assertion unchanged.
    const CTX: u64 = 1;

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
            CTX,
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
            Some(Path::new("/tmp/ws")),
            &timeline,
            "event-probe",
            ActorType::Agent,
            "pane:7",
            CTX,
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
            Some(Path::new("/tmp/ws")),
            &timeline,
            "event-probe",
            ActorType::Agent,
            "pane:7",
            CTX,
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
        // Grant both the pane-derived (CLI) and override (MCP) identities, for
        // both read (subscribe) and write (publish) targets — the two are
        // distinct grant targets and never satisfy each other.
        store.record(allow_grant("pane:7", "event-probe::probe.tick"));
        store.record(allow_grant("mcp:host", "event-probe::probe.tick"));
        store.record(allow_grant("pane:7", "publish:event-probe::probe.tick"));
        store.record(allow_grant("mcp:host", "publish:event-probe::probe.tick"));
        HostSubscriptionService::new_for_test(store, timeline)
    }

    #[test]
    fn app_subscription_uses_app_actor_for_broker_and_pane_for_delivery() {
        let timeline = timeline_with_stream();
        let mut store = GrantStore::default();
        let mut grant = allow_grant("notes-app", "event-probe::probe.tick");
        grant.actor_type = ActorType::App;
        store.record(grant);
        let service = HostSubscriptionService::new_for_test(store, Arc::clone(&timeline));
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        let request = HostSubscribeRequest {
            publisher_app_id: "event-probe".to_string(),
            event_names: vec!["probe.tick".to_string()],
            payload_mode: PayloadMode::Full,
            trigger_mode: TriggerMode::Conversation,
            resource_id: None,
            from_pane_id: Some(41),
            subscriber_override: Some(app_subscriber_id(41)),
            subscriber_type_override: Some(ActorType::App),
            broker_actor_override: Some("notes-app".to_string()),
            workspace_root_override: Some(PathBuf::from("/workspace/notes")),
            context_id_override: Some(CTX),
            peer_ancestry: None,
            reply,
        };

        assert!(service.classify_subscribe_request(request).is_none());
        let HostSubscribeReply::Ok {
            subscriber_type,
            subscriber_id,
            ..
        } = receiver.recv().unwrap()
        else {
            panic!("app subscription should be allowed");
        };
        assert_eq!(subscriber_type, ActorType::App);
        assert_eq!(subscriber_id, "app-pane:41");
        let timeline = timeline.lock().unwrap();
        let subscription = &timeline.subscriptions()[0];
        assert_eq!(subscription.subscriber_type, ActorType::App);
        assert_eq!(subscription.subscriber_id, "app-pane:41");
    }

    fn subscribe_request(
        event_names: Vec<String>,
        payload_mode: PayloadMode,
        override_id: Option<&str>,
    ) -> (
        HostSubscribeRequest,
        std::sync::mpsc::Receiver<HostSubscribeReply>,
    ) {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let req = HostSubscribeRequest {
            publisher_app_id: "event-probe".to_string(),
            event_names,
            payload_mode,
            trigger_mode: TriggerMode::Conversation,
            resource_id: None,
            from_pane_id: Some(7),
            subscriber_override: override_id.map(String::from),
            subscriber_type_override: None,
            broker_actor_override: None,
            workspace_root_override: Some(PathBuf::from("/tmp/ws")),
            context_id_override: Some(CTX),
            peer_ancestry: None,
            reply: tx,
        };
        (req, rx)
    }

    /// A subscribe request that decouples the delivery routing key from the
    /// broker actor id (the host-MCP shape): grants are checked against
    /// `broker_actor`, deliveries route to the unique `delivery_id` (#2290).
    fn subscribe_request_decoupled(
        delivery_id: &str,
        broker_actor: &str,
    ) -> (
        HostSubscribeRequest,
        std::sync::mpsc::Receiver<HostSubscribeReply>,
    ) {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let req = HostSubscribeRequest {
            publisher_app_id: "event-probe".to_string(),
            event_names: vec!["probe.tick".to_string()],
            payload_mode: PayloadMode::Full,
            trigger_mode: TriggerMode::Conversation,
            resource_id: None,
            from_pane_id: None,
            subscriber_override: Some(delivery_id.to_string()),
            subscriber_type_override: None,
            broker_actor_override: Some(broker_actor.to_string()),
            workspace_root_override: Some(PathBuf::from("/tmp/ws")),
            context_id_override: Some(CTX),
            peer_ancestry: None,
            reply: tx,
        };
        (req, rx)
    }

    /// Two concurrent host-MCP waiters share the stable broker actor `mcp:host`
    /// but mint distinct delivery keys. Each must receive its own delivery, and
    /// tearing one down must leave the other live (#2290).
    #[test]
    fn concurrent_mcp_subscribers_do_not_collide() {
        let timeline = timeline_with_stream();
        let svc = granted_service(Arc::clone(&timeline));

        let (req_a, rx_a) = subscribe_request_decoupled("mcp:host:aaaa", "mcp:host");
        let (req_b, rx_b) = subscribe_request_decoupled("mcp:host:bbbb", "mcp:host");
        assert!(svc.classify_subscribe_request(req_a).is_none());
        assert!(svc.classify_subscribe_request(req_b).is_none());
        let (ta, sa) = match rx_a.recv().unwrap() {
            HostSubscribeReply::Ok {
                subscriber_type,
                subscriber_id,
                ..
            } => (subscriber_type, subscriber_id),
            HostSubscribeReply::Err { message } => panic!("subscribe A failed: {message}"),
        };
        let (tb, sb) = match rx_b.recv().unwrap() {
            HostSubscribeReply::Ok {
                subscriber_type,
                subscriber_id,
                ..
            } => (subscriber_type, subscriber_id),
            HostSubscribeReply::Err { message } => panic!("subscribe B failed: {message}"),
        };
        // The reply echoes the unique routing key, not the shared broker actor.
        assert_eq!(sa, "mcp:host:aaaa");
        assert_eq!(sb, "mcp:host:bbbb");
        assert_eq!(timeline.lock().unwrap().subscriptions().len(), 2);

        // One emitted event fans out to both independent subscriptions.
        timeline
            .lock()
            .unwrap()
            .record_event(CTX, "event-probe", 7, probe_event(1))
            .expect("emit should record");

        // Tear down A. B's subscription and its queued delivery must survive.
        let (subs, _drops) = timeline.lock().unwrap().clear_subscriber(ta, &sa);
        assert_eq!(subs, 1, "clearing A removes exactly A's subscription");
        assert_eq!(
            timeline.lock().unwrap().subscriptions().len(),
            1,
            "B's subscription must remain after A is cleared"
        );
        let deliveries_b = timeline.lock().unwrap().take_deliveries_for(tb, &sb);
        assert_eq!(deliveries_b.len(), 1, "B still receives its own delivery");
        assert_eq!(deliveries_b[0].event, "probe.tick");
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
        let (req, rx) = subscribe_request(
            vec!["probe.tick".to_string()],
            PayloadMode::Full,
            Some("mcp:host"),
        );
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
        let consent = svc
            .classify_subscribe_request(req)
            .expect("Ask must park a consent");
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
        let mut svc = HostSubscriptionService::new(dir.path(), Arc::clone(&timeline));
        let (req, rx) = subscribe_request(vec!["probe.tick".to_string()], PayloadMode::Full, None);
        let consent = svc
            .classify_subscribe_request(req)
            .expect("Ask must park a consent");
        svc.resolve_consent(consent, ConsentChoice::AllowAlways, dir.path());
        assert!(matches!(rx.recv().unwrap(), HostSubscribeReply::Ok { .. }));

        // Fresh service over the same profile dir: the persisted grant makes the
        // next subscribe pass inline (no parked consent).
        let svc2 = HostSubscriptionService::new(dir.path(), Arc::clone(&timeline));
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
            .record_event(CTX, "event-probe", 7, probe_event(3))
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
            .record_event(CTX, "event-probe", 7, probe_event(4))
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
            .record_event(CTX, "event-probe", 7, probe_event(1))
            .expect("emit should record");
        let deliveries = timeline.lock().unwrap().take_deliveries_for(stype, &sid);
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].summary.as_deref(), Some("Probe tick 1"));
        assert_eq!(deliveries[0].payload, None);
    }

    // ── Publish (declare + emit) ────────────────────────────────────────────

    fn publish_request(
        app_id: &str,
        action: PublishAction,
        override_id: Option<&str>,
    ) -> (
        HostPublishRequest,
        std::sync::mpsc::Receiver<HostPublishReply>,
    ) {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let req = HostPublishRequest {
            app_id: app_id.to_string(),
            action,
            from_pane_id: Some(7),
            subscriber_override: override_id.map(String::from),
            workspace_root_override: Some(PathBuf::from("/tmp/ws")),
            context_id_override: Some(CTX),
            peer_ancestry: None,
            reply: tx,
        };
        (req, rx)
    }

    /// A granted emit records the event and host-stamps the emitter identity
    /// (`pane:7`) — never letting `actor_id` default to the app-id namespace.
    #[test]
    fn publish_emit_records_and_host_stamps_actor() {
        let timeline = timeline_with_stream();
        let svc = granted_service(Arc::clone(&timeline));
        let (req, rx) = publish_request(
            "event-probe",
            PublishAction::Emit(Box::new(probe_event(3))),
            None,
        );
        assert!(svc.classify_publish_request(req).is_none());
        match rx.recv().unwrap() {
            HostPublishReply::Ok { event_id, .. } => assert_eq!(event_id, Some(1)),
            HostPublishReply::Err { message } => panic!("emit should record: {message}"),
        }
        let t = timeline.lock().unwrap();
        assert_eq!(t.events().len(), 1);
        assert_eq!(t.events()[0].actor_id, "pane:7");
    }

    /// The host-stamped override identity (MCP) is honoured for publish too.
    #[test]
    fn publish_honours_override_identity() {
        let timeline = timeline_with_stream();
        let svc = granted_service(Arc::clone(&timeline));
        let (req, rx) = publish_request(
            "event-probe",
            PublishAction::Emit(Box::new(probe_event(5))),
            Some("mcp:host"),
        );
        assert!(svc.classify_publish_request(req).is_none());
        assert!(matches!(rx.recv().unwrap(), HostPublishReply::Ok { .. }));
        assert_eq!(timeline.lock().unwrap().events()[0].actor_id, "mcp:host");
    }

    /// Emitting on a stream the app never declared is refused before the broker,
    /// with a precise error and nothing recorded.
    #[test]
    fn publish_emit_rejects_undeclared_stream() {
        let timeline = timeline_with_stream();
        let svc = granted_service(Arc::clone(&timeline));
        let mut ev = probe_event(1);
        ev.event = "nope.stream".to_string();
        let (req, rx) = publish_request("event-probe", PublishAction::Emit(Box::new(ev)), None);
        assert!(svc.classify_publish_request(req).is_none());
        match rx.recv().unwrap() {
            HostPublishReply::Err { message } => assert!(message.contains("not declared")),
            HostPublishReply::Ok { .. } => panic!("undeclared emit must be refused"),
        }
        assert!(timeline.lock().unwrap().events().is_empty());
    }

    /// Default `Ask` parks a publish consent (verb "publish to") instead of
    /// declaring; `AllowOnce` then declares the stream.
    #[test]
    fn publish_declare_ask_then_allow_once_declares() {
        let timeline = Arc::new(Mutex::new(AppTimeline::default()));
        let mut svc =
            HostSubscriptionService::new_for_test(GrantStore::default(), Arc::clone(&timeline));
        let decls = vec![EventStreamDecl {
            name: "foo.stream".to_string(),
            schema: serde_json::json!({"type": "object"}),
            description: None,
        }];
        let (req, rx) = publish_request("declare-probe", PublishAction::Declare(decls), None);
        let consent = svc
            .classify_publish_request(req)
            .expect("Ask must park a consent");
        assert_eq!(consent.subscriber_id, "pane:7");
        assert_eq!(consent.action_verb(), "publish to");
        assert!(timeline
            .lock()
            .unwrap()
            .declared_streams(CTX, "declare-probe")
            .is_empty());

        svc.resolve_consent(consent, ConsentChoice::AllowOnce, Path::new("/tmp/ws"));
        match rx.recv().unwrap() {
            HostPublishReply::Ok { detail, .. } => assert!(detail.contains("foo.stream")),
            HostPublishReply::Err { message } => panic!("allow-once should declare: {message}"),
        }
        assert_eq!(
            timeline
                .lock()
                .unwrap()
                .declared_streams(CTX, "declare-probe")
                .len(),
            1
        );
    }

    /// `Deny` answers the transport with a permission error and records nothing.
    #[test]
    fn publish_deny_refuses_and_records_nothing() {
        let timeline = timeline_with_stream();
        let mut svc =
            HostSubscriptionService::new_for_test(GrantStore::default(), Arc::clone(&timeline));
        let (req, rx) = publish_request(
            "event-probe",
            PublishAction::Emit(Box::new(probe_event(1))),
            None,
        );
        let consent = svc
            .classify_publish_request(req)
            .expect("Ask must park a consent");
        svc.resolve_consent(consent, ConsentChoice::Deny, Path::new("/tmp/ws"));
        match rx.recv().unwrap() {
            HostPublishReply::Err { message } => assert!(message.contains("denied by user")),
            HostPublishReply::Ok { .. } => panic!("deny must refuse"),
        }
        assert!(timeline.lock().unwrap().events().is_empty());
    }

    /// `AllowAlways` persists an `Allow` grant: a fresh service over the same
    /// profile dir emits without prompting (classify answers inline).
    #[test]
    fn publish_allow_always_persists_grant() {
        let dir = tempfile::tempdir().unwrap();
        let timeline = timeline_with_stream();
        let mut svc = HostSubscriptionService::new(dir.path(), Arc::clone(&timeline));
        let (req, rx) = publish_request(
            "event-probe",
            PublishAction::Emit(Box::new(probe_event(1))),
            None,
        );
        let consent = svc
            .classify_publish_request(req)
            .expect("Ask must park a consent");
        svc.resolve_consent(consent, ConsentChoice::AllowAlways, dir.path());
        assert!(matches!(rx.recv().unwrap(), HostPublishReply::Ok { .. }));

        let svc2 = HostSubscriptionService::new(dir.path(), Arc::clone(&timeline));
        let (req2, rx2) = publish_request(
            "event-probe",
            PublishAction::Emit(Box::new(probe_event(2))),
            None,
        );
        assert!(
            svc2.classify_publish_request(req2).is_none(),
            "persisted ALWAYS grant must publish inline, not re-prompt"
        );
        assert!(matches!(rx2.recv().unwrap(), HostPublishReply::Ok { .. }));
        assert_eq!(timeline.lock().unwrap().events().len(), 2);
    }

    /// A persisted subscribe (read) `AllowAlways` grant must NOT authorize a
    /// publish: the emit path still parks a consent (broker `Ask`), because read
    /// and write use distinct grant targets.
    #[test]
    fn subscribe_grant_does_not_authorize_publish() {
        let dir = tempfile::tempdir().unwrap();
        let timeline = timeline_with_stream();
        let mut svc = HostSubscriptionService::new(dir.path(), Arc::clone(&timeline));

        // Grant a persistent subscribe (read) Allow-Always.
        let (sub_req, sub_rx) =
            subscribe_request(vec!["probe.tick".to_string()], PayloadMode::Full, None);
        let consent = svc
            .classify_subscribe_request(sub_req)
            .expect("Ask must park a subscribe consent");
        svc.resolve_consent(consent, ConsentChoice::AllowAlways, dir.path());
        assert!(matches!(
            sub_rx.recv().unwrap(),
            HostSubscribeReply::Ok { .. }
        ));

        // A publish from the same identity must still be gated (Ask → parked),
        // not silently authorized by the read grant.
        let svc2 = HostSubscriptionService::new(dir.path(), Arc::clone(&timeline));
        let (pub_req, _pub_rx) = publish_request(
            "event-probe",
            PublishAction::Emit(Box::new(probe_event(1))),
            None,
        );
        assert!(
            svc2.classify_publish_request(pub_req).is_some(),
            "a read grant must not authorize a publish"
        );
    }

    /// A persisted publish (write) `AllowAlways` grant must NOT authorize a
    /// subscribe: the subscribe path still parks a consent (broker `Ask`).
    #[test]
    fn publish_grant_does_not_authorize_subscribe() {
        let dir = tempfile::tempdir().unwrap();
        let timeline = timeline_with_stream();
        let mut svc = HostSubscriptionService::new(dir.path(), Arc::clone(&timeline));

        // Grant a persistent publish (write) Allow-Always.
        let (pub_req, pub_rx) = publish_request(
            "event-probe",
            PublishAction::Emit(Box::new(probe_event(1))),
            None,
        );
        let consent = svc
            .classify_publish_request(pub_req)
            .expect("Ask must park a publish consent");
        svc.resolve_consent(consent, ConsentChoice::AllowAlways, dir.path());
        assert!(matches!(
            pub_rx.recv().unwrap(),
            HostPublishReply::Ok { .. }
        ));

        // A subscribe from the same identity must still be gated (Ask → parked).
        let svc2 = HostSubscriptionService::new(dir.path(), Arc::clone(&timeline));
        let (sub_req, _sub_rx) =
            subscribe_request(vec!["probe.tick".to_string()], PayloadMode::Full, None);
        assert!(
            svc2.classify_subscribe_request(sub_req).is_some(),
            "a write grant must not authorize a subscribe"
        );
    }
}
