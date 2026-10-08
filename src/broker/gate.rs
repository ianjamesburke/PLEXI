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
    /// Refuse this call only. The next call asks again.
    Deny,
    /// Store a tool-scoped deny until a person resets or allows it.
    DenyAlways,
}

/// One host record for everything waiting on the human.
///
/// Each-time and time-boxed sign-off live on the Touch ID spike. This gate
/// files click approvals, agent questions, and blocked runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NeedsYouKind {
    ApprovalClick,
    Question,
    BlockedRun,
    /// An agent asked to widen a permission. Approval applies the change.
    PermissionChange,
    /// A grant, deny, or audit file failed its integrity check.
    /// Resolving it does not mint a grant.
    Integrity,
}

impl NeedsYouKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApprovalClick => "approval_click",
            Self::Question => "question",
            Self::BlockedRun => "blocked_run",
            Self::PermissionChange => "permission_change",
            Self::Integrity => "integrity",
        }
    }

    fn is_approval(self) -> bool {
        matches!(self, Self::ApprovalClick)
    }

    /// A paired phone may answer a question or a blocked run.
    /// An approval click, a permission change, and an integrity alert
    /// stay on the desktop. Approving one of those is not a grant.
    pub fn phone_may_approve(self) -> bool {
        matches!(self, Self::Question | Self::BlockedRun)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsYouReceipt {
    pub id: String,
    pub resolution: NeedsYouResolution,
    pub already: bool,
}

#[derive(Debug, Clone)]
struct PendingWiden {
    entry_id: String,
    action: String,
}

/// One row the Permissions app and `plexi permissions list` share.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PermissionEntry {
    pub id: String,
    /// `allow`, `deny`, or `pending`.
    pub kind: String,
    /// `once`, `session`, `until`, `always`, or `ask`.
    pub duration: String,
    pub actor_type: String,
    pub actor_id: String,
    pub tool: String,
    pub resource_id: Option<String>,
    pub resource_scope: String,
    pub package_id: String,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub when: String,
    pub source: String,
    pub workspace: String,
    pub summary: String,
}

/// Who is asking to change a stored permission.
#[derive(Debug, Clone)]
pub struct PermissionCaller {
    /// A person at a terminal with no pane and no call credential.
    /// The Permissions app's own buttons also pass `true`.
    pub human: bool,
    pub actor_id: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PermissionMutation {
    Applied { entry_id: String },
    NeedsYou { needs_you_id: String, entry_id: String },
    Missing { entry_id: String },
    Rejected { entry_id: String, error: String },
}

pub fn needs_you_phone_items(items: &[NeedsYouRecord]) -> Vec<serde_json::Value> {
    items
        .iter()
        .map(|item| {
            let mut value = serde_json::to_value(item).unwrap_or_else(|_| serde_json::json!({}));
            if let Some(obj) = value.as_object_mut() {
                obj.insert(
                    "phone_can_approve".to_string(),
                    serde_json::Value::Bool(item.kind.phone_may_approve()),
                );
            }
            value
        })
        .collect()
}

/// One painted permission-sheet button, in window points.
#[derive(Debug, Clone)]
struct SheetButton {
    label: String,
    bounds: [f64; 4],
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
    created_at: i64,
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
    audit_mem: Mutex<Vec<AuditFact>>,
    fail_audit: AtomicBool,
    /// Open and resolved items waiting on the human. One map, one resolution.
    needs_you: Mutex<BTreeMap<String, NeedsYouRecord>>,
    /// Widen requests filed by an agent, applied only when Needs you approves.
    widens: Mutex<BTreeMap<String, PendingWiden>>,
    /// Serializes list, expiry, and resolve so one terminal receipt wins.
    needs_resolve: Mutex<()>,
    /// Pairing codes waiting for a desktop click. Not tool grants.
    pairing_codes: Mutex<Vec<String>>,
    /// Painted permission-sheet buttons, so a real pointer click can find
    /// "Allow once" when the accesskit tree has not published that label.
    sheet_buttons: Mutex<Vec<SheetButton>>,
    /// Frame currently painting. `sheet_published_epoch` matches it only
    /// after this frame drew the sheet.
    sheet_open_epoch: AtomicU64,
    sheet_published_epoch: AtomicU64,
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
            .or_insert_with(|| Self::open_profile(dir))
            .clone()
    }

    /// Open `dir` without the process cache. A restart test uses this so a
    /// second look actually reads the files again.
    pub fn open_profile(dir: &Path) -> Arc<Self> {
        Arc::new(Self::open(dir))
    }

    /// Private store. Used by tests that must not share a profile monitor.
    pub fn ephemeral() -> Arc<Self> {
        Arc::new(Self::new(GrantStore::default(), None))
    }

    fn open(dir: &Path) -> Self {
        // Snapshot before the grant load. Adopting a legacy file creates the
        // MAC key, and a later check would then look sealed.
        let never_sealed = super::seal::profile_never_sealed(dir);
        let store = GrantStore::load_or_default(dir);
        let mut faults = store.integrity_faults().to_vec();
        // `AgentHost::production` and `HostSubscriptionService::new` load the
        // grant file during startup, before this monitor exists. That load
        // quarantines a bad MAC. The fault is noted so this open still files
        // Needs you after the file is already gone.
        for fault in super::seal::take_integrity_faults(dir) {
            if !faults
                .iter()
                .any(|have| have.file == fault.file && have.reason == fault.reason)
            {
                faults.push(fault);
            }
        }
        let audit = dir.join("permission-audit.jsonl");
        if let Err(reason) = super::seal::verify_audit(&audit) {
            if never_sealed
                && super::seal::unauthenticated_audit(&reason)
                && super::seal::adopt_legacy_audit(&audit).is_ok()
            {
                log::info!(
                    "permission_monitor: migrated legacy unsealed permission-audit.jsonl"
                );
            } else {
                super::seal::reject_untrusted_audit(&audit, &reason);
                faults.push(super::seal::IntegrityFault {
                    file: "permission-audit.jsonl".to_string(),
                    reason,
                });
            }
        }
        log::info!(
            "permission_monitor: opened profile {} audit {} integrity_faults={}",
            dir.display(),
            audit.display(),
            faults.len()
        );
        let monitor = Self::new(store, Some(audit));
        for fault in &faults {
            monitor.raise_integrity(fault);
        }
        monitor
    }

