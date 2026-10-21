use super::{is_executable, print_tip, PlexiCommands, UserCommandScope, APP_ID};
use std::process::Command;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ResolvedRunTarget {
    TomlCommand {
        scope: UserCommandScope,
        run: String,
        secrets: Vec<String>,
    },
    GlobalScript(std::path::PathBuf),
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
    let target = match resolve_command_target(ws_root.as_deref(), command_name) {
        Ok(Some(t)) => t,
        Ok(None) => {
            let commands_file = super::commands_file();
            let has_ws_toml = ws_root
                .as_ref()
                .is_some_and(|r| r.join(&commands_file).exists());
            let has_global_toml = super::global_commands_path().exists();
            let available = super::resolve_user_commands(ws_root.as_deref());
            if !has_ws_toml && !has_global_toml && available.is_empty() {
                eprintln!("error: no {commands_file} found in {}", cwd.display());
                eprintln!("Create a {commands_file} to define runnable commands.");
                return 1;
            }
            eprintln!("error: unknown command '{command_name}'");
            if available.is_empty() {
                eprintln!("No commands defined in {commands_file}.");
            } else {
                let names: Vec<&str> = available.iter().map(|c| c.name.as_str()).collect();
                eprintln!("Available commands: {}", names.join(", "));
            }
            return 1;
        }
        Err(err) => {
            eprintln!("error: {err}");
            return 1;
        }
    };

    let (scope, run_snippet, secret_keys) = match target {
        ResolvedRunTarget::GlobalScript(script_path) => {
            log::info!("cli: running global script {:?}", script_path);
            let mut child_cmd = Command::new(&script_path);
            for arg in extra_args {
                child_cmd.arg(arg);
            }
            child_cmd.env("PLEXI_CONFIG_DIR", crate::config::config_dir());
            return match child_cmd.status() {
                Ok(status) => status.code().unwrap_or(1),
                Err(e) => {
                    eprintln!("error: failed to spawn script: {e}");
                    1
                }
            };
        }
        ResolvedRunTarget::TomlCommand {
            scope,
            run,
            secrets,
        } => (scope, run, secrets),
    };
    log::info!(
        "cli: running command '{command_name}' from scope={}",
        scope.as_str()
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
            let ws_root = crate::app::registry::resolve_workspace_root(&cwd);
            let ws_context: Option<(String, WorkspaceSecrets)> =
                ws_root.as_ref().and_then(|root| {
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
                    return 1;
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
        return 1;
    }

    // Spawn the command via sh -c with secrets injected as env vars.
    // PLEXI_CONFIG_DIR lets scripts reference channel-correct paths without hardcoding ~/.plexi/.
    // Extra positional args from `plexi run <cmd> -- arg1 arg2` are passed via `sh -c '...' -- arg1 arg2`,
    // making them available as $1, $2, … inside the shell snippet.
    let mut child_cmd = Command::new("sh");
    child_cmd.arg("-c").arg(&run_snippet).arg("--");
    for arg in extra_args {
        child_cmd.arg(arg);
    }
    child_cmd.env("PLEXI_CONFIG_DIR", crate::config::config_dir());
    for (key, value) in &resolved {
        child_cmd.env(key, value);
    }

    match child_cmd.status() {
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
                secrets: vec!["SHARED_KEY".to_string()],
            })
        );
        assert_eq!(
            resolve_command_target(Some(ws.path()), "global_only").unwrap(),
            Some(ResolvedRunTarget::TomlCommand {
                scope: UserCommandScope::Global,
                run: "echo global-only".to_string(),
                secrets: vec!["GLOBAL_SECRET".to_string()],
            })
        );
    }
}
