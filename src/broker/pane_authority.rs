//! The authority a terminal pane runs under (stint 1632).
//!
//! [`PaneAuthority`] is the durable grant: computed once, at launch
//! admission, from the verified launch source and the parent's own grant, by
//! [`PaneAuthority::admit`]. It fixes the pane's principal, provenance,
//! ownership, delegation limits, permission ceiling, secret eligibility and
//! admitted credential scope. It is saved whole for every terminal and
//! restored whole; nothing observed about a running pane (a session, a badge,
//! a role file, a harness) ever adds to it.
//!
//! [`LivePaneAuthority`] is one terminal incarnation's revocable handle to
//! the grant. The pane, its verified-peer lookup and both of its credentials
//! (the call credential and the host-MCP bearer) share the one handle, so
//! every transport resolves the same grant and the same scope revision. A
//! host-authorized scope change ([`LivePaneAuthority::publish_scope`]) is
//! the only way the scope moves, and it moves for every transport at once.
//!
//! A grant contains bounds, never bearer tokens, secret values or standing
//! approvals.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use super::PermissionPosture;

/// The saved grant format this host writes and reads.
pub(crate) const GRANT_VERSION: u32 = 1;

/// The roles a pane may delegate to with one action (`agent call` or
/// handoff). An empty `Only` delegates to no role.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind", content = "roles")]
pub(crate) enum Delegation {
    Unrestricted,
    Only(BTreeSet<String>),
}

impl Delegation {
    pub(crate) fn none() -> Self {
        Self::Only(BTreeSet::new())
    }

    pub(crate) fn only(roles: &[String]) -> Self {
        Self::Only(roles.iter().cloned().collect())
    }

    pub(crate) fn allows(&self, role: &str) -> bool {
        match self {
            Self::Unrestricted => true,
            Self::Only(roles) => roles.contains(role),
        }
    }

    /// The stricter of two limits: their intersection.
    pub(crate) fn meet(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Unrestricted, limit) | (limit, Self::Unrestricted) => limit.clone(),
            (Self::Only(a), Self::Only(b)) => Self::Only(a.intersection(b).cloned().collect()),
        }
    }
}

/// Whether a pane acts with the person's own authority. Only positive
/// evidence (input the host owns) makes a pane `Desktop`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Ownership {
    Desktop,
    Restricted,
}

/// Who admitted the pane, as the host established it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub(crate) enum Provenance {
    /// Input the host owns: the person at the keyboard or the host's UI.
    DesktopInput,
    /// The host itself: boot, a scheduled routine, workspace setup.
    HostSystem,
    /// A terminal process the host resolved to this pane.
    VerifiedPane { pane_id: u64 },
    /// A plugin view instance.
    PluginInstance { pane_id: u64 },
    /// A socket peer the kernel identified that is no pane of this host.
    OutsideProcess,
    /// A file in the profile's spawn queue: no identity at all.
    SpawnQueue,
    /// A team operation under a run principal.
    Team,
    /// A pre-grant save, imported with what could be recovered from it.
    LegacyImport,
}

/// The pane's permission ceiling.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub(crate) enum Ceiling {
    /// Nothing beyond the profile's own posture: a deliberately
    /// unconfigured pane.
    Baseline,
    /// At most this posture.
    Posture { posture: PermissionPosture },
    /// Nothing is authorized under this pane's identity, whatever grants
    /// exist: authority that could not be established. The pane holds no
    /// credential and is refused before the broker is consulted.
    Sealed,
}

impl Ceiling {
    /// The stricter of two ceilings.
    pub(crate) fn meet(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Sealed, _) | (_, Self::Sealed) => Self::Sealed,
            (Self::Baseline, c) | (c, Self::Baseline) => c.clone(),
            (Self::Posture { posture: a }, Self::Posture { posture: b }) => Self::Posture {
                posture: a.clone().restricted_by(b),
            },
        }
    }

    /// The posture the broker evaluates under (`None` is the baseline), or
    /// a refusal for a sealed ceiling.
    pub(crate) fn posture(&self) -> Result<Option<&PermissionPosture>, SealedAuthority> {
        match self {
            Self::Baseline => Ok(None),
            Self::Posture { posture } => Ok(Some(posture)),
            Self::Sealed => Err(SealedAuthority),
        }
    }
}

