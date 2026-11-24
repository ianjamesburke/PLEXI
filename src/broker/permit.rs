//! Broker-minted permits: the proof a protected host effect requires
//! (stint 1716; `.stint/runs/design-1285/DESIGN.md` section 3 and
//! `.stint/runs/codex-3306/design.md`).
//!
//! The host classifies a transport into a pane incarnation or an unbound
//! principal. Only [`mint_pane`] and [`mint_unbound`] make a [`Permit`], and
//! each sink takes the permit for its own operation, so a pane number or
//! subscriber id alone reaches nothing. An operation only a pane may hold
//! does not implement [`UnboundOperation`], so minting it without a pane
//! does not compile.
//!
//! A permit holds the exact questions the broker approved (one per admitted
//! member: a secret, or one stream) and the profile's live permission
//! monitor. [`Permit::commit`] asks the member's question again at the
//! effect, so a revoke, expiry, narrowing or overriding deny stops the next
//! commit, while an equivalent or wider grant keeps it allowed. It also
//! re-checks the pane incarnation and scope revision.

use std::marker::PhantomData;
use std::sync::{Arc, MutexGuard};

use super::gate::{Approved, PermissionMonitor};
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
    /// The canonical target, and the one admitted member: one secret name in
    /// one scope.
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
    /// A principal with no pane incarnation (a plugin view, a run, an outside
    /// connection, a workspace agent, the Assistant).
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

/// The broker's approval behind a permit: each admitted member's question,
/// asked again in `monitor`'s live store at commit.
#[derive(Clone)]
pub(crate) struct Coverage {
    monitor: Arc<PermissionMonitor>,
    members: Vec<(String, Approved)>,
}

impl Coverage {
    /// `members` pairs each admitted member (a stream name, or a secret's
    /// target) with the question the broker allowed for it. Members are
    /// frozen: a member not listed is never committed.
    pub(crate) fn new(monitor: Arc<PermissionMonitor>, members: Vec<(String, Approved)>) -> Self {
        Self { monitor, members }
    }
}

/// Proof that `subject` was admitted for `Op` on one target.
pub(crate) struct Permit<Op> {
    subject: PermitSubject,
    target: String,
    coverage: Coverage,
    _op: PhantomData<fn() -> Op>,
}

impl<Op> Clone for Permit<Op> {
    fn clone(&self) -> Self {
        Self {
            subject: self.subject.clone(),
            target: self.target.clone(),
            coverage: self.coverage.clone(),
            _op: PhantomData,
        }
    }
}

impl<Op: Operation> std::fmt::Debug for Permit<Op> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let members: Vec<&str> = self
            .coverage
            .members
            .iter()
            .map(|(m, _)| m.as_str())
            .collect();
        write!(
            f,
            "Permit({} {} {} members={members:?})",
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
    coverage: Coverage,
) -> Result<Permit<Op>, String> {
    admitted.current()?;
    Ok(minted(PermitSubject::Pane(admitted), target, coverage))
}

/// Mint for a principal with no pane incarnation.
pub(crate) fn mint_unbound<Op: UnboundOperation>(
    class: &'static str,
    target: String,
    coverage: Coverage,
) -> Permit<Op> {
    minted(PermitSubject::Unbound { class }, target, coverage)
}

fn minted<Op: Operation>(subject: PermitSubject, target: String, coverage: Coverage) -> Permit<Op> {
    let permit = Permit {
        subject,
        target,
        coverage,
        _op: PhantomData,
    };
    log::info!("permit: minted {permit:?}");
    permit
}

impl<Op: Operation> Permit<Op> {
    /// Commit `member` of `target`: the broker must still allow the member's
    /// approved question, and a pane subject must still be unrevoked at its
    /// admitted revision. The returned guard holds off revoke and reset until
    /// it is dropped; hand the effect over, then drop it, and never hold it
    /// across a blocking wait.
    pub(crate) fn commit(&self, target: &str, member: &str) -> Result<MutexGuard<'_, ()>, String> {
        if target != self.target {
            return Err(format!(
                "{} permit is for {}, not {target}",
                Op::NAME,
                self.target
            ));
        }
        let Some((_, approved)) = self.coverage.members.iter().find(|(m, _)| m == member) else {
            return Err(format!("{} permit does not cover {member}", Op::NAME));
        };
        let guard = self.coverage.monitor.commit(approved)?;
        match &self.subject {
            PermitSubject::Pane(scope) => scope.current().map(|_| guard),
            PermitSubject::Unbound { .. } => Ok(guard),
        }
    }
}

/// A delivery permit for a fixture subscriber: an ephemeral monitor holding
/// a once allow for each of `events` (any actor may hold one), claimed by
/// the broker's admission.
#[cfg(test)]
pub(crate) fn delivery_fixture(
    subscriber_type: super::ActorType,
    subscriber_id: &str,
    plugin_id: &str,
    events: &[String],
) -> Permit<DeliverEvents> {
    let monitor = PermissionMonitor::ephemeral();
    let workspace = std::path::Path::new("/tmp/permit-fixture");
    let questions = events
        .iter()
        .map(|event| {
            let target = format!("{plugin_id}::{event}");
            monitor
                .store()
                .record(super::GrantRecord::event_stream_allow(
                    subscriber_type,
                    subscriber_id,
                    super::ActorScope::User,
                    &target,
                    workspace,
                    super::GrantDuration::Once,
                    super::GrantSource::Workspace,
                    None,
                ));
            super::PermissionRequest::exact(&super::ExactBinding::event_stream(
                subscriber_type,
                subscriber_id,
                super::ActorScope::User,
                &target,
                workspace,
                None,
            ))
        })
        .collect();
    let approved = monitor
        .admit_questions(questions, None)
        .expect("fixture rows allow");
    let members = events
        .iter()
        .cloned()
        .zip(approved.into_iter().map(|(approved, _)| approved))
        .collect();
    mint_unbound(
        "test",
        DeliverEvents::target(subscriber_type, subscriber_id, plugin_id),
        Coverage::new(monitor, members),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::pane_authority::{Ceiling, LivePaneAuthority, PaneAuthority};
    use crate::broker::ActorType;

    #[test]
    fn a_permit_commits_only_its_target_members_pane_and_live_grant() {
        let permit = delivery_fixture(ActorType::Agent, "pane:7", "probe", &["a".into()]);
        let target = DeliverEvents::target(ActorType::Agent, "pane:7", "probe");
        assert!(permit.commit(&target, "a").is_ok());
        assert!(permit.commit("other", "a").is_err(), "another target");
        assert!(
            permit.commit(&target, "b").is_err(),
            "a member never admitted"
        );

        let live = LivePaneAuthority::admitted(PaneAuthority::for_test(
            7,
            Ceiling::Baseline,
            1,
            std::path::PathBuf::from("/tmp/ws"),
        ));
        let pane = mint_pane::<DeliverEvents>(
            AdmittedScope::of(&live),
            target.clone(),
            permit.coverage.clone(),
        )
        .unwrap();
        live.publish_scope(std::path::PathBuf::from("/tmp/elsewhere"));
        assert!(pane.commit(&target, "a").is_err(), "a moved scope revision");

        let monitor = &permit.coverage.monitor;
        let id = monitor.store().records()[0].grant_id.clone();
        monitor.revoke_grant_id(&id).unwrap();
        assert!(permit.commit(&target, "a").is_err(), "a revoked grant");
    }
}
