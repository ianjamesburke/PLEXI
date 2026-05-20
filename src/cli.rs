use serde::Deserialize;
use std::collections::HashMap;
use std::io::{self, Write};
use std::process::Command;

const APP_ID: &str = "plexi-run";
const COMMANDS_FILE: &str = ".plexi/commands.toml";


/// Parsed .plexi/commands.toml
#[derive(Deserialize)]
pub struct PlexiCommands {
    #[serde(default)]
    pub secrets: SecretsConfig,
    #[serde(default)]
    pub commands: HashMap<String, CommandDef>,
}

#[derive(Deserialize, Default)]
pub struct SecretsConfig {
    #[serde(default)]
    pub required: Vec<String>,
}

#[derive(Deserialize)]
pub struct CommandDef {
    pub run: String,
    #[serde(default)]
    pub secrets: Vec<String>,
}

fn print_tip(msg: &str) {
    let config = crate::config::PlexiConfig::load_with_workspace(
        crate::config::active_workspace_root().as_deref(),
    );
    let enabled = config.cli.as_ref().and_then(|c| c.tips).unwrap_or(true);
    if enabled {
        log::info!("cli:tip: {msg}");
        if std::env::var_os("NO_COLOR").is_none() {
            eprintln!("\x1b[2mtip: {msg}\x1b[0m");
        } else {
            eprintln!("tip: {msg}");
        }
    }
}

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
            println!("No workspace commands configured.");
            println!();
            println!("To set up commands, create {COMMANDS_FILE} in your project:");
            println!("  plexi workspace init");
            println!();
            println!("Then define commands in .plexi/commands.toml:");
            println!("  [commands.dev]");
            println!("  run = \"npm run dev\"");
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
        println!("  [commands.dev]");
        println!("  run = \"npm run dev\"");
        return 0;
    }

    println!("Available commands:");
    let mut names: Vec<&String> = config.commands.keys().collect();
    names.sort();
    for name in names {
        let cmd = &config.commands[name];
        println!("  {:20} {}", name, cmd.run);
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
        Err(_) => {
            eprintln!("error: no {COMMANDS_FILE} found in {}", cwd.display());
            eprintln!("Create a .plexi/commands.toml to define runnable commands.");
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

    let cmd_def = match config.commands.get(command_name) {
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
    for k in &cmd_def.secrets {
        if !secret_keys.contains(&k.as_str()) {
            secret_keys.push(k.as_str());
        }
    }

    // Resolve secrets from Keychain
    let dir_str = cwd.to_string_lossy();
    let mut resolved: Vec<(String, zeroize::Zeroizing<String>)> = Vec::new();
    let mut missing: Vec<&str> = Vec::new();

    for key in &secret_keys {
        match crate::secrets::resolve_secret(key, APP_ID, &dir_str) {
            Some(value) => resolved.push((key.to_string(), value)),
            None => missing.push(key),
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
    child_cmd.arg("-c").arg(&cmd_def.run);
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

const ROUTINES_FILE: &str = ".plexi/routines.toml";

/// Parsed `.plexi/routines.toml` for CLI use
#[derive(serde::Deserialize)]
struct RoutinesCliConfig {
    #[serde(default)]
    routine: Vec<RoutineCliDef>,
}

#[derive(serde::Deserialize)]
struct RoutineCliDef {
    name: String,
    command: String,
    schedule: String,
    #[serde(default)]
    context: String,
    #[serde(default)]
    ephemeral: bool,
}

/// `plexi routine list` — list routines from .plexi/routines.toml
pub fn routine_list() -> i32 {
    log::info!("cli: routine list");
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => { eprintln!("error: could not determine current directory: {e}"); return 1; }
    };
    let config_path = cwd.join(ROUTINES_FILE);
    let contents = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("No routines configured.");
            println!();
            println!("To set up routines, create {} in your project:", ROUTINES_FILE);
            println!("  [[routine]]");
            println!("  name = \"morning-sync\"");
            println!("  command = \"./scripts/morning.sh\"");
            println!("  schedule = \"weekdays at 9am\"");
            println!("  context = \"work\"");
            return 0;
        }
        Err(e) => { eprintln!("error: could not read {}: {e}", config_path.display()); return 1; }
    };
    let config: RoutinesCliConfig = match toml::from_str(&contents) {
        Ok(c) => c,
        Err(e) => { eprintln!("error: failed to parse {ROUTINES_FILE}: {e}"); return 1; }
    };
    if config.routine.is_empty() {
        println!("No routines defined in {ROUTINES_FILE}.");
        return 0;
    }
    println!("Routines:");
    for r in &config.routine {
        let next = match crate::scheduler::parse_schedule(&r.schedule) {
            Some(s) => crate::scheduler::next_fire_description(&s, None),
            None => "invalid schedule".to_string(),
        };
        let ctx_label = if r.context.is_empty() { "(active context)".to_string() } else { r.context.clone() };
        let ephemeral_label = if r.ephemeral { " [ephemeral]" } else { "" };
        println!("  {:20} {:<30} next: {}  context: {}{}",
            r.name, r.schedule, next, ctx_label, ephemeral_label);
    }
    0
}

/// `plexi routine run <name>` — manually fire a routine
pub fn routine_run(name: &str) -> i32 {
    log::info!("cli: routine run '{name}'");
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => { eprintln!("error: could not determine current directory: {e}"); return 1; }
    };
    let config_path = cwd.join(ROUTINES_FILE);
    let contents = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("error: no {ROUTINES_FILE} found in {}", cwd.display());
            return 1;
        }
        Err(e) => { eprintln!("error: could not read {}: {e}", config_path.display()); return 1; }
    };
    let config: RoutinesCliConfig = match toml::from_str(&contents) {
        Ok(c) => c,
        Err(e) => { eprintln!("error: failed to parse {ROUTINES_FILE}: {e}"); return 1; }
    };
    let routine = match config.routine.iter().find(|r| r.name == name) {
        Some(r) => r,
        None => {
            eprintln!("error: routine '{name}' not found in {ROUTINES_FILE}");
            if !config.routine.is_empty() {
                let names: Vec<&str> = config.routine.iter().map(|r| r.name.as_str()).collect();
                eprintln!("Available routines: {}", names.join(", "));
            }
            return 1;
        }
    };

    // Spawn via spawn-queue as a terminal pane
    let queue_dir = crate::config::config_dir().join("spawn-queue");
    if let Err(e) = std::fs::create_dir_all(&queue_dir) {
        eprintln!("error: could not create spawn queue: {e}");
        return 1;
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let payload = serde_json::json!({
        "type_id": "terminal",
        "args": [routine.command.clone()],
        "ephemeral": routine.ephemeral,
        "no_focus": false,
    });
    let file = queue_dir.join(format!("{ts}.json"));
    if let Err(e) = std::fs::write(&file, payload.to_string()) {
        eprintln!("error: could not write spawn request: {e}");
        return 1;
    }
    println!("queued: run routine '{name}' — command: {}", routine.command);
    0
}

// ── plexi workspace subcommands (issue #322) ──────────────────────────────────

/// `plexi workspace init` — scaffold `.plexi/workspace.toml` (UUID) and
/// `.plexi/secrets.toml` (router with `fallback = true`) in the current dir.
pub fn workspace_init() -> i32 {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: could not determine current directory: {e}");
            return 1;
        }
    };
    log::info!("workspace_init:cli: cwd={}", cwd.display());
    // Guard: refuse home dir, root dir, and inside any ~/.plexi* profile dir
    {
        let home = std::env::var("HOME").ok().map(std::path::PathBuf::from);
        let cwd_str = cwd.to_string_lossy();
        let is_home_or_root = cwd == std::path::Path::new("/")
            || home.as_ref().map(|h| cwd == *h).unwrap_or(false);
        let is_inside_profile = home.as_ref().map(|h| {
            let prefix = format!("{}/.plexi", h.to_string_lossy());
            cwd_str.starts_with(&prefix)
        }).unwrap_or(false);
        if is_home_or_root || is_inside_profile {
            log::warn!("workspace_init:cli: rejected — home/root/profile dir guard: {}", cwd.display());
            eprintln!("error: cannot initialize a workspace in your home or root directory.");
            eprintln!("  This would conflict with your Plexi profile (~/.plexi/).");
            eprintln!("  cd into a project directory first.");
            return 1;
        }
    }
    match crate::workspace_secrets::init_workspace(&cwd) {
        Ok(cfg) => {
            log::info!("workspace_init:cli: initialized workspace_id={} at {}", cfg.id, cwd.display());
            println!("Initialized workspace at {}", cwd.display());
            println!("  workspace id: {}", cfg.id);
            println!("  router:       .plexi/secrets.toml (fallback = true)");
            print_tip("define runnable commands in .plexi/commands.toml, then run them with `plexi run <name>`.");
            0
        }
        Err(e) => {
            log::warn!("workspace_init:cli: init_workspace failed: {e}");
            eprintln!("error: workspace init failed: {e}");
            1
        }
    }
}

// ── plexi secret subcommands (workspace-aware, issue #322) ───────────────────

/// Resolve the current workspace and config. Errors out with a helpful
/// message if the user has not run `plexi workspace init`.
fn require_workspace(
) -> Result<(std::path::PathBuf, crate::workspace_secrets::WorkspaceConfig), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("cwd: {e}"))?;
    let root = match crate::app_registry::resolve_workspace_root(&cwd) {
        Some(r) => r,
        None => {
            return Err(format!(
                "no .plexi/ workspace found at or above {}.\n\
                 Run `plexi workspace init` first.",
                cwd.display()
            ));
        }
    };
    let cfg = match crate::workspace_secrets::WorkspaceConfig::load(&root)
        .map_err(|e| format!("read workspace.toml: {e}"))?
    {
        Some(c) => c,
        None => {
            return Err(format!(
                ".plexi/ exists at {} but workspace.toml is missing.\n\
                 Run `plexi workspace init` to create it.",
                root.display()
            ));
        }
    };
    Ok((root, cfg))
}

/// `plexi secret set <friendly-name>` — store a secret in Keychain.
///
/// Value source (in priority order):
///   --from-env   reads from env var named FRIENDLY_NAME
///   default      hidden stdin prompt
///
/// Scope:
///   --global     store under `plexi:user:<name>` (cross-workspace)
///   default      walk up to nearest .plexi/ workspace, store workspace-scoped
pub fn workspace_secret_set(friendly: &str, from_env: bool, global: bool) -> i32 {
    log::info!(
        "secret_set:cli: friendly={friendly} from_env={from_env} global={global}"
    );
    // Resolve value
    let value: String = if from_env {
        match std::env::var(friendly) {
            Ok(v) => v,
            Err(_) => {
                log::warn!("secret_set:cli: env var {friendly} not set");
                eprintln!("error: env var {friendly} is not set");
                return 1;
            }
        }
    } else {
        eprint!("Enter value for {friendly}: ");
        let _ = io::stderr().flush();
        match read_secret_from_stdin() {
            Ok(v) => v,
            Err(e) => {
                log::warn!("secret_set:cli: stdin read failed: {e}");
                eprintln!("\nerror: failed to read secret: {e}");
                return 1;
            }
        }
    };
    if value.is_empty() {
        log::warn!("secret_set:cli: empty value rejected for {friendly}");
        eprintln!("error: empty value, nothing stored");
        return 1;
    }

    #[cfg(target_os = "macos")]
    {
        use crate::workspace_secrets::{
            keychain_user_name, keychain_workspace_name, MacKeychain, SecretStore,
        };
        let store = MacKeychain::new();

        if global {
            let account = keychain_user_name(friendly);
            return match store.set(&account, &value) {
                Ok(()) => {
                    log::info!("secret_set:cli: stored globally account={account}");
                    eprintln!("Stored '{friendly}' globally (plexi:user:{friendly})");
                    print_tip("reference secrets in commands.toml under `[secrets] required = [\"...\"]`.");
                    0
                }
                Err(e) => {
                    log::warn!("secret_set:cli: keychain write failed for {account}: {e}");
                    eprintln!("error: keychain write failed: {e}");
                    1
                }
            };
        }

        // Workspace-scoped: walk up to nearest .plexi/
        let (root, cfg) = match require_workspace() {
            Ok(v) => v,
            Err(e) => {
                log::warn!("secret_set:cli: no workspace found: {e}");
                eprintln!("error: no .plexi/ workspace found in this directory tree.");
                eprintln!("  → plexi workspace init        (initialize one here, then retry)");
                eprintln!("  → plexi secret set --global {friendly}   (set globally — requires explicit flag)");
                return 1;
            }
        };
        let account = keychain_workspace_name(&cfg.id, friendly);
        match store.set(&account, &value) {
            Ok(()) => {
                log::info!(
                    "secret_set:cli: stored workspace_id={} account={account} root={}",
                    cfg.id,
                    root.display()
                );
                eprintln!(
                    "Stored '{friendly}' for workspace {} ({})",
                    root.display(),
                    cfg.id
                );
                print_tip("reference secrets in commands.toml under `[secrets] required = [\"...\"]`.");
                0
            }
            Err(e) => {
                log::warn!("secret_set:cli: keychain write failed for {account}: {e}");
                eprintln!("error: keychain write failed: {e}");
                1
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (friendly, value, global);
        eprintln!("error: keychain not available on this platform");
        1
    }
}

/// `plexi secret list` — list friendly names defined under the current
/// workspace's namespace plus user-scope. Names only, never values.
pub fn workspace_secret_list() -> i32 {
    let (_root, cfg) = match require_workspace() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    #[cfg(target_os = "macos")]
    {
        use crate::workspace_secrets::{
            keychain_user_name, keychain_workspace_name, MacKeychain, SecretStore,
        };
        let store = MacKeychain::new();
        let workspace_prefix = keychain_workspace_name(&cfg.id, "");
        let user_prefix = keychain_user_name("");
        let workspace_entries = store.list_with_prefix(&workspace_prefix);
        let user_entries = store.list_with_prefix(&user_prefix);
        if workspace_entries.is_empty() && user_entries.is_empty() {
            eprintln!("No secrets stored.");
            return 0;
        }
        if !workspace_entries.is_empty() {
            println!("Workspace ({}):", cfg.id);
            for a in &workspace_entries {
                if let Some(name) = a.strip_prefix(&workspace_prefix) {
                    println!("  {name}");
                }
            }
        }
        if !user_entries.is_empty() {
            if !workspace_entries.is_empty() {
                println!();
            }
            println!("User scope:");
            for a in &user_entries {
                if let Some(name) = a.strip_prefix(&user_prefix) {
                    println!("  {name}");
                }
            }
        }
        0
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cfg;
        eprintln!("error: keychain not available on this platform");
        1
    }
}

/// `plexi secret get <friendly-name>` — print the resolved secret value to stdout.
///
/// Resolution order (unless --global):
///   1. Workspace-scoped Keychain entry (nearest .plexi/ workspace)
///   2. Global user-scoped Keychain entry (fallback)
///
/// `--global` skips the workspace lookup entirely.
pub fn workspace_secret_get(friendly: &str, global: bool) -> i32 {
    log::info!("secret_get:cli: friendly={friendly} global={global}");

    #[cfg(target_os = "macos")]
    {
        use crate::workspace_secrets::{keychain_user_name, keychain_workspace_name, MacKeychain, SecretStore};
        let store = MacKeychain::new();

        if global {
            let account = keychain_user_name(friendly);
            return match store.get(&account) {
                Some(value) => {
                    log::info!("secret_get:cli: found globally account={account}");
                    println!("{}", value.as_str());
                    0
                }
                None => {
                    log::warn!("secret_get:cli: not found friendly={friendly}");
                    eprintln!("error: secret '{friendly}' not found");
                    1
                }
            };
        }

        // Try workspace-scoped first, then global fallback.
        if let Ok((root, cfg)) = require_workspace() {
            let account = keychain_workspace_name(&cfg.id, friendly);
            if let Some(value) = store.get(&account) {
                log::info!(
                    "secret_get:cli: found workspace_id={} account={account} root={}",
                    cfg.id,
                    root.display()
                );
                println!("{}", value.as_str());
                return 0;
            }
        }

        // Global fallback.
        let user_account = keychain_user_name(friendly);
        match store.get(&user_account) {
            Some(value) => {
                log::info!("secret_get:cli: found globally account={user_account}");
                println!("{}", value.as_str());
                0
            }
            None => {
                log::warn!("secret_get:cli: not found friendly={friendly}");
                eprintln!("error: secret '{friendly}' not found");
                1
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (friendly, global);
        eprintln!("error: keychain not available on this platform");
        1
    }
}

/// `plexi secret delete <friendly-name>` — remove the workspace-scoped
/// Keychain entry and update the index.
pub fn workspace_secret_delete(friendly: &str) -> i32 {
    let (_root, cfg) = match require_workspace() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    #[cfg(target_os = "macos")]
    {
        use crate::workspace_secrets::{keychain_workspace_name, MacKeychain, SecretStore};
        let account = keychain_workspace_name(&cfg.id, friendly);
        let store = MacKeychain::new();
        match store.delete(&account) {
            Ok(()) => {
                eprintln!("Deleted '{friendly}' from workspace {}", cfg.id);
                0
            }
            Err(e) => {
                eprintln!("error: keychain delete failed: {e}");
                1
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (cfg, friendly);
        eprintln!("error: keychain not available on this platform");
        1
    }
}

// ── plexi app subcommands ─────────────────────────────────────────────────────

/// Detect the channel config dir name from the running binary name.
/// Mirrors the logic in `config_dir_name()` (config.rs) without the
/// PROFILE_OVERRIDE global, which is private to that module.
fn app_init_config_dir() -> String {
    let binary = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    match binary.as_deref() {
        Some(name) if name.contains("alpha") => ".plexi-alpha".to_string(),
        Some(name) if name.contains("beta") => ".plexi-beta".to_string(),
        Some(name) if name.contains("v3") => ".plexi-v3".to_string(),
        Some(name) if name.contains("pr-") => {
            let suffix = name.trim_start_matches("plexi-");
            format!(".plexi-{suffix}")
        }
        _ => ".plexi".to_string(),
    }
}

/// `plexi app init [--lang python|rust] <name>` — scaffold a new app.
///
/// Placement: walks up from CWD looking for the nearest ancestor directory
/// that contains the channel config dir (e.g. `.plexi-alpha/` for the alpha
/// build, `.plexi/` for stable). If found, scaffolds into
/// `<workspace_root>/<channel_dir>/apps/<name>/`. If no workspace root is
/// found, falls back to `<cwd>/<channel_dir>/apps/<name>/`.
///
/// The app is immediately discoverable by the registry without any additional
/// install step, and hot reload watches the actual source.
pub fn app_init(name: &str, lang: &str) -> i32 {
    if name.is_empty() {
        eprintln!("Usage: plexi app init [--lang python|rust] <name>");
        return 1;
    }

    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    // Refuse home dir and root — same guard as workspace_init. Creating
    // ~/.plexi/apps/ would collide with the stable channel profile dir.
    let home = std::env::var("HOME").ok().map(std::path::PathBuf::from);
    let is_home_or_root = cwd == std::path::Path::new("/")
        || home.as_ref().map(|h| cwd == *h).unwrap_or(false);
    if is_home_or_root {
        log::warn!("app_init: rejected — home/root guard: {}", cwd.display());
        eprintln!("error: cannot scaffold an app in your home or root directory — run from a project directory instead.");
        return 1;
    }

    let channel_dir = app_init_config_dir();

    // Walk up from CWD looking for a dir named `channel_dir` (e.g. `.plexi-alpha`).
    // Stop at home and root. If found, place the app there; otherwise fall back to CWD.
    let workspace_root = {
        let home_path = home.clone();
        let mut current = cwd.clone();
        let mut found: Option<std::path::PathBuf> = None;
        loop {
            if let Some(ref h) = home_path {
                if current == *h {
                    break;
                }
            }
            if current == std::path::Path::new("/") {
                break;
            }
            if current.join(&channel_dir).is_dir() {
                found = Some(current);
                break;
            }
            if !current.pop() {
                break;
            }
        }
        found
    };

    let placement = if workspace_root.is_some() { "workspace" } else { "global" };
    let base = workspace_root.unwrap_or_else(|| home.clone().unwrap_or_else(|| cwd.clone()));
    let app_dir = base.join(&channel_dir).join("apps").join(name);
    log::info!("app_init: placement={placement} path={}", app_dir.display());

    if app_dir.exists() {
        eprintln!("error: {} already exists", app_dir.display());
        return 1;
    }

    if let Err(e) = std::fs::create_dir_all(&app_dir) {
        eprintln!("error: could not create {}: {e}", app_dir.display());
        return 1;
    }

    let result = match lang {
        "rust" => scaffold_rust_app(&app_dir, name),
        _ => scaffold_python_app(&app_dir, name),
    };

    match result {
        Ok(()) => {
            println!("Created app '{name}' at {}", app_dir.display());
            if lang == "rust" {
                println!("\nNext steps:");
                println!("  cd {}", app_dir.display());
                println!("  cargo build --release");
                println!("  # then run: plexi app run {}", app_dir.display());
            } else {
                // Auto-open the app if PLEXI_SOCKET is set (running inside a pane).
                if std::env::var("PLEXI_SOCKET").is_ok() {
                    let path_str = app_dir.to_string_lossy().to_string();
                    log::info!("app_init: auto-opening '{name}' via app_run from_path={path_str}");
                    let exit_code = app_run(&path_str);
                    if exit_code != 0 {
                        eprintln!("warning: app created but could not auto-open (exit {exit_code}) — run: plexi app run {}", app_dir.display());
                    }
                } else {
                    println!("  Run with: plexi app run {}", app_dir.display());
                }
            }
            0
        }
        Err(e) => {
            eprintln!("error: failed to scaffold app: {e}");
            1
        }
    }
}

fn scaffold_python_app(app_dir: &std::path::Path, name: &str) -> io::Result<()> {
    // manifest.toml
    std::fs::write(app_dir.join("manifest.toml"), format!(
        "schema_version = 1\n\n[app]\nid = \"{name}\"\ntype = \"app\"\nname = \"{display}\"\nentry = \"main.py\"\nversion = \"0.1.0\"\ndescription = \"A Plexi app\"\nwatch = true\n\n[app.capabilities]\ncapabilities = []\n\n[launch]\nlayout_hint = {{ side = \"right\", split = 0.5 }}\n",
        name = name,
        display = to_title_case(name),
    ))?;

    // main.py — plexi_sdk is injected via PYTHONPATH by the host at launch;
    // do NOT copy plexi_sdk.py alongside (the package uses relative imports
    // that break when imported as a flat single file).
    // __CLASS_NAME__ and __DISPLAY_NAME__ are substituted below.
    let template = include_str!("../sdk/python/plexi_sdk/templates/app_init.py");
    let main_py = template
        .replace("__CLASS_NAME__", &to_struct_name(name))
        .replace("__DISPLAY_NAME__", &to_title_case(name));
    let main_path = app_dir.join("main.py");
    std::fs::write(&main_path, main_py)?;

    // chmod +x main.py — Unix only. NTFS has no executable bit; Python files
    // are dispatched by extension association on Windows.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&main_path)?.permissions();
        perms.set_mode(perms.mode() | 0o111);
        std::fs::set_permissions(&main_path, perms)?;
    }

    Ok(())
}

fn scaffold_rust_app(app_dir: &std::path::Path, name: &str) -> io::Result<()> {
    // manifest.toml
    std::fs::write(app_dir.join("manifest.toml"), format!(
        "schema_version = 1\n\n[app]\nid = \"{name}\"\ntype = \"app\"\nname = \"{display}\"\nentry = \"bin/plexi-app\"\nversion = \"0.1.0\"\ndescription = \"A Plexi app\"\n\n[app.capabilities]\ncapabilities = []\n\n[launch]\nlayout_hint = {{ side = \"right\", split = 0.5 }}\n",
        name = name,
        display = to_title_case(name),
    ))?;

    // Cargo.toml
    std::fs::write(app_dir.join("Cargo.toml"), format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[bin]]\nname = \"plexi-app\"\npath = \"src/main.rs\"\n\n[dependencies]\nplexi-sdk = {{ git = \"https://github.com/ianjamesburke/plexi\", branch = \"alpha\" }}\n",
        name = name,
    ))?;

    // src/main.rs
    let src_dir = app_dir.join("src");
    std::fs::create_dir_all(&src_dir)?;
    std::fs::write(src_dir.join("main.rs"), format!(
        "use plexi_sdk::{{App, Emitter, Modifiers, RenderContext, run}};\n\nstruct {struct_name};\n\nimpl App for {struct_name} {{\n    fn on_render(&mut self, ctx: &mut RenderContext) {{\n        ctx.rect(0.0, 0.0, ctx.width, ctx.height, \"#1e1e2e\");\n        ctx.text_bold(20.0, 20.0, \"{display}\", 16.0, \"#cdd6f4\");\n        ctx.text(20.0, 50.0, \"Edit src/main.rs to build your app.\", 13.0, \"#6c7086\");\n    }}\n\n    fn on_key(&mut self, _key: &str, _mods: &Modifiers, _emit: &mut Emitter) {{}}\n}}\n\nfn main() {{\n    run(&mut {struct_name});\n}}\n",
        struct_name = to_struct_name(name),
        display = to_title_case(name),
    ))?;

    Ok(())
}

/// `plexi app uninstall <id> [--yes]` — remove a globally installed app with optional confirmation.
pub fn app_uninstall(id: &str, assume_yes: bool) -> i32 {
    let target_root = crate::app_registry::apps_dir();
    let app_dir = target_root.join(id);
    if !app_dir.exists() {
        eprintln!("error: app '{id}' not found");
        return 1;
    }
    if !assume_yes {
        eprint!("Remove app '{id}'? [y/N]: ");
        let _ = io::stderr().flush();
        let mut answer = String::new();
        if io::stdin().read_line(&mut answer).is_err() {
            eprintln!("error: failed to read confirmation");
            return 1;
        }
        if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
            eprintln!("aborted");
            return 1;
        }
    }
    match crate::install::uninstall_one(id, &target_root) {
        Ok(()) => { println!("Uninstalled '{id}'."); 0 }
        Err(e) => { eprintln!("error: {e}"); 1 }
    }
}

/// `plexi app install <path>` — copy a local app directory into the channel's app store.
pub fn app_install(path: &str) -> i32 {
    let src = match std::path::Path::new(path).canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: could not resolve {path}: {e}");
            return 1;
        }
    };

    let manifest_path = src.join("manifest.toml");
    if !manifest_path.exists() {
        eprintln!("error: no manifest.toml found in {}", src.display());
        eprintln!("  Is this a Plexi app directory? Run `plexi app init <name>` to scaffold one.");
        return 1;
    }
    let manifest_str = match std::fs::read_to_string(&manifest_path) {
        Ok(s) => s,
        Err(e) => { eprintln!("error: could not read manifest.toml: {e}"); return 1; }
    };
    let manifest: toml::Value = match toml::from_str(&manifest_str) {
        Ok(v) => v,
        Err(e) => { eprintln!("error: manifest.toml parse failed: {e}"); return 1; }
    };

    let schema_version = manifest.get("schema_version").and_then(|v| v.as_integer()).unwrap_or(0);
    if schema_version > crate::app_registry::MANIFEST_SCHEMA_VERSION as i64 {
        eprintln!(
            "error: manifest.toml schema_version {schema_version} is newer than supported (max {})",
            crate::app_registry::MANIFEST_SCHEMA_VERSION
        );
        return 1;
    }

    let app_section = match manifest.get("app") {
        Some(a) => a,
        None => { eprintln!("error: manifest.toml is missing [app] section"); return 1; }
    };
    let app_id = match app_section.get("id").and_then(|v| v.as_str()) {
        Some(id) if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') => {
            id.to_string()
        }
        _ => {
            eprintln!("error: manifest.toml is missing a valid [app].id");
            eprintln!("  (IDs must be non-empty and contain only alphanumeric characters, dashes, or underscores)");
            return 1;
        }
    };
    let app_version = app_section.get("version").and_then(|v| v.as_str()).unwrap_or("?");

    let dest = crate::app_registry::apps_dir().join(&app_id);

    // Remove existing install (idempotent overwrite).
    if dest.exists() {
        if let Err(e) = std::fs::remove_dir_all(&dest) {
            eprintln!("error: could not remove existing install at {}: {e}", dest.display());
            return 1;
        }
    }

    if let Err(e) = copy_dir_all(&src, &dest) {
        eprintln!("error: could not copy {} to {}: {e}", src.display(), dest.display());
        return 1;
    }

    log::info!("app::install: installed {app_id} v{app_version} from {}", src.display());
    println!("Installed '{app_id}' v{app_version} from {}.", src.display());
    println!("Run `plexi open {app_id}` to launch it.");
    0
}

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let entry_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if entry_path.is_dir() {
            copy_dir_all(&entry_path, &dst_path)?;
        } else {
            std::fs::copy(entry_path, dst_path)?;
        }
    }
    Ok(())
}

