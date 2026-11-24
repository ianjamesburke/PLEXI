//! Broker-minted permits: the proof a protected host effect requires
//! (stint 1716, `.stint/runs/design-1285/DESIGN.md` section 3).
//!
//! The host classifies a transport into a [`PermitSubject`]. Only this module
//! turns a subject into a permit, and a sink takes the permit for its own
//! operation, so a run id, pane number or `Host` value alone reaches nothing.
//! A permit holds the admitted incarnation and revision; [`RunPermit::check`]
//! re-validates them at the effect.
//!
//! Run ownership survives a restart as a MAC in the run's own journal line:
//! one write keeps owner and run together, and the journal, which a pane can
//! edit, cannot name a new owner. Runs from before owners were sealed are
//! listed once in sealed state, so removing a MAC does not make a run legacy.

use std::collections::BTreeSet;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use super::gate::PermissionMonitor;
use crate::host::plugin_timeline::AdmittedScope;

/// The owner recorded for a run host code started.
const HOST_OWNER: &str = "host";

/// Who is asking for a permit, as the host established it.
#[derive(Clone)]
pub(crate) enum PermitSubject {
    /// Host code: team setup and the desktop dashboard.
    Host,
    /// A socket or run-delegating principal.
    Principal {
        owner_id: String,
        live: Option<AdmittedScope>,
    },
    /// A caller that authenticated with this run's bearer token.
    RunBearer { run_id: String },
}

impl PermitSubject {
    /// A principal's durable owner id: its pane's owner key when it has a
    /// live pane, because a pane number can be reused after a crash.
    pub(crate) fn principal(
        actor_id: &str,
        authority: Option<&std::sync::Arc<super::pane_authority::LivePaneAuthority>>,
    ) -> Self {
        Self::Principal {
            owner_id: authority.map_or_else(
                || format!("principal:{actor_id}"),
                |live| format!("pane-key:{}", live.grant().owner_key),
            ),
            live: authority.map(AdmittedScope::of),
        }
    }

    fn owner_id(&self) -> &str {
        match self {
            Self::Principal { owner_id, .. } => owner_id,
            Self::Host | Self::RunBearer { .. } => HOST_OWNER,
        }
    }

    fn live(&self) -> Option<AdmittedScope> {
        match self {
            Self::Principal { live, .. } => live.clone(),
            Self::Host | Self::RunBearer { .. } => None,
        }
    }

    fn label(&self) -> &str {
        match self {
            Self::Host => HOST_OWNER,
            Self::Principal { owner_id, .. } => owner_id,
            Self::RunBearer { run_id } => run_id,
        }
    }
}

/// The facts a run's owner MAC covers. A line copied to another workspace,
/// run or admission no longer verifies.
#[derive(Clone, Copy)]
pub(crate) struct RunBinding<'a> {
    pub(crate) workspace: &'a Path,
    pub(crate) run_id: &'a str,
    pub(crate) actor_id: &'a str,
    pub(crate) admission_id: &'a str,
}

impl RunBinding<'_> {
    fn material(&self, owner_id: &str) -> Vec<u8> {
        serde_json::to_vec(&(
            "plexi-run-owner-v1",
            self.workspace,
            self.run_id,
            self.actor_id,
            self.admission_id,
            owner_id,
        ))
        .expect("a tuple of strings and a path serializes")
    }
}

/// Reissue a run's bearer.
pub(crate) struct Reissue;
/// Call a tool as a run.
pub(crate) struct Invoke;
/// Finish or cancel a run.
pub(crate) struct Finish;

/// One operation on one existing run, admitted for one subject.
pub(crate) struct RunPermit<Op> {
    workspace: PathBuf,
    run_id: String,
    live: Option<AdmittedScope>,
    _op: PhantomData<Op>,
}

impl<Op> RunPermit<Op> {
    pub(crate) fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub(crate) fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Re-validate at the effect: the admitted pane is still this
    /// incarnation, unrevoked, at the admitted revision.
    pub(crate) fn check(&self) -> Result<(), String> {
        match &self.live {
            Some(live) => live.current().map(|_| ()),
            None => Ok(()),
        }
    }
}

/// Permission to start one new run of one agent under one admission.
pub(crate) struct StartPermit {
    workspace: PathBuf,
    agent_id: String,
    admission_id: String,
    owner_id: String,
    live: Option<AdmittedScope>,
}

impl StartPermit {
    /// Whether this permit is for exactly this start, and its pane still
    /// stands as admitted.
    pub(crate) fn check(
        &self,
        workspace: &Path,
        agent_id: &str,
        admission_id: &str,
    ) -> Result<(), String> {
        if self.workspace != workspace
            || self.agent_id != agent_id
            || self.admission_id != admission_id
        {
            return Err(format!(
                "start permit is for agent {} admission {}",
                self.agent_id, self.admission_id
            ));
        }
        match &self.live {
            Some(live) => live.current().map(|_| ()),
            None => Ok(()),
        }
    }

