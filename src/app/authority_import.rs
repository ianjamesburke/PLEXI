//! Restoring terminal grants from a saved workspace (stint 1632).
//!
//! One pre-construction pass over the whole workspace decides the grant each
//! saved terminal is restored under, before any pane is built:
//!
//! - A current-format record is validated (version, profile, pane and
//!   context) and restored whole, with its effective scope. A record that
//!   fails any check, or does not decode, is sealed.
//! - A pre-grant (legacy) save is imported. Every window and conjoined
//!   context is indexed so a parent is found wherever it was saved; ancestry
//!   is walked with cycle detection; and each ancestor's recoverable
//!   restriction is met with the child's independently of whether the
//!   child's own role still resolves. A missing parent, an unresolvable role
//!   with no saved ceiling, a cycle, or a plain shell (whose provenance a
//!   legacy save cannot show) yields a sealed grant.
//!
//! Nothing observed about a restored pane (its session, badge, harness or
//! the current role files) can widen what this pass decides. The complete
//! record is written on the next ordinary save.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::broker::pane_authority::{
    Ceiling, CredentialScope, Delegation, LiveScope, Ownership, PaneAuthority, Provenance,
    SavedPaneAuthority, SealReason, SecretEligibility, GRANT_VERSION,
};
use crate::workspace::{SavedPane, SavedPaneKind, WorkspaceFile};

/// The grant and starting scope each saved terminal is restored under.
pub(crate) type RestoredGrants = HashMap<u64, (PaneAuthority, LiveScope)>;

/// What a legacy save lets an importer recover about one pane.
#[derive(Clone, Debug)]
struct Restriction {
    ceiling: Ceiling,
    can_call: Delegation,
    can_handoff: Delegation,
    /// The grant was recovered from a current-format record of desktop
    /// ownership.
    desktop: bool,
}

impl Restriction {
    fn sealed(reason: SealReason) -> Self {
        Self {
            ceiling: Ceiling::Sealed { reason },
            can_call: Delegation::none(),
            can_handoff: Delegation::none(),
            desktop: false,
        }
    }

    fn meet(&self, other: &Self) -> Self {
        Self {
            ceiling: self.ceiling.meet(&other.ceiling),
            can_call: self.can_call.meet(&other.can_call),
            can_handoff: self.can_handoff.meet(&other.can_handoff),
            desktop: self.desktop && other.desktop,
        }
    }
}

/// Decide every saved terminal's grant. `resolve_role` reads a role
/// definition for a legacy agent pane (`None` when it does not resolve).
pub(crate) fn import(
    ws: &WorkspaceFile,
    profile: &Path,
    resolve_role: &dyn Fn(&str, &Path) -> Option<crate::broker::pane_authority::AdmittedRole>,
) -> RestoredGrants {
    let roots: HashMap<u64, PathBuf> = ws
        .contexts
        .iter()
        .map(|c| (c.context_id, c.root.clone()))
        .collect();
    // Every saved pane, with the context its window is in.
    let panes: HashMap<u64, (&SavedPane, u64)> = ws
        .windows
        .iter()
        .flat_map(|win| {
            win.panes
                .iter()
                .map(move |pane| (pane.id, (pane, win.context_id)))
        })
        .collect();
    let importer = Importer {
        profile,
        roots: &roots,
        panes: &panes,
        conjoined: &ws.conjoined_contexts,
        resolve_role,
    };
    let mut memo = HashMap::new();
    let mut grants = RestoredGrants::new();
    for (&pane_id, &(pane, context_id)) in &panes {
        if !matches!(pane.kind, SavedPaneKind::Terminal) {
            continue;
        }
        let restored = importer.restore(pane, context_id, &mut memo);
        log::info!(
            "workspace_restore: pane {pane_id} grant from {} ownership={:?} ceiling={}",
            if pane.authority.is_some() {
                "its record"
            } else {
                "legacy import"
            },
            restored.0.ownership,
            match restored.0.ceiling {
                Ceiling::Baseline => "baseline",
                Ceiling::Posture { .. } => "posture",
                Ceiling::Sealed { .. } => "sealed",
            }
        );
        grants.insert(pane_id, restored);
    }
    grants
}

struct Importer<'a> {
    profile: &'a Path,
    roots: &'a HashMap<u64, PathBuf>,
    panes: &'a HashMap<u64, (&'a SavedPane, u64)>,
    conjoined: &'a HashMap<u64, u64>,
    resolve_role: &'a dyn Fn(&str, &Path) -> Option<crate::broker::pane_authority::AdmittedRole>,
}

