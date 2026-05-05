//! Tier 3 descriptor fallback: infer a PlexiDescriptor from `<cli> --help`.
//!
//! When a CLI does not support `--plexi` natively (Tier 1) and is not in the
//! registry (Tier 2), this module runs `<cli> --help`, parses the output to
//! extract subcommand names and descriptions, and synthesises a best-effort
//! `PlexiDescriptor`. Results are cached to disk under
//! `~/.plexi-<channel>/cache/descriptors/<cli>.json`.

use crate::plexi_descriptor::{Command, PlexiDescriptor, UiHint};

// ── public types ──────────────────────────────────────────────────────────────

pub struct CrawlResult {
    pub descriptor: PlexiDescriptor,
    pub from_cache: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CrawlError {
    #[error("failed to spawn `{cli} --help`: {source}")]
    SpawnFailed {
        cli: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`{cli} --help` exited with non-zero status")]
    NonZeroExit { cli: String },
    #[error("could not extract any commands from `{cli} --help` output")]
    ParseFailed { cli: String },
}

// ── runner abstraction ────────────────────────────────────────────────────────

pub(crate) trait HelpRunner {
    /// Run `<cli> --help`. Returns `(success, stdout)`.
    fn run_help(&self, cli: &str) -> std::io::Result<(bool, String)>;
    /// Run `<cli> --version`. Returns the first line of stdout, or `None`.
    fn run_version(&self, cli: &str) -> Option<String>;
}

pub struct RealRunner;

impl HelpRunner for RealRunner {
    fn run_help(&self, cli: &str) -> std::io::Result<(bool, String)> {
        let out = std::process::Command::new(cli)
            .arg("--help")
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        // Many CLIs print help to stderr and exit 0; combine both to maximise
        // parseable content but prefer stdout.
        let combined = if stdout.trim().is_empty() {
            String::from_utf8_lossy(&out.stderr).into_owned()
        } else {
            stdout
        };
        Ok((out.status.success(), combined))
    }

    fn run_version(&self, cli: &str) -> Option<String> {
        let out = std::process::Command::new(cli)
            .arg("--version")
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.lines().next().map(|l| l.trim().to_string())
    }
}

// ── public entry points ───────────────────────────────────────────────────────

pub fn crawl(cli_name: &str) -> Result<CrawlResult, CrawlError> {
    let cache_dir = crate::config::config_dir()
        .join("cache")
        .join("descriptors");
    crawl_with_runner(cli_name, &RealRunner, &cache_dir)
}

pub(crate) fn crawl_with_runner(
    cli_name: &str,
    runner: &dyn HelpRunner,
    cache_dir: &std::path::Path,
) -> Result<CrawlResult, CrawlError> {
    let cache_file = cache_dir.join(format!("{cli_name}.json"));

    // Cache hit — try to deserialize; on failure, fall through and re-crawl.
    if cache_file.exists() {
        if let Ok(bytes) = std::fs::read_to_string(&cache_file) {
            if let Ok(descriptor) = serde_json::from_str::<PlexiDescriptor>(&bytes) {
                return Ok(CrawlResult {
                    descriptor,
                    from_cache: true,
                });
            }
        }
    }

    // Cache miss — run --help.
    log::info!("cli_crawl: crawling `{cli_name} --help`");

    let (success, help_text) = runner
        .run_help(cli_name)
        .map_err(|source| CrawlError::SpawnFailed {
            cli: cli_name.to_string(),
            source,
        })?;

    if !success && help_text.trim().is_empty() {
        return Err(CrawlError::NonZeroExit {
            cli: cli_name.to_string(),
        });
    }

    let version_line = runner.run_version(cli_name);
    let detected_version = version_line.as_deref().map(|s| extract_version(cli_name, s));

    let descriptor = parse_help(cli_name, &help_text, detected_version);

    if descriptor.commands.is_empty() {
        return Err(CrawlError::ParseFailed {
            cli: cli_name.to_string(),
        });
    }

    let count = descriptor.commands.len();

    // Write cache.
    if let Ok(json) = serde_json::to_string_pretty(&descriptor) {
        if let Some(parent) = cache_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&cache_file, json);
    }

    log::info!("cli_crawl: inferred {count} commands from `{cli_name} --help`");

    Ok(CrawlResult {
        descriptor,
        from_cache: false,
    })
}

// ── parsing ───────────────────────────────────────────────────────────────────

/// Synthesise a `PlexiDescriptor` by parsing `--help` output.
fn parse_help(cli_name: &str, text: &str, version: Option<String>) -> PlexiDescriptor {
    let commands = extract_commands(text);
    let description = extract_description(text);

    PlexiDescriptor {
        plexi_version: "0.1".to_string(),
        name: cli_name.to_string(),
        version: version.unwrap_or_else(|| "unknown".to_string()),
        description,
        icon: None,
        default_view: if commands.is_empty() {
            None
        } else {
            Some(UiHint::List)
        },
        commands,
        live_state: None,
    }
}

/// Keywords that identify a line as a "commands" section header (case-insensitive).
const COMMAND_SECTION_KEYWORDS: &[&str] = &[
    "AVAILABLE COMMANDS",
    "MANAGEMENT COMMANDS",
    "ADDITIONAL COMMANDS",
    "CORE COMMANDS",
    "SUBCOMMANDS",
    "COMMANDS",
];

fn is_command_section_header(line: &str) -> bool {
    let upper = line.trim().to_uppercase();
    // Strip a trailing colon before matching.
    let upper = upper.trim_end_matches(':').trim();
    COMMAND_SECTION_KEYWORDS.iter().any(|kw| upper == *kw)
}

fn is_any_section_header(line: &str) -> bool {
    // A section header is a non-indented (or lightly indented) ALL-CAPS or
    // Title-ish line ending with optional colon. We use a simple heuristic:
    // the line must not start with whitespace (or start with ≤2 spaces),
    // consist of mostly uppercase letters / spaces, and optionally end with `:`.
    let trimmed = line.trim_end_matches(':').trim();
    if trimmed.is_empty() {
        return false;
    }
    // Must not be indented more than 2 spaces.
    let leading = line.len() - line.trim_start().len();
    if leading > 2 {
        return false;
    }
    // At least 3 chars, mostly alphabetic.
    if trimmed.len() < 3 {
        return false;
    }
    let alpha: usize = trimmed.chars().filter(|c| c.is_alphabetic()).count();
    let upper: usize = trimmed.chars().filter(|c| c.is_uppercase()).count();
    // Heuristic: if ≥60% of alpha chars are uppercase, treat as a header.
    alpha >= 2 && upper * 10 >= alpha * 6
}

/// Extract description: the first non-empty, non-Usage line(s) before the
/// first section header.
fn extract_description(text: &str) -> Option<String> {
    let mut lines = text.lines();
    let mut desc_lines: Vec<&str> = Vec::new();

    for line in lines.by_ref() {
        // Stop at any section header.
        if is_any_section_header(line) {
            break;
        }
        let trimmed = line.trim();
        // Skip blank lines, Usage lines, and the binary name repeated alone.
        if trimmed.is_empty() {
            if !desc_lines.is_empty() {
                break; // A blank line after content ends the description.
            }
            continue;
        }
        let upper = trimmed.to_uppercase();
        if upper.starts_with("USAGE") || upper.starts_with("USE:") {
            continue;
        }
        desc_lines.push(trimmed);
    }

    if desc_lines.is_empty() {
        None
    } else {
        Some(desc_lines.join(" "))
    }
}

/// Extract subcommands from the `--help` output.
///
/// Primary strategy: scan for recognized section headers (COMMANDS, SUBCOMMANDS,
/// CORE COMMANDS, etc.) and collect indented lines within those sections.
///
/// Fallback strategy (e.g. `git --help`): when no recognized header is found,
/// do a broad scan — any line with 3+ leading spaces matching the `word   desc`
/// pattern is a candidate command. This handles prose-header CLIs where
/// sections are labelled "start a working area" rather than "SUBCOMMANDS".
fn extract_commands(text: &str) -> Vec<Command> {
    let mut commands: Vec<Command> = Vec::new();
    let mut in_command_section = false;
    let mut found_section_header = false;

    for line in text.lines() {
        if is_command_section_header(line) {
            in_command_section = true;
            found_section_header = true;
            continue;
        }

        if in_command_section {
            // A different section header exits the command section.
            if is_any_section_header(line) && !is_command_section_header(line) {
                in_command_section = false;
                continue;
            }

            if let Some(cmd) = parse_command_line(line) {
                commands.push(cmd);
            }
        }
    }

    // Fallback: broad scan for consistently-indented command lines when no
    // recognized section header was present (e.g. `git --help`).
    if !found_section_header && commands.is_empty() {
        for line in text.lines() {
            let leading = line.len() - line.trim_start().len();
            // Require exactly 3 leading spaces to avoid picking up usage
            // continuation lines (which are typically 11+ spaces in git).
            if leading == 3 {
                if let Some(cmd) = parse_command_line(line) {
                    // Extra guard: name must be all-lowercase alpha (git-style).
                    if cmd.name.chars().all(|c| c.is_ascii_lowercase()) {
                        commands.push(cmd);
                    }
                }
            }
        }
    }

    commands
}

/// Try to extract a (name, description) pair from a single `--help` line.
///
/// Patterns:
/// 1. `  name    description` — 2+ leading spaces, word, 2+ spaces, rest (`gh`)
/// 2. `  name:   description` — same but name ends with colon
/// 3. `    name   description` — 4+ spaces (`cargo`)
fn parse_command_line(line: &str) -> Option<Command> {
    // Must have at least 2 leading spaces.
    let leading = line.len() - line.trim_start().len();
    if leading < 2 {
        return None;
    }

    let trimmed = line.trim_start();

    // Skip blank lines.
    if trimmed.is_empty() {
        return None;
    }

    // Skip flags.
    if trimmed.starts_with('-') {
        return None;
    }

    // Extract the first token (the command name).
    let (raw_name, rest) = if let Some(pos) = trimmed.find(|c: char| c.is_whitespace()) {
        (&trimmed[..pos], trimmed[pos..].trim())
    } else {
        // Single token, no description.
        (trimmed, "")
    };

    // Strip trailing colon from name (pattern 2).
    let name = raw_name.trim_end_matches(':');

    // Sanity-check: name must be non-empty, no spaces, no leading punctuation.
    if name.is_empty() || name.contains(' ') {
        return None;
    }
    // Reject if name looks like a flag or punctuation-only.
    if name.starts_with('-') || name.starts_with('[') || name.starts_with('<') {
        return None;
    }

    // The description is everything after 2+ whitespace chars following the name.
    // `rest` is already trimmed from the whitespace split above.
    let description = if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    };

