//! Shared names-only inventory and user-initiated secret mutations.
//! Read admission and click-only approvals remain owned by the permission monitor.
pub(crate) use super::folder::valid_env_name;
use super::{SecretStore, WorkspaceConfig};
use crate::broker::custody::host_process;
use crate::broker::gate::PermissionMonitor;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SecretTarget {
    Global,
    Workspace {
        id: String,
        root: PathBuf,
    },
    /// The canonical directory selected before admission.
    Folder(PathBuf),
}

impl SecretTarget {
    pub fn label(&self) -> String {
        match self {
            Self::Global => "global".into(),
            Self::Workspace { root, .. } => format!("workspace: {}", root.display()),
            Self::Folder(folder) => format!("folder: {}", folder.display()),
        }
    }

    pub fn resource(&self, name: &str) -> String {
        let scope = match self {
            Self::Global => "global".into(),
            Self::Workspace { root, .. } => crate::platform::path::canonical_or_self(root)
                .display()
                .to_string(),
            Self::Folder(folder) => format!("folder:{}", folder.display()),
        };
        format!("{name}@{scope}")
    }

    /// The key reads are granted and audited under. Inventory uses the
    /// captured workspace root. Folder reads use the bare path.
    fn read_resource(&self, name: &str) -> String {
        match self {
            Self::Workspace { root, .. } | Self::Folder(root) => {
                format!("{name}@{}", root.display())
            }
            Self::Global => self.resource(name),
        }
    }

