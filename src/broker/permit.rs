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
//! under; [`Permit::check`] re-validates both, and its target, at the effect.

use std::marker::PhantomData;

use crate::host::plugin_timeline::AdmittedScope;

/// One protected effect a permit can be minted for.
pub(crate) trait Operation {
    const NAME: &'static str;
}

/// An operation a principal with no pane incarnation may also hold.
pub(crate) trait UnboundOperation: Operation {}

/// Write a secret value into a caller's reply.
pub(crate) enum SecretRelease {}

impl Operation for SecretRelease {
    const NAME: &'static str = "secret.release";
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

/// Proof that `subject` was admitted for `Op` on one target.
pub(crate) struct Permit<Op> {
    subject: PermitSubject,
    target: String,
    _op: PhantomData<fn() -> Op>,
}

impl<Op> Clone for Permit<Op> {
    fn clone(&self) -> Self {
        Self {
            subject: self.subject.clone(),
            target: self.target.clone(),
            _op: PhantomData,
        }
    }
}

impl<Op: Operation> std::fmt::Debug for Permit<Op> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Permit({} {} {})",
            Op::NAME,
            self.subject.label(),
            self.target
        )
    }
}

/// Mint for a pane, which must still be at the incarnation and revision it
/// was admitted under.
pub(crate) fn mint_pane<Op: Operation>(
    admitted: AdmittedScope,
    target: String,
) -> Result<Permit<Op>, String> {
    admitted.current()?;
    Ok(minted(PermitSubject::Pane(admitted), target))
}

/// Mint for a principal with no pane incarnation: host code, a plugin view,
/// a run or an outside connection, each bounded by its own transport.
pub(crate) fn mint_unbound<Op: UnboundOperation>(
    class: &'static str,
    target: String,
) -> Permit<Op> {
    minted(PermitSubject::Unbound { class }, target)
}

fn minted<Op: Operation>(subject: PermitSubject, target: String) -> Permit<Op> {
    log::info!(
        "permit: minted {} for {} on {target}",
        Op::NAME,
        subject.label()
    );
    Permit {
        subject,
        target,
        _op: PhantomData,
    }
}

impl<Op: Operation> Permit<Op> {
    /// Re-validate at the effect: the same target, and a pane subject still
    /// unrevoked at the admitted revision.
    pub(crate) fn check(&self, target: &str) -> Result<(), String> {
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
        let permit = mint_pane::<DeliverEvents>(AdmittedScope::of(&live), "a".into()).unwrap();
        assert!(permit.check("a").is_ok());
        assert!(permit.check("b").is_err(), "another target");
        live.publish_scope(std::path::PathBuf::from("/tmp/elsewhere"));
        assert!(permit.check("a").is_err(), "a moved scope revision");
        let moved = mint_pane::<SecretRelease>(AdmittedScope::of(&live), "a".into()).unwrap();
        live.revoke();
        assert!(moved.check("a").is_err(), "a revoked incarnation");
        assert!(mint_pane::<SecretRelease>(AdmittedScope::of(&live), "a".into()).is_err());
    }
}
