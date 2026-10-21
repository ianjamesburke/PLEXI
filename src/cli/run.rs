use super::{is_executable, print_tip, PlexiCommands, UserCommandScope, APP_ID};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolvedRunTarget {
    TomlCommand {
        scope: UserCommandScope,
        run: String,
        steps: Vec<String>,
        on_load: Vec<String>,
        secrets: Vec<String>,
    },
    GlobalScript(std::path::PathBuf),
}

/// Outcome of executing a sequence of steps (`steps = [...]` or `on_load = [...]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRunReport {
    pub exit_code: i32,
    pub total_steps: usize,
    pub completed_steps: usize,
    /// 1-based index of the step that failed (`1..=total_steps`), or `None` when all steps succeeded.
    pub failed_step_index: Option<usize>,
    pub failed_step: Option<String>,
    pub error_message: Option<String>,
}

/// A parsed step inside `steps = [...]` or `on_load = [...]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedStep {
    /// A Plexi CLI invocation with `"plexi"` as `argv[0]`.
    PlexiCli(Vec<String>),
    /// A shell command string to execute via `sh -c`.
    Shell(String),
}

const PLEXI_SUBCOMMANDS: &[&str] = &[
    "pane",
    "app",
    "context",
    "workspace",
    "secret",
    "secrets",
    "note",
    "config",
    "agent",
    "assistant",
    "host",
    "notify",
    "permissions",
    "needs-you",
    "ledger",
    "debug",
    "skill",
    "command-view",
    "changes",
    "account",
    "relay",
    "demo",
    "doctor",
    "routine",
    "events",
    "registry",
    "open",
    "close",
    "focus",
    "send",
    "capture",
    "list",
    "prev",
];