    fn account(&self, name: &str) -> Result<String, String> {
        match self {
            Self::Global => Ok(super::keychain_user_name(name)),
            Self::Workspace { id, .. } => {
                let account = super::keychain_workspace_name(id, name);
                if super::workspace_id_is_reserved(id)
                    || super::secret_account_is_reserved(&account)
                {
                    return Err(super::reserved_secret_error(id));
                }
                Ok(account)
            }
            Self::Folder(folder) => {
                if !crate::release::feature_enabled(crate::release::ReleaseFeature::FolderSecrets) {
                    return Err("Folder secrets experiment is off.".into());
                }
                if super::canonical_folder(folder)? != *folder {
                    return Err("The selected folder changed. Refresh before retrying.".into());
                }
                Ok(super::folder::account_name(folder, name))
            }
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SecretEntry {
    pub name: String,
    pub scope: String,
    /// Unix seconds for the most recent audited use; null means unknown.
    pub last_used: Option<u64>,
    /// Live actor-specific permissions, not a promise that every caller is allowed.
    pub grant: Vec<String>,
    pub target: SecretTarget,
}

pub fn workspace_target(cwd: &Path) -> Result<Option<SecretTarget>, String> {
    let Some(root) = crate::app::registry::resolve_workspace_root(cwd) else {
        return Ok(None);
    };
    Ok(WorkspaceConfig::load(&root)?.map(|cfg| SecretTarget::Workspace { id: cfg.id, root }))
}

/// [`workspace_target`] for a caller that has not consented yet. A parser
/// diagnostic quotes the offending source line, so the error and the log name
/// the file and never its contents.
fn consent_workspace_target(cwd: &Path) -> Result<Option<SecretTarget>, String> {
    let Some(root) = crate::app::registry::resolve_workspace_root(cwd) else {
        return Ok(None);
    };
    WorkspaceConfig::load(&root)
        .map(|cfg| {
            cfg.map(|cfg| SecretTarget::Workspace {
                id: cfg.id,
                root: root.clone(),
            })
        })
        .map_err(|_| {
            let file = root
                .join(crate::config::workspace_channel_dir())
                .join("workspace.toml");
            log::warn!(
                "secret_inventory: workspace config {} is unreadable or invalid",
                file.display()
            );
            format!(
                "The workspace config {} is unreadable or invalid. Fix it, then rerun.",
                file.display()
            )
        })
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum SkippedSecrets {
    OutsidePaneContext {
        scope: String,
        path: PathBuf,
        pane_context_root: PathBuf,
        pane_context_name: Option<String>,
    },
    ExperimentDisabled {
        scope: String,
        experiment: String,
        count: usize,
    },
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct SecretInventory {
    pub entries: Vec<SecretEntry>,
    pub skipped: Vec<SkippedSecrets>,
    /// Why grant and last-used columns are empty: they come from the running
    /// host, and this process is not the host (stint 1291). `None` when they
    /// are filled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_details_unavailable: Option<String>,
}

/// Printed once when a listing runs outside the host.
pub const GRANT_DETAILS_FROM_HOST: &str =
    "Grant and last-used details come from the running Plexi host and are not available to this command.";

/// Whether a global secret is stored under its canonical or a legacy spelling.
/// Scans account attributes only: no value is read and nothing is migrated.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub fn global_secret_stored(name: &str) -> Result<bool, String> {
    let accounts = super::system_store(&host_process()?)
        .scan_accounts()
        .map_err(|error| error.to_string())?;
    Ok(std::iter::once(name)
        .chain(super::reconcile::legacy_friendly_names(name))
        .map(super::keychain_user_name)
        .any(|account| accounts.contains(&account)))
}

pub fn secret_inventory(cwd: &Path, global_only: bool) -> Result<Vec<SecretEntry>, String> {
    let scope = InventoryScope::resolve(cwd, global_only, None, Vec::new())?;
    Ok(inventory_for_scope(scope, None)?.entries)
}

/// Scope selected before consent and reused by the scan after consent.
pub(crate) struct InventoryScope {
    targets: Vec<SecretTarget>,
    folders: Option<FolderInventoryScope>,
    skipped: Vec<SkippedSecrets>,
}

struct FolderInventoryScope {
    cwd: PathBuf,
    pane_root: Option<PathBuf>,
    enabled: bool,
}

impl InventoryScope {
    pub(crate) fn resolve(
        cwd: &Path,
        global_only: bool,
        pane_root: Option<&Path>,
        mut skipped: Vec<SkippedSecrets>,
    ) -> Result<Self, String> {
        let mut targets = vec![SecretTarget::Global];
        let folders = if global_only {
            None
        } else {
            if let Some(SecretTarget::Workspace { id, root }) = consent_workspace_target(cwd)? {
                let root = crate::platform::path::canonical_or_self(&root);
                if let Some(pane_root) = pane_root.filter(|pane_root| !root.starts_with(pane_root))
                {
                    skipped.push(SkippedSecrets::OutsidePaneContext {
                        scope: "workspace".into(),
                        path: root,
                        pane_context_root: pane_root.to_path_buf(),
                        pane_context_name: None,
                    });
                } else if !super::workspace_id_is_reserved(&id) {
                    targets.push(SecretTarget::Workspace { id, root });
                }
            }
            Some(FolderInventoryScope {
                cwd: crate::platform::path::canonical_or_self(cwd),
                pane_root: pane_root.map(Path::to_path_buf),
                enabled: crate::release::feature_enabled(
                    crate::release::ReleaseFeature::FolderSecrets,
                ),
            })
        };
        Ok(Self {
            targets,
            folders,
            skipped,
        })
    }

    pub(crate) fn disclosure(&self) -> String {
        let mut scopes: Vec<_> = self.targets.iter().map(SecretTarget::label).collect();
        if let Some(folders) = &self.folders {
            let cwd = folders.cwd.display();
            scopes.push(if folders.enabled {
                format!("folder: {cwd} and its ancestors")
            } else {
                format!("hidden folder secret count for {cwd} and its ancestors")
            });
        }
        scopes.join(", ")
    }
}

/// Inventory for an admitted socket caller. The grant column shows only its rows.
pub(crate) fn inventory_for_caller(
    scope: InventoryScope,
    requester: &crate::broker::principal::Principal,
) -> Result<SecretInventory, String> {
    inventory_for_scope(scope, Some(requester))
}

/// The desktop reconciles legacy entries. Socket inventory scans names only.
fn inventory_for_scope(
    scope: InventoryScope,
    requester: Option<&crate::broker::principal::Principal>,
) -> Result<SecretInventory, String> {
    let InventoryScope {
        targets,
        folders,
        mut skipped,
    } = scope;
    let host = host_process()?;
    let accounts = if requester.is_some() {
        Some(
            super::system_store(&host)
                .scan_accounts()
                .map_err(|error| error.to_string())?,
        )
    } else {
        match super::reconcile_index_with_keychain(&host) {
            Ok(report) if !report.is_noop() => log::info!(
                "secrets_manager: reconciled index adopted={} renamed={} stale={} conflicts={}",
                report.adopted.len(),
                report.renamed.len(),
                report.stale.len(),
                report.conflicts.len()
            ),
            Ok(_) => {}
            Err(error) => {
                log::warn!("secrets_manager: reconcile failed, listing indexed names: {error}")
            }
        }
        None
    };
    let mut entries = Vec::new();
    let (uses, permissions, grant_details_unavailable) =
        match PermissionMonitor::for_profile(&crate::config::config_dir()) {
            Ok(monitor) => (
                monitor.secret_last_uses()?,
                match requester {
                    Some(principal) => monitor.records_for(principal).entries(),
                    None => monitor.host_records().entries(),
                },
                None,
            ),
            Err(error) => {
                log::info!("secrets_manager: listing names without grant details: {error}");
                (
                    Default::default(),
                    Vec::new(),
                    Some(GRANT_DETAILS_FROM_HOST.to_string()),
                )
            }
        };
    let mut add = |name: String, target: SecretTarget| {
        let resource = target.read_resource(&name);
        let grant = permissions
            .iter()
            .filter(|row| {
                row.resource_id.as_deref() == Some(resource.as_str())
                    && matches!(row.tool.as_str(), "secret.read" | "secret.get")
            })
            .map(|row| format!("{}: {} ({})", row.actor_id, row.kind, row.duration))
            .collect();
        entries.push(SecretEntry {
            name,
            scope: target.label(),
            last_used: uses.get(&resource).copied(),
            grant,
            target,
        });
    };
    for target in targets {
        let prefix = match &target {
            SecretTarget::Global => super::keychain_user_name(""),
            SecretTarget::Workspace { id, .. } => super::keychain_workspace_name(id, ""),
            SecretTarget::Folder(_) => continue,
        };
        let names;
        let accounts = match &accounts {
            Some(accounts) => accounts,
            None => {
                names = super::system_store(&host).list_with_prefix(&prefix);
                &names
            }
        };
        for account in accounts {
            if let Some(name) = account.strip_prefix(&prefix) {
                add(name.into(), target.clone());
            }
        }
    }
    if let Some(FolderInventoryScope {
        cwd,
        pane_root,
        enabled,
    }) = folders
    {
        let mut hidden = 0;
        let bindings = if requester.is_some() {
            super::folder_store(&host)
                .scan_accounts()
                .map_err(|error| error.to_string())?
                .iter()
                .filter_map(|account| super::folder::parse_account(account))
                .collect()
        } else {
            super::list_folder_secrets(super::folder_store(&host))
        };
        for meta in bindings {
            let folder = crate::platform::path::canonical_or_self(&meta.folder);
            if cwd.starts_with(&folder)
                && pane_root
                    .as_deref()
                    .is_none_or(|root| folder.starts_with(root))
            {
                if enabled {
                    add(meta.name, SecretTarget::Folder(meta.folder));
                } else {
                    hidden += 1;
                }
            }
        }
        if hidden > 0 {
            log::info!(
                "secret_list: hidden folder entries count={hidden} experiment=folder_secrets"
            );
            skipped.push(SkippedSecrets::ExperimentDisabled {
                scope: "folder".into(),
                experiment: "folder_secrets".into(),
                count: hidden,
            });
        }
    }
    entries.sort_by(|a, b| {
        let rank = |target: &SecretTarget| match target {
            SecretTarget::Global => 0,
            SecretTarget::Workspace { .. } => 1,
            SecretTarget::Folder(_) => 2,
        };
        rank(&a.target)
            .cmp(&rank(&b.target))
            .then(a.scope.cmp(&b.scope))
            .then(a.name.cmp(&b.name))
    });
    Ok(SecretInventory {
        entries,
        skipped,
        grant_details_unavailable,
    })
}

pub enum Mutation<'a> {
    Set(&'a str),
    SetRouted { canonical: &'a str, value: &'a str },
    Delete,
}

/// One write path for add, replacement, deletion and import. Audit intent
/// before touching the backend; an unavailable audit prevents the mutation.
/// No grant is minted and no read-admission behavior is changed here.
pub fn mutate_secret(
    store: &dyn SecretStore,
    monitor: &PermissionMonitor,
    target: &SecretTarget,
    name: &str,
    mutation: Mutation<'_>,
) -> Result<(), String> {
    let (canonical, value) = match mutation {
        Mutation::Set(value) => (name, Some(value)),
        Mutation::SetRouted { canonical, value } => {
            if !matches!(target, SecretTarget::Workspace { .. }) {
                return Err("An alias requires a workspace.".into());
            }
            (canonical, Some(value))
        }
        Mutation::Delete => (name, None),
    };
    let account = target.account(name)?;
    if value == Some("") {
        return Err("Value cannot be empty.".into());
    }
    let operation = if value.is_some() { "set" } else { "delete" };
    let resource = target.resource(name);
    let call_id = format!("secret-mutation-{}", uuid::Uuid::new_v4());
    monitor.note_secret_mutation(&call_id, &resource, operation, "requested")?;
    let result = match value {
        Some(value) => match target {
            SecretTarget::Folder(_) => {
                super::folder::set_folder_account(store, &account, name, value)
            }
            _ => store
                .set(&account, value)
                .map_err(|_| "Secret store write failed; value was not saved.".to_string()),
        },
        None => match target {
            SecretTarget::Folder(folder) => {
                super::folder::remove_folder_account(store, &account, name, folder)
            }
            _ => match store.delete(&account) {
                Ok(super::DeleteOutcome::Deleted) => Ok(()),
                Ok(super::DeleteOutcome::NoMatchReported) => Err(format!(
                    "The secret store returned no match for '{name}'. Refresh before retrying."
                )),
                Err(error) => {
                    log::warn!("secrets_manager: delete name={name} failed: {error}");
                    Err(format!("Secret store deletion failed: {error}"))
                }
            },
        },
    };
    monitor
        .note_secret_mutation(
            &call_id,
            &resource,
            operation,
            if result.is_ok() { "ok" } else { "failed" },
        )
        .map_err(|_| {
            "Mutation reached the store, but its completion audit failed. Refresh before retrying."
                .to_string()
        })?;
    result?;
    if let (SecretTarget::Workspace { root, .. }, Some(_)) = (target, value) {
        super::write_default_route(root, canonical, name).map_err(|error| {
            log::warn!("secrets_manager: saved {name}, default route failed: {error}");
            "Secret saved, but its workspace route could not be saved. Refresh before retrying."
                .to_string()
        })?;
    }
    log::info!(
        "secrets_manager: mutation operation={operation} name={name} scope={}",
        target.label()
    );
    Ok(())
}

/// Import literal dotenv assignments. Shell evaluation and variable expansion
/// are deliberately unsupported: credentials must not depend on process env.
/// Parse the complete file before writing; backend failures report partial work.
pub fn import_env(
    store: &dyn SecretStore,
    monitor: &PermissionMonitor,
    target: &SecretTarget,
    path: &Path,
) -> Result<usize, String> {
    let text = zeroize::Zeroizing::new(std::fs::read_to_string(path).map_err(|error| {
        log::warn!(
            "secrets_manager: read import {} failed: {error}",
            path.display()
        );
        "Could not read the selected .env file.".to_string()
    })?);
    let entries = parse_env(&text)?;
    for (done, (name, value)) in entries.iter().enumerate() {
        mutate_secret(store, monitor, target, name, Mutation::Set(value))
            .map_err(|error| format!("Imported {done} of {} secrets. {error}", entries.len()))?;
    }
    log::info!(
        "secrets_manager: imported count={} scope={}",
        entries.len(),
        target.label()
    );
    Ok(entries.len())
}

fn parse_env(text: &str) -> Result<Vec<(String, zeroize::Zeroizing<String>)>, String> {
    let mut entries = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let invalid = || {
            format!("Invalid .env assignment on line {}. Use NAME=value; shell expansion and multiline values are unsupported.", index + 1)
        };
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let (name, raw) = line.split_once('=').ok_or_else(invalid)?;
        let name = name.trim();
        if !valid_env_name(name) {
            return Err(invalid());
        }
        let raw = raw.trim();
        let value = if let Some(quote) = raw.chars().next().filter(|c| *c == '\'' || *c == '"') {
            let end = raw[1..].find(quote).ok_or_else(invalid)? + 1;
            let suffix = raw[end + 1..].trim();
            if !suffix.is_empty() && !suffix.starts_with('#') {
                return Err(invalid());
            }
            raw[1..end].to_string()
        } else {
            raw.split_once(" #")
                .map_or(raw, |(value, _)| value)
                .trim_end()
                .to_string()
        };
        if value.is_empty()
            || value.contains("${")
            || value.contains("$(")
            || value.contains('`')
            || entries.iter().any(|(key, _)| key == name)
        {
            return Err(invalid());
        }
        entries.push((name.to_string(), zeroize::Zeroizing::new(value)));
    }
    Ok(entries)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::secrets::{InMemoryKeychain, NonDestructiveStore};

    #[test]
    fn fresh_profile_folder_secrets_are_off() {
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().into());
        assert!(!crate::release::feature_enabled(
            crate::release::ReleaseFeature::FolderSecrets
        ));
    }

    #[test]
    fn audited_mutations_and_import_keep_values_out_of_audit() {
        let profile = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let monitor = PermissionMonitor::for_profile(profile.path()).unwrap();
        let store = InMemoryKeychain::new();
        let target = SecretTarget::Workspace {
            id: "test-workspace".into(),
            root: workspace.path().into(),
        };
        mutate_secret(
            &store,
            &monitor,
            &target,
            "SERVICE_TOKEN",
            Mutation::Set("SENTINEL_SECRET_0948_DO_NOT_LEAK"),
        )
        .unwrap();
        mutate_secret(
            &store,
            &monitor,
            &target,
            "SERVICE_TOKEN",
            Mutation::Set("canary-edited-value"),
        )
        .unwrap();
        assert_eq!(
            store
                .get("plexi:test-workspace:SERVICE_TOKEN")
                .unwrap()
                .as_str(),
            "canary-edited-value"
        );
        mutate_secret(&store, &monitor, &target, "SERVICE_TOKEN", Mutation::Delete).unwrap();
        assert!(store.get("plexi:test-workspace:SERVICE_TOKEN").is_none());
        let fixture = workspace.path().join("fixture.env");
        let bytes =
            b"# fixture\nexport IMPORT_TOKEN='canary-import-value'\nSECOND=\"with spaces\"\n";
        std::fs::write(&fixture, bytes).unwrap();
        assert_eq!(import_env(&store, &monitor, &target, &fixture).unwrap(), 2);
        assert_eq!(std::fs::read(&fixture).unwrap(), bytes);
        assert!(store.get("plexi:test-workspace:IMPORT_TOKEN").is_some());
        let audit = std::fs::read_to_string(profile.path().join("permission-audit.jsonl")).unwrap();
        assert!(audit.contains("SERVICE_TOKEN"));
        assert!(audit.contains("IMPORT_TOKEN"));
        assert!(!audit.contains("canary-"));
        assert!(!audit.contains("SENTINEL_SECRET_0948_DO_NOT_LEAK"));
        assert!(!audit.contains("with spaces"));
        let rows = monitor.host_records().audit();
        assert_eq!(
            rows.iter()
                .filter(|r| r.kind == "secret_mutation" && r.decision == "ok")
                .count(),
            5
        );
        assert!(rows.iter().any(|r| r.operation_id == "delete"));
        assert!(
            crate::workspace::secrets::WorkspaceSecrets::load(workspace.path())
                .unwrap()
                .unwrap()
                .terminal
                .env
                .inject
                .is_empty()
        );
    }

    /// Stint 1635: deletion is one store call whose outcome decides the
    /// audit. The store panics on any read or list, so the old inventory
    /// pre-check fails here; intent and completion are both audited.
    #[test]
    fn delete_issues_one_store_call_and_audits_its_outcome() {
        use crate::workspace::secrets::{DeleteOnlyStore, DeleteOutcome};
        let profile = tempfile::tempdir().unwrap();
        let monitor = PermissionMonitor::for_profile(profile.path()).unwrap();
        let cases: [(
            fn() -> Result<DeleteOutcome, super::super::store::SecretError>,
            &str,
        ); 3] = [
            (|| Ok(DeleteOutcome::Deleted), "ok"),
            (|| Ok(DeleteOutcome::NoMatchReported), "failed"),
            (
                || {
                    Err(super::super::store::SecretError::Backend(
                        "status -50".into(),
                    ))
                },
                "failed",
            ),
        ];
        for (answer, audited) in cases {
            let store = DeleteOnlyStore::answering(answer);
            let result = mutate_secret(
                &store,
                &monitor,
                &SecretTarget::Global,
                "GONE",
                Mutation::Delete,
            );
            assert_eq!(result.is_ok(), audited == "ok", "{result:?}");
            assert_eq!(store.deletes(), 1);
            let rows = monitor.host_records().audit();
            let last = &rows[rows.len() - 1];
            assert_eq!(
                (last.operation_id.as_str(), last.decision.as_str()),
                ("delete", audited)
            );
            assert_eq!(rows[rows.len() - 2].decision, "requested");
        }
    }

    #[test]
    fn invalid_import_is_validated_before_any_mutation() {
        let profile = tempfile::tempdir().unwrap();
        let store = InMemoryKeychain::new();
        let monitor = PermissionMonitor::for_profile(profile.path()).unwrap();
        let fixture = profile.path().join("invalid.env");
        std::fs::write(&fixture, "GOOD=one\nnot a valid assignment\n").unwrap();
        assert!(import_env(&store, &monitor, &SecretTarget::Global, &fixture).is_err());
        assert!(store.list_with_prefix("plexi:").is_empty());
        let reserved = SecretTarget::Workspace {
            id: "host".into(),
            root: profile.path().into(),
        };
        assert!(
            mutate_secret(&store, &monitor, &reserved, "TOKEN", Mutation::Set("nope")).is_err()
        );
        monitor.fail_audit(true);
        assert!(mutate_secret(
            &store,
            &monitor,
            &SecretTarget::Global,
            "TOKEN",
            Mutation::Set("nope")
        )
        .is_err());
        assert!(store.list_with_prefix("plexi:").is_empty());
    }
}

#[cfg(test)]
mod inventory_tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn workspace_inventory_usage_stays_on_the_resolved_root() {
        let fixture = tempfile::tempdir().unwrap();
        let parent = fixture.path().canonicalize().unwrap();
        let root = parent.join("workspace");
        let moved = parent.join("moved");
        std::fs::create_dir(&root).unwrap();
        let target = SecretTarget::Workspace {
            id: "workspace-fixture".into(),
            root: root.clone(),
        };
        std::fs::rename(&root, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &root).unwrap();
        assert_eq!(
            target.read_resource("FIXTURE_TOKEN"),
            format!("FIXTURE_TOKEN@{}", root.display())
        );
    }

    #[test]
    fn folder_inventory_shows_read_grants_and_last_use() {
        use crate::broker::gate::ApprovalChoice;
        use crate::workspace::secrets::{read_folder_secret, ReadResult, SecretActor};
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().into());
        let _experiment =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
        let root = tempfile::tempdir().unwrap();
        let path = crate::platform::path::canonical_or_self(root.path());
        let name = format!("FOLDER_INV_{}", uuid::Uuid::new_v4().simple());
        let monitor = PermissionMonitor::for_profile(profile.path()).unwrap();
        let store = super::super::folder_store(crate::broker::custody::TEST_HOST);
        mutate_secret(
            store,
            &monitor,
            &SecretTarget::Folder(path.clone()),
            &name,
            Mutation::Set("folder-inventory-canary"),
        )
        .unwrap();
        let actor = SecretActor::agent("folder-reader");
        monitor.set_now_for_test(1_800_000_000);
        let ReadResult::PermissionRequired { pending_id } =
            read_folder_secret(&monitor, store, &actor, &name, &path).unwrap()
        else {
            panic!("read must ask")
        };
        monitor
            .approve_pending(&pending_id, ApprovalChoice::Session)
            .unwrap();
        assert!(matches!(
            read_folder_secret(&monitor, store, &actor, &name, &path).unwrap(),
            ReadResult::Value(_)
        ));
        let entries = secret_inventory(&path, false).unwrap();
        let entry = entries
            .iter()
            .find(|e| e.name == name && matches!(e.target, SecretTarget::Folder(_)))
            .unwrap();
        assert_eq!(entry.last_used, Some(1_800_000_000));
        assert!(entry
            .grant
            .iter()
            .any(|grant| grant.contains("folder-reader") && grant.contains("allow")));
        mutate_secret(
            store,
            &monitor,
            &SecretTarget::Folder(path),
            &name,
            Mutation::Delete,
        )
        .unwrap();
    }

