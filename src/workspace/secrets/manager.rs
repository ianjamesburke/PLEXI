//! Shared names-only inventory and user-initiated secret mutations.
//! Read admission and click-only approvals remain owned by the permission monitor.
use super::{SecretStore, WorkspaceConfig};
use crate::broker::gate::PermissionMonitor;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SecretTarget {
    Global,
    Workspace { id: String, root: PathBuf },
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
            Self::Folder(folder) => crate::platform::path::canonical_or_self(folder)
                .display()
                .to_string(),
        };
        format!("{name}@{scope}")
    }

    fn account(&self, name: &str) -> Result<String, String> {
        if !super::folder::valid_env_name(name) {
            return Err(
                "Use an environment variable name (letters, digits and underscores).".into(),
            );
        }
        match self {
            Self::Global => Ok(super::keychain_user_name(name)),
            Self::Workspace { id, .. } => {
                if super::workspace_id_is_reserved(id) {
                    return Err(super::reserved_secret_error(id));
                }
                Ok(super::keychain_workspace_name(id, name))
            }
            Self::Folder(folder) => {
                if !crate::release::feature_enabled(crate::release::ReleaseFeature::FolderSecrets) {
                    return Err("Folder secrets experiment is off.".into());
                }
                Ok(super::folder::account_name(
                    &super::canonical_folder(folder)?,
                    name,
                ))
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
    /// Set when the record's name is reserved for the pane environment: it
    /// is never exported to a pane, and this says how to move it to a free
    /// name. Computed from the name alone; no value is read (stint 1682).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quarantined: Option<String>,
}

/// How to recover a record whose name the pane environment reserves, or
/// `None` for an exportable name.
pub fn quarantine_remedy(name: &str, target: &SecretTarget) -> Option<String> {
    if !super::is_reserved_env_name(name) {
        return None;
    }
    let cli = crate::cli::setup::cli_name();
    Some(match target {
        SecretTarget::Workspace { .. } => format!(
            "'{name}' is reserved for the pane environment and is not exported. Route a free name to it: add NEW = \"{name}\" under [default] in {}/secrets.toml, then turn terminal injection on for NEW and off for {name}.",
            crate::config::workspace_channel_dir()
        ),
        SecretTarget::Global => format!(
            "'{name}' is reserved for the pane environment and is not exported. Copy it to a free name with `{cli} secret copy {name} NEW --global`, then delete the old record with `{cli} secret delete {name} --global`."
        ),
        SecretTarget::Folder(folder) => format!(
            "'{name}' is reserved for the pane environment and is not exported. Copy it to a free name with `{cli} secret copy {name} NEW --folder {}`, then remove the old record with `{cli} secret rm {name} --folder {}`.",
            folder.display(),
            folder.display()
        ),
    })
}

/// Copy a global or folder record to a free exportable name, leaving the
/// original in place (stint 1682). The write is create-only, so an existing
/// `to` is never overwritten, and the copy is verified by read-back. Any
/// failure leaves `from` untouched; the owner removes it once satisfied.
pub fn copy_secret(
    store: &dyn SecretStore,
    target: &SecretTarget,
    from: &str,
    to: &str,
) -> Result<(), String> {
    if let SecretTarget::Workspace { .. } = target {
        return Err(format!(
            "A workspace secret moves to a new name by route, not by copy: add {to} = \"{from}\" under [default] in {}/secrets.toml.",
            crate::config::workspace_channel_dir()
        ));
    }
    super::SecretEnvName::parse(to)?;
    let from_account = target.account(from)?;
    let to_account = target.account(to)?;
    let value = store
        .get_interactive(&from_account)
        .map_err(|error| format!("could not read '{from}' in {}: {error}", target.label()))?
        .ok_or_else(|| format!("no secret named '{from}' in {}", target.label()))?;
    store.add_new(&to_account, &value).map_err(|error| {
        log::info!(
            "secrets_manager: copy from={from} to={to} scope={} refused: {error}",
            target.label()
        );
        format!(
            "could not create '{to}' in {} ({error}); '{from}' is unchanged",
            target.label()
        )
    })?;
    if store
        .get_interactive(&to_account)
        .ok()
        .flatten()
        .is_none_or(|copied| copied.as_str() != value.as_str())
    {
        log::warn!(
            "secrets_manager: copy from={from} to={to} scope={} did not read back",
            target.label()
        );
        return Err(format!(
            "'{to}' did not read back the copied value; '{from}' is unchanged. Check '{to}' before removing anything."
        ));
    }
    log::info!(
        "secrets_manager: copied from={from} to={to} scope={}",
        target.label()
    );
    Ok(())
}

pub fn workspace_target(cwd: &Path) -> Result<Option<SecretTarget>, String> {
    let Some(root) = crate::app::registry::resolve_workspace_root(cwd) else {
        return Ok(None);
    };
    Ok(WorkspaceConfig::load(&root)?.map(|cfg| SecretTarget::Workspace { id: cfg.id, root }))
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
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

pub fn secret_inventory(cwd: &Path, global_only: bool) -> Result<Vec<SecretEntry>, String> {
    Ok(secret_inventory_report(cwd, global_only)?.entries)
}

pub fn secret_inventory_report(cwd: &Path, global_only: bool) -> Result<SecretInventory, String> {
    let mut skipped = Vec::new();
    let allowed = global_only
        || crate::cli::workspace::enforce_pane_context_scope(cwd, false, "secret.list").is_ok();
    let pane_root = crate::cli::workspace::active_pane_context_root();
    if !allowed {
        if let Some(root) = &pane_root {
            skipped.push(SkippedSecrets::OutsidePaneContext {
                scope: "workspace".into(),
                path: cwd.to_path_buf(),
                pane_context_root: root.clone(),
                pane_context_name: std::env::var("PLEXI_CONTEXT_NAME").ok(),
            });
        }
    }
    inventory_for_scope(cwd, global_only || !allowed, pane_root.as_deref(), skipped)
}

/// Only the host may use this after admitting an explicit context request.
pub(crate) fn inventory_for_context(root: &Path) -> Result<SecretInventory, String> {
    inventory_for_scope(root, false, Some(root), Vec::new())
}

/// Reconcile first, list names without reading values, then attach the same
/// permission metadata for both native UI and CLI.
fn inventory_for_scope(
    cwd: &Path,
    global_only: bool,
    pane_root: Option<&Path>,
    mut skipped: Vec<SkippedSecrets>,
) -> Result<SecretInventory, String> {
    match super::reconcile_index_with_keychain() {
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
    let mut targets = vec![SecretTarget::Global];
    if !global_only {
        if let Some(target) = workspace_target(cwd)? {
            if let (SecretTarget::Workspace { root, .. }, Some(pane_root)) = (&target, pane_root) {
                if !crate::platform::path::canonical_or_self(root).starts_with(pane_root) {
                    skipped.push(SkippedSecrets::OutsidePaneContext {
                        scope: "workspace".into(), path: root.clone(),
                        pane_context_root: pane_root.to_path_buf(), pane_context_name: None,
                    });
                } else {
                    targets.push(target);
                }
            } else {
                targets.push(target);
            }
        }
    }
    let mut entries = Vec::new();
    let (uses, permissions, grant_details_unavailable) =
        match PermissionMonitor::for_profile(&crate::config::config_dir()) {
            Ok(monitor) => (monitor.secret_last_uses()?, monitor.list_entries(), None),
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
        let resource = target.resource(&name);
        let grant = permissions
            .iter()
            .filter(|row| {
                row.resource_id.as_deref() == Some(resource.as_str())
                    && matches!(row.tool.as_str(), "secret.read" | "secret.get")
            })
            .map(|row| format!("{}: {} ({})", row.actor_id, row.kind, row.duration))
            .collect();
        let quarantined = quarantine_remedy(&name, &target);
        entries.push(SecretEntry {
            name,
            scope: target.label(),
            last_used: uses.get(&resource).copied(),
            grant,
            quarantined,
            target,
        });
    };
    for target in targets {
        let prefix = match &target {
            SecretTarget::Global => super::keychain_user_name(""),
            SecretTarget::Workspace { id, .. } if !super::workspace_id_is_reserved(id) => {
                super::keychain_workspace_name(id, "")
            }
            _ => continue,
        };
        for account in super::system_store().list_with_prefix(&prefix) {
            if let Some(name) = account.strip_prefix(&prefix) {
                add(name.into(), target.clone());
            }
        }
    }
    if !global_only {
        let cwd = crate::platform::path::canonical_or_self(cwd);
        let enabled =
            crate::release::feature_enabled(crate::release::ReleaseFeature::FolderSecrets);
        let mut hidden = 0;
        for meta in super::list_folder_secrets(super::folder_store()) {
            let folder = crate::platform::path::canonical_or_self(&meta.folder);
            if cwd.starts_with(&folder) && pane_root.is_none_or(|root| folder.starts_with(root)) {
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
    let account = target.account(name)?;
    // Creation and replacement take an exportable name; deletion keeps
    // working under a legacy reserved name (stint 1682).
    if matches!(mutation, Mutation::Set(_)) {
        super::SecretEnvName::parse(name)?;
    }
    if matches!(mutation, Mutation::Set(value) if value.is_empty()) {
        return Err("Value cannot be empty.".into());
    }
    let operation = match mutation {
        Mutation::Set(_) => "set",
        Mutation::Delete => "delete",
    };
    let resource = target.resource(name);
    let call_id = format!("secret-mutation-{}", uuid::Uuid::new_v4());
    monitor.note_secret_mutation(&call_id, &resource, operation, "requested")?;
    let result = match mutation {
        Mutation::Set(value) => match target {
            SecretTarget::Folder(folder) => {
                super::set_folder_secret(store, name, folder, value).map(|_| ())
            }
            _ => store
                .set(&account, value)
                .map_err(|_| "Secret store write failed; value was not saved.".to_string()),
        },
        Mutation::Delete => match target {
            SecretTarget::Folder(folder) => super::remove_folder_secret(store, name, folder),
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
    if let (SecretTarget::Workspace { root, .. }, Mutation::Set(_)) = (target, mutation) {
        super::write_default_route(root, name, name).map_err(|error| {
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
        if !super::folder::valid_env_name(name) {
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

    /// Stint 1682: a global record under a reserved name moves to a free
    /// name by a create-only, read-back-verified copy that keeps the
    /// original; a taken or reserved target, a failed write and a failed
    /// read-back each leave the original untouched.
    #[test]
    fn copy_moves_a_quarantined_record_without_touching_the_original() {
        let store = InMemoryKeychain::new();
        let target = SecretTarget::Global;
        store.set("plexi:user:LC_PAPER", "paper").unwrap();
        // Reserved or malformed target names are refused before any read.
        for to in ["HOME", "PLEXI_X", "bad-name"] {
            assert!(
                copy_secret(&store, &target, "LC_PAPER", to).is_err(),
                "{to}"
            );
        }
        copy_secret(&store, &target, "LC_PAPER", "PAPER_TOKEN").unwrap();
        assert_eq!(
            store.get("plexi:user:PAPER_TOKEN").unwrap().as_str(),
            "paper"
        );
        assert_eq!(store.get("plexi:user:LC_PAPER").unwrap().as_str(), "paper");
        // A taken name is never overwritten.
        store.set("plexi:user:TAKEN_1682", "mine").unwrap();
        assert!(copy_secret(&store, &target, "LC_PAPER", "TAKEN_1682").is_err());
        assert_eq!(store.get("plexi:user:TAKEN_1682").unwrap().as_str(), "mine");
        // A missing source copies nothing.
        assert!(copy_secret(&store, &target, "LC_TIME", "TIME_TOKEN").is_err());
        assert!(store.get("plexi:user:TIME_TOKEN").is_none());
        // An interrupted copy (the write landed, the read-back did not)
        // reports failure and keeps the original.
        let flaky = InMemoryKeychain::with_unreadable_account("plexi:user:FLAKY_1682");
        flaky.set("plexi:user:LC_PAPER", "paper").unwrap();
        let error = copy_secret(&flaky, &target, "LC_PAPER", "FLAKY_1682").unwrap_err();
        assert!(error.contains("did not read back"), "{error}");
        assert_eq!(flaky.get("plexi:user:LC_PAPER").unwrap().as_str(), "paper");
        // A workspace record moves by route instead.
        let workspace = SecretTarget::Workspace {
            id: "ws".into(),
            root: std::path::PathBuf::from("/tmp"),
        };
        assert!(copy_secret(&store, &workspace, "LC_PAPER", "PAPER_TOKEN").is_err());
    }

    /// Stint 1682: a folder record copied to a free name is what an eligible
    /// pane in that folder then receives; the quarantined original is not.
    #[test]
    fn a_copied_folder_record_is_exported_in_place_of_the_quarantined_one() {
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().into());
        let _enabled =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
        let folder = tempfile::tempdir().unwrap();
        let canon = super::super::canonical_folder(folder.path()).unwrap();
        let store = InMemoryKeychain::new();
        store
            .set(
                &super::super::folder::account_name(&canon, "LC_PAPER"),
                "paper",
            )
            .unwrap();
        let target = SecretTarget::Folder(canon.clone());
        copy_secret(&store, &target, "LC_PAPER", "PAPER_TOKEN").unwrap();
        let env = super::super::env_for_cwd(&canon, &store).unwrap();
        let names: Vec<&str> = env.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["PAPER_TOKEN"]);
        assert_eq!(super::super::list_folder_secrets(&store).len(), 2);
    }

    /// Stint 1682: the inventory names a quarantined record's scope and
    /// remedy from its name alone, reading no value; deletion under the
    /// original name still works and creation under it does not.
    #[test]
    fn inventory_marks_quarantined_records_without_reading_values() {
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().into());
        let store = super::super::system_store();
        let legacy = "LC_MEASUREMENT";
        store
            .set(&super::super::keychain_user_name(legacy), "legacy")
            .unwrap();
        let ok = format!("OK_{}", uuid::Uuid::new_v4().simple());
        store
            .set(&super::super::keychain_user_name(&ok), "fine")
            .unwrap();
        let cwd = tempfile::tempdir().unwrap();
        super::super::store::take_test_value_reads();
        let report = inventory_for_context(cwd.path()).unwrap();
        assert_eq!(super::super::store::take_test_value_reads(), 0);
        let entry = |name: &str| {
            report
                .entries
                .iter()
                .find(|entry| entry.name == name && entry.target == SecretTarget::Global)
                .unwrap_or_else(|| panic!("{name} listed"))
        };
        let remedy = entry(legacy).quarantined.clone().expect("quarantined");
        assert!(
            remedy.contains("secret copy LC_MEASUREMENT NEW --global"),
            "{remedy}"
        );
        assert!(entry(&ok).quarantined.is_none());
        let monitor = PermissionMonitor::for_profile(profile.path()).unwrap();
        assert!(mutate_secret(
            store,
            &monitor,
            &SecretTarget::Global,
            legacy,
            Mutation::Set("new")
        )
        .is_err());
        mutate_secret(
            store,
            &monitor,
            &SecretTarget::Global,
            legacy,
            Mutation::Delete,
        )
        .unwrap();
        store
            .delete(&super::super::keychain_user_name(&ok))
            .unwrap();
    }

    #[test]
    fn fresh_profile_folder_secrets_are_off() {
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().into());
        assert!(!crate::release::feature_enabled(
            crate::release::ReleaseFeature::FolderSecrets
        ));
    }

    #[test]
    fn inventory_outside_pane_scope_still_lists_global_names() {
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().into());
        let own = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let name = format!("PARTIAL_{}", uuid::Uuid::new_v4().simple());
        let store = super::super::system_store();
        let account = super::super::keychain_user_name(&name);
        store.set(&account, "not-for-output").unwrap();
        let _scope = crate::cli::workspace::set_test_pane_context_root(own.path());
        let report = secret_inventory_report(other.path(), false).unwrap();
        assert!(report.entries.iter().any(|entry| entry.name == name));
        assert!(report.entries.iter().all(|entry| entry.scope == "global"));
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["skipped"][0]["reason"], "outside_pane_context");
        assert!(json["skipped"][0]["pane_context_root"].is_string());
        store.delete(&account).unwrap();
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
        let rows = monitor.audit_records();
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
            let rows = monitor.audit_records();
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
        let store = super::super::system_store();
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
            super::super::folder_store(),
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
            super::super::folder_store(),
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