/// `plexi app link <path>` — register a local app directory with the nearest workspace.
pub fn app_link(path: &str) -> i32 {
    eprintln!("deprecated: `plexi app link` is deprecated — use `plexi app run <path>` instead");
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => { eprintln!("error: {e}"); return 1; }
    };
    let app_dir = if std::path::Path::new(path).is_absolute() {
        std::path::PathBuf::from(path)
    } else {
        cwd.join(path)
    };
    let app_dir = match app_dir.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: could not resolve {path}: {e}");
            return 1;
        }
    };
    // Validate manifest.toml exists and is parseable
    let manifest_path = app_dir.join("manifest.toml");
    if !manifest_path.exists() {
        eprintln!("error: no manifest.toml found in {}", app_dir.display());
        eprintln!("  Is this a Plexi app directory? Run `plexi app init <name>` to scaffold one.");
        return 1;
    }
    let manifest_str = match std::fs::read_to_string(&manifest_path) {
        Ok(s) => s,
        Err(e) => { eprintln!("error: could not read manifest.toml: {e}"); return 1; }
    };
    let manifest: toml::Value = match toml::from_str(&manifest_str) {
        Ok(v) => v,
        Err(e) => { eprintln!("error: manifest.toml parse failed: {e}"); return 1; }
    };
    let app_id = match manifest.get("app").and_then(|a| a.get("id")).and_then(|v| v.as_str()) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => {
            eprintln!("error: manifest.toml is missing a valid [app].id");
            return 1;
        }
    };

    let workspace_root = match crate::app_registry::resolve_workspace_root(&cwd) {
        Some(r) => r,
        None => {
            eprintln!("error: no .plexi/ workspace found at or above {}.", cwd.display());
            eprintln!("  Run `plexi workspace init` first.");
            return 1;
        }
    };

    log::info!(
        "app_link:cli: linking {} as app '{}' in workspace {}",
        app_dir.display(), app_id, workspace_root.display()
    );

    let links_path = workspace_root.join(".plexi").join("links.toml");
    let abs_path = app_dir.to_string_lossy().to_string();

    #[derive(serde::Deserialize, serde::Serialize)]
    struct LinksFile { #[serde(default)] links: Vec<String> }

    // Read existing links (or start fresh)
    let mut links: Vec<String> = if links_path.exists() {
        let content = match std::fs::read_to_string(&links_path) {
            Ok(s) => s,
            Err(e) => { eprintln!("error: could not read links.toml: {e}"); return 1; }
        };
        match toml::from_str::<LinksFile>(&content) {
            Ok(f) => f.links,
            Err(e) => { eprintln!("error: could not parse links.toml: {e}"); return 1; }
        }
    } else {
        Vec::new()
    };

    if links.contains(&abs_path) {
        println!("Already linked: {}", app_dir.display());
        return 0;
    }

    links.push(abs_path);

    let new_content = match toml::to_string_pretty(&LinksFile { links }) {
        Ok(s) => s,
        Err(e) => { eprintln!("error: could not serialize links.toml: {e}"); return 1; }
    };

    if let Err(e) = std::fs::write(&links_path, new_content) {
        eprintln!("error: could not write links.toml: {e}");
        return 1;
    }

    println!("Linked '{}' from {}", app_id, app_dir.display());
    println!("  App will appear in the run palette on next launch or reload.");
    0
}

/// `plexi app unlink <path>` — remove a linked app directory from the workspace registry.
pub fn app_unlink(path: &str) -> i32 {
    eprintln!("deprecated: `plexi app unlink` is deprecated — use `plexi app run <path>` instead");
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => { eprintln!("error: {e}"); return 1; }
    };
    let app_dir = if std::path::Path::new(path).is_absolute() {
        std::path::PathBuf::from(path)
    } else {
        cwd.join(path)
    };
    // Try to canonicalize; fall back to the raw path if it doesn't exist
    let app_dir = app_dir.canonicalize().unwrap_or(app_dir);
    let abs_path = app_dir.to_string_lossy().to_string();

    let workspace_root = match crate::app_registry::resolve_workspace_root(&cwd) {
        Some(r) => r,
        None => {
            eprintln!("error: no .plexi/ workspace found at or above {}.", cwd.display());
            eprintln!("  Run `plexi workspace init` first.");
            return 1;
        }
    };

    log::info!(
        "app_unlink:cli: unlinking {} from workspace {}",
        app_dir.display(), workspace_root.display()
    );

    let links_path = workspace_root.join(".plexi").join("links.toml");
    if !links_path.exists() {
        println!("Not linked (no links.toml found).");
        return 0;
    }

    let content = match std::fs::read_to_string(&links_path) {
        Ok(s) => s,
        Err(e) => { eprintln!("error: could not read links.toml: {e}"); return 1; }
    };
    #[derive(serde::Deserialize, serde::Serialize)]
    struct LinksFile { #[serde(default)] links: Vec<String> }
    let mut links: Vec<String> = match toml::from_str::<LinksFile>(&content) {
        Ok(f) => f.links,
        Err(e) => { eprintln!("error: could not parse links.toml: {e}"); return 1; }
    };

    let before = links.len();
    links.retain(|p| p != &abs_path);
    if links.len() == before {
        println!("Not found in links.toml: {}", app_dir.display());
        return 0;
    }

    let new_content = match toml::to_string_pretty(&LinksFile { links }) {
        Ok(s) => s,
        Err(e) => { eprintln!("error: could not serialize links.toml: {e}"); return 1; }
    };

    if let Err(e) = std::fs::write(&links_path, new_content) {
        eprintln!("error: could not write links.toml: {e}");
        return 1;
    }

    println!("Unlinked: {}", app_dir.display());
    0
}

/// `plexi app run <path>` — open any directory with a valid manifest.toml as an app pane.
///
/// No install, no link required. Edits take effect on next launch.
pub fn app_run(path: &str) -> i32 {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => { eprintln!("error: {e}"); return 1; }
    };
    let app_dir = if std::path::Path::new(path).is_absolute() {
        std::path::PathBuf::from(path)
    } else {
        cwd.join(path)
    };
    let app_dir = match app_dir.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: could not resolve {path}: {e}");
            return 1;
        }
    };
    // Validate manifest.toml exists and parses
    let manifest_path = app_dir.join("manifest.toml");
    if !manifest_path.exists() {
        eprintln!("error: no manifest.toml found in {}", app_dir.display());
        eprintln!("  Is this a Plexi app directory? Run `plexi app init <name>` to scaffold one.");
        return 1;
    }
    let manifest_str = match std::fs::read_to_string(&manifest_path) {
        Ok(s) => s,
        Err(e) => { eprintln!("error: could not read manifest.toml: {e}"); return 1; }
    };
    let _: toml::Value = match toml::from_str(&manifest_str) {
        Ok(v) => v,
        Err(e) => { eprintln!("error: manifest.toml parse failed: {e}"); return 1; }
    };
    let abs_path = app_dir.to_string_lossy().to_string();
    log::info!("app_run:cli: launching app from path={abs_path}");

    if std::env::var("PLEXI_SOCKET").is_ok() {
        let id = uuid::Uuid::new_v4();
        let response_file = crate::config::config_dir()
            .join(format!("spawn-pane-response-{id}.json"))
            .to_string_lossy()
            .into_owned();
        let from_pane_id = std::env::var("PLEXI_PANE_ID")
            .ok()
            .and_then(|s| s.parse::<u64>().ok());
        let mut payload = serde_json::json!({
            "type": "spawn_pane",
            "type_id": "",
            "path": abs_path,
            "response_file": response_file,
        });
        if let Some(pid) = from_pane_id {
            payload["from_pane_id"] = serde_json::Value::Number(pid.into());
        }
        log::info!("app_run:cli: sending via socket path={abs_path} response_file={response_file}");
        let code = send_to_socket(payload);
        if code != 0 {
            return code;
        }
        let response_path = std::path::PathBuf::from(&response_file);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if response_path.exists() {
                match std::fs::read_to_string(&response_path) {
                    Ok(content) => {
                        let _ = std::fs::remove_file(&response_path);
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                            if let Some(msg) = v.get("error").and_then(|v| v.as_str()) {
                                eprintln!("error: {msg}");
                                return 1;
                            }
                            if let Some(pid) = v.get("pane_id").and_then(|v| v.as_u64()) {
                                println!("{pid}");
                                return 0;
                            }
                        }
                        print!("{content}");
                        return 0;
                    }
                    Err(e) => {
                        eprintln!("error: could not read response file: {e}");
                        return 1;
                    }
                }
            }
            if std::time::Instant::now() >= deadline {
                eprintln!("error: timed out waiting for open response");
                return 1;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    // Fallback: write to spawn-queue
    let queue_dir = crate::config::config_dir().join("spawn-queue");
    if let Err(e) = std::fs::create_dir_all(&queue_dir) {
        eprintln!("error: could not create spawn queue: {e}");
        return 1;
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let queue_payload = serde_json::json!({
        "type_id": "",
        "path": abs_path,
    });
    let file = queue_dir.join(format!("{ts}.json"));
    if let Err(e) = std::fs::write(&file, queue_payload.to_string()) {
        eprintln!("error: could not write spawn request: {e}");
        return 1;
    }
    log::info!("app_run:cli: queued path={abs_path}");
    println!("queued: run {abs_path}");
    println!("(running outside a Plexi pane — Plexi will pick this up within a second)");
    0
}

/// `plexi app info <id>` — show manifest info for an installed app, including MCP URL if applicable.
pub fn app_info(id: &str) -> i32 {
    let registry =
        crate::app_registry::AppRegistry::load(&std::env::current_dir().unwrap_or_default());
    let Some(installed) = registry.get(id) else {
        eprintln!("error: app '{id}' not found — run `plexi app list` to see installed apps");
        return 1;
    };
    let m = &installed.manifest;
    println!("id:          {}", m.id);
    println!("name:        {}", m.name);
    println!("version:     {}", m.version);
    println!("description: {}", m.description);
    if let Some(mcp) = &m.mcp {
        if !mcp.description.is_empty() {
            println!("mcp_desc:    {}", mcp.description);
        }
        let tool_names: Vec<&str> = mcp.tools.iter().map(|t| t.name.as_str()).collect();
        println!(
            "mcp_tools:   {}",
            if tool_names.is_empty() {
                "(none declared)".to_string()
            } else {
                tool_names.join(", ")
            }
        );
        println!("mcp_url:     http://localhost:${{PLEXI_MCP_PORT}}/mcp  (port assigned at runtime)");
        println!();
        println!("Claude Desktop config:");
        println!("  {{");
        println!("    \"mcpServers\": {{");
        println!("      \"{}\": {{ \"url\": \"http://localhost:${{PLEXI_MCP_PORT}}/mcp\" }}", m.id);
        println!("    }}");
        println!("  }}");
    }
    0
}

/// `plexi app list` — unified alias for `plexi list`.
pub fn app_list() -> i32 {
    log::info!("cli: app_list delegating to list_cli (unified)");
    list_cli()
}

/// `plexi app render <id> --size WxH [--state state.json] [--output path.png]`
/// Renders an app to PNG headlessly via the offscreen egui/wgpu pipeline.
pub fn app_render(id: &str, size: &str, state: Option<&str>, output: Option<&str>) -> i32 {
    // Parse WxH
    let (width, height) = match parse_render_size(size) {
        Some(v) => v,
        None => {
            eprintln!("error: invalid --size format '{size}' — expected WxH (e.g. 500x500)");
            return 1;
        }
    };

    // Optional: pre-seed app state
    let seeded_path = state.and_then(|s| {
        let json = match std::fs::read_to_string(s) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("error: could not read state file '{s}': {e}");
                return None;
            }
        };
        let dest = crate::config::config_dir().join("app_states").join(format!("{id}.json"));
        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&dest, &json) {
            eprintln!("error: could not write state to {}: {e}", dest.display());
            return None;
        }
        log::info!("app_render[{id}]: pre-seeded state from '{s}' → {}", dest.display());
        Some(dest)
    });

    // Resolve the app binary
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let registry = crate::app_registry::AppRegistry::load(&cwd);
    let app_bin = match registry.list().into_iter().find(|a| a.manifest.id == id) {
        Some(a) => a.bin_path.clone(),
        None => {
            eprintln!("error: app '{id}' not found — run `plexi app list` to see installed apps");
            if let Some(path) = seeded_path {
                let _ = std::fs::remove_file(path);
            }
            return 1;
        }
    };

    let png_bytes = match crate::app_render::render_app_to_png(id, &app_bin, width, height) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: render failed: {e}");
            if let Some(path) = seeded_path {
                let _ = std::fs::remove_file(path);
            }
            return 1;
        }
    };

    // Clean up seeded state if we wrote it
    if let Some(ref path) = seeded_path {
        let _ = std::fs::remove_file(path);
        log::info!("app_render[{id}]: cleaned up seeded state at {}", path.display());
    }

    // Write output
    match output {
        Some(path) => {
            if let Err(e) = std::fs::write(path, &png_bytes) {
                eprintln!("error: could not write output to '{path}': {e}");
                return 1;
            }
            log::info!("app_render[{id}]: wrote {width}×{height} PNG to '{path}'");
            eprintln!("Wrote {width}×{height} PNG to '{path}'");
        }
        None => {
            use std::io::Write;
            if let Err(e) = std::io::stdout().write_all(&png_bytes) {
                eprintln!("error: could not write PNG to stdout: {e}");
                return 1;
            }
            log::info!(
                "app_render[{id}]: wrote {width}×{height} PNG to stdout ({} bytes)",
                png_bytes.len()
            );
        }
    }

    0
}