    fn raise_integrity(&self, fault: &super::seal::IntegrityFault) {
        let summary = format!(
            "{} failed integrity ({}). Stored permission decisions in that file were ignored.",
            fault.file, fault.reason
        );
        if let Err(error) = self.file_needs_you(NeedsYouFile {
            kind: NeedsYouKind::Integrity,
            actor: "host".to_string(),
            resource: fault.file.clone(),
            summary,
            expires_at: None,
            run_tag: Some(format!("integrity:{}", fault.file)),
        }) {
            log::error!("permission_seal: could not file integrity alert: {error}");
        }
        let fact = AuditFact {
            kind: "integrity".to_string(),
            actor: "host".to_string(),
            call_id: String::new(),
            grant_id: String::new(),
            resource_id: fault.file.clone(),
            args_fingerprint: String::new(),
            operation_id: String::new(),
            decision: fault.reason.clone(),
            revision_before: String::new(),
            revision_after: String::new(),
        };
        if let Some(path) = &self.audit_path {
            match super::seal::append_audit(path, &fact) {
                Ok(()) => {
                    self.audit_mem
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .push(fact);
                    log::info!(
                        "permission_seal: audited integrity fault {} ({})",
                        fault.file, fault.reason
                    );
                }
                Err(error) => log::error!(
                    "permission_seal: integrity needs-you stands; audit row was not written: {error}"
                ),
            }
        }
    }

    fn new(store: GrantStore, audit_path: Option<PathBuf>) -> Self {
        Self::from_arc(Arc::new(Mutex::new(store)), audit_path)
    }

