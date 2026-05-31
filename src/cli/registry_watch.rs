use crate::cli::registry as cli_registry;
use std::collections::BTreeSet;
use std::process::Command;

/// Indirection so the watch path can be tested without spawning real
/// processes.
pub trait CliInspector {
    /// `which <name>` — `Some(path)` when the CLI is installed.
    fn which(&self, name: &str) -> Option<String>;
    /// Captured stdout of `<name> --version`. `None` when the spawn
    /// itself fails.
    fn version(&self, name: &str) -> Option<String>;
    /// Captured stdout of `<name> --help`. Empty string on failure (we
    /// downgrade to "couldn't read help" rather than blowing up).
    fn help(&self, name: &str) -> String;
}

pub struct RealInspector;

impl CliInspector for RealInspector {
    fn which(&self, name: &str) -> Option<String> {
        let out = Command::new("which").arg(name).output().ok()?;
        if !out.status.success() {
            return None;
        }
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if path.is_empty() { None } else { Some(path) }
    }
    fn version(&self, name: &str) -> Option<String> {
        let out = Command::new(name).arg("--version").output().ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
    fn help(&self, name: &str) -> String {
        match Command::new(name).arg("--help").output() {
            Ok(out) => {
                let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
                if s.is_empty() {
                    // Some CLIs (e.g. cargo) print --help to stderr.
                    s = String::from_utf8_lossy(&out.stderr).into_owned();
                }
                s
            }
            Err(_) => String::new(),
        }
    }
}

/// Per-CLI status emitted by the watcher. Variants are exhaustive so
/// callers can pivot rendering by case (text now, JSON later).
#[derive(Debug, PartialEq, Eq)]
pub enum WatchStatus {
    NotInstalled,
    UpToDate { version: String },
    Stale { installed: String, registered: String },
    DescriptorDrift { added: Vec<String>, removed: Vec<String> },
    RegistryError(String),
}

pub struct WatchReport {
    pub cli: String,
    pub status: WatchStatus,
}

/// Compare a CLI's installed `--version`/`--help` against its registry
/// descriptor. Pure given an inspector — drives both the real CLI surface
/// and the unit tests.
pub fn watch_one<I: CliInspector>(inspector: &I, cli: &str) -> WatchReport {
    if inspector.which(cli).is_none() {
        return WatchReport {
            cli: cli.to_string(),
            status: WatchStatus::NotInstalled,
        };
    }
    let descriptor = match cli_registry::lookup(cli, None) {
        Ok(d) => d,
        Err(e) => {
            return WatchReport {
                cli: cli.to_string(),
                status: WatchStatus::RegistryError(e.to_string()),
            };
        }
    };
    let installed_version_raw = inspector.version(cli).unwrap_or_default();
    let installed_version = extract_version(&installed_version_raw);

    if !installed_version.is_empty() && installed_version != descriptor.version {
        return WatchReport {
            cli: cli.to_string(),
            status: WatchStatus::Stale {
                installed: installed_version,
                registered: descriptor.version.clone(),
            },
        };
    }

    // Help-diff heuristic: pull top-level command names from --help, diff
    // against descriptor.commands[].name. Heuristic because every CLI
    // formats --help differently; we don't try to be exhaustive.
    let help = inspector.help(cli);
    let help_commands = parse_top_level_commands(&help);
    let descriptor_commands: BTreeSet<String> =
        descriptor.commands.iter().map(|c| c.name.clone()).collect();
    let added: Vec<String> = help_commands
        .difference(&descriptor_commands)
        .cloned()
        .collect();
    let removed: Vec<String> = descriptor_commands
        .difference(&help_commands)
        .cloned()
        .collect();

    if !added.is_empty() || !removed.is_empty() {
        return WatchReport {
            cli: cli.to_string(),
            status: WatchStatus::DescriptorDrift { added, removed },
        };
    }

    WatchReport {
        cli: cli.to_string(),
        status: WatchStatus::UpToDate {
            version: descriptor.version.clone(),
        },
    }
}

/// CLI entry point. Walks every registered CLI (or just the named one),
/// prints a human-readable summary, returns 0 on success — failures here
/// are *informational* (a stale descriptor is the watcher's whole point),
/// so we don't treat them as exit-1.
pub fn watch_cli(only: Option<&str>) -> i32 {
    let inspector = RealInspector;
    let clis: Vec<String> = match only {
        Some(c) => vec![c.to_string()],
        None => cli_registry::list_clis(),
    };
    if clis.is_empty() {
        println!("registry: no CLIs registered");
        return 0;
    }
    for cli in &clis {
        let report = watch_one(&inspector, cli);
        print_report(&report);
    }
    0
}

fn print_report(report: &WatchReport) {
    match &report.status {
        WatchStatus::NotInstalled => {
            println!("  {}  (not installed — skipping)", report.cli);
        }
        WatchStatus::UpToDate { version } => {
            println!("  {}  up to date (v{version})", report.cli);
        }
        WatchStatus::Stale {
            installed,
            registered,
        } => {
            println!(
                "  [STALE] {}  registry has v{registered}; installed v{installed} \
                 — descriptor may be outdated",
                report.cli
            );
        }
        WatchStatus::DescriptorDrift { added, removed } => {
            println!("  [DRIFT] {}  --help shows commands not in registry:", report.cli);
            if !added.is_empty() {
                println!("    + {}", added.join(", "));
            }
            if !removed.is_empty() {
                println!("    - {} (in registry but not in --help)", removed.join(", "));
            }
        }
        WatchStatus::RegistryError(msg) => {
            println!("  [ERROR] {}  {msg}", report.cli);
        }
    }
}

/// Pull the first whitespace-separated token that contains a `.` from a
/// `<cli> --version` line. Handles "gh version 2.40.0 ..." and
/// "cargo 1.75.0 (...)" without baking in per-CLI parsers.
fn extract_version(raw: &str) -> String {
    for token in raw.split_whitespace() {
        if token.chars().any(|c| c == '.')
            && token.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
        {
            // Strip trailing punctuation/build-metadata.
            let cleaned: String = token
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            return cleaned;
        }
    }
    String::new()
}

/// Heuristic: scan `--help` output for indented two-column "name<spaces>
/// description" rows under a "Commands:" / "COMMANDS" / "CORE COMMANDS"
/// header and treat the first column as a command name. The parser is
/// intentionally loose — every CLI formats --help differently, and the
/// goal is "good enough drift signal", not exhaustive parsing.
fn parse_top_level_commands(help: &str) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    let mut in_commands = false;
    for line in help.lines() {
        let trimmed = line.trim_start();
        let lower = trimmed.trim_end_matches(':').to_ascii_lowercase();
        // Recognize any section header whose name *contains* "command" /
        // "subcommand" as an entry point into command-listing mode. This
        // covers "Commands:", "COMMANDS", "Core Commands", "Management
        // Commands", "All Commands" without per-CLI special cases.
        let is_command_header = !line.starts_with(' ')
            && !line.starts_with('\t')
            && (lower.ends_with("command")
                || lower.ends_with("commands")
                || lower.ends_with("subcommand")
                || lower.ends_with("subcommands"));
        if is_command_header {
            in_commands = true;
            continue;
        }
        // Any other column-0 header that doesn't mention "command" exits
        // the section (e.g. OPTIONS, FLAGS, EXAMPLES, ENVIRONMENT).
        if in_commands && !line.starts_with(' ') && !line.starts_with('\t') && !trimmed.is_empty()
        {
            in_commands = false;
            continue;
        }
        if in_commands {
            if trimmed.is_empty() {
                // Some CLIs (gh) split commands into labelled subsections
                // separated by blanks. Stay in command mode across blanks.
                continue;
            }
            if let Some(first) = trimmed.split_whitespace().next() {
                // Strip the gh-style trailing colon.
                let name = first.trim_end_matches(':');
                if name.is_empty() || name.starts_with('-') {
                    continue;
                }
                if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                    out.insert(name.to_string());
                }
            }
        }
    }
    out
}

