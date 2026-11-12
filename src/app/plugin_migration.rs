//! One-time, versioned rename of installed configuration. Original packages
//! remain in an inert backup; document state and permission stores never move.
use std::path::{Path, PathBuf};

const MIGRATION: &str = "migration-plugin-v2";
const OPERATION_TREE: &str = "plugin migration v2";
const OPERATION_PACK: &str = "plugin pack migration v2";

/// Migration runs during profile initialization, before `platform::logging::init`
/// installs the file logger, so lines logged then are dropped. Every trace is
/// also buffered here with its level and replayed by [`replay_early_traces`].
static EARLY_TRACES: std::sync::Mutex<Vec<(log::Level, String)>> =
    std::sync::Mutex::new(Vec::new());

fn record(level: log::Level, message: String) {
    log::log!(level, "{message}");
    EARLY_TRACES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push((level, message));
}

fn trace(message: String) {
    record(log::Level::Info, message);
}

/// Re-emit migration traces recorded before the logger existed. Call once,
/// immediately after logger initialization.
pub(crate) fn replay_early_traces() {
    let drained = std::mem::take(
        &mut *EARLY_TRACES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    replay_into(drained, |level, message| log::log!(level, "{message}"));
}

fn replay_into(entries: Vec<(log::Level, String)>, mut emit: impl FnMut(log::Level, &str)) {
    for (level, message) in entries {
        emit(level, &format!("{message} (recorded before logger init)"));
    }
}

/// A migration that did not complete. Carries what the user needs to fix it:
/// the profile, the package and manifest at fault, why it failed, and the fix.
/// `reason` never quotes file contents, only parser messages and line numbers.
#[derive(Debug, Clone)]
pub(crate) struct MigrationFailure {
    pub(crate) operation: &'static str,
    pub(crate) profile_dir: PathBuf,
    pub(crate) package: Option<String>,
    pub(crate) file: Option<PathBuf>,
    pub(crate) reason: String,
    pub(crate) hint: Option<String>,
    /// True when the failed attempt left every original package where it was.
    pub(crate) unchanged: bool,
}

impl MigrationFailure {
    fn new(operation: &'static str, profile_dir: &Path, reason: impl Into<String>) -> Self {
        Self {
            operation,
            profile_dir: profile_dir.to_path_buf(),
            package: None,
            file: None,
            reason: reason.into(),
            hint: None,
            unchanged: false,
        }
    }

    fn package(mut self, name: &str) -> Self {
        self.package = Some(name.to_owned());
        self
    }

    fn file(mut self, file: PathBuf) -> Self {
        self.file = Some(file);
        self
    }

    fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// The structured log line: operation, profile, package, file, reason, fix.
    fn log_line(&self) -> String {
        let package = self.package.as_deref().unwrap_or("none");
        let file = self
            .file
            .as_ref()
            .map_or_else(|| "none".to_owned(), |file| file.display().to_string());
        let mut line = format!(
            "plugin_migration: outcome=failed operation=\"{}\" profile={} package={package} file={file} unchanged={} reason={:?}",
            self.operation,
            self.profile_dir.display(),
            self.unchanged,
            self.reason,
        );
        if let Some(hint) = &self.hint {
            line.push_str(&format!(" fix={hint:?}"));
        }
        line
    }
}

impl std::fmt::Display for MigrationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} at {} failed: ",
            self.operation,
            self.profile_dir.display()
        )?;
        match (&self.file, &self.package) {
            (Some(file), Some(package)) => write!(f, "{} (package {package}): ", file.display())?,
            (Some(file), None) => write!(f, "{}: ", file.display())?,
            _ => {}
        }
        write!(f, "{}", self.reason)?;
        if let Some(hint) = &self.hint {
            write!(f, "\nfix: {hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for MigrationFailure {}

impl From<Box<MigrationFailure>> for String {
    fn from(failure: Box<MigrationFailure>) -> Self {
        failure.to_string()
    }
}

/// Why one legacy manifest cannot become a schema 2 manifest. Each class owns
/// its fix; none carries manifest contents.
#[derive(Debug)]
enum ManifestFault {
    Syntax {
        line: Option<usize>,
        message: String,
    },
    UnsupportedSchema,
    MissingApp,
    NotATable(&'static str),
    MissingId,
    MissingType,
    UnsupportedType,
    MissingEntry,
    Unwritable(String),
}

impl ManifestFault {
    fn syntax(raw: &str, error: &toml::de::Error) -> Self {
        Self::Syntax {
            line: error_line(raw, error),
            message: error.message().to_owned(),
        }
    }

    fn reason(&self) -> String {
        match self {
            Self::Syntax {
                line: Some(line),
                message,
            } => format!("invalid TOML at line {line}: {message}"),
            Self::Syntax {
                line: None,
                message,
            } => format!("invalid TOML: {message}"),
            Self::UnsupportedSchema => {
                "only installed schema_version 1 manifests can migrate to [plugin]".into()
            }
            Self::MissingApp => "legacy manifest is missing [app]".into(),
            Self::NotATable(name) => format!("{name} must be a table"),
            Self::MissingId => "missing legacy app id".into(),
            Self::MissingType => "missing legacy app type".into(),
            Self::UnsupportedType => "unsupported legacy app type".into(),
            Self::MissingEntry => "missing legacy app entry".into(),
            Self::Unwritable(message) => format!("cannot write the converted manifest: {message}"),
        }
    }

    /// The edit that fixes this class, before the shared "or move it out" tail.
    fn fix(&self) -> String {
        match self {
            Self::Syntax {
                line: Some(line), ..
            } => format!("correct the TOML syntax at line {line} of the named manifest"),
            Self::Syntax { line: None, .. } => {
                "correct the TOML syntax of the named manifest".into()
            }
            Self::UnsupportedSchema => {
                "set `schema_version = 1` at the top of the named manifest".into()
            }
            Self::MissingApp => {
                "add an `[app]` table with id, name, type and entry to the named manifest".into()
            }
            Self::NotATable(name) => format!("make `{name}` a table in the named manifest"),
            Self::MissingId => "add `id = \"<plugin id>\"` to `[app]` in the named manifest".into(),
            Self::MissingType => "add `type = \"app\"` to `[app]` in the named manifest".into(),
            Self::UnsupportedType => {
                "set `type = \"app\"` (or `\"wasm\"`) in `[app]` of the named manifest".into()
            }
            Self::MissingEntry => {
                "add `entry = \"<file>\"`, the plugin's entry file, to `[app]` in the named manifest"
                    .into()
            }
            Self::Unwritable(_) => {
                "correct the manifest structure named in the reason".into()
            }
        }
    }
}

/// The 1-based line a parser error points at, from its byte span.
fn error_line(raw: &str, error: &toml::de::Error) -> Option<usize> {
    let start = error.span()?.start.min(raw.len());
    Some(
        raw.as_bytes()[..start]
            .iter()
            .filter(|b| **b == b'\n')
            .count()
            + 1,
    )
}

fn move_out_hint(fix: &str, package: &str) -> String {
    format!("{fix}, or move package `{package}` out of apps/, then relaunch.")
}

fn package_failure(
    root: &Path,
    old: &Path,
    package: &str,
    reason: String,
    fix: &str,
) -> MigrationFailure {
    MigrationFailure::new(OPERATION_TREE, root, reason)
        .package(package)
        .file(old.join(package).join("manifest.toml"))
        .hint(move_out_hint(fix, package))
}

fn count_entries(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |entries| entries.flatten().count())
}

/// Convert a failed attempt into the single place failures are logged. The
/// failure is also buffered so it reaches `plexi.log` when it happened before
/// the logger existed.
pub(crate) fn migrate(root: &Path) -> Result<(), Box<MigrationFailure>> {
    migrate_profile(root).inspect_err(|failure| record(log::Level::Error, failure.log_line()))
}

fn migrate_profile(root: &Path) -> Result<(), Box<MigrationFailure>> {
    if !root.exists() {
        return Ok(());
    }
    let lock_path = root.join("migration-plugin-v2.lock");
    let lock_failure = |action: &str, error: std::io::Error| {
        MigrationFailure::new(OPERATION_TREE, root, format!("{action}: {error}"))
            .file(lock_path.clone())
    };
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|error| Box::new(lock_failure("cannot open the migration lock", error)))?;
    lock.lock()
        .map_err(|error| Box::new(lock_failure("cannot lock the migration", error)))?;
    migrate_pack(root).map_err(|error| {
        let mut failure = into_failure(OPERATION_PACK, root, error);
        failure.unchanged = root.join("apps.toml").is_file() && !root.join("plugins.toml").exists();
        failure
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
            let mut failure = MigrationFailure::new(
                OPERATION_TREE,
                root,
                "legacy apps/ reappeared after plugin migration; use plugins/ and schema_version = 2",
            )
            .file(old.clone())
            .hint("move the packages you want into plugins/ with a schema_version = 2 manifest, then move apps/ out of the profile and relaunch.");
            failure.unchanged = true;
            return Err(Box::new(failure));
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
            return Err(MigrationFailure::new(
                OPERATION_TREE,
                root,
                "both apps/ and plugins/ exist; resolve the collision before migration",
            )
            .hint(format!(
                "move whichever of {} or {} you do not want out of the profile, then relaunch.",
                old.display(),
                new.display()
            ))
            .into());
        }
        if old.exists() {
            if backup.exists() {
                return Err(MigrationFailure::new(
                    OPERATION_TREE,
                    root,
                    "migration backup already exists while apps/ is present; refusing to overwrite it",
                )
                .file(backup.clone())
                .hint(format!(
                    "check that {} holds the packages you expect, move it out of the profile, then relaunch.",
                    backup.display()
                ))
                .into());
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
                .output()
                .map_err(|error| {
                    MigrationFailure::new(
                        OPERATION_TREE,
                        root,
                        format!("cannot run `uv` for the source migration: {error}"),
                    )
                    .hint("install uv and make sure it is on PATH, then relaunch.")
                })?;
            if !output.status.success() {
                // The last stderr line is the exception message; earlier lines
                // of a Python traceback quote source code.
                let stderr = String::from_utf8_lossy(&output.stderr);
                let summary: String = stderr
                    .lines()
                    .rev()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("no error output")
                    .chars()
                    .take(300)
                    .collect();
                return Err(MigrationFailure::new(
                    OPERATION_TREE,
                    root,
                    format!("Python source migration failed: {summary}"),
                )
                .file(old.clone())
                .into());
            }
            for entry in std::fs::read_dir(&staged)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.file_type()?.is_symlink() {
                    return Err(package_failure(
                        root,
                        &old,
                        &name,
                        "cannot migrate a symlinked package; migrate its source explicitly".into(),
                        "replace the symlink with a real copy of the package",
                    )
                    .into());
                }
                let path = entry.path().join("manifest.toml");
                if path.is_symlink() {
                    return Err(package_failure(
                        root,
                        &old,
                        &name,
                        "cannot migrate a symlinked manifest".into(),
                        "replace the symlink with a real manifest file",
                    )
                    .into());
                }
                if !path.is_file() {
                    continue;
                }
                let raw = std::fs::read_to_string(&path)?;
                let (rewritten, dropped) = migrate_manifest(&raw).map_err(|fault| {
                    package_failure(root, &old, &name, fault.reason(), &fault.fix())
                })?;
                // Validate the destination before moving any original directory.
                toml::from_str::<super::registry::PluginManifest>(&rewritten).map_err(|error| {
                    package_failure(
                        root,
                        &old,
                        &name,
                        format!(
                            "does not convert to a valid schema 2 manifest: {}",
                            error.message()
                        ),
                        "correct the field named in the reason in the named manifest",
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
                return Err(MigrationFailure::new(
                    OPERATION_TREE,
                    root,
                    "interrupted migration has no complete staged tree; original packages remain in migration-plugin-v2/apps",
                )
                .file(backup.clone())
                .hint(format!(
                    "move {} back to {}, then relaunch to retry.",
                    backup.display(),
                    old.display()
                ))
                .into());
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
        let mut failure = into_failure(OPERATION_TREE, root, error);
        failure.unchanged = old.exists() && !backup.exists();
        failure
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

/// Keep a classified failure as is; wrap anything else (an I/O error) with the
/// operation and profile so it is still attributable.
fn into_failure(
    operation: &'static str,
    root: &Path,
    error: Box<dyn std::error::Error>,
) -> Box<MigrationFailure> {
    error
        .downcast::<MigrationFailure>()
        .unwrap_or_else(|other| Box::new(MigrationFailure::new(operation, root, other.to_string())))
}

fn migrate_pack(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let old = root.join("apps.toml");
    if !old.is_file() {
        return Ok(());
    }
    let new = root.join("plugins.toml");
    let fail = |reason: String, hint: String| -> Box<dyn std::error::Error> {
        MigrationFailure::new(OPERATION_PACK, root, reason)
            .file(old.clone())
            .hint(hint)
            .into()
    };
    if new.exists() {
        return Err(fail(
            "both apps.toml and plugins.toml exist; resolve the collision before migration".into(),
            format!(
                "move whichever of {} or {} you do not want out of the profile, then relaunch.",
                old.display(),
                new.display()
            ),
        ));
    }
    let raw = std::fs::read_to_string(&old)?;
    let mut value: toml::Value = toml::from_str(&raw).map_err(|error| {
        fail(
            ManifestFault::syntax(&raw, &error).reason(),
            "correct the TOML syntax of apps.toml, then relaunch.".into(),
        )
    })?;
    let table = value.as_table_mut().ok_or_else(|| {
        fail(
            "pack must be a table".into(),
            "make apps.toml a TOML table, then relaunch.".into(),
        )
    })?;
    if table
        .get("schema_version")
        .and_then(toml::Value::as_integer)
        != Some(1)
    {
        return Err(fail(
            "only schema_version 1 workspace packs can migrate".into(),
            "set `schema_version = 1` at the top of apps.toml, then relaunch.".into(),
        ));
    }
    table.insert("schema_version".into(), toml::Value::Integer(2));
    if let Some(entries) = table.remove("app") {
        table.insert("plugin".into(), entries);
    }
    let content = toml::to_string_pretty(&value)?;
    toml::from_str::<super::packs::Pack>(&content).map_err(|error| {
        fail(
            format!("pack does not convert to schema 2: {}", error.message()),
            "correct the `[[app]]` entry named in the reason in apps.toml, then relaunch.".into(),
        )
    })?;
    let journal = root.join(MIGRATION);
    std::fs::create_dir_all(&journal)?;
    let backup = journal.join("apps.toml");
    if backup.exists() {
        return Err(fail(
            "pack migration backup already exists; refusing to overwrite it".into(),
            format!(
                "check {} holds the pack you expect, move it out of the profile, then relaunch.",
                backup.display()
            ),
        ));
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

fn migrate_manifest(raw: &str) -> Result<(String, Vec<String>), ManifestFault> {
    let mut value: toml::Value =
        toml::from_str(raw).map_err(|error| ManifestFault::syntax(raw, &error))?;
    let schema = value
        .get("schema_version")
        .and_then(toml::Value::as_integer);
    if schema == Some(2) {
        return Ok((raw.to_owned(), Vec::new()));
    }
    if schema != Some(1) {
        return Err(ManifestFault::UnsupportedSchema);
    }
    let table = value
        .as_table_mut()
        .ok_or(ManifestFault::NotATable("the manifest"))?;
    let mut dropped = Vec::new();
    table.retain(|key, _| {
        let keep = LEGACY_TOP_LEVEL_KEYS.contains(&key);
        if !keep {
            dropped.push(key.to_owned());
        }
        keep
    });
    let mut plugin = table.remove("app").ok_or(ManifestFault::MissingApp)?;
    let identity = plugin
        .as_table_mut()
        .ok_or(ManifestFault::NotATable("[app]"))?;
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
        .ok_or(ManifestFault::MissingId)?
        .to_owned();
    let kind = identity.remove("type").ok_or(ManifestFault::MissingType)?;
    let kind = match kind.as_str() {
        Some("app") => "python",
        Some("wasm") => "wasm",
        _ => return Err(ManifestFault::UnsupportedType),
    };
    let entry = identity
        .remove("entry")
        .ok_or(ManifestFault::MissingEntry)?;
    let runtime = table
        .entry("runtime")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .ok_or(ManifestFault::NotATable("runtime"))?;
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
    let rewritten = toml::to_string_pretty(&value)
        .map_err(|error| ManifestFault::Unwritable(error.to_string()))?;
    Ok((rewritten, dropped))
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
            .map(|(_, line)| line)
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
        assert!(traces.iter().any(|(_, line)| line.contains("package=logs")
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
        let error = migrate(root.path()).unwrap_err().to_string();
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

    const MISSING_TYPE: &str =
        "schema_version = 1\n[app]\nid = 'bad'\nname = 'Bad'\nentry = 'main.py'\n";

    #[test]
    fn plugin_rename_failure_is_recorded_for_log_replay_with_package_and_file() {
        let root = tempfile::tempdir().unwrap();
        stage_legacy(root.path(), "bad", MISSING_TYPE);
        migrate(root.path()).unwrap_err();
        let manifest = root.path().join("apps/bad/manifest.toml");
        let traces = EARLY_TRACES.lock().unwrap();
        let (level, line) = traces
            .iter()
            .find(|(_, line)| line.contains(&manifest.display().to_string()))
            .expect("a failed migration must be buffered for replay after logger init");
        assert_eq!(*level, log::Level::Error);
        assert!(line.contains(&root.path().display().to_string()), "{line}");
        assert!(line.contains("outcome=failed"), "{line}");
        assert!(line.contains("package=bad"), "{line}");
        assert!(line.contains("missing legacy app type"), "{line}");
    }

    #[test]
    fn plugin_rename_missing_type_error_tells_the_user_how_to_fix_it() {
        let root = tempfile::tempdir().unwrap();
        stage_legacy(root.path(), "bad", MISSING_TYPE);
        let error = migrate(root.path()).unwrap_err().to_string();
        assert!(error.contains("type = \"app\""), "{error}");
        assert!(
            error.contains("or move package `bad` out of apps/"),
            "{error}"
        );
    }

    #[test]
    fn plugin_rename_failure_never_echoes_manifest_contents() {
        let root = tempfile::tempdir().unwrap();
        stage_legacy(
            root.path(),
            "bad",
            "schema_version = 1\n[app]\nid = 'bad'\nname = 'CANARY-CONTENT' entry\n",
        );
        let error = migrate(root.path()).unwrap_err().to_string();
        assert!(error.contains("package bad"), "{error}");
        assert!(!error.contains("CANARY-CONTENT"), "{error}");
        let traces = EARLY_TRACES.lock().unwrap();
        assert!(
            !traces
                .iter()
                .any(|(_, line)| line.contains("CANARY-CONTENT")),
            "log lines must not carry manifest contents"
        );
    }

    fn failure_for(manifest: &str) -> Box<MigrationFailure> {
        let root = tempfile::tempdir().unwrap();
        stage_legacy(root.path(), "bad", manifest);
        migrate(root.path()).unwrap_err()
    }

    #[test]
    fn plugin_rename_every_manifest_fault_class_has_its_own_fix() {
        let cases = [
            ("schema_version = 1\n[app\n", "correct the TOML syntax at line 2"),
            ("name = 'x'\n", "set `schema_version = 1`"),
            ("schema_version = 1\n", "add an `[app]` table"),
            ("schema_version = 1\napp = 3\n", "make `[app]` a table"),
            (
                "schema_version = 1\n[app]\nname = 'B'\ntype = 'app'\nentry = 'main.py'\n",
                "add `id =",
            ),
            (MISSING_TYPE, "add `type = \"app\"` to `[app]`"),
            (
                "schema_version = 1\n[app]\nid = 'bad'\ntype = 'rust'\nentry = 'main.py'\n",
                "set `type = \"app\"` (or `\"wasm\"`)",
            ),
            (
                "schema_version = 1\n[app]\nid = 'bad'\ntype = 'app'\n",
                "add `entry =",
            ),
            (
                "schema_version = 1\nruntime = 3\n[app]\nid = 'bad'\ntype = 'app'\nentry = 'main.py'\n",
                "make `runtime` a table",
            ),
        ];
        for (manifest, fix) in cases {
            let failure = failure_for(manifest);
            let hint = failure.hint.as_deref().expect("every class has a fix");
            assert!(hint.contains(fix), "{manifest:?}: {hint}");
            assert!(
                hint.contains("or move package `bad` out of apps/, then relaunch."),
                "{hint}"
            );
            assert!(failure.unchanged, "{manifest:?}");
            assert_eq!(failure.package.as_deref(), Some("bad"));
        }
    }

    #[test]
    fn plugin_rename_failure_display_carries_the_fix_for_the_cli() {
        let rendered = failure_for(MISSING_TYPE).to_string();
        assert!(
            rendered.contains("(package bad): missing legacy app type"),
            "{rendered}"
        );
        assert!(
            rendered.contains("\nfix: add `type = \"app\"`"),
            "{rendered}"
        );
    }

    #[test]
    fn plugin_rename_converted_manifest_that_fails_schema_2_names_the_field() {
        let failure = failure_for(
            "schema_version = 1\n[app]\nid = 'bad'\ntype = 'app'\nentry = 'main.py'\n[runtime]\nkind = 7\n",
        );
        assert!(
            failure
                .reason
                .starts_with("does not convert to a valid schema 2 manifest: "),
            "{}",
            failure.reason
        );
        assert!(failure
            .hint
            .unwrap()
            .contains("correct the field named in the reason"));
    }

    #[test]
    fn plugin_rename_pack_failure_names_apps_toml_and_leaves_it_in_place() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("apps.toml"), "schema_version = 7\n").unwrap();
        let failure = migrate(root.path()).unwrap_err();
        assert_eq!(failure.operation, OPERATION_PACK);
        assert_eq!(failure.file, Some(root.path().join("apps.toml")));
        assert!(failure.unchanged);
        assert!(failure.hint.unwrap().contains("schema_version = 1"));
        assert!(root.path().join("apps.toml").is_file());
        assert!(!root.path().join("plugins.toml").exists());
    }

    #[test]
    fn plugin_rename_collision_failure_is_unchanged_and_says_which_to_move() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("apps")).unwrap();
        std::fs::create_dir(root.path().join("plugins")).unwrap();
        let failure = migrate(root.path()).unwrap_err();
        assert!(failure.unchanged);
        assert!(failure.hint.unwrap().contains("move whichever of"));
    }

    #[test]
    fn plugin_rename_replay_marks_entries_and_keeps_their_level() {
        let mut seen = Vec::new();
        replay_into(
            vec![
                (log::Level::Info, "ok line".to_owned()),
                (log::Level::Error, "bad line".to_owned()),
            ],
            |level, message| seen.push((level, message.to_owned())),
        );
        assert_eq!(
            seen,
            vec![
                (
                    log::Level::Info,
                    "ok line (recorded before logger init)".to_owned()
                ),
                (
                    log::Level::Error,
                    "bad line (recorded before logger init)".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn plugin_rename_collision_preserves_both_trees() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("apps")).unwrap();
        std::fs::create_dir(root.path().join("plugins")).unwrap();
        assert!(migrate(root.path())
            .unwrap_err()
            .to_string()
            .contains("both apps/ and plugins/"));
        assert!(root.path().join("apps").is_dir());
        assert!(root.path().join("plugins").is_dir());
        assert!(!root.path().join(MIGRATION).join("complete").exists());
    }
}
