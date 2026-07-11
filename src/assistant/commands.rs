//! Slash command parsing for the host Assistant.
//!
//! Commands are recognized only when `/` is the first non-whitespace
//! character in the composer (spec: docs/assistant-host-app.md). The
//! parser is a pure function — no model or store access — so it is unit
//! tested in isolation.

/// One parsed slash command: the bare name (no leading `/`) plus the raw
/// argument string after the name (trimmed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub name: String,
    pub args: String,
}

/// The built-in command table: `(name, purpose)`. Every entry maps to a real
/// handler in `AssistantModel::execute_command`; unimplemented ideas live in
/// stint tasks and `docs/assistant-host-app.md`, never in this surface.
pub const BUILT_IN_COMMANDS: &[(&str, &str)] = &[
    ("help", "List the built-in commands."),
    ("clear", "Start a new conversation in the same workspace."),
    (
        "new",
        "Create a new named conversation without deleting the current one.",
    ),
    (
        "tools",
        "Show app connector tools and event streams with their broker decisions.",
    ),
    ("apps", "Show app connectors available in the workspace."),
    (
        "permissions",
        "Show persisted grants for the assistant actor.",
    ),
    ("revoke", "Revoke a persisted grant by target id."),
    (
        "audit",
        "Show recent Assistant tool calls, grants, app writes, and denied attempts.",
    ),
    (
        "thoughts",
        "Show or hide the model's reasoning for every turn.",
    ),
    ("model", "Show or change the model tier for this session."),
    ("agent", "List, inspect, create, edit, or switch Assistant agents."),
    ("effort", "Show or change reasoning effort for this session."),
    (
        "settings",
        "Show resolved Assistant settings and their sources.",
    ),
    ("config", "Show Assistant settings (alias for /settings)."),
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

/// Render the `/help` listing as markdown (the transcript renders assistant
/// rows through CommonMark).
pub fn help_text() -> String {
    let mut out = String::from("**Built-in commands:**\n\n");
    for (name, purpose) in BUILT_IN_COMMANDS {
        out.push_str(&format!("- `/{name}` — {purpose}\n"));
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
        assert!(!BUILT_IN_COMMANDS.iter().any(|(n, _)| *n == cmd.name));
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
        assert_eq!(
            hits,
            vec![BUILT_IN_COMMANDS
                .iter()
                .copied()
                .find(|(n, _)| *n == "permissions")
                .unwrap()]
        );
        assert!(filter_commands("zzz").is_empty());
    }

    #[test]
    fn help_lists_every_builtin_and_marks_none_unimplemented() {
        let help = help_text();
        for (name, _) in BUILT_IN_COMMANDS {
            assert!(
                help.contains(&format!("/{name}")),
                "{name} missing from /help"
            );
        }
        assert!(
            !help.contains("not yet implemented"),
            "no built-in should advertise itself as unimplemented"
        );
    }

    #[test]
    fn settings_and_model_handlers_are_exposed_in_help_and_picker() {
        let names: Vec<&str> = BUILT_IN_COMMANDS.iter().map(|(name, _)| *name).collect();

        assert!(names.contains(&"model"));
        assert!(names.contains(&"settings"));
        assert!(names.contains(&"config"));
        assert!(help_text().contains("/model"));
        assert_eq!(
            filter_commands("config")
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>(),
            vec!["config"]
        );
    }
}
