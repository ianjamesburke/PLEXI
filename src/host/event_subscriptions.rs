//! Host-owned event subscription + publish core.
//!
//! One permission model, one delivery lifecycle, one schema. Both
//! agent-facing transports — the CLI NDJSON stream (`plexi events
//! subscribe`/`emit`/`declare`) and the host-level MCP server — wrap this
//! module. They are transports only, never second event buses.
//!
//! CLI/agent *publish* (declare + emit) is gated symmetrically to subscribe:
//! a foreign identity is broker-checked on `TargetType::PluginEventStream` the
//! first time and, on `AllowAlways`, its grant persists — see
//! [`HostSubscriptionService::classify_publish_request`]. Read and write use
//! distinct grant targets (publish targets carry a `publish:` prefix), so an
//! Allow-Always granted on a read prompt never authorizes an emit, nor the
//! reverse.
//!
//! - [`HostSubscriptionService`] is the single broker-gated path that turns a
//!   subscribe request into a [`SubscriptionRecord`]. Python and Rust WASM plugin
//!   runtimes use it alongside CLI and host MCP transports.
//! - The service owns the host grant store, posture, and global timeline. Every
//!   caller supplies a host-stamped delivery identity; plugin runtimes also supply
//!   their host-known plugin identity and workspace for authorization.

use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};

use crate::broker::principal::Principal;
use crate::broker::{
    ActorScope, ActorType, Decision, GrantDuration, GrantRecord, GrantSource, GrantStore,
    PermissionPosture, PermissionRequest,
};
use crate::host::event_log;
use crate::host::plugin_timeline::{EmittedEvent, PluginTimeline, SubscriptionRecord};
use crate::protocol::{EventStreamDecl, PayloadMode, TriggerMode};

pub fn plugin_subscriber_id(pane_id: u64) -> String {
    format!("app-pane:{pane_id}")
}

/// Evaluate broker grants for a subscription and, on a unanimous `Allow`,
/// record it in `timeline`. One `TargetType::PluginEventStream` evaluation per
/// event name (`"<plugin_id>::<event>"`, or `"<plugin_id>::*"` for all streams);
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
    timeline: &Arc<Mutex<PluginTimeline>>,
    publisher_plugin_id: &str,
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
        publisher_plugin_id,
        &Principal::from_actor(subscriber_type, subscriber_id, None)
            .expect("test subscriber is a requesting principal"),
        ActorScope::User,
        None,
        &event_names,
    );
    if decision != Decision::Allow {
        log::info!(
            "event_subscriptions: subscription to '{publisher_plugin_id}' events for \
             {subscriber_type:?} '{subscriber_id}' blocked by broker ({})",
            decision.as_str()
        );
        return Err(decision);
    }
    Ok(record_subscription(
        timeline,
        publisher_plugin_id,
        subscriber_type,
        subscriber_id,
        subscriber_context_id,
        event_names,
        payload_mode,
        trigger_mode,
        resource_id,
        duration,
        crate::broker::permit::mint_host(
            "test",
            crate::broker::permit::DeliverEvents::target(
                subscriber_type,
                subscriber_id,
                publisher_plugin_id,
            ),
        ),
    ))
}

/// The broker target ids a subscription (read) touches: one `"<plugin>::<event>"`
/// per concrete member. An all-streams request is expanded to its members
/// before it gets here, so no synthetic `"<plugin>::*"` target is ever
/// decided for a read (stint 1632).
///
/// Read and write grants must never satisfy each other: an Allow-Always the user
/// granted on a read prompt must not silently authorize an emit, and vice versa.
/// Subscribe targets are the bare `"<plugin>::<event>"` form; publish targets carry
/// a `publish:` prefix ([`publish_targets`]) so a grant matches only the
/// operation it was granted for.
fn subscription_targets(publisher_plugin_id: &str, event_names: &[String]) -> Vec<String> {
    event_names
        .iter()
        .map(|n| format!("{publisher_plugin_id}::{n}"))
        .collect()
}

/// The broker target ids a publish (declare/emit) touches — the subscribe
/// targets with a `publish:` prefix so publish grants and subscribe grants are
/// distinct and can never match each other (see [`subscription_targets`]).
fn publish_targets(plugin_id: &str, event_names: &[String]) -> Vec<String> {
    if event_names.is_empty() {
        return vec![format!("publish:{plugin_id}::*")];
    }
    subscription_targets(plugin_id, event_names)
        .into_iter()
        .map(|t| format!("publish:{t}"))
        .collect()
}

/// Run the broker over every target a subscription touches and return the
/// strictest decision (`Deny` > `Ask` > `Allow`). Records nothing — callers
/// decide what to do with the verdict (record on `Allow`, prompt on `Ask`).
/// Test seam; host callers use [`evaluate_subscription_supported`] so the
/// permit can bind the grants an allow rests on.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn evaluate_subscription(
    grant_store: &GrantStore,
    posture: Option<&PermissionPosture>,
    workspace_root: Option<&Path>,
    publisher_plugin_id: &str,
    subscriber: &Principal,
    actor_scope: ActorScope,
    session_id: Option<&str>,
    event_names: &[String],
) -> Decision {
    evaluate_subscription_supported(
        grant_store,
        posture,
        workspace_root,
        publisher_plugin_id,
        subscriber,
        actor_scope,
        session_id,
        event_names,
    )
    .0
}

/// [`evaluate_subscription`] with the grant rows an `Allow` rests on, which
/// the delivery permit binds so revoking any of them stops delivery.
#[allow(clippy::too_many_arguments)]
pub(crate) fn evaluate_subscription_supported(
    grant_store: &GrantStore,
    posture: Option<&PermissionPosture>,
    workspace_root: Option<&Path>,
    publisher_plugin_id: &str,
    subscriber: &Principal,
    actor_scope: ActorScope,
    session_id: Option<&str>,
    event_names: &[String],
) -> (Decision, Vec<String>) {
    evaluate_targets(
        grant_store,
        posture,
        workspace_root,
        subscriber,
        actor_scope,
        session_id,
        "event.subscribe",
        &subscription_targets(publisher_plugin_id, event_names),
    )
}

/// Publish twin of [`evaluate_subscription`]: evaluate the broker over the
/// `publish:`-prefixed targets so a subscribe grant never authorizes a publish.
#[allow(clippy::too_many_arguments)]
fn evaluate_publish(
    grant_store: &GrantStore,
    posture: Option<&PermissionPosture>,
    workspace_root: Option<&Path>,
    plugin_id: &str,
    publisher: &Principal,
    actor_scope: ActorScope,
    session_id: Option<&str>,
    event_names: &[String],
) -> Decision {
    evaluate_targets(
        grant_store,
        posture,
        workspace_root,
        publisher,
        actor_scope,
        session_id,
        "event.publish",
        &publish_targets(plugin_id, event_names),
    )
    .0
}

