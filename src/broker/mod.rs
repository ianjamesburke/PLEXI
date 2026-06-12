//! Unified permissions broker — `docs/prm/permissions-broker.md`.
//!
//! Generalizes the app capability permission model (`app_id::workspace::cap`
//! → Green/Yellow/Red) into one host broker that evaluates
//! `actor -> action -> target -> scope -> decision` for apps, agents, the
//! system, and managed policy.
//!
//! Phase A scope: grant model + persistence + legacy migration + the PGAP app
//! capability path. Event streams, undo, model routing enforcement, and the
//! `/permissions` UI surface come later; their target types exist as data only.

use crate::app::permissions::{Capability, PermissionState, PermissionStore};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

// ── Core enums ────────────────────────────────────────────────────────────────

/// Who is asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorType {
    App,
    Agent,
    System,
    ManagedPolicy,
}

/// What kind of thing is being accessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetType {
    Capability,
    HostTool,
    AppConnector,
    AppEventStream,
    McpTool,
    FileScope,
    Secret,
    Package,
    ModelPointer,
    UndoCheckpoint,
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

/// Where a grant came from. Managed grants cannot be overridden by any other
/// source — they are evaluated first at each decision tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantSource {
    Managed,
    User,
    Workspace,
    Session,
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
/// `app_id::workspace::capability → state` entry.
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
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds; `None` = no expiry.
    #[serde(default)]
    pub expires_at: Option<i64>,
}

impl GrantRecord {
    /// Build the record an app capability decision produces — the migrated
    /// shape from the spec's migration table.
    pub fn app_capability(
        app_id: &str,
        workspace_root: &Path,
        cap: Capability,
        decision: Decision,
    ) -> Self {
        Self {
            actor_type: ActorType::App,
            actor_id: app_id.to_string(),
            actor_scope: ActorScope::User,
            workspace_root: Some(canonical(workspace_root)),
            target_type: TargetType::Capability,
            target_id: cap.as_str().to_string(),
            resource_scope: ResourceScope::Workspace,
            resource_id: None,
            decision,
            duration: GrantDuration::Always,
            source: GrantSource::User,
            created_at: now_unix(),
            expires_at: None,
        }
    }

