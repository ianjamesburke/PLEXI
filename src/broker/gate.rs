//! Mandatory permission monitor for every tool dispatch.
//!
//! Constructors of `ToolDispatcher` take an `Arc<PermissionMonitor>`. Admission
//! compares the full exact binding (actor trust, workspace, resource, package,
//! instance, argument fingerprint, session, expiry). Schema-0 grants never
//! match. Revoke and commit share one admission lock.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use sha2::{Digest, Sha256};

use super::signoff::{self, PersonalSigner, SignoffParts, SignoffTier};
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct PendingView {
    pub pending_request_id: String,
    pub actor_id: String,
    pub tool: String,
    pub resource_id: Option<String>,
    pub args_fingerprint: String,
    pub input_summary: String,
    /// Set when the tool requires personal sign-off. A click cannot clear it.
    pub signoff_tier: Option<String>,
    pub signoff_label: Option<String>,
}

#[derive(Clone)]
struct Pending {
    id: String,
    binding: ExactBinding,
    tool: String,
    input_summary: String,
    signoff: Option<SignoffParts>,
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
    /// Tool → required personal-signoff tier. Entries are only raised, never lowered.
    requirements: Mutex<BTreeMap<String, SignoffTier>>,
    signer: Mutex<Arc<dyn PersonalSigner>>,
    /// call_id → grant_id for a signature that has been verified and not yet admitted.
    fresh: Mutex<BTreeMap<String, String>>,
    used_nonces: Mutex<BTreeMap<String, ()>>,
    time_boxed_ttl: AtomicI64,
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
        Arc::new(Self::from_arc(
            Arc::new(Mutex::new(GrantStore::default())),
            None,
            Arc::new(signoff::RefuseSigner),
        ))
    }

    fn open(dir: &Path) -> Self {
        let store = GrantStore::load_or_default(dir);
        let audit = dir.join("permission-audit.jsonl");
        let settings = signoff::SignoffSettings::load(dir);
        log::info!(
            "permission_monitor: opened profile {} audit {} personal_signoff_fallback={}",
            dir.display(),
            audit.display(),
            settings.fallback.as_str()
        );
        let monitor = Self::from_arc(
            Arc::new(Mutex::new(store)),
            Some(audit),
            Arc::new(signoff::RefuseSigner),
        );
        monitor.install_signer(signoff::production_signer(dir, settings));
        monitor
            .time_boxed_ttl
            .store(settings.time_boxed_ttl_secs, Ordering::SeqCst);
        monitor
    }

    fn from_arc(
        store: Arc<Mutex<GrantStore>>,
        audit_path: Option<PathBuf>,
        signer: Arc<dyn PersonalSigner>,
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
            audit_mem: Mutex::new(Vec::new()),
            fail_audit: AtomicBool::new(false),
            requirements: Mutex::new(BTreeMap::new()),
            signer: Mutex::new(signer),
            fresh: Mutex::new(BTreeMap::new()),
            used_nonces: Mutex::new(BTreeMap::new()),
            time_boxed_ttl: AtomicI64::new(signoff::TIME_BOXED_TTL_SECS),
            #[cfg(test)]
            now_override: AtomicI64::new(0),
        }
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
        self.absorb_grant_markers(req.tool);
        let tier = self
            .requirements
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(req.tool)
            .copied();
        let decision = self.store().evaluate(&request, None);
        log::info!(
            "permission_monitor: admit actor={} tool={} resource={:?} call_id={} -> {} signoff={}",
            binding.actor_id,
            binding.target_id,
            binding.resource_id,
            binding.call_id,
            decision.as_str(),
            tier.map(SignoffTier::as_str).unwrap_or("click")
        );
        if let Some(tier) = tier {
            if decision == Decision::Deny {
                self.note_denial(
                    &binding.actor_id,
                    &binding.call_id,
                    binding.resource_id.as_deref().unwrap_or(""),
                    &binding.operation_id,
                    "deny",
                );
                return Admission::Denied {
                    code: "permission_denied",
                };
            }
            return self.admit_signoff(req, &binding, tier);
        }
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
        if let Some(existing) = pending.iter().find(|row| same_pending(row, binding)) {
            log::info!(
                "permission_monitor: reuse pending {} for actor={} tool={}",
                existing.id,
                binding.actor_id,
                tool
            );
            return existing.id.clone();
        }
        let id = format!("req_{}", uuid::Uuid::new_v4());
        log::info!(
            "permission_monitor: pending {id} actor={} tool={} resource={:?}",
            binding.actor_id,
            tool,
            binding.resource_id
        );
        pending.push(Pending {
            id: id.clone(),
            binding: binding.clone(),
            tool: tool.to_string(),
            input_summary: summarize(input_json),
            signoff: None,
        });
        id
    }

    /// Record a manifest or grant requirement. A later call cannot lower the tier.
    pub fn note_manifest(&self, tool: &str, tier: SignoffTier) {
        self.raise_requirement(tool, tier, "manifest");
    }

    pub fn install_signer(&self, signer: Arc<dyn PersonalSigner>) {
        let label = signer.label();
        log::info!("personal_signoff: installed signer {label}");
        *self.signer.lock().unwrap_or_else(|e| e.into_inner()) = signer;
    }

    #[cfg(test)]
    pub fn set_now_for_test(&self, unix_secs: i64) {
        self.now_override.store(unix_secs, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub fn set_time_boxed_ttl_for_test(&self, secs: i64) {
        self.time_boxed_ttl.store(secs, Ordering::SeqCst);
    }

    pub fn challenge_message(&self, pending_id: &str) -> Option<String> {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|row| row.id == pending_id)
            .and_then(|row| row.signoff.as_ref().map(signoff::canonical_message))
    }

    pub fn pending_requires_signoff(&self, pending_id: &str) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|row| row.id == pending_id && row.signoff.is_some())
    }

    /// Prompt the installed signer and admit the signature. A missing or
    /// invalid signature is refused and audited.
    pub fn sign_pending(&self, pending_id: &str) -> Result<String, String> {
        let (reason, label) = {
            let rows = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            let Some(row) = rows.iter().find(|row| row.id == pending_id) else {
                return Err(format!("unknown pending request {pending_id}"));
            };
            let Some(parts) = row.signoff.as_ref() else {
                return Err(format!("pending {pending_id} is not a personal sign-off"));
            };
            let label = self
                .signer
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .label();
            (
                format!(
                    "Personal sign-off ({label}) for {} on {}",
                    parts.action,
                    parts.resource_id.as_deref().unwrap_or("the workspace")
                ),
                label,
            )
        };
        let message = self.challenge_message(pending_id).ok_or_else(|| {
            format!("pending {pending_id} lost its personal sign-off challenge")
        })?;
        log::info!("personal_signoff: prompting {label} for pending {pending_id}");
        let signature = {
            let signer = self.signer.lock().unwrap_or_else(|e| e.into_inner());
            match signer.sign(message.as_bytes(), &reason) {
                Ok(signature) => signature,
                Err(error) => {
                    drop(signer);
                    self.audit_signoff_refusal(pending_id, "personal_signoff_refused", &error);
                    return Err(error);
                }
            }
        };
        self.submit_signoff(pending_id, &signature)
    }

    /// Verify a signature for a pending personal sign-off. Click approval
    /// cannot be used instead.
    pub fn submit_signoff(&self, pending_id: &str, signature: &[u8]) -> Result<String, String> {
        let pending = {
            let rows = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            rows.iter().find(|row| row.id == pending_id).cloned()
        };
        let Some(pending) = pending else {
            return Err(format!("unknown pending request {pending_id}"));
        };
        let Some(parts) = pending.signoff.clone() else {
            return Err(format!(
                "pending {pending_id} is a click approval, not a personal sign-off"
            ));
        };
        let now = self.now_secs();
        if now >= parts.deadline || now >= parts.expiry {
            self.audit_signoff_refusal(pending_id, "personal_signoff_expired", "challenge expired");
            return Err("personal sign-off challenge expired".to_string());
        }
        if self
            .used_nonces
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&parts.nonce)
        {
            self.audit_signoff_refusal(pending_id, "personal_signoff_replay", "nonce reused");
            return Err("personal sign-off nonce was already used".to_string());
        }
        let (verified, mechanism) = {
            let signer = self.signer.lock().unwrap_or_else(|e| e.into_inner());
            let mechanism = signer.mechanism_name().to_string();
            let verified = signer.verify(signoff::canonical_message(&parts).as_bytes(), signature);
            (verified, mechanism)
        };
        if !verified {
            self.audit_signoff_refusal(
                pending_id,
                "personal_signoff_invalid",
                "signature did not verify",
            );
            return Err("personal sign-off signature did not verify".to_string());
        }
        let grant_id = format!("grant_{}", uuid::Uuid::new_v4());
        let (duration, source) = match parts.tier {
            SignoffTier::EachTime => (GrantDuration::Once, GrantSource::Session),
            SignoffTier::TimeBoxed => (GrantDuration::Always, GrantSource::User),
        };
        let mut record = GrantRecord::from_binding(
            &pending.binding,
            Decision::Allow,
            duration,
            source,
            &grant_id,
        );
        record.expires_at = Some(parts.expiry);
        record.signoff_tier = parts.tier.as_str().to_string();
        record.signoff_signature = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            signature,
        );
        record.signoff_mechanism = mechanism.clone();
        record.signoff_nonce = parts.nonce.clone();
        record.signoff_deadline = Some(parts.deadline);
        if parts.tier == SignoffTier::EachTime {
            record.consumed = true;
            record.bound_operation_id = Some(pending.binding.operation_id.clone());
        }
        let rebuilt = signoff::canonical_message(&signoff_parts_of(&record));
        if rebuilt != signoff::canonical_message(&parts) {
            self.audit_signoff_refusal(
                pending_id,
                "personal_signoff_invalid",
                "rebuilt message diverged",
            );
            return Err("personal sign-off message did not round-trip".to_string());
        }
        if parts.tier == SignoffTier::EachTime {
            self.fresh
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(pending.binding.call_id.clone(), grant_id.clone());
        }
        self.used_nonces
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(parts.nonce.clone(), ());
        self.store().record(record);
        if parts.tier == SignoffTier::TimeBoxed {
            self.store().save();
        }
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|row| row.id != pending_id);
        let _ = self.audit(&AuditFact {
            kind: "grant".to_string(),
            actor: pending.binding.actor_id.clone(),
            call_id: pending.binding.call_id.clone(),
            grant_id: grant_id.clone(),
            resource_id: pending.binding.resource_id.clone().unwrap_or_default(),
            args_fingerprint: pending.binding.args_fingerprint.clone(),
            operation_id: pending.binding.operation_id.clone(),
            decision: format!("personal_signoff:{}:{mechanism}", parts.tier.as_str()),
            revision_before: String::new(),
            revision_after: String::new(),
        });
        log::info!(
            "personal_signoff: verified {} for {} as {grant_id} via {mechanism}",
            parts.tier.as_str(),
            pending.binding.target_id
        );
        Ok(grant_id)
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

    fn raise_requirement(&self, tool: &str, tier: SignoffTier, source: &str) {
        let mut map = self.requirements.lock().unwrap_or_else(|e| e.into_inner());
        let next = map
            .get(tool)
            .copied()
            .map(|existing| existing.stricter(tier))
            .unwrap_or(tier);
        if map.get(tool).copied() == Some(next) {
            return;
        }
        map.insert(tool.to_string(), next);
        log::info!(
            "personal_signoff: {tool} requires {} ({source})",
            next.as_str()
        );
    }

    fn absorb_grant_markers(&self, tool: &str) {
        let tiers: Vec<SignoffTier> = self
            .store()
            .records()
            .iter()
            .filter(|record| record.requires_personal_signoff && record.target_id == tool)
            .map(|record| SignoffTier::parse(&record.signoff_tier))
            .collect();
        for tier in tiers {
            self.raise_requirement(tool, tier, "grant");
        }
    }

    fn signature_ok(&self, record: &GrantRecord) -> bool {
        let Ok(signature) = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            record.signoff_signature.as_bytes(),
        ) else {
            return false;
        };
        let message = signoff::canonical_message(&signoff_parts_of(record));
        self.signer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .verify_mechanism(&record.signoff_mechanism, message.as_bytes(), &signature)
    }

    fn find_time_boxed(&self, binding: &ExactBinding) -> Option<GrantRecord> {
        let now = self.now_secs();
        let workspace = crate::platform::path::canonical_or_self(&binding.workspace_root);
        self.store()
            .records()
            .iter()
            .find(|record| {
                record.signoff_tier == SignoffTier::TimeBoxed.as_str()
                    && record.decision == Decision::Allow
                    && !record.signoff_signature.is_empty()
                    && record.actor_type == binding.actor_type
                    && record.actor_id == binding.actor_id
                    && record.actor_scope == binding.actor_scope
                    && record.trust_origin == binding.trust_origin
                    && record.target_type == binding.target_type
                    && record.target_id == binding.target_id
                    && record.resource_scope == binding.resource_scope
                    && record.resource_id == binding.resource_id
                    && record.package_id == binding.package_id
                    && record.workspace_root.as_ref() == Some(&workspace)
                    && record.expires_at.is_some_and(|expiry| now < expiry)
            })
            .cloned()
    }

    fn admit_signoff(
        &self,
        req: AdmitRequest<'_>,
        binding: &ExactBinding,
        tier: SignoffTier,
    ) -> Admission {
        if tier == SignoffTier::TimeBoxed {
            if let Some(grant) = self.find_time_boxed(binding) {
                if self.signature_ok(&grant) {
                    log::info!(
                        "personal_signoff: time-boxed grant {} covers actor={} tool={}",
                        grant.grant_id,
                        binding.actor_id,
                        binding.target_id
                    );
                    return Admission::Proceed {
                        grant_id: grant.grant_id,
                        fingerprint: binding.args_fingerprint.clone(),
                        resource_scope: binding.resource_scope,
                        resource_id: binding.resource_id.clone(),
                    };
                }
                self.note_denial(
                    &binding.actor_id,
                    &binding.call_id,
                    binding.resource_id.as_deref().unwrap_or(""),
                    &binding.operation_id,
                    "personal_signoff_invalid",
                );
                return Admission::Denied {
                    code: "permission_denied",
                };
            }
        }
        let fresh_id = self
            .fresh
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&binding.call_id);
        if let Some(grant_id) = fresh_id {
            let ok = self
                .store()
                .records()
                .iter()
                .find(|record| record.grant_id == grant_id)
                .is_some_and(|record| self.signature_ok(record));
            if ok {
                log::info!(
                    "personal_signoff: fresh signature grant {grant_id} admits call {}",
                    binding.call_id
                );
                return Admission::Proceed {
                    grant_id,
                    fingerprint: binding.args_fingerprint.clone(),
                    resource_scope: binding.resource_scope,
                    resource_id: binding.resource_id.clone(),
                };
            }
            self.note_denial(
                &binding.actor_id,
                &binding.call_id,
                binding.resource_id.as_deref().unwrap_or(""),
                &binding.operation_id,
                "personal_signoff_invalid",
            );
            return Admission::Denied {
                code: "permission_denied",
            };
        }
        let id = self.persist_signoff_pending(binding, req.tool, req.input_json, tier);
        let label = self
            .signer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .label()
            .to_string();
        let _ = self.audit(&AuditFact {
            kind: "ask".to_string(),
            actor: binding.actor_id.clone(),
            call_id: binding.call_id.clone(),
            grant_id: String::new(),
            resource_id: binding.resource_id.clone().unwrap_or_default(),
            args_fingerprint: binding.args_fingerprint.clone(),
            operation_id: binding.operation_id.clone(),
            decision: format!("personal_signoff:{}:{label}", tier.as_str()),
            revision_before: String::new(),
            revision_after: String::new(),
        });
        Admission::Required {
            pending_request_id: id,
        }
    }

    fn persist_signoff_pending(
        &self,
        binding: &ExactBinding,
        tool: &str,
        input_json: &str,
        tier: SignoffTier,
    ) -> String {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = pending.iter().find(|row| {
            same_pending(row, binding)
                && row
                    .signoff
                    .as_ref()
                    .is_some_and(|parts| parts.tier.stricter(tier) == parts.tier)
        }) {
            return existing.id.clone();
        }
        pending.retain(|row| !(same_pending(row, binding) && row.signoff.is_some()));
        let now = self.now_secs();
        let deadline = now + signoff::challenge_ttl_secs();
        let expiry = match tier {
            SignoffTier::EachTime => deadline,
            SignoffTier::TimeBoxed => now + self.time_boxed_ttl.load(Ordering::SeqCst),
        };
        let parts = SignoffParts {
            actor_type: binding.actor_type,
            actor_scope: binding.actor_scope,
            trust_origin: binding.trust_origin.clone(),
            actor_id: binding.actor_id.clone(),
            resource_scope: binding.resource_scope,
            resource_id: binding.resource_id.clone(),
            action: binding.target_id.clone(),
            args_fingerprint: binding.args_fingerprint.clone(),
            nonce: format!("nonce-{}", uuid::Uuid::new_v4()),
            expiry,
            deadline,
            package: binding.package_id.clone(),
            tier,
        };
        let id = format!("req_{}", uuid::Uuid::new_v4());
        log::info!(
            "personal_signoff: challenge {id} tier={} tool={tool} actor={}",
            tier.as_str(),
            binding.actor_id
        );
        pending.push(Pending {
            id: id.clone(),
            binding: binding.clone(),
            tool: tool.to_string(),
            input_summary: summarize(input_json),
            signoff: Some(parts),
        });
        id
    }

    fn audit_signoff_refusal(&self, pending_id: &str, decision: &str, detail: &str) {
        let (actor, call_id, resource, fingerprint, operation) = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|row| row.id == pending_id)
            .map(|row| {
                (
                    row.binding.actor_id.clone(),
                    row.binding.call_id.clone(),
                    row.binding.resource_id.clone().unwrap_or_default(),
                    row.binding.args_fingerprint.clone(),
                    row.binding.operation_id.clone(),
                )
            })
            .unwrap_or_default();
        log::info!("personal_signoff: {decision} pending={pending_id} {detail}");
        let _ = self.audit(&AuditFact {
            kind: "deny".to_string(),
            actor,
            call_id,
            grant_id: String::new(),
            resource_id: resource,
            args_fingerprint: fingerprint,
            operation_id: operation,
            decision: decision.to_string(),
            revision_before: String::new(),
            revision_after: String::new(),
        });
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
        if pending.signoff.is_some() && choice != ApprovalChoice::Deny {
            self.audit_signoff_refusal(
                pending_id,
                "personal_signoff_click_refused",
                "a click cannot satisfy personal sign-off",
            );
            return Err(
                "personal sign-off cannot be satisfied by a click".to_string(),
            );
        }
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
        Ok(())
    }

    /// Record use before execution. Failure denies the call. Revoke that wins
    /// the admission lock first makes this return `Err`.
    pub fn note_use(&self, actor: &str, call_id: &str, grant_id: &str, fingerprint: &str, resource: &str, operation_id: &str) -> Result<(), String> {
        let _admission = self.admission.lock().unwrap_or_else(|e| e.into_inner());
        let epoch = self.epoch.load(Ordering::SeqCst);
        if epoch == 0 {
            return Err("permission monitor epoch is unset".to_string());
        }
        let signed = self
            .store()
            .records()
            .iter()
            .find(|record| record.grant_id == grant_id && !record.signoff_signature.is_empty())
            .cloned();
        if let Some(record) = signed {
            if !self.signature_ok(&record) {
                let _ = self.audit_locked(&AuditFact {
                    kind: "deny".to_string(),
                    actor: actor.to_string(),
                    call_id: call_id.to_string(),
                    grant_id: grant_id.to_string(),
                    resource_id: resource.to_string(),
                    args_fingerprint: fingerprint.to_string(),
                    operation_id: operation_id.to_string(),
                    decision: "personal_signoff_invalid".to_string(),
                    revision_before: String::new(),
                    revision_after: String::new(),
                });
                log::info!(
                    "personal_signoff: refused use of {grant_id}; signature did not verify"
                );
                return Err("personal sign-off signature did not verify".to_string());
            }
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
        signoff_tier: row.signoff.as_ref().map(|parts| parts.tier.as_str().to_string()),
        signoff_label: row.signoff.as_ref().map(|_| "personal_signoff".to_string()),
    }
}