    Some(Command {
        name: name.to_string(),
        description,
        icon: None,
        ui_hint: Some(UiHint::List),
        args: vec![],
        flags: vec![],
        writes: vec![],
        reads: vec![],
        streaming: None,
        output_format: None,
        commands: vec![],
    })
}

/// Strip common prefixes from `--version` output to get a bare version string.
///
/// e.g. `gh version 2.40.0 (2024-01-01)` → `2.40.0 (2024-01-01)`
///      `cargo 1.76.0` → `1.76.0`
///      `v1.2.3` → `1.2.3`
fn extract_version(cli_name: &str, line: &str) -> String {
    let s = line.trim();
    // Strip `<cli> version ` prefix.
    let lower = s.to_lowercase();
    let cli_lower = cli_name.to_lowercase();
    if let Some(rest) = lower
        .strip_prefix(&cli_lower)
        .and_then(|r| r.trim_start().strip_prefix("version"))
    {
        // Use the original string at the same offset.
        let offset = s.len() - rest.len();
        return s[offset..].trim().to_string();
    }
    // Strip `<cli> ` prefix.
    if let Some(rest) = lower.strip_prefix(&cli_lower) {
        let offset = s.len() - rest.len();
        return s[offset..].trim().trim_start_matches('v').to_string();
    }
    // Strip leading `v`.
    s.trim_start_matches('v').to_string()
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plexi_descriptor::PlexiDescriptor;

    struct MockRunner {
        help: String,
        version: Option<String>,
    }

    impl HelpRunner for MockRunner {
        fn run_help(&self, _cli: &str) -> std::io::Result<(bool, String)> {
            Ok((true, self.help.clone()))
        }
        fn run_version(&self, _cli: &str) -> Option<String> {
            self.version.clone()
        }
    }

    #[test]
    fn parse_gh_style_help() {
        let help = r#"Work seamlessly with GitHub from the command line.

USAGE
  gh <command> <subcommand> [flags]

CORE COMMANDS
  auth:        Authenticate gh and git with GitHub
  browse:      Open the repository in the browser
  codespace:   Connect to and manage codespaces
  gist:        Manage gists
  issue:       Manage issues
  org:         Manage organizations
  pr:          Manage pull requests

FLAGS
  --help      Show help for command
  --version   Show gh version
"#;
        let d = parse_help("gh", help, None);
        assert_eq!(d.name, "gh");
        assert!(
            d.commands.len() >= 5,
            "expected ≥5 commands, got {}",
            d.commands.len()
        );
        assert!(d.commands.iter().any(|c| c.name == "auth"));
        assert!(d.commands.iter().any(|c| c.name == "pr"));
        // Flags should not be mixed into commands.
        assert!(!d.commands.iter().any(|c| c.name.starts_with("--")));
    }

    #[test]
    fn parse_cargo_style_help() {
        let help = r#"Rust's package manager

USAGE:
    cargo [+toolchain] [OPTIONS] [SUBCOMMAND]

OPTIONS:
    -V, --version   Print version info and exit
    -h, --help      Print help information

SUBCOMMANDS:
    build        Compile the current package
    check        Analyze the current package and report errors
    clean        Remove the target directory
    doc          Build package documentation
    new          Create a new cargo package
    run          Run the current package
    test         Run the tests
"#;
        let d = parse_help("cargo", help, None);
        assert!(d.commands.len() >= 5);
        assert!(d.commands.iter().any(|c| c.name == "build"));
        assert!(d.commands.iter().any(|c| c.name == "test"));
        assert!(!d.commands.iter().any(|c| c.name.starts_with('-')));
    }

    #[test]
    fn cache_round_trip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cache_file = tmp.path().join("test.json");
        let descriptor = PlexiDescriptor {
            plexi_version: "0.1".to_string(),
            name: "test-cli".to_string(),
            version: "1.0.0".to_string(),
            description: Some("A test CLI".to_string()),
            icon: None,
            default_view: Some(UiHint::List),
            commands: vec![Command {
                name: "run".to_string(),
                description: Some("Run the thing".to_string()),
                icon: None,
                ui_hint: Some(UiHint::List),
                args: vec![],
                flags: vec![],
                writes: vec![],
                reads: vec![],
                streaming: None,
                output_format: None,
                commands: vec![],
            }],
            live_state: None,
        };
        let json = serde_json::to_string_pretty(&descriptor).unwrap();
        std::fs::write(&cache_file, &json).unwrap();
        let loaded: PlexiDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.name, "test-cli");
        assert_eq!(loaded.commands.len(), 1);
        assert_eq!(loaded.commands[0].name, "run");
    }

    #[test]
    fn parse_git_style_help() {
        let help = r#"usage: git [-v | --version] [-h | --help] [-C <path>]
           <command> [<args>]

These are common Git commands used in various situations:

start a working area (see also: git help tutorial)
   clone      Clone a repository into a new directory
   init       Create an empty Git repository or reinitialize an existing one

work on the current change (see also: git help everyday)
   add        Add file contents to the index
   mv         Move or rename a file, a directory, or a symlink
   restore    Restore working tree files
   rm         Remove files from the working tree and from the index

examine the history and state (see also: git help revisions)
   bisect     Use binary search to find the commit that introduced a bug
   diff       Show changes between commits, commit and working tree, etc
   log        Show commit logs
   show       Show various types of objects
   status     Show the working tree status

collaborate (see also: git help workflows)
   fetch      Download objects and refs from another repository
   pull       Fetch from and integrate with another repository or a local branch
   push       Update remote refs along with associated objects
"#;
        let d = parse_help("git", help, None);
        assert!(
            d.commands.len() >= 8,
            "expected ≥8 commands, got {}",
            d.commands.len()
        );
        assert!(d.commands.iter().any(|c| c.name == "clone"));
        assert!(d.commands.iter().any(|c| c.name == "push"));
        assert!(d.commands.iter().any(|c| c.name == "log"));
        assert!(!d.commands.iter().any(|c| c.name.starts_with('-')));
    }

    #[test]
    fn crawl_with_mock_runner_returns_inferred_descriptor() {
        let tmp = tempfile::TempDir::new().unwrap();
        let runner = MockRunner {
            help: r#"A mock CLI

COMMANDS
  run    Run something
  stop   Stop something
"#
            .to_string(),
            version: Some("mock-cli 1.2.3".to_string()),
        };
        let result =
            crawl_with_runner("mock-cli", &runner, &tmp.path().join("cache")).unwrap();
        assert!(!result.from_cache);
        assert_eq!(result.descriptor.name, "mock-cli");
        assert!(result.descriptor.commands.len() >= 2);
    }

    #[test]
    fn crawl_serves_from_cache_on_second_call() {
        let tmp = tempfile::TempDir::new().unwrap();
        let runner = MockRunner {
            help: "COMMANDS\n  go   Do the thing\n".to_string(),
            version: None,
        };
        let cache_dir = tmp.path().join("cache");
        let _ = crawl_with_runner("cached-cli", &runner, &cache_dir).unwrap();
        let result2 = crawl_with_runner("cached-cli", &runner, &cache_dir).unwrap();
        assert!(result2.from_cache);
    }
}
