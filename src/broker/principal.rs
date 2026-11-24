//! Who is asking: the requesting principal every admission carries
//! (`docs/prm/permissions-broker.md`, "The one broker"; stint 1238 scope 7).
//!
//! The host stamps the principal; a guest never supplies it. Its stored form is
//! the `(actor_type, actor_id)` pair every `GrantRecord` and `ExactBinding`
//! already carries. Pane owners include their terminal lifetime. Named agents
//! and plugins retain their stable identity across launches.

use std::fmt;

use super::pane_authority::{LivePaneAuthority, PaneIncarnation};
use super::ActorType;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PanePrincipal {
    number: u64,
    incarnation: PaneIncarnation,
    stored_id: String,
}

impl From<PanePrincipal> for String {
    fn from(owner: PanePrincipal) -> String {
        owner.stored_id
    }
}

impl TryFrom<String> for PanePrincipal {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_stored(&value).ok_or("invalid stored pane principal")
    }
}

impl PanePrincipal {
    fn new(number: u64, incarnation: PaneIncarnation) -> Self {
        let stored_id = format!("pane:{number}#{}", incarnation.as_str());
        Self {
            number,
            incarnation,
            stored_id,
        }
    }

    pub(super) fn from_authority(live: &LivePaneAuthority) -> Self {
        Self::new(live.pane_id(), PaneIncarnation::from_authority(live))
    }

    pub(super) fn from_stored(actor_id: &str) -> Option<Self> {
        let (number, key) = actor_id.strip_prefix("pane:")?.split_once('#')?;
        let owner = Self::new(number.parse().ok()?, PaneIncarnation::from_stored(key)?);
        (owner.stored_id == actor_id).then_some(owner)
    }

    pub(crate) fn number(&self) -> u64 {
        self.number
    }

    pub(crate) fn actor_id(&self) -> &str {
        &self.stored_id
    }

    pub(crate) fn label(&self) -> &str {
        self.stored_id
            .split_once('#')
            .expect("constructed pane id")
            .0
    }

    #[cfg(test)]
    pub(crate) fn for_test(number: u64) -> Self {
        Self::new(
            number,
            PaneIncarnation::from_stored(&uuid::Uuid::new_v4().to_string()).unwrap(),
        )
    }
}

/// Authenticated storage represents a requester by two fields. History whose
/// fields do not identify a requester stays readable without granting access.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(from = "ActorFields", into = "ActorFields")]
pub struct StoredActor(StoredIdentity);

#[derive(Debug, Clone, PartialEq, Eq)]
enum StoredIdentity {
    Requester(Principal),
    History(ActorType, String),
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ActorFields {
    actor_type: ActorType,
    actor_id: String,
}

impl From<ActorFields> for StoredActor {
    fn from(fields: ActorFields) -> Self {
        Principal::from_actor(fields.actor_type, &fields.actor_id, None)
            .map(Self::from)
            .unwrap_or_else(|| Self(StoredIdentity::History(fields.actor_type, fields.actor_id)))
    }
}

impl From<StoredActor> for ActorFields {
    fn from(actor: StoredActor) -> Self {
        Self {
            actor_type: actor.kind(),
            actor_id: actor.id().to_owned(),
        }
    }
}

impl From<Principal> for StoredActor {
    fn from(principal: Principal) -> Self {
        let principal = match principal {
            Principal::Plugin(plugin) => Principal::plugin(plugin.plugin_id, None),
            other => other,
        };
        Self(StoredIdentity::Requester(principal))
    }
}

impl StoredActor {
    pub(super) fn unbound() -> Self {
        Self(StoredIdentity::History(ActorType::System, String::new()))
    }

    pub(crate) fn kind(&self) -> ActorType {
        match &self.0 {
            StoredIdentity::Requester(principal) => principal.actor_type(),
            StoredIdentity::History(kind, _) => *kind,
        }
    }

    pub(crate) fn id(&self) -> &str {
        match &self.0 {
            StoredIdentity::Requester(principal) => principal.actor_id(),
            StoredIdentity::History(_, id) => id,
        }
    }

    pub(crate) fn principal(&self) -> Option<&Principal> {
        match &self.0 {
            StoredIdentity::Requester(principal) => Some(principal),
            StoredIdentity::History(..) => None,
        }
    }