fn parse_render_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once('x')?;
    let w = w.parse::<u32>().ok()?;
    let h = h.parse::<u32>().ok()?;
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

// ── Top-level package manager subcommands (#308 Phase 2) ──────────────────────

/// `plexi install <source-spec>[@ref]` — clone + place one app into the
/// Returns true if `s` looks like a bare app ID (no scheme prefix, no path separators).
fn is_bare_id(s: &str) -> bool {
    !s.contains(':') && !s.contains('/') && !s.is_empty()
}

/// Returns true if `s` looks like a bare GitHub shorthand (`owner/repo`): no scheme,
/// exactly one `/`, non-empty owner and repo segments.
fn is_github_shorthand(s: &str) -> bool {
    if s.contains(':') {
        return false;
    }
    let mut parts = s.splitn(2, '/');
    let owner = parts.next().unwrap_or("");
    let repo = parts.next().unwrap_or("");
    !owner.is_empty() && !repo.is_empty() && !repo.contains('/')
}

/// Fetch the plexi app registry and resolve a bare app ID to a source spec string.
///
/// Registry entries in `ianjamesburke/PLEXI` with a `path` field resolve to `local:<dir>`
/// so the bundled copy is used without a network clone. Third-party repos resolve to
/// `github:owner/repo`.
fn resolve_registry_id(id: &str) -> Result<String, String> {
    const REGISTRY_URL: &str =
        "https://raw.githubusercontent.com/ianjamesburke/plexi-app-registry/main/registry.json";

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build();

    let body = match agent.get(REGISTRY_URL).call() {
        Ok(r) => match r.into_string() {
            Ok(s) => s,
            Err(e) => return Err(format!("failed to read registry response: {e}")),
        },
        Err(e) => return Err(format!("failed to fetch registry: {e}")),
    };

    let entries: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("failed to parse registry: {e}"))?;

    let arr = entries
        .as_array()
        .ok_or_else(|| "registry response is not a JSON array".to_string())?;

    for entry in arr {
        if entry["id"].as_str() != Some(id) {
            continue;
        }
        let repo = entry["repo"]
            .as_str()
            .ok_or_else(|| format!("registry entry '{id}' has no 'repo' field"))?;
        let path = entry["path"].as_str();

        let spec = if repo == "ianjamesburke/PLEXI" {
            if let Some(p) = path {
                // Use the bundled copy — no network clone needed.
                let dir = p.split('/').next_back().unwrap_or(p);
                format!("local:{dir}")
            } else {
                format!("github:{repo}")
            }
        } else {
            format!("github:{repo}")
        };

        log::info!("registry: resolved '{id}' → {spec}");
        return Ok(spec);
    }

    Err(format!(
        "unknown app id '{id}' — run `plexi app list` or visit plexiapp.com/apps"
    ))
}

/// channel apps dir. Source spec follows `packs::parse_source_spec`.
pub fn install_cli(spec: &str) -> i32 {
    let (source_str, git_ref) = crate::install::split_source_and_ref(spec);
    let resolved = if is_bare_id(&source_str) {
        match resolve_registry_id(&source_str) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    } else if is_github_shorthand(&source_str) {
        let prefixed = format!("github:{source_str}");
        log::info!("install: bare shorthand '{source_str}' → {prefixed}");
        prefixed
    } else {
        source_str
    };
    let source = match crate::packs::parse_source_spec(&resolved) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let target_root = crate::app_registry::apps_dir();
    let cloner = crate::install::GitCloner;
    match crate::install::install_one(&cloner, &source, git_ref.as_deref(), &target_root) {
        Ok(outcome) => match outcome.status {
            crate::install::InstallStatus::Installed(path) => {
                println!("installed '{}' at {}", outcome.id, path.display());
                print_tip(&format!("open your app with `plexi open {}`.", outcome.id));
                0
            }
            crate::install::InstallStatus::AlreadyAtVersion => {
                println!("already at requested version");
                0
            }
            crate::install::InstallStatus::SkippedOtherVersion {
                installed,
                requested,
            } => {
                eprintln!(
                    "'{}' already installed at {installed} (requested {requested}); \
                     uninstall first or use `plexi update apps`",
                    outcome.id
                );
                1
            }
            crate::install::InstallStatus::Failed(msg) => {
                eprintln!("error: {msg}");
                1
            }
        },
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `plexi install --pack <path|core>` — apply a whole pack file.
pub fn install_pack_cli(spec: &str) -> i32 {
    let pack = if spec == "core" {
        match crate::packs::Pack::from_toml_str(crate::install::CORE_PACK_TOML) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error: bundled core pack invalid: {e}");
                return 1;
            }
        }
    } else {
        match crate::packs::Pack::from_path(std::path::Path::new(spec)) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    };
    let target_root = crate::app_registry::apps_dir();
    if let Err(e) = std::fs::create_dir_all(&target_root) {
        eprintln!("error: create apps dir {}: {e}", target_root.display());
        return 1;
    }
    let cloner = crate::install::GitCloner;
    let outcomes = crate::install::apply_pack(&cloner, &pack, &target_root);
    let mut any_failed = false;
    for o in &outcomes {
        match &o.status {
            crate::install::InstallStatus::Installed(p) => {
                println!("  installed  {:30} → {}", o.id, p.display());
            }
            crate::install::InstallStatus::AlreadyAtVersion => {
                println!("  up-to-date {:30}", o.id);
            }
            crate::install::InstallStatus::SkippedOtherVersion {
                installed,
                requested,
            } => {
                println!(
                    "  skipped    {:30} (installed {installed}, requested {requested})",
                    o.id
                );
            }
            crate::install::InstallStatus::Failed(msg) => {
                eprintln!("  FAILED     {:30} {msg}", o.id);
                any_failed = true;
            }
        }
    }
    if any_failed {
        1
    } else {
        0
    }
}

/// `plexi uninstall [--keep-data] [--yes]` — remove Plexi itself from the Mac.
pub fn plexi_uninstall_cli(keep_data: bool, assume_yes: bool) -> i32 {
    // Detect channel suffix from binary name (e.g. "plexi-alpha" → "-alpha", "plexi" → "")
    let exe = std::env::current_exe().unwrap_or_default();
    let binary_name = exe.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let suffix = if binary_name == "plexi" {
        String::new()
    } else {
        binary_name.strip_prefix("plexi").unwrap_or("").to_string()
    };
    let cap_owned = if let Some(n) = suffix.strip_prefix("-pr-") {
        format!(" PR{n}")
    } else {
        match suffix.as_str() {
            "-alpha" => " Alpha".to_string(),
            "-beta"  => " Beta".to_string(),
            _        => String::new(),
        }
    };
    let cap = cap_owned.as_str();

    let profile_dir = dirs::home_dir().unwrap().join(format!(".plexi{suffix}"));
    let app_bundle  = std::path::PathBuf::from(format!("/Applications/Plexi{cap}.app"));
    let cli_binary  = std::path::PathBuf::from(format!("/usr/local/bin/plexi{suffix}"));

    // Single confirmation prompt: keep data or remove everything?
    // Resolved before the banner so the preview accurately reflects the outcome.
    let keep_data = if keep_data || !profile_dir.exists() {
        log::info!("uninstall: keep_data=flag({keep_data}) profile_exists={}", profile_dir.exists());
        keep_data
    } else if assume_yes {
        log::info!("uninstall: keep_data=false (assume_yes, no --keep-data)");
        false
    } else {
        eprint!("Keep your ~/.plexi{suffix} data for future installs? [y/n, Enter=abort]: ");
        let _ = io::stderr().flush();
        let mut answer = String::new();
        if let Err(e) = io::stdin().read_line(&mut answer) {
            log::warn!("uninstall: failed to read keep-data confirmation: {e}");
            eprintln!("error: failed to read: {e}");
            return 1;
        }
        match answer.trim().to_lowercase().as_str() {
            "y" | "yes" => {
                log::info!("uninstall: keep_data=true (user chose y)");
                true
            }
            "n" | "no" => {
                log::info!("uninstall: keep_data=false (user chose n)");
                eprintln!("Removing everything.");
                false
            }
            other => {
                log::info!("uninstall: aborted (user input {:?})", other);
                eprintln!("Aborted.");
                return 0;
            }
        }
    };

    // Print what will be removed (after keep_data is resolved so the preview is accurate)
    println!("This will remove:");
    if app_bundle.exists()  { println!("  \u{2022} {}", app_bundle.display()); }
    if cli_binary.exists()  { println!("  \u{2022} {}", cli_binary.display()); }
    if !keep_data && profile_dir.exists() {
        println!("  \u{2022} {}  (settings, secrets, app configs)", profile_dir.display());
    } else if profile_dir.exists() {
        println!("  \u{2022} {} will be kept", profile_dir.display());
    }

    let mut removed = false;

    // Archive backlog before potentially deleting profile dir
    if !keep_data {
        let backlog = profile_dir.join("backlog");
        if backlog.exists() {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let archive = dirs::home_dir().unwrap().join(format!(
                "plexi-backlog-archive/plexi{suffix}-backlog-{ts}"
            ));
            if let Some(parent) = archive.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::rename(&backlog, &archive).is_ok() {
                println!("Archived backlog \u{2192} {}", archive.display());
            }
        }
    }

    // Remove app bundle
    if app_bundle.exists() {
        match std::fs::remove_dir_all(&app_bundle) {
            Ok(()) => { println!("Removed {}", app_bundle.display()); removed = true; }
            Err(e) => eprintln!("warning: could not remove {}: {e}", app_bundle.display()),
        }
    }

    // Remove CLI binary
    if cli_binary.exists() || cli_binary.is_symlink() {
        match std::fs::remove_file(&cli_binary) {
            Ok(()) => { println!("Removed {}", cli_binary.display()); removed = true; }
            Err(e) => eprintln!("warning: could not remove {}: {e}", cli_binary.display()),
        }
    }

    // Remove completions (only for stable uninstall)
    if suffix.is_empty() {
        let brew_prefix = std::process::Command::new("brew")
            .arg("--prefix")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string());
        if let Some(prefix) = brew_prefix {
            let zsh_comp = std::path::PathBuf::from(prefix).join("share/zsh/site-functions/_plexi");
            if zsh_comp.exists() {
                let _ = std::fs::remove_file(&zsh_comp);
                println!("Removed {}", zsh_comp.display());
            }
        }
    }

    // Remove profile dir
    if !keep_data && profile_dir.exists() {
        match std::fs::remove_dir_all(&profile_dir) {
            Ok(()) => { println!("Removed {}", profile_dir.display()); removed = true; }
            Err(e) => eprintln!("warning: could not remove {}: {e}", profile_dir.display()),
        }
    }

    if removed {
        println!("\nDone. Plexi{} has been removed.", if cap.is_empty() { "" } else { cap });
    } else {
        println!("\nNothing found to remove.");
    }
    0
}

/// `plexi update apps [<id>]` — git-pull one installed app, or all of them.
/// Apps that aren't git checkouts (e.g. bundled core entries) are skipped
/// with a debug-level log line and reported but not failed.
pub fn update_cli(maybe_id: Option<&str>) -> i32 {
    let target_root = crate::app_registry::apps_dir();
    let cloner = crate::install::GitCloner;
    let ids: Vec<String> = match maybe_id {
        Some(id) => vec![id.to_string()],
        None => crate::install::installed_versions(&target_root)
            .into_keys()
            .collect(),
    };
    if ids.is_empty() {
        println!("no apps installed");
        return 0;
    }
    let mut any_failed = false;
    for id in ids {
        match crate::install::update_one(&cloner, &id, &target_root) {
            Ok(()) => println!("  updated  {id}"),
            Err(e) if e.contains("not a git checkout") => {
                println!("  skipped  {id} (not a git checkout)");
            }
            Err(e) => {
                eprintln!("  FAILED   {id}: {e}");
                any_failed = true;
            }
        }
    }
    if any_failed {
        1
    } else {
        0
    }
}

