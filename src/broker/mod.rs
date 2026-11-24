//! Unified permissions broker — `docs/prm/permissions-broker.md`.
//!
//! Generalizes the plugin capability permission model (`plugin_id::workspace::cap`
//! → Green/Yellow/Red) into one host broker that evaluates
//! `actor -> action -> target -> scope -> decision` for plugins, agents, the
//! system, and managed policy.
//!
//! Phase A scope: grant model + persistence + legacy migration + the PGAP plugin
//! capability path. Event streams, undo, model routing enforcement, and the
//! `/permissions` UI surface come later; their target types exist as data only.

pub mod custody;
pub mod gate;
pub(crate) mod caller;
pub(crate) mod consent;
pub(crate) mod host_key;
pub mod integrity;
mod needs_you_store;
pub(crate) mod pane_authority;
pub(crate) mod permit;
pub mod principal;
pub(crate) mod seal;

use crate::app::permissions::{Capability, PermissionState, PermissionStore};
use crate::platform::toml_store::TomlStore;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Log prefix for every line this store emits.
const STORE_LABEL: &str = "grant_store";

// ── Core enums ────────────────────────────────────────────────────────────────

/// Who is asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorType {
    // Grant-store identity is stable across the public vocabulary change.
    #[serde(rename = "app")]
    Plugin,
    Agent,
    System,
    ManagedPolicy,
    /// The local user (`principal::LOCAL_USER_ID`). No existing row carries it.
    User,
    /// Processes outside every pane (`principal::EXTERNAL_ID`, stint 1285).
    /// No row written before 1285 carries it, so every existing row keeps
    /// its meaning; an agent named `external` stays `Agent`.
    External,
}

/// What kind of thing is being accessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetType {
    Capability,
    HostTool,
    // Preserve sealed grant identity; renaming a public type must not mint a grant.
    #[serde(rename = "app_connector")]
    PluginConnector,
    #[serde(rename = "app_event_stream")]
    PluginEventStream,
    McpTool,
    FileScope,
    Secret,
    /// Loading one plugin tree that is not bundled with Plexi (stint 1398).
    /// A profile install binds the plugin id in that profile (not its
    /// content; stint 1279); a plugin folder binds the folder. Never bound to
    /// a pane or context.
    Package,
    ModelPointer,
    UndoCheckpoint,
    /// One typed one-shot CLI operation of a plugin package (stint 1175). The
    /// grant binds package content, workspace, operation definition, mode,
    /// resolved executable and executable content through `package_id` and
    /// `resource_id`, never through pane or context ids.
    CliOperation,
}

/// The broker's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Ask,
    Deny,
}

impl Decision {
    /// The one ordering of decisions by how much they permit: deny < ask <
    /// allow (stint 1285).
    pub fn permissiveness(self) -> u8 {
        match self {
            Self::Deny => 0,
            Self::Ask => 1,
            Self::Allow => 2,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }

    /// Map a legacy Green/Yellow/Red state to a broker decision.
    pub fn from_permission_state(state: PermissionState) -> Self {
        match state {
            PermissionState::Green => Self::Allow,
            PermissionState::Yellow => Self::Ask,
            PermissionState::Red => Self::Deny,
        }
    }
}

/// How long a grant lasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantDuration {
    Once,
    Session,
    Workspace,
    Document,
    Game,
    Path,
    PackageIdentity,
    Always,
}

impl GrantDuration {
    /// The durations a grant held by every process outside Plexi may have:
    /// an allowlist, so a duration added later is refused until classified
    /// (stint 1285). What one outside process is granted, all of them hold,
    /// so it never outlives the session.
    pub fn allowed_for_external(self) -> bool {
        match self {
            Self::Once | Self::Session => true,
            Self::Workspace
            | Self::Document
            | Self::Game
            | Self::Path
            | Self::PackageIdentity
            | Self::Always => false,
        }
    }
}

pub(crate) fn workspace_secret_tool(tool: &str) -> bool {
    matches!(tool, "secret.get" | "secret.set" | "secret.delete")
}

impl TargetType {
    fn is_workspace_secret(self, tool: &str) -> bool {
        self == Self::Secret && workspace_secret_tool(tool)
    }
}

impl GrantRecord {
    fn is_secret_once(&self) -> bool {
        self.duration == GrantDuration::Once
            && self.decision == Decision::Allow
            && self.target_type.is_workspace_secret(&self.target_id)
    }

    /// A row that says the person decided it can only be written with the
    /// evidence of that decision (stint 1285).
    pub fn needs_decision_evidence(&self) -> bool {
        self.source == GrantSource::User
    }

    /// The run a desktop approval bound this row to. A one-shot's operation
    /// id and a pane session row's pane tag are not runs.
    pub fn bound_run(&self) -> Option<&str> {
        self.bound_operation_id
            .as_deref()
            .filter(|bound| !self.consumed && !self.is_secret_once() && !bound.starts_with("pane:"))
    }

    /// Whether this row may exist for its actor at all: an outside-process
    /// row only for once or session (stint 1285).
    pub fn duration_admissible(&self) -> bool {
        if self.actor_type == ActorType::External
            && self.target_type == TargetType::Secret
            && self.decision == Decision::Allow
            && workspace_secret_tool(&self.target_id)
        {
            return self.duration == GrantDuration::Once;
        }
        self.actor_type != ActorType::External || self.duration.allowed_for_external()
    }
}

/// Whether a grant is permanent (`hard`) or time-limited (`soft`).
///
/// Old `grants.toml` entries without a `kind` field deserialize as `Hard` via
/// `#[serde(default)]`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantKind {
    #[default]
    Hard,
    Soft,
}

impl GrantKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hard => "hard",
            Self::Soft => "soft",
        }
    }
}

/// Where a grant came from. Managed grants cannot be overridden by any other
/// source — they are evaluated first at each decision tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantSource {
    Managed,
    User,
    Workspace,
    Session,
    /// Delegated by a caller from grants it holds, never by the person
    /// (stint 1285).
    Delegated,
}

/// Trust origin of the actor itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorScope {
    BuiltIn,
    User,
    Workspace,
    Marketplace,
    Managed,
}

/// What the grant's resource binding is scoped to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceScope {
    Workspace,
    Pane,
    Document,
    Game,
    Path,
    PackageIdentity,
    Account,
    Global,
}

// ── GrantRecord ───────────────────────────────────────────────────────────────

/// One persisted grant — the generalized form of the old
/// `plugin_id::workspace::capability → state` entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrantRecord {
    pub actor_type: ActorType,
    pub actor_id: String,
    pub actor_scope: ActorScope,
    /// Canonical workspace root this grant is bound to; `None` = global.
    pub workspace_root: Option<PathBuf>,
    pub target_type: TargetType,
    pub target_id: String,
    pub resource_scope: ResourceScope,
    /// Optional stable resource id/path (document id, game id, file path…).
    #[serde(default)]
    pub resource_id: Option<String>,
    pub decision: Decision,
    pub duration: GrantDuration,
    pub source: GrantSource,
    /// Hard (permanent until revoked) vs soft (expires into `ask`).
    #[serde(default)]
    pub kind: GrantKind,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds; `None` = no expiry.
    #[serde(default)]
    pub expires_at: Option<i64>,
    /// `0` is a legacy row and never authorizes a tool, host tool, or subscription.
    /// Exact grants are schema `1`.
    #[serde(default)]
    pub binding_schema: u32,
    #[serde(default)]
    pub grant_id: String,
    /// SHA-256 of the canonical argument JSON. Empty never matches.
    #[serde(default)]
    pub args_fingerprint: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub package_id: String,
    #[serde(default)]
    pub instance_id: Option<u64>,
    #[serde(default)]
    pub context_id: Option<u64>,
    /// Host-stamped trust origin (`host`, `user`, `managed`). Compared on exact grants.
    #[serde(default)]
    pub trust_origin: String,
    #[serde(default)]
    pub revocation_epoch: u64,
    /// A consumed one-shot no longer authorizes a new commit.
    #[serde(default)]
    pub consumed: bool,
    /// A live secret Once reserves its host call ID until claimed.
    /// Claiming a Once replaces the binding with the use ID for commit.
    #[serde(default)]
    pub bound_operation_id: Option<String>,
    /// The pane whose refused request a run-bound approval answered: only
    /// that pane may delegate the tool for that run (stint 1285).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_requester_pane: Option<u64>,
    /// Actor + tool + resource, ignoring the argument fingerprint.
    ///
    /// Exact one-shot and session grants stay fingerprint-bound. An explicit
    /// Always deny, and an allow created from the Permissions plugin, set this so
    /// the next call of that tool is answered without a matching payload.
    /// A `tool_scoped` row matches its resource id when one is set, and any
    /// resource when it is not. It does not also require package, instance,
    /// context, or session equality.
    #[serde(default)]
    pub tool_scoped: bool,
    /// Tool scope for an agent run: actor, tool, and resource still have to
    /// match, but argument bytes do not. Exact call grants leave this false.
    /// A scope grant is still one `GrantRecord` in this store, evaluated by
    /// `matches`. The Permissions plugin does not use this flag; it uses
    /// `tool_scoped`.
    #[serde(default)]
    pub args_unbound: bool,
}

impl GrantRecord {
    /// Build the record a plugin capability decision produces — the migrated
    /// shape from the spec's migration table.
    pub fn plugin_capability(
        plugin_id: &str,
        workspace_root: &Path,
        cap: Capability,
        decision: Decision,
    ) -> Self {
        Self {
            actor_type: ActorType::Plugin,
            actor_id: plugin_id.to_string(),
            actor_scope: ActorScope::User,
            workspace_root: Some(crate::platform::path::canonical_or_self(workspace_root)),
            target_type: TargetType::Capability,
            target_id: cap.as_str().to_string(),
            resource_scope: ResourceScope::Workspace,
            resource_id: None,
            decision,
            duration: GrantDuration::Always,
            source: GrantSource::User,
            created_at: crate::platform::clock::now_secs() as i64,
            expires_at: None,
            grant_id: format!("cap:{plugin_id}:{}", cap.as_str()),
            ..Self::unbound()
        }
    }