/// A pane whose authority could not be established acts for nobody.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SealedAuthority;

impl std::fmt::Display for SealedAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "permission_denied: this pane's authority could not be established; open a new pane",
        )
    }
}

/// The directory-derived secret classes a pane may receive, and the one
/// directory that bounds their resolution.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct SecretEligibility {
    pub(crate) workspace_terminal_env: bool,
    pub(crate) folder: bool,
    pub(crate) directory: Option<PathBuf>,
}

impl SecretEligibility {
    pub(crate) fn none() -> Self {
        Self {
            workspace_terminal_env: false,
            folder: false,
            directory: None,
        }
    }

    /// The classes construction resolves. No directory, no secrets.
    pub(crate) fn policy(&self) -> crate::host::shell::SecretPolicy {
        if self.directory.is_none() {
            return crate::host::shell::SecretPolicy::NONE;
        }
        crate::host::shell::SecretPolicy {
            terminal: self.workspace_terminal_env,
            folder: self.folder,
        }
    }
}

/// The context and workspace root a pane's credentials are scoped to.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct CredentialScope {
    pub(crate) context_id: u64,
    pub(crate) workspace_root: PathBuf,
}

/// A role admitted for a named worker, as its definition resolved at
/// admission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AdmittedRole {
    pub(crate) id: String,
    pub(crate) posture: PermissionPosture,
    pub(crate) can_call: Vec<String>,
    pub(crate) can_handoff: Vec<String>,
}

/// The verified source of a launch.
#[derive(Clone, Debug)]
pub(crate) enum GrantSource {
    DesktopInput,
    HostSystem,
    /// The calling pane's own grant: the new pane is bounded by it.
    VerifiedPane(Box<PaneAuthority>),
    PluginInstance {
        pane_id: u64,
    },
    OutsideProcess,
    SpawnQueue,
    Team,
}

/// Everything [`PaneAuthority::admit`] computes a grant from.
#[derive(Clone, Debug)]
pub(crate) struct GrantRequest {
    pub(crate) pane_id: u64,
    pub(crate) source: GrantSource,
    /// The logical tree parent, when the pane has one. Never a claim from
    /// the wire.
    pub(crate) parent: Option<u64>,
    /// A named worker's admitted role.
    pub(crate) role: Option<AdmittedRole>,
    pub(crate) scope: CredentialScope,
    /// The directory secrets would be resolved from, when the source may
    /// receive any.
    pub(crate) secret_directory: Option<PathBuf>,
    /// Folder secrets are wanted for that directory (a shell for the folder,
    /// not a command run in it).
    pub(crate) folder_secrets: bool,
    /// The pane starts an agent harness: it is restricted whoever asked.
    pub(crate) agent_command: bool,
    /// The display provenance (`human`, `agent:<role>`, `system`, ...).
    pub(crate) spawned_by: String,
}

/// A pane's durable grant. See the module docs.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PaneAuthority {
    pub(crate) version: u32,
    /// The profile the grant was admitted in.
    pub(crate) profile: PathBuf,
    pub(crate) pane_id: u64,
    pub(crate) provenance: Provenance,
    pub(crate) spawned_by: String,
    pub(crate) parent: Option<u64>,
    pub(crate) ownership: Ownership,
    pub(crate) role: Option<String>,
    pub(crate) can_call: Delegation,
    pub(crate) can_handoff: Delegation,
    pub(crate) ceiling: Ceiling,
    pub(crate) secrets: SecretEligibility,
    pub(crate) scope: CredentialScope,
}