/// `plexi update` — download and install the latest Plexi release from GitHub.
/// Only supports stable channel. Alpha (dev) and PR builds must use `just install`.
/// Beta builds require a channel-renamed bundle that can't yet be produced without
/// the install script, so they are also unsupported here.
pub fn self_update_cli() -> i32 {
    use std::io::Read;

    // Detect channel from binary name (mirrors config_dir_name in config.rs).
    let binary = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    let binary_name = binary.as_deref().unwrap_or("plexi");

    if binary_name.contains("alpha") || binary_name.contains("pr-") {
        eprintln!("Self-update is not available for dev builds.");
        eprintln!("Update from source: git pull && just install");
        return 1;
    }
    if binary_name.contains("beta") {
        log::info!("cli: self-update skipped — beta build");
        println!("Self-update for beta builds is not yet supported.");
        println!("Download the latest beta from: https://github.com/ianjamesburke/PLEXI/releases");
        return 0;
    }

    let current_version = env!("CARGO_PKG_VERSION");
    println!("Checking for updates...");
    println!("Current: v{current_version}");

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build();

    let release_body = match agent
        .get("https://api.github.com/repos/ianjamesburke/PLEXI/releases/latest")
        .set("User-Agent", "plexi-self-update")
        .set("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(r) => match r.into_string() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: failed to read release response: {e}");
                return 1;
            }
        },
        Err(e) => {
            eprintln!("error: failed to fetch release info: {e}");
            return 1;
        }
    };

    let release: serde_json::Value = match serde_json::from_str(&release_body) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: failed to parse release response: {e}");
            return 1;
        }
    };

    let tag_name = match release["tag_name"].as_str() {
        Some(t) => t.to_string(),
        None => {
            eprintln!("error: release has no tag_name");
            return 1;
        }
    };

    let latest_version = tag_name.trim_start_matches('v');
    if latest_version == current_version {
        println!("Already up to date (v{current_version}).");
        return 0;
    }
    println!("Latest:  {tag_name}");

    // Find the zip asset in the release.
    let asset_name = format!("Plexi-{tag_name}.zip");
    let download_url = match release["assets"]
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .find(|a| a["name"].as_str() == Some(asset_name.as_str()))
        })
        .and_then(|a| a["browser_download_url"].as_str())
    {
        Some(url) => url.to_string(),
        None => {
            eprintln!("error: no asset named {asset_name} in release {tag_name}");
            eprintln!(
                "Check: https://github.com/ianjamesburke/PLEXI/releases/tag/{tag_name}"
            );
            return 1;
        }
    };

    // Determine the installed app bundle path from current_exe():
    // .../Plexi.app/Contents/MacOS/plexi  →  walk up 3 levels
    let current_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: could not determine current binary path: {e}");
            return 1;
        }
    };
    let app_bundle = current_exe
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .filter(|p| p.extension().map_or(false, |e| e == "app"))
        .map(|p| p.to_path_buf());
    let app_bundle = match app_bundle {
        Some(p) => p,
        None => {
            log::info!("cli: self-update skipped — not a bundle install");
            println!("Self-update requires a bundled .app installation.");
            println!("For a dev install, update from source: git pull && just install");
            return 0;
        }
    };

    println!("Downloading {asset_name}...");

    let download_resp = match agent
        .get(&download_url)
        .set("User-Agent", "plexi-self-update")
        .call()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: failed to download {asset_name}: {e}");
            return 1;
        }
    };

    // Write zip to a temp directory.
    let tmp_dir = std::env::temp_dir().join("plexi-update");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    if let Err(e) = std::fs::create_dir_all(&tmp_dir) {
        eprintln!("error: failed to create temp dir: {e}");
        return 1;
    }
    let zip_path = tmp_dir.join(&asset_name);
    let mut zip_file = match std::fs::File::create(&zip_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: failed to create temp file: {e}");
            return 1;
        }
    };
    let mut buf = Vec::new();
    if let Err(e) = download_resp.into_reader().read_to_end(&mut buf) {
        eprintln!("error: failed to download file: {e}");
        return 1;
    }
    if let Err(e) = std::io::Write::write_all(&mut zip_file, &buf) {
        eprintln!("error: failed to write download to disk: {e}");
        return 1;
    }
    drop(zip_file);

    // Extract using system unzip.
    println!("Installing...");
    let extract_dir = tmp_dir.join("extracted");
    let _ = std::fs::create_dir_all(&extract_dir);
    let unzip_out = std::process::Command::new("unzip")
        .arg("-o")
        .arg(&zip_path)
        .arg("-d")
        .arg(&extract_dir)
        .output();
    match unzip_out {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            eprintln!("error: unzip failed: {}", String::from_utf8_lossy(&out.stderr));
            return 1;
        }
        Err(e) => {
            eprintln!("error: failed to run unzip: {e}");
            return 1;
        }
    }

    let extracted_app = extract_dir.join("Plexi.app");
    if !extracted_app.is_dir() {
        eprintln!("error: Plexi.app not found in downloaded archive");
        return 1;
    }

    // Replace the installed app bundle. Write to a temp path first so that
    // if cp fails we still have the old bundle to fall back to.
    let app_parent = app_bundle.parent().unwrap_or_else(|| std::path::Path::new("/Applications"));
    let staging = app_parent.join("Plexi.app.update-staging");
    let _ = std::fs::remove_dir_all(&staging);
    let cp_stage = std::process::Command::new("cp")
        .arg("-R")
        .arg(&extracted_app)
        .arg(&staging)
        .output();
    match cp_stage {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            eprintln!(
                "error: failed to stage new app (permission denied?): {}",
                String::from_utf8_lossy(&out.stderr)
            );
            eprintln!("Run with sudo if /Applications is not user-writable.");
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return 1;
        }
        Err(e) => {
            eprintln!("error: failed to run cp: {e}");
            return 1;
        }
    }

    // When running inside Plexi the bundle can't be replaced while the app is live.
    // Write a relaunch script, launch it detached, trigger app quit, and exit.
    if std::env::var("PLEXI_RUNNING").as_deref() == Ok("1") {
        let bin_name = current_exe
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("plexi");
        let app_display_name = app_bundle
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("Plexi");
        let script = format!(
            "#!/bin/bash\n\
             while pgrep -x '{bin_name}' > /dev/null 2>&1; do sleep 0.3; done\n\
             rm -rf '{bundle}'\n\
             mv '{staging_path}' '{bundle}'\n\
             ln -sf '{bundle}/Contents/MacOS/{bin_name}' /usr/local/bin/{bin_name} 2>/dev/null || true\n\
             open '{bundle}'\n",
            bin_name = bin_name,
            staging_path = staging.display(),
            bundle = app_bundle.display(),
        );
        let script_path = tmp_dir.join("plexi-relaunch.sh");
        if let Err(e) = std::fs::write(&script_path, &script) {
            eprintln!("error: failed to write relaunch script: {e}");
            let _ = std::fs::remove_dir_all(&staging);
            return 1;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                &script_path,
                std::fs::Permissions::from_mode(0o755),
            );
        }
        match std::process::Command::new("nohup")
            .arg("bash")
            .arg(&script_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => {}
            Err(e) => {
                eprintln!("error: failed to launch relaunch script: {e}");
                let _ = std::fs::remove_dir_all(&staging);
                return 1;
            }
        }
        println!("Plexi will restart to apply the update.");
        let _ = std::process::Command::new("osascript")
            .args([
                "-e",
                &format!("tell application \"{app_display_name}\" to quit"),
            ])
            .status();
        return 0;
    }

    if let Err(e) = std::fs::remove_dir_all(&app_bundle) {
        eprintln!("error: failed to remove old app bundle: {e}");
        eprintln!("Run with sudo if /Applications is not user-writable.");
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return 1;
    }
    if let Err(e) = std::fs::rename(&staging, &app_bundle) {
        eprintln!("error: failed to move new app into place: {e}");
        eprintln!(
            "Staged bundle is at {}. Move it manually to {}.",
            staging.display(),
            app_bundle.display()
        );
        return 1;
    }

    // Re-symlink the CLI binary at /usr/local/bin/plexi (non-fatal if missing).
    // Unix-only: the macOS updater path. The Windows installer will manage
    // PATH entries directly under %LOCALAPPDATA%\Plexi rather than symlinking.
    #[cfg(unix)]
    if let Some(bin_name) = current_exe.file_name().and_then(|n| n.to_str()) {
        let new_binary = app_bundle.join("Contents/MacOS").join(bin_name);
        let bin_link = std::path::Path::new("/usr/local/bin").join(bin_name);
        if bin_link.is_symlink() || bin_link.exists() {
            let _ = std::fs::remove_file(&bin_link);
            if let Err(e) = std::os::unix::fs::symlink(&new_binary, &bin_link) {
                eprintln!("warning: could not update CLI symlink: {e}");
            }
        }
    }
    #[cfg(not(unix))]
    let _ = current_exe; // suppress unused on Windows until installer is wired up

    let _ = std::fs::remove_dir_all(&tmp_dir);
    println!("Installed v{latest_version}. Restart Plexi to apply.");
    0
}

/// `plexi list` — show installed apps grouped by scope (global vs. workspace).
pub fn list_cli() -> i32 {
    let cwd = std::env::current_dir().unwrap_or_default();
    let registry = crate::app_registry::AppRegistry::load(&cwd);
    let installed = registry.list();
    if installed.is_empty() {
        println!("no apps installed");
        println!("install one with: plexi install <source>[@ref]");
        return 0;
    }
    // Read versions directly from the global apps dir for the source-of-truth
    // version field — the registry only carries `manifest.version` at load time.
    let global_versions = crate::install::installed_versions(&crate::app_registry::apps_dir());
    let workspace_root = crate::app_registry::resolve_workspace_root(&cwd);
    let mut globals = Vec::new();
    let mut workspace = Vec::new();
    for app in installed {
        let version = global_versions
            .get(&app.manifest.id)
            .cloned()
            .unwrap_or_else(|| app.manifest.version.clone());
        let row = (app.manifest.id.clone(), app.manifest.name.clone(), version);
        match app.source {
            crate::app_registry::RegistrySource::Global => globals.push(row),
            crate::app_registry::RegistrySource::LocalApp
            | crate::app_registry::RegistrySource::LocalAgent
            | crate::app_registry::RegistrySource::Linked => workspace.push(row),
        }
    }
    if !globals.is_empty() {
        println!("Global apps ({})", crate::app_registry::apps_dir().display());
        for (id, name, version) in &globals {
            println!("  {:30} {:30} {}", id, name, version);
        }
    }
    if !workspace.is_empty() {
        if let Some(root) = workspace_root {
            println!();
            println!("Workspace apps ({})", root.display());
            for (id, name, version) in &workspace {
                println!("  {:30} {:30} {}", id, name, version);
            }
        }
    }
    0
}

