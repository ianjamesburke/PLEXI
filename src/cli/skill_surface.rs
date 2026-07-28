//! Verifies `skills/plexi-cli/SKILL.md` against the clap command tree built from
//! this same tree (stint 0570 fix round). A version string cannot detect drift —
//! alpha carries the last released version number between releases — so this
//! gate checks the documented *surface* itself: every subcommand path and every
//! `--flag` named inside the skill's fenced code blocks must exist on the
//! compiled CLI. `scripts/promote.sh` runs this module in the beta tree before
//! beta→main, so a release cannot ship a skill documenting commands its binary
//! does not have.

use clap::CommandFactory;
use std::path::PathBuf;

fn skill_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills/plexi-cli/SKILL.md")
}

fn skill_text() -> String {
    let path = skill_path();
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn built_cli() -> clap::Command {
    let mut cmd = super::args::Cli::command();
    cmd.build(); // propagate global args into subcommands
    cmd
}

/// True for a token that can only be a subcommand name: lowercase kebab word.
fn is_command_word(tok: &str) -> bool {
    !tok.is_empty()
        && tok.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && tok
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn find_subcommand<'a>(cmd: &'a clap::Command, tok: &str) -> Option<&'a clap::Command> {
    cmd.get_subcommands()
        .find(|s| s.get_name() == tok || s.get_all_aliases().any(|a| a == tok))
}

fn has_long_flag(cmd: &clap::Command, flag: &str) -> bool {
    flag == "help"
        || flag == "version"
        || cmd
            .get_arguments()
            .any(|a| a.get_long() == Some(flag) || a.get_all_aliases().iter().flatten().any(|al| *al == flag))
}

/// Extract every `--long-flag` name mentioned in a token, stripping the
/// brackets, alternation, and punctuation the reference blocks use
/// (`[--json]`, `--parent[=NAME]`, `--layout tiled|columns`, `-d/--down,`).
fn long_flags_in(tok: &str) -> Vec<String> {
    let mut flags = Vec::new();
    let mut rest = tok;
    while let Some(pos) = rest.find("--") {
        let after = &rest[pos + 2..];
        let name: String = after
            .chars()
            .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
            .collect();
        let name = name.trim_end_matches('-').to_string();
        rest = &after[name.len()..];
        if !name.is_empty() {
            flags.push(name);
        }
    }
    flags
}

struct Block {
    lang: String,
    lines: Vec<String>,
}

fn fenced_blocks(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut current: Option<Block> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("```") {
            match current.take() {
                Some(block) => blocks.push(block),
                None => {
                    current = Some(Block {
                        lang: rest.trim().to_string(),
                        lines: Vec::new(),
                    })
                }
            }
        } else if let Some(block) = current.as_mut() {
            block.lines.push(line.to_string());
        }
    }
    blocks
}

/// Walk as many leading tokens as resolve to subcommands. Returns the deepest
/// resolved command, the resolved path, and the index of the first unconsumed
/// token.
fn descend<'a>(
    root: &'a clap::Command,
    tokens: &[&str],
) -> (&'a clap::Command, Vec<String>, usize) {
    let mut cmd = root;
    let mut path = Vec::new();
    let mut idx = 0;
    while idx < tokens.len() && is_command_word(tokens[idx]) {
        match find_subcommand(cmd, tokens[idx]) {
            Some(sub) => {
                cmd = sub;
                path.push(tokens[idx].to_string());
                idx += 1;
            }
            None => break,
        }
    }
    (cmd, path, idx)
}

/// Fence languages the extractor may legitimately skip: they can never carry
/// CLI invocations. Anything else that isn't bare/bash/sh is an ERROR — a
/// silent skip would let a markdown relabel hollow out the gate while it
/// still reports ok (fix round 2). `text` is deliberately NOT inert: it is
/// the natural relabel target for the reference blocks.
const INERT_FENCE_LANGS: &[&str] = &["json", "toml", "rust"];

/// Coverage floors: canaries against silent extraction collapse. The gate
/// currently resolves ~85 command paths and checks ~82 flags; a legitimate
/// skill edit moves these gradually, a broken extractor drops them off a
/// cliff. Raise/lower deliberately when the skill genuinely grows or shrinks.
const MIN_COMMANDS_RESOLVED: usize = 60;
const MIN_FLAGS_CHECKED: usize = 50;

#[derive(Default)]
struct Report {
    errors: Vec<String>,
    commands_resolved: usize,
    flags_checked: usize,
}

fn check_flags(cmd: &clap::Command, path: &[String], tokens: &[&str], report: &mut Report) {
    for tok in tokens {
        for flag in long_flags_in(tok) {
            report.flags_checked += 1;
            if !has_long_flag(cmd, &flag) {
                report.errors.push(format!(
                    "`plexi {}` has no `--{flag}` flag, but the skill documents one",
                    path.join(" "),
                ));
            }
        }
    }
}