fn shell_escape_step_arg(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/' | b':'))
    {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// Substitute `$1`..`$9`, `${1}`..`${9}`, and `$@` in a step string when
/// `extra_args` are provided (or when `$1`..`$9` / `$@` appear outside single
/// quotes). Environment references such as `$HOME` or `$PLEXI_CONFIG_DIR` are
/// left untouched.
pub fn substitute_step_args(step: &str, extra_args: &[String]) -> String {
    let mut out = String::with_capacity(step.len());
    let chars: Vec<char> = step.chars().collect();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;

    while i < chars.len() {
        let ch = chars[i];
        if ch == '\\' && !in_single && i + 1 < chars.len() {
            out.push(ch);
            out.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
            out.push(ch);
            i += 1;
            continue;
        }
        if ch == '"' && !in_single {
            in_double = !in_double;
            out.push(ch);
            i += 1;
            continue;
        }
        if ch == '$' && !in_single && i + 1 < chars.len() {
            let next = chars[i + 1];
            if next == '@' {
                if in_double {
                    out.push_str(&extra_args.join(" "));
                } else {
                    let joined = extra_args
                        .iter()
                        .map(|a| shell_escape_step_arg(a))
                        .collect::<Vec<_>>()
                        .join(" ");
                    out.push_str(&joined);
                }
                i += 2;
                continue;
            }
            if next.is_ascii_digit() && next != '0' {
                let idx = (next as usize) - ('1' as usize);
                if let Some(arg) = extra_args.get(idx) {
                    if in_double {
                        out.push_str(arg);
                    } else {
                        out.push_str(&shell_escape_step_arg(arg));
                    }
                }
                i += 2;
                continue;
            }
            if next == '{' && i + 3 < chars.len() && chars[i + 3] == '}' {
                let digit = chars[i + 2];
                if digit.is_ascii_digit() && digit != '0' {
                    let idx = (digit as usize) - ('1' as usize);
                    if let Some(arg) = extra_args.get(idx) {
                        if in_double {
                            out.push_str(arg);
                        } else {
                            out.push_str(&shell_escape_step_arg(arg));
                        }
                    }
                    i += 4;
                    continue;
                }
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

fn strip_matching_outer_quotes(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            let inner = &s[1..s.len() - 1];
            let quote_char = first as char;
            if !inner.contains(quote_char) {
                return inner.to_string();
            }
        }
    }
    s.to_string()
}

fn is_plexi_binary_token(tok: &str) -> bool {
    tok == "plexi" || tok.starts_with("plexi-")
}

fn is_pane_new_prefix(tokens: &[String]) -> bool {
    matches!(
        tokens.iter().map(String::as_str).collect::<Vec<_>>().as_slice(),
        ["pane", "new", ..]
    ) || (tokens.len() >= 2
        && is_plexi_binary_token(&tokens[0])
        && tokens[1] == "pane"
        && tokens.get(2).map(String::as_str) == Some("new"))
}

/// Tokenize a step into words unless an unquoted shell operator appears before
/// a `--` separator. For `pane new ... -- <cmd>`, everything after `--` is kept
/// as a single command string so inner flags and quotes reach the spawned pane
/// intact.
fn tokenize_step_for_cli(step: &str) -> Option<Vec<String>> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut token_was_quoted = false;
    let mut in_single = false;
    let mut in_double = false;
    let mut indices = step.char_indices().peekable();

    while let Some((byte_idx, ch)) = indices.next() {
        if in_single {
            if ch == '\'' {
                in_single = false;
            } else {
                current.push(ch);
            }
            continue;
        }
        if in_double {
            if ch == '"' {
                in_double = false;
            } else if ch == '\\' {
                if let Some((_, next_ch)) = indices.next() {
                    current.push(next_ch);
                }
            } else if ch == '$' || ch == '`' {
                // Shell variable / command expansion inside double quotes: run via sh -c.
                return None;
            } else {
                current.push(ch);
            }
            continue;
        }

        // Unquoted context: reject shell control operators before `--`.
        if matches!(ch, '|' | ';' | '&' | '>' | '<' | '\n' | '`' | '$') {
            return None;
        }
        if ch == '\'' {
            in_single = true;
            in_token = true;
            token_was_quoted = true;
            continue;
        }
        if ch == '"' {
            in_double = true;
            in_token = true;
            token_was_quoted = true;
            continue;
        }
        if ch == '\\' {
            if let Some((_, next_ch)) = indices.next() {
                current.push(next_ch);
                in_token = true;
            }
            continue;
        }
        if ch.is_whitespace() {
            if in_token {
                let finished = std::mem::take(&mut current);
                in_token = false;
                let was_unquoted_dash_dash = !token_was_quoted && finished == "--";
                token_was_quoted = false;
                tokens.push(finished);
                if was_unquoted_dash_dash && is_pane_new_prefix(&tokens) {
                    let rest = step[byte_idx + ch.len_utf8()..].trim();
                    if !rest.is_empty() {
                        tokens.push(strip_matching_outer_quotes(rest));
                    }
                    return Some(tokens);
                }
            }
            continue;
        }
        current.push(ch);
        in_token = true;
    }

    if in_single || in_double {
        return None;
    }
    if in_token {
        tokens.push(current);
    }
    Some(tokens)
}

/// Classify a step string as either a Plexi CLI invocation or a shell snippet.
pub fn parse_step(step: &str) -> ParsedStep {
    let trimmed = step.trim();
    if trimmed.is_empty() {
        return ParsedStep::Shell(String::new());
    }
    if let Some(mut tokens) = tokenize_step_for_cli(trimmed) {
        if let Some(first) = tokens.first() {
            if is_plexi_binary_token(first) {
                if tokens.len() >= 2 {
                    tokens[0] = "plexi".to_string();
                    return ParsedStep::PlexiCli(tokens);
                }
            } else if PLEXI_SUBCOMMANDS.contains(&first.as_str()) {
                tokens.insert(0, "plexi".to_string());
                return ParsedStep::PlexiCli(tokens);
            }
        }
    }
    ParsedStep::Shell(trimmed.to_string())
}

/// Execute `steps` sequentially using `runner`, stopping at the first step
/// whose exit code is non-zero and reporting the 1-based step index.
pub fn run_steps_with<F>(
    steps: &[String],
    extra_args: &[String],
    mut runner: F,
) -> StepRunReport
where
    F: FnMut(usize, &str, &ParsedStep) -> i32,
{
    let total = steps.len();
    for (idx, raw_step) in steps.iter().enumerate() {
        let step_num = idx + 1;
        let rendered = substitute_step_args(raw_step, extra_args);
        let parsed = parse_step(&rendered);
        log::info!("commands: running step {step_num}/{total}: {rendered}");
        let code = runner(step_num, &rendered, &parsed);
        if code != 0 {
            let msg = format!("step {step_num}/{total} failed: {rendered} (exit {code})");
            log::warn!("commands: {msg}");
            eprintln!("error: {msg}");
            return StepRunReport {
                exit_code: code,
                total_steps: total,
                completed_steps: idx,
                failed_step_index: Some(step_num),
                failed_step: Some(rendered),
                error_message: Some(msg),
            };
        }
    }
    log::info!("commands: completed {total}/{total} step(s)");
    StepRunReport {
        exit_code: 0,
        total_steps: total,
        completed_steps: total,
        failed_step_index: None,
        failed_step: None,
        error_message: None,
    }
}

fn layout_from_flags(
    down: bool,
    left: bool,
    up: bool,
    right: bool,
    tab: bool,
    window: bool,
    overlay: bool,
) -> Option<&'static str> {
    if down {
        Some("split_v")
    } else if left {
        Some("split_left")
    } else if up {
        Some("split_above")
    } else if right {
        Some("split_h")
    } else if tab {
        Some("tab")
    } else if window {
        Some("new_window")
    } else if overlay {
        Some("overlay")
    } else {
        None
    }
}

/// Dispatch a parsed `ParsedStep::PlexiCli` in-process where possible, falling
/// back to spawning the current binary for uncommon subcommands.
pub fn dispatch_cli_step(
    argv: &[String],
    cwd: Option<&std::path::Path>,
    env_vars: &[(String, String)],
) -> i32 {
    use crate::cli::args::{AppCmd, Cli, Commands, PaneCmd};
    use clap::Parser;

    let parsed = match Cli::try_parse_from(argv) {
        Ok(cli) => cli,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };

    if let Some(socket) = parsed.socket {
        crate::cli::set_command_socket_override(socket);
    }

    match parsed.command {
        Some(Commands::Pane { cmd }) => match cmd {
            PaneCmd::New {
                cmd,
                name,
                down,
                left,
                up,
                right,
                tab,
                window,
                overlay,
                from,
                ephemeral,
                no_focus,
                cwd: step_cwd,
                worktree,
                result_schema,
                on_load,
                extra_args,
                agent,
                boot_timeout,
            } => {
                let agent = agent.map(|command| crate::cli::AgentBootRequest {
                    command,
                    timeout_secs: boot_timeout,
                });
                let layout =
                    layout_from_flags(down, left, up, right, tab, window, overlay).unwrap_or("split_h");
                let effective_cwd = step_cwd
                    .or_else(|| cwd.map(|p| p.display().to_string()));
                crate::cli::pane_new_cli(crate::cli::PaneNewCliArgs {
                    cmd: cmd.as_deref(),
                    name: name.as_deref(),
                    layout: Some(layout),
                    from_pane_id: from,
                    cwd: effective_cwd.as_deref(),
                    ephemeral,
                    no_focus,
                    extra_args: &extra_args,
                    agent: agent.as_ref(),
                    worktree: worktree.as_deref(),
                    result_schema: result_schema.as_deref(),
                    on_load: &on_load,
                    ..Default::default()
                })
            }
            PaneCmd::List { context, json: _ } => {
                let (context_id, current) = match context.as_deref() {
                    None => (None, false),
                    Some("current") => (None, true),
                    Some(s) => match s.parse::<u64>() {
                        Ok(id) => (Some(id), false),
                        Err(_) => {
                            eprintln!(
                                "error: --context value must be a numeric context ID or omitted for current context"
                            );
                            return 1;
                        }
                    },
                };
                crate::cli::pane_list_cli(context_id, current)
            }
            PaneCmd::Name { first, second } => {
                let (pane_id, name) = match second {
                    Some(title) => match first.parse::<u64>() {
                        Ok(id) => (Some(id), title),
                        Err(_) => {
                            eprintln!(
                                "error: expected a numeric pane ID as first argument, got {:?}",
                                first
                            );
                            return 1;
                        }
                    },
                    None => (None, first),
                };
                crate::cli::pane_set_title_cli(pane_id, &name)
            }
            PaneCmd::Focus { pane_id } => crate::cli::pane_focus_cli(pane_id),
            PaneCmd::Close { pane_id } => {
                let id = match pane_id {
                    Some(id) => id,
                    None => match std::env::var("PLEXI_PANE_ID")
                        .ok()
                        .and_then(|s| s.parse::<u64>().ok())
                    {
                        Some(id) => id,
                        None => {
                            eprintln!(
                                "error: PLEXI_PANE_ID is not set — pass a pane ID explicitly"
                            );
                            return 1;
                        }
                    },
                };
                crate::cli::pane_close_cli(id)
            }
            PaneCmd::Send {
                pane_id,
                text,
                submit,
                paste,
            } => crate::cli::pane_send_cli(pane_id, &text, submit, paste),
            PaneCmd::Command {
                pane_id,
                text,
                enter,
            } => crate::cli::pane_send_cli(pane_id, &text, enter, false),
            PaneCmd::Key { pane_id, key } => crate::cli::pane_key_cli(pane_id, &key),
            PaneCmd::Capture {
                pane_id,
                lines,
                full_output,
                from_cursor,
                plain,
            } => crate::cli::pane_capture_cli(pane_id, lines, full_output, from_cursor, plain),
            _ => spawn_plexi_subcommand(argv, cwd, env_vars),
        },
        Some(Commands::App {
            cmd:
                AppCmd::Open {
                    type_id,
                    mcp,
                    cli: cli_flag,
                    extra_args,
                    down,
                    left,
                    up,
                    right,
                    tab,
                    window,
                    split,
                    from,
                },
        }) => {
            let layout = match split.as_deref() {
                Some("down") => Some("split_v"),
                Some("left") => Some("split_left"),
                Some("up") => Some("split_above"),
                Some("right") => Some("split_h"),
                Some("tab") => Some("tab"),
                Some("window") => Some("new_window"),
                _ => layout_from_flags(down, left, up, right, tab, window, false),
            };
            if let Some(ref tid) = type_id {
                crate::cli::open_cli(tid, &extra_args, layout, from, None)
            } else if !mcp.is_empty() {
                let title = crate::cli::mcp_pane_title(&mcp);
                crate::cli::pane_new_cli(crate::cli::PaneNewCliArgs {
                    name: Some(title.as_str()),
                    layout,
                    from_pane_id: from,
                    app: Some("mcp-renderer"),
                    mcp: &mcp,
                    ..Default::default()
                })
            } else if let Some(binary) = cli_flag {
                crate::cli::open_cli_by_name(&binary, layout, from, None)
            } else {
                1
            }
        }
        _ => spawn_plexi_subcommand(argv, cwd, env_vars),
    }
}

fn spawn_plexi_subcommand(
    argv: &[String],
    cwd: Option<&std::path::Path>,
    env_vars: &[(String, String)],
) -> i32 {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: could not determine current executable: {e}");
            return 1;
        }
    };
    let mut cmd = Command::new(exe);
    cmd.args(&argv[1..]);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.env("PLEXI_CONFIG_DIR", crate::config::config_dir());
    for (k, v) in env_vars {
        cmd.env(k, v);
    }
    match cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("error: failed to spawn plexi step: {e}");
            1
        }
    }
}