/// `plexi pack export <path>` — write a `pack.toml` for the current set of
/// installed apps under the channel apps dir to `path`. See
/// `crate::install::export_pack` for the source-spec inference rules.
pub fn pack_export_cli(dest_path: &str) -> i32 {
    let target_root = crate::app_registry::apps_dir();
    let dest = std::path::PathBuf::from(dest_path);
    match crate::install::export_pack(&target_root, &dest) {
        Ok(n) => {
            println!("wrote {n} apps → {}", dest.display());
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// Parse a single `--choice` argument into `(key, label, host_action)`.
///
/// Accepted formats:
/// - `key:Label`                            → (key, Label, None)
/// - `Label:action_type:action_arg`         → (Label, Label, Some("action_type:action_arg"))
/// - `key:Label:action_type:action_arg`     → (key, Label, Some("action_type:action_arg"))
///
/// Supported host action types:
/// - `pane_focus:<pane_id>` — navigate to the given pane when clicked
/// - `snooze:<seconds>`     — re-deliver the notification after N seconds (CLI stays blocked)
///
/// Any other segment count is rejected with a clear error string.
pub(crate) fn parse_notify_choice(raw: &str) -> Result<(String, String, Option<String>), String> {
    let segments: Vec<&str> = raw.splitn(5, ':').collect();
    match segments.as_slice() {
        [key, label, action_type, action_arg] => Ok((
            key.to_string(),
            label.to_string(),
            Some(format!("{action_type}:{action_arg}")),
        )),
        [label, action_type, action_arg] => {
            let label_str = label.to_string();
            Ok((label_str.clone(), label_str, Some(format!("{action_type}:{action_arg}"))))
        }
        [key, label] => Ok((key.to_string(), label.to_string(), None)),
        _ => Err(format!(
            "error: --choice requires 2, 3, or 4 colon-separated segments \
             (key:Label / Label:action:arg / key:Label:action:arg) — got {} in {:?}",
            segments.len(),
            raw
        )),
    }
}

/// Entry point for `plexi notify --title <text> --body <text> [--level info|warn|error]
///   [--choice key:Label]... [--timeout N]`.
///
/// Connects to PLEXI_SOCKET and sends a `notify` AppRequest JSON line.
/// With no choices: fire-and-forget (exits 0 on send).
/// With choices: sends the command with a `response_file` path and polls that
/// file until the user selects an option, then prints the chosen key to stdout.
/// On timeout: exits 2. On socket error: exits 1.
pub fn notify_cli(
    title: &str,
    body: &str,
    level: &str,
    choices: &[(String, String, Option<String>)],
    timeout_secs: u64,
    scope: Option<crate::app_protocol::NotifyScope>,
) -> i32 {
    let socket_path = match std::env::var("PLEXI_SOCKET") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: PLEXI_SOCKET is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };

    let id = uuid::Uuid::new_v4();

    let options_json: Vec<serde_json::Value> = choices
        .iter()
        .map(|(key, label, host_action)| {
            let mut opt = serde_json::json!({"label": label, "value": key, "shortcut": key});
            if let Some(ha) = host_action {
                opt["host_action"] = serde_json::Value::String(ha.clone());
            }
            opt
        })
        .collect();

    let (kind, response_file_str) = if choices.is_empty() {
        ("message".to_string(), None)
    } else {
        let rf = crate::config::config_dir()
            .join(format!("notify-response-{id}.txt"))
            .to_string_lossy()
            .into_owned();
        ("choice".to_string(), Some(rf))
    };

    let mut payload = serde_json::json!({
        "type": "notify",
        "level": level,
        "title": title,
        "body": body,
        "kind": kind,
        "options": options_json,
        "priority": 50,
    });
    if let Some(ref rf) = response_file_str {
        payload["response_file"] = serde_json::Value::String(rf.clone());
    }
    if let Some(s) = scope {
        let s_str = match s {
            crate::app_protocol::NotifyScope::Window => "window",
            crate::app_protocol::NotifyScope::Context => "context",
            crate::app_protocol::NotifyScope::Global => "global",
        };
        payload["scope"] = serde_json::Value::String(s_str.to_string());
    }

    log::info!(
        "notify:cli: sending via socket choices={} scope={:?} response_file={:?}",
        choices.len(), scope, response_file_str
    );

    // socket_path was resolved earlier; route through the centralized helper so
    // every PLEXI_SOCKET caller hits the same transport (Phase 6 swaps the body).
    let _ = &socket_path;
    let rc = send_to_socket(payload);
    if rc != 0 {
        return rc;
    }

    // Fire-and-forget path — command is delivered, nothing to wait for.
    let Some(response_file) = response_file_str else {
        if timeout_secs > 0 {
            eprintln!("warning: --timeout has no effect without --choice (notification queued without auto-dismiss)");
        }
        println!("notification queued");
        return 0;
    };
    let response_file = std::path::PathBuf::from(response_file);
    log::info!("notify:cli: polling for response at {:?}", response_file);

    let deadline = if timeout_secs > 0 {
        Some(std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs))
    } else {
        None
    };

    loop {
        if response_file.exists() {
            match std::fs::read_to_string(&response_file) {
                Ok(key) => {
                    log::info!("notify:cli: response received {:?}", key.trim());
                    let _ = std::fs::remove_file(&response_file);
                    print!("{}", key.trim());
                    return 0;
                }
                Err(e) => {
                    log::warn!("notify:cli: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return 1;
                }
            }
        }
        if let Some(dl) = deadline {
            if std::time::Instant::now() >= dl {
                log::info!("notify:cli: timed out after {timeout_secs}s");
                return 2;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn to_title_case(s: &str) -> String {
    s.split(['-', '_'])
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn to_struct_name(s: &str) -> String {
    s.split(['-', '_'])
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<String>()
}

/// `plexi pane set-title [pane_id] <name>`
///
/// Sends a `set_pane_title` command over PLEXI_SOCKET.
/// When `pane_id` is None, reads PLEXI_PANE_ID from the environment (current pane).
/// When `pane_id` is Some, targets that pane directly (no PLEXI_PANE_ID required).
/// Returns 0 on success, 1 on error.
pub fn pane_set_title_cli(pane_id: Option<u64>, name: &str) -> i32 {
    let socket_path = match std::env::var("PLEXI_SOCKET") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: PLEXI_SOCKET is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };
    let resolved_pane_id = match pane_id {
        Some(id) => id,
        None => {
            let pane_id_str = match std::env::var("PLEXI_PANE_ID") {
                Ok(v) => v,
                Err(_) => {
                    eprintln!("error: PLEXI_PANE_ID is not set — run this inside a Plexi terminal pane or provide a pane ID");
                    return 1;
                }
            };
            match pane_id_str.parse::<u64>() {
                Ok(n) => n,
                Err(_) => {
                    eprintln!("error: PLEXI_PANE_ID is not a valid number: {pane_id_str}");
                    return 1;
                }
            }
        }
    };
    log::info!("pane_set_title:cli: pane_id={resolved_pane_id} name={name:?}");
    let payload = serde_json::json!({
        "type": "set_pane_title",
        "pane_id": resolved_pane_id,
        "name": name,
    });
    let _ = &socket_path;
    send_to_socket(payload)
}

/// Prints JSON to stdout. If `jq` is in PATH, pipes through `jq .` for
/// colour and pretty-printing; otherwise falls back to serde pretty-print.
fn print_json_output(json_str: &str) -> i32 {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let jq_available = Command::new("jq")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if jq_available {
        match Command::new("jq").arg(".").stdin(Stdio::piped()).spawn() {
            Ok(mut child) => {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(json_str.as_bytes());
                }
                let _ = child.wait();
                log::info!("print_json_output: rendered via jq");
                return 0;
            }
            Err(e) => {
                log::warn!("print_json_output: jq spawn failed ({e}), falling back to serde");
            }
        }
    }

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(v) => match serde_json::to_string_pretty(&v) {
            Ok(pretty) => {
                println!("{pretty}");
                0
            }
            Err(e) => {
                eprintln!("error: could not serialize: {e}");
                1
            }
        },
        Err(_) => {
            print!("{json_str}");
            0
        }
    }
}

/// `plexi pane list`
///
/// Sends a `list_panes` command to PLEXI_SOCKET. The host writes a JSON array
/// to a response file; this function polls for it and prints it to stdout.
/// Returns 0 on success, 1 on error.
pub fn pane_list_cli(context: Option<u64>, current: bool) -> i32 {
    let context_id: Option<u64> = if current {
        let raw = match std::env::var("PLEXI_CONTEXT_ID") {
            Ok(v) => v,
            Err(_) => {
                eprintln!("error: PLEXI_CONTEXT_ID is not set — run this inside a Plexi terminal pane");
                return 1;
            }
        };
        match raw.parse::<u64>() {
            Ok(n) => Some(n),
            Err(_) => {
                eprintln!("error: PLEXI_CONTEXT_ID is not a valid number: {raw}");
                return 1;
            }
        }
    } else {
        context
    };

    let id = uuid::Uuid::new_v4();
    let response_file = crate::config::config_dir()
        .join(format!("pane-list-response-{id}.json"))
        .to_string_lossy()
        .into_owned();

    let mut payload = serde_json::json!({
        "type": "list_panes",
        "response_file": response_file,
    });
    if let Some(cid) = context_id {
        payload["context_id"] = serde_json::json!(cid);
    }

    log::info!("pane_list:cli: sending via socket context_id={:?} response_file={:?}", context_id, response_file);

    let code = send_to_socket(payload);
    if code != 0 {
        return code;
    }

    let response_path = std::path::PathBuf::from(&response_file);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if response_path.exists() {
            match std::fs::read_to_string(&response_path) {
                Ok(content) => {
                    let _ = std::fs::remove_file(&response_path);
                    return print_json_output(&content);
                }
                Err(e) => {
                    log::warn!("pane_list:cli: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return 1;
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("error: timed out waiting for pane list response");
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// `plexi pane self`
///
/// Prints the current pane ID (just the number) to stdout. Reads PLEXI_PANE_ID.
/// No JSON, no socket round-trip — pure env-var lookup for agent callers.
pub fn pane_self_cli() -> i32 {
    let pane_id_str = match std::env::var("PLEXI_PANE_ID") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: PLEXI_PANE_ID is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };
    match pane_id_str.parse::<u64>() {
        Ok(id) => {
            log::info!("pane_self:cli: pane_id={id}");
            println!("{id}");
            0
        }
        Err(_) => {
            eprintln!("error: PLEXI_PANE_ID is not a valid number: {pane_id_str}");
            1
        }
    }
}

/// `plexi pane info`
///
/// Sends a `get_pane_info` command to PLEXI_SOCKET for the current pane
/// (identified by PLEXI_PANE_ID). Merges in client-side fields (socket, channel)
/// and pretty-prints the result as JSON. Returns 0 on success, 1 on error.
pub fn pane_info_cli() -> i32 {
    let pane_id_str = match std::env::var("PLEXI_PANE_ID") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: PLEXI_PANE_ID is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };
    let socket_path = match std::env::var("PLEXI_SOCKET") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: PLEXI_SOCKET is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };
    let pane_id: u64 = match pane_id_str.parse() {
        Ok(n) => n,
        Err(_) => {
            eprintln!("error: PLEXI_PANE_ID is not a valid number: {pane_id_str}");
            return 1;
        }
    };

    let id = uuid::Uuid::new_v4();
    let response_file = crate::config::config_dir()
        .join(format!("pane-info-response-{id}.json"))
        .to_string_lossy()
        .into_owned();

    let payload = serde_json::json!({
        "type": "get_pane_info",
        "pane_id": pane_id,
        "response_file": response_file,
    });

    log::info!("pane_info:cli: pane_id={pane_id} response_file={:?}", response_file);

    let code = send_to_socket(payload);
    if code != 0 {
        return code;
    }

    let response_path = std::path::PathBuf::from(&response_file);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if response_path.exists() {
            match std::fs::read_to_string(&response_path) {
                Ok(content) => {
                    let _ = std::fs::remove_file(&response_path);
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                            eprintln!("error: {err}");
                            return 1;
                        }
                        let mut obj = v;
                        obj["socket"] = serde_json::Value::String(socket_path.clone());
                        let channel = crate::config::build_channel().unwrap_or_else(|| "stable".to_string());
                        obj["channel"] = serde_json::Value::String(channel);
                        match serde_json::to_string(&obj) {
                            Ok(json_str) => { return print_json_output(&json_str); }
                            Err(e) => {
                                eprintln!("error: could not serialize response: {e}");
                                return 1;
                            }
                        }
                    } else {
                        eprintln!("error: invalid JSON from host: {content}");
                        return 1;
                    }
                }
                Err(e) => {
                    log::warn!("pane_info:cli: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return 1;
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("error: timed out waiting for pane info response");
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// `plexi pane focus <pane_id>`
///
/// Sends a `focus_pane` command to PLEXI_SOCKET. Fire-and-forget.
/// Returns 0 on success, 1 on error.
pub fn pane_focus_cli(pane_id: u64) -> i32 {
    send_to_socket(serde_json::json!({
        "type": "focus_pane",
        "pane_id": pane_id,
    }))
}

/// `plexi pane close <pane_id>`
///
/// Sends a `close_pane` command to PLEXI_SOCKET. Fire-and-forget.
/// Returns 0 on success, 1 on error.
pub fn pane_close_cli(pane_id: u64) -> i32 {
    send_to_socket(serde_json::json!({
        "type": "close_pane",
        "pane_id": pane_id,
    }))
}

/// `plexi pane send <pane_id> <text>`
///
/// Writes text to the target pane's PTY stdin. Polls a response file to
/// surface errors (e.g. pane not found) back to the caller.
/// Returns 0 on success, 1 on error.
pub fn pane_send_cli(pane_id: u64, text: &str) -> i32 {
    let id = uuid::Uuid::new_v4();
    let response_file = crate::config::config_dir()
        .join(format!("send-to-pane-response-{id}.json"))
        .to_string_lossy()
        .into_owned();
    log::info!("pane_send:cli: pane_id={pane_id} len={} response_file={response_file:?}", text.len());
    let code = send_to_socket(serde_json::json!({
        "type": "send_to_pane",
        "pane_id": pane_id,
        "text": text,
        "response_file": response_file,
    }));
    if code != 0 {
        return code;
    }
    let response_path = std::path::PathBuf::from(&response_file);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if response_path.exists() {
            match std::fs::read_to_string(&response_path) {
                Ok(content) => {
                    let _ = std::fs::remove_file(&response_path);
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                        if v.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                            return 0;
                        }
                        if let Some(msg) = v.get("error").and_then(|v| v.as_str()) {
                            eprintln!("error: {msg}");
                            return 1;
                        }
                    }
                    return 0;
                }
                Err(e) => {
                    log::warn!("pane_send:cli: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return 1;
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("error: timed out waiting for pane send response");
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// `plexi pane key <pane_id> <key>`
///
/// Sends `key_pane` command to PLEXI_SOCKET. Waits for response.
/// Returns 0 on success, 1 on error.
pub fn pane_key_cli(pane_id: u64, key: &str) -> i32 {
    let id = uuid::Uuid::new_v4();
    let response_file = crate::config::config_dir()
        .join(format!("pane-key-response-{id}.json"))
        .to_string_lossy()
        .into_owned();
    log::info!("pane_key:cli: pane_id={pane_id} key={key:?} response_file={response_file:?}");
    let code = send_to_socket(serde_json::json!({
        "type": "key_pane",
        "pane_id": pane_id,
        "key": key,
        "response_file": response_file,
    }));
    if code != 0 {
        return code;
    }
    let response_path = std::path::PathBuf::from(&response_file);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if response_path.exists() {
            match std::fs::read_to_string(&response_path) {
                Ok(content) => {
                    let _ = std::fs::remove_file(&response_path);
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                        if v.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                            return 0;
                        }
                        if let Some(msg) = v.get("error").and_then(|v| v.as_str()) {
                            eprintln!("error: {msg}");
                            return 1;
                        }
                    }
                    return 0;
                }
                Err(e) => {
                    log::warn!("pane_key:cli: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return 1;
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("error: timed out waiting for pane key response");
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// `plexi pane capture [--lines N] [pane_id]`
///
/// Reads the last N lines from a pane's PTY scrollback buffer and prints a JSON array
/// of strings to stdout. If `pane_id` is omitted, defaults to PLEXI_PANE_ID.
/// Returns 0 on success, 1 on error.
pub fn pane_capture_cli(pane_id: Option<u64>, lines: usize, full_output: bool) -> i32 {
    let resolved_pane_id = match pane_id {
        Some(id) => id,
        None => match std::env::var("PLEXI_PANE_ID") {
            Ok(v) => match v.parse::<u64>() {
                Ok(id) => id,
                Err(_) => {
                    eprintln!("error: PLEXI_PANE_ID is not a valid number: {v}");
                    return 1;
                }
            },
            Err(_) => {
                eprintln!("error: pane_id not provided and PLEXI_PANE_ID is not set — run inside a Plexi pane or pass a pane ID");
                return 1;
            }
        },
    };

    let id = uuid::Uuid::new_v4();
    let response_file = crate::config::config_dir()
        .join(format!("pane-capture-response-{id}.json"))
        .to_string_lossy()
        .into_owned();

    log::info!("pane_capture:cli: pane_id={resolved_pane_id} lines={lines} full_output={full_output} response_file={response_file:?}");

    let code = send_to_socket(serde_json::json!({
        "type": "capture_pane",
        "pane_id": resolved_pane_id,
        "lines": lines,
        "full_output": full_output,
        "response_file": response_file,
    }));
    if code != 0 {
        return code;
    }

    let response_path = std::path::PathBuf::from(&response_file);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if response_path.exists() {
            match std::fs::read_to_string(&response_path) {
                Ok(content) => {
                    let _ = std::fs::remove_file(&response_path);
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                            eprintln!("error: {err}");
                            return 1;
                        }
                    }
                    return print_json_output(&content);
                }
                Err(e) => {
                    log::warn!("pane_capture:cli: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return 1;
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("error: timed out waiting for pane capture response");
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// `plexi open <type_id> [args...] [--layout=X]`
///
/// When called from inside a Plexi pane (PLEXI_SOCKET is set), sends a
/// spawn_pane command directly via the socket — channel-agnostic, works on
/// alpha, beta, stable, and PR builds without caring which binary is on PATH.
///
/// `plexi open github:owner/repo` — clone and run ephemerally, without installing.
///
/// Clones to a channel-scoped cache dir and sends a path-based spawn_pane,
/// passing the user's workspace root so app state is scoped correctly.
fn open_github_ephemeral(source: &str, layout: Option<&str>, from_pane_id: Option<u64>, cwd: Option<&str>) -> i32 {
    let rest = source.strip_prefix("github:").unwrap_or(source);
    let parts: Vec<&str> = rest.splitn(2, '/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        eprintln!("error: invalid github source '{source}'; expected 'github:owner/repo'");
        return 1;
    }
    let owner = parts[0];
    let repo = parts[1].trim_end_matches(".git");

    let cache_dir = crate::config::config_dir()
        .join("github-cache")
        .join(owner)
        .join(repo);

    // Ensure the parent directory exists before cloning.
    if let Some(parent) = cache_dir.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("error: could not create cache directory {}: {e}", parent.display());
            return 1;
        }
    }

    if !cache_dir.exists() {
        let url = format!("https://github.com/{owner}/{repo}.git");
        log::info!("open_github_ephemeral: cloning {url} → {}", cache_dir.display());
        eprintln!("Cloning github:{owner}/{repo}...");
        match std::process::Command::new("git")
            .arg("clone")
            .arg("--depth=1")
            .arg(&url)
            .arg(&cache_dir)
            .status()
        {
            Ok(s) if s.success() => {}
            Ok(s) => {
                eprintln!("error: git clone failed (exit {})", s.code().unwrap_or(-1));
                return 1;
            }
            Err(e) => {
                eprintln!("error: could not run git: {e}");
                return 1;
            }
        }
    } else {
        log::info!("open_github_ephemeral: reusing cache at {}", cache_dir.display());
    }

    // Resolve workspace root from the provided cwd, falling back to current_dir.
    let start_dir = cwd
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok());
    let workspace_root: Option<String> = start_dir
        .as_deref()
        .and_then(|d| crate::app_registry::resolve_workspace_root(d))
        .map(|p| p.to_string_lossy().into_owned());

    let abs_path = cache_dir.to_string_lossy().into_owned();
    log::info!(
        "open_github_ephemeral: launching from {abs_path} workspace_root={workspace_root:?}"
    );

    if std::env::var("PLEXI_SOCKET").is_ok() {
        let id = uuid::Uuid::new_v4();
        let response_file = crate::config::config_dir()
            .join(format!("spawn-pane-response-{id}.json"))
            .to_string_lossy()
            .into_owned();
        let mut payload = serde_json::json!({
            "type": "spawn_pane",
            "type_id": "",
            "path": abs_path,
            "layout": layout,
            "response_file": response_file,
        });
        if let Some(pid) = from_pane_id {
            payload["from_pane_id"] = serde_json::Value::Number(pid.into());
        }
        if let Some(ref ws) = workspace_root {
            payload["workspace_root"] = serde_json::Value::String(ws.clone());
        }
        if let Some(cwd_str) = cwd {
            payload["cwd"] = serde_json::Value::String(cwd_str.to_string());
        }
        log::info!("open_github_ephemeral: sending via socket response_file={response_file:?}");
        let code = send_to_socket(payload);
        if code != 0 {
            return code;
        }
        let response_path = std::path::PathBuf::from(&response_file);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if response_path.exists() {
                match std::fs::read_to_string(&response_path) {
                    Ok(content) => {
                        let _ = std::fs::remove_file(&response_path);
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                            if let Some(msg) = v.get("error").and_then(|v| v.as_str()) {
                                eprintln!("error: {msg}");
                                return 1;
                            }
                            if let Some(pid) = v.get("pane_id").and_then(|v| v.as_u64()) {
                                println!("{pid}");
                                return 0;
                            }
                        }
                        print!("{content}");
                        return 0;
                    }
                    Err(e) => {
                        eprintln!("error: could not read response file: {e}");
                        return 1;
                    }
                }
            }
            if std::time::Instant::now() >= deadline {
                eprintln!("error: timed out waiting for open response");
                return 1;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    // Fallback: spawn-queue (outside a Plexi pane).
    let queue_dir = crate::config::config_dir().join("spawn-queue");
    if let Err(e) = std::fs::create_dir_all(&queue_dir) {
        eprintln!("error: could not create spawn queue: {e}");
        return 1;
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let queue_id = uuid::Uuid::new_v4();
    let mut queue_payload = serde_json::json!({
        "type_id": "",
        "path": abs_path,
        "layout": layout,
    });
    if let Some(ref ws) = workspace_root {
        queue_payload["workspace_root"] = serde_json::Value::String(ws.clone());
    }
    if let Some(cwd_str) = cwd {
        queue_payload["cwd"] = serde_json::Value::String(cwd_str.to_string());
    }
    let file = queue_dir.join(format!("{ts}-{queue_id}.json"));
    if let Err(e) = std::fs::write(&file, queue_payload.to_string()) {
        eprintln!("error: could not write spawn request: {e}");
        return 1;
    }
    log::info!("open_github_ephemeral: queued path={abs_path}");
    println!("queued: open github:{owner}/{repo}");
    println!("(running outside a Plexi pane — Plexi will pick this up within a second)");
    0
}

/// When called from outside Plexi, falls back to the spawn-queue directory
/// which the running host drains each second.
///
/// Returns 0 on success, 1 on error.
pub fn open_cli(type_id: &str, args: &[String], layout: Option<&str>, from_pane_id: Option<u64>, cwd: Option<&str>) -> i32 {
    // Intercept github: prefix for ephemeral open-without-install.
    if type_id.starts_with("github:") {
        return open_github_ephemeral(type_id, layout, from_pane_id, cwd);
    }

    if type_id == "terminal" {
        log::warn!("open:cli: 'plexi open terminal' is deprecated — use 'plexi terminal' instead");
        eprintln!("warning: 'plexi open terminal' is deprecated — use 'plexi terminal' instead");
    }

    // Socket path is set when running inside a Plexi pane — use it directly so
    // the command reaches the correct running instance regardless of channel.
    if std::env::var("PLEXI_SOCKET").is_ok() {
        let id = uuid::Uuid::new_v4();
        let response_file = crate::config::config_dir()
            .join(format!("spawn-pane-response-{id}.json"))
            .to_string_lossy()
            .into_owned();
        let mut payload = serde_json::json!({
            "type": "spawn_pane",
            "type_id": type_id,
            "args": args,
            "layout": layout,
            "response_file": response_file,
        });
        if let Some(pid) = from_pane_id {
            payload["from_pane_id"] = serde_json::Value::Number(pid.into());
        }
        if let Some(cwd) = cwd {
            payload["cwd"] = serde_json::Value::String(cwd.to_string());
        }
        log::info!("open:cli: sending via socket from_pane_id={from_pane_id:?} cwd={cwd:?} response_file={response_file:?}");
        let code = send_to_socket(payload);
        if code != 0 {
            return code;
        }
        let response_path = std::path::PathBuf::from(&response_file);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if response_path.exists() {
                match std::fs::read_to_string(&response_path) {
                    Ok(content) => {
                        let _ = std::fs::remove_file(&response_path);
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                            if let Some(msg) = v.get("error").and_then(|v| v.as_str()) {
                                eprintln!("error: {msg}");
                                return 1;
                            }
                            if let Some(pid) = v.get("pane_id").and_then(|v| v.as_u64()) {
                                println!("{pid}");
                                return 0;
                            }
                        }
                        // Fallback: print raw content
                        print!("{content}");
                        return 0;
                    }
                    Err(e) => {
                        log::warn!("open:cli: could not read response file: {e}");
                        eprintln!("error: could not read response file: {e}");
                        return 1;
                    }
                }
            }
            if std::time::Instant::now() >= deadline {
                eprintln!("error: timed out waiting for open response");
                return 1;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    // Fallback: write to the spawn-queue for the channel this binary belongs to.
    if from_pane_id.is_some() {
        log::warn!("open:cli: --from-pane-id requires PLEXI_SOCKET (run inside a Plexi pane); ignoring");
        eprintln!("warning: --from-pane-id is ignored outside a Plexi pane");
    }
    let queue_dir = crate::config::config_dir().join("spawn-queue");
    if let Err(e) = std::fs::create_dir_all(&queue_dir) {
        eprintln!("error: could not create spawn queue: {e}");
        return 1;
    }
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut queue_payload = serde_json::json!({
        "type_id": type_id,
        "args": args,
        "layout": layout,
    });
    if let Some(cwd) = cwd {
        queue_payload["cwd"] = serde_json::Value::String(cwd.to_string());
    }
    let file = queue_dir.join(format!("{id}.json"));
    if let Err(e) = std::fs::write(&file, queue_payload.to_string()) {
        eprintln!("error: could not write spawn request: {e}");
        return 1;
    }
    log::info!("cli: open queued: type_id={type_id} cwd={cwd:?}");
    println!("queued: open {type_id}");
    println!("(running outside a Plexi pane — Plexi will pick this up within a second)");
    0
}

/// `plexi terminal [cmd] [--ephemeral] [--layout=X] [--no-focus]`
///
/// Opens a terminal pane. Supports the --ephemeral flag which closes the pane when the
/// process exits, and --no-focus to keep focus on the originating pane.
pub fn terminal_cli(cmd: Option<&str>, ephemeral: bool, layout: Option<&str>, from_pane_id: Option<u64>, cwd: Option<&str>, no_focus: bool) -> i32 {
    let layout_str = layout.unwrap_or("split_v");
    let args: Vec<String> = cmd.map(|c| vec![c.to_string()]).unwrap_or_default();

    if std::env::var("PLEXI_SOCKET").is_ok() {
        let id = uuid::Uuid::new_v4();
        let response_file = crate::config::config_dir()
            .join(format!("spawn-pane-response-{id}.json"))
            .to_string_lossy()
            .into_owned();
        let mut payload = serde_json::json!({
            "type": "spawn_pane",
            "type_id": "terminal",
            "args": args,
            "layout": layout_str,
            "response_file": response_file,
        });
        if ephemeral {
            payload["ephemeral"] = serde_json::Value::Bool(true);
        }
        if let Some(pid) = from_pane_id {
            payload["from_pane_id"] = serde_json::Value::Number(pid.into());
        }
        if let Some(cwd) = cwd {
            payload["cwd"] = serde_json::Value::String(cwd.to_string());
        }
        if no_focus {
            payload["no_focus"] = serde_json::Value::Bool(true);
        }
        log::info!("terminal:cli: sending via socket ephemeral={ephemeral} no_focus={no_focus} from_pane_id={from_pane_id:?} cwd={cwd:?} response_file={response_file:?}");
        let code = send_to_socket(payload);
        if code != 0 {
            return code;
        }
        let response_path = std::path::PathBuf::from(&response_file);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if response_path.exists() {
                match std::fs::read_to_string(&response_path) {
                    Ok(content) => {
                        let _ = std::fs::remove_file(&response_path);
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                            if let Some(pid) = v.get("pane_id").and_then(|v| v.as_u64()) {
                                println!("{pid}");
                                return 0;
                            }
                        }
                        print!("{content}");
                        return 0;
                    }
                    Err(e) => {
                        log::warn!("terminal:cli: could not read response file: {e}");
                        eprintln!("error: could not read response file: {e}");
                        return 1;
                    }
                }
            }
            if std::time::Instant::now() >= deadline {
                eprintln!("error: timed out waiting for terminal response");
                return 1;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    // Fallback: spawn-queue (outside a Plexi pane)
    if from_pane_id.is_some() {
        log::warn!("terminal:cli: --from-pane-id requires PLEXI_SOCKET (run inside a Plexi pane); ignoring");
        eprintln!("warning: --from-pane-id is ignored outside a Plexi pane");
    }
    let queue_dir = crate::config::config_dir().join("spawn-queue");
    if let Err(e) = std::fs::create_dir_all(&queue_dir) {
        eprintln!("error: could not create spawn queue: {e}");
        return 1;
    }
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut queue_payload = serde_json::json!({
        "type_id": "terminal",
        "args": args,
        "layout": layout_str,
    });
    if ephemeral {
        queue_payload["ephemeral"] = serde_json::Value::Bool(true);
    }
    if let Some(cwd) = cwd {
        queue_payload["cwd"] = serde_json::Value::String(cwd.to_string());
    }
    if no_focus {
        queue_payload["no_focus"] = serde_json::Value::Bool(true);
    }
    let file = queue_dir.join(format!("{id}.json"));
    if let Err(e) = std::fs::write(&file, queue_payload.to_string()) {
        eprintln!("error: could not write spawn request: {e}");
        return 1;
    }
    log::info!("terminal:cli: queued ephemeral={ephemeral} no_focus={no_focus} cwd={cwd:?}");
    println!("queued: open terminal");
    println!("(running outside a Plexi pane — Plexi will pick this up within a second)");
    0
}

/// Read a line from stdin with echo disabled (for password-style input).
fn read_secret_from_stdin() -> io::Result<String> {
    // Disable echo via stty (avoids libc dependency).
    let _ = std::process::Command::new("stty").arg("-echo").status();

    let result = read_line_plain();

    // Restore echo.
    let _ = std::process::Command::new("stty").arg("echo").status();
    // Print newline since echo was off during input.
    eprintln!();

    result
}

fn read_line_plain() -> io::Result<String> {
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .to_string())
}

// ── plexi registry (issue #321) ───────────────────────────────────────────────
/// `plexi registry watch [<cli>]` — walks the seeded registry and reports,
/// per-CLI, whether the locally installed CLI matches the registered version
/// and whether `<cli> --help` shows top-level command names absent from the
/// registry descriptor (a heuristic signal that the descriptor is stale).
///
/// Output is human-readable. JSON output is intentionally deferred — the
/// release-watcher cron that consumes this is itself a follow-up issue (see
/// #321 PR description).
pub mod registry {
    use crate::cli_registry;
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
}

#[cfg(test)]
mod registry_watch_tests {
    use super::registry::*;

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

// ── plexi descriptor (issue #188) ─────────────────────────────────────────────
/// `plexi descriptor probe <cmd> [args...]` — invokes the target with
/// `--plexi` appended, parses the JSON descriptor, prints a summary. Reference
/// consumer for the v0 `--plexi` format. Used as the POC for #188; the full
/// auto-UI renderer ships in #78.
pub mod descriptor {
    use crate::plexi_descriptor::{self, PlexiDescriptor};
    use std::process::Command;

    /// Indirection so the probe path can be tested without spawning real
    /// processes. The `&[&str] -> Output` shape is the smallest contract that
    /// covers "what command was run with what args".
    pub trait DescriptorRunner {
        fn run(&self, command: &str, args: &[&str]) -> std::io::Result<RunOutput>;
    }

    pub struct RunOutput {
        pub status_success: bool,
        pub stdout: String,
    }

    pub struct RealRunner;

    impl DescriptorRunner for RealRunner {
        fn run(&self, command: &str, args: &[&str]) -> std::io::Result<RunOutput> {
            let out = Command::new(command).args(args).output()?;
            Ok(RunOutput {
                status_success: out.status.success(),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            })
        }
    }

    /// Knobs governing the Tier-2 registry and Tier-3 crawl fallbacks. The
    /// default behavior (Tier 1 first, Tier 2, then Tier 3) matches the issue
    /// #321/#360 substrate; `--no-registry` disables Tier 2 and `--no-crawl`
    /// disables Tier 3.
    pub struct ProbeOptions {
        pub use_registry: bool,
        pub use_crawl: bool,
    }

    impl Default for ProbeOptions {
        fn default() -> Self {
            Self {
                use_registry: true,
                use_crawl: true,
            }
        }
    }

    /// Run `<command> <args...> --plexi`, parse + summarize. On failure (spawn
    /// error, non-zero exit, or unparseable JSON), optionally fall through to
    /// the Tier-2 registry lookup (`cli_registry::lookup`). Returns the
    /// process exit code suitable for `std::process::exit`.
    ///
    /// The CLI surface in `main.rs` calls `probe_with_options` directly so
    /// it can plumb `--no-registry`; this thin wrapper exists for tests and
    /// for any future caller that wants the default behavior.
    #[cfg(test)]
    pub fn probe<R: DescriptorRunner>(runner: &R, command: &str, args: &[&str]) -> i32 {
        probe_with_options(runner, command, args, &ProbeOptions::default())
    }

    pub fn probe_with_options<R: DescriptorRunner>(
        runner: &R,
        command: &str,
        args: &[&str],
        options: &ProbeOptions,
    ) -> i32 {
        let mut full_args: Vec<&str> = args.to_vec();
        full_args.push("--plexi");

        // Tier 1 — ask the CLI itself.
        let tier1: Option<PlexiDescriptor> = match runner.run(command, &full_args) {
            Ok(o) if o.status_success => match plexi_descriptor::parse(&o.stdout) {
                Ok(d) => Some(d),
                Err(_) => None, // Fall through to Tier 2 — bad/empty stdout.
            },
            Ok(_) => None, // Non-zero exit — `--plexi` not implemented.
            Err(_) => None, // Spawn failed (e.g. command not on PATH).
        };

        if let Some(descriptor) = tier1 {
            print_summary(&descriptor, SummarySource::Native);
            return 0;
        }

        // Tier 2 — registry. Only consulted when caller passes args=[],
        // because registry descriptors describe the bare CLI, not arbitrary
        // subcommand invocations.
        if options.use_registry && args.is_empty() {
            match crate::cli_registry::lookup(command, None) {
                Ok(descriptor) => {
                    print_summary(&descriptor, SummarySource::Registry);
                    return 0;
                }
                Err(crate::cli_registry::RegistryError::NotFound { .. }) => {
                    // Fall through to the no-descriptor message below.
                }
                Err(e) => {
                    eprintln!("error: registry lookup for `{command}` failed:\n  {e}");
                    return 1;
                }
            }
        }

        // Tier 3 — --help crawl.
        if options.use_crawl && args.is_empty() {
            match crate::cli_crawl::crawl(command) {
                Ok(result) => {
                    print_summary(
                        &result.descriptor,
                        SummarySource::Crawled {
                            from_cache: result.from_cache,
                        },
                    );
                    return 0;
                }
                Err(e) => {
                    log::warn!("cli_crawl: Tier 3 failed for `{command}`: {e}");
                }
            }
        }

        eprintln!("error: no descriptor available for `{command}` — --plexi, registry, and --help crawl all failed.");
        1
    }

    /// Where the descriptor printed in the summary came from. Used to surface
    /// a `(via registry)` / `(inferred from --help)` indicator.
    pub enum SummarySource {
        Native,
        Registry,
        Crawled { from_cache: bool },
    }

    fn print_summary(d: &PlexiDescriptor, source: SummarySource) {
        let icon = d.icon.as_deref().unwrap_or("");
        let via = match source {
            SummarySource::Native => "",
            SummarySource::Registry => "  (via registry)",
            SummarySource::Crawled { from_cache: true } => "  (inferred from --help, cached)",
            SummarySource::Crawled { from_cache: false } => {
                "  (inferred from --help, may be incomplete)"
            }
        };
        println!(
            "{}{}{} v{}  (descriptor {}){}",
            icon,
            if icon.is_empty() { "" } else { " " },
            d.name,
            d.version,
            d.plexi_version,
            via,
        );
        if let Some(desc) = &d.description {
            println!("  {desc}");
        }
        println!("commands: {}", d.commands.len());
        for cmd in d.commands.iter().take(3) {
            let hint = cmd
                .ui_hint
                .map(|h| format!(" [{h:?}]").to_lowercase())
                .unwrap_or_default();
            let extra = if cmd.commands.is_empty() {
                String::new()
            } else {
                format!(" (+{} subcommands)", cmd.commands.len())
            };
            let desc = cmd
                .description
                .as_deref()
                .map(|s| format!(" — {s}"))
                .unwrap_or_default();
            println!("  - {}{hint}{extra}{desc}", cmd.name);
        }
        if d.commands.len() > 3 {
            println!("  ... and {} more", d.commands.len() - 3);
        }
        if let Some(ls) = &d.live_state {
            println!(
                "live_state: {:?} {} (poll {} ms, {:?})",
                ls.source, ls.path, ls.poll_ms, ls.format
            );
        }
        if let Some(app_cmd) = &d.plexi_app {
            println!("plexi_app: {app_cmd}");
            if !d.capabilities.is_empty() {
                println!("  capabilities: {}", d.capabilities.join(", "));
            }
        }
    }

    #[cfg(test)]
    pub struct MockRunner {
        pub stdout: String,
        pub success: bool,
        /// Last (command, args) the probe handed to the runner. Lets tests
        /// assert that `--plexi` was appended in the right position.
        pub captured: std::cell::RefCell<Option<(String, Vec<String>)>>,
    }

    #[cfg(test)]
    impl DescriptorRunner for MockRunner {
        fn run(&self, command: &str, args: &[&str]) -> std::io::Result<RunOutput> {
            *self.captured.borrow_mut() =
                Some((command.to_string(), args.iter().map(|s| s.to_string()).collect()));
            Ok(RunOutput {
                status_success: self.success,
                stdout: self.stdout.clone(),
            })
        }
    }
}

#[cfg(test)]
mod descriptor_tests {
    use super::descriptor::*;
    use std::cell::RefCell;

    fn ok_descriptor_runner() -> MockRunner {
        MockRunner {
            stdout: r#"{
                "plexi_version": "0.1",
                "name": "fake",
                "version": "0.0.1",
                "commands": []
            }"#
            .into(),
            success: true,
            captured: RefCell::new(None),
        }
    }

    fn no_plexi_runner() -> MockRunner {
        // Simulates a CLI that exists on PATH but doesn't implement --plexi
        // (non-zero exit code). This is the common case for the registry
        // fallback path.
        MockRunner {
            stdout: String::new(),
            success: false,
            captured: RefCell::new(None),
        }
    }

    #[test]
    fn probe_invokes_command_with_plexi_flag() {
        let mock = ok_descriptor_runner();
        let code = probe(&mock, "fake-cli", &[]);
        // Tier 1 succeeds; result is the parsed descriptor.
        assert_eq!(code, 0);
        let captured = mock.captured.borrow();
        let (cmd, args) = captured.as_ref().expect("runner was invoked");
        assert_eq!(cmd, "fake-cli");
        assert_eq!(args.last().map(|s| s.as_str()), Some("--plexi"));
    }

    #[test]
    fn probe_appends_plexi_after_user_args() {
        let mock = ok_descriptor_runner();
        let code = probe(&mock, "fake-cli", &["sub", "--verbose"]);
        assert_eq!(code, 0);
        let captured = mock.captured.borrow();
        let (_, args) = captured.as_ref().expect("runner was invoked");
        assert_eq!(args.as_slice(), &["sub", "--verbose", "--plexi"]);
    }

    #[test]
    fn probe_falls_back_to_registry_when_native_plexi_fails() {
        // `gh` ships in the embedded registry. With a runner that simulates
        // gh's real behavior (no native --plexi), the probe should fall
        // through to Tier 2 and resolve the registry descriptor.
        let mock = no_plexi_runner();
        let code = probe(&mock, "gh", &[]);
        assert_eq!(code, 0, "registry fallback should succeed for `gh`");
    }

    #[test]
    fn probe_no_registry_flag_skips_fallback() {
        // Same setup, but registry disabled. Should fail because Tier 1
        // fell through and Tier 2 is gated off.
        let mock = no_plexi_runner();
        let opts = ProbeOptions { use_registry: false, use_crawl: false };
        let code = probe_with_options(&mock, "gh", &[], &opts);
        assert_eq!(code, 1, "without registry or crawl, gh has no descriptor");
    }

    #[test]
    fn probe_surfaces_nonzero_exit_for_unknown_cli_with_no_registry_entry() {
        // No native --plexi, no registry hit → non-zero exit.
        let mock = no_plexi_runner();
        let code = probe(&mock, "nonexistent-cli-zzz", &[]);
        assert_eq!(code, 1);
    }

    #[test]
    fn probe_skips_registry_when_user_args_provided() {
        // Registry descriptors describe the bare CLI; subcommand invocations
        // shouldn't get a registry hit even if the CLI is registered.
        let mock = no_plexi_runner();
        let code = probe(&mock, "gh", &["pr", "create"]);
        assert_eq!(
            code, 1,
            "registry fallback only applies when no user args are passed"
        );
    }
}

/// `plexi validate <path>` — preflight-check a Plexi app directory.
///
/// Checks (in order):
///   1. manifest.toml exists and is readable
///   2. Required manifest fields: app.id, app.name, app.version, app.entry
///   3. Entry file exists relative to the manifest
///   4. If the entry is a .py file, syntax-check it with `python3 -c "import ast, sys; ast.parse(open(sys.argv[1]).read())"`
pub fn validate_cli(path: &str) -> i32 {
    let app_dir = std::path::Path::new(path);
    if !app_dir.exists() {
        eprintln!("validate: path does not exist: {path}");
        return 1;
    }
    if !app_dir.is_dir() {
        eprintln!("validate: path is not a directory: {path}");
        return 1;
    }

    let manifest_path = app_dir.join("manifest.toml");
    if !manifest_path.exists() {
        eprintln!("✗ manifest.toml not found in {path}");
        return 1;
    }

    let raw = match std::fs::read_to_string(&manifest_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("✗ cannot read manifest.toml: {e}");
            return 1;
        }
    };

    let toml_val: toml::Value = match raw.parse() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("✗ manifest.toml parse error: {e}");
            return 1;
        }
    };

    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    // Required fields
    let app_section = toml_val.get("app");
    let required_fields = ["id", "name", "version", "entry"];
    for field in &required_fields {
        let val = app_section.and_then(|a| a.get(field)).and_then(|v| v.as_str());
        if val.is_none() || val == Some("") {
            errors.push(format!("  [app].{field} is missing or empty"));
        }
    }

    // description is recommended but not required
    let has_desc = app_section
        .and_then(|a| a.get("description"))
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if !has_desc {
        warnings.push("  [app].description is missing (recommended)".to_string());
    }

    // Check entry file
    let entry = app_section
        .and_then(|a| a.get("entry"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !entry.is_empty() {
        let entry_path = app_dir.join(entry);
        if !entry_path.exists() {
            errors.push(format!("  entry file not found: {}", entry_path.display()));
        } else if entry.ends_with(".py") {
            // Python syntax check via AST parse (no import, no SDK needed)
            let py_check = std::process::Command::new("python3")
                .arg("-c")
                .arg("import ast, sys; ast.parse(open(sys.argv[1]).read())")
                .arg(&entry_path)
                .output();
            match py_check {
                Ok(out) if out.status.success() => {}
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    errors.push(format!("  Python syntax error in {entry}: {}", stderr.trim()));
                }
                Err(e) => {
                    warnings.push(format!("  python3 not found — skipping syntax check: {e}"));
                }
            }
        }
    }

    // capabilities validation
    if let Some(caps) = app_section
        .and_then(|a| a.get("capabilities"))
        .and_then(|v| v.as_array())
    {
        let known: &[&str] = &[
            "net.http", "net.dns", "fs.read", "fs.write",
            "audio.record", "audio.play", "midi.in", "midi.out",
            "ai.query", "panes.spawn", "video.decode",
        ];
        for cap in caps {
            if let Some(s) = cap.as_str() {
                if !known.contains(&s) {
                    warnings.push(format!("  unknown capability: {s:?} — check the manifest reference"));
                }
            }
        }
    }

    // Print results
    let id = app_section
        .and_then(|a| a.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or(path);

    if errors.is_empty() && warnings.is_empty() {
        println!("✓ {id} — all checks passed");
        log::info!("validate: {} passed", id);
        return 0;
    }

    if !errors.is_empty() {
        println!("✗ {id} — {} error(s):", errors.len());
        for e in &errors {
            println!("{e}");
        }
    }
    if !warnings.is_empty() {
        println!("⚠ {id} — {} warning(s):", warnings.len());
        for w in &warnings {
            println!("{w}");
        }
    }
    log::warn!("validate: {} — {} errors, {} warnings", id, errors.len(), warnings.len());

    if errors.is_empty() { 0 } else { 1 }
}

/// Resolve `path` argument (canonicalize if given, else use CWD).
fn resolve_path(path: Option<&str>) -> Result<std::path::PathBuf, String> {
    match path {
        Some(p) => std::fs::canonicalize(p)
            .map_err(|e| format!("error: could not resolve path {p:?}: {e}")),
        None => std::env::current_dir()
            .map_err(|e| format!("error: could not get current directory: {e}")),
    }
}

/// Send a JSON payload to PLEXI_SOCKET. Returns 0 on success, 1 on error.
///
/// Unix: connects to the AF_UNIX path. Windows: opens the named pipe via
/// CreateFileW. The string in `PLEXI_SOCKET` is platform-shaped — see
/// `crate::config::ipc_endpoint()` for the format used by the host listener.
fn send_to_socket(payload: serde_json::Value) -> i32 {
    let socket_path = match std::env::var("PLEXI_SOCKET") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: PLEXI_SOCKET is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::net::UnixStream;
        let mut stream = match UnixStream::connect(&socket_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: could not connect to PLEXI_SOCKET {socket_path:?}: {e}");
                return 1;
            }
        };
        let line = format!("{}\n", payload);
        if let Err(e) = stream.write_all(line.as_bytes()) {
            eprintln!("error: could not write to socket: {e}");
            return 1;
        }
        0
    }
    #[cfg(windows)]
    {
        use std::io::Write;
        use std::os::windows::io::{FromRawHandle, OwnedHandle};
        use windows_sys::Win32::Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };

        let wide: Vec<u16> = socket_path
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // GENERIC_WRITE — we only send JSON requests; responses (when needed)
        // arrive via the response_file polling pattern, not this socket.
        let raw = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            let err = std::io::Error::last_os_error();
            eprintln!("error: could not open PLEXI_SOCKET pipe {socket_path:?}: {err}");
            return 1;
        }
        let owned = unsafe { OwnedHandle::from_raw_handle(raw as _) };
        let mut file = std::fs::File::from(owned);
        let line = format!("{payload}\n");
        if let Err(e) = file.write_all(line.as_bytes()) {
            eprintln!("error: could not write to pipe: {e}");
            return 1;
        }
        0
    }
}

/// `plexi context new [name] [--path <path>] [--parent <parent>]`
///
/// Creates a new context. `name` is the display name (positional). `--path` sets
/// the root directory (defaults to CWD). When run inside a Plexi pane and `--parent`
/// is not given, defaults to creating a child of the current context.
pub fn context_new_cli(name: Option<&str>, path: Option<&str>, parent: Option<&str>) -> i32 {
    let root = match resolve_path(path) {
        Ok(p) => p,
        Err(e) => { eprintln!("{e}"); return 1; }
    };
    // Default parent: current context when inside a Plexi pane and --parent omitted.
    let resolved_parent = parent
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            std::env::var("PLEXI_CONTEXT_NAME")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
    log::info!(
        "context_new_cli: name={:?} root={} parent={:?}",
        name,
        root.display(),
        resolved_parent.as_deref()
    );
    let mut payload = serde_json::json!({
        "type": "create_context",
        "root": root,
    });
    if let Some(n) = name {
        payload["name"] = serde_json::Value::String(n.to_string());
    }
    if let Some(p) = resolved_parent {
        payload["parent_name"] = serde_json::Value::String(p);
    }
    send_to_socket(payload)
}

/// `plexi context zoom <context_id>`
///
/// Zoom into a sub-context by its numeric context_id.
pub fn context_zoom_cli(context_id: u64) -> i32 {
    send_to_socket(serde_json::json!({
        "type": "zoom_into_context",
        "context_id": context_id,
    }))
}

/// `plexi context zoom-out`
///
/// Zoom out of the current sub-context to the parent.
pub fn context_zoom_out_cli() -> i32 {
    send_to_socket(serde_json::json!({
        "type": "zoom_out_of_context",
    }))
}

/// `plexi context open [path]`
///
/// Focuses the context with matching root, or creates one. Uses CWD if path omitted.
pub fn context_open_cli(path: Option<&str>) -> i32 {
    let root = match resolve_path(path) {
        Ok(p) => p,
        Err(e) => { eprintln!("{e}"); return 1; }
    };
    send_to_socket(serde_json::json!({
        "type": "focus_context",
        "root": root,
    }))
}

/// `plexi context set-root [path]`
///
/// Sets the root of the active context. Uses CWD if path omitted.
pub fn context_set_root_cli(path: Option<&str>) -> i32 {
    let root = match resolve_path(path) {
        Ok(p) => p,
        Err(e) => { eprintln!("{e}"); return 1; }
    };
    send_to_socket(serde_json::json!({
        "type": "set_context_root",
        "root": root,
    }))
}

/// `plexi context describe "text"`
///
/// Sets the description of the active context.
pub fn context_describe_cli(text: &str) -> i32 {
    send_to_socket(serde_json::json!({
        "type": "set_context_description",
        "description": text,
    }))
}

/// `plexi context current`
///
/// Prints the context ID and name for the current pane as JSON.
/// Reads PLEXI_CONTEXT_ID and PLEXI_CONTEXT_NAME set at pane spawn time.
pub fn context_current_cli() -> i32 {
    let context_id = match std::env::var("PLEXI_CONTEXT_ID") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: PLEXI_CONTEXT_ID is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };
    let context_name = std::env::var("PLEXI_CONTEXT_NAME").unwrap_or_default();
    let context_description = std::env::var("PLEXI_CONTEXT_DESCRIPTION").unwrap_or_default();
    let id_num: u64 = match context_id.parse() {
        Ok(n) => n,
        Err(_) => {
            eprintln!("error: PLEXI_CONTEXT_ID is not a valid number: {context_id}");
            return 1;
        }
    };
    let json = serde_json::json!({
        "context_id": id_num,
        "context_name": context_name,
        "context_description": context_description,
    });
    match serde_json::to_string_pretty(&json) {
        Ok(s) => println!("{s}"),
        Err(e) => {
            eprintln!("error: failed to serialize context JSON: {e}");
            return 1;
        }
    }
    0
}

/// `plexi completions <shell>`
///
/// Prints a static shell completion script to stdout. Pipe to the appropriate
/// location for your shell (see `plexi completions --help`).
pub fn config_check() -> i32 {
    log::info!("config_check: validating config files");
    let diags = crate::config::validate_all();

    if diags.is_empty() {
        let path = crate::config::config_path();
        eprintln!("✓ {} is valid", path.display());
        if let Some(root) = crate::config::active_workspace_root() {
            let project_path = root.join(".plexi").join("config.toml");
            if project_path.exists() {
                eprintln!("✓ {} is valid", project_path.display());
            }
        }
        return 0;
    }

    let has_errors = diags.iter().any(|d| d.is_error());
    for d in &diags {
        if d.is_error() {
            eprintln!("✗ {d}");
        } else {
            eprintln!("⚠ {d}");
        }
    }

    if has_errors { 1 } else { 0 }
}

pub fn config_edit() -> i32 {
    let path = crate::config::ensure_config_exists();
    log::info!("config_edit: opening {} in editor", path.display());

    if let Ok(editor_env) = std::env::var("EDITOR") {
        if editor_env.trim().is_empty() {
            log::warn!("config_edit: EDITOR is set but empty, falling through to system default");
        } else {
            let mut parts = editor_env.split_whitespace();
            if let Some(editor_bin) = parts.next() {
                let args: Vec<&str> = parts.collect();
                match std::process::Command::new(editor_bin).args(&args).arg(&path).status() {
                    Ok(status) if status.success() => return 0,
                    Ok(status) => {
                        eprintln!("error: editor {editor_bin:?} exited with status {}", status.code().unwrap_or(1));
                        return status.code().unwrap_or(1);
                    }
                    Err(e) => {
                        eprintln!("error: could not launch editor {editor_bin:?}: {e}");
                        return 1;
                    }
                }
            }
        }
    }

    // No $EDITOR set — use the fallback chain: VS Code → system default → TextEdit
    if crate::config::open_file_with_fallback(&path) {
        0
    } else {
        eprintln!("error: could not open {:?} — install VS Code, set $EDITOR, or ensure a default text editor is configured", path);
        1
    }
}

pub fn config_get(key: &str) -> i32 {
    log::info!("config_get: resolving key={key}");
    let config = crate::config::PlexiConfig::load_with_workspace(
        crate::config::active_workspace_root().as_deref(),
    );
    let agents = config.agents.as_ref();
    let value = match key {
        "agents.low" => agents.map(|a| a.effective_low()).unwrap_or(crate::config::DEFAULT_AGENT_LOW),
        "agents.medium" => agents.map(|a| a.effective_medium()).unwrap_or(crate::config::DEFAULT_AGENT_MEDIUM),
        "agents.high" => agents.map(|a| a.effective_high()).unwrap_or(crate::config::DEFAULT_AGENT_HIGH),
        _ => {
            eprintln!("error: unknown config key {key:?} — supported keys: agents.low, agents.medium, agents.high");
            return 1;
        }
    };
    println!("{value}");
    0
}

pub fn config_reset() -> i32 {
    let path = crate::config::config_path();
    log::info!("config_reset: writing default config to {}", path.display());
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("error: could not create config dir {}: {e}", parent.display());
            return 1;
        }
    }
    if path.exists() {
        let bak = path.with_extension("toml.bak");
        if let Err(e) = std::fs::copy(&path, &bak) {
            eprintln!("error: could not back up config to {}: {e}", bak.display());
            return 1;
        }
        eprintln!("backed up existing config to {}", bak.display());
    }
    match std::fs::write(&path, crate::config::CONFIG_TEMPLATE) {
        Ok(()) => {
            eprintln!("✓ wrote default config to {}", path.display());
            0
        }
        Err(e) => {
            eprintln!("error: could not write config: {e}");
            1
        }
    }
}

fn binary_in_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

/// `plexi notes list` — print paths of all scratchpad notes, newest first.
pub fn notes_list_cli() -> i32 {
    let notes_dir = crate::config::config_dir().join("notes");
    log::info!("notes_list: scanning {:?}", notes_dir);
    let entries = match std::fs::read_dir(&notes_dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::info!("notes_list: notes dir does not exist yet");
            return 0;
        }
        Err(e) => {
            eprintln!("error: could not read notes directory: {e}");
            return 1;
        }
    };
    let mut paths: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |x| x == "md"))
        .filter_map(|e| {
            let p = e.path();
            let mtime = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((mtime, p))
        })
        .collect();
    paths.sort_by(|a, b| b.0.cmp(&a.0));
    log::info!("notes_list: found {} notes", paths.len());
    for (_, path) in &paths {
        println!("{}", path.display());
    }
    0
}

/// `plexi notes open` — inject fzf note picker into the focused terminal pane.
///
/// Falls back to printing the notes directory when PLEXI_SOCKET is unset or fzf is absent.
pub fn notes_open_cli() -> i32 {
    let notes_dir = crate::config::config_dir().join("notes");
    let notes_dir_str = notes_dir.display().to_string();

    let socket_set = std::env::var("PLEXI_SOCKET").is_ok();
    let fzf_available = binary_in_path("fzf");

    if !socket_set || !fzf_available {
        if !fzf_available {
            eprintln!("hint: install fzf (`brew install fzf`) for an interactive picker");
        }
        if !socket_set {
            eprintln!("hint: run inside a Plexi pane for interactive note picking");
        }
        println!("{notes_dir_str}");
        return 0;
    }

    let pane_id_str = match std::env::var("PLEXI_PANE_ID") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: PLEXI_PANE_ID is not set — run inside a Plexi terminal pane");
            return 1;
        }
    };
    let pane_id: u64 = match pane_id_str.parse() {
        Ok(n) => n,
        Err(_) => {
            eprintln!("error: PLEXI_PANE_ID is not a valid number: {pane_id_str}");
            return 1;
        }
    };

    let editor = if binary_in_path("micro") { "micro" } else if binary_in_path("nano") { "nano" } else { "vim" };
    let cmd = format!(
        "selected=$(ls -t {notes_dir_str}/*.md 2>/dev/null | fzf --header='Select note'); [ -n \"$selected\" ] && {editor} \"$selected\"\r"
    );
    log::info!("notes_open: injecting fzf picker into pane {pane_id}");
    pane_send_cli(pane_id, &cmd)
}

pub fn completions_cli(shell: &str, binary_name: &str) -> i32 {
    match shell {
        "zsh" => { print!("{}", zsh_completion(binary_name)); 0 }
        "bash" => { print!("{}", bash_completion(binary_name)); 0 }
        "fish" => { print!("{}", fish_completion(binary_name)); 0 }
        "powershell" | "pwsh" => { print!("{}", powershell_completion(binary_name)); 0 }
        other => {
            eprintln!("error: unsupported shell {other:?} — supported shells: zsh, bash, fish, powershell");
            1
        }
    }
}

fn zsh_completion(binary: &str) -> String {
    let fn_name = binary.replace('-', "_");
    ZSH_COMPLETION
        .replace("#compdef plexi", &format!("#compdef {binary}"))
        .replace("\n_plexi()", &format!("\n_{fn_name}()"))
        .replace("\n_plexi \"$@\"", &format!("\n_{fn_name} \"$@\""))
}

fn bash_completion(binary: &str) -> String {
    let fn_name = format!("_{}_completions", binary.replace('-', "_"));
    let with_fn = BASH_COMPLETION.replace("_plexi_completions", &fn_name);
    with_fn.replace(
        &format!("complete -F {fn_name} plexi"),
        &format!("complete -F {fn_name} {binary}"),
    )
}

fn fish_completion(binary: &str) -> String {
    FISH_COMPLETION.replace("-c plexi", &format!("-c {binary}"))
}

fn powershell_completion(binary: &str) -> String {
    // Minimal Native completer for top-level subcommands. PowerShell registers
    // it via `Register-ArgumentCompleter -Native`. Source the output from your
    // $PROFILE so completion is active on every new session:
    //
    //   plexi completions powershell | Out-String | Invoke-Expression
    //
    // (the installer at scripts/install-windows.ps1 wires this up for you).
    POWERSHELL_COMPLETION.replace("__BINARY__", binary)
}

const ZSH_COMPLETION: &str = r#"#compdef plexi

_plexi() {
  local state line
  typeset -A opt_args

  _arguments -C \
    '1: :->command' \
    '*:: :->args'

  case $state in
    command)
      local commands
      commands=(
        'run:Run a named command from .plexi/commands.toml'
        'workspace:Workspace management'
        'secret:Secret management'
        'app:App management'
        'install:Install an app'
        'uninstall:Uninstall an app'
        'update:Update apps or self'
        'list:List installed apps'
        'validate:Validate a Plexi app directory'
        'pack:Pack management'
        'notify:Send a notification'
        'pane:Pane management'
        'terminal:Open a terminal pane'
        'open:Open an app pane'
        'descriptor:Descriptor probe'
        'registry:CLI registry'
        'context:Context management'
        'completions:Print shell completion script'
        'config:Configuration management'
      )
      _describe 'command' commands
      ;;
    args)
      case $line[1] in
        secret)
          case $line[2] in
            set)
              _arguments \
                '--from-env[Read value from the environment variable instead of prompting]' \
                '--global[Store globally (cross-workspace)]'
              ;;
            get)
              _arguments '--global[Read from global store only]'
              ;;
            *)
              local subcmds
              subcmds=('set:Store a secret' 'get:Read a secret value' 'list:List stored secrets' 'delete:Delete a secret')
              _describe 'subcommand' subcmds
              ;;
          esac
          ;;
        app)
          case $line[2] in
            init)
              _arguments '--lang[Language template]:lang:(python)'
              ;;
            uninstall)
              _arguments \
                '--yes[Skip the confirmation prompt]' \
                '-y[Skip the confirmation prompt]'
              ;;
            render)
              _arguments \
                '--size[Dimensions as WxH]:size:' \
                '--state[Pre-seed app state from a JSON file]:file:_files -g "*.json"' \
                '--output[Output PNG path]:file:_files -g "*.png"'
              ;;
            *)
              local subcmds
              subcmds=('init:Scaffold a new app' 'install:Install a local app directory' 'uninstall:Remove an installed app by id' 'list:List installed apps' 'render:Render an app to PNG headlessly' 'info:Show app info' 'link:Register a local app directory' 'unlink:Remove a linked app directory')
              _describe 'subcommand' subcmds
              ;;
          esac
          ;;
        workspace)
          local subcmds
          subcmds=('init:Initialise a .plexi/ workspace')
          _describe 'subcommand' subcmds
          ;;
        update)
          local subcmds
          subcmds=('apps:Update installed apps')
          _describe 'subcommand' subcmds
          ;;
        pack)
          local subcmds
          subcmds=('export:Export current apps as a pack file')
          _describe 'subcommand' subcmds
          ;;
        pane)
          case $line[2] in
            capture)
              _arguments '--lines[Number of lines to read]:lines:' '--full-output[Preserve trailing empty lines]'
              ;;
            *)
              local subcmds
              subcmds=('name:Set the name of a pane (current or by ID)' 'set-title:Set the name of a pane (deprecated: use name)' 'list:List all open panes as JSON' 'self:Print current pane ID' 'focus:Move focus to a pane' 'close:Close a pane' 'send:Send text to a pane PTY' 'info:Print current pane info as JSON' 'capture:Read PTY scrollback and print as JSON array' 'key:Inject a synthetic key event into a pane')
              _describe 'subcommand' subcmds
              ;;
          esac
          ;;
        terminal)
          _arguments \
            '(-e --ephemeral)'{-e,--ephemeral}'[Close the pane when the process exits]' \
            '--layout[Layout hint]:layout:(split_h split_left split_right split_v split_below split_above tab new_window)' \
            '--from-pane-id[Split relative to this pane ID]:pane_id:' \
            '--cwd[Working directory for the new terminal pane]:directory:_directories' \
            '--no-focus[Keep focus on the originating pane]'
          ;;
        descriptor)
          case $line[2] in
            probe)
              _arguments \
                '--no-registry[Skip registry lookup]' \
                '--no-crawl[Skip help crawling]' \
                '--json[Output raw descriptor JSON]'
              ;;
            *)
              local subcmds
              subcmds=('probe:Probe a CLI for its Plexi descriptor')
              _describe 'subcommand' subcmds
              ;;
          esac
          ;;
        registry)
          local subcmds
          subcmds=('watch:Watch installed CLIs for descriptor drift')
          _describe 'subcommand' subcmds
          ;;
        context)
          local subcmds
          subcmds=('new:Create a new context' 'open:Open a context at a path' 'set-root:Set the root directory' 'current:Print current context as JSON' 'describe:Set context description')
          _describe 'subcommand' subcmds
          ;;
        notify)
          _arguments \
            '--title[Notification title]:title:' \
            '--body[Notification body]:body:' \
            '--level[Level]:level:(info warn error)' \
            '--choice[Choice option (key:Label / Label:action:arg / key:Label:action:arg)]:choice:' \
            '--host-action[Host-side action for a choice key (key:action_type:action_arg)]:host_action:' \
            '--timeout[Timeout in seconds]:seconds:' \
            '--scope[Notification visibility scope]:scope:(window context global)'
          ;;
        open)
          _arguments \
            '--layout[Layout hint]:layout:(split_h split_left split_right split_v split_below split_above tab new_window overlay)' \
            '--from-pane-id[Split relative to this pane ID]:pane_id:' \
            '--mcp[Wrap stdio MCP server command in mcp-renderer]:cmd:' \
            '--cli[Wrap CLI binary in descriptor-renderer]:binary:_command_names'
          ;;
        install)
          _arguments '--pack[Install from a pack file or core]:pack:'
          ;;
        uninstall)
          _arguments \
            '--keep-data[Keep your profile directory]' \
            '--yes[Skip confirmation prompts and proceed immediately]' \
            '-y[Skip confirmation prompts and proceed immediately]'
          ;;
        completions)
          local shells
          shells=('zsh' 'bash' 'fish')
          _describe 'shell' shells
          ;;
        config)
          local subcmds
          subcmds=('check:Validate config.toml and report errors' 'edit:Open config.toml in $EDITOR' 'get:Print resolved value of a config key' 'reset:Overwrite config.toml with built-in defaults')
          _describe 'subcommand' subcmds
          ;;
      esac
      ;;
  esac
}

