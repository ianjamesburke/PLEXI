//! Mandatory permission monitor for every tool dispatch.
//!
//! Constructors of `ToolDispatcher` take an `Arc<PermissionMonitor>`. Admission
//! compares the full exact binding (actor trust, workspace, resource, package,
//! instance, argument fingerprint, session, expiry). Schema-0 grants never
//! match. Revoke and commit share one admission lock.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(test)]
use std::sync::atomic::AtomicI64;
use std::sync::{Arc, Mutex, OnceLock};
use sha2::{Digest, Sha256};

use super::{
    ActorScope, ActorType, Decision, ExactBinding, GrantDuration, GrantRecord, GrantSource,
    GrantStore, PermissionRequest, ResourceScope, TargetType,
};

const SCHEMA: u32 = 1;
const ALWAYS_TTL_SECS: i64 = 30 * 24 * 60 * 60;

/// Process cache so Assistant, AgentHost, MCP, and `plexi app call` in one
/// profile share grants. Tests with distinct profile directories stay isolated.
fn monitors() -> &'static Mutex<std::collections::HashMap<PathBuf, Arc<PermissionMonitor>>> {
    static MONITORS: OnceLock<Mutex<std::collections::HashMap<PathBuf, Arc<PermissionMonitor>>>> =
        OnceLock::new();
    MONITORS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalChoice {
    Once,
    Session,
    Always,
    Deny,
}

/// One host record for everything waiting on the human.
///
/// Each-time and time-boxed sign-off live on the Touch ID spike. This gate
/// files click approvals, agent questions, and blocked runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NeedsYouKind {
    ApprovalClick,
    Question,
    BlockedRun,
    /// The host rejected a queue file. It is not a grant and not an approval.
    Integrity,
}

