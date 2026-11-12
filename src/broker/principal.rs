//! Who is asking: the requesting principal every admission carries
//! (`docs/prm/permissions-broker.md`, "The one broker"; stint 1238 scope 7).
//!
//! The host stamps the principal; a guest never supplies it. Its stored form is
//! the `(actor_type, actor_id)` pair every `GrantRecord` and `ExactBinding`
//! already carries, so recording it changes no existing row. Instance identity
//! is the caller's live instance and is traced, not stored: grants stay keyed to
//! the plugin or agent, and the binding's own `instance_id` keeps today's meaning.

use std::fmt;

use super::ActorType;

/// Stored actor id of the one local user. There is no multi-user behavior.
pub const LOCAL_USER_ID: &str = "user:local";

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
}

/// The requesting principal. A delegation chain (an agent or plugin invoking a
/// contributed command) extends this type; call sites build it through the
/// constructors below, so adding the chain does not change them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    /// The local user. One fixed value today.
    LocalUser,
    /// A named agent, by its stored actor id (`agent:<id>`, `pane:<n>`, ...).
    Agent { agent_id: String },
    /// A plugin instance.
    Plugin(PluginInstance),
}

impl Principal {
    pub fn agent(agent_id: impl Into<String>) -> Self {
        Self::Agent {
            agent_id: agent_id.into(),
        }
    }

    pub fn plugin(plugin_id: impl Into<String>, instance_id: Option<u64>) -> Self {
        Self::Plugin(PluginInstance::new(plugin_id, instance_id))
    }

    /// Decode a host-stamped `(actor_type, actor_id)` pair. System and managed
    /// policy are grant sources, never requesters, so they return `None`, as
    /// does a user actor that is not the local user.
    pub fn from_actor(
        actor_type: ActorType,
        actor_id: &str,
        instance_id: Option<u64>,
    ) -> Option<Self> {
        match actor_type {
            ActorType::User if actor_id == LOCAL_USER_ID => Some(Self::LocalUser),
            ActorType::Agent => Some(Self::agent(actor_id)),
            ActorType::Plugin => Some(Self::plugin(actor_id, instance_id)),
            ActorType::User | ActorType::System | ActorType::ManagedPolicy => None,
        }
    }

    /// Stored actor type, as grants and bindings record it.
    pub fn actor_type(&self) -> ActorType {
        match self {
            Self::LocalUser => ActorType::User,
            Self::Agent { .. } => ActorType::Agent,
            Self::Plugin(_) => ActorType::Plugin,
        }
    }

    /// Stored actor id, as grants and bindings record it.
    pub fn actor_id(&self) -> &str {
        match self {
            Self::LocalUser => LOCAL_USER_ID,
            Self::Agent { agent_id } => agent_id,
            Self::Plugin(plugin) => &plugin.plugin_id,
        }
    }
}

impl fmt::Display for Principal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LocalUser => f.write_str("user/local"),
            Self::Agent { agent_id } => write!(f, "agent/{agent_id}"),
            Self::Plugin(PluginInstance {
                plugin_id,
                instance_id: Some(instance),
            }) => write!(f, "plugin/{plugin_id}#{instance}"),
            Self::Plugin(PluginInstance {
                plugin_id,
                instance_id: None,
            }) => write!(f, "plugin/{plugin_id}"),
        }
    }
}

/// One admission decision, as the info trace records it. Carries ids and the
/// outcome only: never arguments, secret values or file contents.
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
        super::gate::trace_gate(format!(
            "admission: principal={} effect={} resource_class={} decision={} mechanism={} call_id={} {}",
            self.principal,
            self.effect,
            self.resource_class,
            self.decision,
            self.mechanism,
            self.call_id,
            self.outcome_ref
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_form_round_trips_for_every_requester() {
        for principal in [
            Principal::LocalUser,
            Principal::agent("agent:editor"),
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
        let agent = Principal::agent(format!("agent:trace-{tag}"));
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
            .show_pending(&pending_request_id)
            .expect("pending row");
        assert_eq!(pending.actor_id, agent.actor_id());
    }

    /// Existing rows store `app`/`agent`/`system`; the local user is a new value
    /// that no existing row carries, so old rows still decode to the same type.
    #[test]
    fn stored_actor_types_keep_their_existing_encoding() {
        let encoded = |actor: ActorType| serde_json::to_string(&actor).unwrap();
        assert_eq!(encoded(ActorType::Plugin), "\"app\"");
        assert_eq!(encoded(ActorType::Agent), "\"agent\"");
        assert_eq!(encoded(ActorType::System), "\"system\"");
        assert_eq!(encoded(ActorType::ManagedPolicy), "\"managed_policy\"");
        assert_eq!(encoded(ActorType::User), "\"user\"");
        for raw in ["\"app\"", "\"agent\"", "\"system\"", "\"managed_policy\""] {
            let decoded: ActorType = serde_json::from_str(raw).unwrap();
            assert_ne!(decoded, ActorType::User);
        }
    }
}
