use super::{PlexiCommands, COMMANDS_FILE, APP_ID, list_global_scripts, is_executable, print_tip};
use std::process::Command;


/// Entry point for `plexi run <command_name>`.
/// Returns the exit code.
/// `plexi run` with no argument — list available commands from .plexi/commands.toml.
pub fn run_list_commands() -> i32 {
    log::info!("cli: run called with no command, listing available workspace commands");
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: could not determine current directory: {e}");
            return 1;
        }
    };

    let config_path = cwd.join(COMMANDS_FILE);
    let contents = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::info!("cli: no workspace commands.toml, falling back to global scripts");
            let scripts_dir = crate::config::config_dir().join("scripts");
            let global_scripts = list_global_scripts(&scripts_dir);
            if !global_scripts.is_empty() {
                println!("Built-in scripts:");
                for name in &global_scripts {
                    println!("  {name}");
                }
                println!();
                println!("Run one with: plexi run <script>");
                println!();
            }
            println!("No workspace commands configured.");
            println!();
            println!("To add project commands:");
            println!("  plexi workspace init");
            return 0;
        }
        Err(e) => {
            eprintln!("error: could not read {}: {e}", config_path.display());
            return 1;
        }
    };

    let config: PlexiCommands = match toml::from_str(&contents) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: failed to parse {COMMANDS_FILE}: {e}");
            return 1;
        }
    };

    if config.commands.is_empty() {
        println!("No commands defined in {COMMANDS_FILE}.");
        println!();
        println!("Add a command:");
        println!("  [commands]");
        println!("  dev = \"npm run dev\"");
        println!("  build = {{ run = \"cargo build\", description = \"Build the project\" }}");
        return 0;
    }

    println!("Available commands:");
    let mut names: Vec<&String> = config.commands.keys().collect();
    names.sort();
    for name in names {
        let entry = &config.commands[name];
        if let Some(desc) = entry.description() {
            println!("  {:20} {}  # {}", name, entry.run(), desc);
        } else {
            println!("  {:20} {}", name, entry.run());
        }
    }
    println!();
    println!("Run one with: plexi run <command>");
    0
}