impl NeedsYouKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApprovalClick => "approval_click",
            Self::Question => "question",
            Self::BlockedRun => "blocked_run",
            Self::Integrity => "integrity",
        }
    }

    fn is_approval(self) -> bool {
        matches!(self, Self::ApprovalClick)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NeedsYouResolution {
    Approved,
    Denied,
}

impl NeedsYouResolution {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Denied => "denied",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct NeedsYouRecord {
    pub id: String,
    pub kind: NeedsYouKind,
    pub actor: String,
    pub resource: String,
    pub summary: String,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub run_tag: Option<String>,
    pub resolution: Option<NeedsYouResolution>,
}

/// A question or a blocked run filed by the host. Approvals are filed by the gate.
#[derive(Debug, Clone)]
pub struct NeedsYouFile {
    pub kind: NeedsYouKind,
    pub actor: String,
    pub resource: String,
    pub summary: String,
    pub expires_at: Option<i64>,
    pub run_tag: Option<String>,
}

/// What happened to the run that was waiting.
///
/// `unblocked` means this host process still owns that run, so the decision
/// applies to it. `outcome_unknown` means the run was filed by a host process
/// that is gone: the decision is recorded and the item is not dropped, and the
/// host does not claim the original run observed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Unblocked,
    OutcomeUnknown,
}

impl RunOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unblocked => "unblocked",
            Self::OutcomeUnknown => "outcome_unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsYouReceipt {
    pub id: String,
    pub resolution: NeedsYouResolution,
    pub already: bool,
    pub run_outcome: RunOutcome,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PendingView {
    pub pending_request_id: String,
    pub actor_id: String,
    pub tool: String,
    pub resource_id: Option<String>,
    pub args_fingerprint: String,
    pub input_summary: String,
}

#[derive(Clone)]
struct Pending {
    id: String,
    binding: ExactBinding,
    tool: String,
    input_summary: String,
}

#[derive(Debug, Clone)]
pub struct AuditFact {
    pub kind: String,
    pub actor: String,
    pub call_id: String,
    pub grant_id: String,
    pub resource_id: String,
    pub args_fingerprint: String,
    pub operation_id: String,
    pub decision: String,
    pub revision_before: String,
    pub revision_after: String,
}

pub struct PermissionMonitor {
    store: Arc<Mutex<GrantStore>>,
    pending: Mutex<Vec<Pending>>,
    /// Choices already applied, so a sheet and a dispatcher can both approve once.
    resolutions: Mutex<std::collections::HashMap<String, ApprovalChoice>>,
    session_id: String,
    credentials: Mutex<std::collections::HashMap<String, SessionCredential>>,
    admission: Mutex<()>,
    epoch: AtomicU64,
    audit_path: Option<PathBuf>,
    /// Profile whose `<profile>/host` queue this monitor persists. `None` for
    /// an ephemeral monitor, which never touches disk.
    profile_dir: Option<PathBuf>,
    audit_mem: Mutex<Vec<AuditFact>>,
    fail_audit: AtomicBool,
    /// Open and resolved items waiting on the human. One map, one resolution.
    needs_you: Mutex<BTreeMap<String, NeedsYouRecord>>,
    /// Host session that filed each item. A different session means the
    /// original run is gone.
    origins: Mutex<BTreeMap<String, String>>,
    /// Outcome recorded for a resolved id, so a second resolve repeats it.
    run_outcomes: Mutex<BTreeMap<String, RunOutcome>>,
    /// Serializes list, expiry, and resolve so one terminal receipt wins.
    needs_resolve: Mutex<()>,
    #[cfg(test)]
    now_override: AtomicI64,
}

#[derive(Clone)]
pub struct SessionCredential {
    pub token: String,
    pub pane_id: Option<u64>,
    pub context_id: u64,
    pub workspace_root: PathBuf,
    pub actor_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityError {
    Missing,
    Forged,
    Mismatch,
    StaleCredential,
}

#[derive(Debug, Clone)]
pub struct VerifiedCaller {
    pub actor_id: String,
    pub actor_type: ActorType,
    pub actor_scope: ActorScope,
    pub pane_id: Option<u64>,
    pub context_id: u64,
    pub workspace_root: PathBuf,
    /// Socket and credential callers are never the human at the keyboard.
    pub is_human: bool,
}

pub struct AdmitRequest<'a> {
    pub call_id: &'a str,
    pub tool: &'a str,
    pub input_json: &'a str,
    pub actor_type: ActorType,
    pub actor_id: &'a str,
    pub actor_scope: ActorScope,
    pub trust_origin: &'a str,
    pub workspace_root: &'a Path,
    pub context_id: u64,
    pub package_id: &'a str,
    pub instance_id: u64,
    pub target_type: TargetType,
}

pub enum Admission {
    Proceed {
        grant_id: String,
        fingerprint: String,
        resource_scope: ResourceScope,
        resource_id: Option<String>,
    },
    Required {
        pending_request_id: String,
    },
    Denied {
        code: &'static str,
    },
}

impl PermissionMonitor {
    pub fn for_profile(dir: &Path) -> Arc<Self> {
        let key = crate::platform::path::canonical_or_self(dir);
        let mut map = monitors().lock().unwrap_or_else(|e| e.into_inner());
        map.entry(key)
            .or_insert_with(|| Arc::new(Self::open(dir)))
            .clone()
    }

    /// Private store. Used by tests that must not share a profile monitor.
    pub fn ephemeral() -> Arc<Self> {
        Arc::new(Self::new(GrantStore::default(), None))
    }

    fn open(dir: &Path) -> Self {
        let store = GrantStore::load_or_default(dir);
        let audit = dir.join("permission-audit.jsonl");
        log::info!(
            "permission_monitor: opened profile {} audit {}",
            dir.display(),
            audit.display()
        );
        let monitor = Self::from_arc(Arc::new(Mutex::new(store)), Some(audit), Some(dir.to_path_buf()));
        monitor.restore_queue();
        monitor
    }

    fn new(store: GrantStore, audit_path: Option<PathBuf>) -> Self {
        Self::from_arc(Arc::new(Mutex::new(store)), audit_path, None)
    }

    fn from_arc(
        store: Arc<Mutex<GrantStore>>,
        audit_path: Option<PathBuf>,
        profile_dir: Option<PathBuf>,
    ) -> Self {
        let session_id = format!("sess-{}", uuid::Uuid::new_v4());
        log::info!("permission_monitor: session {session_id} started");
        Self {
            store,
            pending: Mutex::new(Vec::new()),
            resolutions: Mutex::new(std::collections::HashMap::new()),
            session_id,
            credentials: Mutex::new(std::collections::HashMap::new()),
            admission: Mutex::new(()),
            epoch: AtomicU64::new(1),
            audit_path,
            profile_dir,
            audit_mem: Mutex::new(Vec::new()),
            fail_audit: AtomicBool::new(false),
            needs_you: Mutex::new(BTreeMap::new()),
            origins: Mutex::new(BTreeMap::new()),
            run_outcomes: Mutex::new(BTreeMap::new()),
            needs_resolve: Mutex::new(()),
            #[cfg(test)]
            now_override: AtomicI64::new(0),
        }
    }

    /// True when a previous host left a queue file. A frame uses this so it
    /// does not open a profile store when nothing is waiting.
    pub fn has_persisted_queue(dir: &Path) -> bool {
        super::needs_you_store::queue_file_exists(dir)
    }

    /// Drop the process cache and open `dir` again. Tests use this as a restart.
    #[cfg(test)]
    pub fn reload_for_test(dir: &Path) -> Arc<Self> {
        let key = crate::platform::path::canonical_or_self(dir);
        monitors()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&key);
        Self::for_profile(dir)
    }

    /// The process-cached monitor for `dir`, if something in this process has
    /// already opened it. Painting and expiry use this so a frame never creates
    /// a profile store just to discover that nothing is waiting.
    pub fn loaded(dir: &Path) -> Option<Arc<Self>> {
        let key = crate::platform::path::canonical_or_self(dir);
        monitors()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .cloned()
    }

    pub fn store(&self) -> std::sync::MutexGuard<'_, GrantStore> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn replace_store(&self, store: GrantStore) {
        *self.store() = store;
        log::info!("permission_monitor: grant store replaced");
    }

    #[cfg(test)]
    pub fn fail_audit(&self, fail: bool) {
        self.fail_audit.store(fail, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub fn hold_admission(&self) -> std::sync::MutexGuard<'_, ()> {
        self.admission.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn admit(&self, req: AdmitRequest<'_>) -> Admission {
        let Ok(fingerprint) = fingerprint_args(req.input_json) else {
            log::info!(
                "permission_monitor: deny invalid arguments tool={} call_id={}",
                req.tool,
                req.call_id
            );
            return Admission::Denied {
                code: "invalid_argument",
            };
        };
        if req.workspace_root.as_os_str().is_empty() {
            return Admission::Denied {
                code: "permission_denied",
            };
        }
        let (resource_scope, resource_id) = resource_of(req.tool, req.input_json);
        let binding = ExactBinding {
            actor_type: req.actor_type,
            actor_id: req.actor_id.to_string(),
            actor_scope: req.actor_scope,
            trust_origin: req.trust_origin.to_string(),
            workspace_root: req.workspace_root.to_path_buf(),
            target_type: req.target_type,
            target_id: req.tool.to_string(),
            resource_scope,
            resource_id,
            args_fingerprint: fingerprint,
            session_id: Some(self.session_id.clone()),
            package_id: req.package_id.to_string(),
            instance_id: Some(req.instance_id),
            context_id: Some(req.context_id),
            call_id: req.call_id.to_string(),
            operation_id: operation_id_of(req.input_json),
        };
        let request = PermissionRequest::exact(&binding);
        let decision = self.store().evaluate(&request, None);
        log::info!(
            "permission_monitor: admit actor={} tool={} resource={:?} call_id={} -> {}",
            binding.actor_id,
            binding.target_id,
            binding.resource_id,
            binding.call_id,
            decision.as_str()
        );
        match decision {
            Decision::Deny => {
                self.note_denial(
                    &binding.actor_id,
                    &binding.call_id,
                    binding.resource_id.as_deref().unwrap_or(""),
                    &binding.operation_id,
                    "deny",
                );
                Admission::Denied {
                    code: "permission_denied",
                }
            }
            Decision::Allow => {
                let grant_id = self
                    .store()
                    .records()
                    .iter()
                    .find(|record| record.matches(&request, crate::platform::clock::now_secs() as i64))
                    .map(|record| record.grant_id.clone())
                    .unwrap_or_default();
                Admission::Proceed {
                    grant_id,
                    fingerprint: binding.args_fingerprint.clone(),
                    resource_scope: binding.resource_scope,
                    resource_id: binding.resource_id.clone(),
                }
            }
            Decision::Ask => {
                let id = self.persist_pending(&binding, req.tool, req.input_json);
                let _ = self.audit(&AuditFact {
                    kind: "ask".to_string(),
                    actor: binding.actor_id.clone(),
                    call_id: binding.call_id.clone(),
                    grant_id: String::new(),
                    resource_id: binding.resource_id.clone().unwrap_or_default(),
                    args_fingerprint: binding.args_fingerprint.clone(),
                    operation_id: operation_id_of(req.input_json),
                    decision: "ask".to_string(),
                    revision_before: String::new(),
                    revision_after: String::new(),
                });
                Admission::Required {
                    pending_request_id: id,
                }
            }
        }
    }

    fn persist_pending(&self, binding: &ExactBinding, tool: &str, input_json: &str) -> String {
        let id = {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(existing) = pending.iter().find(|row| same_pending(row, binding)).cloned() {
                log::info!(
                    "permission_monitor: reuse pending {} for actor={} tool={}",
                    existing.id,
                    binding.actor_id,
                    tool
                );
                let id = existing.id.clone();
                drop(pending);
                self.upsert_approval_needs_you(&existing);
                id
            } else {
                let id = format!("req_{}", uuid::Uuid::new_v4());
                log::info!(
                    "permission_monitor: pending {id} actor={} tool={} resource={:?}",
                    binding.actor_id,
                    tool,
                    binding.resource_id
                );
                let row = Pending {
                    id: id.clone(),
                    binding: binding.clone(),
                    tool: tool.to_string(),
                    input_summary: summarize(input_json),
                };
                self.upsert_approval_needs_you(&row);
                pending.push(row);
                id
            }
        };
        self.persist_queue();
        id
    }

    pub fn approve_pending(&self, pending_id: &str, choice: ApprovalChoice) -> Result<(), String> {
        if let Some(previous) = self
            .resolutions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(pending_id)
            .copied()
        {
            return if previous == choice {
                let _ = self.close_needs_you(
                    pending_id,
                    resolution_for_choice(choice),
                    decision_for_choice(choice),
                    "",
                );
                Ok(())
            } else {
                Err(format!("pending {pending_id} already resolved"))
            };
        }
        let pending = {
            let rows = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            rows.iter().find(|row| row.id == pending_id).cloned()
        };
        let Some(pending) = pending else {
            return Err(format!("unknown pending request {pending_id}"));
        };
        if choice == ApprovalChoice::Deny {
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|row| row.id != pending_id);
            self.resolutions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(pending_id.to_string(), choice);
            let _ = self.audit(&AuditFact {
                kind: "deny".to_string(),
                actor: pending.binding.actor_id.clone(),
                call_id: pending.binding.call_id.clone(),
                grant_id: String::new(),
                resource_id: pending.binding.resource_id.clone().unwrap_or_default(),
                args_fingerprint: pending.binding.args_fingerprint.clone(),
                operation_id: String::new(),
                decision: "deny".to_string(),
                revision_before: String::new(),
                revision_after: String::new(),
            });
            log::info!("permission_monitor: denied pending {pending_id}");
            let _ = self.close_needs_you(pending_id, NeedsYouResolution::Denied, "denied", "");
            return Ok(());
        }
        let (duration, session_id, expires_at) = match choice {
            ApprovalChoice::Once => (GrantDuration::Once, None, None),
            ApprovalChoice::Session => (
                GrantDuration::Session,
                Some(self.session_id().to_string()),
                None,
            ),
            ApprovalChoice::Always => (
                GrantDuration::Always,
                None,
                Some(crate::platform::clock::now_secs() as i64 + ALWAYS_TTL_SECS),
            ),
            ApprovalChoice::Deny => unreachable!("deny returned above"),
        };
        let mut binding = pending.binding.clone();
        binding.session_id = session_id.clone();
        let grant_id = format!("grant_{}", uuid::Uuid::new_v4());
        let mut record = GrantRecord::from_binding(
            &binding,
            Decision::Allow,
            duration,
            match choice {
                ApprovalChoice::Always => GrantSource::User,
                _ => GrantSource::Session,
            },
            &grant_id,
        );
        record.session_id = session_id;
        record.expires_at = expires_at;
        self.store().record(record);
        if choice == ApprovalChoice::Always {
            self.store().save();
        }
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|row| row.id != pending_id);
        self.resolutions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(pending_id.to_string(), choice);
        self.audit(&AuditFact {
            kind: "grant".to_string(),
            actor: binding.actor_id.clone(),
            call_id: binding.call_id,
            grant_id: grant_id.clone(),
            resource_id: binding.resource_id.unwrap_or_default(),
            args_fingerprint: binding.args_fingerprint,
            operation_id: String::new(),
            decision: match choice {
                ApprovalChoice::Once => "allow_once",
                ApprovalChoice::Session => "allow_session",
                ApprovalChoice::Always => "allow_until",
                ApprovalChoice::Deny => "deny",
            }
            .to_string(),
            revision_before: String::new(),
            revision_after: String::new(),
        })?;
        log::info!("permission_monitor: approved pending {pending_id} as {grant_id}");
        let _ = self.close_needs_you(
            pending_id,
            NeedsYouResolution::Approved,
            "approved",
            &grant_id,
        );
        Ok(())
    }

    /// File an agent question or a blocked run. Gate approvals are filed by admission.
    pub fn file_needs_you(&self, filed: NeedsYouFile) -> Result<String, String> {
        if filed.kind.is_approval() || matches!(filed.kind, NeedsYouKind::Integrity) {
            return Err("approval items are filed by the permission gate".to_string());
        }
        if filed.actor.is_empty() || filed.summary.is_empty() {
            return Err("needs-you actor and summary are required".to_string());
        }
        let id = {
            let mut map = self.needs_you.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(tag) = filed.run_tag.as_deref() {
                if let Some(existing) = map.values().find(|row| {
                    row.resolution.is_none()
                        && row.kind == filed.kind
                        && row.run_tag.as_deref() == Some(tag)
                }) {
                    log::info!("needs_you: reuse {} kind={}", existing.id, existing.kind.as_str());
                    return Ok(existing.id.clone());
                }
            }
            let id = format!("ny_{}", uuid::Uuid::new_v4());
            let record = NeedsYouRecord {
                id: id.clone(),
                kind: filed.kind,
                actor: filed.actor,
                resource: filed.resource,
                summary: filed.summary,
                created_at: self.now_secs(),
                expires_at: filed.expires_at,
                run_tag: filed.run_tag,
                resolution: None,
            };
            log::info!(
                "needs_you: filed {} kind={} actor={} resource={}",
                record.id,
                record.kind.as_str(),
                record.actor,
                record.resource
            );
            map.insert(id.clone(), record);
            drop(map);
            self.remember_origin(&id);
            id
        };
        self.persist_queue();
        Ok(id)
    }

    /// Drop expired open items, denying each one once.
    pub fn expire_needs_you(&self) {
        let _guard = self.needs_resolve.lock().unwrap_or_else(|e| e.into_inner());
        self.expire_needs_you_locked();
    }

    fn expire_needs_you_locked(&self) {
        let now = self.now_secs();
        let due: Vec<NeedsYouRecord> = self
            .needs_you
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|row| {
                row.resolution.is_none() && row.expires_at.is_some_and(|expires| now >= expires)
            })
            .cloned()
            .collect();
        for row in due {
            if !self.close_needs_you(&row.id, NeedsYouResolution::Denied, "auto_denied", "") {
                continue;
            }
            log::info!(
                "needs_you: auto-denied {} kind={}",
                row.id,
                row.kind.as_str()
            );
            if row.kind.is_approval() {
                if let Err(error) = self.approve_pending(&row.id, ApprovalChoice::Deny) {
                    log::info!("needs_you: auto-deny pending {} failed: {error}", row.id);
                }
            }
        }
    }

    /// Open items, after expired ones have been auto-denied.
    pub fn list_needs_you(&self) -> Vec<NeedsYouRecord> {
        self.expire_needs_you();
        let rows = self.open_needs_you();
        log::info!("needs_you: list open count={}", rows.len());
        rows
    }

    /// Open items without expiring. The frame loop expires; the badge only reads.
    pub fn open_needs_you(&self) -> Vec<NeedsYouRecord> {
        let mut rows: Vec<NeedsYouRecord> = self
            .needs_you
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|row| row.resolution.is_none())
            .cloned()
            .collect();
        rows.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        rows
    }

    /// Resolve one item from any surface. A second call returns the first receipt.
    pub fn resolve_needs_you(&self, id: &str, approve: bool) -> Result<NeedsYouReceipt, String> {
        let _guard = self.needs_resolve.lock().unwrap_or_else(|e| e.into_inner());
        self.expire_needs_you_locked();
        let Some(row) = self.needs_you_of(id) else {
            return Err(format!("unknown needs-you item {id}"));
        };
        if let Some(resolution) = row.resolution {
            log::info!("needs_you: resolve {id} already {}", resolution.as_str());
            return Ok(NeedsYouReceipt {
                id: id.to_string(),
                resolution,
                already: true,
                run_outcome: self.stored_outcome(id),
            });
        }
        let outcome = self.run_outcome_for(id);
        match row.kind {
            NeedsYouKind::ApprovalClick => {
                let choice = if approve {
                    ApprovalChoice::Once
                } else {
                    ApprovalChoice::Deny
                };
                if let Err(error) = self.approve_pending(id, choice) {
                    if let Some(existing) = self.resolved_receipt(id, true) {
                        return Ok(existing);
                    }
                    return Err(error);
                }
            }
            NeedsYouKind::Question | NeedsYouKind::BlockedRun | NeedsYouKind::Integrity => {
                let resolution = if approve {
                    NeedsYouResolution::Approved
                } else {
                    NeedsYouResolution::Denied
                };
                let decision = if approve { "approved" } else { "denied" };
                if !self.close_needs_you(id, resolution, decision, "") {
                    if let Some(existing) = self.resolved_receipt(id, true) {
                        return Ok(existing);
                    }
                    return Err(format!("needs-you item {id} could not be audited"));
                }
            }
        }
        self.run_outcomes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id.to_string(), outcome);
        log::info!("needs_you: resolve {id} run_outcome={}", outcome.as_str());
        self.resolved_receipt(id, false)
            .ok_or_else(|| format!("needs-you item {id} was not resolved"))
    }

    fn resolved_receipt(&self, id: &str, already: bool) -> Option<NeedsYouReceipt> {
        self.needs_you_of(id)
            .and_then(|row| row.resolution)
            .map(|resolution| NeedsYouReceipt {
                id: id.to_string(),
                resolution,
                already,
                run_outcome: self.stored_outcome(id),
            })
    }

    fn stored_outcome(&self, id: &str) -> RunOutcome {
        self.run_outcomes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(id)
            .copied()
            .unwrap_or(RunOutcome::OutcomeUnknown)
    }

    fn run_outcome_for(&self, id: &str) -> RunOutcome {
        if self.origin_is_current(id) {
            RunOutcome::Unblocked
        } else {
            RunOutcome::OutcomeUnknown
        }
    }

    fn origin_is_current(&self, id: &str) -> bool {
        self.origins
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(id)
            .is_some_and(|origin| origin == &self.session_id)
    }

    fn remember_origin(&self, id: &str) {
        self.origins
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(id.to_string())
            .or_insert_with(|| self.session_id.clone());
    }

    fn needs_you_of(&self, id: &str) -> Option<NeedsYouRecord> {
        self.needs_you
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    fn upsert_approval_needs_you(&self, pending: &Pending) {
        let mut map = self.needs_you.lock().unwrap_or_else(|e| e.into_inner());
        if map.contains_key(&pending.id) {
            drop(map);
            self.remember_origin(&pending.id);
            return;
        }
        let record = NeedsYouRecord {
            id: pending.id.clone(),
            kind: NeedsYouKind::ApprovalClick,
            actor: pending.binding.actor_id.clone(),
            resource: pending
                .binding
                .resource_id
                .clone()
                .unwrap_or_else(|| pending.tool.clone()),
            summary: pending.input_summary.clone(),
            created_at: self.now_secs(),
            expires_at: None,
            run_tag: Some(pending.binding.call_id.clone()),
            resolution: None,
        };
        log::info!(
            "needs_you: filed {} kind={} actor={} resource={}",
            record.id,
            record.kind.as_str(),
            record.actor,
            record.resource
        );
        map.insert(record.id.clone(), record);
        drop(map);
        self.remember_origin(&pending.id);
    }

    /// Mark an open item resolved and audit it. Returns false when it was already resolved
    /// or the audit write failed (the item stays open in that case).
    fn close_needs_you(
        &self,
        id: &str,
        resolution: NeedsYouResolution,
        decision: &str,
        grant_id: &str,
    ) -> bool {
        let snapshot = {
            let mut map = self.needs_you.lock().unwrap_or_else(|e| e.into_inner());
            let Some(row) = map.get_mut(id) else {
                return false;
            };
            if row.resolution.is_some() {
                return false;
            }
            row.resolution = Some(resolution);
            row.clone()
        };
        let audited_decision = if self.origin_is_current(id) {
            decision.to_string()
        } else {
            match decision {
                "approved" | "denied" => "outcome_unknown".to_string(),
                other => other.to_string(),
            }
        };
        let fact = AuditFact {
            kind: "needs_you".to_string(),
            actor: snapshot.actor.clone(),
            call_id: snapshot.run_tag.clone().unwrap_or_else(|| snapshot.id.clone()),
            grant_id: grant_id.to_string(),
            resource_id: snapshot.resource.clone(),
            args_fingerprint: String::new(),
            operation_id: snapshot.id.clone(),
            decision: audited_decision,
            revision_before: String::new(),
            revision_after: String::new(),
        };
        if let Err(error) = self.audit(&fact) {
            log::error!("needs_you: audit failed for {id}: {error}");
            if let Some(row) = self
                .needs_you
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_mut(id)
            {
                row.resolution = None;
            }
            return false;
        }
        log::info!("needs_you: resolved {id} {decision}");
        self.persist_queue();
        true
    }

    fn restore_queue(&self) {
        let Some(profile) = self.profile_dir.clone() else {
            return;
        };
        match super::needs_you_store::load(&profile) {
            super::needs_you_store::LoadedQueue::Empty => {
                log::info!("needs_you: no persisted queue in {}", profile.display());
            }
            super::needs_you_store::LoadedQueue::Items(items) => {
                let count = items.len();
                self.install_restored(items);
                log::info!("needs_you: restored {count} open items from {}", profile.display());
            }
            super::needs_you_store::LoadedQueue::Untrusted(reason) => {
                log::error!("needs_you: rejected queue in {}: {reason}", profile.display());
                if super::needs_you_store::quarantine(&profile).is_some() {
                    self.file_integrity(&reason);
                } else if super::needs_you_store::queue_file_exists(&profile) {
                    log::error!(
                        "needs_you: left untrusted queue in place at {}",
                        profile.display()
                    );
                    self.file_integrity_memory_only(&reason);
                } else {
                    self.file_integrity(&reason);
                }
            }
        }
    }

    fn install_restored(&self, items: Vec<super::needs_you_store::RestoredItem>) {
        let mut pending = self.pending.lock().unwrap_or_else(|error| error.into_inner());
        let mut needs = self.needs_you.lock().unwrap_or_else(|error| error.into_inner());
        let mut origins = self.origins.lock().unwrap_or_else(|error| error.into_inner());
        for item in items {
            if let (NeedsYouKind::ApprovalClick, Some(row)) = (item.kind, item.pending.clone()) {
                pending.push(Pending {
                    id: item.id.clone(),
                    binding: row.binding,
                    tool: row.tool,
                    input_summary: row.input_summary,
                });
            }
            origins.insert(item.id.clone(), item.origin_session.clone());
            needs.insert(
                item.id.clone(),
                NeedsYouRecord {
                    id: item.id,
                    kind: item.kind,
                    actor: item.actor,
                    resource: item.resource,
                    summary: item.summary,
                    created_at: item.created_at,
                    expires_at: item.expires_at,
                    run_tag: item.run_tag,
                    resolution: None,
                },
            );
        }
    }

    fn file_integrity(&self, reason: &str) {
        self.file_integrity_memory_only(reason);
        self.persist_queue();
    }

    fn file_integrity_memory_only(&self, reason: &str) {
        let id = format!("ny_{}", uuid::Uuid::new_v4());
        let summary = format!("Needs you queue failed its integrity check ({reason}). Stored entries in that file were not loaded.");
        log::info!("needs_you: filed {id} kind=integrity");
        self.needs_you
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(
                id.clone(),
                NeedsYouRecord {
                    id: id.clone(),
                    kind: NeedsYouKind::Integrity,
                    actor: "host".to_string(),
                    resource: "needs-you".to_string(),
                    summary,
                    created_at: self.now_secs(),
                    expires_at: None,
                    run_tag: Some(format!("integrity:{id}")),
                    resolution: None,
                },
            );
        self.remember_origin(&id);
    }

    fn persist_queue(&self) {
        let Some(profile) = self.profile_dir.clone() else {
            return;
        };
        let pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let needs = self
            .needs_you
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let origins = self
            .origins
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let mut items = Vec::new();
        for record in needs.values() {
            if record.resolution.is_some() {
                continue;
            }
            let origin = origins
                .get(&record.id)
                .cloned()
                .unwrap_or_else(|| self.session_id.clone());
            let restored_pending = if record.kind == NeedsYouKind::ApprovalClick {
                let Some(row) = pending.iter().find(|row| row.id == record.id) else {
                    log::error!(
                        "needs_you: skipped persisting approval {} with no pending request",
                        record.id
                    );
                    continue;
                };
                Some(super::needs_you_store::RestoredPending {
                    tool: row.tool.clone(),
                    input_summary: row.input_summary.clone(),
                    binding: row.binding.clone(),
                })
            } else {
                None
            };
            items.push(super::needs_you_store::RestoredItem {
                id: record.id.clone(),
                kind: record.kind,
                actor: record.actor.clone(),
                resource: record.resource.clone(),
                summary: record.summary.clone(),
                created_at: record.created_at,
                expires_at: record.expires_at,
                run_tag: record.run_tag.clone(),
                origin_session: origin,
                pending: restored_pending,
            });
        }
        if let Err(error) = super::needs_you_store::save(&profile, &items) {
            log::error!("needs_you: persist failed for {}: {error}", profile.display());
        }
    }

    fn now_secs(&self) -> i64 {
        #[cfg(test)]
        {
            let over = self.now_override.load(Ordering::SeqCst);
            if over > 0 {
                return over;
            }
        }
        crate::platform::clock::now_secs() as i64
    }

    #[cfg(test)]
    pub fn set_now_for_test(&self, unix_secs: i64) {
        self.now_override.store(unix_secs, Ordering::SeqCst);
    }

    /// Record use before execution. Failure denies the call. Revoke that wins
    /// the admission lock first makes this return `Err`.
    pub fn note_use(&self, actor: &str, call_id: &str, grant_id: &str, fingerprint: &str, resource: &str, operation_id: &str) -> Result<(), String> {
        let _admission = self.admission.lock().unwrap_or_else(|e| e.into_inner());
        let epoch = self.epoch.load(Ordering::SeqCst);
        if epoch == 0 {
            return Err("permission monitor epoch is unset".to_string());
        }
        let alive = self.store().records().iter().any(|record| {
            record.grant_id == grant_id && record.decision == Decision::Allow
        });
        if grant_id.is_empty() || !alive {
            return Err("grant revoked".to_string());
        }
        self.audit_locked(&AuditFact {
            kind: "use".to_string(),
            actor: actor.to_string(),
            call_id: call_id.to_string(),
            grant_id: grant_id.to_string(),
            resource_id: resource.to_string(),
            args_fingerprint: fingerprint.to_string(),
            operation_id: operation_id.to_string(),
            decision: "use".to_string(),
            revision_before: String::new(),
            revision_after: String::new(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn note_outcome(
        &self,
        actor: &str,
        call_id: &str,
        grant_id: &str,
        fingerprint: &str,
        resource: &str,
        operation_id: &str,
        decision: &str,
        revision_before: &str,
        revision_after: &str,
    ) -> Result<(), String> {
        self.audit(&AuditFact {
            kind: "outcome".to_string(),
            actor: actor.to_string(),
            call_id: call_id.to_string(),
            grant_id: grant_id.to_string(),
            resource_id: resource.to_string(),
            args_fingerprint: fingerprint.to_string(),
            operation_id: operation_id.to_string(),
            decision: decision.to_string(),
            revision_before: revision_before.to_string(),
            revision_after: revision_after.to_string(),
        })
    }

    pub fn consume_once(&self, grant_id: &str, operation_id: Option<&str>) {
        let _admission = self.admission.lock().unwrap_or_else(|e| e.into_inner());
        let mut store = self.store();
        if let Some(record) = store
            .records_mut()
            .iter_mut()
            .find(|record| record.grant_id == grant_id && record.duration == GrantDuration::Once)
        {
            record.consumed = true;
            record.bound_operation_id = operation_id.map(str::to_string);
            log::info!("permission_monitor: consumed once-grant {grant_id}");
        }
    }

    /// Revoke under the admission lock so a commit that has not entered yet
    /// observes the removal.
    pub fn revoke_grant_id(&self, grant_id: &str) -> bool {
        let _admission = self.admission.lock().unwrap_or_else(|e| e.into_inner());
        self.revoke_locked(grant_id)
    }

    #[cfg(test)]
    pub fn revoke_while_held(&self, _hold: &std::sync::MutexGuard<'_, ()>, grant_id: &str) -> bool {
        self.revoke_locked(grant_id)
    }

    fn revoke_locked(&self, grant_id: &str) -> bool {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        let mut store = self.store();
        let before = store.records().len();
        let actor = store
            .records()
            .iter()
            .find(|record| record.grant_id == grant_id)
            .map(|record| record.actor_id.clone())
            .unwrap_or_default();
        store.records_mut().retain(|record| record.grant_id != grant_id);
        let removed = store.records().len() != before;
        drop(store);
        if removed {
            let _ = self.audit_locked(&AuditFact {
                kind: "revoke".to_string(),
                actor,
                call_id: String::new(),
                grant_id: grant_id.to_string(),
                resource_id: String::new(),
                args_fingerprint: String::new(),
                operation_id: String::new(),
                decision: "revoked".to_string(),
                revision_before: String::new(),
                revision_after: String::new(),
            });
            log::info!("permission_monitor: revoked {grant_id}");
        }
        removed
    }

    pub fn list_pending(&self) -> Vec<PendingView> {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(pending_view)
            .collect()
    }

    pub fn show_pending(&self, id: &str) -> Option<PendingView> {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|row| row.id == id)
            .map(pending_view)
    }

    pub fn issue_credential(
        &self,
        pane_id: Option<u64>,
        context_id: u64,
        workspace_root: &Path,
        actor_id: &str,
    ) -> String {
        let token = format!("cred_{}", uuid::Uuid::new_v4());
        self.credentials
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                token.clone(),
                SessionCredential {
                    token: token.clone(),
                    pane_id,
                    context_id,
                    workspace_root: crate::platform::path::canonical_or_self(workspace_root),
                    actor_id: actor_id.to_string(),
                },
            );
        log::info!(
            "permission_monitor: issued call credential for actor={actor_id} pane={pane_id:?} context={context_id}"
        );
        token
    }

    /// Socket identity. A missing, forged, or mismatched claim is denied.
    /// No pane is never the human `user`.
    pub fn authenticate_call(
        &self,
        claimed_pane: Option<u64>,
        credential: Option<&str>,
        peer_pane: Option<u64>,
    ) -> Result<VerifiedCaller, IdentityError> {
        if let Some(token) = credential {
            let creds = self.credentials.lock().unwrap_or_else(|e| e.into_inner());
            let Some(cred) = creds.get(token) else {
                log::info!("permission_monitor: stale or unknown call credential");
                return Err(IdentityError::StaleCredential);
            };
            if let (Some(cred_pane), Some(claim)) = (cred.pane_id, claimed_pane) {
                if cred_pane != claim {
                    log::info!(
                        "permission_monitor: credential pane {cred_pane} != claim {claim}"
                    );
                    return Err(IdentityError::Mismatch);
                }
            }
            if let (Some(peer), Some(claim)) = (peer_pane, claimed_pane.or(cred.pane_id)) {
                if peer != claim {
                    log::info!("permission_monitor: peer pane {peer} != claim {claim}");
                    return Err(IdentityError::Mismatch);
                }
            }
            if cred.token != token || (cred.pane_id.is_none() && claimed_pane.is_some()) {
                return Err(IdentityError::Mismatch);
            }
            log::info!(
                "permission_monitor: authenticated credential actor={} pane={:?} context={}",
                cred.actor_id,
                cred.pane_id,
                cred.context_id
            );
            return Ok(VerifiedCaller {
                actor_id: cred.actor_id.clone(),
                actor_type: ActorType::Agent,
                actor_scope: ActorScope::User,
                pane_id: cred.pane_id,
                context_id: cred.context_id,
                workspace_root: cred.workspace_root.clone(),
                is_human: false,
            });
        }
        let Some(peer) = peer_pane else {
            log::info!("permission_monitor: call missing peer identity and credential");
            return Err(if claimed_pane.is_some() {
                IdentityError::Forged
            } else {
                IdentityError::Missing
            });
        };
        if let Some(claim) = claimed_pane {
            if claim != peer {
                log::info!("permission_monitor: forged caller pane {claim} (peer {peer})");
                return Err(IdentityError::Forged);
            }
        }
        Ok(VerifiedCaller {
            actor_id: format!("pane:{peer}"),
            actor_type: ActorType::Agent,
            actor_scope: ActorScope::User,
            pane_id: Some(peer),
            context_id: 0,
            workspace_root: PathBuf::new(),
            is_human: false,
        })
    }

    /// A refusal that never starts the tool. The audit row and the info line
    /// are the same fact: callers can read the row, and the host log shows it.
    pub fn note_denial(
        &self,
        actor: &str,
        call_id: &str,
        resource: &str,
        operation_id: &str,
        decision: &str,
    ) {
        let _ = self.audit(&AuditFact {
            kind: "deny".to_string(),
            actor: actor.to_string(),
            call_id: call_id.to_string(),
            grant_id: String::new(),
            resource_id: resource.to_string(),
            args_fingerprint: String::new(),
            operation_id: operation_id.to_string(),
            decision: decision.to_string(),
            revision_before: String::new(),
            revision_after: String::new(),
        });
        trace_gate(format!(
            "permission_monitor: deny actor={actor} call_id={call_id} decision={decision} resource={resource} op={operation_id}"
        ));
    }

    pub fn audit_records(&self) -> Vec<AuditFact> {
        self.audit_mem.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// A socket or CLI resolve of a click approval. The pending stays, no
    /// grant is written, and the audit records the refusal.
    pub fn refuse_client_resolve(&self, pending_id: &str) {
        let actor = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|row| row.id == pending_id)
            .map(|row| row.binding.actor_id.clone())
            .unwrap_or_else(|| "socket".to_string());
        let _ = self.audit(&AuditFact {
            kind: "refuse".to_string(),
            actor,
            call_id: pending_id.to_string(),
            grant_id: String::new(),
            resource_id: String::new(),
            args_fingerprint: String::new(),
            operation_id: String::new(),
            decision: "refused_resolve".to_string(),
            revision_before: String::new(),
            revision_after: String::new(),
        });
        log::info!("permission_monitor: refused resolve pending={pending_id}");
    }

    fn audit(&self, fact: &AuditFact) -> Result<(), String> {
        let _admission = self.admission.lock().unwrap_or_else(|e| e.into_inner());
        self.audit_locked(fact)
    }

    fn audit_locked(&self, fact: &AuditFact) -> Result<(), String> {
        if self.fail_audit.load(Ordering::SeqCst) {
            log::error!("permission_monitor: audit write failed (injected) kind={}", fact.kind);
            return Err("audit write failed".to_string());
        }
        self.audit_mem
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(fact.clone());
        if let Some(path) = &self.audit_path {
            if let Some(parent) = path.parent() {
                if let Err(error) = std::fs::create_dir_all(parent) {
                    log::error!("permission_monitor: audit dir {}: {error}", parent.display());
                    return Err(error.to_string());
                }
            }
            let line = serde_json::to_string(fact).map_err(|error| error.to_string())?;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|error| {
                    log::error!("permission_monitor: audit open {}: {error}", path.display());
                    error.to_string()
                })?;
            use std::io::Write;
            writeln!(file, "{line}").map_err(|error| {
                log::error!("permission_monitor: audit write {}: {error}", path.display());
                error.to_string()
            })?;
        }
        trace_gate(format!(
            "permission_monitor: audit {} actor={} call_id={} grant_id={} resource={} op={}",
            fact.kind,
            fact.actor,
            fact.call_id,
            fact.grant_id,
            fact.resource_id,
            fact.operation_id
        ));
        Ok(())
    }
}

fn resolution_for_choice(choice: ApprovalChoice) -> NeedsYouResolution {
    match choice {
        ApprovalChoice::Deny => NeedsYouResolution::Denied,
        ApprovalChoice::Once | ApprovalChoice::Session | ApprovalChoice::Always => {
            NeedsYouResolution::Approved
        }
    }
}

fn decision_for_choice(choice: ApprovalChoice) -> &'static str {
    match choice {
        ApprovalChoice::Once => "allow_once",
        ApprovalChoice::Session => "allow_session",
        ApprovalChoice::Always => "allow_until",
        ApprovalChoice::Deny => "deny",
    }
}