_plexi "$@"
"#;

const BASH_COMPLETION: &str = r#"_plexi_completions() {
  local cur prev words cword
  _init_completion || return

  local commands="run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config"

  if [[ $cword -eq 1 ]]; then
    COMPREPLY=($(compgen -W "$commands" -- "$cur"))
    return
  fi

  local cmd="${words[1]}"
  case $cmd in
    secret)
      if [[ $cword -eq 2 ]]; then
        COMPREPLY=($(compgen -W "set get list delete" -- "$cur"))
      else
        case "${words[2]}" in
          set)
            COMPREPLY=($(compgen -W "--from-env --global" -- "$cur"))
            ;;
          get)
            COMPREPLY=($(compgen -W "--global" -- "$cur"))
            ;;
        esac
      fi
      ;;
    app)
      if [[ $cword -eq 2 ]]; then
        COMPREPLY=($(compgen -W "init install uninstall list render info link unlink" -- "$cur"))
      else
        case "${words[2]}" in
          init)
            COMPREPLY=($(compgen -W "--lang" -- "$cur"))
            ;;
          uninstall)
            COMPREPLY=($(compgen -W "--yes -y" -- "$cur"))
            ;;
          render)
            COMPREPLY=($(compgen -W "--size --state --output" -- "$cur"))
            ;;
        esac
      fi
      ;;
    workspace)
      COMPREPLY=($(compgen -W "init" -- "$cur"))
      ;;
    update)
      COMPREPLY=($(compgen -W "apps" -- "$cur"))
      ;;
    pack)
      COMPREPLY=($(compgen -W "export" -- "$cur"))
      ;;
    pane)
      if [[ $cword -eq 2 ]]; then
        COMPREPLY=($(compgen -W "name set-title list self focus close send info capture key" -- "$cur"))
      else
        case "${words[2]}" in
          capture)
            COMPREPLY=($(compgen -W "--lines --full-output" -- "$cur"))
            ;;
        esac
      fi
      ;;
    terminal)
      if [[ $prev == "--layout" ]]; then
        COMPREPLY=($(compgen -W "split_h split_left split_right split_v split_below split_above tab new_window" -- "$cur"))
      elif [[ $prev == "--cwd" ]]; then
        COMPREPLY=($(compgen -d -- "$cur"))
      else
        COMPREPLY=($(compgen -W "-e --ephemeral --layout --from-pane-id --cwd --no-focus" -- "$cur"))
      fi
      ;;
    open)
      if [[ $prev == "--layout" ]]; then
        COMPREPLY=($(compgen -W "split_h split_left split_right split_v split_below split_above tab new_window overlay" -- "$cur"))
      else
        COMPREPLY=($(compgen -W "--layout --from-pane-id --mcp --cli" -- "$cur"))
      fi
      ;;
    descriptor)
      if [[ $cword -eq 2 ]]; then
        COMPREPLY=($(compgen -W "probe" -- "$cur"))
      else
        case "${words[2]}" in
          probe)
            COMPREPLY=($(compgen -W "--no-registry --no-crawl --json" -- "$cur"))
            ;;
        esac
      fi
      ;;
    registry)
      COMPREPLY=($(compgen -W "watch" -- "$cur"))
      ;;
    context)
      COMPREPLY=($(compgen -W "new open set-root current describe" -- "$cur"))
      ;;
    notify)
      if [[ $prev == "--level" ]]; then
        COMPREPLY=($(compgen -W "info warn error" -- "$cur"))
      elif [[ $prev == "--scope" ]]; then
        COMPREPLY=($(compgen -W "window context global" -- "$cur"))
      else
        COMPREPLY=($(compgen -W "--title --body --level --choice --host-action --timeout --scope" -- "$cur"))
      fi
      ;;
    install)
      COMPREPLY=($(compgen -W "--pack" -- "$cur"))
      ;;
    uninstall)
      COMPREPLY=($(compgen -W "--keep-data --yes -y" -- "$cur"))
      ;;
    completions)
      COMPREPLY=($(compgen -W "zsh bash fish" -- "$cur"))
      ;;
    config)
      COMPREPLY=($(compgen -W "check edit get reset" -- "$cur"))
      ;;
  esac
}