    fn from_arc(store: Arc<Mutex<GrantStore>>, audit_path: Option<PathBuf>) -> Self {
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
            audit_mem: Mutex::new(Vec::new()),
            fail_audit: AtomicBool::new(false),
            needs_you: Mutex::new(BTreeMap::new()),
            widens: Mutex::new(BTreeMap::new()),
            needs_resolve: Mutex::new(()),
            pairing_codes: Mutex::new(Vec::new()),
            sheet_buttons: Mutex::new(Vec::new()),
            sheet_open_epoch: AtomicU64::new(0),
            sheet_published_epoch: AtomicU64::new(0),
            #[cfg(test)]
            now_override: AtomicI64::new(0),
        }
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
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = pending.iter().find(|row| same_pending(row, binding)).cloned() {
            log::info!(
                "permission_monitor: reuse pending {} for actor={} tool={}",
                existing.id,
                binding.actor_id,
                tool
            );
            let id = existing.id.clone();
            self.upsert_approval_needs_you(&existing);
            return id;
        }
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
            created_at: self.now_secs(),
        };
        self.upsert_approval_needs_you(&row);
        pending.push(row);
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
        if matches!(choice, ApprovalChoice::Deny | ApprovalChoice::DenyAlways) {
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|row| row.id != pending_id);
            self.resolutions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(pending_id.to_string(), choice);
            let grant_id = if choice == ApprovalChoice::DenyAlways {
                self.record_tool_scoped(&pending.binding, Decision::Deny, None)
            } else {
                String::new()
            };
            let decision = if choice == ApprovalChoice::DenyAlways {
                "deny_always"
            } else {
                "deny"
            };
            let close_decision = if choice == ApprovalChoice::DenyAlways {
                "deny_always"
            } else {
                "denied"
            };
            let _ = self.audit(&AuditFact {
                kind: "deny".to_string(),
                actor: pending.binding.actor_id.clone(),
                call_id: pending.binding.call_id.clone(),
                grant_id: grant_id.clone(),
                resource_id: pending.binding.resource_id.clone().unwrap_or_default(),
                args_fingerprint: pending.binding.args_fingerprint.clone(),
                operation_id: String::new(),
                decision: decision.to_string(),
                revision_before: String::new(),
                revision_after: String::new(),
            });
            log::info!("permission_monitor: denied pending {pending_id} decision={decision}");
            let _ = self.close_needs_you(
                pending_id,
                NeedsYouResolution::Denied,
                close_decision,
                &grant_id,
            );
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
            ApprovalChoice::Deny | ApprovalChoice::DenyAlways => {
                unreachable!("deny returned above")
            }
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
                ApprovalChoice::Deny | ApprovalChoice::DenyAlways => "deny",
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
        if filed.kind.is_approval() {
            return Err("approval items are filed by the permission gate".to_string());
        }
        if filed.actor.is_empty() || filed.summary.is_empty() {
            return Err("needs-you actor and summary are required".to_string());
        }
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
            });
        }
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
            NeedsYouKind::PermissionChange => {
                let resolution = if approve {
                    NeedsYouResolution::Approved
                } else {
                    NeedsYouResolution::Denied
                };
                let decision = if approve { "approved" } else { "denied" };
                let widen = self
                    .widens
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(id);
                if approve {
                    let (entry_id, action) = if let Some(pending) = widen {
                        (pending.entry_id, pending.action)
                    } else {
                        split_widen_tag(row.run_tag.as_deref())
                    };
                    if !entry_id.is_empty() && !action.is_empty() {
                        let outcome = self.apply_mutation(&entry_id, &action);
                        log::info!(
                            "needs_you: permission change {id} applied action={action} entry={entry_id} outcome={}",
                            mutation_label(&outcome)
                        );
                    }
                }
                if !self.close_needs_you(id, resolution, decision, "") {
                    if let Some(existing) = self.resolved_receipt(id, true) {
                        return Ok(existing);
                    }
                    return Err(format!("needs-you item {id} could not be audited"));
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
            })
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
        let fact = AuditFact {
            kind: "needs_you".to_string(),
            actor: snapshot.actor.clone(),
            call_id: snapshot.run_tag.clone().unwrap_or_else(|| snapshot.id.clone()),
            grant_id: grant_id.to_string(),
            resource_id: snapshot.resource.clone(),
            args_fingerprint: String::new(),
            operation_id: snapshot.id.clone(),
            decision: decision.to_string(),
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
        true
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
    /// observes the removal. `revoke` and `reset` on the permissions monitor
    /// call this. It does not grant.
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
            self.save_durable();
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

    /// Live inventory: durable grants and denies, session and one-shot grants
    /// still in memory, and asks waiting on a person. One list, no second store.
    pub fn list_entries(&self) -> Vec<PermissionEntry> {
        let now = self.now_secs();
        let mut entries = Vec::new();
        for record in self.store().records() {
            if let Some(expires) = record.expires_at {
                if now >= expires {
                    continue;
                }
            }
            if record.duration == GrantDuration::Once && record.consumed {
                continue;
            }
            if record.grant_id.is_empty() {
                continue;
            }
            entries.push(entry_from_record(record));
        }
        let pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        for row in pending.iter() {
            entries.push(entry_from_pending(row));
        }
        drop(pending);
        entries.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        entries
    }

    /// Persist one app capability. A later click replaces an auto-grant of the
    /// same capability instead of adding a second row.
    pub fn grant_app_capability(
        &self,
        app_id: &str,
        workspace_root: &Path,
        cap: crate::app::permissions::Capability,
        decision: Decision,
    ) {
        self.grant_capability_id(app_id, workspace_root, cap.as_str(), decision, GrantSource::User);
    }

    /// Persist one capability id (manifest capability or raw WASM import).
    pub fn grant_capability_id(
        &self,
        app_id: &str,
        workspace_root: &Path,
        capability_id: &str,
        decision: Decision,
        source: GrantSource,
    ) {
        {
            let mut store = self.store();
            store.upsert_capability(app_id, workspace_root, capability_id, decision, source);
        }
        self.save_durable();
        log::info!(
            "permission_monitor: stored capability {capability_id} for {app_id} = {} ({source:?})",
            decision.as_str()
        );
    }

    /// Turn declared capabilities into gate rows and an [`AppPermissions`] set.
    /// Non-sensitive capabilities with no stored decision are auto-granted.
    /// Sensitive capabilities stay withheld until a human grant exists.
    pub fn materialize_app_permissions(
        &self,
        app_id: &str,
        workspace_root: &Path,
        declared: &std::collections::HashSet<crate::app::permissions::Capability>,
        allowed_hosts: Vec<String>,
    ) -> crate::app::permissions::AppPermissions {
        let ids: Vec<String> = declared.iter().map(|cap| cap.as_str().to_string()).collect();
        let (granted, blocked) = self.materialize_capability_ids(app_id, workspace_root, &ids);
        let mut capabilities = std::collections::HashSet::new();
        let mut blocked_caps = std::collections::HashSet::new();
        for id in granted {
            if let Ok(cap) = crate::app::permissions::Capability::try_from(id.as_str()) {
                capabilities.insert(cap);
            }
        }
        for id in blocked {
            if let Ok(cap) = crate::app::permissions::Capability::try_from(id.as_str()) {
                blocked_caps.insert(cap);
            }
        }
        crate::app::permissions::AppPermissions {
            capabilities,
            blocked: blocked_caps,
            is_builtin: false,
            allowed_hosts,
        }
    }

    /// Raw WASM capability ids, same auto-grant rule as manifest capabilities.
    pub fn materialize_wasm_sets(
        &self,
        app_id: &str,
        workspace_root: &Path,
        declared: &std::collections::HashSet<String>,
    ) -> (std::collections::HashSet<String>, std::collections::HashSet<String>) {
        let ids: Vec<String> = declared.iter().cloned().collect();
        let (granted, blocked) = self.materialize_capability_ids(app_id, workspace_root, &ids);
        (
            granted.into_iter().collect(),
            blocked.into_iter().collect(),
        )
    }

    fn materialize_capability_ids(
        &self,
        app_id: &str,
        workspace_root: &Path,
        declared: &[String],
    ) -> (std::collections::HashSet<String>, std::collections::HashSet<String>) {
        let ws = crate::platform::path::canonical_or_self(workspace_root);
        let now = self.now_secs();
        let mut granted = std::collections::HashSet::new();
        let mut blocked = std::collections::HashSet::new();
        let mut dirty = false;
        {
            let mut store = self.store();
            for capability_id in declared {
                match stored_capability_decision(&store, app_id, &ws, capability_id, now) {
                    Some(Decision::Deny) => {
                        blocked.insert(capability_id.clone());
                    }
                    Some(Decision::Allow) => {
                        granted.insert(capability_id.clone());
                    }
                    Some(Decision::Ask) => {}
                    None if declared_capability_needs_click(capability_id) => {
                        log::info!(
                            "permission_monitor: withheld {capability_id} for {app_id} until a human grants it"
                        );
                    }
                    None => {
                        store.upsert_capability(
                            app_id,
                            workspace_root,
                            capability_id,
                            Decision::Allow,
                            GrantSource::Workspace,
                        );
                        granted.insert(capability_id.clone());
                        dirty = true;
                        log::info!(
                            "permission_monitor: auto-granted {capability_id} for {app_id}"
                        );
                    }
                }
            }
            let declared_set: std::collections::HashSet<&str> =
                declared.iter().map(String::as_str).collect();
            let extras: Vec<(String, Decision)> = store
                .records()
                .iter()
                .filter(|record| {
                    record.actor_type == ActorType::App
                        && record.actor_id == app_id
                        && record.target_type == TargetType::Capability
                        && record.workspace_root.as_deref() == Some(ws.as_path())
                        && !declared_set.contains(record.target_id.as_str())
                        && record.expires_at.is_none_or(|expires| now < expires)
                })
                .map(|record| (record.target_id.clone(), record.decision))
                .collect();
            for (capability_id, decision) in extras {
                match decision {
                    Decision::Allow => {
                        granted.insert(capability_id);
                    }
                    Decision::Deny => {
                        blocked.insert(capability_id);
                    }
                    Decision::Ask => {}
                }
            }
            if dirty {
                store.save();
            }
        }
        (granted, blocked)
    }

    /// `reset` and `allow` widen. An agent files Needs you and changes nothing.
    /// `revoke` narrows and runs from any caller.
    pub fn mutate_entry(
        &self,
        id: &str,
        action: &str,
        caller: &PermissionCaller,
    ) -> PermissionMutation {
        let action = action.trim();
        if !matches!(action, "reset" | "allow" | "revoke") {
            return PermissionMutation::Rejected {
                entry_id: id.to_string(),
                error: "action must be reset, allow, or revoke".to_string(),
            };
        }
        let widening = matches!(action, "reset" | "allow");
        if widening && !caller.human {
            return self.file_widen(id, action, &caller.actor_id);
        }
        let outcome = self.apply_mutation(id, action);
        log::info!(
            "permission_monitor: mutate {action} id={id} human={} actor={} outcome={}",
            caller.human,
            caller.actor_id,
            mutation_label(&outcome)
        );
        outcome
    }

    fn file_widen(&self, id: &str, action: &str, actor: &str) -> PermissionMutation {
        let Some(entry) = self.entry_by_id(id) else {
            log::info!("permission_monitor: widen {action} missing id={id} actor={actor}");
            return PermissionMutation::Missing {
                entry_id: id.to_string(),
            };
        };
        if action == "reset" && entry.kind != "deny" {
            return PermissionMutation::Rejected {
                entry_id: id.to_string(),
                error: "reset only clears a stored denial".to_string(),
            };
        }
        if action == "allow" && entry.kind == "allow" {
            return PermissionMutation::Applied {
                entry_id: id.to_string(),
            };
        }
        match self.file_needs_you(NeedsYouFile {
            kind: NeedsYouKind::PermissionChange,
            actor: if actor.is_empty() {
                entry.actor_id.clone()
            } else {
                actor.to_string()
            },
            resource: entry.tool.clone(),
            summary: format!(
                "Approve {action} for {} {} ({id}). An agent cannot widen this itself.",
                entry.actor_id, entry.tool
            ),
            expires_at: None,
            run_tag: Some(format!("{action}:{id}")),
        }) {
            Ok(needs_you_id) => {
                self.widens
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(
                        needs_you_id.clone(),
                        PendingWiden {
                            entry_id: id.to_string(),
                            action: action.to_string(),
                        },
                    );
                log::info!(
                    "permission_monitor: widen {action} id={id} filed needs_you={needs_you_id} actor={actor}"
                );
                PermissionMutation::NeedsYou {
                    needs_you_id,
                    entry_id: id.to_string(),
                }
            }
            Err(error) => PermissionMutation::Rejected {
                entry_id: id.to_string(),
                error,
            },
        }
    }

    fn apply_mutation(&self, id: &str, action: &str) -> PermissionMutation {
        match action {
            "revoke" => self.revoke_entry(id),
            "reset" => self.reset_entry(id),
            "allow" => self.allow_entry(id),
            _ => PermissionMutation::Rejected {
                entry_id: id.to_string(),
                error: "action must be reset, allow, or revoke".to_string(),
            },
        }
    }

    fn revoke_entry(&self, id: &str) -> PermissionMutation {
        if self.pending_exists(id) {
            return match self.approve_pending(id, ApprovalChoice::Deny) {
                Ok(()) => PermissionMutation::Applied {
                    entry_id: id.to_string(),
                },
                Err(error) => PermissionMutation::Rejected {
                    entry_id: id.to_string(),
                    error,
                },
            };
        }
        let Some(entry) = self.entry_by_id(id) else {
            return PermissionMutation::Missing {
                entry_id: id.to_string(),
            };
        };
        if entry.kind == "deny" {
            return PermissionMutation::Rejected {
                entry_id: id.to_string(),
                error: "a denial is cleared with reset, not revoke".to_string(),
            };
        }
        if self.revoke_grant_id(id) {
            PermissionMutation::Applied {
                entry_id: id.to_string(),
            }
        } else {
            PermissionMutation::Missing {
                entry_id: id.to_string(),
            }
        }
    }

    fn reset_entry(&self, id: &str) -> PermissionMutation {
        let Some(entry) = self.entry_by_id(id) else {
            return PermissionMutation::Missing {
                entry_id: id.to_string(),
            };
        };
        if entry.kind != "deny" {
            return PermissionMutation::Rejected {
                entry_id: id.to_string(),
                error: "reset only clears a stored denial".to_string(),
            };
        }
        if self.revoke_grant_id(id) {
            PermissionMutation::Applied {
                entry_id: id.to_string(),
            }
        } else {
            PermissionMutation::Missing {
                entry_id: id.to_string(),
            }
        }
    }

    fn allow_entry(&self, id: &str) -> PermissionMutation {
        if self.pending_exists(id) {
            return match self.approve_pending(id, ApprovalChoice::Always) {
                Ok(()) => PermissionMutation::Applied {
                    entry_id: id.to_string(),
                },
                Err(error) => PermissionMutation::Rejected {
                    entry_id: id.to_string(),
                    error,
                },
            };
        }
        let record = {
            let store = self.store();
            store
                .records()
                .iter()
                .find(|record| record.grant_id == id)
                .cloned()
        };
        let Some(record) = record else {
            return PermissionMutation::Missing {
                entry_id: id.to_string(),
            };
        };
        if record.decision == Decision::Allow {
            return PermissionMutation::Applied {
                entry_id: id.to_string(),
            };
        }
        if record.decision != Decision::Deny {
            return PermissionMutation::Rejected {
                entry_id: id.to_string(),
                error: "allow replaces a denial or approves a pending ask".to_string(),
            };
        }
        let grant_id = format!("grant_{}", uuid::Uuid::new_v4());
        let mut allow = record.clone();
        allow.grant_id = grant_id.clone();
        allow.decision = Decision::Allow;
        allow.duration = GrantDuration::Always;
        allow.source = GrantSource::User;
        allow.tool_scoped = true;
        allow.consumed = false;
        allow.created_at = self.now_secs();
        allow.expires_at = Some(self.now_secs() + ALWAYS_TTL_SECS);
        {
            let mut store = self.store();
            store.records_mut().retain(|row| row.grant_id != id);
            store.record(allow);
        }
        self.save_durable();
        let _ = self.audit(&AuditFact {
            kind: "grant".to_string(),
            actor: record.actor_id,
            call_id: String::new(),
            grant_id: grant_id.clone(),
            resource_id: record.resource_id.unwrap_or_default(),
            args_fingerprint: String::new(),
            operation_id: String::new(),
            decision: "allow_until".to_string(),
            revision_before: id.to_string(),
            revision_after: grant_id.clone(),
        });
        log::info!("permission_monitor: allowed {id} as {grant_id}");
        PermissionMutation::Applied {
            entry_id: grant_id,
        }
    }

    fn pending_exists(&self, id: &str) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|row| row.id == id)
    }

    fn entry_by_id(&self, id: &str) -> Option<PermissionEntry> {
        self.list_entries().into_iter().find(|entry| entry.id == id)
    }

    fn record_tool_scoped(
        &self,
        binding: &ExactBinding,
        decision: Decision,
        expires_at: Option<i64>,
    ) -> String {
        let grant_id = format!("grant_{}", uuid::Uuid::new_v4());
        let mut record = GrantRecord::from_binding(
            binding,
            decision,
            GrantDuration::Always,
            GrantSource::User,
            &grant_id,
        );
        record.tool_scoped = true;
        record.expires_at = expires_at;
        record.session_id = None;
        self.store().record(record);
        self.save_durable();
        log::info!(
            "permission_monitor: stored tool-scoped {} {grant_id} tool={}",
            decision.as_str(),
            binding.target_id
        );
        grant_id
    }

    /// Write durable rows and leave session and one-shot rows in memory only.
    fn save_durable(&self) {
        let mut store = self.store();
        let ephemeral: Vec<GrantRecord> = store
            .records()
            .iter()
            .filter(|record| !record_is_durable(record))
            .cloned()
            .collect();
        store.records_mut().retain(record_is_durable);
        store.save();
        store.records_mut().extend(ephemeral);
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    pub fn list_pending(&self) -> Vec<PendingView> {
        let mut rows: Vec<PendingView> = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(pending_view)
            .collect();
        for code in self
            .pairing_codes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            rows.push(pairing_view(code));
        }
        rows
    }

    pub fn show_pending(&self, id: &str) -> Option<PendingView> {
        if self
            .pairing_codes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|code| code == id)
        {
            return Some(pairing_view(id));
        }
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|row| row.id == id)
            .map(pending_view)
    }

    /// A socket, CLI, or synthetic-input resolve. The pending stays, no grant
    /// is written, and the audit records the refusal.
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

    /// Remember a pairing code until the person clicks the desktop sheet.
    /// The code is not logged and the click is not a tool grant.
    pub fn track_pairing_code(&self, code: &str) {
        if code.is_empty() {
            return;
        }
        let mut codes = self
            .pairing_codes
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if codes.len() == 1 && codes[0] == code {
            return;
        }
        codes.clear();
        codes.push(code.to_string());
        log::info!("permission_monitor: pairing confirmation waiting for a click");
    }

    pub fn clear_pairing_code(&self, code: &str) {
        self.pairing_codes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|item| item != code);
    }

    /// Start one paint. Buttons published earlier belong to the previous frame.
    pub fn begin_sheet_frame(&self) {
        let _ = self.sheet_open_epoch.fetch_add(1, Ordering::Relaxed);
    }

    /// Record the buttons this frame actually drew.
    pub fn publish_sheet_buttons(&self, buttons: &[(&str, [f64; 4])]) {
        let next: Vec<SheetButton> = buttons
            .iter()
            .map(|(label, bounds)| SheetButton {
                label: (*label).to_string(),
                bounds: *bounds,
            })
            .collect();
        let mut guard = self
            .sheet_buttons
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let labels_changed = guard.len() != next.len()
            || guard
                .iter()
                .zip(next.iter())
                .any(|(old, new)| old.label != new.label);
        if labels_changed {
            log::info!(
                "permission_monitor: sheet buttons published count={}",
                next.len()
            );
        }
        *guard = next;
        self.sheet_published_epoch
            .store(self.sheet_open_epoch.load(Ordering::Relaxed), Ordering::Relaxed);
    }

    /// Drop buttons when this frame did not draw the sheet.
    pub fn finish_sheet_frame(&self) {
        if self.sheet_published_epoch.load(Ordering::Relaxed)
            != self.sheet_open_epoch.load(Ordering::Relaxed)
        {
            self.sheet_buttons
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
        }
    }

    pub fn sheet_buttons_json(&self) -> Vec<serde_json::Value> {
        self.sheet_buttons
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|button| {
                serde_json::json!({
                    "label": button.label,
                    "bounds": button.bounds,
                })
            })
            .collect()
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
            match super::seal::append_audit(path, fact) {
                Ok(()) => {}
                Err(super::seal::SealError::Untrusted(reason)) => {
                    self.raise_integrity(&super::seal::IntegrityFault {
                        file: "permission-audit.jsonl".to_string(),
                        reason,
                    });
                    return Err("audit integrity check failed".to_string());
                }
                Err(super::seal::SealError::Io(reason)) => {
                    log::error!("permission_monitor: audit write {}: {reason}", path.display());
                    return Err(reason);
                }
            }
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