    /// True when this record answers the given request (identity match — the
    /// decision tiers are applied by `GrantStore::evaluate`).
    fn matches(&self, req: &PermissionRequest, now: i64) -> bool {
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
            (None, _) => true, // global grant matches any workspace
            (Some(_), None) => false,
            (Some(g), Some(r)) => g == r,
        }
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
    /// Canonicalized workspace the action happens in; `None` = global action.
    pub workspace_root: Option<PathBuf>,
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
            workspace_root: workspace_root.map(canonical),
        }
    }

    /// The PGAP app capability check shape.
    pub fn app_capability(app_id: &str, workspace_root: &Path, cap: Capability) -> Self {
        Self::new(
            ActorType::App,
            app_id,
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
#[derive(Debug, Clone, PartialEq)]
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
    /// Load the user-level posture from `<config_dir>/config.toml`.
    /// Returns `None` when the file or its `[permissions]` table is absent
    /// (the posture is optional). A present-but-invalid table is logged
    /// loudly and treated as absent so a config typo cannot silently widen
    /// permissions (the broker then falls back to Ask).
    pub fn load_from_config(config_dir: &Path) -> Option<Self> {
        let path = config_dir.join("config.toml");
        let raw = std::fs::read_to_string(&path).ok()?;
        let has_table = toml::from_str::<toml::Value>(&raw)
            .ok()
            .is_some_and(|v| v.get("permissions").is_some());
        if !has_table {
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
        let parsed: PostureToml = toml::from_str(raw)
            .map_err(|e| format!("invalid [permissions] table: {e}"))?;
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

// ── GrantStore ────────────────────────────────────────────────────────────────

/// On-disk shape of `grants.toml`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct GrantStoreData {
    #[serde(default)]
    records: Vec<GrantRecord>,
}

/// Loads, evaluates, and persists generalized grants in
/// `<config_dir>/grants.toml`. Legacy `permissions.toml` entries are migrated
/// in on load (loss-free, idempotent); the legacy store keeps working in
/// parallel until every call site is switched.
#[derive(Debug, Default)]
pub struct GrantStore {
    data: GrantStoreData,
    path: PathBuf,
}

impl GrantStore {
    /// Load `grants.toml` from the channel config dir and migrate any legacy
    /// `permissions.toml` entries not yet represented. On parse failure the
    /// corrupt file is backed up and an empty store returned (fail open to
    /// empty, never crash the host).
    pub fn load_or_default(config_dir: &Path) -> Self {
        let path = config_dir.join("grants.toml");
        let mut store = match std::fs::read_to_string(&path) {
            Err(_) => Self {
                data: GrantStoreData::default(),
                path: path.clone(),
            },
            Ok(raw) => match toml::from_str::<GrantStoreData>(&raw) {
                Ok(data) => {
                    log::info!(
                        "grant_store: loaded {} records from {}",
                        data.records.len(),
                        path.display()
                    );
                    Self {
                        data,
                        path: path.clone(),
                    }
                }
                Err(e) => {
                    let ts = now_unix();
                    let backup = path.with_file_name(format!("grants.toml.corrupt-{ts}"));
                    log::error!(
                        "grant_store: failed to parse {}: {e} — backing up to {}",
                        path.display(),
                        backup.display()
                    );
                    if let Err(rename_err) = std::fs::rename(&path, &backup) {
                        log::error!(
                            "grant_store: could not rename corrupt file to {}: {rename_err}",
                            backup.display()
                        );
                    }
                    Self {
                        data: GrantStoreData::default(),
                        path: path.clone(),
                    }
                }
            },
        };

        let legacy = PermissionStore::load_or_default(config_dir);
        let migrated = store.migrate_legacy(&legacy);
        if migrated > 0 {
            log::info!(
                "grant_store: migrated {migrated} legacy permissions.toml entries into {}",
                path.display()
            );
            store.save();
        }
        store
    }

    /// Import legacy `app_id::workspace::capability → state` entries as
    /// generalized records. Loss-free (every parseable entry maps) and
    /// idempotent (entries already represented are skipped). Returns the
    /// number of records added. The legacy store is never mutated.
    pub fn migrate_legacy(&mut self, legacy: &PermissionStore) -> usize {
        let mut added = 0;
        for (app_id, workspace, cap, state) in legacy.iter_entries() {
            let record = GrantRecord::app_capability(
                app_id,
                Path::new(workspace),
                cap,
                Decision::from_permission_state(state),
            );
            let exists = self.data.records.iter().any(|r| {
                r.actor_type == record.actor_type
                    && r.actor_id == record.actor_id
                    && r.target_type == record.target_type
                    && r.target_id == record.target_id
                    && r.workspace_root == record.workspace_root
            });
            if !exists {
                self.data.records.push(record);
                added += 1;
            }
        }
        added
    }

    /// Append-or-replace a grant. An existing record with the same identity
    /// (actor, target, workspace, source) is replaced so user decisions
    /// update in place instead of accumulating contradictions.
    pub fn record(&mut self, record: GrantRecord) {
        self.data.records.retain(|r| {
            !(r.actor_type == record.actor_type
                && r.actor_id == record.actor_id
                && r.target_type == record.target_type
                && r.target_id == record.target_id
                && r.workspace_root == record.workspace_root
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
        self.data.records.push(record);
    }

    /// Convenience: persist an app capability decision (the prompt-modal and
    /// SetPermission write path).
    pub fn record_app_capability(
        &mut self,
        app_id: &str,
        workspace_root: &Path,
        cap: Capability,
        decision: Decision,
    ) {
        self.record(GrantRecord::app_capability(
            app_id,
            workspace_root,
            cap,
            decision,
        ));
    }

    /// Remove every grant for `(actor_type, actor_id, target_id)` regardless
    /// of workspace, source, or duration. Returns the number of records
    /// removed. The caller decides whether to `save()`.
    pub fn revoke(&mut self, actor_type: ActorType, actor_id: &str, target_id: &str) -> usize {
        let before = self.data.records.len();
        self.data.records.retain(|r| {
            !(r.actor_type == actor_type && r.actor_id == actor_id && r.target_id == target_id)
        });
        let removed = before - self.data.records.len();
        log::info!(
            "grant_store: revoked {removed} record(s) for {actor_type:?} '{actor_id}' -> '{target_id}'"
        );
        removed
    }

    /// All persisted records (read-only).
    pub fn records(&self) -> &[GrantRecord] {
        &self.data.records
    }

    /// Atomically write to disk (temp file + rename). No-op for path-less
    /// test stores.
    pub fn save(&self) {
        if self.path.as_os_str().is_empty() {
            return;
        }
        match toml::to_string_pretty(&self.data) {
            Ok(s) => {
                let tmp = self.path.with_extension("toml.tmp");
                if let Err(e) =
                    std::fs::write(&tmp, &s).and_then(|_| std::fs::rename(&tmp, &self.path))
                {
                    log::error!("grant_store: failed to save {}: {e}", self.path.display());
                } else {
                    log::info!("grant_store: saved {}", self.path.display());
                }
            }
            Err(e) => log::error!("grant_store: serialize error: {e}"),
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
        let now = now_unix();
        let matching: Vec<&GrantRecord> = self
            .data
            .records
            .iter()
            .filter(|r| r.matches(req, now))
            .collect();

        let has = |decision: Decision, managed: bool| {
            matching
                .iter()
                .any(|r| r.decision == decision && (r.source == GrantSource::Managed) == managed)
        };

        let decision = if has(Decision::Deny, true) // 2. managed deny
            || has(Decision::Deny, false) // 3. user/workspace/session deny
            || posture.is_some_and(|p| PermissionPosture::lists_contain(&p.deny, &req.target_id))
        // 4. actor deny
        {
            Decision::Deny
        } else if has(Decision::Ask, true) // 5. managed ask
            || has(Decision::Ask, false) // 6. user/workspace/session ask
            || posture.is_some_and(|p| PermissionPosture::lists_contain(&p.ask, &req.target_id))
        // 7. actor ask
        {
            Decision::Ask
        } else if has(Decision::Allow, true) || has(Decision::Allow, false) // 8/10. persisted allow
            || posture.is_some_and(|p| PermissionPosture::lists_contain(&p.allow, &req.target_id))
        // 9. actor allow
        {
            Decision::Allow
        } else {
            // 11. default posture fallback.
            posture.map_or(Decision::Ask, |p| p.default_posture)
        };

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

    /// Capability sets for an app launch, equivalent to the legacy
    /// `build_permission_sets` but read from generalized records:
    /// `(allowed, denied)` capabilities for this app+workspace.
    pub fn app_capability_sets(
        &self,
        app_id: &str,
        workspace_root: &Path,
    ) -> (HashSet<Capability>, HashSet<Capability>) {
        let ws = canonical(workspace_root);
        let now = now_unix();
        let mut allowed = HashSet::new();
        let mut denied = HashSet::new();
        for r in &self.data.records {
            if r.actor_type != ActorType::App
                || r.actor_id != app_id
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

// ── helpers ───────────────────────────────────────────────────────────────────

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_grant(decision: Decision, source: GrantSource) -> GrantRecord {
        GrantRecord {
            actor_type: ActorType::Agent,
            actor_id: "chess-opponent".to_string(),
            actor_scope: ActorScope::Workspace,
            workspace_root: None,
            target_type: TargetType::AppConnector,
            target_id: "chess.make_move".to_string(),
            resource_scope: ResourceScope::Game,
            resource_id: Some("game-1".to_string()),
            decision,
            duration: GrantDuration::Session,
            source,
            created_at: now_unix(),
            expires_at: None,
        }
    }

    fn agent_request() -> PermissionRequest {
        PermissionRequest::new(
            ActorType::Agent,
            "chess-opponent",
            TargetType::AppConnector,
            "chess.make_move",
            None,
        )
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
            Decision::Allow
        );
        assert_eq!(
            store.evaluate(&req("anything.else"), Some(&posture)),
            Decision::Allow,
            "unlisted target falls back to default_posture"
        );
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
    fn agent_actor_evaluates_same_path_as_app() {
        // Same target id, different actor types — grants must not bleed.
        let mut store = GrantStore::default();
        store.record(agent_grant(Decision::Allow, GrantSource::User));
        assert_eq!(store.evaluate(&agent_request(), None), Decision::Allow);

        let app_req = PermissionRequest::new(
            ActorType::App,
            "chess-opponent",
            TargetType::AppConnector,
            "chess.make_move",
            None,
        );
        assert_eq!(
            store.evaluate(&app_req, None),
            Decision::Ask,
            "an agent grant must not satisfy an app request"
        );
    }

    #[test]
    fn workspace_scoping_respected() {
        let ws_a = tempfile::tempdir().unwrap();
        let ws_b = tempfile::tempdir().unwrap();
        let mut store = GrantStore::default();
        store.record(GrantRecord::app_capability(
            "my-app",
            ws_a.path(),
            Capability::FsWrite,
            Decision::Allow,
        ));
        let req_a = PermissionRequest::app_capability("my-app", ws_a.path(), Capability::FsWrite);
        let req_b = PermissionRequest::app_capability("my-app", ws_b.path(), Capability::FsWrite);
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
        g.expires_at = Some(now_unix() - 10);
        store.record(g);
        assert_eq!(store.evaluate(&agent_request(), None), Decision::Ask);
    }

    #[test]
    fn record_replaces_same_identity_same_source() {
        let mut store = GrantStore::default();
        store.record(agent_grant(Decision::Allow, GrantSource::User));
        store.record(agent_grant(Decision::Deny, GrantSource::User));
        assert_eq!(store.records().len(), 1, "same identity+source must replace");
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
        let mut store = GrantStore::load_or_default(tmp.path());
        store.record(GrantRecord::app_capability(
            "my-app",
            ws.path(),
            Capability::NetHttp,
            Decision::Deny,
        ));
        store.record(agent_grant(Decision::Allow, GrantSource::User));
        store.save();

        let reloaded = GrantStore::load_or_default(tmp.path());
        assert_eq!(reloaded.records().len(), 2);
        assert_eq!(reloaded.evaluate(&agent_request(), None), Decision::Allow);
        let req = PermissionRequest::app_capability("my-app", ws.path(), Capability::NetHttp);
        assert_eq!(reloaded.evaluate(&req, None), Decision::Deny);
    }

    #[test]
    fn corrupt_grants_file_backed_up() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("grants.toml");
        std::fs::write(&path, b"not toml ][[[").unwrap();
        let store = GrantStore::load_or_default(tmp.path());
        assert!(!path.exists(), "corrupt grants.toml must be renamed away");
        assert!(store.records().is_empty());
        let backup_exists = std::fs::read_dir(tmp.path()).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("grants.toml.corrupt-")
        });
        assert!(backup_exists);
    }

    #[test]
    fn legacy_migration_is_loss_free_and_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();

        // Seed a legacy permissions.toml with all three states.
        let mut legacy = PermissionStore::load_or_default(tmp.path());
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
        let store = GrantStore::load_or_default(tmp.path());
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
            assert_eq!(r.actor_type, ActorType::App);
            assert_eq!(r.actor_id, "my-app");
            assert_eq!(r.target_type, TargetType::Capability);
            assert_eq!(r.resource_scope, ResourceScope::Workspace);
            assert_eq!(r.workspace_root.as_deref(), Some(ws_canon.as_path()));
        }

        // Second load must not duplicate (idempotent), and legacy store must
        // be untouched.
        let store2 = GrantStore::load_or_default(tmp.path());
        assert_eq!(store2.records().len(), 3, "migration must be idempotent");
        let legacy2 = PermissionStore::load_or_default(tmp.path());
        assert_eq!(
            legacy2.iter_entries().count(),
            3,
            "legacy store must keep working untouched"
        );

        // Migrated records evaluate correctly.
        let req = PermissionRequest::app_capability("my-app", ws.path(), Capability::FsWrite);
        assert_eq!(store2.evaluate(&req, None), Decision::Allow);
        let req = PermissionRequest::app_capability("my-app", ws.path(), Capability::NetHttp);
        assert_eq!(store2.evaluate(&req, None), Decision::Deny);
    }

    #[test]
    fn app_capability_sets_reads_generalized_records() {
        let ws = tempfile::tempdir().unwrap();
        let mut store = GrantStore::default();
        store.record(GrantRecord::app_capability(
            "my-app",
            ws.path(),
            Capability::FsWrite,
            Decision::Allow,
        ));
        store.record(GrantRecord::app_capability(
            "my-app",
            ws.path(),
            Capability::NetHttp,
            Decision::Deny,
        ));
        store.record(GrantRecord::app_capability(
            "other-app",
            ws.path(),
            Capability::PanesRead,
            Decision::Allow,
        ));
        let (allowed, denied) = store.app_capability_sets("my-app", ws.path());
        assert!(allowed.contains(&Capability::FsWrite));
        assert!(denied.contains(&Capability::NetHttp));
        assert!(
            !allowed.contains(&Capability::PanesRead),
            "other-app's grant must not bleed"
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
        let err = PermissionPosture::from_toml_str(
            "[permissions]\ndefault_posture = \"sometimes\"\n",
        )
        .unwrap_err();
        assert!(err.contains("sometimes"), "error must name the bad value: {err}");
        assert!(
            PermissionPosture::from_toml_str("[other]\nx = 1\n").is_err(),
            "missing [permissions] table must fail, not silently default"
        );
    }
}
