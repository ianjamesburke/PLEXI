//! One-time, versioned rename of installed configuration. Original packages
//! remain in an inert backup; document state and permission stores never move.
use std::path::Path;

const MIGRATION: &str = "migration-plugin-v2";

/// Migration runs during profile initialization, before `platform::logging::init`
/// installs the file logger, so `log::info!` lines emitted then are dropped.
/// Every trace is also buffered here and replayed by [`replay_early_traces`].
static EARLY_TRACES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn trace(message: String) {
    log::info!("{message}");
    if let Ok(mut buffer) = EARLY_TRACES.lock() {
        buffer.push(message);
    }
}

/// Re-emit migration traces recorded before the logger existed. Call once,
/// immediately after logger initialization.
pub(crate) fn replay_early_traces() {
    let drained = EARLY_TRACES
        .lock()
        .map(|mut buffer| std::mem::take(&mut *buffer))
        .unwrap_or_default();
    for message in drained {
        log::info!("{message} (recorded before logger init)");
    }
}

/// Keys the 0.4.x manifest parser read. That parser ignored every other key in
/// the top-level table and in `[app]`, so such keys were inert; the strict 0.5
/// parser rejects them. Migration drops them (the original stays in the backup)
/// and names each one in its trace.
const LEGACY_TOP_LEVEL_KEYS: &[&str] = &[
    "schema_version",
    "app",
    "launch",
    "runtime",
    "secrets",
    "marketplace",
    "requires",
    "state",
    "cli_operation",
];
const LEGACY_APP_KEYS: &[&str] = &[
    "id",
    "name",
    "entry",
    "type",
    "version",
    "min_sdk_version",
    "description",
    "capabilities",
    "watch",
    "author",
    "tags",
    "repo",
    "dependencies",
];

fn count_entries(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |entries| entries.flatten().count())
}

pub(crate) fn migrate(root: &Path) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("migration-plugin-v2.lock"))
        .map_err(|error| format!("open plugin migration lock at {}: {error}", root.display()))?;
    lock.lock()
        .map_err(|error| format!("lock plugin migration at {}: {error}", root.display()))?;
    migrate_pack(root).map_err(|error| {
        let message = format!("plugin pack migration v2 at {}: {error}", root.display());
        log::error!("{message}");
        message
    })?;
    let old = root.join("apps");
    let new = root.join("plugins");
    let journal = root.join(MIGRATION);
    let backup = journal.join("apps");
    let staged = journal.join("staged");
    let ready = journal.join("ready");
    let done = journal.join("complete");
    if done.is_file() {
        if old.exists() {
            return Err(format!("legacy apps/ reappeared after plugin migration at {}; use plugins/ and schema_version = 2", root.display()));
        }
        return Ok(());
    }
    if !old.exists() && !backup.exists() {
        return Ok(());
    }
    let source_count = count_entries(&old);
    let run = || -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir_all(&journal)?;
        if old.exists() && new.exists() {
            return Err(
                "both apps/ and plugins/ exist; resolve the collision before migration".into(),
            );
        }
        if old.exists() {
            if backup.exists() {
                return Err("migration backup already exists while apps/ is present; refusing to overwrite it".into());
            }
            if staged.exists() {
                std::fs::remove_dir_all(&staged)?;
            }
            let output = std::process::Command::new("uv")
                .args([
                    "run",
                    "--offline",
                    "--no-project",
                    "--python",
                    "3.12",
                    "python",
                    "-I",
                    "-c",
                ])
                .arg(include_str!("plugin_source_migration.py"))
                .arg(&old)
                .arg(&staged)
                .stdin(std::process::Stdio::null())
                .output()?;
            if !output.status.success() {
                return Err(format!(
                    "Python source migration failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )
                .into());
            }
            for entry in std::fs::read_dir(&staged)? {
                let entry = entry?;
                if entry.file_type()?.is_symlink() {
                    return Err(format!(
                        "cannot migrate symlinked package {}; migrate its source explicitly",
                        entry.path().display()
                    )
                    .into());
                }
                let path = entry.path().join("manifest.toml");
                if path.is_symlink() {
                    return Err("cannot migrate a symlinked manifest".into());
                }
                if !path.is_file() {
                    continue;
                }
                let raw = std::fs::read_to_string(&path)?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let (rewritten, dropped) = migrate_manifest(&raw).map_err(|error| {
                    format!(
                        "{} (package {name}): {error}",
                        old.join(&name).join("manifest.toml").display()
                    )
                })?;
                // Validate the destination before moving any original directory.
                super::registry::PluginManifest::parse(&rewritten).map_err(|error| {
                    format!(
                        "{} (package {name}) does not convert to a valid schema 2 manifest: {error}",
                        old.join(&name).join("manifest.toml").display()
                    )
                })?;
                if !dropped.is_empty() {
                    trace(format!(
                        "plugin_migration: package={name} dropped_keys_ignored_by_0.4={} (original kept in backup)",
                        dropped.join(",")
                    ));
                }
                std::fs::write(&path, rewritten)?;
            }
            std::fs::write(&ready, b"schema=2\n")?;
            std::fs::rename(&old, &backup)?;
        }
        if !new.exists() {
            if !ready.is_file() || !staged.is_dir() {
                return Err("interrupted migration has no complete staged tree; original packages remain in migration-plugin-v2/apps".into());
            }
            std::fs::rename(&staged, &new)?;
        }
        std::fs::write(&done, b"schema=2\n")?;
        Ok(())
    };
    run().map_err(|error| {
        // A failed attempt leaves the profile as it found it: no half-built
        // staging tree and no empty journal. The lock file is deliberately kept:
        // it is an inert flock target, and unlinking it would let two processes
        // lock different inodes.
        if !ready.is_file() && staged.exists() {
            if let Err(cleanup) = std::fs::remove_dir_all(&staged) {
                log::warn!(
                    "plugin migration cleanup: cannot remove {}: {cleanup}",
                    staged.display()
                );
            }
        }
        if journal.is_dir() && count_entries(&journal) == 0 {
            if let Err(cleanup) = std::fs::remove_dir(&journal) {
                log::warn!(
                    "plugin migration cleanup: cannot remove {}: {cleanup}",
                    journal.display()
                );
            }
        }
        let message = format!("plugin migration v2 at {} failed: {error}", root.display());
        log::error!("{message}");
        message
    })?;
    trace(format!(
        "plugin_migration: schema=2 source={} destination={} backup={} packages_in={} packages_out={} outcome=ok",
        old.display(),
        new.display(),
        backup.display(),
        source_count,
        count_entries(&new),
    ));
    Ok(())
}