fn run_shell_snippet(
    snippet: &str,
    extra_args: &[String],
    cwd: Option<&std::path::Path>,
    env_vars: &[(String, String)],
) -> i32 {
    let mut child_cmd = Command::new("sh");
    child_cmd.arg("-c").arg(snippet).arg("--");
    for arg in extra_args {
        child_cmd.arg(arg);
    }
    if let Some(dir) = cwd {
        child_cmd.current_dir(dir);
    }
    child_cmd.env("PLEXI_CONFIG_DIR", crate::config::config_dir());
    for (k, v) in env_vars {
        child_cmd.env(k, v);
    }
    match child_cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("error: failed to spawn command: {e}");
            1
        }
    }
}

/// Execute a list of steps (CLI invocations or shell snippets) sequentially.
pub fn execute_steps(
    steps: &[String],
    extra_args: &[String],
    cwd: Option<&std::path::Path>,
    env_vars: &[(String, String)],
) -> StepRunReport {
    run_steps_with(steps, extra_args, |_step_num, _rendered, parsed| match parsed {
        ParsedStep::PlexiCli(argv) => dispatch_cli_step(argv, cwd, env_vars),
        ParsedStep::Shell(snippet) => run_shell_snippet(snippet, extra_args, cwd, env_vars),
    })
}