fn signoff_parts_of(record: &GrantRecord) -> SignoffParts {
    SignoffParts {
        actor_type: record.actor_type,
        actor_scope: record.actor_scope,
        trust_origin: record.trust_origin.clone(),
        actor_id: record.actor_id.clone(),
        resource_scope: record.resource_scope,
        resource_id: record.resource_id.clone(),
        action: record.target_id.clone(),
        args_fingerprint: record.args_fingerprint.clone(),
        nonce: record.signoff_nonce.clone(),
        expiry: record.expires_at.unwrap_or(0),
        deadline: record.signoff_deadline.unwrap_or(0),
        package: record.package_id.clone(),
        tier: SignoffTier::parse(&record.signoff_tier),
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

    fn signed_monitor() -> Arc<PermissionMonitor> {
        let monitor = PermissionMonitor::ephemeral();
        monitor.install_signer(Arc::new(signoff::MockSigner::new()));
        monitor.set_now_for_test(1_700_000_000);
        monitor
    }

    fn vault_binding(monitor: &PermissionMonitor, args: &str) -> ExactBinding {
        let mut row = binding(&fingerprint_args(args).unwrap());
        row.target_id = "vault.export".into();
        row.resource_scope = ResourceScope::Workspace;
        row.resource_id = None;
        row.package_id = "vault".into();
        row.session_id = Some(monitor.session_id().to_string());
        row
    }

    fn pending_id(admission: Admission) -> String {
        match admission {
            Admission::Required { pending_request_id } => pending_request_id,
            Admission::Proceed { .. } => panic!("expected a personal sign-off challenge"),
            Admission::Denied { code } => panic!("expected a challenge, got {code}"),
        }
    }

    fn has_decision(monitor: &PermissionMonitor, decision: &str) -> bool {
        monitor
            .audit_records()
            .iter()
            .any(|fact| fact.decision == decision)
    }

    #[test]
    fn click_cannot_satisfy_personal_signoff() {
        let monitor = signed_monitor();
        monitor.note_manifest("vault.export", SignoffTier::EachTime);
        let args = r#"{"amount":1}"#;
        let base = vault_binding(&monitor, args);
        let mut allow = GrantRecord::from_binding(
            &base,
            Decision::Allow,
            GrantDuration::Always,
            GrantSource::User,
            "click-grant",
        );
        allow.expires_at = Some(1_700_000_000 + 3600);
        monitor.store().record(allow);
        let id = pending_id(admit_of(&monitor, &base, args));
        assert!(monitor.pending_requires_signoff(&id));
        let err = monitor
            .approve_pending(&id, ApprovalChoice::Once)
            .unwrap_err();
        assert!(err.contains("cannot be satisfied by a click"));
        assert!(monitor.pending_requires_signoff(&id));
        assert!(has_decision(&monitor, "personal_signoff_click_refused"));
        let view = monitor
            .list_pending()
            .into_iter()
            .find(|row| row.pending_request_id == id)
            .unwrap();
        assert_eq!(view.signoff_tier.as_deref(), Some("each_time"));
        assert_ne!(view.signoff_label.as_deref(), Some("Touch ID"));
    }

    #[test]
    fn each_time_signature_admits_once_and_rejects_replay() {
        let monitor = signed_monitor();
        monitor.note_manifest("vault.export", SignoffTier::EachTime);
        let args = r#"{"amount":1}"#;
        let base = vault_binding(&monitor, args);
        let id = pending_id(admit_of(&monitor, &base, args));
        let message = monitor.challenge_message(&id).expect("challenge");
        assert!(message.contains("tier=each_time"));
        assert!(message.contains("actor=agent:user:host:agent:chess"));
        assert!(message.contains("nonce="));
        monitor.sign_pending(&id).expect("mock signature");
        let Admission::Proceed { grant_id, .. } = admit_of(&monitor, &base, args) else {
            panic!("a fresh signature admits the call once");
        };
        monitor
            .note_use(
                &base.actor_id,
                &base.call_id,
                &grant_id,
                &base.args_fingerprint,
                "",
                &base.operation_id,
            )
            .expect("verified signature may be used");
        let again = pending_id(admit_of(&monitor, &base, args));
        let stale = signoff::MockSigner::new()
            .sign(message.as_bytes(), "replay")
            .unwrap();
        let err = monitor.submit_signoff(&again, &stale).unwrap_err();
        assert!(err.contains("did not verify"));
        assert!(has_decision(&monitor, "personal_signoff_invalid"));
    }

    #[test]
    fn time_boxed_grant_covers_new_args_until_revoke_or_tamper() {
        let monitor = signed_monitor();
        monitor.set_time_boxed_ttl_for_test(7 * 24 * 60 * 60);
        monitor.note_manifest("vault.export", SignoffTier::TimeBoxed);
        let first = r#"{"amount":1}"#;
        let second = r#"{"amount":2}"#;
        let base = vault_binding(&monitor, first);
        let id = pending_id(admit_of(&monitor, &base, first));
        let grant_id = monitor.sign_pending(&id).unwrap();
        let mut other = base.clone();
        other.call_id = "call-b".into();
        match admit_of(&monitor, &other, second) {
            Admission::Proceed { grant_id: covered, .. } => assert_eq!(covered, grant_id),
            Admission::Required { .. } => panic!("time-boxed grant must cover different arguments"),
            Admission::Denied { code } => panic!("time-boxed grant was denied: {code}"),
        }
        assert!(monitor.revoke_grant_id(&grant_id));
        let id = pending_id(admit_of(&monitor, &base, first));
        let grant_id = monitor.sign_pending(&id).unwrap();
        {
            let mut store = monitor.store();
            let record = store
                .records_mut()
                .iter_mut()
                .find(|record| record.grant_id == grant_id)
                .unwrap();
            record.expires_at = Some(record.expires_at.unwrap() - 1);
        }
        match admit_of(&monitor, &other, second) {
            Admission::Denied { code } => assert_eq!(code, "permission_denied"),
            Admission::Proceed { .. } => panic!("a tampered expiry must not authorize"),
            Admission::Required { .. } => panic!("a tampered covering grant is a denial, not a new challenge"),
        }
        assert!(has_decision(&monitor, "personal_signoff_invalid"));
    }

    #[test]
    fn grant_marker_forces_signoff_and_cannot_lower_each_time() {
        let monitor = signed_monitor();
        monitor.store().record(GrantRecord {
            target_id: "vault.export".into(),
            requires_personal_signoff: true,
            signoff_tier: "time_boxed".into(),
            ..GrantRecord::unbound()
        });
        let args = r#"{"amount":1}"#;
        let base = vault_binding(&monitor, args);
        let id = pending_id(admit_of(&monitor, &base, args));
        let message = monitor.challenge_message(&id).unwrap();
        assert!(message.contains("tier=time_boxed"), "{message}");
        monitor.note_manifest("vault.export", SignoffTier::EachTime);
        let raised = pending_id(admit_of(&monitor, &base, args));
        let raised_message = monitor.challenge_message(&raised).unwrap();
        assert!(raised_message.contains("tier=each_time"), "{raised_message}");
        assert_ne!(id, raised);
    }

    #[test]
    fn matching_deny_wins_over_personal_signoff() {
        let monitor = signed_monitor();
        monitor.note_manifest("vault.export", SignoffTier::EachTime);
        let args = r#"{"amount":1}"#;
        let base = vault_binding(&monitor, args);
        monitor.store().record(GrantRecord::from_binding(
            &base,
            Decision::Deny,
            GrantDuration::Always,
            GrantSource::User,
            "deny-vault",
        ));
        match admit_of(&monitor, &base, args) {
            Admission::Denied { code } => assert_eq!(code, "permission_denied"),
            Admission::Required { .. } => panic!("a matching deny must not open a sign-off challenge"),
            Admission::Proceed { .. } => panic!("a matching deny must not proceed"),
        }
    }

    #[test]
    fn refuse_signer_blocks_the_call_and_is_not_touch_id() {
        let monitor = PermissionMonitor::ephemeral();
        monitor.set_now_for_test(1_700_000_000);
        monitor.note_manifest("vault.export", SignoffTier::EachTime);
        let args = r#"{"amount":1}"#;
        let base = vault_binding(&monitor, args);
        let id = pending_id(admit_of(&monitor, &base, args));
        let err = monitor.sign_pending(&id).unwrap_err();
        assert!(err.contains("not Touch ID"), "{err}");
        assert!(!err.contains("touch_id"), "{err}");
        assert!(has_decision(&monitor, "personal_signoff_refused"));
        assert!(monitor.pending_requires_signoff(&id));
    }
}

impl PermissionMonitor {
    #[cfg(test)]
    fn clone_for_test(self: &Arc<Self>) -> Arc<Self> {
        Arc::clone(self)
    }
}