fn migrate_pack(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let old = root.join("apps.toml");
    if !old.is_file() {
        return Ok(());
    }
    let new = root.join("plugins.toml");
    if new.exists() {
        return Err(
            "both apps.toml and plugins.toml exist; resolve the collision before migration".into(),
        );
    }
    let mut value: toml::Value = toml::from_str(&std::fs::read_to_string(&old)?)?;
    let table = value.as_table_mut().ok_or("pack must be a table")?;
    if table
        .get("schema_version")
        .and_then(toml::Value::as_integer)
        != Some(1)
    {
        return Err("only schema_version 1 workspace packs can migrate".into());
    }
    table.insert("schema_version".into(), toml::Value::Integer(2));
    if let Some(entries) = table.remove("app") {
        table.insert("plugin".into(), entries);
    }
    let content = toml::to_string_pretty(&value)?;
    let _: super::packs::Pack = toml::from_str(&content)?;
    let journal = root.join(MIGRATION);
    std::fs::create_dir_all(&journal)?;
    let backup = journal.join("apps.toml");
    if backup.exists() {
        return Err("pack migration backup already exists; refusing to overwrite it".into());
    }
    let staged = journal.join("plugins.toml");
    std::fs::write(&staged, content)?;
    // Publish first, then archive the input. A crash leaves an explicit collision,
    // never a missing pack or an implicit fallback to the old parser.
    std::fs::rename(staged, new)?;
    std::fs::rename(old, &backup)?;
    trace(format!(
        "plugin_pack_migration: schema=2 source={} destination={} backup={} outcome=ok",
        root.join("apps.toml").display(),
        root.join("plugins.toml").display(),
        backup.display(),
    ));
    Ok(())
}