/// Execute per-pane `on_load` steps after a pane is spawned. If `pane_id` is
/// known, it is passed both as `$1` and as `PLEXI_PANE_ID` for child steps.
pub fn run_on_load_steps(steps: &[String], pane_id: Option<u64>) -> StepRunReport {
    let extra_args: Vec<String> = pane_id.map(|id| vec![id.to_string()]).unwrap_or_default();
    let env_vars: Vec<(String, String)> = pane_id
        .map(|id| vec![("PLEXI_PANE_ID".to_string(), id.to_string())])
        .unwrap_or_default();
    let cwd = std::env::current_dir().ok();
    execute_steps(steps, &extra_args, cwd.as_deref(), &env_vars)
}

/// Format the output of `plexi run` / `plexi run --json` for a given workspace root.
pub(crate) fn format_list_commands_for_root(
    workspace_root: Option<&std::path::Path>,
    json: bool,
) -> Result<String, String> {
    let cmds = super::resolve_user_commands_checked(workspace_root)?;
    if json {
        return serde_json::to_string_pretty(&cmds)
            .map_err(|e| format!("failed to serialize commands JSON: {e}"));
    }

    let commands_file = super::commands_file();
    let has_ws_toml = workspace_root
        .is_some_and(|root| root.join(&commands_file) != super::global_commands_path() && root.join(&commands_file).exists());

    if cmds.is_empty() {
        if has_ws_toml {
            return Ok(format!(
                "No commands defined in {commands_file}.\n\n\
                 Add a command:\n\
                 \x20 [commands]\n\
                 \x20 dev = \"npm run dev\"\n\
                 \x20 build = {{ run = \"cargo build\", description = \"Build the project\" }}\n"
            ));
        }
        return Ok(
            "No workspace commands configured.\n\n\
             To add project commands:\n\
             \x20 plexi workspace init\n"
                .to_string(),
        );
    }

    let mut out = String::from("Available commands:\n");
    for cmd in &cmds {
        let scope_tag = format!("[{}]", cmd.scope.as_str());
        let target = cmd.run.as_deref().unwrap_or("(script)");
        if let Some(desc) = cmd.description.as_deref() {
            out.push_str(&format!(
                "  {:20} {:11} {}  # {}\n",
                cmd.name, scope_tag, target, desc
            ));
        } else {
            out.push_str(&format!("  {:20} {:11} {}\n", cmd.name, scope_tag, target));
        }
    }
    out.push_str("\nRun one with: plexi run <command>\n");
    Ok(out)
}

