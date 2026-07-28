use super::open::pane_new_cli;
use crate::host::scheduler::{parse_schedule, routines_file, RoutinesConfig};
use std::path::{Path, PathBuf};

/// Accepted `--schedule` grammar, printed whenever a schedule fails to parse.
/// Mirrors `crate::host::scheduler::parse_schedule` — the same parser the
/// scheduler fires on, so a written schedule can never be silently dropped.
const SCHEDULE_FORMS_HELP: &str = "Accepted schedule forms:
  every N seconds|minutes|hours    (every 30s / every 5 minutes / every 2 hours)
  every minute / every hour
  daily at HH:MM                   (daily at 09:00 / daily at 9am)
  weekdays at HH:MM                (weekdays at 09:00)
  weekends at HH:MM                (weekends at 10:30am)
  weekly on <day> at HH:MM         (weekly on monday at 09:00)
  monthly on N at HH:MM            (monthly on 1 at 08:00)
  5-field cron: m h dom mon dow    (0 9 * * 1-5)";

/// A failure while reading or mutating `routines.toml`. Wrapped by the CLI
/// verbs into user-facing messages; kept typed so tests can assert the cause.
#[derive(Debug)]
enum RoutineFileError {
    /// The schedule string is not accepted by `parse_schedule`.
    BadSchedule(String),
    /// `routine add` with a name that already exists in the file.
    DuplicateName(String),
    /// A mutating verb named a routine that is not in the file.
    UnknownName { name: String, available: Vec<String> },
    /// I/O or parse failure — message names the path and the operation.
    Io(String),
}

impl std::fmt::Display for RoutineFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoutineFileError::BadSchedule(s) => {
                write!(f, "schedule '{s}' is not a recognized format")
            }
            RoutineFileError::DuplicateName(name) => {
                write!(f, "a routine named '{name}' already exists")
            }
            RoutineFileError::UnknownName { name, available } => {
                if available.is_empty() {
                    write!(f, "routine '{name}' not found (no routines defined)")
                } else {
                    write!(
                        f,
                        "routine '{name}' not found. Available routines: {}",
                        available.join(", ")
                    )
                }
            }
            RoutineFileError::Io(msg) => write!(f, "{msg}"),
        }
    }
}

/// Resolve the workspace root that owns `routines.toml`, walking up from the
/// current directory like every other workspace-scoped CLI resolution
/// (stint 0574). `None` means no workspace was found.
fn workspace_root() -> Option<PathBuf> {
    crate::config::active_workspace_root()
}

fn print_no_workspace_error() {
    eprintln!("error: no workspace found from the current directory");
    eprintln!("Run `plexi workspace init` in your project root first.");
}

/// Parse `routines.toml` at `path` into the scheduler-owned config type.
fn load_config(path: &Path) -> Result<RoutinesConfig, String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    toml::from_str(&contents).map_err(|e| format!("failed to parse {}: {e}", path.display()))
}

pub fn routine_list() -> i32 {
    log::info!("cli: routine list");
    let rf = routines_file();
    let Some(root) = workspace_root() else {
        println!("No workspace found — routines live in {rf} at a workspace root.");
        println!("Run `plexi workspace init` in your project root first.");
        return 0;
    };
    let config_path = root.join(&rf);
    let config = match std::fs::read_to_string(&config_path) {
        Ok(contents) => match toml::from_str::<RoutinesConfig>(&contents) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error: failed to parse {}: {e}", config_path.display());
                return 1;
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("No routines configured.");
            println!();
            println!(
                "To set up routines, run `plexi routine add`, or create {} in workspace {}:",
                rf,
                root.display()
            );
            println!("  [[routine]]");
            println!("  name = \"morning-sync\"");
            println!("  command = \"./scripts/morning.sh\"");
            println!("  schedule = \"weekdays at 9am\"");
            println!("  context = \"work\"");
            return 0;
        }
        Err(e) => {
            eprintln!("error: could not read {}: {e}", config_path.display());
            return 1;
        }
    };
    if config.routine.is_empty() {
        println!("No routines defined in {rf}.");
        return 0;
    }
    println!("Routines:");
    for r in &config.routine {
        let next = match parse_schedule(&r.schedule) {
            Some(s) => crate::host::scheduler::next_fire_description(&s, None),
            None => "invalid schedule".to_string(),
        };
        let ctx_label = if r.context.is_empty() {
            "(active context)".to_string()
        } else {
            r.context.clone()
        };
        let ephemeral_label = if r.ephemeral { " [ephemeral]" } else { "" };
        let disabled_label = if r.enabled { "" } else { " [disabled]" };
        println!(
            "  {:20} {:<30} next: {}  context: {}{}{}",
            r.name, r.schedule, next, ctx_label, ephemeral_label, disabled_label
        );
    }
    0
}