fn migrate_manifest(raw: &str) -> Result<(String, Vec<String>), Box<dyn std::error::Error>> {
    let mut value: toml::Value = toml::from_str(raw)?;
    if value
        .get("schema_version")
        .and_then(toml::Value::as_integer)
        == Some(2)
    {
        return Ok((raw.to_owned(), Vec::new()));
    }
    if value
        .get("schema_version")
        .and_then(toml::Value::as_integer)
        != Some(1)
    {
        return Err("only installed schema_version 1 manifests can migrate to [plugin]".into());
    }
    let table = value.as_table_mut().ok_or("manifest must be a table")?;
    let mut dropped = Vec::new();
    table.retain(|key, _| {
        let keep = LEGACY_TOP_LEVEL_KEYS.contains(&key);
        if !keep {
            dropped.push(key.to_owned());
        }
        keep
    });
    let mut plugin = table
        .remove("app")
        .ok_or("legacy manifest is missing [app]")?;
    let identity = plugin.as_table_mut().ok_or("[app] must be a table")?;
    identity.retain(|key, _| {
        let keep = LEGACY_APP_KEYS.contains(&key);
        if !keep {
            dropped.push(format!("app.{key}"));
        }
        keep
    });
    if let Some(capabilities) = identity
        .get_mut("capabilities")
        .and_then(|value| value.get_mut("capabilities"))
        .and_then(toml::Value::as_array_mut)
    {
        for capability in capabilities {
            if capability.as_str() == Some("spawn.app") {
                *capability = toml::Value::String("spawn.plugin".into());
            }
        }
    }
    let id = identity
        .get("id")
        .and_then(toml::Value::as_str)
        .ok_or("missing legacy app id")?
        .to_owned();
    let kind = identity.remove("type").ok_or("missing legacy app type")?;
    let kind = match kind.as_str() {
        Some("app") => "python",
        Some("wasm") => "wasm",
        _ => return Err("unsupported legacy app type".into()),
    };
    let entry = identity.remove("entry").ok_or("missing legacy app entry")?;
    let runtime = table
        .entry("runtime")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .ok_or("runtime must be a table")?;
    runtime.insert("kind".into(), toml::Value::String(kind.into()));
    runtime.insert("entry".into(), entry);
    table.insert("plugin".into(), plugin);
    table.insert("schema_version".into(), toml::Value::Integer(2));
    let mut view = toml::Table::new();
    view.insert("id".into(), toml::Value::String(format!("{id}.main")));
    view.insert("default".into(), toml::Value::Boolean(true));
    let mut contributes = toml::Table::new();
    contributes.insert(
        "views".into(),
        toml::Value::Array(vec![toml::Value::Table(view)]),
    );
    table.insert("contributes".into(), toml::Value::Table(contributes));
    Ok((toml::to_string_pretty(&value)?, dropped))
}

/// Rewrite a persisted launch path only when its owning config root has an
/// explicit v2 migration receipt. This never searches an alternate data tier.
pub(crate) fn migrated_path(path: &Path) -> Option<std::path::PathBuf> {
    for ancestor in path.ancestors().skip(1) {
        if ancestor.file_name().is_some_and(|name| name == "apps") {
            let root = ancestor.parent()?;
            if root.join(MIGRATION).join("complete").is_file() {
                return Some(root.join("plugins").join(path.strip_prefix(ancestor).ok()?));
            }
        }
    }
    None
}