    /// A capability row addressed by its wire id. `grant_id` is stable so
    /// `plexi permissions list` and the Permissions plugin show the same row.
    pub fn capability_id(
        plugin_id: &str,
        workspace_root: &Path,
        capability_id: &str,
        decision: Decision,
        source: GrantSource,
    ) -> Self {
        Self {
            actor_type: ActorType::Plugin,
            actor_id: plugin_id.to_string(),
            actor_scope: ActorScope::User,
            workspace_root: Some(crate::platform::path::canonical_or_self(workspace_root)),
            target_type: TargetType::Capability,
            target_id: capability_id.to_string(),
            resource_scope: ResourceScope::Workspace,
            resource_id: None,
            decision,
            duration: GrantDuration::Always,
            source,
            created_at: crate::platform::clock::now_secs() as i64,
            expires_at: None,
            grant_id: format!("cap:{plugin_id}:{capability_id}"),
            ..Self::unbound()
        }
    }

    /// Schema-0 placeholder. Spreading this into a struct literal fills the
    /// exact-binding fields with values that do not authorize a tool call.
    pub fn unbound() -> Self {
        Self {
            actor_type: ActorType::System,
            actor_id: String::new(),
            actor_scope: ActorScope::User,
            workspace_root: None,
            target_type: TargetType::Capability,
            target_id: String::new(),
            resource_scope: ResourceScope::Global,
            resource_id: None,
            decision: Decision::Deny,
            duration: GrantDuration::Once,
            source: GrantSource::Session,
            kind: GrantKind::Hard,
            created_at: 0,
            expires_at: None,
            binding_schema: 0,
            grant_id: String::new(),
            args_fingerprint: String::new(),
            session_id: None,
            package_id: String::new(),
            instance_id: None,
            context_id: None,
            trust_origin: String::new(),
            revocation_epoch: 0,
            consumed: false,
            bound_operation_id: None,
            run_requester_pane: None,
            tool_scoped: false,
            args_unbound: false,
        }
    }

    /// An exact schema-1 grant. `workspace_root: None` is not representable.
    pub fn from_binding(
        binding: &ExactBinding,
        decision: Decision,
        duration: GrantDuration,
        source: GrantSource,
        grant_id: &str,
    ) -> Self {
        Self {
            actor_type: binding.actor_type,
            actor_id: binding.actor_id.clone(),
            actor_scope: binding.actor_scope,
            workspace_root: Some(crate::platform::path::canonical_or_self(
                &binding.workspace_root,
            )),
            target_type: binding.target_type,
            target_id: binding.target_id.clone(),
            resource_scope: binding.resource_scope,
            resource_id: binding.resource_id.clone(),
            decision,
            duration,
            source,
            kind: GrantKind::Hard,
            created_at: crate::platform::clock::now_secs() as i64,
            expires_at: None,
            binding_schema: 1,
            grant_id: grant_id.to_string(),
            args_fingerprint: binding.args_fingerprint.clone(),
            session_id: binding.session_id.clone(),
            package_id: binding.package_id.clone(),
            instance_id: binding.instance_id,
            context_id: binding.context_id,
            trust_origin: binding.trust_origin.clone(),
            revocation_epoch: 0,
            consumed: false,
            bound_operation_id: None,
            run_requester_pane: None,
            tool_scoped: false,
            args_unbound: false,
        }
    }

    /// A remembered plugin capability decision: it covers every pane of the
    /// plugin in its workspace, for its resource only (stint 1489).
    pub fn plugin_wide(&self) -> bool {
        self.actor_type == ActorType::Plugin
            && self.target_type == TargetType::Capability
            && self.binding_schema >= 1
            && self.duration == GrantDuration::Always
    }

    /// True when this record answers the given request.
    ///
    /// A missing workspace is never a wildcard. Schema-0 rows answer only
    /// capability containment (WASM), never a tool, host tool, or event
    /// subscription. Exact grants compare actor trust, resource, package,
    /// instance, context, session, and the argument fingerprint.
    pub fn matches(&self, req: &PermissionRequest, now: i64) -> bool {
        if let Some(expires) = self.expires_at {
            if now >= expires {
                return false;
            }
        }
        if self.actor_type != req.actor_type
            || self.actor_id != req.actor_id
            || self.target_type != req.target_type
            || self.target_id != req.target_id
        {
            return false;
        }
        match (&self.workspace_root, &req.workspace_root) {
            (Some(grant_ws), Some(req_ws)) if grant_ws == req_ws => {}
            _ => return false,
        }
        let toolish = matches!(
            self.target_type,
            TargetType::PluginConnector
                | TargetType::HostTool
                | TargetType::McpTool
                | TargetType::PluginEventStream
        );
        if self.binding_schema < 1 || req.binding_schema < 1 {
            // A coarse deny still withholds a named tool from discovery. It does
            // not authorize a call, and it does not answer an exact tool request.
            if self.decision == Decision::Deny
                && self.binding_schema < 1
                && req.binding_schema < 1
                && toolish
            {
                return true;
            }
            // Legacy capability rows stay containment-only. They are not translated.
            // A legacy row answers an exact request only to deny it: it names no
            // resource, so an allow would cover every resource (stint 1489).
            return !toolish
                && self.target_type == TargetType::Capability
                && req.target_type == TargetType::Capability
                && (req.binding_schema < 1 || self.decision == Decision::Deny);
        }
        if self.resource_scope != req.resource_scope || self.resource_id != req.resource_id {
            return false;
        }
        if self.actor_scope != req.actor_scope || self.trust_origin != req.trust_origin {
            return false;
        }
        // A CLI operation grant is bound to package content and the operation
        // binding, not to the pane that asked: a remembered grant must survive a
        // restart that reassigns pane and context ids. A remembered plugin
        // capability decision is plugin-wide in the same way (stint 1489), and
        // so is permission to load plugin code (stint 1398).
        let pane_bound = !matches!(
            self.target_type,
            TargetType::CliOperation | TargetType::Package
        ) && !self.plugin_wide();
        if self.package_id != req.package_id
            || (pane_bound
                && (self.instance_id != req.instance_id || self.context_id != req.context_id))
        {
            return false;
        }
        if self.is_secret_once() {
            let owner = if self.consumed {
                &req.operation_id
            } else {
                &req.call_id
            };
            if owner.is_empty() || self.bound_operation_id.as_ref() != Some(owner) {
                return false;
            }
        }
        if self.tool_scoped {
            return true;
        }
        if !self.args_unbound
            && (self.args_fingerprint.is_empty() || self.args_fingerprint != req.args_fingerprint)
        {
            return false;
        }
        if self.duration == GrantDuration::Session {
            match (&self.session_id, &req.session_id) {
                (Some(grant_session), Some(req_session)) if grant_session == req_session => {}
                _ => return false,
            }
        }
        if self.duration == GrantDuration::Once && self.consumed {
            return self.bound_operation_id.as_deref() == Some(req.operation_id.as_str())
                && !req.operation_id.is_empty();
        }
        true
    }
}

/// Every field an exact grant and the call that uses it must share.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactBinding {
    pub actor_type: ActorType,
    pub actor_id: String,
    pub actor_scope: ActorScope,
    pub trust_origin: String,
    pub workspace_root: PathBuf,
    pub target_type: TargetType,
    pub target_id: String,
    pub resource_scope: ResourceScope,
    pub resource_id: Option<String>,
    pub args_fingerprint: String,
    pub session_id: Option<String>,
    pub package_id: String,
    pub instance_id: Option<u64>,
    pub context_id: Option<u64>,
    pub call_id: String,
    pub operation_id: String,
}

impl ExactBinding {
    /// Shared binding for an event-stream grant and the question that uses it.
    /// Workspace is required. A missing session id does not match a session grant.
    pub fn event_stream(
        actor_type: ActorType,
        actor_id: &str,
        actor_scope: ActorScope,
        target_id: &str,
        workspace: &Path,
        session_id: Option<String>,
    ) -> Self {
        Self {
            actor_type,
            actor_id: actor_id.to_string(),
            actor_scope,
            trust_origin: "host".to_string(),
            workspace_root: workspace.to_path_buf(),
            target_type: TargetType::PluginEventStream,
            target_id: target_id.to_string(),
            resource_scope: ResourceScope::Global,
            resource_id: None,
            args_fingerprint: crate::broker::gate::stream_fingerprint(target_id),
            session_id,
            package_id: String::new(),
            instance_id: None,
            context_id: None,
            call_id: String::new(),
            operation_id: String::new(),
        }
    }
}

impl GrantRecord {
    /// Exact schema-1 allow for one event stream. Old broad rows are not translated.
    #[allow(clippy::too_many_arguments)]
    pub fn event_stream_allow(
        actor_type: ActorType,
        actor_id: &str,
        actor_scope: ActorScope,
        target_id: &str,
        workspace: &Path,
        duration: GrantDuration,
        source: GrantSource,
        session_id: Option<String>,
    ) -> Self {
        let binding = ExactBinding::event_stream(
            actor_type,
            actor_id,
            actor_scope,
            target_id,
            workspace,
            session_id.clone(),
        );
        let mut record = Self::from_binding(
            &binding,
            Decision::Allow,
            duration,
            source,
            &format!("grant-stream-{}", uuid::Uuid::new_v4()),
        );
        record.session_id = session_id;
        record
    }
}

// ── PermissionRequest ─────────────────────────────────────────────────────────

/// A typed permission question: `actor -> action -> target -> scope`.
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionRequest {
    pub actor_type: ActorType,
    pub actor_id: String,
    pub target_type: TargetType,
    pub target_id: String,
    /// Canonicalized workspace the action happens in. `None` matches nothing.
    pub workspace_root: Option<PathBuf>,
    /// `0` for a coarse question that must not match an exact tool grant.
    pub binding_schema: u32,
    pub actor_scope: ActorScope,
    pub trust_origin: String,
    pub resource_scope: ResourceScope,
    pub resource_id: Option<String>,
    pub args_fingerprint: String,
    pub session_id: Option<String>,
    pub package_id: String,
    pub instance_id: Option<u64>,
    pub context_id: Option<u64>,
    pub call_id: String,
    pub operation_id: String,
}