/// Resolve a named command across workspace `commands.toml` → global `commands.toml` → global `scripts/`.
pub(crate) fn resolve_command_target(
    workspace_root: Option<&std::path::Path>,
    command_name: &str,
) -> Result<Option<ResolvedRunTarget>, String> {
    let global_path = super::global_commands_path();

    // 1. Workspace `commands.toml` wins per name.
    if let Some(root) = workspace_root {
        let rel_file = super::commands_file();
        let ws_path = root.join(&rel_file);
        if ws_path != global_path {
            match std::fs::read_to_string(&ws_path) {
                Ok(contents) => {
                    let config: PlexiCommands = toml::from_str(&contents)
                        .map_err(|e| format!("failed to parse {rel_file}: {e}"))?;
                    if let Some(entry) = config.commands.get(command_name) {
                        return Ok(Some(ResolvedRunTarget::TomlCommand {
                            scope: UserCommandScope::Workspace,
                            run: entry.run().to_string(),
                            steps: entry.steps().to_vec(),
                            on_load: entry.on_load().to_vec(),
                            secrets: super::merged_secret_names(&config, entry),
                        }));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(format!("could not read {}: {e}", ws_path.display()));
                }
            }
        }
    }

    // 2. Global `config_dir()/commands.toml`.
    match std::fs::read_to_string(&global_path) {
        Ok(contents) => {
            let config: PlexiCommands = toml::from_str(&contents)
                .map_err(|e| format!("failed to parse {}: {e}", global_path.display()))?;
            if let Some(entry) = config.commands.get(command_name) {
                return Ok(Some(ResolvedRunTarget::TomlCommand {
                    scope: UserCommandScope::Global,
                    run: entry.run().to_string(),
                    steps: entry.steps().to_vec(),
                    on_load: entry.on_load().to_vec(),
                    secrets: super::merged_secret_names(&config, entry),
                }));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(format!("could not read {}: {e}", global_path.display()));
        }
    }

    // 3. Global script in `config_dir()/scripts/<command_name>`.
    let script_path = crate::config::config_dir()
        .join("scripts")
        .join(command_name);
    if script_path.is_file() && is_executable(&script_path) {
        return Ok(Some(ResolvedRunTarget::GlobalScript(script_path)));
    }

    Ok(None)
}

/// `plexi run` with no command argument — list available workspace and global commands.
pub fn run_list_commands(json: bool) -> i32 {
    log::info!(
        "cli: run called with no command, listing available commands (json={json})"
    );
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: could not determine current directory: {e}");
            return 1;
        }
    };

    let ws_root = super::resolve_commands_workspace_root(&cwd);
    match format_list_commands_for_root(ws_root.as_deref(), json) {
        Ok(text) => {
            print!("{text}");
            if !text.ends_with('\n') {
                println!();
            }
            0
        }
        Err(err) => {
            eprintln!("error: {err}");
            1
        }
    }
}

/// Entry point for `plexi run <command_name>`.
/// Returns the exit code.
pub fn run_command(command_name: &str, extra_args: &[String]) -> i32 {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: could not determine current directory: {e}");
            return 1;
        }
    };
    let ws_root = super::resolve_commands_workspace_root(&cwd);
    run_command_for_root(ws_root.as_deref(), &cwd, command_name, extra_args).exit_code
}