/// Restore may visit an inactive workspace before its registry is loaded.
/// Migrate only the current channel's explicit config root, never a repository
/// source folder named `apps` or another channel's installed packages.
pub(crate) fn migrate_launch_root(path: &Path) -> Result<(), String> {
    let channel = crate::config::workspace_channel_dir();
    for ancestor in path.ancestors().skip(1) {
        if ancestor.file_name().is_some_and(|name| name == "apps") {
            if let Some(root) = ancestor.parent() {
                if root
                    .file_name()
                    .is_some_and(|name| name == channel.as_str())
                {
                    migrate(root)?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_rename_restore_migrates_inactive_workspace_but_not_source_apps() {
        let workspace = tempfile::tempdir().unwrap();
        let config = workspace
            .path()
            .join(crate::config::workspace_channel_dir());
        let legacy = config.join("apps/notes");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("manifest.toml"), "schema_version = 1\n[app]\nid = 'notes'\nname = 'Notes'\ntype = 'app'\nentry = 'main.py'\n[app.capabilities]\ncapabilities = ['spawn.app']\n").unwrap();
        migrate_launch_root(&legacy).unwrap();
        let migrated = migrated_path(&legacy).unwrap();
        let manifest: super::super::registry::PluginManifest =
            toml::from_str(&std::fs::read_to_string(migrated.join("manifest.toml")).unwrap())
                .unwrap();
        assert_eq!(
            manifest.plugin.capabilities.capabilities,
            vec!["spawn.plugin"]
        );
        let source = workspace.path().join("apps/source");
        std::fs::create_dir_all(&source).unwrap();
        migrate_launch_root(&source).unwrap();
        assert!(source.exists());
        assert!(migrated_path(&source).is_none());
    }

    #[test]
    fn plugin_rename_pack_migration_preserves_pin_and_keeps_original() {
        let root = tempfile::tempdir().unwrap();
        let original = "schema_version = 1\n[[app]]\nid = 'notes'\nsource = 'github:example/notes'\nversion = 'v0.4.4'\n";
        std::fs::write(root.path().join("apps.toml"), original).unwrap();
        migrate(root.path()).unwrap();
        migrate(root.path()).unwrap();
        let pack = super::super::packs::Pack::from_path(&root.path().join("plugins.toml")).unwrap();
        assert_eq!(pack.plugins[0].version, "v0.4.4");
        assert_eq!(
            std::fs::read_to_string(root.path().join(MIGRATION).join("apps.toml")).unwrap(),
            original
        );
    }

    #[test]
    fn plugin_rename_tree_migration_records_trace_with_counts() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("apps/notes");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("manifest.toml"), "schema_version = 1\n[app]\nid = 'notes'\nname = 'Notes'\ntype = 'app'\nentry = 'main.py'\n").unwrap();
        std::fs::write(legacy.join("main.py"), "").unwrap();
        migrate(root.path()).unwrap();
        assert!(root.path().join("plugins/notes/manifest.toml").is_file());
        assert!(root.path().join(MIGRATION).join("apps/notes").is_dir());
        let traces = EARLY_TRACES.lock().unwrap();
        let line = traces
            .iter()
            .find(|line| line.contains(&root.path().join("apps").display().to_string()))
            .expect("migration must record a trace");
        assert!(
            line.contains("packages_in=1 packages_out=1 outcome=ok"),
            "{line}"
        );
        assert!(line.contains("destination="), "{line}");
    }

    const LEGACY_WITH_INERT_KEYS: &str = "schema_version = 1\nextra_note = 'x'\n[app]\nid = 'logs'\nname = 'Logs'\ntype = 'app'\nentry = 'main.py'\nmouse_tracking = true\n[runtime]\npython_compat = true\n[launch]\non_launch = 'focus'\n";

    fn stage_legacy(root: &Path, id: &str, manifest: &str) {
        let dir = root.join("apps").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(dir.join("main.py"), "").unwrap();
    }

    /// 0.4.x ignored unknown keys in the top level and `[app]` (the owner's
    /// bundled Logs manifest carries `mouse_tracking`); migration must not die on them.
    #[test]
    fn plugin_rename_migrates_manifest_with_keys_the_old_parser_ignored() {
        let root = tempfile::tempdir().unwrap();
        stage_legacy(root.path(), "logs", LEGACY_WITH_INERT_KEYS);
        migrate(root.path()).unwrap();
        let migrated =
            std::fs::read_to_string(root.path().join("plugins/logs/manifest.toml")).unwrap();
        assert!(!migrated.contains("mouse_tracking") && !migrated.contains("extra_note"));
        super::super::registry::PluginManifest::parse(&migrated).unwrap();
        let backup =
            std::fs::read_to_string(root.path().join(MIGRATION).join("apps/logs/manifest.toml"))
                .unwrap();
        assert!(
            backup.contains("mouse_tracking"),
            "original must be kept verbatim"
        );
        let traces = EARLY_TRACES.lock().unwrap();
        assert!(traces.iter().any(|line| line.contains("package=logs")
            && line.contains("app.mouse_tracking")
            && line.contains("extra_note")));
    }

    #[test]
    fn plugin_rename_failed_attempt_names_file_and_leaves_profile_clean() {
        let root = tempfile::tempdir().unwrap();
        stage_legacy(root.path(), "good", LEGACY_WITH_INERT_KEYS);
        stage_legacy(
            root.path(),
            "bad",
            "schema_version = 1\n[app]\nid = 'bad'\nname = 'Bad'\nentry = 'main.py'\n",
        );
        let error = migrate(root.path()).unwrap_err();
        assert!(
            error.contains("apps/bad/manifest.toml") && error.contains("package bad"),
            "{error}"
        );
        assert!(error.contains("missing legacy app type"), "{error}");
        assert!(root.path().join("apps/good/manifest.toml").is_file());
        assert!(root.path().join("apps/bad/manifest.toml").is_file());
        assert!(!root.path().join("plugins").exists());
        assert!(
            !root.path().join(MIGRATION).exists(),
            "journal and staging must be removed"
        );
    }

    /// A lock file and stale staging tree left by a killed attempt must not
    /// block or corrupt the next attempt.
    #[test]
    fn plugin_rename_retry_after_leftover_lock_and_staged_succeeds() {
        let root = tempfile::tempdir().unwrap();
        stage_legacy(root.path(), "logs", LEGACY_WITH_INERT_KEYS);
        let journal = root.path().join(MIGRATION);
        std::fs::create_dir_all(journal.join("staged/junk")).unwrap();
        std::fs::write(journal.join("staged/junk/stale.txt"), "stale").unwrap();
        std::fs::write(root.path().join("migration-plugin-v2.lock"), "").unwrap();
        migrate(root.path()).unwrap();
        assert!(root.path().join("plugins/logs/manifest.toml").is_file());
        assert!(
            !root.path().join("plugins/junk").exists(),
            "stale staging must not leak"
        );
        assert!(journal.join("complete").is_file());
    }

    #[test]
    fn plugin_rename_collision_preserves_both_trees() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("apps")).unwrap();
        std::fs::create_dir(root.path().join("plugins")).unwrap();
        assert!(migrate(root.path())
            .unwrap_err()
            .contains("both apps/ and plugins/"));
        assert!(root.path().join("apps").is_dir());
        assert!(root.path().join("plugins").is_dir());
        assert!(!root.path().join(MIGRATION).join("complete").exists());
    }
}
