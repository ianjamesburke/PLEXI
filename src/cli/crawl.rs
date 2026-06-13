//! Tier 3 descriptor fallback: infer a PlexiDescriptor from `<cli> --help`.
//!
//! When a CLI does not support `--plexi` natively (Tier 1) and is not in the
//! registry (Tier 2), this module runs `<cli> --help`, parses the output to
//! extract subcommands, then recursively crawls each subcommand's own `--help`
//! to recover its flags, positional args, and nested subcommands. The result
//! is a best-effort `PlexiDescriptor` rich enough to drive the renderer's
//! per-command form. Results are cached to disk under
//! `~/.plexi-<channel>/cache/descriptors/<cli>.json`, keyed by CLI version.
//!
//! Safety: only `--help` is ever invoked — never the command itself. Every
//! subprocess is timeout-bounded (`subprocess::run_capture`), the recursion is
//! depth-capped, and the whole crawl is bounded by a wall-clock budget and a
//! hard ceiling on the number of `--help` probes.

use crate::app::plexi_descriptor::{ArgSpec, ArgType, Command, PlexiDescriptor, UiHint};
use std::collections::HashSet;
use std::time::{Duration, Instant};

// ── crawl limits ───────────────────────────────────────────────────────────────

/// Per-subprocess timeout for a single `--help` invocation.
const HELP_TIMEOUT: Duration = Duration::from_secs(5);
/// Wall-clock ceiling for the entire recursive crawl. Once exceeded, the crawl
/// stops descending and returns whatever it has gathered so far.
const TOTAL_CRAWL_BUDGET: Duration = Duration::from_secs(20);
/// How deep to recurse. Depth 1 = enrich top-level commands (run
/// `<cli> <cmd> --help`). Depth 2 = also enrich their direct subcommands.
/// Grandchildren keep their names but are not probed further.
const MAX_DEPTH: usize = 2;
/// Hard ceiling on subcommand `--help` probes, independent of depth — guards
/// against combinatorial explosion on CLIs with hundreds of subcommands.
const MAX_PROBES: usize = 60;
/// Max commands collected per level (defensive against pathological help text).
const MAX_COMMANDS_PER_LEVEL: usize = 50;

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
    /// Run `<cli> <args…>` (always a help invocation). Returns `(success, text)`
    /// where `text` is stdout, falling back to stderr when stdout is empty.
    fn run_help_args(&self, cli: &str, args: &[&str]) -> std::io::Result<(bool, String)>;
    /// Run `<cli> --version`. Returns the first line of stdout, or `None`.
    fn run_version(&self, cli: &str) -> Option<String>;
}

pub struct RealRunner;

impl HelpRunner for RealRunner {
    fn run_help_args(&self, cli: &str, args: &[&str]) -> std::io::Result<(bool, String)> {
        let captured = crate::cli::subprocess::run_capture(cli, args, HELP_TIMEOUT)?;
        Ok((captured.success, captured.help_text()))
    }

    fn run_version(&self, cli: &str) -> Option<String> {
        let captured =
            crate::cli::subprocess::run_capture(cli, &["--version"], HELP_TIMEOUT).ok()?;
        captured.stdout.lines().next().map(|l| l.trim().to_string())
    }
}

/// Mutable budget threaded through the recursive crawl.
struct CrawlBudget {
    deadline: Instant,
    probes_left: usize,
}

impl CrawlBudget {
    fn new() -> Self {
        Self {
            deadline: Instant::now() + TOTAL_CRAWL_BUDGET,
            probes_left: MAX_PROBES,
        }
    }