    pub(crate) fn owner(&self) -> RecordOwner {
        self.principal()
            .map(RecordOwner::requester)
            .unwrap_or_else(|| RecordOwner::Legacy(self.id().to_owned()))
    }
}

/// Durable owner of a broker record. Host and legacy labels cannot authorize a requester.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordOwner {
    Pane(PanePrincipal),
    Actor { actor_type: ActorType, id: String },
    Host(String),
    Legacy(String),
}

impl RecordOwner {
    pub fn requester(principal: &Principal) -> Self {
        match principal {
            Principal::Pane(pane) => Self::Pane(pane.clone()),
            _ => Self::Actor {
                actor_type: principal.actor_type(),
                id: principal.actor_id().to_owned(),
            },
        }
    }

    pub(crate) fn stored(kind: ActorType, id: &str) -> Self {
        Principal::from_actor(kind, id, None)
            .as_ref()
            .map(Self::requester)
            .unwrap_or_else(|| Self::Legacy(id.to_owned()))
    }

    pub(super) fn stored_id(&self) -> &str {
        match self {
            Self::Pane(owner) => owner.actor_id(),
            _ => self.label(),
        }
    }

    pub fn pane(&self) -> Option<&PanePrincipal> {
        match self {
            Self::Pane(owner) => Some(owner),
            _ => None,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Pane(owner) => owner.label(),
            Self::Actor { id, .. } | Self::Host(id) | Self::Legacy(id) => id,
        }
    }

    pub fn serialize_label<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.label())
    }
}

impl fmt::Display for RecordOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedAgent(String);

/// Stored actor id of the one local user. There is no multi-user behavior.
pub const LOCAL_USER_ID: &str = "user:local";

/// Stored actor id of every process outside Plexi that the kernel identified
/// (stint 1285). It lives under its own actor type, so no agent name can
/// produce the same key.
pub const EXTERNAL_ID: &str = "outside-process";

/// A plugin and, when the host has placed it, the live instance asking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInstance {
    pub plugin_id: String,
    /// Host-assigned pane id of the asking instance. `None` before placement
    /// (launch materialization) or when the runtime has no stamped instance.
    pub instance_id: Option<u64>,
}

impl PluginInstance {
    pub fn new(plugin_id: impl Into<String>, instance_id: Option<u64>) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            instance_id,
        }
    }

    /// The one rule for pane ids: `0` is the host's "not placed" sentinel and
    /// names no instance.
    pub fn in_pane(plugin_id: impl Into<String>, pane_id: u64) -> Self {
        Self::new(plugin_id, (pane_id != 0).then_some(pane_id))
    }
}

/// The requesting principal. A delegation chain (an agent or plugin invoking a
/// contributed command) extends this type; call sites build it through the
/// constructors below, so adding the chain does not change them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    /// The local user. One fixed value today.
    LocalUser,
    /// A named agent. The reserved pane namespace belongs to terminal lifetimes.
    Agent(NamedAgent),
    Pane(PanePrincipal),
    /// A plugin instance.
    Plugin(PluginInstance),
    /// Any kernel-identified process outside every pane. One shared
    /// principal: what one outside process is granted, every outside
    /// process holds.
    External,
}