/// Execute a named command against an explicit `workspace_root` and `cwd`,
/// returning the full [`StepRunReport`].
pub fn run_command_for_root(
    ws_root: Option<&std::path::Path>,
    cwd: &std::path::Path,
    command_name: &str,
    extra_args: &[String],
) -> StepRunReport {
    let fail = |code: i32| StepRunReport {
        exit_code: code,
        total_steps: 0,
        completed_steps: 0,
        failed_step_index: None,
        failed_step: None,
        error_message: None,
    };

    let target = match resolve_command_target(ws_root, command_name) {
        Ok(Some(t)) => t,
        Ok(None) => {
            let commands_file = super::commands_file();
            let has_ws_toml = ws_root.is_some_and(|r| r.join(&commands_file).exists());
            let has_global_toml = super::global_commands_path().exists();
            let available = super::resolve_user_commands(ws_root);
            if !has_ws_toml && !has_global_toml && available.is_empty() {
                eprintln!("error: no {commands_file} found in {}", cwd.display());
                eprintln!("Create a {commands_file} to define runnable commands.");
                return fail(1);
            }
            eprintln!("error: unknown command '{command_name}'");
            if available.is_empty() {
                eprintln!("No commands defined in {commands_file}.");
            } else {
                let names: Vec<&str> = available.iter().map(|c| c.name.as_str()).collect();
                eprintln!("Available commands: {}", names.join(", "));
            }
            return fail(1);
        }
        Err(err) => {
            eprintln!("error: {err}");
            return fail(1);
        }
    };

    let (scope, run_snippet, steps, on_load, secret_keys) = match target {
        ResolvedRunTarget::GlobalScript(script_path) => {
            log::info!("cli: running global script {:?}", script_path);
            let mut child_cmd = Command::new(&script_path);
            for arg in extra_args {
                child_cmd.arg(arg);
            }
            child_cmd.current_dir(cwd);
            child_cmd.env("PLEXI_CONFIG_DIR", crate::config::config_dir());
            let code = match child_cmd.status() {
                Ok(status) => status.code().unwrap_or(1),
                Err(e) => {
                    eprintln!("error: failed to spawn script: {e}");
                    1
                }
            };
            return StepRunReport {
                exit_code: code,
                total_steps: 1,
                completed_steps: usize::from(code == 0),
                failed_step_index: (code != 0).then_some(1),
                failed_step: (code != 0).then(|| script_path.display().to_string()),
                error_message: None,
            };
        }
        ResolvedRunTarget::TomlCommand {
            scope,
            run,
            steps,
            on_load,
            secrets,
        } => (scope, run, steps, on_load, secrets),
    };
    log::info!(
        "cli: running command '{command_name}' from scope={} (steps={}, on_load={})",
        scope.as_str(),
        steps.len(),
        on_load.len()
    );

    // Resolve secrets via workspace-routed v3 resolver.
    let mut resolved: Vec<(String, zeroize::Zeroizing<String>)> = Vec::new();
    let mut missing: Vec<&str> = Vec::new();

    if !secret_keys.is_empty() {
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        {
            use crate::workspace::secrets::{
                resolve, system_store, ResolveOutcome, WorkspaceConfig, WorkspaceSecrets,
            };
            let store = system_store();
            // Load workspace context once — fail early if secrets are required but workspace
            // is not initialised (missing workspace.toml or secrets.toml).
            let ws_root_for_secrets = crate::app::registry::resolve_workspace_root(cwd)
                .or_else(|| ws_root.map(std::path::Path::to_path_buf));
            let ws_context: Option<(String, WorkspaceSecrets)> =
                ws_root_for_secrets.as_ref().and_then(|root| {
                    let cfg = match WorkspaceConfig::load(root) {
                        Ok(Some(c)) => c,
                        Ok(None) => {
                            log::warn!("run_command: workspace.toml missing at {}", root.display());
                            return None;
                        }
                        Err(e) => {
                            log::warn!(
                                "run_command: failed to load workspace.toml at {}: {e}",
                                root.display()
                            );
                            return None;
                        }
                    };
                    let router = match WorkspaceSecrets::load(root) {
                        Ok(Some(r)) => r,
                        Ok(None) => {
                            log::warn!("run_command: secrets.toml missing at {}", root.display());
                            return None;
                        }
                        Err(e) => {
                            log::warn!(
                                "run_command: failed to load secrets.toml at {}: {e}",
                                root.display()
                            );
                            return None;
                        }
                    };
                    Some((cfg.id, router))
                });
            match ws_context {
                None => {
                    log::warn!(
                        "run_command: secrets required but workspace context unavailable in {}",
                        cwd.display()
                    );
                    eprintln!(
                        "error: command requires secrets but workspace is not fully initialized."
                    );
                    eprintln!("  → plexi workspace init   (creates workspace.toml + secrets.toml)");
                    eprintln!("  → plexi secret set <NAME>  (then store the required secrets)");
                    return fail(1);
                }
                Some((ws_id, router)) => {
                    for key in &secret_keys {
                        match resolve(&ws_id, APP_ID, key, &router, store) {
                            ResolveOutcome::Found(value) => {
                                resolved.push((key.to_string(), value));
                            }
                            ResolveOutcome::HardMissing { reason } => {
                                log::warn!("run_command: secret '{key}' hard-missing: {reason}");
                                missing.push(key.as_str());
                            }
                            ResolveOutcome::PromptUser => {
                                log::info!("run_command: secret '{key}' not in workspace Keychain");
                                missing.push(key.as_str());
                            }
                        }
                    }
                }
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            for key in &secret_keys {
                missing.push(key.as_str());
            }
        }
    }

    if !missing.is_empty() {
        eprintln!("error: missing required secrets:");
        for key in &missing {
            eprintln!("  - {key}");
        }
        eprintln!();
        eprintln!("Set them with:");
        for key in &missing {
            eprintln!("  plexi secret set {key}");
        }
        return fail(1);
    }

    // If `steps` or `on_load` are declared, run sequentially through the step runner.
    if !steps.is_empty() || !on_load.is_empty() {
        let mut all_steps = if steps.is_empty() {
            vec![run_snippet]
        } else {
            steps
        };
        all_steps.extend(on_load);
        let env_vars: Vec<(String, String)> = resolved
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().to_string()))
            .collect();
        let report = execute_steps(&all_steps, extra_args, Some(cwd), &env_vars);
        if report.exit_code == 0 {
            print_tip("run `plexi run` to see all available commands.");
        }
        return report;
    }

    // Spawn the single-step shell command via sh -c with secrets injected as env vars.
    // PLEXI_CONFIG_DIR lets scripts reference channel-correct paths without hardcoding ~/.plexi/.
    // Extra positional args from `plexi run <cmd> -- arg1 arg2` are passed via `sh -c '...' -- arg1 arg2`,
    // making them available as $1, $2, … inside the shell snippet.
    let mut child_cmd = Command::new("sh");
    child_cmd.arg("-c").arg(&run_snippet).arg("--");
    for arg in extra_args {
        child_cmd.arg(arg);
    }
    child_cmd.current_dir(cwd);
    child_cmd.env("PLEXI_CONFIG_DIR", crate::config::config_dir());
    for (key, value) in &resolved {
        child_cmd.env(key, value);
    }

    let code = match child_cmd.status() {
        Ok(status) => {
            let code = status.code().unwrap_or(1);
            if code == 0 {
                print_tip("run `plexi run` to see all available commands.");
            } else if code == 127 {
                eprintln!(
                    "hint: command exited with 'not found' (127). If your command references \
                     $PLEXI_CONFIG_DIR, check that the script exists in '{}'",
                    crate::config::config_dir().join("scripts").display()
                );
            }
            code
        }
        Err(e) => {
            eprintln!("error: failed to spawn command: {e}");
            1
        }
    };

    StepRunReport {
        exit_code: code,
        total_steps: 1,
        completed_steps: usize::from(code == 0),
        failed_step_index: (code != 0).then_some(1),
        failed_step: (code != 0).then_some(run_snippet),
        error_message: None,
    }
}

#[cfg(test)]
mod command_parse_tests {
    use super::{format_list_commands_for_root, resolve_command_target, ResolvedRunTarget};
    use crate::cli::{CommandEntry, PlexiCommands, UserCommandScope};