/// Run the broker over an explicit target-id list and return the strictest
/// decision (`Deny` > `Ask` > `Allow`) with the `Allow` rows it rests on. The shared core of
/// [`evaluate_subscription`] and [`evaluate_publish`] — they differ only in how
/// they build the target ids.
#[allow(clippy::too_many_arguments)]
fn evaluate_targets(
    grant_store: &GrantStore,
    posture: Option<&PermissionPosture>,
    workspace_root: Option<&Path>,
    principal: &Principal,
    actor_scope: ActorScope,
    session_id: Option<&str>,
    operation: &str,
    targets: &[String],
) -> (Decision, Vec<String>) {
    let trace = |target: &str, decision: Decision, mechanism: &str| {
        crate::broker::principal::AdmissionTrace {
            principal,
            effect: &format!("{operation}:{target}"),
            resource_class: "app_event_stream",
            decision: decision.as_str(),
            mechanism,
            call_id: "",
            outcome_ref: "",
        }
        .emit();
    };
    let Some(workspace) = workspace_root else {
        for target in targets {
            trace(target, Decision::Deny, "missing_workspace");
        }
        return (Decision::Deny, Vec::new());
    };
    let mechanism = if posture.is_some() {
        "grant_store+posture"
    } else {
        "grant_store"
    };
    let mut strictest = Decision::Allow;
    let mut supporting = Vec::new();
    for target in targets {
        let binding = crate::broker::ExactBinding::event_stream(
            principal.actor_type(),
            principal.actor_id(),
            actor_scope,
            target,
            workspace,
            session_id.map(str::to_string),
        );
        let req = PermissionRequest::exact(&binding);
        let decision = grant_store.evaluate(&req, posture);
        trace(target, decision, mechanism);
        match decision {
            Decision::Allow => supporting.extend(grant_store.supporting_allows(&req)),
            Decision::Deny => strictest = Decision::Deny,
            Decision::Ask => {
                if strictest != Decision::Deny {
                    strictest = Decision::Ask;
                }
            }
        }
    }
    (strictest, supporting)
}

/// Add a subscription to `timeline` and return its id. Performs no broker
/// check: the caller has already established consent (a unanimous `Allow`
/// grant or an interactive user approval) and holds the broker's delivery
/// permit for this subscriber and publisher.
#[allow(clippy::too_many_arguments)]
pub fn record_subscription(
    timeline: &Arc<Mutex<PluginTimeline>>,
    publisher_plugin_id: &str,
    subscriber_type: ActorType,
    subscriber_id: &str,
    subscriber_context_id: u64,
    event_names: Vec<String>,
    payload_mode: PayloadMode,
    trigger_mode: TriggerMode,
    resource_id: Option<String>,
    duration: GrantDuration,
    permit: crate::host::plugin_timeline::DeliveryPermit,
) -> String {
    let subscription_id = format!("sub-{}", uuid::Uuid::new_v4());
    log::info!(
        "event_subscriptions: recorded subscription {subscription_id} for {subscriber_type:?} \
         '{subscriber_id}' (context={subscriber_context_id}) -> '{publisher_plugin_id}' \
         {event_names:?} (payload={payload_mode:?}, trigger={trigger_mode:?}) {permit:?}"
    );
    timeline
        .lock()
        .unwrap()
        .add_subscription(SubscriptionRecord {
            subscription_id: subscription_id.clone(),
            subscriber_type,
            subscriber_id: subscriber_id.to_string(),
            plugin_id: publisher_plugin_id.to_string(),
            event_names,
            payload_mode,
            trigger_mode,
            resource_id,
            duration,
            subscriber_context_id,
            created_at: event_log::now_timestamp(),
            permit,
        });
    subscription_id
}

/// Mint the delivery permit for an admitted subscriber, bound to the grants
/// that allowed it. A pane subscriber must still be at the incarnation and
/// revision it was admitted under.
pub(crate) fn mint_delivery_permit(
    authority: &SubscriberAuthority,
    subscriber_type: ActorType,
    subscriber_id: &str,
    publisher_plugin_id: &str,
    grants: crate::broker::permit::Grants,
) -> Result<crate::host::plugin_timeline::DeliveryPermit, String> {
    use crate::broker::permit::{mint_pane, mint_unbound, DeliverEvents};
    let target = DeliverEvents::target(subscriber_type, subscriber_id, publisher_plugin_id);
    match authority {
        SubscriberAuthority::Pane(admitted) => mint_pane(admitted.clone(), target, grants),
        SubscriberAuthority::Plugin => mint_unbound("plugin", target, grants),
        SubscriberAuthority::Run => mint_unbound("run", target, grants),
        SubscriberAuthority::External => mint_unbound("external", target, grants),
        SubscriberAuthority::Sealed => {
            Err("a sealed pane authority holds no delivery permit".into())
        }
    }
}

/// An in-memory subscribe request handed from a transport's connection thread
/// to the host UI thread. The UI thread owns the [`HostSubscriptionService`]
/// (grant store + pane→identity trust), so identity resolution and the broker
/// check happen there; the connection thread then waits on timeline publication
/// and streams its queued deliveries. Not serialized — `reply` is a live
/// channel.
pub struct HostSubscribeRequest {
    pub publisher_plugin_id: String,
    /// Empty = subscribe to all of the plugin's declared streams.
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
    /// Who the subscriber is, resolved by the transport (stint 1632). A pane
    /// subscriber is evaluated under the meet of its grant's ceiling and the
    /// host posture, so a pane's explicit deny is never weakened to a prompt
    /// or a grant; there is no pane variant without its grant.
    pub subscriber_authority: SubscriberAuthority,
    /// The subscriber's host-established context id, resolved the same way
    /// as `workspace_root_override` — the viewer side of `evaluate_reach`
    /// for every stream this subscription will ever match against. `None`
    /// only when the caller has no resolvable pane; resolved to `0` (matches
    /// no real context, mirroring `AgentHost`'s pre-attach sentinel) at the
    /// point the request is classified.
    pub context_id_override: Option<u64>,
    /// The command-socket caller the admission entry classified (stint
    /// 1285). `Some` only for the raw CLI-socket transport, whose
    /// `from_pane_id` is then always `None`: no wire field names a pane.
    /// `None` for transports that establish identity another way (host
    /// MCP's bearer credential, a plugin's own host-known pane id).
    pub socket_caller: Option<crate::broker::caller::SocketCaller>,
    pub cancelled: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub reply: SyncSender<HostSubscribeReply>,
}

/// Who an always event-stream grant is for and what it covers; the
/// person's decision travels beside it.
struct AlwaysStreamGrant<'a> {
    actor_type: ActorType,
    actor_id: &'a str,
    plugin_id: &'a str,
    targets: &'a [String],
    workspace_root: &'a Path,
}

