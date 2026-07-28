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

fn check_flags(cmd: &clap::Command, path: &[String], tokens: &[&str], errors: &mut Vec<String>) {
    for tok in tokens {
        for flag in long_flags_in(tok) {
            if !has_long_flag(cmd, &flag) {
                errors.push(format!(
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
fn check_reference_block(root: &clap::Command, block: &Block, errors: &mut Vec<String>) {
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
                check_flags(cmd, path, &tokens, errors);
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
                    errors.push(format!(
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
                errors.push(format!(
                    "reference line does not start with a known subcommand: `{}`",
                    line.trim()
                ));
            }
            entry = None;
            entry_cmd = None;
            continue;
        }
        check_flags(cmd, &path, &tokens[idx..], errors);
        entry = Some(path);
        entry_cmd = Some(cmd);
    }
}

/// Bash example blocks: every `plexi …` invocation must start with a real
/// subcommand; deeper tokens descend while they resolve (positionals may be
/// lowercase, so a non-resolving token ends the walk without error), and every
/// `--flag` on the invocation must exist on the deepest resolved command.
fn check_bash_block(root: &clap::Command, block: &Block, errors: &mut Vec<String>) {
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
                errors.push(format!(
                    "bash example invokes `plexi {}`, which is not a known subcommand (line: `{}`)",
                    segment[0],
                    line.trim(),
                ));
            } else {
                check_flags(cmd, &path, &segment[idx..], errors);
            }
            i = segment_end;
        }
    }
}

#[test]
fn skill_surface_matches_cli() {
    let text = skill_text();
    let root = built_cli();
    let mut errors = Vec::new();
    for block in fenced_blocks(&text) {
        match block.lang.as_str() {
            "" => check_reference_block(&root, &block, &mut errors),
            "bash" | "sh" => check_bash_block(&root, &block, &mut errors),
            _ => {}
        }
    }
    assert!(
        errors.is_empty(),
        "skills/plexi-cli/SKILL.md documents CLI surface this binary does not have.\n\
         Fix the skill (or the CLI) in the same change — see skills/AGENTS.md.\n\n{}",
        errors.join("\n"),
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