/// `plexi routine run <name>` — manually fire a routine
pub fn routine_run(name: &str, force: bool) -> i32 {
    log::info!("cli: routine run '{name}' force={force}");
    let rf = routines_file();
    let Some(root) = workspace_root() else {
        print_no_workspace_error();
        return 1;
    };
    let config_path = root.join(&rf);
    let config = match load_config(&config_path) {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("error: {msg}");
            return 1;
        }
    };
    let routine = match config.routine.iter().find(|r| r.name == name) {
        Some(r) => r,
        None => {
            eprintln!("error: routine '{name}' not found in {rf}");
            if !config.routine.is_empty() {
                let names: Vec<&str> = config.routine.iter().map(|r| r.name.as_str()).collect();
                eprintln!("Available routines: {}", names.join(", "));
            }
            return 1;
        }
    };
    if !routine.enabled && !force {
        eprintln!(
            "error: routine '{name}' is disabled — run `plexi routine enable {name}` first, or pass --force to fire it anyway"
        );
        return 1;
    }

    // Spawn via socket (when inside a Plexi pane) with spawn-queue fallback.
    // pane_new_cli implements the socket-first pattern used by all other spawn
    // paths. A named context is passed through so the host fires the routine
    // into it — the same targeting the scheduler's fire path uses (stint 0574);
    // the host errors back when the context does not exist.
    let context = Some(routine.context.as_str()).filter(|c| !c.is_empty());
    log::info!(
        "cli: routine run '{name}' — dispatching command: {} context={context:?}",
        routine.command
    );
    pane_new_cli(
        Some(&routine.command),
        Some(name),
        Some("split_h"),
        None,
        None,
        routine.ephemeral,
        false,
        None,
        &[],
        &[],
        context,
    )
}

/// `plexi routine add` — append a `[[routine]]` table to routines.toml.
pub fn routine_add(
    name: &str,
    command: &str,
    schedule: &str,
    context: Option<&str>,
    ephemeral: bool,
) -> i32 {
    let Some(root) = workspace_root() else {
        print_no_workspace_error();
        return 1;
    };
    let channel_dir = root.join(crate::config::workspace_channel_dir());
    if !channel_dir.is_dir() {
        eprintln!(
            "error: workspace channel directory {} does not exist",
            channel_dir.display()
        );
        eprintln!("Run `plexi workspace init` in {} first.", root.display());
        return 1;
    }
    let path = root.join(routines_file());
    match add_routine_to_file(&path, name, command, schedule, context, ephemeral) {
        Ok(()) => {
            log::info!("cli: routine add '{name}' -> {}", path.display());
            println!("Added routine '{name}' ({schedule}) to {}", path.display());
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            if matches!(e, RoutineFileError::BadSchedule(_)) {
                eprintln!("{SCHEDULE_FORMS_HELP}");
            }
            1
        }
    }
}