impl PaneAuthority {
    /// The one grant builder every terminal producer uses.
    ///
    /// A plain pane inherits its verified caller's ownership, delegation
    /// limits and ceiling unchanged; a named worker gets the meet of those
    /// and its admitted role, independently for call and handoff. Only
    /// input the host owns yields secrets (stint 1285's desktop-only rule):
    /// a pane created on behalf of any other source receives none.
    pub(crate) fn admit(profile: &Path, request: GrantRequest) -> Self {
        let GrantRequest {
            pane_id,
            source,
            parent,
            role,
            scope,
            secret_directory,
            folder_secrets,
            agent_command,
            spawned_by,
        } = request;
        let (provenance, ownership, ceiling, can_call, can_handoff, host_owned) = match source {
            GrantSource::DesktopInput => (
                Provenance::DesktopInput,
                Ownership::Desktop,
                Ceiling::Baseline,
                Delegation::Unrestricted,
                Delegation::Unrestricted,
                true,
            ),
            GrantSource::HostSystem => (
                Provenance::HostSystem,
                Ownership::Desktop,
                Ceiling::Baseline,
                Delegation::Unrestricted,
                Delegation::Unrestricted,
                true,
            ),
            GrantSource::VerifiedPane(caller) => (
                Provenance::VerifiedPane {
                    pane_id: caller.pane_id,
                },
                caller.ownership,
                caller.ceiling,
                caller.can_call,
                caller.can_handoff,
                false,
            ),
            GrantSource::PluginInstance { pane_id } => {
                restricted(Provenance::PluginInstance { pane_id })
            }
            GrantSource::OutsideProcess => restricted(Provenance::OutsideProcess),
            GrantSource::SpawnQueue => restricted(Provenance::SpawnQueue),
            GrantSource::Team => restricted(Provenance::Team),
        };
        let (ownership, ceiling, can_call, can_handoff, role) = match role {
            Some(role) => (
                Ownership::Restricted,
                ceiling.meet(&Ceiling::Posture {
                    posture: role.posture,
                }),
                can_call.meet(&Delegation::only(&role.can_call)),
                can_handoff.meet(&Delegation::only(&role.can_handoff)),
                Some(role.id),
            ),
            None if agent_command => (Ownership::Restricted, ceiling, can_call, can_handoff, None),
            None => (ownership, ceiling, can_call, can_handoff, None),
        };
        let secrets = if host_owned && ownership == Ownership::Desktop {
            SecretEligibility {
                workspace_terminal_env: true,
                folder: folder_secrets
                    && secret_directory.is_some()
                    && crate::release::feature_enabled(
                        crate::release::ReleaseFeature::FolderSecrets,
                    ),
                directory: secret_directory,
            }
        } else {
            SecretEligibility::none()
        };
        let grant = Self {
            version: GRANT_VERSION,
            profile: profile.to_path_buf(),
            pane_id,
            provenance,
            spawned_by,
            parent,
            ownership,
            role,
            can_call,
            can_handoff,
            ceiling,
            secrets,
            scope,
        };
        log::info!(
            "pane_authority: admitted pane={pane_id} provenance={:?} ownership={:?} role={:?} ceiling={} secrets={:?}",
            grant.provenance,
            grant.ownership,
            grant.role,
            grant.ceiling_label(),
            grant.secrets.policy()
        );
        grant
    }

    /// A grant that acts for nobody: no ownership, delegation, secrets or
    /// broker authority. Used where authority could not be established.
    pub(crate) fn sealed(
        profile: &Path,
        pane_id: u64,
        provenance: Provenance,
        parent: Option<u64>,
        spawned_by: String,
        scope: CredentialScope,
    ) -> Self {
        Self {
            version: GRANT_VERSION,
            profile: profile.to_path_buf(),
            pane_id,
            provenance,
            spawned_by,
            parent,
            ownership: Ownership::Restricted,
            role: None,
            can_call: Delegation::none(),
            can_handoff: Delegation::none(),
            ceiling: Ceiling::Sealed,
            secrets: SecretEligibility::none(),
            scope,
        }
    }

    /// This grant, never wider than `recorded`: a pane relaunched from a
    /// record runs under at most the restrictions it ran under then.
    pub(crate) fn narrowed_by(mut self, recorded: Option<&PaneAuthority>) -> Self {
        let Some(recorded) = recorded else {
            return self;
        };
        self.ceiling = self.ceiling.meet(&recorded.ceiling);
        self.can_call = self.can_call.meet(&recorded.can_call);
        self.can_handoff = self.can_handoff.meet(&recorded.can_handoff);
        if !recorded.is_desktop() {
            self.ownership = Ownership::Restricted;
            self.secrets = SecretEligibility::none();
        }
        log::info!(
            "pane_authority: pane {} narrowed by the recorded grant of pane {}",
            self.pane_id,
            recorded.pane_id
        );
        self
    }