    /// Consume one probe if budget remains. Returns false when exhausted.
    fn take_probe(&mut self) -> bool {
        if self.probes_left == 0 {
            log::warn!("cli_crawl: probe ceiling ({MAX_PROBES}) reached — stopping recursion");
            return false;
        }
        if Instant::now() >= self.deadline {
            log::warn!(
                "cli_crawl: time budget ({}s) exceeded — stopping recursion",
                TOTAL_CRAWL_BUDGET.as_secs()
            );
            return false;
        }
        self.probes_left -= 1;
        true
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
    let safe_name: String = cli_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cache_file = cache_dir.join(format!("{safe_name}.json"));

    // Cache hit — try to deserialize; on failure, fall through and re-crawl.
    // Also check version: if the CLI version changed since we cached, invalidate.
    if cache_file.exists() {
        if let Ok(bytes) = std::fs::read_to_string(&cache_file) {
            if let Ok(descriptor) = serde_json::from_str::<PlexiDescriptor>(&bytes) {
                let current_version = runner
                    .run_version(cli_name)
                    .as_deref()
                    .map(|s| extract_version(cli_name, s));
                let stale = current_version
                    .as_deref()
                    .map(|v| v != descriptor.version)
                    .unwrap_or(false);
                if stale {
                    log::info!(
                        "cli_crawl: cache stale for `{cli_name}` — version changed from {} to {}; re-crawling",
                        descriptor.version,
                        current_version.as_deref().unwrap_or("unknown"),
                    );
                } else {
                    return Ok(CrawlResult {
                        descriptor,
                        from_cache: true,
                    });
                }
            }
        }
    }

    // Cache miss — run the top-level `--help`.
    log::info!("cli_crawl: crawling `{cli_name} --help`");

    let (success, help_text) = runner
        .run_help_args(cli_name, &["--help"])
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
    let detected_version = version_line
        .as_deref()
        .map(|s| extract_version(cli_name, s));

    let mut descriptor = parse_help(cli_name, &help_text, detected_version);

    if descriptor.commands.is_empty() {
        return Err(CrawlError::ParseFailed {
            cli: cli_name.to_string(),
        });
    }

    // Recursively enrich each command with its flags, positional args, and
    // nested subcommands by crawling `<cli> <cmd…> --help`.
    let mut budget = CrawlBudget::new();
    enrich_commands(cli_name, runner, &mut descriptor.commands, &[], 1, &mut budget);

    let count = descriptor.commands.len();
    let flag_total: usize = count_flags(&descriptor.commands);

    // Write cache. A failure here is non-fatal (we still return the freshly
    // crawled descriptor) but must be logged — a silently failing cache means
    // every open re-runs the expensive crawl with no visible reason.
    if let Ok(json) = serde_json::to_string_pretty(&descriptor) {
        if let Some(parent) = cache_file.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::warn!("cli_crawl: could not create cache dir {parent:?}: {e}");
            }
        }
        if let Err(e) = std::fs::write(&cache_file, json) {
            log::warn!("cli_crawl: could not write cache {cache_file:?}: {e}");
        }
    }

    log::info!(
        "cli_crawl: inferred {count} top-level commands ({flag_total} flags/args across tree) from `{cli_name}`"
    );

    Ok(CrawlResult {
        descriptor,
        from_cache: false,
    })
}

fn count_flags(cmds: &[Command]) -> usize {
    cmds.iter()
        .map(|c| c.flags.len() + c.args.len() + count_flags(&c.commands))
        .sum()
}

/// Crawl each command's own `--help` to recover flags, positional args, and
/// nested subcommands. Recurses up to `MAX_DEPTH`, bounded by `budget`.
fn enrich_commands(
    cli: &str,
    runner: &dyn HelpRunner,
    cmds: &mut [Command],
    parent_path: &[String],
    depth: usize,
    budget: &mut CrawlBudget,
) {
    for cmd in cmds.iter_mut() {
        if !budget.take_probe() {
            break;
        }
        let mut path: Vec<String> = parent_path.to_vec();
        path.push(cmd.name.clone());
        let arg_refs: Vec<&str> = path
            .iter()
            .map(String::as_str)
            .chain(std::iter::once("--help"))
            .collect();

        let text = match runner.run_help_args(cli, &arg_refs) {
            Ok((_, text)) => text,
            Err(e) => {
                log::warn!("cli_crawl: `{cli} {} --help` failed: {e}", path.join(" "));
                continue;
            }
        };

        cmd.flags = parse_flags(&text);
        cmd.args = parse_positional_args(&text);
        let subs = extract_commands(&text);
        cmd.commands = subs;

        // A command with children renders as a list; a leaf with fields as a form.
        cmd.ui_hint = Some(if cmd.commands.is_empty() {
            UiHint::Form
        } else {
            UiHint::List
        });

        if !cmd.commands.is_empty() && depth < MAX_DEPTH {
            enrich_commands(cli, runner, &mut cmd.commands, &path, depth + 1, budget);
        }
    }
}

