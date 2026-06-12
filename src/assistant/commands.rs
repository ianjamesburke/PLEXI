//! Slash command parsing for the host Assistant.
//!
//! Commands are recognized only when `/` is the first non-whitespace
//! character in the composer (spec: docs/prm/assistant-host-app.md). The
//! parser is a pure function — no model or store access — so it is unit
//! tested in isolation.

/// One parsed slash command: the bare name (no leading `/`) plus the raw
/// argument string after the name (trimmed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub name: String,
    pub args: String,
}

/// The built-in command table from the spec: `(name, purpose)`.
/// `/clear`, `/new`, and `/help` are implemented in Phase 1; the rest are
/// recognized and answered with a "not yet implemented" assistant row.
pub const BUILT_IN_COMMANDS: &[(&str, &str)] = &[
    ("help", "Show built-in commands, installed skills, and tool packs."),
    ("clear", "Start a new conversation in the same workspace."),
    ("resume", "Open a previous Assistant session."),
    ("compact", "Summarize older turns and keep the active task context."),
    (
        "context",
        "Show token use, loaded instructions, active pane/app context, and enabled tools.",
    ),
    ("memory", "Show agent prompt files and future memory state."),
    ("model", "Switch model tier or backend for this session/agent."),
    ("effort", "Switch reasoning effort for this session/agent."),
    ("agent", "Switch, inspect, create, or edit agents."),
    ("settings", "Open Assistant settings."),
    ("config", "Alias for /settings."),
    (
        "permissions",
        "Open grant rules, pending grants, denied tools, and audit history.",
    ),
    (
        "tools",
        "Show enabled host tools, app connectors, MCP tools, and tool collections.",
    ),
    ("apps", "Show app connectors available in the workspace."),
    ("skills", "Show installed skills and marketplace skill packs."),
    ("install", "Install a local or marketplace skill/tool/agent package."),
    ("hooks", "Show lifecycle hooks and their source."),
    (
        "audit",
        "Show recent Assistant tool calls, grants, app writes, and denied attempts.",
    ),
    ("revoke", "Revoke a persisted grant by target id."),
    ("export", "Export the current transcript and tool-call log."),
    ("rewind", "Restore the conversation to an earlier checkpoint."),
    ("new", "Create a new named conversation without deleting the current one."),
    ("history", "Open conversation history and checkpoint browser."),
];

/// Commands with a real implementation (Phase 1: help/clear/new; Phase 2:
/// tools/permissions/revoke/audit). Everything else in `BUILT_IN_COMMANDS`
/// is recognized but stubbed.
pub const IMPLEMENTED_COMMANDS: &[&str] = &[
    "help",
    "clear",
    "new",
    "tools",
    "permissions",
    "revoke",
    "audit",
];

/// Parse `input` as a slash command. Returns `None` when the input is not a
/// command: empty, `/` not the first non-whitespace character, or a bare `/`
/// with no name.
pub fn parse_slash_command(input: &str) -> Option<ParsedCommand> {
    let trimmed = input.trim_start();
    let rest = trimmed.strip_prefix('/')?;
    let mut parts = rest.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("");
    if name.is_empty() {
        return None;
    }
    let args = parts.next().unwrap_or("").trim().to_string();
    Some(ParsedCommand {
        name: name.to_string(),
        args,
    })
}

/// True while the composer should show the command picker: `/` is the first
/// non-whitespace character and the user is still typing the command name
/// (no whitespace after it yet).
pub fn picker_active(input: &str) -> bool {
    let trimmed = input.trim_start();
    match trimmed.strip_prefix('/') {
        Some(rest) => !rest.contains(char::is_whitespace),
        None => false,
    }
}

/// Filter the built-in command table by a picker query (the text typed after
/// `/`). Empty query returns the full table.
pub fn filter_commands(query: &str) -> Vec<(&'static str, &'static str)> {
    let q = query.to_lowercase();
    BUILT_IN_COMMANDS
        .iter()
        .filter(|(name, _)| name.contains(&q))
        .copied()
        .collect()
}

/// True if `name` appears in the built-in command table.
pub fn is_builtin(name: &str) -> bool {
    BUILT_IN_COMMANDS.iter().any(|(n, _)| *n == name)
}

/// Render the `/help` listing.
pub fn help_text() -> String {
    let mut out = String::from("Built-in commands:\n");
    for (name, purpose) in BUILT_IN_COMMANDS {
        let status = if IMPLEMENTED_COMMANDS.contains(name) {
            ""
        } else {
            " (not yet implemented)"
        };
        out.push_str(&format!("/{name} — {purpose}{status}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_command_with_args() {
        let cmd = parse_slash_command("/new project notes").unwrap();
        assert_eq!(cmd.name, "new");
        assert_eq!(cmd.args, "project notes");
    }

    #[test]
    fn parses_command_without_args() {
        let cmd = parse_slash_command("/clear").unwrap();
        assert_eq!(cmd.name, "clear");
        assert_eq!(cmd.args, "");
    }

    #[test]
    fn leading_whitespace_is_allowed() {
        let cmd = parse_slash_command("   /help").unwrap();
        assert_eq!(cmd.name, "help");
    }

    #[test]
    fn slash_mid_text_is_not_a_command() {
        assert_eq!(parse_slash_command("look at /etc/hosts"), None);
        assert_eq!(parse_slash_command("a/b"), None);
    }

    #[test]
    fn bare_slash_is_not_a_command() {
        assert_eq!(parse_slash_command("/"), None);
        assert_eq!(parse_slash_command("  / "), None);
    }

    #[test]
    fn empty_input_is_not_a_command() {
        assert_eq!(parse_slash_command(""), None);
        assert_eq!(parse_slash_command("   "), None);
    }

    #[test]
    fn unrecognized_name_still_parses() {
        let cmd = parse_slash_command("/frobnicate now").unwrap();
        assert_eq!(cmd.name, "frobnicate");
        assert!(!is_builtin(&cmd.name));
    }

    #[test]
    fn picker_opens_on_slash_and_closes_after_name() {
        assert!(picker_active("/"));
        assert!(picker_active("/cl"));
        assert!(picker_active("  /he"));
        assert!(!picker_active("/clear "));
        assert!(!picker_active("hello /"));
        assert!(!picker_active(""));
    }

    #[test]
    fn filter_matches_substring() {
        let all = filter_commands("");
        assert_eq!(all.len(), BUILT_IN_COMMANDS.len());
        let hits = filter_commands("perm");
        assert_eq!(hits, vec![BUILT_IN_COMMANDS
            .iter()
            .copied()
            .find(|(n, _)| *n == "permissions")
            .unwrap()]);
        assert!(filter_commands("zzz").is_empty());
    }

    #[test]
    fn builtin_table_covers_phase1_commands() {
        for name in IMPLEMENTED_COMMANDS {
            assert!(is_builtin(name), "{name} missing from BUILT_IN_COMMANDS");
        }
        assert!(help_text().contains("/clear"));
        assert!(help_text().contains("not yet implemented"));
    }
}