    /// The restriction a history record keeps for a relaunch: its saved
    /// grant, or for an older record its saved posture. An undecodable grant
    /// restricts everything.
    pub(crate) fn recorded(
        grant: Option<&serde_json::Value>,
        posture: Option<&PermissionPosture>,
        pane_id: u64,
    ) -> Option<Self> {
        let scope = CredentialScope {
            context_id: 0,
            workspace_root: PathBuf::new(),
        };
        let blank = |ceiling: Ceiling| Self {
            version: GRANT_VERSION,
            profile: PathBuf::new(),
            pane_id,
            provenance: Provenance::LegacyImport,
            spawned_by: String::new(),
            parent: None,
            ownership: Ownership::Restricted,
            role: None,
            can_call: Delegation::Unrestricted,
            can_handoff: Delegation::Unrestricted,
            ceiling,
            secrets: SecretEligibility::none(),
            scope: scope.clone(),
        };
        match grant {
            Some(value) => match serde_json::from_value::<Self>(value.clone()) {
                Ok(grant) if grant.version == GRANT_VERSION => Some(grant),
                Ok(grant) => {
                    log::warn!(
                        "pane_authority: recorded grant of pane {pane_id} has unknown version {}; sealed",
                        grant.version
                    );
                    Some(blank(Ceiling::Sealed))
                }
                Err(error) => {
                    log::warn!("pane_authority: recorded grant of pane {pane_id} unreadable: {error}; sealed");
                    Some(blank(Ceiling::Sealed))
                }
            },
            None => posture.map(|posture| {
                blank(Ceiling::Posture {
                    posture: posture.clone(),
                })
            }),
        }
    }

    pub(crate) fn is_desktop(&self) -> bool {
        self.ownership == Ownership::Desktop
    }

    /// The broker posture for this pane, or a refusal for a sealed one.
    pub(crate) fn posture(&self) -> Result<Option<&PermissionPosture>, SealedAuthority> {
        self.ceiling.posture()
    }

    /// The delegation limit for one action.
    pub(crate) fn delegation(&self, handoff: bool) -> &Delegation {
        if handoff {
            &self.can_handoff
        } else {
            &self.can_call
        }
    }

    fn ceiling_label(&self) -> &'static str {
        match self.ceiling {
            Ceiling::Baseline => "baseline",
            Ceiling::Posture { .. } => "posture",
            Ceiling::Sealed => "sealed",
        }
    }
}

#[cfg(test)]
impl PaneAuthority {
    /// A restricted grant for a test pane under `ceiling`.
    pub(crate) fn for_test(
        pane_id: u64,
        ceiling: Ceiling,
        context_id: u64,
        workspace_root: PathBuf,
    ) -> Self {
        Self {
            version: GRANT_VERSION,
            profile: crate::config::config_dir(),
            pane_id,
            provenance: Provenance::OutsideProcess,
            spawned_by: "test".into(),
            parent: None,
            ownership: Ownership::Restricted,
            role: None,
            can_call: Delegation::Unrestricted,
            can_handoff: Delegation::Unrestricted,
            ceiling,
            secrets: SecretEligibility::none(),
            scope: CredentialScope {
                context_id,
                workspace_root,
            },
        }
    }
}

fn restricted(
    provenance: Provenance,
) -> (Provenance, Ownership, Ceiling, Delegation, Delegation, bool) {
    (
        provenance,
        Ownership::Restricted,
        Ceiling::Baseline,
        Delegation::Unrestricted,
        Delegation::Unrestricted,
        false,
    )
}

/// The scope a live pane's credentials resolve to now.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct LiveScope {
    pub(crate) context_id: u64,
    pub(crate) workspace_root: PathBuf,
    /// Advances on every published change; anything admitted under an
    /// earlier revision is rechecked before it takes effect.
    pub(crate) revision: u64,
}

/// A terminal incarnation's lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lifecycle {
    Constructing,
    Live,
    Revoked,
}

/// One terminal incarnation's revocable handle to its grant. See the module
/// docs.
#[derive(Debug)]
pub(crate) struct LivePaneAuthority {
    incarnation: u64,
    grant: PaneAuthority,
    scope: RwLock<LiveScope>,
    state: AtomicU8,
}