impl PermissionRequest {
    pub fn new(
        actor_type: ActorType,
        actor_id: &str,
        target_type: TargetType,
        target_id: &str,
        workspace_root: Option<&Path>,
    ) -> Self {
        Self {
            actor_type,
            actor_id: actor_id.to_string(),
            target_type,
            target_id: target_id.to_string(),
            workspace_root: workspace_root.map(crate::platform::path::canonical_or_self),
            // Coarse questions do not match exact grants.
            binding_schema: 0,
            actor_scope: ActorScope::User,
            trust_origin: String::new(),
            resource_scope: ResourceScope::Global,
            resource_id: None,
            args_fingerprint: String::new(),
            session_id: None,
            package_id: String::new(),
            instance_id: None,
            context_id: None,
            call_id: String::new(),
            operation_id: String::new(),
        }
    }

    /// Exact tool/subscription question. Every field participates in `matches`.
    pub fn exact(binding: &ExactBinding) -> Self {
        Self {
            actor_type: binding.actor_type,
            actor_id: binding.actor_id.clone(),
            target_type: binding.target_type,
            target_id: binding.target_id.clone(),
            workspace_root: Some(crate::platform::path::canonical_or_self(
                &binding.workspace_root,
            )),
            binding_schema: 1,
            actor_scope: binding.actor_scope,
            trust_origin: binding.trust_origin.clone(),
            resource_scope: binding.resource_scope,
            resource_id: binding.resource_id.clone(),
            args_fingerprint: binding.args_fingerprint.clone(),
            session_id: binding.session_id.clone(),
            package_id: binding.package_id.clone(),
            instance_id: binding.instance_id,
            context_id: binding.context_id,
            call_id: binding.call_id.clone(),
            operation_id: binding.operation_id.clone(),
        }
    }

    /// The PGAP plugin capability check shape.
    #[cfg(test)]
    pub fn plugin_capability(plugin_id: &str, workspace_root: &Path, cap: Capability) -> Self {
        Self::new(
            ActorType::Plugin,
            plugin_id,
            TargetType::Capability,
            cap.as_str(),
            Some(workspace_root),
        )
    }
}

// ── PermissionPosture (settings, not grants) ─────────────────────────────────

/// Actor-requested defaults parsed from a `[permissions]` TOML table.
/// Settings say what the actor wants; only `GrantRecord`s say what the user
/// allowed. Postures participate at the actor-specific tiers of evaluation
/// and can never override a managed or user/workspace/session deny.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionPosture {
    pub default_posture: Decision,
    pub allow: Vec<String>,
    pub ask: Vec<String>,
    pub deny: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PostureToml {
    permissions: PostureTable,
}

#[derive(Debug, Deserialize)]
struct PostureTable {
    default_posture: String,
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    ask: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
}

impl PermissionPosture {
    /// The pointwise meet of two postures: for every target, the stricter of
    /// the two rules as `GrantStore::evaluate_at` applies them. Used to narrow
    /// a worker's requested posture by its parent's immutable posture. The
    /// result is canonical: sorted, disjoint exceptions against the merged
    /// default, so the meet is commutative, associative and idempotent.
    /// An explicit deny survives under a deny default because, unlike a
    /// fallback deny, it also beats an exact allow grant (stint 1632).
    pub fn restricted_by(self, parent: &Self) -> Self {
        let default_posture = if TargetRule::of_default(parent.default_posture)
            > TargetRule::of_default(self.default_posture)
        {
            parent.default_posture
        } else {
            self.default_posture
        };
        let default_rule = TargetRule::of_default(default_posture);
        let mut meet = Self {
            default_posture,
            allow: Vec::new(),
            ask: Vec::new(),
            deny: Vec::new(),
        };
        let targets: std::collections::BTreeSet<&String> = [&self, parent]
            .into_iter()
            .flat_map(|p| p.allow.iter().chain(&p.ask).chain(&p.deny))
            .collect();
        for target in targets {
            let rule = self.rule_for(target).max(parent.rule_for(target));
            if rule == default_rule {
                continue;
            }
            let list = match rule {
                TargetRule::Allow => &mut meet.allow,
                TargetRule::Ask => &mut meet.ask,
                // A fallback deny other than the merged default cannot occur:
                // only a deny default produces one, and it wins the default.
                TargetRule::FallbackDeny | TargetRule::ExplicitDeny => &mut meet.deny,
            };
            list.push(target.clone());
        }
        meet
    }

    /// True when this posture denies `target` before any grant is consulted
    /// or, under a deny default, with no grant to yield to: an explicit deny,
    /// or a deny default the target is not excepted from.
    pub(crate) fn denies(&self, target: &str) -> bool {
        matches!(
            self.rule_for(target),
            TargetRule::FallbackDeny | TargetRule::ExplicitDeny
        )
    }

    /// The rule this posture applies to `target`, decoded with
    /// `evaluate_at`'s precedence: explicit deny, ask, allow, then default.
    fn rule_for(&self, target: &str) -> TargetRule {
        if Self::lists_contain(&self.deny, target) {
            TargetRule::ExplicitDeny
        } else if Self::lists_contain(&self.ask, target) {
            TargetRule::Ask
        } else if Self::lists_contain(&self.allow, target) {
            TargetRule::Allow
        } else {
            TargetRule::of_default(self.default_posture)
        }
    }

    /// Load the user-level posture from `<config_dir>/config.toml`.
    /// Returns `None` when the file or its `[permissions]` table is absent
    /// (the posture is optional). A present-but-invalid table is logged
    /// loudly and treated as absent so a config typo cannot silently widen
    /// permissions (the broker then falls back to Ask).
    pub fn load_from_config(config_dir: &Path) -> Option<Self> {
        let path = config_dir.join("config.toml");
        let raw = std::fs::read_to_string(&path).ok()?;
        let value = toml::from_str::<toml::Value>(&raw).ok()?;
        let perm_table = value.get("permissions").and_then(toml::Value::as_table)?;
        let has_posture_keys = perm_table.contains_key("default_posture")
            || perm_table.contains_key("allow")
            || perm_table.contains_key("ask")
            || perm_table.contains_key("deny");
        if !has_posture_keys {
            return None;
        }
        match Self::from_toml_str(&raw) {
            Ok(posture) => {
                log::info!(
                    "broker: loaded [permissions] posture from {} (default={})",
                    path.display(),
                    posture.default_posture.as_str()
                );
                Some(posture)
            }
            Err(e) => {
                log::error!(
                    "broker: invalid [permissions] table in {} — ignoring posture \
                     (fix the table to re-enable it): {e}",
                    path.display()
                );
                None
            }
        }
    }

    /// Parse from a TOML document containing a `[permissions]` table.
    /// `default_posture` is required — no silent fallback. Accepted values:
    /// `allow`, `ask`, `review` (alias for ask), `deny`.
    pub fn from_toml_str(raw: &str) -> Result<Self, String> {
        let parsed: PostureToml =
            toml::from_str(raw).map_err(|e| format!("invalid [permissions] table: {e}"))?;
        let t = parsed.permissions;
        let default_posture = match t.default_posture.as_str() {
            "allow" => Decision::Allow,
            "ask" | "review" => Decision::Ask,
            "deny" => Decision::Deny,
            other => {
                return Err(format!(
                    "invalid default_posture '{other}' in [permissions] \
                     (expected allow | ask | review | deny)"
                ))
            }
        };
        Ok(Self {
            default_posture,
            allow: t.allow,
            ask: t.ask,
            deny: t.deny,
        })
    }

    fn lists_contain(list: &[String], target_id: &str) -> bool {
        list.iter().any(|t| t == target_id)
    }
}

/// One target's posture rule, loosest first. A fallback deny (the default)
/// yields to an exact allow grant; an explicit deny does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TargetRule {
    Allow,
    Ask,
    FallbackDeny,
    ExplicitDeny,
}

impl TargetRule {
    fn of_default(default_posture: Decision) -> Self {
        match default_posture {
            Decision::Allow => Self::Allow,
            Decision::Ask => Self::Ask,
            Decision::Deny => Self::FallbackDeny,
        }
    }
}

// ── GrantStore ────────────────────────────────────────────────────────────────

/// On-disk shape of `grants.toml`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct GrantStoreData {
    #[serde(default)]
    records: Vec<GrantRecord>,
}

/// Loads, evaluates, and persists generalized grants in
/// `<config_dir>/grants.toml`. A still-present `permissions.toml` is imported
/// once into this file and then retired. Enforcement reads this store only.
///
/// Only authenticated records are imported or loaded.
#[derive(Debug)]
pub struct GrantStore {
    file: TomlStore<GrantStoreData>,
    untrusted: Vec<seal::IntegrityFault>,
    /// Set only by the sealed loader. A store without it is in-memory and
    /// never touches disk (stint 1291).
    custody: Option<custody::HostProcess>,
}

impl Default for GrantStore {
    fn default() -> Self {
        Self {
            file: TomlStore::detached(STORE_LABEL),
            untrusted: Vec::new(),
            custody: None,
        }
    }
}

/// Load the legacy `permissions.toml` store. Host only (stint 1291).
pub fn load_permission_store(
    config_dir: &Path,
) -> Result<PermissionStore, custody::NotHostProcess> {
    let host = custody::host_process()?;
    Ok(PermissionStore::load_sealed(&host, config_dir))
}

impl GrantStore {
    /// Load `grants.toml` from the channel config dir. A missing file is an
    /// empty store. A missing or bad MAC, after the key was read, quarantines
    /// the file and returns an empty store; [`Self::integrity_faults`] names
    /// it, and the legacy file is left in place. An unreadable seal key is
    /// rotated and the profile is resealed empty before this load. When that
    /// rotation still needs a keychain prompt, the files stay put and this
    /// returns an empty store with no integrity fault. A signed file that
    /// does not parse is backed up and
    /// also returns empty. A trusted or absent grants file imports
    /// `permissions.toml` once, seals the result, and retires the legacy file.
    ///
    /// Host only: a process that never ran host start gets [`custody::NotHostProcess`].
    ///
    /// Test-only: the host's one store is the permission monitor's; nothing
    /// in the host loads a private copy of `grants.toml` (stint 1716).
    #[cfg(test)]
    pub fn load_or_default(config_dir: &Path) -> Result<Self, custody::NotHostProcess> {
        let host = custody::host_process()?;
        Ok(Self::load_sealed(&host, config_dir))
    }