    #[test]
    fn scopes_json_usage_and_grants_share_the_inventory() {
        use crate::broker::gate::ApprovalChoice;
        use crate::workspace::secrets::{read_pane_secret, ReadResult, SecretActor, SecretScope};
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().into());
        let root = tempfile::tempdir().unwrap();
        let cfg =
            super::super::init_workspace(root.path(), &crate::config::workspace_channel_dir())
                .unwrap();
        let name = format!("INVENTORY_{}", uuid::Uuid::new_v4().simple());
        let store = super::super::system_store(crate::broker::custody::TEST_HOST);
        let monitor = PermissionMonitor::for_profile(profile.path()).unwrap();
        let global = SecretTarget::Global;
        let workspace = SecretTarget::Workspace {
            id: cfg.id.clone(),
            root: crate::platform::path::canonical_or_self(root.path()),
        };
        mutate_secret(
            store,
            &monitor,
            &global,
            &name,
            Mutation::Set("inventory-secret-canary"),
        )
        .unwrap();
        mutate_secret(
            store,
            &monitor,
            &workspace,
            &name,
            Mutation::Set("inventory-secret-canary"),
        )
        .unwrap();
        let _experiment =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
        let folder = SecretTarget::Folder(crate::platform::path::canonical_or_self(root.path()));
        mutate_secret(
            super::super::folder_store(crate::broker::custody::TEST_HOST),
            &monitor,
            &folder,
            &name,
            Mutation::Set("folder-canary"),
        )
        .unwrap();
        let entries = secret_inventory(root.path(), false).unwrap();
        assert_eq!(entries.iter().filter(|e| e.name == name).count(), 3);
        assert!(entries
            .iter()
            .filter(|e| e.name == name)
            .all(|e| e.last_used.is_none() && e.grant.is_empty()));
        let actor = SecretActor::agent("inventory-reader");
        monitor.set_now_for_test(1_800_000_000);
        let scope = SecretScope::Global;
        let read =
            read_pane_secret(&monitor, store, &actor, &name, &scope, profile.path()).unwrap();
        let ReadResult::PermissionRequired { pending_id } = read else {
            panic!("read must ask")
        };
        monitor
            .approve_pending(&pending_id, ApprovalChoice::Session)
            .unwrap();
        assert!(matches!(
            read_pane_secret(&monitor, store, &actor, &name, &scope, profile.path()).unwrap(),
            ReadResult::Value(_)
        ));
        let entries = secret_inventory(root.path(), true).unwrap();
        assert!(entries.iter().all(|e| e.target == SecretTarget::Global));
        let entry = entries.iter().find(|e| e.name == name).unwrap();
        assert_eq!(entry.last_used, Some(1_800_000_000));
        monitor.set_now_for_test(1_800_000_001);
        assert_eq!(
            secret_inventory(root.path(), true)
                .unwrap()
                .iter()
                .find(|entry| entry.name == name)
                .unwrap()
                .last_used,
            Some(1_800_000_000)
        );
        assert!(entry
            .grant
            .iter()
            .any(|grant| grant.contains("inventory-reader") && grant.contains("allow")));
        // A requester's inventory shows its own grant rows only (stint 1743).
        let stranger = crate::broker::principal::Principal::agent("inventory-stranger").unwrap();
        let scope =
            InventoryScope::resolve(root.path(), false, Some(root.path()), Vec::new()).unwrap();
        let theirs = inventory_for_caller(scope, &stranger).unwrap();
        let seen = theirs
            .entries
            .iter()
            .find(|e| e.name == name && e.target == SecretTarget::Global)
            .unwrap();
        assert!(seen.grant.is_empty(), "{:?}", seen.grant);
        let json = serde_json::to_value(entry).unwrap();
        assert_eq!(json["scope"], "global");
        assert!(json["last_used"].is_number());
        assert!(json["grant"].is_array());
        assert!(!json.to_string().contains("canary"));
        let _disabled = crate::release::set_test_experiments(&[]);
        assert!(!secret_inventory(root.path(), false)
            .unwrap()
            .iter()
            .any(|entry| matches!(entry.target, SecretTarget::Folder(_))));
        assert!(mutate_secret(
            super::super::folder_store(crate::broker::custody::TEST_HOST),
            &monitor,
            &folder,
            &name,
            Mutation::Set("refused")
        )
        .is_err());
        mutate_secret(store, &monitor, &global, &name, Mutation::Delete).unwrap();
        mutate_secret(store, &monitor, &workspace, &name, Mutation::Delete).unwrap();
    }
}