impl Importer<'_> {
    fn scope_of(&self, pane: &SavedPane, context_id: u64) -> CredentialScope {
        let root = self
            .roots
            .get(&context_id)
            .cloned()
            .unwrap_or_else(|| pane.cwd.clone());
        CredentialScope {
            context_id,
            workspace_root: crate::platform::path::canonical_or_self(&root),
        }
    }

    /// A current-format record, if it validates for this pane.
    fn current(
        &self,
        pane: &SavedPane,
        context_id: u64,
    ) -> Option<Result<SavedPaneAuthority, String>> {
        let value = pane.authority.as_ref()?;
        let saved = match serde_json::from_value::<SavedPaneAuthority>(value.clone()) {
            Ok(saved) => saved,
            Err(error) => return Some(Err(format!("record does not decode: {error}"))),
        };
        let grant = &saved.grant;
        let problem = if grant.version != GRANT_VERSION {
            Some(format!("unknown version {}", grant.version))
        } else if grant.profile != self.profile {
            Some(format!("admitted in profile {}", grant.profile.display()))
        } else if grant.pane_id != pane.id {
            Some(format!("record names pane {}", grant.pane_id))
        } else if saved.scope.context_id != context_id {
            Some(format!(
                "record names context {}, saved in context {context_id}",
                saved.scope.context_id
            ))
        } else {
            None
        };
        Some(match problem {
            Some(problem) => Err(problem),
            None => Ok(saved),
        })
    }

    fn restore(
        &self,
        pane: &SavedPane,
        context_id: u64,
        memo: &mut HashMap<u64, Restriction>,
    ) -> (PaneAuthority, LiveScope) {
        let scope = self.scope_of(pane, context_id);
        match self.current(pane, context_id) {
            Some(Ok(saved)) => return (saved.grant, saved.scope),
            Some(Err(problem)) => {
                log::warn!(
                    "workspace_restore: pane {} grant refused: {problem}; sealed",
                    pane.id
                );
                return self.sealed(SealReason::UnreadableRecord, pane, scope);
            }
            None => {}
        }
        let restriction = self.restriction(pane.id, memo, &mut HashSet::new());
        if let Ceiling::Sealed { reason } = restriction.ceiling {
            return self.sealed(reason, pane, scope);
        }
        let session = pane.agent_session.as_ref();
        let grant = PaneAuthority {
            version: GRANT_VERSION,
            profile: self.profile.to_path_buf(),
            pane_id: pane.id,
            provenance: Provenance::LegacyImport,
            spawned_by: session
                .and_then(|s| s.spawned_by.clone())
                .unwrap_or_else(|| "unknown".into()),
            parent: self.parent_of(pane.id),
            // Desktop only where the policy above trusts the save to say so.
            ownership: if restriction.desktop {
                Ownership::Desktop
            } else {
                Ownership::Restricted
            },
            role: session.and_then(|s| s.config_name.clone()),
            can_call: restriction.can_call,
            can_handoff: restriction.can_handoff,
            ceiling: restriction.ceiling,
            secrets: SecretEligibility::none(),
            scope: scope.clone(),
        };
        let live = LiveScope {
            context_id: scope.context_id,
            workspace_root: scope.workspace_root,
            revision: 0,
        };
        (grant, live)
    }

    fn sealed(
        &self,
        reason: SealReason,
        pane: &SavedPane,
        scope: CredentialScope,
    ) -> (PaneAuthority, LiveScope) {
        let live = LiveScope {
            context_id: scope.context_id,
            workspace_root: scope.workspace_root.clone(),
            revision: 0,
        };
        let grant = PaneAuthority::sealed(
            reason,
            self.profile,
            pane.id,
            Provenance::LegacyImport,
            self.parent_of(pane.id),
            pane.agent_session
                .as_ref()
                .and_then(|s| s.spawned_by.clone())
                .unwrap_or_else(|| "unknown".into()),
            scope,
        );
        (grant, live)
    }

    /// The tree parent a legacy save records: the session's parent, or the
    /// pane a conjoined context hangs under. A current-format record names
    /// its own.
    fn parent_of(&self, pane_id: u64) -> Option<u64> {
        let (pane, context_id) = self.panes.get(&pane_id)?;
        if let Some(Ok(saved)) = self.current(pane, *context_id) {
            return saved.grant.parent;
        }
        pane.agent_session
            .as_ref()
            .and_then(|s| s.parent_pane_id)
            .or_else(|| self.conjoined.get(context_id).copied())
            .filter(|parent| *parent != pane_id)
    }

    /// The restriction that bounds `pane_id`: its own, met with every
    /// ancestor's. A missing ancestor or a cycle seals.
    fn restriction(
        &self,
        pane_id: u64,
        memo: &mut HashMap<u64, Restriction>,
        visiting: &mut HashSet<u64>,
    ) -> Restriction {
        if let Some(known) = memo.get(&pane_id) {
            return known.clone();
        }
        if !visiting.insert(pane_id) {
            log::warn!("workspace_restore: ancestry cycle through pane {pane_id}; sealed");
            return Restriction::sealed(SealReason::OlderSave);
        }
        let Some((pane, context_id)) = self.panes.get(&pane_id) else {
            log::warn!("workspace_restore: ancestor pane {pane_id} is not in the save; sealed");
            return Restriction::sealed(SealReason::OlderSave);
        };
        let own = self.own_restriction(pane, *context_id);
        let result = match self.parent_of(pane_id) {
            // A current-format record already carries its ancestry.
            _ if pane.authority.is_some() => own,
            Some(parent) => own.meet(&self.restriction(parent, memo, visiting)),
            None => own,
        };
        visiting.remove(&pane_id);
        memo.insert(pane_id, result.clone());
        result
    }

    /// What a pane's own saved fields establish, without its ancestors.
    fn own_restriction(&self, pane: &SavedPane, context_id: u64) -> Restriction {
        match self.current(pane, context_id) {
            Some(Ok(saved)) => {
                return Restriction {
                    desktop: saved.grant.ownership == Ownership::Desktop,
                    ceiling: saved.grant.ceiling,
                    can_call: saved.grant.can_call,
                    can_handoff: saved.grant.can_handoff,
                }
            }
            Some(Err(_)) => return Restriction::sealed(SealReason::UnreadableRecord),
            None => {}
        }
        if !matches!(pane.kind, SavedPaneKind::Terminal) {
            // A view or portal ancestor holds no grant: it restricts nothing
            // of its own.
            return Restriction {
                ceiling: Ceiling::Baseline,
                can_call: Delegation::Unrestricted,
                can_handoff: Delegation::Unrestricted,
                desktop: false,
            };
        }
        let Some(session) = pane.agent_session.as_ref() else {
            return legacy_plain_shell(pane.id);
        };
        let lookup = pane.cwd.clone();
        let role = session
            .config_name
            .as_deref()
            .and_then(|role| (self.resolve_role)(role, &lookup));
        let ceiling = match (&session.permission_posture, &role) {
            (Some(saved), _) => Ceiling::Posture {
                posture: saved.clone(),
            },
            (None, Some(role)) => Ceiling::Posture {
                posture: role.posture.clone(),
            },
            (None, None) => {
                log::info!(
                    "workspace_restore: legacy agent pane {} has no saved ceiling and role {:?} does not resolve; sealed",
                    pane.id,
                    session.config_name
                );
                return Restriction::sealed(SealReason::OlderSave);
            }
        };
        let (can_call, can_handoff) = match &role {
            Some(role) => (
                Delegation::only(&role.can_call),
                Delegation::only(&role.can_handoff),
            ),
            None => (Delegation::none(), Delegation::none()),
        };
        Restriction {
            ceiling,
            can_call,
            can_handoff,
            desktop: false,
        }
    }
}