    #[test]
    fn simple_string_command() {
        let toml = r#"
[commands]
build = "cargo build"
"#;
        let parsed: PlexiCommands = toml::from_str(toml).unwrap();
        let entry = parsed.commands.get("build").unwrap();
        assert_eq!(entry.run(), "cargo build");
        assert!(entry.description().is_none());
        assert!(entry.secrets().is_empty());
    }

    #[test]
    fn full_inline_table_command() {
        let toml = r#"
[commands]
dev = { run = "npm run dev", description = "Start dev server" }
"#;
        let parsed: PlexiCommands = toml::from_str(toml).unwrap();
        let entry = parsed.commands.get("dev").unwrap();
        assert_eq!(entry.run(), "npm run dev");
        assert_eq!(entry.description(), Some("Start dev server"));
        assert!(entry.secrets().is_empty());
    }

    #[test]
    fn full_inline_table_with_secrets() {
        let toml = r#"
[commands]
deploy = { run = "./deploy.sh", secrets = ["API_KEY", "DB_PASS"] }
"#;
        let parsed: PlexiCommands = toml::from_str(toml).unwrap();
        let entry = parsed.commands.get("deploy").unwrap();
        assert_eq!(entry.run(), "./deploy.sh");
        assert_eq!(
            entry.secrets(),
            &["API_KEY".to_string(), "DB_PASS".to_string()]
        );
    }

    #[test]
    fn nested_section_old_format_still_parses() {
        // [commands.dev]\nrun = "..." is TOML-equivalent to dev = { run = "..." }
        let toml = r#"
[commands.dev]
run = "npm run dev"
"#;
        let parsed: PlexiCommands = toml::from_str(toml).unwrap();
        let entry = parsed.commands.get("dev").unwrap();
        assert_eq!(entry.run(), "npm run dev");
    }

    #[test]
    fn mixed_simple_and_full_commands() {
        let toml = r#"
[commands]
build = "cargo build"
dev = { run = "npm run dev", description = "Start dev server" }
"#;
        let parsed: PlexiCommands = toml::from_str(toml).unwrap();
        assert_eq!(parsed.commands.len(), 2);
        assert!(matches!(
            parsed.commands.get("build").unwrap(),
            CommandEntry::Simple(_)
        ));
        assert!(matches!(
            parsed.commands.get("dev").unwrap(),
            CommandEntry::Full(_)
        ));
    }

    #[test]
    fn run_list_human_and_json_layer_global_and_workspace() {
        let _g = crate::config::set_test_channel("test");
        let ws = tempfile::tempdir().unwrap();
        let ws_dir = ws.path().join(".plexi-test");
        std::fs::create_dir_all(&ws_dir).unwrap();
        std::fs::write(
            ws_dir.join("commands.toml"),
            r#"
[secrets]
required = ["SHARED_KEY"]

[commands]
bar = { run = "echo ws-bar", description = "Workspace bar", secrets = ["BAR_TOKEN"] }
foo = { run = "echo ws-foo", description = "Workspace foo" }
"#,
        )
        .unwrap();

        let profile = tempfile::tempdir().unwrap();
        std::fs::write(
            profile.path().join("commands.toml"),
            r#"
[commands]
foo = { run = "echo global-foo", description = "Global foo" }
global_only = { run = "echo global-only", secrets = ["GLOBAL_SECRET"] }
"#,
        )
        .unwrap();
        let _pg = crate::config::set_test_profile_dir(profile.path().to_path_buf());

        let human = format_list_commands_for_root(Some(ws.path()), false).unwrap();
        assert!(human.contains("Available commands:"));
        assert!(human.contains("bar") && human.contains("[workspace]") && human.contains("echo ws-bar"));
        assert!(human.contains("foo") && human.contains("[workspace]") && human.contains("echo ws-foo"));
        assert!(!human.contains("echo global-foo"));
        assert!(
            human.contains("global_only")
                && human.contains("[global]")
                && human.contains("echo global-only")
        );

        let json_str = format_list_commands_for_root(Some(ws.path()), true).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["name"], "bar");
        assert_eq!(arr[0]["scope"], "workspace");
        assert_eq!(arr[0]["description"], "Workspace bar");
        assert_eq!(arr[0]["run"], "echo ws-bar");
        assert_eq!(arr[0]["secrets"], serde_json::json!(["SHARED_KEY", "BAR_TOKEN"]));

        assert_eq!(arr[1]["name"], "foo");
        assert_eq!(arr[1]["scope"], "workspace");
        assert_eq!(arr[1]["run"], "echo ws-foo");

        assert_eq!(arr[2]["name"], "global_only");
        assert_eq!(arr[2]["scope"], "global");
        assert_eq!(arr[2]["run"], "echo global-only");
        assert_eq!(arr[2]["secrets"], serde_json::json!(["GLOBAL_SECRET"]));