impl Principal {
    pub fn agent(agent_id: impl Into<String>) -> Option<Self> {
        let id = agent_id.into();
        (!id.is_empty() && !id.starts_with("pane:")).then_some(Self::Agent(NamedAgent(id)))
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Pane(pane) => pane.label(),
            _ => self.actor_id(),
        }
    }

    pub fn plugin(plugin_id: impl Into<String>, instance_id: Option<u64>) -> Self {
        Self::Plugin(PluginInstance::new(plugin_id, instance_id))
    }

    /// A plugin asking from a pane; see [`PluginInstance::in_pane`].
    pub fn plugin_in_pane(plugin_id: impl Into<String>, pane_id: u64) -> Self {
        Self::Plugin(PluginInstance::in_pane(plugin_id, pane_id))
    }

    /// Decode a host-stamped `(actor_type, actor_id)` pair. System and managed
    /// policy are grant sources, never requesters, so they return `None`, as
    /// does a user actor that is not the local user. A pane id of `0` names no
    /// instance, as in [`PluginInstance::in_pane`].
    pub fn from_actor(
        actor_type: ActorType,
        actor_id: &str,
        instance_id: Option<u64>,
    ) -> Option<Self> {
        match actor_type {
            ActorType::User if actor_id == LOCAL_USER_ID => Some(Self::LocalUser),
            ActorType::Agent => Self::agent(actor_id),
            ActorType::Pane => PanePrincipal::from_stored(actor_id).map(Self::Pane),
            ActorType::Plugin => Some(Self::plugin(
                actor_id,
                instance_id.filter(|pane| *pane != 0),
            )),
            ActorType::External if actor_id == EXTERNAL_ID => Some(Self::External),
            ActorType::User
            | ActorType::External
            | ActorType::System
            | ActorType::ManagedPolicy => None,
        }
    }

    /// Stored actor type, as grants and bindings record it.
    pub fn actor_type(&self) -> ActorType {
        match self {
            Self::LocalUser => ActorType::User,
            Self::Agent(_) => ActorType::Agent,
            Self::Pane(_) => ActorType::Pane,
            Self::Plugin(_) => ActorType::Plugin,
            Self::External => ActorType::External,
        }
    }

    /// Stored actor id, as grants and bindings record it.
    pub fn actor_id(&self) -> &str {
        match self {
            Self::LocalUser => LOCAL_USER_ID,
            Self::Agent(agent) => &agent.0,
            Self::Pane(pane) => pane.actor_id(),
            Self::Plugin(plugin) => &plugin.plugin_id,
            Self::External => EXTERNAL_ID,
        }
    }
}

impl fmt::Display for Principal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LocalUser => f.write_str("user/local"),
            Self::Agent(agent) => write!(f, "agent/{}", agent.0),
            Self::Pane(pane) => f.write_str(pane.label()),
            Self::Plugin(PluginInstance {
                plugin_id,
                instance_id: Some(instance),
            }) => write!(f, "plugin/{plugin_id}#{instance}"),
            Self::Plugin(PluginInstance {
                plugin_id,
                instance_id: None,
            }) => write!(f, "plugin/{plugin_id}"),
            Self::External => write!(f, "external/{EXTERNAL_ID}"),
        }
    }
}

/// One admission decision, as the info trace records it. Carries ids and the
/// outcome only: never arguments, secret values or file contents. Fields can
/// come from a guest or a model, so `emit` escapes every one: a newline cannot
/// forge a second `admission:` line.
pub struct AdmissionTrace<'a> {
    pub principal: &'a Principal,
    /// Tool name, capability id or event operation being admitted.
    pub effect: &'a str,
    /// Target type of the request (`host_tool`, `capability`, ...).
    pub resource_class: &'a str,
    /// `allow`, `ask` or `deny`.
    pub decision: &'a str,
    /// What decided it (`grant_store`, `python_launch_set`, ...).
    pub mechanism: &'a str,
    /// Caller-local call id, empty when the path has none.
    pub call_id: &'a str,
    /// `grant=<id>`, `pending=<id>` or `code=<code>`, empty when none applies.
    pub outcome_ref: &'a str,
}

impl AdmissionTrace<'_> {
    pub fn emit(&self) {
        super::gate::trace_gate(self.line());
    }

    /// The same line at debug: for a caller that already traced this exact
    /// decision at info and is only repeating it (a poll).
    pub fn emit_repeat(&self) {
        log::debug!("{}", self.line());
    }

    /// The escaped `admission:` line, every field printed.
    pub fn line(&self) -> String {
        format!(
            "admission: principal={} effect={} resource_class={} decision={} mechanism={} call_id={} {}",
            escaped(&self.principal.to_string()),
            escaped(self.effect),
            escaped(self.resource_class),
            escaped(self.decision),
            escaped(self.mechanism),
            escaped(self.call_id),
            escaped(self.outcome_ref)
        )
    }
}