fn declared_capability_needs_click(capability_id: &str) -> bool {
    match crate::app::permissions::Capability::try_from(capability_id) {
        Ok(cap) => cap.is_sensitive(),
        Err(_) => crate::app::permissions::wasm_capability_requires_consent(capability_id),
    }
}

fn stored_capability_decision(
    store: &GrantStore,
    app_id: &str,
    workspace: &Path,
    capability_id: &str,
    now: i64,
) -> Option<Decision> {
    let mut allow = false;
    let mut ask = false;
    let mut deny = false;
    for record in store.records() {
        if record.actor_type != ActorType::App
            || record.actor_id != app_id
            || record.target_type != TargetType::Capability
            || record.target_id != capability_id
            || record.workspace_root.as_deref() != Some(workspace)
        {
            continue;
        }
        if record.expires_at.is_some_and(|expires| now >= expires) {
            continue;
        }
        match record.decision {
            Decision::Deny => deny = true,
            Decision::Ask => ask = true,
            Decision::Allow => allow = true,
        }
    }
    if deny {
        Some(Decision::Deny)
    } else if ask {
        Some(Decision::Ask)
    } else if allow {
        Some(Decision::Allow)
    } else {
        None
    }
}

fn record_is_durable(record: &GrantRecord) -> bool {
    record.source != GrantSource::Session
        && !matches!(
            record.duration,
            GrantDuration::Once | GrantDuration::Session
        )
}