/// The policy for a plain shell restored from a save made before panes
/// recorded their grant (ledger row 155). Such a save cannot show whether the
/// person or an agent opened the shell, so it is sealed: it keeps working as a
/// shell, says why in its own pane, and refuses brokered calls with that
/// reason. Trusting old saves once instead would return a desktop
/// restriction here; nothing else changes.
fn legacy_plain_shell(pane_id: u64) -> Restriction {
    log::info!(
        "workspace_restore: legacy plain shell {pane_id} has no provable provenance; sealed"
    );
    Restriction::sealed(SealReason::OlderSave)
}

/// A pane restored with an effective agent session (one the restore mode
/// keeps) resumes that agent: it is restricted and resolves no secret,
/// whatever its grant allowed the shell it started in. Alpha withheld secrets
/// on exactly this condition. A session the restore mode drops leaves a
/// plain shell under its saved grant (stint 1632, rounds 6 and 7).
pub(crate) fn agent_session_narrowed(pane_id: u64, mut grant: PaneAuthority) -> PaneAuthority {
    if grant.ownership == Ownership::Desktop || grant.secrets != SecretEligibility::none() {
        log::info!(
            "workspace_restore: pane {pane_id} resumes an agent session; restricted, no secrets"
        );
        grant.ownership = Ownership::Restricted;
        grant.secrets = SecretEligibility::none();
    }
    grant
}