/// The resolved principal behind a subscribe request.
#[derive(Clone, Debug)]
pub enum SubscriberAuthority {
    /// A pane incarnation, at the scope revision the request was admitted
    /// under. The revision must still hold when consent is given and at
    /// every delivery.
    Pane(crate::host::plugin_timeline::AdmittedScope),
    /// A plugin view instance: bounded by its capabilities, not a pane grant.
    Plugin,
    /// A team run principal whose token acts for no pane.
    Run,
    /// A caller the host identified as no pane of its own.
    External,
    /// A pane caller whose authority could not be established: refused.
    Sealed,
}

impl SubscriberAuthority {
    /// The pane grant's ceiling, or a refusal for a sealed or revoked one.
    fn ceiling(&self) -> Result<Option<PermissionPosture>, String> {
        match self {
            Self::Pane(admitted) => admitted.current().and_then(|authority| {
                authority
                    .grant()
                    .posture()
                    .map(|p| p.cloned())
                    .map_err(|sealed| sealed.to_string())
            }),
            Self::Plugin | Self::Run | Self::External => Ok(None),
            Self::Sealed => Err(crate::broker::pane_authority::SealedAuthority {
                reason: crate::broker::pane_authority::SealReason::Unestablished,
            }
            .to_string()),
        }
    }

    fn admitted(&self) -> Option<crate::host::plugin_timeline::AdmittedScope> {
        match self {
            Self::Pane(admitted) => Some(admitted.clone()),
            Self::Plugin | Self::Run | Self::External | Self::Sealed => None,
        }
    }
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
    pub plugin_id: String,
    /// Concrete stream names touched; an all-streams request was expanded to
    /// its declared members at admission. Drives both the consent label and
    /// the `AllowAlways` grant targets.
    pub event_names: Vec<String>,
    /// The pane scope the request was admitted under: consent given after a
    /// scope change cannot release the effect.
    pub admitted: Option<crate::host::plugin_timeline::AdmittedScope>,
    /// The subscriber/publisher's host-established context id, resolved once
    /// at classify time and carried in the parked struct — never re-resolved
    /// when the user later answers the modal (command-handler data must be
    /// self-contained; by the time `resolve_consent` runs, the pane that
    /// requested this may have closed).
    pub subscriber_context_id: u64,
    /// Workspace the exact grant is bound to. Captured at classify time.
    pub workspace_root: PathBuf,
    /// Fresh per request (stint 1498): the consent guard arms on this, never
    /// on the request's content, so a repeated request re-arms.
    pub request_id: u64,
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
        /// Who asked, as classified; the delivery permit is minted from it
        /// once the person allows.
        authority: SubscriberAuthority,
        cancelled: Option<Arc<std::sync::atomic::AtomicBool>>,
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
    pub(crate) fn is_cancelled(&self) -> bool {
        matches!(&self.action, ConsentAction::Subscribe { cancelled: Some(flag), .. }
            if flag.load(std::sync::atomic::Ordering::Acquire))
    }