/// `plexi routine remove <name>` — delete the entry from routines.toml.
pub fn routine_remove(name: &str) -> i32 {
    let Some(root) = workspace_root() else {
        print_no_workspace_error();
        return 1;
    };
    let path = root.join(routines_file());
    match remove_routine_from_file(&path, name) {
        Ok(()) => {
            log::info!("cli: routine remove '{name}' -> {}", path.display());
            println!("Removed routine '{name}' from {}", path.display());
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `plexi routine enable <name>` / `plexi routine disable <name>`.
pub fn routine_set_enabled(name: &str, enabled: bool) -> i32 {
    let verb = if enabled { "enable" } else { "disable" };
    let Some(root) = workspace_root() else {
        print_no_workspace_error();
        return 1;
    };
    let path = root.join(routines_file());
    match set_routine_enabled_in_file(&path, name, enabled) {
        Ok(()) => {
            log::info!("cli: routine {verb} '{name}' -> {}", path.display());
            println!("Routine '{name}' {verb}d.");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

// ── format-preserving file mutations (toml_edit) ─────────────────────────────
//
// All writes round-trip through `toml_edit` so hand-written comments and key
// ordering survive untouched, and land atomically (temp file in the same
// directory, then rename) so a crash mid-write can never truncate the file.

/// Write `content` to `path` atomically: temp file in the same directory,
/// then rename over the destination. The temp name is pid-unique so two
/// concurrent CLI invocations can never clobber each other's staged content.
fn atomic_write(path: &Path, content: &str) -> Result<(), RoutineFileError> {
    let tmp = path.with_extension(format!("toml.cli-tmp-{}", std::process::id()));
    std::fs::write(&tmp, content)
        .and_then(|_| std::fs::rename(&tmp, path))
        .map_err(|e| RoutineFileError::Io(format!("could not write {}: {e}", path.display())))
}

/// Parse an existing routines.toml into an editable document. `missing_ok`
/// controls whether an absent file yields an empty document (`routine add`)
/// or an error (every other verb).
fn read_doc(path: &Path, missing_ok: bool) -> Result<toml_edit::DocumentMut, RoutineFileError> {
    match std::fs::read_to_string(path) {
        Ok(contents) => contents.parse().map_err(|e| {
            RoutineFileError::Io(format!("failed to parse {}: {e}", path.display()))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && missing_ok => {
            Ok(toml_edit::DocumentMut::new())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(RoutineFileError::Io(format!(
            "no {} found — nothing to modify. Run `plexi routine add` first.",
            path.display()
        ))),
        Err(e) => Err(RoutineFileError::Io(format!(
            "could not read {}: {e}",
            path.display()
        ))),
    }
}

/// The `[[routine]]` array of tables in `doc`, or an error naming the problem.
fn routine_tables_mut<'a>(
    doc: &'a mut toml_edit::DocumentMut,
    path: &Path,
) -> Result<&'a mut toml_edit::ArrayOfTables, RoutineFileError> {
    doc.entry("routine")
        .or_insert(toml_edit::Item::ArrayOfTables(
            toml_edit::ArrayOfTables::new(),
        ))
        .as_array_of_tables_mut()
        .ok_or_else(|| {
            RoutineFileError::Io(format!(
                "'routine' in {} is not an array of tables ([[routine]])",
                path.display()
            ))
        })
}

fn table_name(table: &toml_edit::Table) -> Option<&str> {
    table.get("name").and_then(|v| v.as_str())
}

fn names_of(tables: &toml_edit::ArrayOfTables) -> Vec<String> {
    tables
        .iter()
        .filter_map(|t| table_name(t).map(str::to_string))
        .collect()
}

fn add_routine_to_file(
    path: &Path,
    name: &str,
    command: &str,
    schedule: &str,
    context: Option<&str>,
    ephemeral: bool,
) -> Result<(), RoutineFileError> {
    // Validate through the same parser the scheduler fires on — never write a
    // routine the scheduler would silently drop (stint 0572).
    if parse_schedule(schedule).is_none() {
        return Err(RoutineFileError::BadSchedule(schedule.to_string()));
    }
    let mut doc = read_doc(path, true)?;
    let tables = routine_tables_mut(&mut doc, path)?;
    if tables.iter().any(|t| table_name(t) == Some(name)) {
        return Err(RoutineFileError::DuplicateName(name.to_string()));
    }
    let mut table = toml_edit::Table::new();
    table["name"] = toml_edit::value(name);
    table["command"] = toml_edit::value(command);
    table["schedule"] = toml_edit::value(schedule);
    if let Some(ctx) = context.filter(|c| !c.is_empty()) {
        table["context"] = toml_edit::value(ctx);
    }
    if ephemeral {
        table["ephemeral"] = toml_edit::value(true);
    }
    tables.push(table);
    atomic_write(path, &doc.to_string())
}

fn remove_routine_from_file(path: &Path, name: &str) -> Result<(), RoutineFileError> {
    let mut doc = read_doc(path, false)?;
    let tables = routine_tables_mut(&mut doc, path)?;
    let Some(idx) = tables.iter().position(|t| table_name(t) == Some(name)) else {
        return Err(RoutineFileError::UnknownName {
            name: name.to_string(),
            available: names_of(tables),
        });
    };
    tables.remove(idx);
    atomic_write(path, &doc.to_string())
}

fn set_routine_enabled_in_file(
    path: &Path,
    name: &str,
    enabled: bool,
) -> Result<(), RoutineFileError> {
    let mut doc = read_doc(path, false)?;
    let tables = routine_tables_mut(&mut doc, path)?;
    let Some(table) = tables.iter_mut().find(|t| table_name(t) == Some(name)) else {
        return Err(RoutineFileError::UnknownName {
            name: name.to_string(),
            available: names_of(tables),
        });
    };
    if enabled {
        // Enabled is the default — drop the key so the file returns to its
        // untouched shape rather than accumulating `enabled = true` lines.
        table.remove("enabled");
    } else {
        table["enabled"] = toml_edit::value(false);
    }
    atomic_write(path, &doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routines_path(dir: &Path) -> PathBuf {
        dir.join("routines.toml")
    }

    #[test]
    fn add_writes_a_file_the_scheduler_parses() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = routines_path(tmp.path());
        add_routine_to_file(
            &path,
            "sync",
            "./sync.sh",
            "daily at 09:00",
            Some("work"),
            true,
        )
        .expect("add succeeds");

        // Host and CLI read the same struct — the written file must parse
        // through the scheduler-owned RoutinesConfig (stint 0574).
        let config: RoutinesConfig =
            toml::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parses");
        assert_eq!(config.routine.len(), 1);
        let r = &config.routine[0];
        assert_eq!(r.name, "sync");
        assert_eq!(r.command, "./sync.sh");
        assert_eq!(r.schedule, "daily at 09:00");
        assert_eq!(r.context, "work");
        assert!(r.ephemeral);
        assert!(r.enabled, "enabled defaults to true");
    }

    #[test]
    fn add_rejects_duplicate_name() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = routines_path(tmp.path());
        add_routine_to_file(&path, "sync", "true", "every 2 hours", None, false)
            .expect("first add succeeds");
        let err = add_routine_to_file(&path, "sync", "false", "every 1 hour", None, false)
            .expect_err("duplicate must be rejected");
        assert!(matches!(err, RoutineFileError::DuplicateName(_)), "{err}");
        assert!(err.to_string().contains("sync"), "{err}");
    }

    #[test]
    fn add_rejects_unparseable_schedule() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = routines_path(tmp.path());
        let err = add_routine_to_file(&path, "sync", "true", "every fortnight", None, false)
            .expect_err("bad schedule must be rejected");
        assert!(matches!(err, RoutineFileError::BadSchedule(_)), "{err}");
        assert!(!path.exists(), "no file may be written on rejection");
    }

    #[test]
    fn disable_then_scheduler_load_skips_the_entry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let dir = root.join(crate::config::workspace_channel_dir());
        std::fs::create_dir_all(&dir).expect("create channel dir");
        let path = dir.join("routines.toml");
        add_routine_to_file(&path, "sync", "true", "every 2 hours", None, false)
            .expect("add succeeds");

        let mut s = crate::host::scheduler::Scheduler::new();
        s.load_from_root(root);
        assert_eq!(s.entries.len(), 1, "enabled routine loads");

        set_routine_enabled_in_file(&path, "sync", false).expect("disable succeeds");
        let mut s = crate::host::scheduler::Scheduler::new();
        assert!(
            s.load_from_root(root).is_empty(),
            "a disabled routine is not a load failure"
        );
        assert!(
            s.entries.is_empty(),
            "disabled routine must never become a SchedulerEntry"
        );

        set_routine_enabled_in_file(&path, "sync", true).expect("enable succeeds");
        let mut s = crate::host::scheduler::Scheduler::new();
        s.load_from_root(root);
        assert_eq!(s.entries.len(), 1, "re-enabled routine loads again");
    }

    #[test]
    fn comments_survive_disable_round_trip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = routines_path(tmp.path());
        let original = "# my routines — do not touch\n\n[[routine]]\n# fires every morning\nname = \"sync\" # the important one\ncommand = \"./sync.sh\"\nschedule = \"daily at 09:00\"\n";
        std::fs::write(&path, original).expect("write");

        set_routine_enabled_in_file(&path, "sync", false).expect("disable succeeds");
        let disabled = std::fs::read_to_string(&path).expect("read");
        assert_eq!(
            disabled,
            format!("{original}enabled = false\n"),
            "disable must only add the enabled key"
        );

        set_routine_enabled_in_file(&path, "sync", true).expect("enable succeeds");
        let enabled = std::fs::read_to_string(&path).expect("read");
        assert_eq!(
            enabled, original,
            "enable must restore the file byte-for-byte"
        );
    }

    #[test]
    fn remove_unknown_name_lists_available() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = routines_path(tmp.path());
        add_routine_to_file(&path, "sync", "true", "every 2 hours", None, false)
            .expect("add succeeds");
        let err = remove_routine_from_file(&path, "ghost").expect_err("unknown name errors");
        match &err {
            RoutineFileError::UnknownName { available, .. } => {
                assert_eq!(available, &vec!["sync".to_string()]);
            }
            other => panic!("expected UnknownName, got {other:?}"),
        }
        assert!(err.to_string().contains("sync"), "{err}");
    }

    #[test]
    fn remove_deletes_only_the_named_entry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = routines_path(tmp.path());
        add_routine_to_file(&path, "a", "true", "every 2 hours", None, false).expect("add a");
        add_routine_to_file(&path, "b", "true", "every 1 hour", None, false).expect("add b");
        remove_routine_from_file(&path, "a").expect("remove a");
        let config: RoutinesConfig =
            toml::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parses");
        assert_eq!(config.routine.len(), 1);
        assert_eq!(config.routine[0].name, "b");
    }

    #[test]
    fn workspace_resolution_walks_up_from_subdirectory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join(crate::config::workspace_channel_dir()))
            .expect("create channel dir");
        let nested = root.join("a").join("b");
        std::fs::create_dir_all(&nested).expect("create nested dirs");
        let resolved = crate::app::registry::resolve_workspace_root(&nested)
            .expect("nested subdirectory must resolve to the workspace root");
        assert_eq!(resolved, root);
    }
}