    pub(in crate::broker) fn load_sealed(host: &custody::HostProcess, config_dir: &Path) -> Self {
        if let Err(error) = seal::ensure_readable_mac_key(host, config_dir) {
            if host_key::is_key_unreadable(&error) {
                log::info!(
                    "grant_store: mac key unreadable and rotation needs keychain access; left {} in place and loaded no grants",
                    config_dir.join("grants.toml").display()
                );
                return Self {
                    file: TomlStore::at(
                        config_dir.join("grants.toml"),
                        STORE_LABEL,
                        GrantStoreData::default(),
                    ),
                    untrusted: Vec::new(),
                    custody: Some(*host),
                };
            }
            log::error!("grant_store: mac key recovery failed: {error}");
        }
        let path = config_dir.join("grants.toml");
        let existed = path.is_file();
        let loaded = seal::load_toml::<GrantStoreData>(host, &path);
        if loaded.trusted {
            log::info!(
                "grant_store: loaded {} records from {}",
                loaded.data.records.len(),
                path.display()
            );
        }
        let rejected = loaded.fault.is_some() || (existed && !loaded.trusted);
        let mut data = loaded.data;
        fill_capability_grant_ids(&mut data.records);
        drop_once_grants(&mut data.records);
        let mut untrusted = Vec::new();
        if let Some(fault) = loaded.fault {
            seal::note_integrity_fault(config_dir, fault.clone());
            untrusted.push(fault);
        }
        let mut store = Self {
            file: TomlStore::at(path, STORE_LABEL, data),
            untrusted,
            custody: Some(*host),
        };
        if rejected {
            log::info!("grant_store: grants.toml was rejected; permissions.toml was not imported");
            return store;
        }
        store.import_and_retire_legacy(host, config_dir, loaded.trusted);
        store
    }

    fn import_and_retire_legacy(
        &mut self,
        host: &custody::HostProcess,
        config_dir: &Path,
        grants_trusted: bool,
    ) {
        let legacy_path = config_dir.join("permissions.toml");
        if !legacy_path.is_file() {
            return;
        }
        let legacy = PermissionStore::load_sealed(host, config_dir);
        if let Some(fault) = legacy.integrity_fault().cloned() {
            log::info!(
                "grant_store: skipped legacy migration; permissions.toml failed integrity ({})",
                fault.reason
            );
            seal::note_integrity_fault(config_dir, fault.clone());
            self.untrusted.push(fault);
            return;
        }
        let migrated = self.migrate_legacy(&legacy);
        if migrated > 0 {
            log::info!(
                "grant_store: migrated {migrated} legacy permissions.toml entries into {}",
                self.file.path.display()
            );
        }
        if migrated == 0 && !grants_trusted {
            retire_legacy_permissions(config_dir);
            return;
        }
        match self.try_save() {
            Ok(()) => retire_legacy_permissions(config_dir),
            Err(error) => log::error!(
                "grant_store: kept permissions.toml because grants.toml could not be sealed: {error}"
            ),
        }
    }

    /// Files the host refused while loading. Empty when the store is trusted.
    pub fn integrity_faults(&self) -> &[seal::IntegrityFault] {
        &self.untrusted
    }

    /// Import legacy `plugin_id::workspace::capability → state` entries as
    /// generalized records. Loss-free (every parseable entry maps) and
    /// idempotent (entries already represented are skipped). Returns the
    /// number of records added. The legacy store is never mutated.
    pub fn migrate_legacy(&mut self, legacy: &PermissionStore) -> usize {
        let mut added = 0;
        for (plugin_id, workspace, cap, state) in legacy.iter_entries() {
            let record = GrantRecord::plugin_capability(
                plugin_id,
                Path::new(workspace),
                cap,
                Decision::from_permission_state(state),
            );
            let exists = self.file.data.records.iter().any(|r| {
                r.actor_type == record.actor_type
                    && r.actor_id == record.actor_id
                    && r.target_type == record.target_type
                    && r.target_id == record.target_id
                    && r.workspace_root == record.workspace_root
            });
            if !exists {
                self.file.data.records.push(record);
                added += 1;
            }
        }
        for (plugin_id, workspace, capability_id, state) in legacy.iter_wasm_entries() {
            let decision = Decision::from_permission_state(state);
            let record = GrantRecord::capability_id(
                plugin_id,
                Path::new(workspace),
                capability_id,
                decision,
                GrantSource::User,
            );
            let exists = self.file.data.records.iter().any(|existing| {
                existing.actor_type == record.actor_type
                    && existing.actor_id == record.actor_id
                    && existing.target_type == record.target_type
                    && existing.target_id == record.target_id
                    && existing.workspace_root == record.workspace_root
            });
            if !exists {
                self.file.data.records.push(record);
                added += 1;
            }
        }
        added
    }

    /// Append-or-replace a grant. An existing record with the same identity
    /// (actor, target, workspace, source) is replaced so user decisions
    /// update in place instead of accumulating contradictions.
    pub fn record(&mut self, record: GrantRecord) {
        if !cfg!(test) && record.needs_decision_evidence() {
            log::error!(
                "grant_store: refused a user-sourced grant for {} on {} with no desktop evidence",
                record.actor_id,
                record.target_id
            );
            return;
        }
        self.insert_record(record);
    }

    /// Record a row the person decided, on the evidence the surface showing
    /// the request issued (stint 1285).
    pub(crate) fn record_decided(
        &mut self,
        record: GrantRecord,
        _decision: consent::CoveredDecision<'_>,
    ) {
        self.insert_record(record);
    }

    fn insert_record(&mut self, record: GrantRecord) {
        if !record.duration_admissible() {
            log::error!(
                "grant_store: refused {:?} grant for {} on {}: duration is not admissible for this outside-process grant",
                record.duration,
                record.actor_id,
                record.target_id
            );
            return;
        }
        // Each live secret Once has its own owner even before the claim.
        // It coexists with sibling grants and with standing decisions.
        let secret_once = record.is_secret_once();
        self.file.data.records.retain(|r| {
            secret_once
                || r.is_secret_once()
                || (r.duration == GrantDuration::Once && r.consumed)
                || !(r.actor_type == record.actor_type
                && r.actor_id == record.actor_id
                && r.actor_scope == record.actor_scope
                && r.target_type == record.target_type
                && r.target_id == record.target_id
                && r.workspace_root == record.workspace_root
                && r.resource_scope == record.resource_scope
                && r.resource_id == record.resource_id
                && r.args_fingerprint == record.args_fingerprint
                && r.package_id == record.package_id
                && r.instance_id == record.instance_id
                && r.context_id == record.context_id
                && r.source == record.source)
        });
        log::info!(
            "grant_store: recorded {:?} {} -> {:?}:{} = {} ({:?}/{:?})",
            record.actor_type,
            record.actor_id,
            record.target_type,
            record.target_id,
            record.decision.as_str(),
            record.duration,
            record.source,
        );
        self.file.data.records.push(record);
    }

    /// Replace every capability row for this plugin, workspace, and id, whatever
    /// its source. A human click and an auto-grant are one row.
    pub fn upsert_capability(
        &mut self,
        plugin_id: &str,
        workspace_root: &Path,
        capability_id: &str,
        decision: Decision,
        source: GrantSource,
    ) {
        let ws = crate::platform::path::canonical_or_self(workspace_root);
        self.file.data.records.retain(|record| {
            !(record.actor_type == ActorType::Plugin
                && record.actor_id == plugin_id
                && record.target_type == TargetType::Capability
                && record.target_id == capability_id
                && record.workspace_root.as_deref() == Some(ws.as_path()))
        });
        let record =
            GrantRecord::capability_id(plugin_id, workspace_root, capability_id, decision, source);
        log::info!(
            "grant_store: upserted {} {} -> {} = {} ({:?})",
            record.actor_id,
            record.grant_id,
            record.target_id,
            record.decision.as_str(),
            record.source,
        );
        self.file.data.records.push(record);
    }

    /// Remove every grant for `(actor_type, actor_id, target_id)` regardless
    /// of workspace, source, or duration. Returns the number of records
    /// removed. The caller decides whether to `save()`.
    pub fn revoke(&mut self, actor_type: ActorType, actor_id: &str, target_id: &str) -> usize {
        let before = self.file.data.records.len();
        self.file.data.records.retain(|r| {
            !(r.actor_type == actor_type && r.actor_id == actor_id && r.target_id == target_id)
        });
        let removed = before - self.file.data.records.len();
        log::info!(
            "grant_store: revoked {removed} record(s) for {actor_type:?} '{actor_id}' -> '{target_id}'"
        );
        removed
    }

    /// All persisted records (read-only).
    pub fn records(&self) -> &[GrantRecord] {
        &self.file.data.records
    }

    /// Mutable records. Callers that revoke or consume a one-shot hold the
    /// monitor admission lock first.
    pub fn records_mut(&mut self) -> &mut Vec<GrantRecord> {
        &mut self.file.data.records
    }

    /// Atomically write a sealed file. No-op for path-less test stores.
    ///
    /// Host only: the custody token is the one this store was loaded with.
    pub fn try_save(&self) -> Result<(), String> {
        let label = self.file.label();
        let Some(host) = self.custody.as_ref() else {
            if self.file.path.as_os_str().is_empty() {
                return Ok(());
            }
            return Err(custody::NotHostProcess.to_string());
        };
        seal::write_toml(host, &self.file.path, label, &self.file.data)?;
        crate::broker::integrity::note_saved_file(&self.file.path);
        Ok(())
    }

    /// Atomically write a sealed file. No-op for path-less test stores.
    pub fn save(&self) {
        let label = self.file.label();
        if let Err(error) = self.try_save() {
            log::error!(
                "{label}: failed to save {}: {error}",
                self.file.path.display()
            );
        }
    }

