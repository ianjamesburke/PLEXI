//! Broker-minted permits: the proof a protected host effect requires
//! (stint 1716, `.stint/runs/design-1285/DESIGN.md` section 3).
//!
//! The host classifies a transport into a pane incarnation or an unbound
//! principal. Only [`mint_pane`] and [`mint_unbound`] make a [`Permit`], and
//! each sink takes the permit for its own operation, so a pane number or
//! subscriber id alone reaches nothing. An operation only a pane may hold
//! does not implement [`UnboundOperation`], so minting it without a pane
//! does not compile.
//! A permit holds the pane incarnation and scope revision it was admitted
//! under and the broker grants that justified it; [`Permit::check`]
//! re-validates all of them, and its target, at the effect.

use std::marker::PhantomData;
use std::sync::Arc;

use super::gate::PermissionMonitor;
use crate::host::plugin_timeline::AdmittedScope;

/// One protected effect a permit can be minted for.
pub(crate) trait Operation {
    const NAME: &'static str;
    /// Whether the effect needs a standing broker grant at commit. One that
    /// does refuses until the grant that admitted it is bound.
    const REQUIRES_GRANT: bool;
}

/// An operation a principal with no pane incarnation may also hold.
pub(crate) trait UnboundOperation: Operation {}

/// Write a secret value into a caller's reply.
pub(crate) enum SecretRelease {}

impl Operation for SecretRelease {
    const NAME: &'static str = "secret.release";
    const REQUIRES_GRANT: bool = true;
}

impl SecretRelease {
    /// The canonical target: one secret name in one scope.
    pub(crate) fn target(name: &str, scope: &crate::workspace::secrets::SecretScope) -> String {
        match scope {
            crate::workspace::secrets::SecretScope::Workspace { id, root } => {
                format!("{name}@workspace:{id}:{}", root.display())
            }
            crate::workspace::secrets::SecretScope::Global => format!("{name}@global"),
        }
    }
}

/// Hand a subscriber the events queued for it.
pub(crate) enum DeliverEvents {}

impl Operation for DeliverEvents {
    const NAME: &'static str = "events.deliver";
    const REQUIRES_GRANT: bool = false;
}

impl UnboundOperation for DeliverEvents {}

impl DeliverEvents {
    /// The canonical target: one subscriber of one publisher's streams.
    pub(crate) fn target(
        subscriber_type: super::ActorType,
        subscriber_id: &str,
        plugin_id: &str,
    ) -> String {
        format!("{subscriber_type:?}:{subscriber_id}->{plugin_id}")
    }
}

/// Who a permit is minted for, as the host classified it.
#[derive(Clone, Debug)]
enum PermitSubject {
    /// A pane incarnation at the scope revision it was admitted under.
    Pane(AdmittedScope),
    /// A principal with no pane incarnation (host code, a plugin view, a
    /// run, an outside connection). Its own transport bounds its lifetime.
    Unbound { class: &'static str },
}

impl PermitSubject {
    fn label(&self) -> String {
        match self {
            Self::Pane(scope) => format!("pane:{}@{}", scope.authority.pane_id(), scope.revision),
            Self::Unbound { class } => (*class).to_string(),
        }
    }
}

/// The broker grants an admission rested on, read live from the profile's
/// one grant store at every check.
#[derive(Clone)]
pub(crate) struct Grants {
    monitor: Option<Arc<PermissionMonitor>>,
    ids: Vec<String>,
}

impl Grants {
    /// An admission no grant row stands behind: a person's single consent,
    /// or host code acting for itself. Nothing to revoke.
    pub(crate) fn none() -> Self {
        Self {
            monitor: None,
            ids: Vec::new(),
        }
    }

    /// The rows in `monitor`'s store that allowed the admission. Revoking or
    /// expiring any of them withdraws the permit.
    pub(crate) fn live(monitor: Arc<PermissionMonitor>, ids: Vec<String>) -> Self {
        Self {
            monitor: Some(monitor),
            ids,
        }
    }

    fn check(&self) -> Result<(), String> {
        let Some(monitor) = &self.monitor else {
            return Ok(());
        };
        match self.ids.iter().find(|id| !monitor.grant_stands(id)) {
            Some(gone) => Err(format!("grant {gone} no longer allows this")),
            None => Ok(()),
        }
    }

    fn is_empty(&self) -> bool {
        self.monitor.is_none() || self.ids.is_empty()
    }
}

/// Proof that `subject` was admitted for `Op` on one target.
pub(crate) struct Permit<Op> {
    subject: PermitSubject,
    target: String,
    grants: Grants,
    _op: PhantomData<fn() -> Op>,
}

impl<Op> Clone for Permit<Op> {
    fn clone(&self) -> Self {
        Self {
            subject: self.subject.clone(),
            target: self.target.clone(),
            grants: self.grants.clone(),
            _op: PhantomData,
        }
    }
}

impl<Op: Operation> std::fmt::Debug for Permit<Op> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Permit({} {} {} grants={:?})",
            Op::NAME,
            self.subject.label(),
            self.target,
            self.grants.ids
        )
    }
}

/// Mint for a pane, which must still be at the incarnation and revision it
/// was admitted under.
pub(crate) fn mint_pane<Op: Operation>(
    admitted: AdmittedScope,
    target: String,
    grants: Grants,
) -> Result<Permit<Op>, String> {
    admitted.current()?;
    grants.check()?;
    Ok(minted(PermitSubject::Pane(admitted), target, grants))
}