static INCARNATIONS: AtomicU64 = AtomicU64::new(1);

impl LivePaneAuthority {
    /// A fresh incarnation of `grant`, starting at `scope`.
    pub(crate) fn new(grant: PaneAuthority, scope: LiveScope) -> Arc<Self> {
        let incarnation = INCARNATIONS.fetch_add(1, Ordering::Relaxed);
        log::info!(
            "pane_authority: incarnation {incarnation} for pane {} at scope revision {}",
            grant.pane_id,
            scope.revision
        );
        Arc::new(Self {
            incarnation,
            grant,
            scope: RwLock::new(scope),
            state: AtomicU8::new(Lifecycle::Constructing as u8),
        })
    }

    /// A fresh incarnation at the grant's admitted scope.
    pub(crate) fn admitted(grant: PaneAuthority) -> Arc<Self> {
        let scope = LiveScope {
            context_id: grant.scope.context_id,
            workspace_root: grant.scope.workspace_root.clone(),
            revision: 0,
        };
        Self::new(grant, scope)
    }

    pub(crate) fn incarnation(&self) -> u64 {
        self.incarnation
    }

    pub(crate) fn grant(&self) -> &PaneAuthority {
        &self.grant
    }

    pub(crate) fn pane_id(&self) -> u64 {
        self.grant.pane_id
    }

    /// One coherent snapshot of the current scope.
    pub(crate) fn scope(&self) -> LiveScope {
        self.scope.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Publish a host-authorized move of the pane's context root. Both
    /// credentials observe it at once. Returns the new revision.
    pub(crate) fn publish_scope(&self, workspace_root: PathBuf) -> u64 {
        let mut scope = self.scope.write().unwrap_or_else(|e| e.into_inner());
        scope.workspace_root = workspace_root;
        scope.revision += 1;
        log::info!(
            "pane_authority: pane {} scope -> {} (revision {})",
            self.grant.pane_id,
            scope.workspace_root.display(),
            scope.revision
        );
        scope.revision
    }

    pub(crate) fn lifecycle(&self) -> Lifecycle {
        match self.state.load(Ordering::Acquire) {
            0 => Lifecycle::Constructing,
            1 => Lifecycle::Live,
            _ => Lifecycle::Revoked,
        }
    }

    /// The pane was installed. A revoked handle stays revoked.
    pub(crate) fn mark_live(&self) {
        let _ = self.state.compare_exchange(
            Lifecycle::Constructing as u8,
            Lifecycle::Live as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    pub(crate) fn revoke(&self) {
        self.state
            .store(Lifecycle::Revoked as u8, Ordering::Release);
    }

    pub(crate) fn is_revoked(&self) -> bool {
        self.lifecycle() == Lifecycle::Revoked
    }
}

/// A grant and the effective scope saved with it, from one revision.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SavedPaneAuthority {
    pub(crate) grant: PaneAuthority,
    pub(crate) scope: LiveScope,
}

impl SavedPaneAuthority {
    pub(crate) fn of(live: &LivePaneAuthority) -> Self {
        Self {
            grant: live.grant().clone(),
            scope: live.scope(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles(names: &[&str]) -> Delegation {
        Delegation::Only(names.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn delegation_meet_is_an_intersection_and_unrestricted_is_its_unit() {
        let a = roles(&["x", "y"]);
        assert_eq!(a.meet(&Delegation::Unrestricted), a);
        assert_eq!(Delegation::Unrestricted.meet(&a), a);
        assert_eq!(a.meet(&roles(&["y", "z"])), roles(&["y"]));
        assert!(!Delegation::none().allows("x"));
    }

    #[test]
    fn a_sealed_ceiling_absorbs_every_other() {
        let posture = PermissionPosture {
            default_posture: crate::broker::Decision::Allow,
            allow: vec![],
            ask: vec![],
            deny: vec![],
        };
        let p = Ceiling::Posture { posture };
        assert_eq!(p.meet(&Ceiling::Sealed), Ceiling::Sealed);
        assert_eq!(Ceiling::Baseline.meet(&p), p);
        assert!(Ceiling::Sealed.posture().is_err());
    }
}