    /// Evaluate a request through the spec's ordered tiers:
    ///
    /// 1. typed request (the argument)
    /// 2. managed deny
    /// 3. user/workspace/session deny
    /// 4. actor-specific (posture) deny
    /// 5. managed ask
    /// 6. user/workspace/session ask
    /// 7. actor-specific (posture) ask
    /// 8. persisted allow grants
    /// 9. actor-specific (posture) allow
    /// 10. user/workspace allow (covered by 8 — grants are the allow store)
    /// 11. default posture fallback (`Ask` when no posture is supplied)
    ///
    /// Deny wins over ask; ask wins over allow; managed policy cannot be
    /// overridden by user settings or postures.
    pub fn evaluate(
        &self,
        req: &PermissionRequest,
        posture: Option<&PermissionPosture>,
    ) -> Decision {
        let now = crate::platform::clock::now_secs() as i64;
        self.evaluate_at(req, posture, now)
    }

    pub fn evaluate_at(
        &self,
        req: &PermissionRequest,
        posture: Option<&PermissionPosture>,
        now: i64,
    ) -> Decision {
        let decision = self.decide(req, posture, now, |_| true);
        log::info!(
            "broker: {:?} '{}' -> {:?}:'{}' (ws={:?}) = {}",
            req.actor_type,
            req.actor_id,
            req.target_type,
            req.target_id,
            req.workspace_root,
            decision.as_str()
        );
        decision
    }

    /// [`Self::evaluate_at`] at a permit's commit (stint 1716): the same
    /// rules, but an unclaimed once grant does not count. Standing rows or
    /// this use's own claim must allow it; a fresh once is never spent here.
    pub(crate) fn decide_at_commit(
        &self,
        req: &PermissionRequest,
        posture: Option<&PermissionPosture>,
        now: i64,
    ) -> Decision {
        self.decide(req, posture, now, |r| {
            r.duration != GrantDuration::Once || r.consumed
        })
    }

    /// The first unclaimed once allow that answers `req`, claimed for
    /// `use_id`: spent, and bound so only that use matches it again.
    pub(crate) fn claim_once(&mut self, req: &PermissionRequest, now: i64, use_id: &str) -> Option<String> {
        let row = self.file.data.records.iter_mut().find(|r| {
            r.duration == GrantDuration::Once
                && !r.consumed
                && r.decision == Decision::Allow
                && r.duration_admissible()
                && r.matches(req, now)
        })?;
        row.consumed = true;
        row.bound_operation_id = Some(use_id.to_string());
        log::info!("grant_store: claimed once-grant {} for {use_id}", row.grant_id);
        Some(row.grant_id.clone())
    }

    fn decide(
        &self,
        req: &PermissionRequest,
        posture: Option<&PermissionPosture>,
        now: i64,
        counts: impl Fn(&GrantRecord) -> bool,
    ) -> Decision {
        let matching: Vec<&GrantRecord> = self
            .file
            .data
            .records
            .iter()
            .filter(|r| counts(r) && r.matches(req, now))
            .filter(|r| {
                let admissible = r.duration_admissible();
                if !admissible {
                    log::info!(
                        "broker: ignored {:?} row {} for outside process (inadmissible duration)",
                        r.duration,
                        r.grant_id
                    );
                }
                admissible
            })
            .collect();

        let has = |decision: Decision, managed: bool| {
            matching
                .iter()
                .any(|r| r.decision == decision && (r.source == GrantSource::Managed) == managed)
        };

        if has(Decision::Deny, true) // 2. managed deny
            || has(Decision::Deny, false) // 3. user/workspace/session deny
            || posture.is_some_and(|p| PermissionPosture::lists_contain(&p.deny, &req.target_id))
        // 4. actor deny
        {
            Decision::Deny
        } else if has(Decision::Ask, true) // 5. managed ask
            || has(Decision::Ask, false)
        // 6. user/workspace/session ask
        {
            Decision::Ask
        } else if has(Decision::Allow, true) || has(Decision::Allow, false) {
            // Persisted/session exact allows authorize, including over actor `ask` posture.
            Decision::Allow
        } else if posture.is_some_and(|p| PermissionPosture::lists_contain(&p.ask, &req.target_id))
        // 7. actor ask
        {
            Decision::Ask
        } else if posture.is_some_and(|p| p.default_posture == Decision::Deny)
            && !posture.is_some_and(|p| PermissionPosture::lists_contain(&p.allow, &req.target_id))
            && !posture.is_some_and(|p| PermissionPosture::lists_contain(&p.ask, &req.target_id))
        {
            Decision::Deny
        } else {
            // Posture allow and an Allow default are not authorization.
            Decision::Ask
        }
    }

    /// Capability sets for a plugin launch, equivalent to the legacy
    /// `build_permission_sets` but read from generalized records:
    /// `(allowed, denied)` capabilities for this plugin+workspace.
    #[cfg(test)]
    pub fn plugin_capability_sets(
        &self,
        plugin_id: &str,
        workspace_root: &Path,
    ) -> (HashSet<Capability>, HashSet<Capability>) {
        let ws = crate::platform::path::canonical_or_self(workspace_root);
        let now = crate::platform::clock::now_secs() as i64;
        let mut allowed = HashSet::new();
        let mut denied = HashSet::new();
        for r in &self.file.data.records {
            if r.actor_type != ActorType::Plugin
                || r.actor_id != plugin_id
                || r.target_type != TargetType::Capability
            {
                continue;
            }
            if let Some(expires) = r.expires_at {
                if now >= expires {
                    continue;
                }
            }
            match &r.workspace_root {
                None => {}
                Some(g) if *g == ws => {}
                Some(_) => continue,
            }
            let Ok(cap) = Capability::try_from(r.target_id.as_str()) else {
                log::warn!(
                    "grant_store: skipping record with unknown capability '{}'",
                    r.target_id
                );
                continue;
            };
            match r.decision {
                Decision::Allow => {
                    allowed.insert(cap);
                }
                Decision::Deny => {
                    denied.insert(cap);
                }
                Decision::Ask => {}
            }
        }
        // A deny record always wins over an allow record for the same cap.
        allowed.retain(|c| !denied.contains(c));
        (allowed, denied)
    }
}

fn fill_capability_grant_ids(records: &mut [GrantRecord]) {
    for record in records {
        if record.target_type == TargetType::Capability
            && record.grant_id.is_empty()
            && !record.actor_id.is_empty()
            && !record.target_id.is_empty()
        {
            record.grant_id = format!("cap:{}:{}", record.actor_id, record.target_id);
        }
    }
}

/// A Once grant is session memory (ruling 140 A). An unrelated save may have
/// written one to disk, so none is read back: a one-time consent never
/// authorizes again after a restart (stint 1638).
fn drop_once_grants(records: &mut Vec<GrantRecord>) {
    let before = records.len();
    records.retain(|record| record.duration != GrantDuration::Once);
    let dropped = before - records.len();
    if dropped > 0 {
        log::info!("grant_store: dropped {dropped} once grant(s) from a previous session");
    }
}

fn retire_legacy_permissions(config_dir: &Path) {
    let legacy = config_dir.join("permissions.toml");
    if !legacy.is_file() {
        return;
    }
    let stamp = crate::platform::clock::now_secs();
    let mut dest = config_dir.join(format!("permissions.toml.imported-{stamp}"));
    if dest.exists() {
        let suffix = &uuid::Uuid::new_v4().simple().to_string()[..8];
        dest = config_dir.join(format!("permissions.toml.imported-{stamp}-{suffix}"));
    }
    match std::fs::rename(&legacy, &dest) {
        Ok(()) => log::info!(
            "grant_store: retired {} to {}",
            legacy.display(),
            dest.display()
        ),
        Err(error) => log::error!(
            "grant_store: could not retire {}: {error}",
            legacy.display()
        ),
    }
}

#[cfg(test)]
pub(crate) mod admission_matrix;

#[cfg(test)]
mod cli_operation_tests;

#[cfg(test)]
mod custody_guard_tests;