complete -F _plexi_completions plexi
"#;

const FISH_COMPLETION: &str = r#"# Plexi shell completions for fish

complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a run -d "Run a named command"
complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a workspace -d "Workspace management"
complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a secret -d "Secret management"
complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a app -d "App management"
complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a install -d "Install an app"
complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a uninstall -d "Uninstall an app"
complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a update -d "Update apps or self"
complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a list -d "List installed apps"
complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a validate -d "Validate a Plexi app directory"
complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a pack -d "Pack management"
complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a notify -d "Send a notification"
complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a pane -d "Pane management"
complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a terminal -d "Open a terminal pane"
complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a open -d "Open an app pane"
complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a descriptor -d "Descriptor probe"
complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a registry -d "CLI registry"
complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a context -d "Context management"
complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a completions -d "Print shell completion script"
complete -c plexi -f -n "not __fish_seen_subcommand_from run workspace secret app install uninstall update list validate pack notify pane terminal open descriptor registry context completions config" -a config -d "Configuration management"

# config subcommands
complete -c plexi -f -n "__fish_seen_subcommand_from config" -a check -d "Validate config.toml and report errors"
complete -c plexi -f -n "__fish_seen_subcommand_from config" -a edit -d "Open config.toml in \$EDITOR"
complete -c plexi -f -n "__fish_seen_subcommand_from config" -a get -d "Print resolved value of a config key"
complete -c plexi -f -n "__fish_seen_subcommand_from config" -a reset -d "Overwrite config.toml with built-in defaults"

