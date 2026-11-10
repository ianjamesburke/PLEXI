//! One-time, versioned rename of installed configuration. Original packages
//! remain in an inert backup; document state and permission stores never move.
use std::path::Path;

const MIGRATION: &str = "migration-plugin-v2";

pub(crate) fn migrate(root: &Path) -> Result<(), String> {
    if !root.exists() { return Ok(()); }
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true)
        .open(root.join("migration-plugin-v2.lock"))
        .map_err(|error| format!("open plugin migration lock at {}: {error}", root.display()))?;
    lock.lock().map_err(|error| format!("lock plugin migration at {}: {error}", root.display()))?;
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
        return Ok(());
    }
    if !old.exists() && !backup.exists() {
        return Ok(());
    }
    let run = || -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir_all(&journal)?;
        if old.exists() && new.exists() {
            return Err("both apps/ and plugins/ exist; resolve the collision before migration".into());
        }
        if old.exists() {
            if backup.exists() {
                return Err("migration backup already exists while apps/ is present; refusing to overwrite it".into());
            }
            if staged.exists() {
                std::fs::remove_dir_all(&staged)?;
            }
            let output = std::process::Command::new("uv")
                .args(["run", "--offline", "--no-project", "--python", "3.12", "python", "-I", "-c"])
                .arg(include_str!("plugin_source_migration.py"))
                .arg(&old).arg(&staged)
                .stdin(std::process::Stdio::null()).output()?;
            if !output.status.success() {
                return Err(format!("Python source migration failed: {}", String::from_utf8_lossy(&output.stderr)).into());
            }
            for entry in std::fs::read_dir(&staged)? {
                let entry = entry?;
                if entry.file_type()?.is_symlink() {
                    return Err(format!("cannot migrate symlinked package {}; migrate its source explicitly", entry.path().display()).into());
                }
                let path = entry.path().join("manifest.toml");
                if path.is_symlink() { return Err("cannot migrate a symlinked manifest".into()); }
                if !path.is_file() { continue; }
                let raw = std::fs::read_to_string(&path)?;
                let rewritten = migrate_manifest(&raw)?;
                // Validate the destination before moving any original directory.
                let _: super::registry::PluginManifest = toml::from_str(&rewritten)?;
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
        let message = format!("plugin migration v2 at {} failed: {error}", root.display());
        log::error!("{message}");
        message
    })?;
    log::info!("plugin_migration: schema=2 root={} backup={}", root.display(), backup.display());
    Ok(())
}

fn migrate_pack(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let old = root.join("apps.toml");
    if !old.is_file() { return Ok(()); }
    let new = root.join("plugins.toml");
    if new.exists() { return Err("both apps.toml and plugins.toml exist; resolve the collision before migration".into()); }
    let mut value: toml::Value = toml::from_str(&std::fs::read_to_string(&old)?)?;
    let table = value.as_table_mut().ok_or("pack must be a table")?;
    if table.get("schema_version").and_then(toml::Value::as_integer) != Some(1) {
        return Err("only schema_version 1 workspace packs can migrate".into());
    }
    table.insert("schema_version".into(), toml::Value::Integer(2));
    if let Some(entries) = table.remove("app") { table.insert("plugin".into(), entries); }
    let content = toml::to_string_pretty(&value)?;
    let _: super::packs::Pack = toml::from_str(&content)?;
    let journal = root.join(MIGRATION);
    std::fs::create_dir_all(&journal)?;
    let backup = journal.join("apps.toml");
    if backup.exists() { return Err("pack migration backup already exists; refusing to overwrite it".into()); }
    let staged = journal.join("plugins.toml");
    std::fs::write(&staged, content)?;
    // Publish first, then archive the input. A crash leaves an explicit collision,
    // never a missing pack or an implicit fallback to the old parser.
    std::fs::rename(staged, new)?;
    std::fs::rename(old, backup)?;
    log::info!("plugin_pack_migration: schema=2 root={}", root.display());
    Ok(())
}

fn migrate_manifest(raw: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut value: toml::Value = toml::from_str(raw)?;
    if value.get("schema_version").and_then(toml::Value::as_integer) == Some(2) {
        return Ok(raw.to_owned());
    }
    if value.get("schema_version").and_then(toml::Value::as_integer) != Some(1) {
        return Err("only installed schema_version 1 manifests can migrate to [plugin]".into());
    }
    let table = value.as_table_mut().ok_or("manifest must be a table")?;
    let mut plugin = table.remove("app").ok_or("legacy manifest is missing [app]")?;
    let identity = plugin.as_table_mut().ok_or("[app] must be a table")?;
    let id = identity.get("id").and_then(toml::Value::as_str).ok_or("missing legacy app id")?.to_owned();
    let kind = identity.remove("type").ok_or("missing legacy app type")?;
    let kind = match kind.as_str() {
        Some("app") => "python", Some("wasm") => "wasm",
        _ => return Err("unsupported legacy app type".into()),
    };
    let entry = identity.remove("entry").ok_or("missing legacy app entry")?;
    let runtime = table.entry("runtime").or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut().ok_or("runtime must be a table")?;
    runtime.insert("kind".into(), toml::Value::String(kind.into()));
    runtime.insert("entry".into(), entry);
    table.insert("plugin".into(), plugin);
    table.insert("schema_version".into(), toml::Value::Integer(2));
    let mut view = toml::Table::new();
    view.insert("id".into(), toml::Value::String(format!("{id}.main")));
    view.insert("default".into(), toml::Value::Boolean(true));
    let mut contributes = toml::Table::new();
    contributes.insert("views".into(), toml::Value::Array(vec![toml::Value::Table(view)]));
    table.insert("contributes".into(), toml::Value::Table(contributes));
    Ok(toml::to_string_pretty(&value)?)
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