/// Mint for a principal with no pane incarnation: host code, a plugin view,
/// a run or an outside connection, each bounded by its own transport.
pub(crate) fn mint_unbound<Op: UnboundOperation>(
    class: &'static str,
    target: String,
    grants: Grants,
) -> Result<Permit<Op>, String> {
    grants.check()?;
    Ok(minted(PermitSubject::Unbound { class }, target, grants))
}

/// A fixture subscriber's permit: no pane incarnation and no grant row.
/// Every host subscriber binds the grants that allowed it instead.
#[cfg(test)]
pub(crate) fn mint_host<Op: UnboundOperation>(class: &'static str, target: String) -> Permit<Op> {
    minted(PermitSubject::Unbound { class }, target, Grants::none())
}

fn minted<Op: Operation>(subject: PermitSubject, target: String, grants: Grants) -> Permit<Op> {
    log::info!(
        "permit: minted {} for {} on {target}",
        Op::NAME,
        subject.label()
    );
    Permit {
        subject,
        target,
        grants,
        _op: PhantomData,
    }
}

impl<Op: Operation> Permit<Op> {
    /// Re-validate at the effect: the same target, a pane subject still
    /// unrevoked at the admitted revision, and every grant it rests on still
    /// allowing. An operation that needs a grant refuses until one is bound.
    pub(crate) fn check(&self, target: &str) -> Result<(), String> {
        self.check_subject(target)?;
        if Op::REQUIRES_GRANT && self.grants.is_empty() {
            return Err(format!("{} needs the grant that admitted it", Op::NAME));
        }
        self.grants.check()
    }

    /// The target and pane half of [`Self::check`], for work that runs before
    /// the broker grant is known (a read admitted inside its worker).
    pub(crate) fn check_subject(&self, target: &str) -> Result<(), String> {
        if target != self.target {
            return Err(format!(
                "{} permit is for {}, not {target}",
                Op::NAME,
                self.target
            ));
        }
        match &self.subject {
            PermitSubject::Pane(scope) => scope.current().map(|_| ()),
            PermitSubject::Unbound { .. } => Ok(()),
        }
    }

    /// Bind the grants the broker admitted this use with. Binding only adds
    /// checks; it never widens what the permit allows.
    pub(crate) fn bind(mut self, grants: Grants) -> Result<Self, String> {
        grants.check()?;
        log::info!("permit: {} bound grants {:?}", Op::NAME, grants.ids);
        self.grants = match (self.grants.monitor.take(), grants.monitor) {
            (_, Some(monitor)) => Grants {
                monitor: Some(monitor),
                ids: self.grants.ids.into_iter().chain(grants.ids).collect(),
            },
            (monitor, None) => Grants {
                monitor,
                ids: self.grants.ids,
            },
        };
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::pane_authority::{Ceiling, LivePaneAuthority, PaneAuthority};

    fn pane() -> std::sync::Arc<LivePaneAuthority> {
        LivePaneAuthority::admitted(PaneAuthority::for_test(
            7,
            Ceiling::Baseline,
            1,
            std::path::PathBuf::from("/tmp/ws"),
        ))
    }

    #[test]
    fn a_permit_holds_only_for_its_target_incarnation_and_revision() {
        let live = pane();
        let permit =
            mint_pane::<DeliverEvents>(AdmittedScope::of(&live), "a".into(), Grants::none())
                .unwrap();
        assert!(permit.check("a").is_ok());
        assert!(permit.check("b").is_err(), "another target");
        live.publish_scope(std::path::PathBuf::from("/tmp/elsewhere"));
        assert!(permit.check("a").is_err(), "a moved scope revision");
        let moved =
            mint_pane::<DeliverEvents>(AdmittedScope::of(&live), "a".into(), Grants::none())
                .unwrap();
        live.revoke();
        assert!(moved.check("a").is_err(), "a revoked incarnation");
        assert!(
            mint_pane::<DeliverEvents>(AdmittedScope::of(&live), "a".into(), Grants::none())
                .is_err()
        );
    }

    #[test]
    fn a_secret_release_needs_a_standing_grant() {
        let live = pane();
        let permit =
            mint_pane::<SecretRelease>(AdmittedScope::of(&live), "a".into(), Grants::none())
                .unwrap();
        assert!(permit.check_subject("a").is_ok());
        assert!(permit.check("a").is_err(), "no grant bound yet");
        let monitor = PermissionMonitor::ephemeral();
        let record = crate::broker::GrantRecord::event_stream_allow(
            crate::broker::ActorType::Agent,
            "pane:7",
            crate::broker::ActorScope::User,
            "probe::tick",
            std::path::Path::new("/tmp/ws"),
            crate::broker::GrantDuration::Always,
            crate::broker::GrantSource::Workspace,
            None,
        );
        let id = record.grant_id.clone();
        monitor.store().record(record);
        let bound = permit
            .bind(Grants::live(monitor.clone(), vec![id.clone()]))
            .unwrap();
        assert!(bound.check("a").is_ok());
        monitor.revoke_grant_id(&id).unwrap();
        assert!(bound.check("a").is_err(), "a revoked grant");
    }
}