    /// Human-readable target for the consent modal: `"<plugin> :: <events>"`.
    pub fn target_label(&self) -> String {
        format!("{} :: {}", self.plugin_id, self.event_names.join(", "))
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
/// under a plugin-id namespace, or emit an event on a declared stream. Handed
/// from a transport connection thread to the UI thread, which owns the grant
/// store and broker gate. Identity is host-stamped from the pane, never taken
/// from a CLI argument (anti-spoof, symmetrical to [`HostSubscribeRequest`]).
pub struct HostPublishRequest {
    /// App-id namespace to declare/emit under.
    pub plugin_id: String,
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
    /// twin. Stamped onto every `PluginEventRecord`/stream declaration this
    /// request produces.
    pub context_id_override: Option<u64>,
    /// See [`HostSubscribeRequest::socket_caller`]; same contract, publish
    /// twin.
    pub socket_caller: Option<crate::broker::caller::SocketCaller>,
    pub reply: SyncSender<HostPublishReply>,
}

/// What a [`HostPublishRequest`] does to the timeline once allowed.
pub enum PublishAction {
    /// Register stream schemas — routes to [`PluginTimeline::declare_streams`].
    Declare(Vec<EventStreamDecl>),
    /// Record + fan out one event — routes to [`PluginTimeline::record_event`].
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

/// Host-level subscription service for non-plugin actors (CLI agents reading
/// subprocess stdout, host MCP clients). Evaluates against the profile's one
/// live grant store (the permission monitor's, so a revoke there is seen at
/// once) under the posture loaded from the profile dir, and holds a clone of
/// the global timeline handle.
///
/// Every transport routes through this so the host has one event permission
/// model, one delivery queue, and one disconnect-cleanup path. The service
/// stamps subscriber identity from host-trusted state; callers never pass an
/// identity sourced from untrusted CLI arguments.
pub struct HostSubscriptionService {
    monitor: Arc<crate::broker::gate::PermissionMonitor>,
    posture: Option<PermissionPosture>,
    timeline: Arc<Mutex<PluginTimeline>>,
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
    pub fn new(config_dir: &Path, timeline: Arc<Mutex<PluginTimeline>>) -> Self {
        // Host only (stint 1291); elsewhere no grants, so every stream asks.
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(config_dir)
            .unwrap_or_else(|error| {
                log::error!("HostSubscriptionService: no grant store: {error}");
                crate::broker::gate::PermissionMonitor::ephemeral()
            });
        let posture = PermissionPosture::load_from_config(config_dir);
        log::info!(
            "HostSubscriptionService: {} grant record(s), posture={}",
            monitor.store().records().len(),
            posture.is_some()
        );
        Self {
            monitor,
            posture,
            timeline,
        }
    }

    /// Construct a service with an explicit grant store + timeline for tests,
    /// bypassing disk loading.
    #[cfg(test)]
    pub fn new_for_test(grant_store: GrantStore, timeline: Arc<Mutex<PluginTimeline>>) -> Self {
        let monitor = crate::broker::gate::PermissionMonitor::ephemeral();
        *monitor.store() = grant_store;
        Self {
            monitor,
            posture: None,
            timeline,
        }
    }

    /// The live grant store this service evaluates against.
    #[cfg(test)]
    pub(crate) fn monitor(&self) -> &Arc<crate::broker::gate::PermissionMonitor> {
        &self.monitor
    }

    /// Reload posture from disk before evaluating a fresh subscription. The
    /// grants are the monitor's live store and need no reload.
    pub fn reload(&mut self, config_dir: &Path) {
        self.posture = PermissionPosture::load_from_config(config_dir);
    }

    /// Derive the trusted subscriber identity for a transport connection from
    /// the host-stamped pane id. CLI/MCP agents are `Agent`-typed; the id is
    /// namespaced by pane so it never collides with the assistant
    /// (`agent:assistant`) or a `WASM plugin runtime` (its `type_id`). A connection with
    /// no pane context (rare) gets a stable anonymous id.
    pub fn resolve_cli_subscriber(from_pane_id: Option<u64>) -> (ActorType, String) {
        match from_pane_id {
            Some(p) => (ActorType::Agent, format!("pane:{p}")),
            // No pane: the one stored identity of a process outside Plexi,
            // disjoint from every agent name (stint 1285).
            None => (
                ActorType::External,
                crate::broker::principal::EXTERNAL_ID.to_string(),
            ),
        }
    }

    /// Record an in-memory session-scoped `Allow` grant so a parent master pane
    /// (`broker_actor_id`, e.g. `"pane:1"`) can subscribe to its spawned child
    /// worker pane's `plexi.pane::lifecycle` stream without a human consent modal.
    pub fn allow_parent_child_lifecycle(&mut self, broker_actor_id: &str, workspace_root: &Path) {
        let target = format!(
            "{}::{}",
            crate::host::pane_lifecycle::PUBLISHER,
            crate::host::pane_lifecycle::STREAM
        );
        self.monitor.store().record(GrantRecord::event_stream_allow(
            ActorType::Agent,
            broker_actor_id,
            ActorScope::User,
            &target,
            workspace_root,
            GrantDuration::Session,
            GrantSource::Session,
            None,
        ));
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
        if req
            .cancelled
            .as_ref()
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire))
        {
            return None;
        }
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
            .filter(|n| {
                !self.stream_is_declared(subscriber_context_id, &req.publisher_plugin_id, n)
            })
            .cloned()
            .collect();
        if !undeclared.is_empty() {
            let _ = req.reply.send(HostSubscribeReply::Err {
                message: format!(
                    "plugin '{}' has not declared stream(s): {}",
                    req.publisher_plugin_id,
                    undeclared.join(", ")
                ),
            });
            return None;
        }
        // "All streams" is the declared, reachable members now, each
        // authorized as itself: a `plugin::*` grant cannot stand in for a
        // member's decision, and a stream declared later needs its own
        // admission (stint 1632).
        let mut req = req;
        if req.event_names.is_empty() {
            req.event_names = self
                .timeline
                .lock()
                .unwrap()
                .declared_stream_names(subscriber_context_id, &req.publisher_plugin_id);
            log::info!(
                "event_subscriptions: all-streams request for '{}' expanded to {:?}",
                req.publisher_plugin_id,
                req.event_names
            );
        }
        let ceiling = match req.subscriber_authority.ceiling() {
            Ok(ceiling) => ceiling,
            Err(message) => {
                log::info!("event_subscriptions: refused {subscriber_id}: {message}");
                let _ = req.reply.send(HostSubscribeReply::Err { message });
                return None;
            }
        };
        let Some(subscriber) =
            Principal::from_actor(subscriber_type, &broker_actor_id, req.from_pane_id)
        else {
            log::info!(
                "event_subscriptions: deny {subscriber_type:?} '{broker_actor_id}': not a requesting principal"
            );
            let _ = req.reply.send(HostSubscribeReply::Err {
                message: format!("'{broker_actor_id}' cannot request event subscriptions"),
            });
            return None;
        };
        let posture = match (&self.posture, &ceiling) {
            (Some(host), Some(pane)) => Some(host.clone().restricted_by(pane)),
            (host, None) => host.clone(),
            (None, Some(pane)) => Some(pane.clone()),
        };
        let (decision, supporting) = evaluate_subscription_supported(
            &self.monitor.store(),
            posture.as_ref(),
            req.workspace_root_override.as_deref(),
            &req.publisher_plugin_id,
            &subscriber,
            ActorScope::User,
            None,
            &req.event_names,
        );
        // An empty expansion authorizes zero deliveries.
        let decision = if req.event_names.is_empty() {
            Decision::Allow
        } else {
            decision
        };
        match decision {
            Decision::Allow => {
                let permit = match mint_delivery_permit(
                    &req.subscriber_authority,
                    subscriber_type,
                    &subscriber_id,
                    &req.publisher_plugin_id,
                    crate::broker::permit::Grants::live(self.monitor.clone(), supporting),
                ) {
                    Ok(permit) => permit,
                    Err(message) => {
                        log::info!("event_subscriptions: refused {subscriber_id}: {message}");
                        let _ = req.reply.send(HostSubscribeReply::Err { message });
                        return None;
                    }
                };
                let subscription_id = record_subscription(
                    &self.timeline,
                    &req.publisher_plugin_id,
                    subscriber_type,
                    &subscriber_id,
                    subscriber_context_id,
                    req.event_names,
                    req.payload_mode,
                    req.trigger_mode,
                    req.resource_id,
                    GrantDuration::Session,
                    permit,
                );
                if req
                    .reply
                    .send(HostSubscribeReply::Ok {
                        subscription_id,
                        subscriber_type,
                        subscriber_id: subscriber_id.clone(),
                    })
                    .is_err()
                {
                    self.timeline
                        .lock()
                        .unwrap()
                        .clear_subscriber(subscriber_type, &subscriber_id);
                    log::info!(
                        "event_subscriptions: disconnected before approval reply: {subscriber_id}"
                    );
                }
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
                    req.publisher_plugin_id
                );
                Some(PendingEventConsent {
                    request_id: crate::broker::consent::next_consent_request_id(),
                    subscriber_type,
                    subscriber_id,
                    broker_actor_id,
                    plugin_id: req.publisher_plugin_id,
                    event_names: req.event_names,
                    admitted: req.subscriber_authority.admitted(),
                    subscriber_context_id,
                    workspace_root: req.workspace_root_override.clone().unwrap_or_default(),
                    action: ConsentAction::Subscribe {
                        payload_mode: req.payload_mode,
                        trigger_mode: req.trigger_mode,
                        resource_id: req.resource_id,
                        authority: req.subscriber_authority,
                        cancelled: req.cancelled,
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
    /// A test standing in for the click on the consent dialog.
    #[cfg(test)]
    pub fn resolve_consent(&mut self, consent: PendingEventConsent, choice: ConsentChoice) {
        let evidence = crate::broker::consent::DecisionEvidence::for_test(
            &consent.request_id.to_string(),
            "allow_always",
        );
        self.resolve_consent_decided(consent, choice, Some(evidence));
    }

    /// Settle one consent. `AllowAlways` writes standing grants, so it needs
    /// the dialog's evidence (stint 1285); without it the consent is denied.
    pub(crate) fn resolve_consent_decided(
        &mut self,
        consent: PendingEventConsent,
        choice: ConsentChoice,
        evidence: Option<crate::broker::consent::DecisionEvidence>,
    ) {
        if consent.is_cancelled() {
            return;
        }
        let PendingEventConsent {
            request_id,
            subscriber_type,
            subscriber_id,
            broker_actor_id,
            plugin_id,
            event_names,
            admitted,
            subscriber_context_id,
            workspace_root,
            action,
        } = consent;
        // An outside process's grant lasts at most the session: its Always
        // is this one use, never a standing row (stint 1285).
        let choice = if subscriber_type == ActorType::External && choice == ConsentChoice::AllowAlways {
            log::info!("event_subscriptions: outside process Always applied as once");
            ConsentChoice::AllowOnce
        } else {
            choice
        };
        // Consent applies to the scope the request was admitted under. A
        // revoked pane or a moved scope releases nothing under it.
        if let Some(stale) = admitted.as_ref().and_then(|a| a.current().err()) {
            log::info!("event_subscriptions: consent for '{subscriber_id}' refused: {stale}");
            let message = format!("consent no longer applies: {stale}");
            match action {
                ConsentAction::Subscribe { reply, .. } => {
                    let _ = reply.send(HostSubscribeReply::Err { message });
                }
                ConsentAction::Publish { reply, .. } => {
                    let _ = reply.send(HostPublishReply::Err { message });
                }
            }
            return;
        }

        // The rows an Always choice wrote: the delivery permit rests on them,
        // so revoking one stops delivery. A once choice writes none.
        let mut granted = Vec::new();
        if choice == ConsentChoice::AllowAlways {
            // Persist against the same operation-qualified targets the matching
            // evaluation uses: subscribe grants the bare `<plugin>::<stream>`
            // targets, publish grants the `publish:`-prefixed ones. A read grant
            // must never authorize a write, so the persisted target must match
            // the operation being consented to, not just the plugin + streams.
            let targets = match action {
                ConsentAction::Subscribe { .. } => subscription_targets(&plugin_id, &event_names),
                ConsentAction::Publish { .. } => publish_targets(&plugin_id, &event_names),
            };
            let decision = evidence
                .as_ref()
                .map(|e| e.admit(&request_id.to_string(), "allow_always"));
            let persisted = match decision {
                Some(Err(error)) => Err(error),
                Some(Ok(evidence)) => self.persist_always_grants(
                    AlwaysStreamGrant {
                        actor_type: subscriber_type,
                        actor_id: &broker_actor_id,
                        plugin_id: &plugin_id,
                        targets: &targets,
                        workspace_root: &workspace_root,
                    },
                    evidence,
                ),
                None => Err("an always grant needs the person's decision".to_string()),
            };
            let persisted = persisted.map(|ids| granted = ids);
            if let Err(message) = persisted {
                match action {
                    ConsentAction::Subscribe { reply, .. } => {
                        let _ = reply.send(HostSubscribeReply::Err { message });
                    }
                    ConsentAction::Publish { reply, .. } => {
                        let _ = reply.send(HostPublishReply::Err { message });
                    }
                }
                return;
            }
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
                authority,
                cancelled: _,
                reply,
            } => {
                if choice == ConsentChoice::Deny {
                    log::info!(
                        "event_subscriptions: user denied subscription for {subscriber_type:?} \
                         '{subscriber_id}' -> '{plugin_id}'"
                    );
                    let _ = reply.send(HostSubscribeReply::Err {
                        message: "subscription denied by user".to_string(),
                    });
                    return;
                }
                let grants = if granted.is_empty() {
                    crate::broker::permit::Grants::none()
                } else {
                    crate::broker::permit::Grants::live(self.monitor.clone(), granted)
                };
                let permit = match mint_delivery_permit(
                    &authority,
                    subscriber_type,
                    &subscriber_id,
                    &plugin_id,
                    grants,
                ) {
                    Ok(permit) => permit,
                    Err(message) => {
                        log::info!(
                            "event_subscriptions: consent for '{subscriber_id}' refused: {message}"
                        );
                        let _ = reply.send(HostSubscribeReply::Err { message });
                        return;
                    }
                };
                let subscription_id = record_subscription(
                    &self.timeline,
                    &plugin_id,
                    subscriber_type,
                    &subscriber_id,
                    subscriber_context_id,
                    event_names,
                    payload_mode,
                    trigger_mode,
                    resource_id,
                    GrantDuration::Session,
                    permit,
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
                         {subscriber_type:?} '{subscriber_id}' -> '{plugin_id}'"
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
                         '{subscriber_id}' -> '{plugin_id}'"
                    );
                    let _ = reply.send(HostPublishReply::Err {
                        message: "publish denied by user".to_string(),
                    });
                    return;
                }
                let outcome = Self::perform_publish(
                    &self.timeline,
                    subscriber_context_id,
                    &plugin_id,
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
                         disconnected; the {scope} publish to '{plugin_id}' was still applied \
                         ({summary})"
                    );
                } else {
                    log::info!(
                        "event_subscriptions: user allowed ({scope}) publish for \
                         {subscriber_type:?} '{subscriber_id}' -> '{plugin_id}' ({summary})"
                    );
                }
            }
        }
    }

    /// Persist an `Always` `Allow` grant for every operation-qualified target
    /// the consent touched (built by the caller via [`subscription_targets`] or
    /// [`publish_targets`]) in the profile's one grant store, and return the
    /// new rows' ids. Shared by the subscribe and publish consent paths.
    fn persist_always_grants(
        &mut self,
        grant: AlwaysStreamGrant<'_>,
        evidence: crate::broker::consent::CoveredDecision<'_>,
    ) -> Result<Vec<String>, String> {
        let AlwaysStreamGrant {
            actor_type,
            actor_id,
            plugin_id,
            targets,
            workspace_root,
        } = grant;
        let ids: Vec<String> = {
            let mut store = self.monitor.store();
            targets
                .iter()
                .map(|target| {
                    let record = GrantRecord::event_stream_allow(
                        actor_type,
                        actor_id,
                        ActorScope::User,
                        target,
                        workspace_root,
                        GrantDuration::Always,
                        GrantSource::User,
                        None,
                    );
                    let id = record.grant_id.clone();
                    store.record_decided(record, evidence);
                    id
                })
                .collect()
        };
        if let Err(error) = self.monitor.save_grants() {
            self.monitor
                .store()
                .records_mut()
                .retain(|record| !ids.contains(&record.grant_id));
            let message = crate::broker::gate::grant_not_saved(&error);
            log::error!("event_subscriptions: {message}");
            return Err(message);
        }
        log::info!(
            "event_subscriptions: user granted ALWAYS to {actor_type:?} '{actor_id}' -> '{plugin_id}' grants={ids:?}"
        );
        Ok(ids)
    }

    /// Resolve host-stamped identity, (for emit) reject an undeclared stream,
    /// broker-gate on `TargetType::PluginEventStream`, and — on `Allow` — apply the
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
        // agent/pane, never the plugin-id namespace it publishes under.
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
            if !self.stream_is_declared(context_id, &req.plugin_id, &ev.event) {
                let _ = req.reply.send(HostPublishReply::Err {
                    message: format!(
                        "plugin '{}' has not declared stream '{}' in this context — declare it first",
                        req.plugin_id, ev.event
                    ),
                });
                return None;
            }
        }
        let Some(publisher) = Principal::from_actor(actor_type, &actor_id, req.from_pane_id) else {
            log::info!(
                "event_subscriptions: deny publish by {actor_type:?} '{actor_id}': not a requesting principal"
            );
            let _ = req.reply.send(HostPublishReply::Err {
                message: format!("'{actor_id}' cannot publish events"),
            });
            return None;
        };
        let decision = evaluate_publish(
            &self.monitor.store(),
            self.posture.as_ref(),
            req.workspace_root_override.as_deref(),
            &req.plugin_id,
            &publisher,
            ActorScope::User,
            None,
            &event_names,
        );
        let pane_id = req.from_pane_id.unwrap_or(0);
        match decision {
            Decision::Allow => {
                let outcome = Self::perform_publish(
                    &self.timeline,
                    context_id,
                    &req.plugin_id,
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
                    req.plugin_id
                );
                Some(PendingEventConsent {
                    request_id: crate::broker::consent::next_consent_request_id(),
                    subscriber_type: actor_type,
                    // Publish is one-shot with no long-poll, so routing and
                    // authorization identity are the same emitter id.
                    broker_actor_id: actor_id.clone(),
                    subscriber_id: actor_id,
                    plugin_id: req.plugin_id,
                    event_names,
                    admitted: None,
                    subscriber_context_id: context_id,
                    workspace_root: req.workspace_root_override.clone().unwrap_or_default(),
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
        timeline: &Arc<Mutex<PluginTimeline>>,
        context_id: u64,
        plugin_id: &str,
        pane_id: u64,
        action: PublishAction,
    ) -> HostPublishReply {
        let mut timeline = timeline.lock().unwrap();
        match action {
            PublishAction::Declare(decls) => {
                match timeline.declare_streams(context_id, plugin_id, decls) {
                    Ok(names) => HostPublishReply::Ok {
                        detail: format!("declared stream(s): {}", names.join(", ")),
                        event_id: None,
                    },
                    Err(message) => HostPublishReply::Err { message },
                }
            }
            PublishAction::Emit(ev) => {
                match timeline.record_event(context_id, plugin_id, pane_id, *ev) {
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

    /// Whether `plugin_id` has declared `stream_name`. Used to reject subscribe
    /// requests for undeclared streams before they reach the broker.
    pub fn stream_is_declared(&self, context_id: u64, plugin_id: &str, stream_name: &str) -> bool {
        self.timeline
            .lock()
            .unwrap()
            .has_stream(context_id, plugin_id, stream_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::{ActorScope, Decision, GrantRecord, GrantSource};
    use crate::host::plugin_timeline::EmittedEvent;
    use crate::protocol::{EventStreamDecl, PluginEventActor};

    /// Fixed owning/viewer context for tests that don't exercise
    /// cross-context behavior (that's `plugin_timeline.rs`'s job) — keeps every
    /// existing same-context assertion unchanged.
    const CTX: u64 = 1;

    fn allow_grant(actor_id: &str, target_id: &str) -> GrantRecord {
        GrantRecord::event_stream_allow(
            ActorType::Agent,
            actor_id,
            ActorScope::User,
            target_id,
            Path::new("/tmp/ws"),
            GrantDuration::Always,
            GrantSource::User,
            None,
        )
    }

    fn timeline_with_stream() -> Arc<Mutex<PluginTimeline>> {
        let mut t = PluginTimeline::default();
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

    fn granted_service(timeline: Arc<Mutex<PluginTimeline>>) -> HostSubscriptionService {
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
    fn view_subscription_uses_view_actor_for_broker_and_pane_for_delivery() {
        let timeline = timeline_with_stream();
        let mut store = GrantStore::default();
        let mut grant = allow_grant("notes-app", "event-probe::probe.tick");
        grant.actor_type = ActorType::Plugin;
        grant.workspace_root = Some(PathBuf::from("/workspace/notes"));
        store.record(grant);
        let service = HostSubscriptionService::new_for_test(store, Arc::clone(&timeline));
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        let request = HostSubscribeRequest {
            publisher_plugin_id: "event-probe".to_string(),
            event_names: vec!["probe.tick".to_string()],
            payload_mode: PayloadMode::Full,
            trigger_mode: TriggerMode::Conversation,
            resource_id: None,
            from_pane_id: Some(41),
            subscriber_override: Some(plugin_subscriber_id(41)),
            subscriber_type_override: Some(ActorType::Plugin),
            broker_actor_override: Some("notes-app".to_string()),
            workspace_root_override: Some(PathBuf::from("/workspace/notes")),
            subscriber_authority: SubscriberAuthority::External,
            context_id_override: Some(CTX),
            socket_caller: None,
            cancelled: None,
            reply,
        };

        assert!(service.classify_subscribe_request(request).is_none());
        let HostSubscribeReply::Ok {
            subscriber_type,
            subscriber_id,
            ..
        } = receiver.recv().unwrap()
        else {
            panic!("plugin subscription should be allowed");
        };
        assert_eq!(subscriber_type, ActorType::Plugin);
        assert_eq!(subscriber_id, "app-pane:41");
        let timeline = timeline.lock().unwrap();
        let subscription = &timeline.subscriptions()[0];
        assert_eq!(subscription.subscriber_type, ActorType::Plugin);
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
            publisher_plugin_id: "event-probe".to_string(),
            event_names,
            payload_mode,
            trigger_mode: TriggerMode::Conversation,
            resource_id: None,
            from_pane_id: Some(7),
            subscriber_override: override_id.map(String::from),
            subscriber_type_override: None,
            broker_actor_override: None,
            workspace_root_override: Some(PathBuf::from("/tmp/ws")),
            subscriber_authority: SubscriberAuthority::External,
            context_id_override: Some(CTX),
            socket_caller: None,
            cancelled: None,
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
            publisher_plugin_id: "event-probe".to_string(),
            event_names: vec!["probe.tick".to_string()],
            payload_mode: PayloadMode::Full,
            trigger_mode: TriggerMode::Conversation,
            resource_id: None,
            from_pane_id: None,
            subscriber_override: Some(delivery_id.to_string()),
            subscriber_type_override: None,
            broker_actor_override: Some(broker_actor.to_string()),
            workspace_root_override: Some(PathBuf::from("/tmp/ws")),
            subscriber_authority: SubscriberAuthority::External,
            context_id_override: Some(CTX),
            socket_caller: None,
            cancelled: None,
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
            actor: PluginEventActor::User,
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

    /// `classify_subscribe_request` rejects a stream the plugin never declared,
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

        svc.resolve_consent(consent, ConsentChoice::AllowOnce);
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
        svc.resolve_consent(consent, ConsentChoice::Deny);
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
        svc.resolve_consent(consent, ConsentChoice::AllowAlways);
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

    /// The rows `actor` holds in the service's live store.
    fn grant_ids_of(svc: &HostSubscriptionService, actor: &str) -> Vec<String> {
        svc.monitor()
            .store()
            .records()
            .iter()
            .filter(|record| record.actor_id == actor)
            .map(|record| record.grant_id.clone())
            .collect()
    }

    /// Stint 1716, one grant store: an Always grant the person gives through
    /// a subscription consent lands in the permission monitor's store, and
    /// revoking it there stops the open subscription's deliveries at once,
    /// with the pane and its scope revision unchanged.
    #[test]
    fn an_always_subscription_grant_revoked_through_the_monitor_stops_delivery() {
        let timeline = timeline_with_stream();
        let mut svc =
            HostSubscriptionService::new_for_test(GrantStore::default(), Arc::clone(&timeline));
        let admitted = pane_scope(7, &[]);
        let (mut req, rx) = subscribe_request(vec!["probe.tick".into()], PayloadMode::Full, None);
        req.subscriber_authority = SubscriberAuthority::Pane(admitted.clone());
        let consent = svc
            .classify_subscribe_request(req)
            .expect("no grant yet: consent is asked");
        svc.resolve_consent(consent, ConsentChoice::AllowAlways);
        assert!(matches!(rx.recv().unwrap(), HostSubscribeReply::Ok { .. }));
        let granted = grant_ids_of(&svc, "pane:7");
        assert_eq!(granted.len(), 1, "the Always row is in the monitor's store");

        emit(&timeline, "probe.tick");
        assert_eq!(delivered(&timeline), vec!["probe.tick"], "positive control");
        emit(&timeline, "probe.tick");
        let revision = admitted.authority.scope().revision;
        assert!(svc.monitor().revoke_grant_id(&granted[0]).unwrap());
        assert!(!admitted.authority.is_revoked());
        assert_eq!(admitted.authority.scope().revision, revision);
        assert_eq!(
            delivered(&timeline),
            Vec::<String>::new(),
            "queued item withheld"
        );
        emit(&timeline, "probe.tick");
        assert_eq!(
            delivered(&timeline),
            Vec::<String>::new(),
            "nothing routed after"
        );

        // The next subscribe asks again.
        let (mut req, _rx) = subscribe_request(vec!["probe.tick".into()], PayloadMode::Full, None);
        req.subscriber_authority = SubscriberAuthority::Pane(admitted);
        assert!(svc.classify_subscribe_request(req).is_some());
    }

    /// Stint 1497 with one grant store: a seal-key rotation resets grants.
    /// The service reads the monitor's store, so after the reset the open
    /// subscription delivers nothing and the next subscribe asks again,
    /// both when the store is reloaded in place (the Keychain-prompt
    /// recovery) and when the host starts again on the reset profile.
    #[test]
    fn a_seal_reset_drops_subscription_grants_and_asks_again() {
        let dir = tempfile::tempdir().unwrap();
        crate::broker::gate::PermissionMonitor::reload_for_test(dir.path());
        let timeline = timeline_with_stream();
        let mut svc = HostSubscriptionService::new(dir.path(), Arc::clone(&timeline));
        let admitted = pane_scope(7, &[]);
        let (mut req, rx) = subscribe_request(vec!["probe.tick".into()], PayloadMode::Full, None);
        req.subscriber_authority = SubscriberAuthority::Pane(admitted.clone());
        let consent = svc.classify_subscribe_request(req).expect("asks first");
        svc.resolve_consent(consent, ConsentChoice::AllowAlways);
        assert!(matches!(rx.recv().unwrap(), HostSubscribeReply::Ok { .. }));
        emit(&timeline, "probe.tick");

        crate::broker::seal::reset_unverified_profile(
            &crate::broker::custody::HostProcess::for_test(),
            dir.path(),
        )
        .unwrap();
        // As `reload_after_keychain_prompt` does: the same monitor's store is
        // replaced by the reset file.
        *svc.monitor().store() = GrantStore::load_or_default(dir.path()).unwrap();
        assert!(grant_ids_of(&svc, "pane:7").is_empty(), "grants were reset");
        assert_eq!(delivered(&timeline), Vec::<String>::new(), "dropped");
        let (mut req, _rx) = subscribe_request(vec!["probe.tick".into()], PayloadMode::Full, None);
        req.subscriber_authority = SubscriberAuthority::Pane(admitted.clone());
        assert!(svc.classify_subscribe_request(req).is_some(), "asks again");

        // A host start on the reset profile opens the same one store.
        crate::broker::gate::PermissionMonitor::reload_for_test(dir.path());
        let svc2 = HostSubscriptionService::new(dir.path(), Arc::clone(&timeline));
        let (mut req, _rx) = subscribe_request(vec!["probe.tick".into()], PayloadMode::Full, None);
        req.subscriber_authority = SubscriberAuthority::Pane(admitted);
        assert!(
            svc2.classify_subscribe_request(req).is_some(),
            "asks after restart"
        );
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
        plugin_id: &str,
        action: PublishAction,
        override_id: Option<&str>,
    ) -> (
        HostPublishRequest,
        std::sync::mpsc::Receiver<HostPublishReply>,
    ) {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let req = HostPublishRequest {
            plugin_id: plugin_id.to_string(),
            action,
            from_pane_id: Some(7),
            subscriber_override: override_id.map(String::from),
            workspace_root_override: Some(PathBuf::from("/tmp/ws")),
            context_id_override: Some(CTX),
            socket_caller: None,
            reply: tx,
        };
        (req, rx)
    }

    /// A granted emit records the event and host-stamps the emitter identity
    /// (`pane:7`) — never letting `actor_id` default to the plugin-id namespace.
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

    /// Emitting on a stream the plugin never declared is refused before the broker,
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
        let timeline = Arc::new(Mutex::new(PluginTimeline::default()));
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

        svc.resolve_consent(consent, ConsentChoice::AllowOnce);
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
        svc.resolve_consent(consent, ConsentChoice::Deny);
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
        svc.resolve_consent(consent, ConsentChoice::AllowAlways);
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
        svc.resolve_consent(consent, ConsentChoice::AllowAlways);
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
        svc.resolve_consent(consent, ConsentChoice::AllowAlways);
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

    /// A pane incarnation under `deny` (exact stream targets).
    fn pane_scope(pane_id: u64, deny: &[&str]) -> crate::host::plugin_timeline::AdmittedScope {
        let live = crate::broker::pane_authority::LivePaneAuthority::admitted(
            crate::broker::pane_authority::PaneAuthority::for_test(
                pane_id,
                crate::broker::pane_authority::Ceiling::Posture {
                    posture: PermissionPosture {
                        default_posture: Decision::Ask,
                        allow: Vec::new(),
                        ask: Vec::new(),
                        deny: deny.iter().map(|d| d.to_string()).collect(),
                    },
                },
                CTX,
                PathBuf::from("/tmp/ws"),
            ),
        );
        live.mark_live();
        crate::host::plugin_timeline::AdmittedScope::of(&live)
    }

    fn declare(timeline: &Arc<Mutex<PluginTimeline>>, name: &str) {
        timeline
            .lock()
            .unwrap()
            .declare_streams(
                CTX,
                "event-probe",
                vec![EventStreamDecl {
                    name: name.to_string(),
                    schema: serde_json::json!({"type": "object"}),
                    description: None,
                }],
            )
            .unwrap();
    }

    fn emit(timeline: &Arc<Mutex<PluginTimeline>>, name: &str) {
        let mut event = probe_event(1);
        event.event = name.to_string();
        timeline
            .lock()
            .unwrap()
            .record_event(CTX, "event-probe", 7, event)
            .expect("emit records");
    }

    fn delivered(timeline: &Arc<Mutex<PluginTimeline>>) -> Vec<String> {
        timeline
            .lock()
            .unwrap()
            .take_deliveries_for(ActorType::Agent, "pane:7")
            .into_iter()
            .map(|d| d.event)
            .collect()
    }

    /// Stint 1716: an event queued while the subscription was admitted is not
    /// handed over once the pane's scope moves, the pane closes, or the grant
    /// that allowed it is revoked. The drain itself checks the delivery
    /// permit, not only routing.
    #[test]
    fn queued_deliveries_are_withheld_after_scope_change_revoke_or_grant_revocation() {
        for change in ["none", "scope", "close", "grant"] {
            let timeline = timeline_with_stream();
            let mut store = GrantStore::default();
            let grant = allow_grant("pane:7", "event-probe::probe.tick");
            let grant_id = grant.grant_id.clone();
            store.record(grant);
            let service = HostSubscriptionService::new_for_test(store, Arc::clone(&timeline));
            let admitted = pane_scope(7, &[]);
            let (mut req, rx) =
                subscribe_request(vec!["probe.tick".into()], PayloadMode::Full, None);
            req.subscriber_authority = SubscriberAuthority::Pane(admitted.clone());
            assert!(service.classify_subscribe_request(req).is_none());
            assert!(matches!(rx.recv().unwrap(), HostSubscribeReply::Ok { .. }));
            emit(&timeline, "probe.tick");
            assert_eq!(
                timeline.lock().unwrap().pending_delivery_count(),
                1,
                "queued"
            );
            let revision = admitted.authority.scope().revision;
            match change {
                "scope" => {
                    admitted
                        .authority
                        .publish_scope(PathBuf::from("/tmp/elsewhere"));
                }
                "close" => admitted.authority.revoke(),
                "grant" => assert!(service.monitor().revoke_grant_id(&grant_id).unwrap()),
                _ => {}
            }
            if change == "none" {
                assert_eq!(delivered(&timeline), vec!["probe.tick"], "positive control");
                continue;
            }
            if change == "grant" {
                assert!(!admitted.authority.is_revoked());
                assert_eq!(admitted.authority.scope().revision, revision);
            }
            assert_eq!(
                delivered(&timeline),
                Vec::<String>::new(),
                "old-scope data is not delivered ({change})"
            );
        }
    }

    /// Round 3 item 4, round 4 aggregate: "all streams" from a pane is
    /// authorized member by member under its grant, with exact allow grants
    /// on file for every member. One denied member refuses the whole request
    /// and nothing is delivered. Allowed members are delivered; a member
    /// declared later is not; and nothing is delivered once the pane's scope
    /// revision moves. Consent given after a scope change releases nothing.
    #[test]
    fn pane_all_streams_subscription_is_member_exact_and_scope_bound() {
        let timeline = timeline_with_stream();
        declare(&timeline, "probe.secret");
        let mut store = GrantStore::default();
        for stream in ["probe.tick", "probe.secret", "probe.late"] {
            store.record(allow_grant("pane:7", &format!("event-probe::{stream}")));
        }
        store.record(allow_grant("pane:7", "event-probe::*"));
        let service = HostSubscriptionService::new_for_test(store, Arc::clone(&timeline));

        let (mut req, rx) = subscribe_request(Vec::new(), PayloadMode::Full, None);
        req.subscriber_authority =
            SubscriberAuthority::Pane(pane_scope(7, &["event-probe::probe.secret"]));
        assert!(service.classify_subscribe_request(req).is_none());
        assert!(
            matches!(rx.recv().unwrap(), HostSubscribeReply::Err { .. }),
            "a denied member refuses the aggregate"
        );
        assert!(timeline.lock().unwrap().subscriptions().is_empty());
        emit(&timeline, "probe.secret");
        emit(&timeline, "probe.tick");
        assert_eq!(
            delivered(&timeline),
            Vec::<String>::new(),
            "zero deliveries"
        );

        // Positive control: every member allowed.
        let admitted = pane_scope(7, &[]);
        let (mut req, rx) = subscribe_request(Vec::new(), PayloadMode::Full, None);
        req.subscriber_authority = SubscriberAuthority::Pane(admitted.clone());
        assert!(service.classify_subscribe_request(req).is_none());
        assert!(matches!(rx.recv().unwrap(), HostSubscribeReply::Ok { .. }));
        emit(&timeline, "probe.secret");
        emit(&timeline, "probe.tick");
        assert_eq!(delivered(&timeline), vec!["probe.secret", "probe.tick"]);
        declare(&timeline, "probe.late");
        emit(&timeline, "probe.late");
        assert_eq!(
            delivered(&timeline),
            Vec::<String>::new(),
            "a member declared later is not delivered"
        );
        admitted
            .authority
            .publish_scope(PathBuf::from("/tmp/elsewhere"));
        emit(&timeline, "probe.tick");
        assert_eq!(
            delivered(&timeline),
            Vec::<String>::new(),
            "nothing is delivered after the scope revision moves"
        );

        // Delayed consent: admitted in one scope, answered in another.
        let mut service =
            HostSubscriptionService::new_for_test(GrantStore::default(), Arc::clone(&timeline));
        let admitted = pane_scope(7, &[]);
        let (mut req, rx) = subscribe_request(vec!["probe.tick".into()], PayloadMode::Full, None);
        req.subscriber_authority = SubscriberAuthority::Pane(admitted.clone());
        let consent = service
            .classify_subscribe_request(req)
            .expect("no grant: consent is asked");
        let before = timeline.lock().unwrap().subscriptions().len();
        admitted
            .authority
            .publish_scope(PathBuf::from("/tmp/elsewhere"));
        service.resolve_consent(consent, ConsentChoice::AllowOnce);
        assert!(
            matches!(rx.recv().unwrap(), HostSubscribeReply::Err { .. }),
            "consent cannot release an effect in another scope"
        );
        assert_eq!(timeline.lock().unwrap().subscriptions().len(), before);
    }
}