#[cfg(test)]
pub struct MockInspector {
    pub installed: bool,
    pub version_str: String,
    pub help_str: String,
}

#[cfg(test)]
impl CliInspector for MockInspector {
    fn which(&self, _: &str) -> Option<String> {
        if self.installed {
            Some("/usr/bin/mock".to_string())
        } else {
            None
        }
    }
    fn version(&self, _: &str) -> Option<String> {
        if self.installed {
            Some(self.version_str.clone())
        } else {
            None
        }
    }
    fn help(&self, _: &str) -> String {
        self.help_str.clone()
    }
}
#[cfg(test)]
mod registry_watch_tests {
    use super::*;

    #[test]
    fn watch_reports_not_installed_for_missing_binary() {
        let inspector = MockInspector {
            installed: false,
            version_str: String::new(),
            help_str: String::new(),
        };
        let report = watch_one(&inspector, "gh");
        assert_eq!(report.status, WatchStatus::NotInstalled);
    }

    #[test]
    fn watch_reports_up_to_date_when_versions_match() {
        // The seeded `gh` registry has version 2.40.0. Match it exactly and
        // give a --help that lists the same top-level commands as the
        // descriptor.
        let inspector = MockInspector {
            installed: true,
            version_str: "gh version 2.40.0 (2023-12-14)".into(),
            help_str: "Usage:  gh <command> <subcommand> [flags]\n\n\
                       CORE COMMANDS\n  \
                       auth:        do auth\n  \
                       pr:          do pr\n  \
                       issue:       do issue\n  \
                       repo:        do repo\n  \
                       release:     do release\n"
                .into(),
        };
        let report = watch_one(&inspector, "gh");
        assert!(
            matches!(report.status, WatchStatus::UpToDate { .. }),
            "expected UpToDate, got {:?}",
            report.status
        );
    }

    #[test]
    fn watch_reports_stale_when_installed_version_exceeds_registry() {
        let inspector = MockInspector {
            installed: true,
            version_str: "gh version 2.99.0 (2026-01-01)".into(),
            help_str: String::new(),
        };
        let report = watch_one(&inspector, "gh");
        match report.status {
            WatchStatus::Stale { installed, registered } => {
                assert_eq!(installed, "2.99.0");
                assert_eq!(registered, "2.40.0");
            }
            other => panic!("expected Stale, got {other:?}"),
        }
    }

    #[test]
    fn watch_reports_registry_error_for_unknown_cli() {
        let inspector = MockInspector {
            installed: true,
            version_str: "fake 0.0.1".into(),
            help_str: String::new(),
        };
        let report = watch_one(&inspector, "nonexistent-cli-zzz");
        assert!(
            matches!(report.status, WatchStatus::RegistryError(_)),
            "expected RegistryError, got {:?}",
            report.status
        );
    }
}