/// Reference blocks (bare ``` fences) are the authoritative surface listing:
/// each unindented line starts with a subcommand path, and every leading
/// lowercase token MUST resolve — a token that doesn't is a documented command
/// the binary does not have. Indented lines continue the previous entry.
fn check_reference_block(root: &clap::Command, block: &Block, report: &mut Report) {
    let mut entry: Option<Vec<String>> = None; // resolved path of the current entry
    let mut entry_cmd: Option<&clap::Command> = None;
    for line in &block.lines {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            // continuation of the current entry: flags only
            if let (Some(path), Some(cmd)) = (&entry, entry_cmd) {
                check_flags(cmd, path, &tokens, report);
            }
            continue;
        }
        let mut cmd = root;
        let mut path: Vec<String> = Vec::new();
        let mut idx = 0;
        let mut failed = false;
        while idx < tokens.len() && is_command_word(tokens[idx]) {
            match find_subcommand(cmd, tokens[idx]) {
                Some(sub) => {
                    cmd = sub;
                    path.push(tokens[idx].to_string());
                    idx += 1;
                }
                None => {
                    report.errors.push(format!(
                        "skill documents subcommand `plexi {} {}`, which does not exist in this CLI",
                        path.join(" "),
                        tokens[idx],
                    ));
                    failed = true;
                    break;
                }
            }
        }
        if failed || path.is_empty() {
            if path.is_empty() && !failed {
                report.errors.push(format!(
                    "reference line does not start with a known subcommand: `{}`",
                    line.trim()
                ));
            }
            entry = None;
            entry_cmd = None;
            continue;
        }
        report.commands_resolved += 1;
        check_flags(cmd, &path, &tokens[idx..], report);
        entry = Some(path);
        entry_cmd = Some(cmd);
    }
}

/// Bash example blocks: every `plexi …` invocation must start with a real
/// subcommand; deeper tokens descend while they resolve (positionals may be
/// lowercase, so a non-resolving token ends the walk without error), and every
/// `--flag` on the invocation must exist on the deepest resolved command.
fn check_bash_block(root: &clap::Command, block: &Block, report: &mut Report) {
    for line in &block.lines {
        let normalized: String = line
            .replace("$(", " ")
            .chars()
            .map(|c| if "()|;&`".contains(c) { ' ' } else { c })
            .collect();
        let tokens: Vec<&str> = normalized.split_whitespace().collect();
        let mut i = 0;
        while i < tokens.len() {
            if tokens[i] != "plexi" {
                i += 1;
                continue;
            }
            let segment_end = tokens[i + 1..]
                .iter()
                .position(|t| *t == "plexi")
                .map(|p| i + 1 + p)
                .unwrap_or(tokens.len());
            let segment = &tokens[i + 1..segment_end];
            if segment.is_empty() {
                break;
            }
            let (cmd, path, idx) = descend(root, segment);
            if path.is_empty() {
                report.errors.push(format!(
                    "bash example invokes `plexi {}`, which is not a known subcommand (line: `{}`)",
                    segment[0],
                    line.trim(),
                ));
            } else {
                report.commands_resolved += 1;
                check_flags(cmd, &path, &segment[idx..], report);
            }
            i = segment_end;
        }
    }
}

#[test]
fn skill_surface_matches_cli() {
    let text = skill_text();
    let root = built_cli();
    let mut report = Report::default();
    for block in fenced_blocks(&text) {
        match block.lang.as_str() {
            "" => check_reference_block(&root, &block, &mut report),
            "bash" | "sh" => check_bash_block(&root, &block, &mut report),
            lang if INERT_FENCE_LANGS.contains(&lang) => {}
            lang => report.errors.push(format!(
                "fence language `{lang}` is not checkable by this gate — CLI-bearing blocks \
                 must be bare (reference) or ```bash; genuinely inert languages go in \
                 INERT_FENCE_LANGS in src/cli/skill_surface.rs, never a silent skip",
            )),
        }
    }
    println!(
        "skill_surface: commands_resolved={} flags_checked={} errors={}",
        report.commands_resolved,
        report.flags_checked,
        report.errors.len(),
    );
    assert!(
        report.errors.is_empty(),
        "skills/plexi-cli/SKILL.md documents CLI surface this binary does not have.\n\
         Fix the skill (or the CLI) in the same change — see skills/AGENTS.md.\n\n{}",
        report.errors.join("\n"),
    );
    assert!(
        report.commands_resolved >= MIN_COMMANDS_RESOLVED,
        "extraction collapse: only {} command paths resolved (floor {MIN_COMMANDS_RESOLVED}) — \
         the extractor is no longer seeing the skill's code blocks",
        report.commands_resolved,
    );
    assert!(
        report.flags_checked >= MIN_FLAGS_CHECKED,
        "extraction collapse: only {} flags checked (floor {MIN_FLAGS_CHECKED}) — \
         the extractor is no longer seeing the skill's code blocks",
        report.flags_checked,
    );
}

#[test]
fn skill_version_matches_binary() {
    let text = skill_text();
    let declared = text
        .lines()
        .find_map(|l| l.strip_prefix("plexi_version:"))
        .map(|v| v.trim().trim_matches('"').to_string())
        .expect("SKILL.md frontmatter has no plexi_version");
    assert_eq!(
        declared,
        env!("CARGO_PKG_VERSION"),
        "SKILL.md plexi_version must match Cargo.toml — `just bump` stamps both; \
         never hand-edit one without the other",
    );
}