fn snake<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .map(|value| match value {
            serde_json::Value::String(text) => text,
            other => other.to_string(),
        })
        .unwrap_or_default()
}

fn when_label(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .map(|stamp| stamp.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| unix.to_string())
}

fn duration_label(record: &GrantRecord) -> &'static str {
    if record.expires_at.is_some() {
        "until"
    } else {
        match record.duration {
            GrantDuration::Once => "once",
            GrantDuration::Session => "session",
            GrantDuration::Always => "always",
            _ => "scoped",
        }
    }
}

fn entry_from_record(record: &GrantRecord) -> PermissionEntry {
    let kind = record.decision.as_str().to_string();
    let duration = duration_label(record).to_string();
    let when = when_label(record.created_at);
    let workspace = record
        .workspace_root
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let summary = format!(
        "{} {kind} {duration} {} since {when}",
        record.actor_id, record.target_id
    );
    PermissionEntry {
        id: record.grant_id.clone(),
        kind,
        duration,
        actor_type: snake(&record.actor_type),
        actor_id: record.actor_id.clone(),
        tool: record.target_id.clone(),
        resource_id: record.resource_id.clone(),
        resource_scope: snake(&record.resource_scope),
        package_id: record.package_id.clone(),
        created_at: record.created_at,
        expires_at: record.expires_at,
        when,
        source: snake(&record.source),
        workspace,
        summary,
    }
}

fn entry_from_pending(row: &Pending) -> PermissionEntry {
    let when = when_label(row.created_at);
    let workspace = row.binding.workspace_root.display().to_string();
    let summary = format!(
        "{} ask {} since {when}",
        row.binding.actor_id, row.tool
    );
    PermissionEntry {
        id: row.id.clone(),
        kind: "pending".to_string(),
        duration: "ask".to_string(),
        actor_type: snake(&row.binding.actor_type),
        actor_id: row.binding.actor_id.clone(),
        tool: row.tool.clone(),
        resource_id: row.binding.resource_id.clone(),
        resource_scope: snake(&row.binding.resource_scope),
        package_id: row.binding.package_id.clone(),
        created_at: row.created_at,
        expires_at: None,
        when,
        source: "pending".to_string(),
        workspace,
        summary,
    }
}

fn split_widen_tag(tag: Option<&str>) -> (String, String) {
    let Some(tag) = tag else {
        return (String::new(), String::new());
    };
    let Some((action, entry_id)) = tag.split_once(':') else {
        return (String::new(), String::new());
    };
    if matches!(action, "reset" | "allow") && !entry_id.is_empty() {
        (entry_id.to_string(), action.to_string())
    } else {
        (String::new(), String::new())
    }
}

fn mutation_label(outcome: &PermissionMutation) -> &'static str {
    match outcome {
        PermissionMutation::Applied { .. } => "applied",
        PermissionMutation::NeedsYou { .. } => "needs_you",
        PermissionMutation::Missing { .. } => "missing",
        PermissionMutation::Rejected { .. } => "rejected",
    }
}