/// Info line for the host log. Tests also retain it so a denial can be asserted.
pub fn trace_gate(line: impl AsRef<str>) {
    let line = line.as_ref();
    log::info!("{line}");
    #[cfg(test)]
    {
        let mut buf = gate_log_buf()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if buf.len() > 400 {
            buf.drain(0..200);
        }
        buf.push(line.to_string());
    }
}

#[cfg(test)]
fn gate_log_buf() -> &'static std::sync::Mutex<Vec<String>> {
    static BUF: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> = std::sync::OnceLock::new();
    BUF.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// True when an info-level gate line containing `fragment` was emitted.
#[cfg(test)]
pub fn gate_log_contains(fragment: &str) -> bool {
    gate_log_buf()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .iter()
        .any(|line| line.contains(fragment))
}

impl serde::Serialize for AuditFact {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut row = serializer.serialize_struct("AuditFact", 10)?;
        row.serialize_field("kind", &self.kind)?;
        row.serialize_field("actor", &self.actor)?;
        row.serialize_field("call_id", &self.call_id)?;
        row.serialize_field("grant_id", &self.grant_id)?;
        row.serialize_field("resource_id", &self.resource_id)?;
        row.serialize_field("args_fingerprint", &self.args_fingerprint)?;
        row.serialize_field("operation_id", &self.operation_id)?;
        row.serialize_field("decision", &self.decision)?;
        row.serialize_field("revision_before", &self.revision_before)?;
        row.serialize_field("revision_after", &self.revision_after)?;
        row.end()
    }
}