#[cfg(test)]
mod claim_order_guard_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_binding() -> ExactBinding {
        ExactBinding {
            actor_type: ActorType::Agent,
            actor_id: "chess-opponent".to_string(),
            actor_scope: ActorScope::Workspace,
            trust_origin: "host".to_string(),
            workspace_root: PathBuf::from("/grant-ws"),
            target_type: TargetType::PluginConnector,
            target_id: "chess.make_move".to_string(),
            resource_scope: ResourceScope::Game,
            resource_id: Some("game-1".to_string()),
            args_fingerprint: "fp-move".to_string(),
            session_id: None,
            package_id: "chess".to_string(),
            instance_id: Some(7),
            context_id: Some(3),
            call_id: "call-1".to_string(),
            operation_id: String::new(),
        }
    }

    fn agent_grant(decision: Decision, source: GrantSource) -> GrantRecord {
        GrantRecord::from_binding(
            &agent_binding(),
            decision,
            GrantDuration::Always,
            source,
            "grant-1",
        )
    }

    fn agent_request() -> PermissionRequest {
        PermissionRequest::exact(&agent_binding())
    }

    /// A Python consent request (stint 1489): plugin actor, exact capability.
    fn plugin_capability_binding(resource: &str) -> ExactBinding {
        ExactBinding {
            actor_type: ActorType::Plugin,
            actor_id: "probe".to_string(),
            actor_scope: ActorScope::User,
            trust_origin: "host".to_string(),
            workspace_root: PathBuf::from("/grant-ws"),
            target_type: TargetType::Capability,
            target_id: "net.http".to_string(),
            resource_scope: ResourceScope::Workspace,
            resource_id: Some(resource.to_string()),
            args_fingerprint: "fp-request".to_string(),
            session_id: None,
            package_id: String::new(),
            instance_id: Some(4),
            context_id: Some(9),
            call_id: "pycap_1".to_string(),
            operation_id: String::new(),
        }
    }

    /// Stint 1489: a legacy (schema-0) capability allow names no resource, so
    /// it never answers an exact request; a legacy deny still does.
    #[test]
    fn schema0_allow_never_answers_an_exact_request() {
        let request = PermissionRequest::exact(&plugin_capability_binding("api.example"));
        let legacy = |decision| {
            GrantRecord::capability_id(
                "probe",
                Path::new("/grant-ws"),
                "net.http",
                decision,
                GrantSource::User,
            )
        };
        assert!(!legacy(Decision::Allow).matches(&request, 0));
        assert!(legacy(Decision::Deny).matches(&request, 0));
        let mut store = GrantStore::default();
        store.record(legacy(Decision::Allow));
        assert_eq!(store.evaluate_at(&request, None, 0), Decision::Ask);
        let coarse = PermissionRequest::plugin_capability(
            "probe",
            Path::new("/grant-ws"),
            crate::app::permissions::Capability::NetHttp,
        );
        assert!(
            legacy(Decision::Allow).matches(&coarse, 0),
            "coarse containment checks keep reading legacy rows"
        );
    }

    /// Stint 1489: a remembered plugin capability decision covers every pane
    /// and context of the plugin, but only its own resource (the host list).
    /// A once or session grant stays bound to its pane.
    #[test]
    fn plugin_wide_always_ignores_pane_and_context_but_not_resource() {
        let mut stored = plugin_capability_binding("api.example");
        stored.instance_id = None;
        stored.context_id = None;
        let always = GrantRecord::from_binding(
            &stored,
            Decision::Allow,
            GrantDuration::Always,
            GrantSource::User,
            "grant-always",
        );
        assert!(always.plugin_wide());
        let mut other_pane = plugin_capability_binding("api.example");
        other_pane.instance_id = Some(11);
        other_pane.context_id = Some(12);
        assert!(always.matches(&PermissionRequest::exact(&other_pane), 0));
        assert!(!always.matches(
            &PermissionRequest::exact(&plugin_capability_binding("api.example,evil.example")),
            0
        ));
        assert!(!always.matches(
            &PermissionRequest::exact(&plugin_capability_binding("*")),
            0
        ));

        let once = GrantRecord::from_binding(
            &plugin_capability_binding("api.example"),
            Decision::Allow,
            GrantDuration::Once,
            GrantSource::Session,
            "grant-once",
        );
        assert!(!once.plugin_wide());
        assert!(once.matches(
            &PermissionRequest::exact(&plugin_capability_binding("api.example")),
            0
        ));
        assert!(!once.matches(&PermissionRequest::exact(&other_pane), 0));
    }

    #[test]
    fn tool_scoped_grant_keeps_actor_origin_and_context_bound() {
        let mut grant = agent_grant(Decision::Allow, GrantSource::Session);
        grant.tool_scoped = true;
        let request = agent_request();
        assert!(grant.matches(&request, grant.created_at));

        let mut other_actor = agent_binding();
        other_actor.actor_id = "other-agent".to_string();
        assert!(!grant.matches(&PermissionRequest::exact(&other_actor), grant.created_at));

        let mut other_origin = agent_binding();
        other_origin.trust_origin = "other-host".to_string();
        assert!(!grant.matches(&PermissionRequest::exact(&other_origin), grant.created_at));

        let mut other_scope = agent_binding();
        other_scope.actor_scope = ActorScope::User;
        assert!(!grant.matches(&PermissionRequest::exact(&other_scope), grant.created_at));

        let mut other_context = agent_binding();
        other_context.context_id = Some(4);
        assert!(!grant.matches(&PermissionRequest::exact(&other_context), grant.created_at));
    }

    #[test]
    fn same_name_actor_subcontext_isolation() {
        let mut store = GrantStore::default();
        let binding_ctx1 = agent_binding();
        store.record(GrantRecord::from_binding(
            &binding_ctx1,
            Decision::Allow,
            GrantDuration::Always,
            GrantSource::Workspace,
            "grant-ctx1",
        ));

        let req_ctx1 = PermissionRequest::exact(&binding_ctx1);
        assert_eq!(store.evaluate(&req_ctx1, None), Decision::Allow);

        // Same agent actor name in a different subcontext must NOT match or inherit the grant.
        let mut binding_ctx2 = agent_binding();
        binding_ctx2.context_id = Some(4);
        let req_ctx2 = PermissionRequest::exact(&binding_ctx2);
        assert_eq!(
            store.evaluate(&req_ctx2, None),
            Decision::Ask,
            "same-name actor in different subcontext must not inherit grant"
        );
    }

    #[test]
    fn default_fallback_is_ask_without_posture() {
        let store = GrantStore::default();
        assert_eq!(store.evaluate(&agent_request(), None), Decision::Ask);
    }

    #[test]
    fn deny_beats_ask_beats_allow() {
        let mut store = GrantStore::default();
        store.record(agent_grant(Decision::Allow, GrantSource::User));
        assert_eq!(store.evaluate(&agent_request(), None), Decision::Allow);

        store.record(agent_grant(Decision::Ask, GrantSource::Session));
        assert_eq!(store.evaluate(&agent_request(), None), Decision::Ask);

        store.record(agent_grant(Decision::Deny, GrantSource::Workspace));
        assert_eq!(store.evaluate(&agent_request(), None), Decision::Deny);
    }

    #[test]
    fn managed_deny_beats_user_allow_and_posture_allow() {
        let mut store = GrantStore::default();
        store.record(agent_grant(Decision::Deny, GrantSource::Managed));
        store.record(agent_grant(Decision::Allow, GrantSource::User));
        let posture = PermissionPosture {
            default_posture: Decision::Allow,
            allow: vec!["chess.make_move".to_string()],
            ask: vec![],
            deny: vec![],
        };
        assert_eq!(
            store.evaluate(&agent_request(), Some(&posture)),
            Decision::Deny,
            "managed deny must not be overridable by user grant or posture"
        );
    }

    #[test]
    fn posture_lists_and_default_apply_in_order() {
        let store = GrantStore::default();
        let posture = PermissionPosture {
            default_posture: Decision::Allow,
            allow: vec!["host.panes.read".to_string()],
            ask: vec!["app.chess.make_move".to_string()],
            deny: vec!["host.secrets.read".to_string()],
        };
        let req = |target: &str| {
            PermissionRequest::new(ActorType::Agent, "a1", TargetType::HostTool, target, None)
        };
        assert_eq!(
            store.evaluate(&req("host.secrets.read"), Some(&posture)),
            Decision::Deny
        );
        assert_eq!(
            store.evaluate(&req("app.chess.make_move"), Some(&posture)),
            Decision::Ask
        );
        assert_eq!(
            store.evaluate(&req("host.panes.read"), Some(&posture)),
            Decision::Ask,
            "a posture allow-list is not a grant"
        );
        assert_eq!(
            store.evaluate(&req("anything.else"), Some(&posture)),
            Decision::Ask,
            "an Allow default is not authorization"
        );
    }

    #[test]
    fn child_posture_cannot_relax_parent_restrictions() {
        let parent = PermissionPosture {
            default_posture: Decision::Deny,
            allow: vec![],
            ask: vec!["pane.send".into()],
            deny: vec!["secret.get".into()],
        };
        let child = PermissionPosture {
            default_posture: Decision::Allow,
            allow: vec!["secret.get".into()],
            ask: vec![],
            deny: vec![],
        }
        .restricted_by(&parent);
        assert_eq!(child.default_posture, Decision::Deny);
        assert!(child.deny.contains(&"secret.get".to_string()));
        assert!(child.ask.contains(&"pane.send".to_string()));
    }

    /// No target may be decided more loosely for the child than for its
    /// deny-by-default parent: the child's own `allow`/`ask` entries are not
    /// escapes the parent granted.
    #[test]
    fn child_posture_is_never_looser_than_a_deny_default_parent() {
        let parent = PermissionPosture {
            default_posture: Decision::Deny,
            allow: vec!["fs.read".into()],
            ask: vec!["pane.send".into()],
            deny: vec![],
        };
        let child = PermissionPosture {
            default_posture: Decision::Ask,
            allow: vec!["fs.read".into(), "net.fetch".into()],
            ask: vec!["secret.get".into()],
            deny: vec![],
        };
        let merged = child.clone().restricted_by(&parent);
        let store = GrantStore::default();
        let strictness = |d: Decision| match d {
            Decision::Allow => 0,
            Decision::Ask => 1,
            Decision::Deny => 2,
        };
        for target in ["fs.read", "pane.send", "net.fetch", "secret.get", "other"] {
            let request = PermissionRequest::new(
                ActorType::Agent,
                "child",
                TargetType::HostTool,
                target,
                None,
            );
            let of = |p: &PermissionPosture| store.evaluate(&request, Some(p));
            let floor = strictness(of(&parent)).max(strictness(of(&child)));
            assert!(
                strictness(of(&merged)) >= floor,
                "{target}: merged {:?} is looser than parent {:?} / child {:?}",
                of(&merged),
                of(&parent),
                of(&child)
            );
        }
    }

    fn posture(
        default_posture: Decision,
        allow: &[&str],
        ask: &[&str],
        deny: &[&str],
    ) -> PermissionPosture {
        let owned = |l: &[&str]| l.iter().map(|t| t.to_string()).collect();
        PermissionPosture {
            default_posture,
            allow: owned(allow),
            ask: owned(ask),
            deny: owned(deny),
        }
    }

    /// The grant states the meet oracle evaluates under: none, an exact
    /// allow, ask or deny, and an expired allow.
    fn oracle_stores(target: &str, now: i64) -> Vec<(&'static str, GrantStore)> {
        let grant = |decision, expires_at| {
            let mut binding = agent_binding();
            binding.target_id = target.to_string();
            let mut record = GrantRecord::from_binding(
                &binding,
                decision,
                GrantDuration::Always,
                GrantSource::User,
                "oracle",
            );
            record.expires_at = expires_at;
            let mut store = GrantStore::default();
            store.record(record);
            store
        };
        vec![
            ("none", GrantStore::default()),
            ("exact allow", grant(Decision::Allow, None)),
            ("exact ask", grant(Decision::Ask, None)),
            ("exact deny", grant(Decision::Deny, None)),
            ("expired allow", grant(Decision::Allow, Some(now - 1))),
        ]
    }

    fn assert_canonical(p: &PermissionPosture) {
        let mut all: Vec<&String> = p.allow.iter().chain(&p.ask).chain(&p.deny).collect();
        for list in [&p.allow, &p.ask, &p.deny] {
            assert!(
                list.windows(2).all(|w| w[0] < w[1]),
                "unsorted or duplicate: {p:?}"
            );
        }
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n, "exceptions overlap: {p:?}");
        let default_rule = TargetRule::of_default(p.default_posture);
        for target in all {
            assert_ne!(
                p.rule_for(target),
                default_rule,
                "redundant exception {target}: {p:?}"
            );
        }
    }

    /// Exhaustive single-target oracle (stint 1632): for every pair of
    /// postures over one mentioned target and one unmentioned target, the
    /// meet decides each request exactly as the stricter operand does, for
    /// the same request, time and store, under every grant state.
    #[test]
    fn child_posture_meet_matches_the_stricter_evaluation_exhaustively() {
        let strictness = |d: Decision| match d {
            Decision::Allow => 0,
            Decision::Ask => 1,
            Decision::Deny => 2,
        };
        let now = 1_000_000;
        let mut operands = Vec::new();
        for default_posture in [Decision::Allow, Decision::Ask, Decision::Deny] {
            for bits in 0..8u8 {
                let pick = |bit: u8| if bits & bit != 0 { vec!["t"] } else { vec![] };
                operands.push(posture(default_posture, &pick(1), &pick(2), &pick(4)));
            }
        }
        let mut checked = 0;
        for a in &operands {
            for b in &operands {
                let meet = a.clone().restricted_by(b);
                for target in ["t", "unmentioned"] {
                    let mut binding = agent_binding();
                    binding.target_id = target.to_string();
                    let request = PermissionRequest::exact(&binding);
                    for (grants, store) in oracle_stores(target, now) {
                        let of = |p: &PermissionPosture| store.evaluate_at(&request, Some(p), now);
                        let stricter = if strictness(of(a)) >= strictness(of(b)) {
                            of(a)
                        } else {
                            of(b)
                        };
                        assert_eq!(
                            of(&meet),
                            stricter,
                            "{target} under {grants}: meet {meet:?} of {a:?} and {b:?}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 24 * 24 * 2 * 5);
        for a in &operands {
            for b in &operands {
                assert_canonical(&a.clone().restricted_by(b));
            }
        }
    }

    /// Round two's witness and its converse: neither side's exception
    /// escapes the other's fallback deny, and an explicit deny under a deny
    /// default still beats an exact allow grant.
    #[test]
    fn child_posture_meet_keeps_fallback_and_explicit_denies() {
        let child = posture(Decision::Deny, &[], &[], &[]);
        let parent = posture(Decision::Deny, &[], &["pane.send"], &[]);
        assert_eq!(child.clone().restricted_by(&parent), child);
        assert_eq!(parent.clone().restricted_by(&child), child);

        let explicit = posture(Decision::Deny, &[], &[], &["pane.send"]);
        let meet = explicit
            .clone()
            .restricted_by(&posture(Decision::Allow, &[], &[], &[]));
        assert_eq!(meet, explicit);
        let mut binding = agent_binding();
        binding.target_id = "pane.send".to_string();
        let mut store = GrantStore::default();
        store.record(GrantRecord::from_binding(
            &binding,
            Decision::Allow,
            GrantDuration::Always,
            GrantSource::User,
            "g",
        ));
        let request = PermissionRequest::exact(&binding);
        assert_eq!(store.evaluate_at(&request, Some(&meet), 0), Decision::Deny);
        assert_eq!(
            store.evaluate_at(&request, Some(&child), 0),
            Decision::Allow
        );
    }

    /// Generated multi-target postures: the meet is commutative,
    /// associative, idempotent, canonical, and independent of duplicate or
    /// reordered exceptions.
    #[test]
    fn child_posture_meet_is_a_canonical_semilattice() {
        let mut seed: u64 = 0x1632;
        let mut next = move |n: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        let targets = ["a", "b", "c", "d"];
        let generate = |next: &mut dyn FnMut(u64) -> u64| {
            let default_posture =
                [Decision::Allow, Decision::Ask, Decision::Deny][next(3) as usize];
            let mut lists: [Vec<String>; 3] = Default::default();
            for _ in 0..next(7) {
                let list = &mut lists[next(3) as usize];
                list.push(targets[next(4) as usize].to_string());
            }
            let [allow, ask, deny] = lists;
            PermissionPosture {
                default_posture,
                allow,
                ask,
                deny,
            }
        };
        for _ in 0..400 {
            let (a, b, c) = (
                generate(&mut next),
                generate(&mut next),
                generate(&mut next),
            );
            let meet = |x: &PermissionPosture, y: &PermissionPosture| x.clone().restricted_by(y);
            let ab = meet(&a, &b);
            assert_canonical(&ab);
            assert_eq!(ab, meet(&b, &a), "commutative: {a:?} {b:?}");
            assert_eq!(
                meet(&ab, &c),
                meet(&a, &meet(&b, &c)),
                "associative: {a:?} {b:?} {c:?}"
            );
            assert_eq!(meet(&a, &a), meet(&meet(&a, &a), &a), "idempotent: {a:?}");
            assert_eq!(meet(&ab, &ab), ab, "idempotent on canonical: {ab:?}");
            let mut shuffled = a.clone();
            for list in [&mut shuffled.allow, &mut shuffled.ask, &mut shuffled.deny] {
                list.extend(list.clone());
                list.reverse();
            }
            assert_eq!(meet(&shuffled, &b), ab, "duplicates and order: {a:?}");
        }
    }

    #[test]
    fn child_posture_inherits_a_parents_default_ask() {
        let parent = PermissionPosture {
            default_posture: Decision::Ask,
            allow: vec![],
            ask: vec![],
            deny: vec![],
        };
        let child = PermissionPosture {
            default_posture: Decision::Allow,
            allow: vec![],
            ask: vec![],
            deny: vec![],
        }
        .restricted_by(&parent);

        assert_eq!(child.default_posture, Decision::Ask);
    }

    #[test]
    fn persisted_deny_beats_posture_allow_list() {
        let mut store = GrantStore::default();
        store.record(agent_grant(Decision::Deny, GrantSource::User));
        let posture = PermissionPosture {
            default_posture: Decision::Allow,
            allow: vec!["chess.make_move".to_string()],
            ask: vec![],
            deny: vec![],
        };
        assert_eq!(
            store.evaluate(&agent_request(), Some(&posture)),
            Decision::Deny
        );
    }

    #[test]
    fn agent_actor_evaluates_same_path_as_view() {
        // Same target id, different actor types — grants must not bleed.
        let mut store = GrantStore::default();
        store.record(agent_grant(Decision::Allow, GrantSource::User));
        assert_eq!(store.evaluate(&agent_request(), None), Decision::Allow);

        let plugin_req = PermissionRequest::new(
            ActorType::Plugin,
            "chess-opponent",
            TargetType::PluginConnector,
            "chess.make_move",
            None,
        );
        assert_eq!(
            store.evaluate(&plugin_req, None),
            Decision::Ask,
            "an agent grant must not satisfy a plugin request"
        );
    }

    #[test]
    fn workspace_scoping_respected() {
        let ws_a = tempfile::tempdir().unwrap();
        let ws_b = tempfile::tempdir().unwrap();
        let mut store = GrantStore::default();
        store.record(GrantRecord::plugin_capability(
            "my-app",
            ws_a.path(),
            Capability::FsWrite,
            Decision::Allow,
        ));
        let req_a =
            PermissionRequest::plugin_capability("my-app", ws_a.path(), Capability::FsWrite);
        let req_b =
            PermissionRequest::plugin_capability("my-app", ws_b.path(), Capability::FsWrite);
        assert_eq!(store.evaluate(&req_a, None), Decision::Allow);
        assert_eq!(
            store.evaluate(&req_b, None),
            Decision::Ask,
            "grant in workspace A must not bleed into workspace B"
        );
    }

    #[test]
    fn expired_record_is_ignored() {
        let mut store = GrantStore::default();
        let mut g = agent_grant(Decision::Allow, GrantSource::User);
        g.expires_at = Some(crate::platform::clock::now_secs() as i64 - 10);
        store.record(g);
        assert_eq!(store.evaluate(&agent_request(), None), Decision::Ask);
    }

    #[test]
    fn record_replaces_same_identity_same_source() {
        let mut store = GrantStore::default();
        store.record(agent_grant(Decision::Allow, GrantSource::User));
        store.record(agent_grant(Decision::Deny, GrantSource::User));
        assert_eq!(
            store.records().len(),
            1,
            "same identity+source must replace"
        );
        assert_eq!(store.evaluate(&agent_request(), None), Decision::Deny);
    }

    #[test]
    fn revoke_removes_matching_records_only() {
        let mut store = GrantStore::default();
        store.record(agent_grant(Decision::Allow, GrantSource::User));
        store.record(agent_grant(Decision::Allow, GrantSource::Session));
        assert_eq!(
            store.revoke(ActorType::Agent, "chess-opponent", "chess.make_move"),
            2
        );
        assert!(store.records().is_empty());
        assert_eq!(store.evaluate(&agent_request(), None), Decision::Ask);
        assert_eq!(
            store.revoke(ActorType::Agent, "chess-opponent", "chess.make_move"),
            0,
            "second revoke must be a no-op"
        );
    }

    #[test]
    fn persistence_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let mut store = GrantStore::load_or_default(tmp.path()).unwrap();
        store.record(GrantRecord::plugin_capability(
            "my-app",
            ws.path(),
            Capability::NetHttp,
            Decision::Deny,
        ));
        store.record(agent_grant(Decision::Allow, GrantSource::User));
        store.save();

        let reloaded = GrantStore::load_or_default(tmp.path()).unwrap();
        assert_eq!(reloaded.records().len(), 2);
        assert_eq!(reloaded.evaluate(&agent_request(), None), Decision::Allow);
        let req = PermissionRequest::plugin_capability("my-app", ws.path(), Capability::NetHttp);
        assert_eq!(reloaded.evaluate(&req, None), Decision::Deny);
    }

    #[test]
    fn corrupt_grants_file_backed_up() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("grants.toml");
        std::fs::write(&path, b"not toml ][[[").unwrap();
        let store = GrantStore::load_or_default(tmp.path()).unwrap();
        assert!(!path.exists(), "corrupt grants.toml must be renamed away");
        assert!(store.records().is_empty());
        let backup_exists = std::fs::read_dir(tmp.path()).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("grants.toml.untrusted-")
        });
        assert!(backup_exists);
        assert!(
            store
                .integrity_faults()
                .iter()
                .any(|fault| fault.file == "grants.toml"),
            "an unsigned grants file is an integrity fault"
        );
    }

    #[test]
    fn legacy_migration_is_loss_free_and_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();

        // Seed a legacy permissions.toml with all three states.
        let mut legacy = PermissionStore::load_or_default(tmp.path()).unwrap();
        legacy.set(
            "my-app",
            ws.path(),
            Capability::FsWrite,
            PermissionState::Green,
        );
        legacy.set(
            "my-app",
            ws.path(),
            Capability::NetHttp,
            PermissionState::Red,
        );
        legacy.set(
            "my-app",
            ws.path(),
            Capability::AiQuery,
            PermissionState::Yellow,
        );
        legacy.save();

        // First load migrates all three, mapped per the spec table.
        let store = GrantStore::load_or_default(tmp.path()).unwrap();
        assert_eq!(store.records().len(), 3, "migration must be loss-free");
        let ws_canon = ws.path().canonicalize().unwrap();
        for (cap, expected) in [
            (Capability::FsWrite, Decision::Allow),
            (Capability::NetHttp, Decision::Deny),
            (Capability::AiQuery, Decision::Ask),
        ] {
            let r = store
                .records()
                .iter()
                .find(|r| r.target_id == cap.as_str())
                .unwrap_or_else(|| panic!("missing migrated record for {cap}"));
            assert_eq!(r.decision, expected);
            assert_eq!(r.actor_type, ActorType::Plugin);
            assert_eq!(r.actor_id, "my-app");
            assert_eq!(r.target_type, TargetType::Capability);
            assert_eq!(r.resource_scope, ResourceScope::Workspace);
            assert_eq!(r.workspace_root.as_deref(), Some(ws_canon.as_path()));
        }

        assert!(store
            .records()
            .iter()
            .all(|record| !record.grant_id.is_empty()));
        assert!(
            !tmp.path().join("permissions.toml").exists(),
            "a successful import retires permissions.toml"
        );

        // Second load must not duplicate. The retired file is not enforcement.
        let store2 = GrantStore::load_or_default(tmp.path()).unwrap();
        assert_eq!(store2.records().len(), 3, "migration must be idempotent");
        std::fs::remove_file(tmp.path().join("grants.toml")).unwrap();
        // Re-seed happens only from a live permissions.toml. With both gone,
        // a fresh load is empty — the retired copy is not read back.
        let retired = std::fs::read_dir(tmp.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("permissions.toml.imported-")
        });
        assert!(retired, "import leaves a retired copy");

        // Migrated records evaluate correctly.
        let req = PermissionRequest::plugin_capability("my-app", ws.path(), Capability::FsWrite);
        assert_eq!(store2.evaluate(&req, None), Decision::Allow);
        let req = PermissionRequest::plugin_capability("my-app", ws.path(), Capability::NetHttp);
        assert_eq!(store2.evaluate(&req, None), Decision::Deny);
    }

    #[test]
    fn rejected_grants_do_not_import_legacy_permissions() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let mut legacy = PermissionStore::load_or_default(tmp.path()).unwrap();
        legacy.set(
            "my-app",
            ws.path(),
            Capability::FsRead,
            PermissionState::Green,
        );
        legacy.save();
        std::fs::write(
            tmp.path().join("grants.toml"),
            b"[[records]]\nactor_id = \"forged\"\n# plexi-mac:00\n",
        )
        .unwrap();
        let store = GrantStore::load_or_default(tmp.path()).unwrap();
        assert!(store.records().is_empty());
        assert!(tmp.path().join("permissions.toml").is_file());
        assert!(!store.integrity_faults().is_empty());
    }

    #[test]
    fn declared_non_sensitive_caps_are_listed_and_sensitive_caps_wait() {
        let profile = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let monitor = gate::PermissionMonitor::open_profile(profile.path()).unwrap();
        let declared =
            std::collections::HashSet::from([Capability::Timer, Capability::PermissionsManage]);
        let perms = monitor.materialize_plugin_permissions(
            &principal::PluginInstance::new("sample", None),
            ws.path(),
            &declared,
            Vec::new(),
        );
        assert!(perms.capabilities.contains(&Capability::Timer));
        assert!(!perms.capabilities.contains(&Capability::PermissionsManage));
        let listed = monitor.list_entries();
        assert!(listed.iter().any(|entry| entry.id == "cap:sample:timer"));
        assert!(listed
            .iter()
            .all(|entry| entry.id != "cap:sample:permissions.manage"));
        let again = monitor.materialize_plugin_permissions(
            &principal::PluginInstance::new("sample", None),
            ws.path(),
            &declared,
            Vec::new(),
        );
        assert_eq!(again.capabilities, perms.capabilities);
        assert_eq!(
            monitor
                .store()
                .records()
                .iter()
                .filter(|record| record.target_id == "timer")
                .count(),
            1
        );
        std::fs::remove_file(profile.path().join("permissions.toml")).ok();
        let reopened = gate::PermissionMonitor::open_profile(profile.path()).unwrap();
        assert_eq!(
            reopened.store().evaluate(
                &PermissionRequest::plugin_capability("sample", ws.path(), Capability::Timer),
                None,
            ),
            Decision::Allow
        );
        assert_eq!(
            reopened.store().evaluate(
                &PermissionRequest::plugin_capability(
                    "sample",
                    ws.path(),
                    Capability::PermissionsManage
                ),
                None,
            ),
            Decision::Ask
        );
    }

    #[test]
    fn plugin_capability_sets_reads_generalized_records() {
        let ws = tempfile::tempdir().unwrap();
        let mut store = GrantStore::default();
        store.record(GrantRecord::plugin_capability(
            "my-app",
            ws.path(),
            Capability::FsWrite,
            Decision::Allow,
        ));
        store.record(GrantRecord::plugin_capability(
            "my-app",
            ws.path(),
            Capability::NetHttp,
            Decision::Deny,
        ));
        store.record(GrantRecord::plugin_capability(
            "other-app",
            ws.path(),
            Capability::PanesRead,
            Decision::Allow,
        ));
        let (allowed, denied) = store.plugin_capability_sets("my-app", ws.path());
        assert!(allowed.contains(&Capability::FsWrite));
        assert!(denied.contains(&Capability::NetHttp));
        assert!(
            !allowed.contains(&Capability::PanesRead),
            "other-plugin's grant must not bleed"
        );
    }

    #[test]
    fn posture_parses_from_permissions_table() {
        let posture = PermissionPosture::from_toml_str(
            r#"
[permissions]
default_posture = "review"
allow = ["host.panes.read"]
ask = ["app.chess.make_move"]
deny = ["host.secrets.read"]
"#,
        )
        .expect("spec example must parse");
        assert_eq!(posture.default_posture, Decision::Ask, "review maps to ask");
        assert_eq!(posture.allow, vec!["host.panes.read"]);
        assert_eq!(posture.ask, vec!["app.chess.make_move"]);
        assert_eq!(posture.deny, vec!["host.secrets.read"]);
    }

    #[test]
    fn posture_loads_from_config_toml() {
        let tmp = tempfile::tempdir().unwrap();
        // Absent file → None.
        assert_eq!(PermissionPosture::load_from_config(tmp.path()), None);

        // Config without a [permissions] table → None.
        std::fs::write(tmp.path().join("config.toml"), "font_size = 14.0\n").unwrap();
        assert_eq!(PermissionPosture::load_from_config(tmp.path()), None);

        // Config with a valid [permissions] table alongside other tables.
        std::fs::write(
            tmp.path().join("config.toml"),
            "font_size = 14.0\n\n[permissions]\ndefault_posture = \"ask\"\ndeny = [\"net.http\"]\n",
        )
        .unwrap();
        let posture =
            PermissionPosture::load_from_config(tmp.path()).expect("valid table must load");
        assert_eq!(posture.default_posture, Decision::Ask);
        assert_eq!(posture.deny, vec!["net.http"]);

        // Invalid table → None (logged, never widens permissions).
        std::fs::write(
            tmp.path().join("config.toml"),
            "[permissions]\ndefault_posture = \"whatever\"\n",
        )
        .unwrap();
        assert_eq!(PermissionPosture::load_from_config(tmp.path()), None);
    }

    #[test]
    fn posture_rejects_unknown_default_and_missing_table() {
        let err =
            PermissionPosture::from_toml_str("[permissions]\ndefault_posture = \"sometimes\"\n")
                .unwrap_err();
        assert!(
            err.contains("sometimes"),
            "error must name the bad value: {err}"
        );
        assert!(
            PermissionPosture::from_toml_str("[other]\nx = 1\n").is_err(),
            "missing [permissions] table must fail, not silently default"
        );
    }

    #[test]
    fn grants_toml_without_kind_loads_as_hard() {
        let dir = tempfile::tempdir().unwrap();
        let legacy_toml = r#"[[records]]
actor_type = "agent"
actor_id = "agent:lead"
actor_scope = "workspace"
workspace_root = "/tmp/ws"
target_type = "host_tool"
target_id = "agents.ping"
resource_scope = "workspace"
decision = "allow"
duration = "always"
source = "user"
created_at = 1700000000
grant_id = "grant_legacy_1"
args_unbound = true
"#;
        // A signed file from a build that predates `kind`; an unsigned file is
        // quarantined, never loaded.
        seal::write_sealed(
            &custody::HostProcess::for_test(),
            &dir.path().join("grants.toml"),
            legacy_toml.as_bytes(),
        )
        .unwrap();
        let store = GrantStore::load_or_default(dir.path()).unwrap();
        assert_eq!(store.records().len(), 1);
        assert_eq!(store.records()[0].grant_id, "grant_legacy_1");
        assert_eq!(store.records()[0].kind, GrantKind::Hard);
        assert_eq!(store.records()[0].expires_at, None);
    }
}