    pub(crate) fn owner_id(&self) -> &str {
        &self.owner_id
    }
}

/// Mint a start permit. A principal must hold every tool the new run would
/// act with; host code brings its own admitted ceiling.
pub(crate) fn mint_start(
    subject: &PermitSubject,
    workspace: &Path,
    agent_id: &str,
    admission_id: &str,
    required: &BTreeSet<String>,
    held: &BTreeSet<String>,
) -> Result<StartPermit, String> {
    if let PermitSubject::RunBearer { run_id } = subject {
        return Err(format!("run {run_id}'s bearer cannot start a run"));
    }
    if matches!(subject, PermitSubject::Principal { .. }) {
        if let Some(missing) = required.difference(held).next() {
            log::info!(
                "permit: start refused subject={} agent={agent_id} missing={missing}",
                subject.label()
            );
            return Err(format!("the caller does not hold {missing}"));
        }
    }
    let live = subject.live();
    if let Some(live) = &live {
        live.current()?;
    }
    log::info!(
        "permit: start minted subject={} agent={agent_id} admission={admission_id}",
        subject.label()
    );
    Ok(StartPermit {
        workspace: workspace.to_path_buf(),
        agent_id: agent_id.to_string(),
        admission_id: admission_id.to_string(),
        owner_id: subject.owner_id().to_string(),
        live,
    })
}

/// Mint a permit for `Op` on an existing run. `owner_seal` is the MAC in the
/// run's journal line; `legacy` is the sealed list of runs that predate it.
pub(crate) fn mint_run<Op>(
    gate: &PermissionMonitor,
    subject: &PermitSubject,
    binding: RunBinding<'_>,
    owner_seal: Option<&str>,
    legacy: &BTreeSet<String>,
) -> Result<RunPermit<Op>, String> {
    let admitted = match subject {
        PermitSubject::Host => true,
        PermitSubject::RunBearer { run_id } => run_id == binding.run_id,
        PermitSubject::Principal { owner_id, .. } => match owner_seal {
            Some(seal) => gate.run_owner_matches(&binding.material(owner_id), seal)?,
            None => {
                let known = legacy.contains(binding.run_id);
                if known {
                    log::info!(
                        "permit: run={} predates sealed owners; admitted {owner_id}",
                        binding.run_id
                    );
                }
                known
            }
        },
    };
    if !admitted {
        log::info!(
            "permit: refused run={} subject={}",
            binding.run_id,
            subject.label()
        );
        return Err(format!("run {} was not started by this caller", binding.run_id));
    }
    let live = subject.live();
    if let Some(live) = &live {
        live.current()?;
    }
    Ok(RunPermit {
        workspace: binding.workspace.to_path_buf(),
        run_id: binding.run_id.to_string(),
        live,
        _op: PhantomData,
    })
}

/// The MAC that records `permit`'s subject as the owner of a new run.
pub(crate) fn seal_run_owner(
    gate: &PermissionMonitor,
    permit: &StartPermit,
    binding: RunBinding<'_>,
) -> Result<String, String> {
    gate.seal_run_owner(&binding.material(permit.owner_id()))
}

impl PermissionMonitor {
    fn seal_run_owner(&self, material: &[u8]) -> Result<String, String> {
        let dir = self.run_owner_dir()?;
        super::seal::keyed_mac(&self.custody()?, &dir, material)
    }

    fn run_owner_matches(&self, material: &[u8], seal: &str) -> Result<bool, String> {
        let dir = self.run_owner_dir()?;
        super::seal::keyed_mac_matches(&self.custody()?, &dir, material, seal)
    }

    fn run_owner_dir(&self) -> Result<PathBuf, String> {
        self.profile_dir()
            .map(Path::to_path_buf)
            .ok_or_else(|| "run ownership needs a profile-backed monitor".to_string())
    }

    /// The runs in `workspace` that predate sealed owners. The first call
    /// for a workspace records `unsealed()` once; later edits to the journal
    /// cannot add to it.
    pub(crate) fn legacy_runs(
        &self,
        workspace: &Path,
        unsealed: impl FnOnce() -> Result<BTreeSet<String>, String>,
    ) -> Result<BTreeSet<String>, String> {
        let dir = self.run_owner_dir()?;
        let host = self.custody()?;
        if let Some(known) = super::seal::legacy_runs(&host, &dir, workspace)? {
            return Ok(known);
        }
        let captured = unsealed()?;
        super::seal::record_legacy_runs(&host, &dir, workspace, &captured)?;
        log::info!(
            "permit: recorded {} pre-owner run(s) for {}",
            captured.len(),
            workspace.display()
        );
        Ok(captured)
    }
}