// ── parsing ───────────────────────────────────────────────────────────────────

/// Synthesise a `PlexiDescriptor` by parsing top-level `--help` output. Only
/// command names + descriptions are extracted here; flags/args/subcommands are
/// filled in by the recursive `enrich_commands` pass.
pub(crate) fn parse_help(cli_name: &str, text: &str, version: Option<String>) -> PlexiDescriptor {
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
        plexi_app: None,
        capabilities: vec![],
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

/// Keywords that identify a line as a "flags/options" section header.
const FLAG_SECTION_KEYWORDS: &[&str] = &["OPTIONS", "FLAGS", "GLOBAL OPTIONS", "GLOBAL FLAGS"];

/// Keywords that identify a line as a "positional arguments" section header.
const ARG_SECTION_KEYWORDS: &[&str] = &["ARGUMENTS", "ARGS", "POSITIONAL ARGUMENTS"];

fn header_matches(line: &str, keywords: &[&str]) -> bool {
    let upper = line.trim().to_uppercase();
    let upper = upper.trim_end_matches(':').trim();
    keywords.contains(&upper)
}

fn is_command_section_header(line: &str) -> bool {
    header_matches(line, COMMAND_SECTION_KEYWORDS)
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

/// Collect the raw lines that fall inside the first section whose header
/// matches `keywords`, stopping at the next section header.
fn lines_in_section<'a>(text: &'a str, keywords: &[&str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if header_matches(line, keywords) {
            inside = true;
            continue;
        }
        if inside {
            // Another (non-matching) section header ends this section.
            if is_any_section_header(line) && !header_matches(line, keywords) {
                break;
            }
            out.push(line);
        }
    }
    out
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
    let mut seen: HashSet<String> = HashSet::new();
    let mut in_command_section = false;
    let mut found_section_header = false;

    for line in text.lines() {
        if commands.len() >= MAX_COMMANDS_PER_LEVEL {
            log::warn!("cli_crawl: command cap ({MAX_COMMANDS_PER_LEVEL}) reached — truncating");
            break;
        }
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
                if seen.insert(cmd.name.clone()) {
                    commands.push(cmd);
                }
            }
        }
    }

    // Fallback: broad scan for consistently-indented command lines when no
    // recognized section header was present (e.g. `git --help`).
    if !found_section_header && commands.is_empty() {
        for line in text.lines() {
            if commands.len() >= MAX_COMMANDS_PER_LEVEL {
                break;
            }
            let leading = line.len() - line.trim_start().len();
            // Require exactly 3 leading spaces to avoid picking up usage
            // continuation lines (which are typically 11+ spaces in git).
            if leading == 3 {
                if let Some(cmd) = parse_command_line(line) {
                    // Extra guard: name must be all-lowercase alpha (git-style).
                    if cmd.name.chars().all(|c| c.is_ascii_lowercase())
                        && seen.insert(cmd.name.clone())
                    {
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

/// Parse flag/option specs from a command's `--help` output.
///
/// Scans the OPTIONS/FLAGS section(s); if none are present, falls back to a
/// bounded scan of every indented `-`-leading line. `--help`/`--version` are
/// always skipped — they add nothing to a generated form.
fn parse_flags(text: &str) -> Vec<ArgSpec> {
    let mut flags = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // Gather candidate flag lines from recognised sections first.
    let mut candidate_lines: Vec<&str> = Vec::new();
    for kw in FLAG_SECTION_KEYWORDS {
        candidate_lines.extend(lines_in_section(text, &[kw]));
    }
    // Fallback: no recognised section — scan all flag-shaped lines.
    if candidate_lines.is_empty() {
        candidate_lines = text.lines().collect();
    }

    for line in candidate_lines {
        if let Some(spec) = parse_flag_line(line) {
            if seen.insert(spec.name.clone()) {
                flags.push(spec);
            }
        }
    }
    flags
}

/// Split a trimmed line into its `(spec, description)` halves at the first run
/// of 2+ spaces — the convention every clap/cobra/getopt help formatter uses.
fn split_on_double_space(s: &str) -> (&str, &str) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b' ' && bytes[i + 1] == b' ' {
            return (s[..i].trim_end(), s[i..].trim());
        }
        i += 1;
    }
    (s.trim_end(), "")
}

fn looks_like_metavar(tok: &str) -> bool {
    if tok.starts_with('<') || tok.starts_with('[') {
        return true;
    }
    // An ALL-CAPS bare word like FILE / PATH / VALUE.
    let inner = tok.trim_matches(['<', '>', '[', ']', '.']);
    !inner.is_empty()
        && inner.len() >= 2
        && inner.chars().all(|c| c.is_ascii_uppercase() || c == '_' || c == '-')
}

fn metavar_clean(tok: &str) -> String {
    tok.trim_matches(['<', '>', '[', ']', '.']).to_string()
}

fn looks_like_path(metavar: &str) -> bool {
    let m = metavar.to_lowercase();
    m.contains("file") || m.contains("path") || m.contains("dir")
}

/// Parse a single flag line such as:
///   `-v, --verbose            Enable verbose output`
///   `    --output <FILE>      Write output to FILE`
///   `-n, --number=N           Set the count`
fn parse_flag_line(line: &str) -> Option<ArgSpec> {
    let leading = line.len() - line.trim_start().len();
    // Flags are always indented in help output.
    if leading < 1 {
        return None;
    }
    let trimmed = line.trim_start();
    if !trimmed.starts_with('-') {
        return None;
    }

    let (spec_part, desc) = split_on_double_space(trimmed);

    let mut long: Option<String> = None;
    let mut short: Option<String> = None;
    let mut metavar: Option<String> = None;

    for tok in spec_part
        .split([',', ' ', '\t', '|'])
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
    {
        if let Some(rest) = tok.strip_prefix("--") {
            let (name, inline_meta) = match rest.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (rest, None),
            };
            if is_flag_ident(name) {
                long = Some(format!("--{name}"));
            }
            if let Some(v) = inline_meta {
                if !v.is_empty() {
                    metavar = Some(metavar_clean(v));
                }
            }
        } else if let Some(rest) = tok.strip_prefix('-') {
            // Short flag: take the first char only (`-n`, ignore `-nVAL`).
            if let Some(c) = rest.chars().next() {
                if c.is_ascii_alphanumeric() {
                    short = Some(format!("-{c}"));
                }
            }
        } else if looks_like_metavar(tok) {
            metavar = Some(metavar_clean(tok));
        }
    }

    let name = long.or(short)?;
    let bare = name.trim_start_matches('-');
    // Never surface help/version controls in a generated form.
    if matches!(bare, "help" | "version" | "h" | "V") {
        return None;
    }

    let ty = match &metavar {
        Some(m) if looks_like_path(m) => ArgType::Path,
        Some(_) => ArgType::String,
        None => ArgType::Bool,
    };

    Some(ArgSpec {
        name,
        ty,
        required: Some(false),
        default: None,
        description: if desc.is_empty() {
            None
        } else {
            Some(desc.to_string())
        },
        placeholder: metavar,
        enum_values: None,
        min: None,
        max: None,
    })
}

fn is_flag_ident(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && name.chars().next().map(|c| c.is_ascii_alphanumeric()).unwrap_or(false)
}

/// Parse positional arguments from a command's `--help` ARGUMENTS section.
///
/// Lines look like `<NAME>  description` (required) or `[NAME]  description`
/// (optional), or a bare `NAME  description`.
fn parse_positional_args(text: &str) -> Vec<ArgSpec> {
    let mut args = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let mut lines: Vec<&str> = Vec::new();
    for kw in ARG_SECTION_KEYWORDS {
        lines.extend(lines_in_section(text, &[kw]));
    }

    for line in lines {
        if let Some(spec) = parse_arg_line(line) {
            if seen.insert(spec.name.clone()) {
                args.push(spec);
            }
        }
    }
    args
}

fn parse_arg_line(line: &str) -> Option<ArgSpec> {
    let leading = line.len() - line.trim_start().len();
    if leading < 1 {
        return None;
    }
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('-') {
        return None;
    }
    let (spec_part, desc) = split_on_double_space(trimmed);
    let token = spec_part.split_whitespace().next()?;

    let required = token.starts_with('<');
    let name = token.trim_matches(['<', '>', '[', ']', '.']);
    // Drop ellipsis/variadic markers and empties.
    let name = name.trim_end_matches('.').trim();
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return None;
    }

    Some(ArgSpec {
        name: name.to_string(),
        ty: ArgType::String,
        required: Some(required),
        default: None,
        description: if desc.is_empty() {
            None
        } else {
            Some(desc.to_string())
        },
        placeholder: None,
        enum_values: None,
        min: None,
        max: None,
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
    use crate::app::plexi_descriptor::PlexiDescriptor;
    use std::collections::HashMap;

    /// A runner that returns canned help text keyed by the joined args, e.g.
    /// `"--help"` for the top level or `"remote --help"` for a subcommand.
    struct MapRunner {
        help: HashMap<String, String>,
        version: Option<String>,
    }

    impl MapRunner {
        fn new(version: Option<&str>) -> Self {
            Self {
                help: HashMap::new(),
                version: version.map(str::to_string),
            }
        }
        fn with(mut self, key: &str, text: &str) -> Self {
            self.help.insert(key.to_string(), text.to_string());
            self
        }
    }

    impl HelpRunner for MapRunner {
        fn run_help_args(&self, _cli: &str, args: &[&str]) -> std::io::Result<(bool, String)> {
            let key = args.join(" ");
            match self.help.get(&key) {
                Some(t) => Ok((true, t.clone())),
                None => Ok((true, String::new())),
            }
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
            plexi_app: None,
            capabilities: vec![],
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
    fn crawl_with_runner_returns_inferred_descriptor() {
        let tmp = tempfile::TempDir::new().unwrap();
        let runner = MapRunner::new(Some("mock-cli 1.2.3")).with(
            "--help",
            "A mock CLI\n\nCOMMANDS\n  run    Run something\n  stop   Stop something\n",
        );
        let result = crawl_with_runner("mock-cli", &runner, &tmp.path().join("cache")).unwrap();
        assert!(!result.from_cache);
        assert_eq!(result.descriptor.name, "mock-cli");
        assert!(result.descriptor.commands.len() >= 2);
    }

    #[test]
    fn crawl_serves_from_cache_on_second_call() {
        let tmp = tempfile::TempDir::new().unwrap();
        let runner =
            MapRunner::new(None).with("--help", "COMMANDS\n  go   Do the thing\n");
        let cache_dir = tmp.path().join("cache");
        let _ = crawl_with_runner("cached-cli", &runner, &cache_dir).unwrap();
        let result2 = crawl_with_runner("cached-cli", &runner, &cache_dir).unwrap();
        assert!(result2.from_cache);
    }

    #[test]
    fn cache_invalidated_on_version_change() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cache_dir = tmp.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        // Pre-populate cache with version "0.9.0".
        let stale_descriptor = PlexiDescriptor {
            plexi_version: "0.1".to_string(),
            name: "version-cli".to_string(),
            version: "0.9.0".to_string(),
            description: None,
            icon: None,
            default_view: Some(UiHint::List),
            commands: vec![Command {
                name: "run".to_string(),
                description: None,
                icon: None,
                ui_hint: None,
                args: vec![],
                flags: vec![],
                writes: vec![],
                reads: vec![],
                streaming: None,
                output_format: None,
                commands: vec![],
            }],
            live_state: None,
            plexi_app: None,
            capabilities: vec![],
        };
        let cache_file = cache_dir.join("version-cli.json");
        std::fs::write(
            &cache_file,
            serde_json::to_string(&stale_descriptor).unwrap(),
        )
        .unwrap();

        // Runner reports version "1.0.0" and help with commands so crawl succeeds.
        let runner = MapRunner::new(Some("version-cli 1.0.0"))
            .with("--help", "COMMANDS\n  run   Do the thing\n");

        let result = crawl_with_runner("version-cli", &runner, &cache_dir).unwrap();
        assert!(
            !result.from_cache,
            "stale cache should have been invalidated"
        );
        assert_eq!(result.descriptor.version, "1.0.0");
    }

    #[test]
    fn path_traversal_cli_name_stays_within_cache_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cache_dir = tmp.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let runner =
            MapRunner::new(None).with("--help", "COMMANDS\n  run   Do the thing\n");

        // "../../evil" → each of '.','.','/','.','.','/' is non-alphanumeric → "______evil"
        crawl_with_runner("../../evil", &runner, &cache_dir)
            .expect("crawl should succeed even with traversal input");

        let expected = cache_dir.join("______evil.json");
        assert!(
            expected.exists(),
            "sanitized cache file should exist at {expected:?}"
        );
        assert!(
            expected.starts_with(&cache_dir),
            "cache file escaped cache_dir: {expected:?}"
        );

        // No file created outside the cache dir.
        let parent = tmp.path().parent().unwrap();
        assert!(
            !parent.join("evil.json").exists(),
            "path traversal wrote outside cache_dir"
        );
    }

    // ── recursive enrichment ────────────────────────────────────────────────

    #[test]
    fn recursive_crawl_populates_flags_and_args() {
        let tmp = tempfile::TempDir::new().unwrap();
        let top = "A demo CLI\n\nCOMMANDS\n  greet   Greet someone\n";
        let greet = r#"Greet someone warmly

USAGE:
    demo greet [OPTIONS] <NAME>

ARGUMENTS:
    <NAME>    Who to greet

OPTIONS:
    -l, --loud           Shout the greeting
    -o, --output <FILE>  Write to FILE instead of stdout
    -h, --help           Print help
"#;
        let runner = MapRunner::new(Some("demo 0.1.0"))
            .with("--help", top)
            .with("greet --help", greet);

        let result =
            crawl_with_runner("demo", &runner, &tmp.path().join("cache")).unwrap();
        let greet_cmd = result
            .descriptor
            .commands
            .iter()
            .find(|c| c.name == "greet")
            .expect("greet command present");

        // Positional arg parsed and marked required.
        assert_eq!(greet_cmd.args.len(), 1, "expected 1 positional arg");
        assert_eq!(greet_cmd.args[0].name, "NAME");
        assert_eq!(greet_cmd.args[0].required, Some(true));

        // Flags parsed; --help excluded.
        let flag_names: Vec<&str> = greet_cmd.flags.iter().map(|f| f.name.as_str()).collect();
        assert!(flag_names.contains(&"--loud"), "flags: {flag_names:?}");
        assert!(flag_names.contains(&"--output"), "flags: {flag_names:?}");
        assert!(
            !flag_names.contains(&"--help"),
            "--help must be excluded: {flag_names:?}"
        );

        // --loud is a bool (no metavar); --output takes a path-ish value.
        let loud = greet_cmd.flags.iter().find(|f| f.name == "--loud").unwrap();
        assert_eq!(loud.ty, ArgType::Bool);
        let output = greet_cmd.flags.iter().find(|f| f.name == "--output").unwrap();
        assert_eq!(output.ty, ArgType::Path);

        // Leaf command should be hinted as a form.
        assert_eq!(greet_cmd.ui_hint, Some(UiHint::Form));
    }

    #[test]
    fn recursive_crawl_captures_nested_subcommands() {
        let tmp = tempfile::TempDir::new().unwrap();
        let top = "A VCS\n\nCOMMANDS\n  remote   Manage remotes\n";
        let remote = "Manage remotes\n\nCOMMANDS\n  add      Add a remote\n  remove   Remove a remote\n";
        let remote_add = "Add a remote\n\nARGUMENTS:\n  <NAME>   Remote name\n  <URL>    Remote URL\n";
        let runner = MapRunner::new(Some("vcs 2.0.0"))
            .with("--help", top)
            .with("remote --help", remote)
            .with("remote add --help", remote_add);

        let result = crawl_with_runner("vcs", &runner, &tmp.path().join("cache")).unwrap();
        let remote_cmd = result
            .descriptor
            .commands
            .iter()
            .find(|c| c.name == "remote")
            .expect("remote command present");
        assert_eq!(remote_cmd.ui_hint, Some(UiHint::List), "parent → list");
        let add_cmd = remote_cmd
            .commands
            .iter()
            .find(|c| c.name == "add")
            .expect("nested add command present");
        // Depth 2 (remote add) is enriched: its positional args are captured.
        assert_eq!(add_cmd.args.len(), 2, "remote add should have 2 args");
        assert!(add_cmd.args.iter().any(|a| a.name == "NAME"));
        assert!(add_cmd.args.iter().any(|a| a.name == "URL"));
    }

    #[test]
    fn recursive_crawl_respects_depth_limit() {
        // Build a chain deeper than MAX_DEPTH; ensure probing stops.
        let tmp = tempfile::TempDir::new().unwrap();
        let runner = MapRunner::new(Some("deep 1.0.0"))
            .with("--help", "COMMANDS\n  a   level a\n")
            .with("a --help", "COMMANDS\n  b   level b\n")
            .with("a b --help", "COMMANDS\n  c   level c\n")
            .with("a b c --help", "OPTIONS\n  --deep   too deep\n");

        let result = crawl_with_runner("deep", &runner, &tmp.path().join("cache")).unwrap();
        let a = &result.descriptor.commands[0];
        assert_eq!(a.name, "a");
        let b = &a.commands[0];
        assert_eq!(b.name, "b");
        // `c` should exist by name (extracted from `a b --help`) but NOT be
        // probed further, so its flags stay empty.
        let c = &b.commands[0];
        assert_eq!(c.name, "c");
        assert!(
            c.flags.is_empty() && c.commands.is_empty(),
            "depth limit should stop probing at `a b c`"
        );
    }

    // ── flag/arg line parsing ──────────────────────────────────────────────

    #[test]
    fn flag_line_bool_vs_value() {
        let bool_flag = parse_flag_line("  -v, --verbose       Be loud").unwrap();
        assert_eq!(bool_flag.name, "--verbose");
        assert_eq!(bool_flag.ty, ArgType::Bool);
        assert_eq!(bool_flag.description.as_deref(), Some("Be loud"));

        let val_flag = parse_flag_line("      --count <N>       How many").unwrap();
        assert_eq!(val_flag.name, "--count");
        assert_eq!(val_flag.ty, ArgType::String);
        assert_eq!(val_flag.placeholder.as_deref(), Some("N"));

        let eq_flag = parse_flag_line("  --name=NAME    Set name").unwrap();
        assert_eq!(eq_flag.name, "--name");
        assert_eq!(eq_flag.ty, ArgType::String);
    }

    #[test]
    fn flag_line_skips_help_and_version() {
        assert!(parse_flag_line("  -h, --help     Print help").is_none());
        assert!(parse_flag_line("  -V, --version  Print version").is_none());
    }

    #[test]
    fn flag_line_short_only() {
        let f = parse_flag_line("  -q             Quiet mode").unwrap();
        assert_eq!(f.name, "-q");
        assert_eq!(f.ty, ArgType::Bool);
    }

    #[test]
    fn arg_line_required_vs_optional() {
        let req = parse_arg_line("  <PATH>    The path").unwrap();
        assert_eq!(req.name, "PATH");
        assert_eq!(req.required, Some(true));

        let opt = parse_arg_line("  [TARGET]  Optional target").unwrap();
        assert_eq!(opt.name, "TARGET");
        assert_eq!(opt.required, Some(false));
    }

    #[test]
    fn non_flag_lines_rejected() {
        assert!(parse_flag_line("  not a flag at all").is_none());
        assert!(parse_flag_line("").is_none());
    }
}