fn escaped(field: &str) -> String {
    field.escape_debug().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_form_round_trips_for_every_requester() {
        for principal in [
            Principal::LocalUser,
            Principal::agent("agent:editor").expect("named agent"),
            Principal::Pane(PanePrincipal::for_test(7)),
            Principal::plugin("notes", Some(4)),
        ] {
            let instance = match &principal {
                Principal::Plugin(plugin) => plugin.instance_id,
                _ => None,
            };
            assert_eq!(
                Principal::from_actor(principal.actor_type(), principal.actor_id(), instance),
                Some(principal.clone())
            );
        }
        assert_eq!(Principal::from_actor(ActorType::System, "", None), None);
        assert_eq!(
            Principal::from_actor(ActorType::ManagedPolicy, "org", None),
            None
        );
        assert_eq!(
            Principal::from_actor(ActorType::User, "user:other", None),
            None
        );
    }

    /// The CLI/agent adapter: an agent admission traces the agent principal with
    /// ids and outcome, stores the principal's actor type and id in the pending
    /// binding, and never writes the argument payload to the log.
    #[test]
    fn agent_admission_traces_principal_without_payload() {
        use crate::broker::gate::{gate_log_contains, Admission, AdmitRequest, PermissionMonitor};
        use crate::broker::{ActorScope, TargetType};

        let tag = uuid::Uuid::new_v4().simple().to_string();
        let agent = Principal::agent(format!("agent:trace-{tag}")).unwrap();
        let sentinel = format!("payload-sentinel-{tag}");
        let input = format!(r#"{{"path":"notes.md","body":"{sentinel}"}}"#);
        let ws = tempfile::tempdir().unwrap();
        let monitor = PermissionMonitor::ephemeral();
        let call_id = format!("call-{tag}");
        let Admission::Required { pending_request_id } = monitor.admit(AdmitRequest {
            call_id: &call_id,
            tool: "host.files.edit",
            input_json: &input,
            principal: &agent,
            actor_scope: ActorScope::User,
            trust_origin: "host",
            workspace_root: ws.path(),
            context_id: 1,
            package_id: "",
            instance_id: 0,
            target_type: TargetType::HostTool,
        }) else {
            panic!("an ungranted agent edit asks");
        };
        assert!(gate_log_contains(&format!(
            "admission: principal=agent/agent:trace-{tag} effect=host.files.edit resource_class=host_tool decision=ask mechanism=grant_store call_id={call_id} pending={pending_request_id}"
        )));
        assert!(
            !gate_log_contains(&sentinel),
            "the trace never carries the payload"
        );
        let pending = monitor
            .host_records()
            .pending_id(&pending_request_id)
            .expect("pending row");
        assert_eq!(pending.actor_id, agent.actor_id());
    }

    /// A guest-chosen capability id with a newline cannot forge a trace line.
    #[test]
    fn trace_escapes_control_characters_in_every_field() {
        use crate::broker::gate::gate_log_contains;

        let tag = uuid::Uuid::new_v4().simple().to_string();
        let principal = Principal::plugin(format!("evil\nplugin-{tag}"), Some(3));
        let forged =
            format!("x\nadmission: principal=user/local effect=forged-{tag} decision=allow");
        AdmissionTrace {
            principal: &principal,
            effect: &forged,
            resource_class: "capability",
            decision: "deny",
            mechanism: "python_launch_set",
            call_id: "call\r1",
            outcome_ref: "",
        }
        .emit();
        assert!(gate_log_contains(&format!(
            "admission: principal=plugin/evil\\nplugin-{tag}#3 effect=x\\nadmission: principal=user/local effect=forged-{tag} decision=allow resource_class=capability decision=deny mechanism=python_launch_set call_id=call\\r1"
        )));
        assert!(!gate_log_contains(&format!(
            "\nadmission: principal=user/local effect=forged-{tag}"
        )));
    }

    /// Existing rows store `app`/`agent`/`system`; the local user is a new value
    /// that no existing row carries, so old rows still decode to the same type.
    #[test]
    fn stored_actor_types_keep_their_existing_encoding() {
        let encoded = |actor: ActorType| serde_json::to_string(&actor).unwrap();
        assert_eq!(encoded(ActorType::Plugin), "\"app\"");
        assert_eq!(encoded(ActorType::Agent), "\"agent\"");
        assert_eq!(encoded(ActorType::Pane), "\"pane\"");
        assert_eq!(encoded(ActorType::System), "\"system\"");
        assert_eq!(encoded(ActorType::ManagedPolicy), "\"managed_policy\"");
        assert_eq!(encoded(ActorType::User), "\"user\"");
        for raw in ["\"app\"", "\"agent\"", "\"system\"", "\"managed_policy\""] {
            let decoded: ActorType = serde_json::from_str(raw).unwrap();
            assert_ne!(decoded, ActorType::User);
        }
    }
}