fn resolution_for_choice(choice: ApprovalChoice) -> NeedsYouResolution {
    match choice {
        ApprovalChoice::Deny | ApprovalChoice::DenyAlways => NeedsYouResolution::Denied,
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
        ApprovalChoice::DenyAlways => "deny_always",
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

fn pairing_view(code: &str) -> PendingView {
    PendingView {
        pending_request_id: code.to_string(),
        actor_id: "phone".to_string(),
        tool: "relay.pair".to_string(),
        resource_id: None,
        args_fingerprint: String::new(),
        input_summary: "confirm this phone".to_string(),
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
    if tool == "secret.read" {
        let name = parsed.as_ref().and_then(|value| value.get("name")).and_then(|item| item.as_str());
        let folder = parsed.as_ref().and_then(|value| value.get("folder")).and_then(|item| item.as_str());
        if let (Some(name), Some(folder)) = (name, folder) {
            return (ResourceScope::Path, Some(format!("{name}@{folder}")));
        }
    }
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
    let mut error = serde_json::json!({
        "code": code,
        "state": state,
        "pending_request_id": pending_request_id,
        "retry": retry,
    });
    if code == "permission_denied" {
        error["undo"] = serde_json::json!(crate::cli::introspect::permission_undo_text());
    }
    serde_json::json!({
        "schema_version": SCHEMA,
        "call_id": call_id,
        "error": error,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn pairing_code_is_listed_until_the_click_clears_it() {
        let monitor = PermissionMonitor::ephemeral();
        assert!(monitor.list_pending().is_empty());
        monitor.track_pairing_code("481516");
        monitor.track_pairing_code("481516");
        let listed = monitor.list_pending();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].pending_request_id, "481516");
        assert_eq!(listed[0].tool, "relay.pair");
        assert!(monitor.approve_pending("481516", ApprovalChoice::Once).is_err());
        assert_eq!(monitor.list_pending().len(), 1);
        monitor.clear_pairing_code("481516");
        assert!(monitor.list_pending().is_empty());
    }

    #[test]
    fn phone_may_approve_questions_and_blocked_runs_only() {
        assert!(!NeedsYouKind::ApprovalClick.phone_may_approve());
        assert!(NeedsYouKind::Question.phone_may_approve());
        assert!(NeedsYouKind::BlockedRun.phone_may_approve());
        let items = needs_you_phone_items(&[NeedsYouRecord {
            id: "ny-1".to_string(),
            kind: NeedsYouKind::ApprovalClick,
            actor: "agent:chess".to_string(),
            resource: "chess.play".to_string(),
            summary: "play e2e4".to_string(),
            created_at: 0,
            expires_at: None,
            run_tag: None,
            resolution: None,
        }]);
        assert_eq!(items[0]["phone_can_approve"], serde_json::json!(false));
    }

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
    fn deny_once_asks_again_and_always_deny_sticks_until_reset() {
        let monitor = PermissionMonitor::ephemeral();
        let args = r#"{"game_id":"game-1","move":"e2e4"}"#;
        let row = binding("ignored");
        let Admission::Required { pending_request_id } = admit_of(&monitor, &row, args) else {
            panic!("first call asks");
        };
        monitor
            .approve_pending(&pending_request_id, ApprovalChoice::Deny)
            .unwrap();
        assert!(
            monitor.list_entries().iter().all(|entry| entry.kind != "deny"),
            "a single deny stores nothing"
        );
        let Admission::Required { pending_request_id } = admit_of(&monitor, &row, args) else {
            panic!("deny once asks again");
        };
        monitor
            .approve_pending(&pending_request_id, ApprovalChoice::DenyAlways)
            .unwrap();
        let deny_id = monitor
            .list_entries()
            .into_iter()
            .find(|entry| entry.kind == "deny")
            .expect("always deny is listed")
            .id;
        assert!(
            matches!(admit_of(&monitor, &row, args), Admission::Denied { .. }),
            "always deny answers the next call"
        );
        let agent = PermissionCaller {
            human: false,
            actor_id: "agent:test".to_string(),
        };
        let filed = monitor.mutate_entry(&deny_id, "allow", &agent);
        assert!(
            matches!(filed, PermissionMutation::NeedsYou { .. }),
            "an agent cannot allow itself: {filed:?}"
        );
        assert!(monitor.list_entries().iter().any(|entry| entry.id == deny_id));
        let human = PermissionCaller {
            human: true,
            actor_id: "human".to_string(),
        };
        let reset = monitor.mutate_entry(&deny_id, "reset", &human);
        assert!(matches!(reset, PermissionMutation::Applied { .. }), "{reset:?}");
        assert!(
            matches!(admit_of(&monitor, &row, args), Admission::Required { .. }),
            "reset asks again"
        );
        let Admission::Required { pending_request_id } = admit_of(&monitor, &row, args) else {
            panic!("pending after reset");
        };
        // The previous required admission is still pending; approving always
        // on it creates the grant the revoke case removes.
        monitor
            .approve_pending(&pending_request_id, ApprovalChoice::Always)
            .unwrap();
        let grant_id = monitor
            .list_entries()
            .into_iter()
            .find(|entry| entry.kind == "allow")
            .expect("persistent grant is listed")
            .id;
        assert!(matches!(admit_of(&monitor, &row, args), Admission::Proceed { .. }));
        let revoked = monitor.mutate_entry(&grant_id, "revoke", &agent);
        assert!(matches!(revoked, PermissionMutation::Applied { .. }), "{revoked:?}");
        assert!(
            matches!(admit_of(&monitor, &row, args), Admission::Required { .. }),
            "revoke asks again"
        );
        let denied = structured_error("permission_denied", "call-1", None);
        assert!(denied.contains("plexi permissions list"), "{denied}");
        assert!(denied.contains("plexi permissions reset"), "{denied}");
        assert!(!denied.contains("gear"), "{denied}");
    }

    #[test]
    fn hand_edited_grant_file_does_not_grant() {
        let victim = tempfile::tempdir().unwrap();
        let donor = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let mut donor_store = GrantStore::load_or_default(donor.path());
        donor_store.record(GrantRecord::app_capability(
            "my-app",
            ws.path(),
            crate::app::permissions::Capability::NetHttp,
            Decision::Allow,
        ));
        donor_store.save();
        let empty = GrantStore::load_or_default(victim.path());
        empty.save();
        let signed = std::fs::read_to_string(donor.path().join("grants.toml")).unwrap();
        let sealed_empty = std::fs::read_to_string(victim.path().join("grants.toml")).unwrap();
        let body = signed
            .lines()
            .filter(|line| !line.starts_with("# plexi-mac:"))
            .collect::<Vec<_>>()
            .join("\n");
        let mac = sealed_empty
            .lines()
            .find(|line| line.starts_with("# plexi-mac:"))
            .expect("empty file is sealed");
        let forged = format!("{body}\n{mac}\n");
        assert!(
            forged.contains("allow"),
            "the forged file must actually add a grant: {forged}"
        );
        std::fs::write(victim.path().join("grants.toml"), &forged).unwrap();

        // A startup loader can quarantine the file before any monitor exists.
        // The later monitor must still fail closed and file Needs you.
        let preloaded = GrantStore::load_or_default(victim.path());
        assert!(preloaded.records().is_empty());
        assert!(
            !victim.path().join("grants.toml").exists(),
            "the startup load quarantines a forged grants file"
        );
        let monitor = PermissionMonitor::open_profile(victim.path());
        assert!(monitor.store().records().is_empty());
        let req = PermissionRequest::app_capability(
            "my-app",
            ws.path(),
            crate::app::permissions::Capability::NetHttp,
        );
        assert_eq!(monitor.store().evaluate(&req, None), Decision::Ask);
        let alerts = monitor.open_needs_you();
        assert!(
            alerts.iter().any(|row| row.kind == NeedsYouKind::Integrity),
            "{alerts:?}"
        );
        assert!(
            !victim.path().join("grants.toml").exists(),
            "a forged grants file is quarantined"
        );
    }

    #[test]
    fn edited_and_deleted_audit_lines_are_detected() {
        let edited = tempfile::tempdir().unwrap();
        let monitor = PermissionMonitor::open_profile(edited.path());
        monitor.note_denial("actor", "c1", "res", "op", "denied");
        monitor.note_denial("actor", "c2", "res", "op", "denied");
        drop(monitor);
        let path = edited.path().join("permission-audit.jsonl");
        let original = std::fs::read_to_string(&path).unwrap();
        let mut lines: Vec<String> = original.lines().map(str::to_string).collect();
        assert!(lines.len() >= 2, "{original}");
        lines[0] = lines[0].replace("denied", "allowed");
        std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
        let err = crate::broker::seal::verify_audit(&path).expect_err("an edited line must fail");
        assert!(err.contains("mac"), "{err}");
        let monitor = PermissionMonitor::open_profile(edited.path());
        assert!(
            monitor
                .open_needs_you()
                .iter()
                .any(|row| row.kind == NeedsYouKind::Integrity),
            "an edited audit line files needs you"
        );

        let deleted = tempfile::tempdir().unwrap();
        let monitor = PermissionMonitor::open_profile(deleted.path());
        monitor.note_denial("actor", "c1", "res", "op", "denied");
        monitor.note_denial("actor", "c2", "res", "op", "denied");
        drop(monitor);
        let path = deleted.path().join("permission-audit.jsonl");
        let original = std::fs::read_to_string(&path).unwrap();
        let first = original.lines().next().expect("first audit line");
        std::fs::write(&path, format!("{first}\n")).unwrap();
        let err = crate::broker::seal::verify_audit(&path).expect_err("a deleted line must fail");
        assert!(err.contains("tip"), "{err}");
        std::fs::remove_file(&path).unwrap();
        let err = crate::broker::seal::verify_audit(&path).expect_err("a deleted log must fail");
        assert!(err.contains("deleted"), "{err}");

        let monitor = PermissionMonitor::open_profile(deleted.path());
        assert!(
            monitor
                .open_needs_you()
                .iter()
                .any(|row| row.kind == NeedsYouKind::Integrity),
            "a deleted audit log files needs you"
        );
    }

    fn needs_you_decisions(monitor: &PermissionMonitor, decision: &str) -> usize {
        monitor
            .audit_records()
            .iter()
            .filter(|fact| fact.kind == "needs_you" && fact.decision == decision)
            .count()
    }
    #[test]
    fn client_resolve_is_refused_and_leaves_the_pending() {
        let monitor = PermissionMonitor::ephemeral();
        let args = r#"{"game_id":"game-1","move":"e2e4","expected_revision":0,"operation_id":"op-refuse"}"#;
        let fp = fingerprint_args(args).unwrap();
        let mut row = binding(&fp);
        row.session_id = Some(monitor.session_id().to_string());
        let id = match admit_of(&monitor, &row, args) {
            Admission::Required { pending_request_id } => pending_request_id,
            Admission::Proceed { .. } => panic!("expected a pending ask, grant matched"),
            Admission::Denied { code } => panic!("expected a pending ask, denied {code}"),
        };
        monitor.refuse_client_resolve(&id);
        assert!(monitor.show_pending(&id).is_some(), "the pending stays");
        let audit = monitor.audit_records();
        assert!(
            audit.iter().any(|fact| {
                fact.kind == "refuse"
                    && fact.decision == "refused_resolve"
                    && fact.call_id == id
            }),
            "refuse row missing: {audit:?}"
        );
        assert!(
            monitor.approve_pending(&id, ApprovalChoice::Once).is_ok(),
            "the desktop path can still approve"
        );
    }

    #[test]
    fn legacy_unsealed_grants_are_adopted_once() {
        crate::broker::host_key::without_mac_key(|| {
            let dir = tempfile::tempdir().unwrap();
            let ws = tempfile::tempdir().unwrap();
            let records = vec![
                GrantRecord::event_stream_allow(
                    ActorType::Agent,
                    "assistant",
                    ActorScope::User,
                    "chess::*",
                    ws.path(),
                    GrantDuration::Always,
                    GrantSource::User,
                    None,
                ),
                GrantRecord::event_stream_allow(
                    ActorType::Agent,
                    "agent:default",
                    ActorScope::User,
                    "host.events.subscribe",
                    ws.path(),
                    GrantDuration::Always,
                    GrantSource::User,
                    None,
                ),
            ];
            write_legacy_grants(
                dir.path(),
                records,
                "[[records]]\nactor_id = \"skipped\"\n",
            );
            write_legacy_permissions(dir.path(), ws.path());
            std::fs::write(
                dir.path().join("permission-audit.jsonl"),
                "{\"kind\":\"grant\",\"decision\":\"allow\"}\n",
            )
            .unwrap();

            let preloaded = GrantStore::load_or_default(dir.path());
            assert!(
                preloaded.integrity_faults().is_empty(),
                "{:?}",
                preloaded.integrity_faults()
            );
            assert_legacy_grants_kept(&preloaded);
            assert!(
                dir.path().join("grants.toml").is_file(),
                "adopt must not quarantine grants.toml"
            );
            let sealed = std::fs::read_to_string(dir.path().join("grants.toml")).unwrap();
            assert!(
                sealed.contains("# plexi-mac:"),
                "adopted grants must be sealed: {sealed}"
            );
            assert!(
                !dir.path().join("permissions.toml").is_file(),
                "a readable legacy permissions.toml is imported"
            );
            assert_no_untrusted(dir.path());
            crate::broker::seal::verify_audit(&dir.path().join("permission-audit.jsonl"))
                .expect("legacy audit is replaced with an authenticated tip");

            let monitor = PermissionMonitor::open_profile(dir.path());
            assert!(
                monitor.open_needs_you().is_empty(),
                "legacy adopt files no integrity item: {:?}",
                monitor.open_needs_you()
            );
            assert_legacy_grants_kept(&monitor.store());

            crate::broker::host_key::delete(crate::broker::host_key::MAC_ITEM).unwrap();
            strip_mac(&dir.path().join("grants.toml"));
            let tampered = PermissionMonitor::open_profile(dir.path());
            assert!(
                tampered.store().records().is_empty(),
                "seal marker and audit tip block adopt after the mac key is removed"
            );
            assert_grants_quarantined(dir.path(), &tampered);
            assert!(
                dir.path().join("permission-audit.jsonl").is_file(),
                "the authenticated audit stays in place"
            );
            crate::broker::seal::verify_audit(&dir.path().join("permission-audit.jsonl"))
                .expect("audit tip still matches");
        });
    }

    #[test]
    fn legacy_unsealed_grants_without_an_audit_log_are_adopted() {
        crate::broker::host_key::without_mac_key(|| {
            let dir = tempfile::tempdir().unwrap();
            let ws = tempfile::tempdir().unwrap();
            write_legacy_grants(
                dir.path(),
                vec![GrantRecord::app_capability(
                    "assistant",
                    ws.path(),
                    crate::app::permissions::Capability::FsRead,
                    Decision::Allow,
                )],
                "",
            );
            assert!(!dir.path().join("permission-audit.jsonl").exists());

            let monitor = PermissionMonitor::open_profile(dir.path());
            assert!(monitor.open_needs_you().is_empty(), "{:?}", monitor.open_needs_you());
            assert!(monitor.store().records().iter().any(|record| {
                record.actor_id == "assistant" && record.target_id == "fs.read"
            }));
            assert_no_untrusted(dir.path());
            let audit = dir.path().join("permission-audit.jsonl");
            assert!(audit.is_file(), "adopt starts an authenticated audit tip");
            crate::broker::seal::verify_audit(&audit).expect("new audit tip verifies");
        });
    }

    #[test]
    fn sealed_profile_rejects_stripped_or_corrupt_mac() {
        let stripped = tempfile::tempdir().unwrap();
        seal_one_grant(stripped.path());
        strip_mac(&stripped.path().join("grants.toml"));
        let monitor = PermissionMonitor::open_profile(stripped.path());
        assert!(monitor.store().records().is_empty());
        assert_grants_quarantined(stripped.path(), &monitor);
        assert_eq!(integrity_resources(&monitor), vec!["grants.toml".to_string()]);

        let corrupt = tempfile::tempdir().unwrap();
        seal_one_grant(corrupt.path());
        corrupt_mac(&corrupt.path().join("grants.toml"));
        let monitor = PermissionMonitor::open_profile(corrupt.path());
        assert!(monitor.store().records().is_empty());
        assert_grants_quarantined(corrupt.path(), &monitor);
        let resources = integrity_resources(&monitor);
        assert_eq!(resources, vec!["grants.toml".to_string()], "{resources:?}");
        assert!(
            monitor
                .open_needs_you()
                .iter()
                .any(|row| row.summary.contains("bad mac")),
            "{:?}",
            monitor.open_needs_you()
        );

        crate::broker::host_key::without_mac_key(|| {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(
                dir.path().join("grants.toml"),
                "records = []\n# plexi-mac:00\n",
            )
            .unwrap();
            let monitor = PermissionMonitor::open_profile(dir.path());
            assert!(
                monitor.store().records().is_empty(),
                "a mac line is not a pre-seal file"
            );
            assert_grants_quarantined(dir.path(), &monitor);
        });
    }

    fn write_legacy_grants(dir: &Path, records: Vec<GrantRecord>, extra: &str) {
        let mut body =
            toml::to_string_pretty(&super::super::GrantStoreData { records }).unwrap();
        if !extra.is_empty() {
            body.push('\n');
            body.push_str(extra);
            if !extra.ends_with('\n') {
                body.push('\n');
            }
        }
        assert!(
            !body.contains("# plexi-mac:"),
            "fixture must be unsealed: {body}"
        );
        std::fs::write(dir.join("grants.toml"), body).unwrap();
    }

    fn write_legacy_permissions(dir: &Path, workspace: &Path) {
        let key = format!("sample::{}::fs.write", workspace.display());
        let body = format!("[entries]\n\"{key}\" = \"green\"\n");
        std::fs::write(dir.join("permissions.toml"), body).unwrap();
    }

    fn assert_legacy_grants_kept(store: &GrantStore) {
        let records = store.records();
        assert!(
            records.iter().any(|record| {
                record.actor_id == "assistant" && record.target_id == "chess::*"
            }),
            "assistant chess grant dropped: {records:?}"
        );
        assert!(
            records.iter().any(|record| {
                record.actor_id == "agent:default" && record.target_id == "host.events.subscribe"
            }),
            "agent subscribe grant dropped: {records:?}"
        );
        assert!(
            records.iter().any(|record| {
                record.actor_id == "sample"
                    && record.target_id == "fs.write"
                    && record.decision == Decision::Allow
            }),
            "legacy permissions.toml entry was not imported: {records:?}"
        );
        assert!(
            records.iter().all(|record| record.actor_id != "skipped"),
            "malformed record was kept: {records:?}"
        );
    }

    fn seal_one_grant(dir: &Path) {
        let ws = tempfile::tempdir().unwrap();
        let mut store = GrantStore::load_or_default(dir);
        store.record(GrantRecord::app_capability(
            "my-app",
            ws.path(),
            crate::app::permissions::Capability::NetHttp,
            Decision::Allow,
        ));
        store.save();
        assert!(dir.join("grants.toml").is_file());
    }

    fn strip_mac(path: &Path) {
        let text = std::fs::read_to_string(path).unwrap();
        let body = text
            .lines()
            .filter(|line| !line.starts_with("# plexi-mac:"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(path, format!("{body}\n")).unwrap();
    }

    fn corrupt_mac(path: &Path) {
        let text = std::fs::read_to_string(path).unwrap();
        let body = text
            .lines()
            .filter(|line| !line.starts_with("# plexi-mac:"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(path, format!("{body}\n# plexi-mac:00\n")).unwrap();
    }

    fn assert_no_untrusted(dir: &Path) {
        let names = dir_names(dir);
        assert!(
            names.iter().all(|name| !name.contains(".untrusted-")),
            "{names:?}"
        );
    }

    fn assert_grants_quarantined(dir: &Path, monitor: &PermissionMonitor) {
        assert!(
            !dir.join("grants.toml").is_file(),
            "tampered grants.toml must be quarantined"
        );
        let names = dir_names(dir);
        assert!(
            names.iter().any(|name| name.starts_with("grants.toml.untrusted-")),
            "{names:?}"
        );
        assert!(
            monitor
                .open_needs_you()
                .iter()
                .any(|row| row.kind == NeedsYouKind::Integrity && row.resource == "grants.toml"),
            "{:?}",
            monitor.open_needs_you()
        );
    }

    fn integrity_resources(monitor: &PermissionMonitor) -> Vec<String> {
        monitor
            .open_needs_you()
            .into_iter()
            .filter(|row| row.kind == NeedsYouKind::Integrity)
            .map(|row| row.resource)
            .collect()
    }

    fn dir_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl PermissionMonitor {
    #[cfg(test)]
    fn clone_for_test(self: &Arc<Self>) -> Arc<Self> {
        Arc::clone(self)
    }
}