pub fn run_command(command_name: &str) -> i32 {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: could not determine current directory: {e}");
            return 1;
        }
    };

    let config_path = cwd.join(COMMANDS_FILE);
    let contents = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // No workspace commands.toml — try global script from config_dir()/scripts/.
            let script_path = crate::config::config_dir().join("scripts").join(command_name);
            if script_path.is_file() && is_executable(&script_path) {
                log::info!("cli: running global script {:?}", script_path);
                let mut child_cmd = Command::new(&script_path);
                child_cmd.env("PLEXI_CONFIG_DIR", crate::config::config_dir());
                return match child_cmd.status() {
                    Ok(status) => status.code().unwrap_or(1),
                    Err(e) => {
                        eprintln!("error: failed to spawn script: {e}");
                        1
                    }
                };
            }
            eprintln!("error: no {COMMANDS_FILE} found in {}", cwd.display());
            eprintln!("Create a .plexi/commands.toml to define runnable commands.");
            return 1;
        }
        Err(e) => {
            eprintln!("error: could not read {}: {e}", config_path.display());
            return 1;
        }
    };

    let config: PlexiCommands = match toml::from_str(&contents) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: failed to parse {COMMANDS_FILE}: {e}");
            return 1;
        }
    };

    let cmd_entry = match config.commands.get(command_name) {
        Some(c) => c,
        None => {
            eprintln!("error: unknown command '{command_name}'");
            if config.commands.is_empty() {
                eprintln!("No commands defined in {COMMANDS_FILE}.");
            } else {
                let mut names: Vec<&str> = config.commands.keys().map(|s| s.as_str()).collect();
                names.sort();
                eprintln!("Available commands: {}", names.join(", "));
            }
            return 1;
        }
    };

    // Collect all required secret keys: global + command-specific
    let mut secret_keys: Vec<&str> = config.secrets.required.iter().map(|s| s.as_str()).collect();
    for k in cmd_entry.secrets() {
        if !secret_keys.contains(&k.as_str()) {
            secret_keys.push(k.as_str());
        }
    }

    // Resolve secrets via workspace-routed v3 resolver.
    let mut resolved: Vec<(String, zeroize::Zeroizing<String>)> = Vec::new();
    let mut missing: Vec<&str> = Vec::new();

    if !secret_keys.is_empty() {
        #[cfg(target_os = "macos")]
        {
            use crate::workspace_secrets::{
                resolve, MacKeychain, ResolveOutcome, WorkspaceConfig, WorkspaceSecrets,
            };
            let store = MacKeychain::new();
            // Load workspace context once — fail early if secrets are required but workspace
            // is not initialised (missing workspace.toml or secrets.toml).
            let ws_root = crate::app_registry::resolve_workspace_root(&cwd);
            let ws_context: Option<(String, WorkspaceSecrets)> = ws_root.as_ref().and_then(|root| {
                let cfg = match WorkspaceConfig::load(root) {
                    Ok(Some(c)) => c,
                    Ok(None) => {
                        log::warn!("run_command: workspace.toml missing at {}", root.display());
                        return None;
                    }
                    Err(e) => {
                        log::warn!("run_command: failed to load workspace.toml at {}: {e}", root.display());
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
                        log::warn!("run_command: failed to load secrets.toml at {}: {e}", root.display());
                        return None;
                    }
                };
                Some((cfg.id, router))
            });
            match ws_context {
                None => {
                    log::warn!("run_command: secrets required but workspace context unavailable in {}", cwd.display());
                    eprintln!("error: command requires secrets but workspace is not fully initialized.");
                    eprintln!("  → plexi workspace init   (creates workspace.toml + secrets.toml)");
                    eprintln!("  → plexi secret set <NAME>  (then store the required secrets)");
                    return 1;
                }
                Some((ws_id, router)) => {
                    for key in &secret_keys {
                        match resolve(&ws_id, APP_ID, key, &router, &store) {
                            ResolveOutcome::Found(value) => {
                                resolved.push((key.to_string(), value));
                            }
                            ResolveOutcome::HardMissing { reason } => {
                                log::warn!("run_command: secret '{key}' hard-missing: {reason}");
                                missing.push(key);
                            }
                            ResolveOutcome::PromptUser => {
                                log::info!("run_command: secret '{key}' not in workspace Keychain");
                                missing.push(key);
                            }
                        }
                    }
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            for key in &secret_keys {
                missing.push(key);
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
    let mut child_cmd = Command::new("sh");
    child_cmd.arg("-c").arg(cmd_entry.run());
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

// ── plexi routine subcommands ─────────────────────────────────────────────────

pub(super) const ROUTINES_FILE: &str = ".plexi/routines.toml";

/// Parsed `.plexi/routines.toml` for CLI use
#[derive(serde::Deserialize)]
pub(super) struct RoutinesCliConfig {
    #[serde(default)]
    pub(super) routine: Vec<RoutineCliDef>,
}

#[derive(serde::Deserialize)]
pub(super) struct RoutineCliDef {
    pub(super) name: String,
    pub(super) command: String,
    pub(super) schedule: String,
    #[serde(default)]
    pub(super) context: String,
    #[serde(default)]
    pub(super) ephemeral: bool,
}

/// `plexi routine list` — list routines from .plexi/routines.toml
#[cfg(test)]
mod command_parse_tests {
    use crate::cli::{CommandEntry, PlexiCommands};

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
        assert_eq!(entry.secrets(), &["API_KEY".to_string(), "DB_PASS".to_string()]);
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
        assert!(matches!(parsed.commands.get("build").unwrap(), CommandEntry::Simple(_)));
        assert!(matches!(parsed.commands.get("dev").unwrap(), CommandEntry::Full(_)));
    }
}