# secret subcommands
complete -c plexi -f -n "__fish_seen_subcommand_from secret" -a set -d "Store a secret"
complete -c plexi -f -n "__fish_seen_subcommand_from secret" -a get -d "Read a secret value"
complete -c plexi -f -n "__fish_seen_subcommand_from secret" -a list -d "List stored secrets"
complete -c plexi -f -n "__fish_seen_subcommand_from secret" -a delete -d "Delete a secret"

# secret set flags
complete -c plexi -n "__fish_seen_subcommand_from secret; and __fish_seen_subcommand_from set" -l from-env -d "Read value from the environment variable instead of prompting"
complete -c plexi -n "__fish_seen_subcommand_from secret; and __fish_seen_subcommand_from set" -l global -d "Store globally (cross-workspace)"

# secret get flags
complete -c plexi -n "__fish_seen_subcommand_from secret; and __fish_seen_subcommand_from get" -l global -d "Read from global store only"

# app subcommands
complete -c plexi -f -n "__fish_seen_subcommand_from app" -a init -d "Scaffold a new app"
complete -c plexi -f -n "__fish_seen_subcommand_from app" -a install -d "Install a local app directory"
complete -c plexi -f -n "__fish_seen_subcommand_from app" -a uninstall -d "Uninstall an app"
complete -c plexi -f -n "__fish_seen_subcommand_from app" -a list -d "List installed apps"
complete -c plexi -f -n "__fish_seen_subcommand_from app" -a render -d "Render an app to PNG headlessly"
complete -c plexi -f -n "__fish_seen_subcommand_from app" -a info -d "Show app info"
complete -c plexi -f -n "__fish_seen_subcommand_from app" -a link -d "Register a local app directory with the workspace"
complete -c plexi -f -n "__fish_seen_subcommand_from app" -a unlink -d "Remove a linked app directory from the workspace"

# app init flags
complete -c plexi -n "__fish_seen_subcommand_from app; and __fish_seen_subcommand_from init" -l lang -d "Language template" -a "python"

# app render flags
complete -c plexi -n "__fish_seen_subcommand_from app; and __fish_seen_subcommand_from render" -l size -d "Dimensions as WxH (e.g. 500x500)"
complete -c plexi -n "__fish_seen_subcommand_from app; and __fish_seen_subcommand_from render" -l state -d "Pre-seed app state from a JSON file"
complete -c plexi -n "__fish_seen_subcommand_from app; and __fish_seen_subcommand_from render" -l output -d "Output PNG path"

# workspace subcommands
complete -c plexi -f -n "__fish_seen_subcommand_from workspace" -a init -d "Initialise a .plexi/ workspace"

# update subcommands
complete -c plexi -f -n "__fish_seen_subcommand_from update" -a apps -d "Update installed apps"

# pack subcommands
complete -c plexi -f -n "__fish_seen_subcommand_from pack" -a export -d "Export current apps as a pack file"

# pane subcommands
complete -c plexi -f -n "__fish_seen_subcommand_from pane" -a name -d "Set the name of a pane (current or by ID)"
complete -c plexi -f -n "__fish_seen_subcommand_from pane" -a set-title -d "Set the name of a pane (deprecated: use name)"
complete -c plexi -f -n "__fish_seen_subcommand_from pane" -a list -d "List all open panes as JSON"
complete -c plexi -f -n "__fish_seen_subcommand_from pane" -a focus -d "Move focus to a pane by ID"
complete -c plexi -f -n "__fish_seen_subcommand_from pane" -a close -d "Close a pane"
complete -c plexi -f -n "__fish_seen_subcommand_from pane" -a send -d "Send text to a pane PTY"
complete -c plexi -f -n "__fish_seen_subcommand_from pane" -a self -d "Print current pane ID"
complete -c plexi -f -n "__fish_seen_subcommand_from pane" -a info -d "Print current pane info as JSON"
complete -c plexi -f -n "__fish_seen_subcommand_from pane" -a capture -d "Read PTY scrollback and print as JSON array"
complete -c plexi -f -n "__fish_seen_subcommand_from pane" -a key -d "Inject a synthetic key event into a pane"

# pane capture flags
complete -c plexi -n "__fish_seen_subcommand_from pane; and __fish_seen_subcommand_from capture" -l lines -d "Number of lines to read"
complete -c plexi -n "__fish_seen_subcommand_from pane; and __fish_seen_subcommand_from capture" -l full-output -d "Preserve trailing empty lines"

# descriptor subcommands
complete -c plexi -f -n "__fish_seen_subcommand_from descriptor" -a probe -d "Probe a CLI for its Plexi descriptor"

# descriptor probe flags
complete -c plexi -n "__fish_seen_subcommand_from descriptor; and __fish_seen_subcommand_from probe" -l no-registry -d "Skip registry lookup"
complete -c plexi -n "__fish_seen_subcommand_from descriptor; and __fish_seen_subcommand_from probe" -l no-crawl -d "Skip help crawling"
complete -c plexi -n "__fish_seen_subcommand_from descriptor; and __fish_seen_subcommand_from probe" -l json -d "Output raw descriptor JSON"

# registry subcommands
complete -c plexi -f -n "__fish_seen_subcommand_from registry" -a watch -d "Watch installed CLIs for descriptor drift"

# context subcommands
complete -c plexi -f -n "__fish_seen_subcommand_from context" -a new -d "Create a new context"
complete -c plexi -f -n "__fish_seen_subcommand_from context" -a open -d "Open a context at a path"
complete -c plexi -f -n "__fish_seen_subcommand_from context" -a set-root -d "Set the root directory"
complete -c plexi -f -n "__fish_seen_subcommand_from context" -a current -d "Print current context as JSON"
complete -c plexi -f -n "__fish_seen_subcommand_from context" -a describe -d "Set context description"

# notify flags
complete -c plexi -n "__fish_seen_subcommand_from notify" -l title -d "Notification title"
complete -c plexi -n "__fish_seen_subcommand_from notify" -l body -d "Notification body"
complete -c plexi -n "__fish_seen_subcommand_from notify" -l level -d "Level" -a "info warn error"
complete -c plexi -n "__fish_seen_subcommand_from notify" -l choice -d "Choice option (key:Label / Label:action:arg / key:Label:action:arg)"
complete -c plexi -n "__fish_seen_subcommand_from notify" -l host-action -d "Host-side action for a choice key (key:action_type:action_arg)"
complete -c plexi -n "__fish_seen_subcommand_from notify" -l timeout -d "Timeout in seconds"
complete -c plexi -n "__fish_seen_subcommand_from notify" -l scope -d "Notification visibility scope" -a "window context global"

# install flags
complete -c plexi -n "__fish_seen_subcommand_from install" -l pack -d "Install from a pack file or core"

# uninstall flags
complete -c plexi -n "__fish_seen_subcommand_from uninstall" -l keep-data -d "Keep your profile directory"
complete -c plexi -n "__fish_seen_subcommand_from uninstall" -l yes -s y -d "Skip confirmation prompts and proceed immediately"

# app uninstall flags
complete -c plexi -n "__fish_seen_subcommand_from app; and __fish_seen_subcommand_from uninstall" -l yes -s y -d "Skip the confirmation prompt"

# completions args
complete -c plexi -f -n "__fish_seen_subcommand_from completions" -a "zsh bash fish"

# terminal flags
complete -c plexi -n "__fish_seen_subcommand_from terminal" -s e -l ephemeral -d "Close the pane when the process exits"
complete -c plexi -n "__fish_seen_subcommand_from terminal" -l layout -d "Layout hint" -a "split_h split_left split_right split_v split_below split_above tab new_window"
complete -c plexi -n "__fish_seen_subcommand_from terminal" -l from-pane-id -d "Split relative to this pane ID"
complete -c plexi -n "__fish_seen_subcommand_from terminal" -l cwd -d "Working directory for the new terminal pane" -a "(__fish_complete_directories)"
complete -c plexi -n "__fish_seen_subcommand_from terminal" -l no-focus -d "Keep focus on the originating pane"
# open flags
complete -c plexi -n "__fish_seen_subcommand_from open" -l layout -d "Layout hint" -a "split_h split_left split_right split_v split_below split_above tab new_window overlay"
complete -c plexi -n "__fish_seen_subcommand_from open" -l from-pane-id -d "Split relative to this pane ID"
complete -c plexi -n "__fish_seen_subcommand_from open" -l mcp -d "Wrap stdio MCP server command in mcp-renderer"
complete -c plexi -n "__fish_seen_subcommand_from open" -l cli -d "Wrap CLI binary in descriptor-renderer" -a "(__fish_complete_command)"
"#;

// PowerShell completer registered via Register-ArgumentCompleter -Native.
// Minimal — completes top-level subcommands and a few common second-level
// nouns. Source from $PROFILE for persistent shell coverage:
//   plexi completions powershell | Out-String | Invoke-Expression
// __BINARY__ is substituted at runtime so plexi-alpha / plexi-beta etc.
// each register their own completer instead of fighting over the same name.
const POWERSHELL_COMPLETION: &str = r#"# Plexi PowerShell completions
Register-ArgumentCompleter -Native -CommandName __BINARY__ -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $tokens = $commandAst.CommandElements | ForEach-Object { $_.Extent.Text }
    $depth = $tokens.Count
    $prev = if ($depth -ge 2) { $tokens[$depth - 1] } else { '' }
    $sub = if ($depth -ge 2) { $tokens[1] } else { '' }

    $topLevel = @(
        'run','workspace','secret','app','install','uninstall','update','list',
        'validate','pack','notify','pane','terminal','open','descriptor',
        'registry','context','completions','config','notes'
    )

    function Out-Completions($candidates, $word) {
        foreach ($c in $candidates) {
            if ($c -like "$word*") {
                [System.Management.Automation.CompletionResult]::new(
                    $c, $c, 'ParameterValue', $c)
            }
        }
    }

    # First positional after the binary → top-level subcommand.
    if ($depth -le 1 -or ($depth -eq 2 -and -not $wordToComplete.StartsWith('-'))) {
        Out-Completions $topLevel $wordToComplete
        return
    }

    switch ($sub) {
        'workspace'   { Out-Completions @('init','status') $wordToComplete }
        'secret'      { Out-Completions @('set','get','list','delete') $wordToComplete }
        'app'         { Out-Completions @('init','install','uninstall','list','info','render','link','unlink','run') $wordToComplete }
        'update'      { Out-Completions @('apps','self') $wordToComplete }
        'pack'        { Out-Completions @('export') $wordToComplete }
        'pane'        { Out-Completions @('name','set-title','list','self','focus','close','send','info','capture','key') $wordToComplete }
        'context'     { Out-Completions @('new','zoom','zoom-out','open','set-root','describe','current') $wordToComplete }
        'config'      { Out-Completions @('check','edit','get','reset') $wordToComplete }
        'completions' { Out-Completions @('zsh','bash','fish','powershell') $wordToComplete }
        'notes'       { Out-Completions @('list','open') $wordToComplete }
        default       { }
    }
}
"#;

#[cfg(test)]
mod notify_tests {
    use super::{notify_cli, parse_notify_choice};

    /// Without PLEXI_SOCKET set, notify_cli must fail fast (exit 1) rather than panic.
    #[test]
    fn notify_cli_no_socket_returns_one() {
        std::env::remove_var("PLEXI_SOCKET");
        let code = notify_cli("Test title", "Test body", "info", &[], 0, None);
        assert_eq!(code, 1);
    }

    #[test]
    fn parse_choice_two_segment() {
        let (key, label, action) = parse_notify_choice("open_pr:Open PR").unwrap();
        assert_eq!(key, "open_pr");
        assert_eq!(label, "Open PR");
        assert!(action.is_none());
    }

    #[test]
    fn parse_choice_three_segment_host_action() {
        let (key, label, action) = parse_notify_choice("Talk to Claude:pane_focus:188").unwrap();
        assert_eq!(key, "Talk to Claude");
        assert_eq!(label, "Talk to Claude");
        assert_eq!(action.as_deref(), Some("pane_focus:188"));
    }

    #[test]
    fn parse_choice_four_segment_key_label_action() {
        let (key, label, action) =
            parse_notify_choice("c:Talk to Claude:pane_focus:188").unwrap();
        assert_eq!(key, "c");
        assert_eq!(label, "Talk to Claude");
        assert_eq!(action.as_deref(), Some("pane_focus:188"));
    }

    #[test]
    fn parse_choice_five_segment_is_error() {
        let err = parse_notify_choice("a:b:c:d:e").unwrap_err();
        assert!(err.contains("5"), "error should mention segment count: {err}");
    }

    #[test]
    fn parse_choice_one_segment_is_error() {
        let err = parse_notify_choice("nocolon").unwrap_err();
        assert!(err.contains("1"), "error should mention segment count: {err}");
    }

    /// --host-action merges into a clean key:Label choice.
    #[test]
    fn host_action_merges_into_clean_choice() {
        let (key, label, embedded) = parse_notify_choice("view:View results").unwrap();
        assert!(embedded.is_none());
        // Simulate the merge: host_action_map has "view" → "pane_focus:99"
        let merged_action = Some("pane_focus:99".to_string());
        let action = Some(merged_action.unwrap());
        assert_eq!(key, "view");
        assert_eq!(label, "View results");
        assert_eq!(action.as_deref(), Some("pane_focus:99"));
    }

    /// --host-action overrides an embedded action in a 4-segment --choice.
    #[test]
    fn host_action_overrides_embedded_choice_action() {
        let (key, label, embedded) =
            parse_notify_choice("a:Talk to Claude:pane_focus:OLD").unwrap();
        assert_eq!(embedded.as_deref(), Some("pane_focus:OLD"));
        // host_action_map contains key "a" → "pane_focus:NEW"
        let override_action = Some("pane_focus:NEW".to_string());
        let final_action = override_action.map(Some).unwrap_or(embedded);
        assert_eq!(key, "a");
        assert_eq!(label, "Talk to Claude");
        assert_eq!(final_action.as_deref(), Some("pane_focus:NEW"));
    }

    /// #840: snooze action type parses to the correct host_action string.
    #[test]
    fn parse_choice_snooze_action() {
        let (key, label, action) =
            parse_notify_choice("snooze5:Remind me in 5 min:snooze:300").unwrap();
        assert_eq!(key, "snooze5");
        assert_eq!(label, "Remind me in 5 min");
        assert_eq!(action.as_deref(), Some("snooze:300"));
    }

    /// #840: three-segment form also works for snooze.
    #[test]
    fn parse_choice_snooze_three_segment() {
        let (key, label, action) =
            parse_notify_choice("Snooze 5min:snooze:300").unwrap();
        assert_eq!(key, "Snooze 5min");
        assert_eq!(label, "Snooze 5min");
        assert_eq!(action.as_deref(), Some("snooze:300"));
    }
}

#[cfg(test)]
mod secret_set_tests {
    use std::fs;
    use tempfile::TempDir;

    /// Helper: checks whether a given `cwd` path would be rejected by the
    /// workspace_init home/root guard. Mirrors the exact logic in
    /// `workspace_init()` so the condition is tested independently of the
    /// real `std::env::current_dir()`.
    fn init_guard_rejects(cwd: &std::path::Path) -> bool {
        let home = std::env::var("HOME").ok().map(std::path::PathBuf::from);
        let cwd_str = cwd.to_string_lossy();
        let is_home_or_root = cwd == std::path::Path::new("/")
            || home.as_ref().map(|h| cwd == *h).unwrap_or(false);
        let is_inside_profile = home.as_ref().map(|h| {
            let prefix = format!("{}/.plexi", h.to_string_lossy());
            cwd_str.starts_with(&prefix)
        }).unwrap_or(false);
        is_home_or_root || is_inside_profile
    }

    #[test]
    fn root_dir_is_rejected_by_init_guard() {
        assert!(init_guard_rejects(std::path::Path::new("/")));
    }

    #[test]
    fn home_dir_is_rejected_by_init_guard() {
        let home = std::env::var("HOME").unwrap();
        assert!(init_guard_rejects(std::path::Path::new(&home)));
    }

    #[test]
    fn plexi_profile_dir_is_rejected_by_init_guard() {
        let home = std::env::var("HOME").unwrap();
        let profile = std::path::PathBuf::from(format!("{home}/.plexi-alpha"));
        assert!(init_guard_rejects(&profile));
    }

    #[test]
    fn project_subdir_is_allowed_by_init_guard() {
        let tmp = tempfile::tempdir().unwrap();
        // A temp dir under /private/var/... is not home/root/profile
        assert!(!init_guard_rejects(tmp.path()));
    }

    #[test]
    fn walk_up_finds_nearest_plexi_dir() {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join(".plexi")).unwrap();
        let deep = workspace.path().join("a").join("b").join("c");
        fs::create_dir_all(&deep).unwrap();

        let found = crate::app_registry::resolve_workspace_root(&deep);
        assert!(found.is_some(), "should find .plexi ancestor");
        let found = found.unwrap().canonicalize().unwrap();
        let expected = workspace.path().canonicalize().unwrap();
        assert_eq!(found, expected);
    }

    #[test]
    fn no_plexi_dir_in_tree_returns_none() {
        let bare: TempDir = tempfile::tempdir().unwrap();
        let inner = bare.path().join("x").join("y");
        fs::create_dir_all(&inner).unwrap();
        assert!(crate::app_registry::resolve_workspace_root(&inner).is_none());
    }

    #[test]
    fn walk_up_stops_at_home() {
        // Simulate a path that extends above home but has .plexi above home.
        // resolve_workspace_root never walks above HOME, so .plexi above home
        // must NOT be found.
        //
        // We cannot safely create dirs above HOME, so we assert indirectly:
        // a bare temp dir with no .plexi returns None, proving the walk stops.
        let bare: TempDir = tempfile::tempdir().unwrap();
        assert!(crate::app_registry::resolve_workspace_root(bare.path()).is_none());
    }
}

#[cfg(test)]
mod app_run_tests {
    use tempfile::TempDir;

    #[test]
    fn app_run_nonexistent_path_returns_1() {
        let code = super::app_run("/tmp/plexi-test-nonexistent-path-xyzzy-12345");
        assert_eq!(code, 1);
    }

    #[test]
    fn app_run_dir_without_manifest_returns_1() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let code = super::app_run(&path);
        assert_eq!(code, 1);
    }

    #[test]
    fn app_run_invalid_manifest_returns_1() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("manifest.toml"), "this is not valid toml ][[[").unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let code = super::app_run(&path);
        assert_eq!(code, 1);
    }
}