fn pending_view(row: &Pending) -> PendingView {
    PendingView {
        pending_request_id: row.id.clone(),
        actor_id: row.binding.actor_id.clone(),
        tool: row.tool.clone(),
        resource_id: row.binding.resource_id.clone(),
        args_fingerprint: row.binding.args_fingerprint.clone(),
        input_summary: row.input_summary.clone(),
    }
}

fn same_pending(row: &Pending, binding: &ExactBinding) -> bool {
    row.binding.actor_id == binding.actor_id
        && row.binding.actor_type == binding.actor_type
        && row.binding.actor_scope == binding.actor_scope
        && row.binding.trust_origin == binding.trust_origin
        && row.binding.target_id == binding.target_id
        && row.binding.resource_scope == binding.resource_scope
        && row.binding.resource_id == binding.resource_id
        && row.binding.args_fingerprint == binding.args_fingerprint
        && row.binding.package_id == binding.package_id
        && row.binding.instance_id == binding.instance_id
        && row.binding.context_id == binding.context_id
        && row.binding.workspace_root == binding.workspace_root
}

fn summarize(input_json: &str) -> String {
    let mut text = input_json.chars().take(180).collect::<String>();
    if input_json.chars().count() > 180 {
        text.push('…');
    }
    text
}

pub fn operation_id_of(input_json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(input_json)
        .ok()
        .and_then(|value| {
            value
                .get("operation_id")
                .and_then(|item| item.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default()
}

pub(crate) fn revisions_from_output(output: Option<&str>) -> (String, String) {
    let Some(raw) = output else {
        return (String::new(), String::new());
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return (String::new(), String::new());
    };
    let pick = |key: &str| {
        value
            .get(key)
            .map(|item| match item {
                serde_json::Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default()
    };
    (pick("revision_before"), pick("revision_after"))
}

pub(crate) fn resource_of(tool: &str, input_json: &str) -> (ResourceScope, Option<String>) {
    let parsed = serde_json::from_str::<serde_json::Value>(input_json).ok();
    let game = parsed.as_ref().and_then(|value| {
        value
            .get("game_id")
            .and_then(|item| item.as_str())
            .map(str::to_string)
    });
    if tool.contains("chess") {
        if let Some(game_id) = game {
            return (ResourceScope::Game, Some(game_id));
        }
    }
    if let Some(path) = parsed.as_ref().and_then(|value| {
        value
            .get("path")
            .and_then(|item| item.as_str())
            .map(str::to_string)
    }) {
        return (ResourceScope::Path, Some(path));
    }
    (ResourceScope::Workspace, None)
}

/// Canonical argument fingerprint. Duplicate object keys are rejected. Object
/// key order is normalized. String contents and numeric lexemes are preserved.
/// Fingerprint of one event-stream target id. Grants and evaluations share it.
pub fn stream_fingerprint(target_id: &str) -> String {
    let raw = format!(
        "{{\"stream\":{}}}",
        serde_json::to_string(target_id).unwrap_or_else(|_| "\"\"".to_string())
    );
    fingerprint_args(&raw).unwrap_or_default()
}

pub fn fingerprint_args(raw: &str) -> Result<String, String> {
    let value = parse_canon(raw)?;
    let canonical = canon_string(&value);
    let mut hasher = Sha256::new();
    hasher.update(b"plexi-args-v1\n");
    hasher.update(canonical.as_bytes());
    Ok(hex_encode(&hasher.finalize()))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

#[derive(Debug)]
enum Canon {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Canon>),
    Object(BTreeMap<String, Canon>),
}

fn canon_string(value: &Canon) -> String {
    match value {
        Canon::Null => "null".to_string(),
        Canon::Bool(flag) => flag.to_string(),
        Canon::Number(lexeme) => lexeme.clone(),
        Canon::String(text) => serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string()),
        Canon::Array(items) => {
            let body = items.iter().map(canon_string).collect::<Vec<_>>().join(",");
            format!("[{body}]")
        }
        Canon::Object(map) => {
            let body = map
                .iter()
                .map(|(key, item)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_default(),
                        canon_string(item)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
    }
}

struct Parser<'a> {
    raw: &'a [u8],
    index: usize,
}

fn parse_canon(raw: &str) -> Result<Canon, String> {
    let mut parser = Parser {
        raw: raw.as_bytes(),
        index: 0,
    };
    parser.skip_ws();
    let value = parser.parse_value()?;
    parser.skip_ws();
    if parser.index != parser.raw.len() {
        return Err("trailing data in tool arguments".to_string());
    }
    Ok(value)
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while let Some(byte) = self.raw.get(self.index) {
            if byte.is_ascii_whitespace() {
                self.index += 1;
            } else {
                break;
            }
        }
    }

    fn parse_value(&mut self) -> Result<Canon, String> {
        self.skip_ws();
        match self.raw.get(self.index).copied() {
            Some(b'n') => self.consume_lit(b"null", Canon::Null),
            Some(b't') => self.consume_lit(b"true", Canon::Bool(true)),
            Some(b'f') => self.consume_lit(b"false", Canon::Bool(false)),
            Some(b'"') => Ok(Canon::String(self.parse_string()?)),
            Some(b'[') => self.parse_array(),
            Some(b'{') => self.parse_object(),
            Some(b'-') | Some(b'0'..=b'9') => self.parse_number(),
            _ => Err("invalid JSON value".to_string()),
        }
    }

    fn consume_lit(&mut self, lit: &[u8], value: Canon) -> Result<Canon, String> {
        if self.raw[self.index..].starts_with(lit) {
            self.index += lit.len();
            Ok(value)
        } else {
            Err("invalid JSON literal".to_string())
        }
    }

    fn parse_array(&mut self) -> Result<Canon, String> {
        self.index += 1;
        let mut items = Vec::new();
        self.skip_ws();
        if self.raw.get(self.index) == Some(&b']') {
            self.index += 1;
            return Ok(Canon::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.raw.get(self.index).copied() {
                Some(b',') => {
                    self.index += 1;
                }
                Some(b']') => {
                    self.index += 1;
                    break;
                }
                _ => return Err("invalid JSON array".to_string()),
            }
        }
        Ok(Canon::Array(items))
    }

    fn parse_object(&mut self) -> Result<Canon, String> {
        self.index += 1;
        let mut map = BTreeMap::new();
        self.skip_ws();
        if self.raw.get(self.index) == Some(&b'}') {
            self.index += 1;
            return Ok(Canon::Object(map));
        }
        loop {
            self.skip_ws();
            if self.raw.get(self.index) != Some(&b'"') {
                return Err("object key must be a string".to_string());
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.raw.get(self.index) != Some(&b':') {
                return Err("expected ':' after object key".to_string());
            }
            self.index += 1;
            let value = self.parse_value()?;
            if map.insert(key.clone(), value).is_some() {
                return Err(format!("duplicate object key {key}"));
            }
            self.skip_ws();
            match self.raw.get(self.index).copied() {
                Some(b',') => self.index += 1,
                Some(b'}') => {
                    self.index += 1;
                    break;
                }
                _ => return Err("invalid JSON object".to_string()),
            }
        }
        Ok(Canon::Object(map))
    }

    fn parse_number(&mut self) -> Result<Canon, String> {
        let start = self.index;
        if self.raw.get(self.index) == Some(&b'-') {
            self.index += 1;
        }
        if self.raw.get(self.index) == Some(&b'0') {
            self.index += 1;
        } else if self.raw.get(self.index).is_some_and(|b| b.is_ascii_digit()) {
            while self.raw.get(self.index).is_some_and(|b| b.is_ascii_digit()) {
                self.index += 1;
            }
        } else {
            return Err("invalid number".to_string());
        }
        if self.raw.get(self.index) == Some(&b'.') {
            self.index += 1;
            if !self.raw.get(self.index).is_some_and(|b| b.is_ascii_digit()) {
                return Err("invalid number".to_string());
            }
            while self.raw.get(self.index).is_some_and(|b| b.is_ascii_digit()) {
                self.index += 1;
            }
        }
        if matches!(self.raw.get(self.index), Some(b'e' | b'E')) {
            self.index += 1;
            if matches!(self.raw.get(self.index), Some(b'+' | b'-')) {
                self.index += 1;
            }
            if !self.raw.get(self.index).is_some_and(|b| b.is_ascii_digit()) {
                return Err("invalid number".to_string());
            }
            while self.raw.get(self.index).is_some_and(|b| b.is_ascii_digit()) {
                self.index += 1;
            }
        }
        let lexeme = std::str::from_utf8(&self.raw[start..self.index])
            .map_err(|_| "invalid number".to_string())?
            .to_string();
        Ok(Canon::Number(lexeme))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.index += 1;
        let mut out = String::new();
        while let Some(byte) = self.raw.get(self.index).copied() {
            self.index += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let esc = self.raw.get(self.index).copied().ok_or("bad escape")?;
                    self.index += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hex = self.raw.get(self.index..self.index + 4).ok_or("bad unicode")?;
                            self.index += 4;
                            let text = std::str::from_utf8(hex).map_err(|_| "bad unicode")?;
                            let code = u32::from_str_radix(text, 16).map_err(|_| "bad unicode")?;
                            out.push(char::from_u32(code).ok_or("bad unicode")?);
                        }
                        _ => return Err("bad escape".to_string()),
                    }
                }
                _ => out.push(byte as char),
            }
        }
        Err("unterminated string".to_string())
    }
}

/// Structured error body carried on the Rust result, the JSON bridge, MCP, and CLI.
pub fn structured_error(code: &str, call_id: &str, pending_request_id: Option<&str>) -> String {
    let state = match code {
        "permission_required" => "needs-grant",
        "permission_denied" => "denied",
        "stale_revision" => "stale",
        "edit_conflict" => "conflict",
        "operation_conflict" => "conflict",
        "outcome_unknown" => "unknown",
        _ => "error",
    };
    let retry = if code == "permission_required" {
        "resume_exact_request"
    } else {
        "none"
    };
    serde_json::json!({
        "schema_version": SCHEMA,
        "call_id": call_id,
        "error": {
            "code": code,
            "state": state,
            "pending_request_id": pending_request_id,
            "retry": retry,
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    fn binding(fingerprint: &str) -> ExactBinding {
        ExactBinding {
            actor_type: ActorType::Agent,
            actor_id: "agent:chess".to_string(),
            actor_scope: ActorScope::User,
            trust_origin: "host".to_string(),
            workspace_root: PathBuf::from("/ws"),
            target_type: TargetType::AppConnector,
            target_id: "chess.play".to_string(),
            resource_scope: ResourceScope::Game,
            resource_id: Some("game-1".to_string()),
            args_fingerprint: fingerprint.to_string(),
            session_id: Some("sess-a".to_string()),
            package_id: "chess".to_string(),
            instance_id: Some(4),
            context_id: Some(9),
            call_id: "call-a".to_string(),
            operation_id: "op-1".to_string(),
        }
    }

    fn admit_of(monitor: &PermissionMonitor, binding: &ExactBinding, args: &str) -> Admission {
        monitor.admit(AdmitRequest {
            call_id: &binding.call_id,
            tool: &binding.target_id,
            input_json: args,
            actor_type: binding.actor_type,
            actor_id: &binding.actor_id,
            actor_scope: binding.actor_scope,
            trust_origin: &binding.trust_origin,
            workspace_root: &binding.workspace_root,
            context_id: binding.context_id.unwrap_or(0),
            package_id: &binding.package_id,
            instance_id: binding.instance_id.unwrap_or(0),
            target_type: binding.target_type,
        })
    }

    #[test]
    fn grant_binding_and_lifetime_matrix() {
        let args_a = r#"{"game_id":"game-1","move":"e2e4","expected_revision":0,"operation_id":"op-1"}"#;
        let args_b = r#"{"move":"e2e4","operation_id":"op-1","expected_revision":0,"game_id":"game-1"}"#;
        let fp = fingerprint_args(args_a).unwrap();
        assert_eq!(fp, fingerprint_args(args_b).unwrap(), "key order is insignificant");
        assert_ne!(
            fp,
            fingerprint_args(r#"{"game_id":"game-1","move":"e2e4 ","expected_revision":0,"operation_id":"op-1"}"#).unwrap(),
            "string whitespace is significant"
        );
        assert_ne!(
            fingerprint_args("{\"n\":1}").unwrap(),
            fingerprint_args("{\"n\":1.0}").unwrap(),
            "numeric lexemes are significant"
        );
        assert!(fingerprint_args(r#"{"a":1,"a":2}"#).is_err(), "duplicate keys rejected");

        let monitor = PermissionMonitor::ephemeral();
        let mut base = binding(&fp);
        base.session_id = Some(monitor.session_id().to_string());
        // Schema-0 broad grant does not authorize.
        monitor.store().record(GrantRecord {
            actor_type: ActorType::Agent,
            actor_id: base.actor_id.clone(),
            actor_scope: ActorScope::User,
            workspace_root: None,
            target_type: TargetType::AppConnector,
            target_id: "chess.play".to_string(),
            resource_scope: ResourceScope::Global,
            resource_id: None,
            decision: Decision::Allow,
            duration: GrantDuration::Always,
            source: GrantSource::User,
            created_at: 0,
            expires_at: None,
            ..GrantRecord::unbound()
        });
        assert!(matches!(
            admit_of(&monitor, &base, args_a),
            Admission::Required { .. }
        ));

        let mut record = GrantRecord::from_binding(&base, Decision::Allow, GrantDuration::Always, GrantSource::User, "g1");
        record.expires_at = Some(crate::platform::clock::now_secs() as i64 + 3600);
        monitor.store().record(record);
        assert!(matches!(admit_of(&monitor, &base, args_a), Admission::Proceed { .. }));

        let mut wrong = base.clone();
        wrong.actor_id = "agent:other".into();
        assert!(matches!(admit_of(&monitor, &wrong, args_a), Admission::Required { .. } | Admission::Denied { .. }));
        wrong.actor_id = base.actor_id.clone();
        wrong.resource_id = Some("game-2".into());
        // resource comes from args, so pass a different game id
        let other_game = r#"{"game_id":"game-2","move":"e2e4","expected_revision":0,"operation_id":"op-1"}"#;
        assert!(matches!(admit_of(&monitor, &base, other_game), Admission::Required { .. }));
        let changed = r#"{"game_id":"game-1","move":"d2d4","expected_revision":0,"operation_id":"op-1"}"#;
        assert!(matches!(admit_of(&monitor, &base, changed), Admission::Required { .. }));

        // Expiry.
        let mut expired = GrantRecord::from_binding(&base, Decision::Allow, GrantDuration::Always, GrantSource::User, "g-exp");
        expired.expires_at = Some(crate::platform::clock::now_secs() as i64 - 5);
        expired.args_fingerprint = fingerprint_args(changed).unwrap();
        expired.resource_id = Some("game-1".into());
        monitor.store().record(expired);
        assert!(matches!(admit_of(&monitor, &base, changed), Admission::Required { .. }));

        // Session mismatch.
        let session_args = r#"{"game_id":"game-1","move":"a2a3","expected_revision":0,"operation_id":"op-s"}"#;
        let mut session_binding = base.clone();
        session_binding.args_fingerprint = fingerprint_args(session_args).unwrap();
        session_binding.session_id = Some("other-session".into());
        let mut session_grant = GrantRecord::from_binding(
            &session_binding,
            Decision::Allow,
            GrantDuration::Session,
            GrantSource::Session,
            "g-sess",
        );
        session_grant.session_id = Some("other-session".into());
        monitor.store().record(session_grant);
        assert!(
            matches!(admit_of(&monitor, &base, session_args), Admission::Required { .. }),
            "a session grant for a different session must not match"
        );

        // Once: second distinct operation does not reuse it.
        let once_args = r#"{"game_id":"game-1","move":"b2b3","expected_revision":0,"operation_id":"op-once"}"#;
        let once_fp = fingerprint_args(once_args).unwrap();
        let mut once_binding = base.clone();
        once_binding.args_fingerprint = once_fp.clone();
        once_binding.session_id = Some(monitor.session_id().to_string());
        monitor.store().record(GrantRecord::from_binding(
            &once_binding,
            Decision::Allow,
            GrantDuration::Once,
            GrantSource::Session,
            "g-once",
        ));
        assert!(matches!(admit_of(&monitor, &once_binding, once_args), Admission::Proceed { .. }));
        monitor.consume_once("g-once", Some("op-once"));
        assert!(
            matches!(admit_of(&monitor, &once_binding, once_args), Admission::Proceed { .. }),
            "the same operation id recovers the original receipt"
        );
        let fresh_once = r#"{"game_id":"game-1","move":"b2b3","expected_revision":0,"operation_id":"op-once-2"}"#;
        assert!(
            matches!(admit_of(&monitor, &once_binding, fresh_once), Admission::Required { .. } | Admission::Denied { .. }),
            "a consumed one-shot does not authorize a different operation"
        );

        // Revoke wins before a new commit.
        let hold = monitor.hold_admission();
        let worker = monitor.clone_for_test();
        let fp_for_worker = fp.clone();
        let handle = thread::spawn(move || worker.note_use("agent:chess", "c", "g1", &fp_for_worker, "game-1", "op-1"));
        thread::sleep(Duration::from_millis(40));
        assert!(monitor.revoke_while_held(&hold, "g1"));
        drop(hold);
        let result = handle.join().unwrap();
        assert!(result.is_err(), "no new commit starts after revoke wins: {result:?}");
        monitor.fail_audit(true);
        assert!(monitor.note_outcome("agent:chess", "c2", "g1", &fp, "game-1", "op-1", "ok", "0", "1").is_err());
        monitor.fail_audit(false);
        let _ = monitor.list_pending();
        let _ = monitor.audit_records();
    }

    #[test]
    fn needs_you_click_approval_resolves_once_and_the_tool_proceeds() {
        let monitor = PermissionMonitor::ephemeral();
        let args = r#"{"game_id":"game-1","move":"e2e4","expected_revision":0,"operation_id":"op-1"}"#;
        let mut base = binding(&fingerprint_args(args).unwrap());
        base.session_id = Some(monitor.session_id().to_string());
        let Admission::Required { pending_request_id } = admit_of(&monitor, &base, args) else {
            panic!("a click approval must be filed");
        };
        let open = monitor.list_needs_you();
        assert_eq!(open.len(), 1, "{open:?}");
        assert_eq!(open[0].id, pending_request_id);
        assert_eq!(open[0].kind, NeedsYouKind::ApprovalClick);
        assert!(open[0].resolution.is_none());
        assert!(open[0].expires_at.is_none());
        assert_eq!(open[0].actor, base.actor_id);
        assert_eq!(open[0].resource, "game-1");
        assert_eq!(open[0].run_tag.as_deref(), Some(base.call_id.as_str()));
        monitor.set_now_for_test(9_000_000_000);
        assert_eq!(monitor.list_needs_you().len(), 1, "click items do not expire");
        let receipt = monitor.resolve_needs_you(&pending_request_id, true).unwrap();
        assert!(!receipt.already);
        assert_eq!(receipt.resolution, NeedsYouResolution::Approved);
        assert!(monitor.list_needs_you().is_empty());
        assert!(
            matches!(admit_of(&monitor, &base, args), Admission::Proceed { .. }),
            "the approved tool proceeds"
        );
        assert_eq!(needs_you_decisions(&monitor, "approved"), 1);
        let again = monitor.resolve_needs_you(&pending_request_id, false).unwrap();
        assert!(again.already);
        assert_eq!(again.resolution, NeedsYouResolution::Approved);
        assert_eq!(needs_you_decisions(&monitor, "approved"), 1);
        assert_eq!(needs_you_decisions(&monitor, "denied"), 0);
    }

    #[test]
    fn refuse_client_resolve_leaves_the_click_open_and_deny_still_settles() {
        let monitor = PermissionMonitor::ephemeral();
        let args = r#"{"game_id":"game-1","move":"e2e4","expected_revision":0,"operation_id":"op-refuse"}"#;
        let mut base = binding(&fingerprint_args(args).unwrap());
        base.session_id = Some(monitor.session_id().to_string());
        let Admission::Required { pending_request_id } = admit_of(&monitor, &base, args) else {
            panic!("a click approval must be filed");
        };
        monitor.refuse_client_resolve(&pending_request_id);
        assert_eq!(monitor.list_needs_you().len(), 1, "a refused approve leaves the row open");
        assert!(
            matches!(admit_of(&monitor, &base, args), Admission::Required { .. }),
            "a refused approve does not unblock the tool"
        );
        assert!(
            monitor.audit_records().iter().any(|fact| {
                fact.kind == "refuse"
                    && fact.decision == "refused_resolve"
                    && fact.call_id == pending_request_id
            }),
            "the refusal is an audit row"
        );
        let denied = monitor.resolve_needs_you(&pending_request_id, false).unwrap();
        assert!(!denied.already);
        assert_eq!(denied.resolution, NeedsYouResolution::Denied);
        assert!(monitor.list_needs_you().is_empty());
        assert!(monitor.store().records().is_empty(), "deny does not mint a grant");
    }

    #[test]
    fn needs_you_question_and_blocked_run_resolve_without_a_grant() {
        let monitor = PermissionMonitor::ephemeral();
        monitor.set_now_for_test(1_000);
        let question = monitor
            .file_needs_you(NeedsYouFile {
                kind: NeedsYouKind::Question,
                actor: "agent:guide".into(),
                resource: "repo".into(),
                summary: "Which branch should I push?".into(),
                expires_at: Some(1_050),
                run_tag: Some("run-9".into()),
            })
            .unwrap();
        let blocked = monitor
            .file_needs_you(NeedsYouFile {
                kind: NeedsYouKind::BlockedRun,
                actor: "agent:guide".into(),
                resource: "ci".into(),
                summary: "Runner stopped before the build finished".into(),
                expires_at: None,
                run_tag: Some("run-9".into()),
            })
            .unwrap();
        assert!(monitor
            .file_needs_you(NeedsYouFile {
                kind: NeedsYouKind::ApprovalClick,
                actor: "agent:guide".into(),
                resource: "repo".into(),
                summary: "no".into(),
                expires_at: None,
                run_tag: None,
            })
            .is_err());
        let again_blocked = monitor
            .file_needs_you(NeedsYouFile {
                kind: NeedsYouKind::BlockedRun,
                actor: "agent:guide".into(),
                resource: "ci".into(),
                summary: "Runner stopped before the build finished".into(),
                expires_at: None,
                run_tag: Some("run-9".into()),
            })
            .unwrap();
        assert_eq!(again_blocked, blocked);
        assert_eq!(monitor.list_needs_you().len(), 2);
        monitor.set_now_for_test(1_050);
        let open = monitor.list_needs_you();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, blocked);
        assert_eq!(needs_you_decisions(&monitor, "auto_denied"), 1);
        let grants = monitor.store().records().len();
        let receipt = monitor.resolve_needs_you(&blocked, true).unwrap();
        assert!(!receipt.already);
        assert_eq!(receipt.resolution, NeedsYouResolution::Approved);
        assert_eq!(monitor.store().records().len(), grants);
        let again = monitor.resolve_needs_you(&blocked, false).unwrap();
        assert!(again.already);
        assert_eq!(again.resolution, NeedsYouResolution::Approved);
        let denied = monitor.resolve_needs_you(&question, true).unwrap();
        assert!(denied.already);
        assert_eq!(denied.resolution, NeedsYouResolution::Denied);
        assert_eq!(needs_you_decisions(&monitor, "approved"), 1);
    }

    #[test]
    fn needs_you_survives_reload_and_reports_outcome_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let monitor = PermissionMonitor::for_profile(dir.path());
        let args = r#"{"game_id":"game-1","move":"e2e4","expected_revision":0,"operation_id":"op-persist"}"#;
        let mut base = binding(&fingerprint_args(args).unwrap());
        base.session_id = Some(monitor.session_id().to_string());
        let Admission::Required { pending_request_id } = admit_of(&monitor, &base, args) else {
            panic!("a click approval must be filed");
        };
        let blocked = monitor
            .file_needs_you(NeedsYouFile {
                kind: NeedsYouKind::BlockedRun,
                actor: "agent:guide".into(),
                resource: "ci".into(),
                summary: "Runner stopped".into(),
                expires_at: None,
                run_tag: Some("run-persist".into()),
            })
            .unwrap();
        drop(monitor);
        let restored = PermissionMonitor::reload_for_test(dir.path());
        let open = restored.list_needs_you();
        assert!(
            open.iter().any(|row| row.id == pending_request_id),
            "pending id must survive restart: {open:?}"
        );
        assert!(open.iter().any(|row| row.id == blocked), "{open:?}");
        let receipt = restored
            .resolve_needs_you(&pending_request_id, true)
            .unwrap();
        assert!(!receipt.already);
        assert_eq!(receipt.resolution, NeedsYouResolution::Approved);
        assert_eq!(receipt.run_outcome, RunOutcome::OutcomeUnknown);
        assert!(
            restored
                .list_needs_you()
                .iter()
                .all(|row| row.id != pending_request_id),
            "resolving must close the item"
        );
        assert_eq!(needs_you_decisions(&restored, "outcome_unknown"), 1);
        let mut retry = base.clone();
        retry.session_id = Some(restored.session_id().to_string());
        assert!(
            matches!(admit_of(&restored, &retry, args), Admission::Proceed { .. }),
            "the exact request is unblocked by the restored approval"
        );
        let question = restored.resolve_needs_you(&blocked, true).unwrap();
        assert_eq!(question.run_outcome, RunOutcome::OutcomeUnknown);
    }

    #[test]
    fn forged_queue_is_not_a_grant_and_is_not_silent() {
        let dir = tempfile::tempdir().unwrap();
        let monitor = PermissionMonitor::for_profile(dir.path());
        let args = r#"{"game_id":"game-1","move":"e2e4","expected_revision":0,"operation_id":"op-forge"}"#;
        let mut base = binding(&fingerprint_args(args).unwrap());
        base.session_id = Some(monitor.session_id().to_string());
        let Admission::Required { pending_request_id } = admit_of(&monitor, &base, args) else {
            panic!("a click approval must be filed");
        };
        drop(monitor);
        let journal = dir.path().join("host").join("needs-you.json");
        std::fs::write(
            &journal,
            b"{\"schema\":1,\"items\":[{\"id\":\"req_forged\",\"kind\":\"approval_click\",\"resolution\":\"approved\"}]}\n#mac 00\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("needs-you.json"),
            b"{\"id\":\"req_planted\",\"resolution\":\"approved\"}\n",
        )
        .unwrap();
        let restored = PermissionMonitor::reload_for_test(dir.path());
        let open = restored.list_needs_you();
        assert!(
            open.iter().all(|row| row.id != "req_forged" && row.id != "req_planted"),
            "forged ids must not be listed: {open:?}"
        );
        assert!(
            open.iter().any(|row| row.kind == NeedsYouKind::Integrity),
            "a rejected queue must surface an integrity item, not disappear: {open:?}"
        );
        assert!(restored.resolve_needs_you("req_forged", true).is_err());
        assert!(
            restored.store().records().iter().all(|record| record.grant_id != "req_forged"),
            "a forged file must not become a grant"
        );
        assert_ne!(pending_request_id, "req_forged");
        let _ = pending_request_id;
    }

    #[test]
    fn deleted_queue_is_an_integrity_item() {
        let dir = tempfile::tempdir().unwrap();
        let monitor = PermissionMonitor::for_profile(dir.path());
        let args = r#"{"game_id":"game-1","move":"e2e4","expected_revision":0,"operation_id":"op-delete"}"#;
        let mut base = binding(&fingerprint_args(args).unwrap());
        base.session_id = Some(monitor.session_id().to_string());
        let Admission::Required { .. } = admit_of(&monitor, &base, args) else {
            panic!("a click approval must be filed");
        };
        drop(monitor);
        std::fs::remove_file(dir.path().join("host").join("needs-you.json")).unwrap();
        let restored = PermissionMonitor::reload_for_test(dir.path());
        let open = restored.list_needs_you();
        assert!(
            open.iter().any(|row| row.kind == NeedsYouKind::Integrity),
            "deleting the queue must be reported: {open:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn host_queue_directory_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let monitor = PermissionMonitor::for_profile(dir.path());
        let args = r#"{"game_id":"game-1","move":"e2e4","expected_revision":0,"operation_id":"op-mode"}"#;
        let mut base = binding(&fingerprint_args(args).unwrap());
        base.session_id = Some(monitor.session_id().to_string());
        let Admission::Required { .. } = admit_of(&monitor, &base, args) else {
            panic!("a click approval must be filed");
        };
        let host = dir.path().join("host");
        let dir_mode = std::fs::metadata(&host).unwrap().permissions().mode() & 0o777;
        let file_mode = std::fs::metadata(host.join("needs-you.json")).unwrap().permissions().mode() & 0o777;
        let key_mode = std::fs::metadata(host.join("seal.key")).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700, "host directory must be mode 0700");
        assert_eq!(file_mode, 0o600);
        assert_eq!(key_mode, 0o600);
        let journal = std::fs::read(host.join("needs-you.json")).unwrap();
        let key = std::fs::read(host.join("seal.key")).unwrap();
        assert!(!journal.windows(key.len()).any(|window| window == key.as_slice()));
        assert!(!dir.path().join("needs-you.json").exists());
        assert!(!dir.path().join("secrets.json").exists());
    }

    fn needs_you_decisions(monitor: &PermissionMonitor, decision: &str) -> usize {
        monitor
            .audit_records()
            .iter()
            .filter(|fact| fact.kind == "needs_you" && fact.decision == decision)
            .count()
    }
}

impl PermissionMonitor {
    #[cfg(test)]
    fn clone_for_test(self: &Arc<Self>) -> Arc<Self> {
        Arc::clone(self)
    }
}