        // And target resolution matches: `foo` resolves to workspace, `global_only` resolves to global.
        assert_eq!(
            resolve_command_target(Some(ws.path()), "foo").unwrap(),
            Some(ResolvedRunTarget::TomlCommand {
                scope: UserCommandScope::Workspace,
                run: "echo ws-foo".to_string(),
                steps: Vec::new(),
                on_load: Vec::new(),
                secrets: vec!["SHARED_KEY".to_string()],
            })
        );
        assert_eq!(
            resolve_command_target(Some(ws.path()), "global_only").unwrap(),
            Some(ResolvedRunTarget::TomlCommand {
                scope: UserCommandScope::Global,
                run: "echo global-only".to_string(),
                steps: Vec::new(),
                on_load: Vec::new(),
                secrets: vec!["GLOBAL_SECRET".to_string()],
            })
        );
    }

    #[test]
    fn parse_commands_toml_accepts_steps_and_rejects_run_plus_steps_or_empty_steps() {
        let valid = r#"
[commands.play]
description = "Start mixer + muted nooise"
steps = [
  "pane new --right -- mixer",
  "pane new --down -- nooise --mute",
]
on_load = "pane name mixer"
"#;
        let parsed: PlexiCommands = toml::from_str(valid).expect("valid stepped command");
        let play = parsed.commands.get("play").expect("play entry");
        assert_eq!(
            play.steps(),
            &[
                "pane new --right -- mixer".to_string(),
                "pane new --down -- nooise --mute".to_string(),
            ]
        );
        assert_eq!(play.on_load(), &["pane name mixer".to_string()]);
        assert_eq!(play.description(), Some("Start mixer + muted nooise"));

        let both = r#"
[commands.bad]
run = "echo hi"
steps = ["echo bye"]
"#;
        let err = match toml::from_str::<PlexiCommands>(both) {
            Ok(_) => panic!("expected error for run + steps"),
            Err(e) => e.to_string(),
        };
        assert!(
            err.contains("cannot specify both `run` and `steps`"),
            "unexpected error: {err}"
        );

        let empty = r#"
[commands.empty]
steps = []
"#;
        let err = match toml::from_str::<PlexiCommands>(empty) {
            Ok(_) => panic!("expected error for empty steps"),
            Err(e) => e.to_string(),
        };
        assert!(
            err.contains("`steps` cannot be empty"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn substitute_step_args_replaces_positional_and_at() {
        let args = vec!["alpha".to_string(), "two words".to_string()];
        assert_eq!(
            super::substitute_step_args("pane new --right -- echo $1 and $2 ($@)", &args),
            "pane new --right -- echo alpha and 'two words' (alpha 'two words')"
        );
        assert_eq!(
            super::substitute_step_args("echo $1 $3", &args),
            "echo alpha "
        );
    }

    #[test]
    fn failing_step_reports_one_based_index_and_stops_later_steps() {
        let dir = tempfile::tempdir().unwrap();
        let marker1 = dir.path().join("step1.txt");
        let marker3 = dir.path().join("step3.txt");
        let steps = vec![
            format!("touch '{}'", marker1.display()),
            "false".to_string(),
            format!("touch '{}'", marker3.display()),
        ];

        let report = super::execute_steps(&steps, &[], Some(dir.path()), &[]);
        assert_eq!(report.exit_code, 1);
        assert_eq!(report.total_steps, 3);
        assert_eq!(report.completed_steps, 1);
        assert_eq!(report.failed_step_index, Some(2));
        assert_eq!(report.failed_step.as_deref(), Some("false"));
        assert_eq!(
            report.error_message.as_deref(),
            Some("step 2/3 failed: false (exit 1)")
        );
        assert!(marker1.exists(), "step 1 should have run before step 2 failed");
        assert!(
            !marker3.exists(),
            "step 3 must not run after step 2 fails"
        );
    }

    #[test]
    fn config_check_validates_user_command_keys_and_rejects_reserved_shortcut_collision() {
        let dir = tempfile::tempdir().unwrap();
        let valid_cfg = dir.path().join("valid.toml");
        std::fs::write(
            &valid_cfg,
            r#"
config_version = 1

[keys]
"cmd+shift+p" = "run:play"
"#,
        )
        .unwrap();
        let diags = crate::config::validate_from_path(&valid_cfg);
        assert!(
            diags.is_empty(),
            "cmd+shift+p = run:play must pass config check, got {diags:?}"
        );

        let colliding_cfg = dir.path().join("colliding.toml");
        std::fs::write(
            &colliding_cfg,
            r#"
config_version = 1

[keys]
"cmd+p" = "run:play"
"#,
        )
        .unwrap();
        let diags = crate::config::validate_from_path(&colliding_cfg);
        assert!(
            diags.iter().any(|d| d.is_error() && d.to_string().contains("reserved shortcut")),
            "binding that collides with reserved shortcut cmd+p must be rejected by config check, got {diags:?}"
        );

        let bad_target_cfg = dir.path().join("bad_target.toml");
        std::fs::write(
            &bad_target_cfg,
            r#"
config_version = 1

[keys]
"cmd+shift+p" = "play"
"#,
        )
        .unwrap();
        let diags = crate::config::validate_from_path(&bad_target_cfg);
        assert!(
            diags.iter().any(|d| d.is_error() && d.to_string().contains("run:<command>")),
            "target without run: prefix must be rejected by config check, got {diags:?}"
        );
    }
}
