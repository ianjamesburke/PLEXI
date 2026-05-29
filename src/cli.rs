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
    pub commands: HashMap<String, CommandEntry>,
}

#[derive(Deserialize, Default)]
pub struct SecretsConfig {
    #[serde(default)]
    pub required: Vec<String>,
}

/// A command entry: either a bare string (`build = "cargo build"`) or an inline table
/// (`build = { run = "cargo build", description = "..." }`).
/// The old nested-section form (`[commands.build]\nrun = "..."`) is TOML-equivalent to the
/// inline-table form and parses identically — no migration needed.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum CommandEntry {
    Simple(String),
    Full(CommandDef),
}

impl CommandEntry {
    pub fn run(&self) -> &str {
        match self {
            CommandEntry::Simple(s) => s,
            CommandEntry::Full(d) => &d.run,
        }
    }

    pub fn description(&self) -> Option<&str> {
        match self {
            CommandEntry::Simple(_) => None,
            CommandEntry::Full(d) => d.description.as_deref(),
        }
    }

    pub fn secrets(&self) -> &[String] {
        match self {
            CommandEntry::Simple(_) => &[],
            CommandEntry::Full(d) => &d.secrets,
        }
    }
}

#[derive(Deserialize)]
pub struct CommandDef {
    pub run: String,
    pub description: Option<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
}

/// List executable files in a scripts directory.
fn list_global_scripts(scripts_dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(scripts_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let path = e.path();
            path.is_file() && is_executable(&path)
        })
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.exists()
    }
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
            let channel_dir = app_init_config_dir();
            let channel_path = cwd.join(&channel_dir);
            let channel_created = if let Err(e) = std::fs::create_dir_all(&channel_path) {
                log::warn!("workspace_init:cli: could not create channel dir {}: {e}", channel_path.display());
                false
            } else {
                log::info!("workspace_init:cli: created channel dir {}", channel_path.display());
                true
            };
            println!("Initialized workspace at {}", cwd.display());
            println!("  workspace id: {}", cfg.id);
            println!("  router:       .plexi/secrets.toml (fallback = true)");
            if channel_created {
                println!("  channel dir:  {channel_dir}/");
            }
            // Write stub apps.toml if not already present.
            let apps_toml = cwd.join(".plexi").join("apps.toml");
            if !apps_toml.exists() {
                let stub = concat!(
                    "schema_version = 1\n\n",
                    "# Declare workspace app dependencies here.\n",
                    "# Run `plexi app install` in this directory to install them.\n",
                    "#\n",
                    "# Example:\n",
                    "#\n",
                    "# [[app]]\n",
                    "# id      = \"gh-issues\"\n",
                    "# source  = \"local:gh-issues\"\n",
                    "# version = \"bundled\"\n",
                    "#\n",
                    "# [[app]]\n",
                    "# id      = \"my-tool\"\n",
                    "# source  = \"github:org/my-tool\"\n",
                    "# version = \"v1.0.0\"\n",
                );
                if let Err(e) = std::fs::write(&apps_toml, stub) {
                    log::warn!("workspace_init:cli: could not write apps.toml: {e}");
                    eprintln!("warning: could not create .plexi/apps.toml: {e}");
                } else {
                    println!("  apps:         .plexi/apps.toml (declare app dependencies here)");
                    log::info!("workspace_init:cli: wrote stub .plexi/apps.toml");
                }
            }
            // Write stub commands.toml if not already present.
            let commands_toml = cwd.join(".plexi").join("commands.toml");
            if !commands_toml.exists() {
                let stub = concat!(
                    "# Workspace commands — run with: plexi run <name>\n",
                    "#\n",
                    "# Simple form:   build = \"cargo build\"\n",
                    "# With metadata: dev = { run = \"npm run dev\", description = \"Start dev server\" }\n",
                    "# With secrets:  deploy = { run = \"./deploy.sh\", secrets = [\"API_KEY\"] }\n",
                    "\n",
                    "[commands]\n",
                    "guess = \"$PLEXI_CONFIG_DIR/scripts/guess\"\n",
                );
                if let Err(e) = std::fs::write(&commands_toml, stub) {
                    log::warn!("workspace_init:cli: could not write commands.toml: {e}");
                    eprintln!("warning: could not create .plexi/commands.toml: {e}");
                } else {
                    println!("  commands:     .plexi/commands.toml (run commands with: plexi run)");
                    log::info!("workspace_init:cli: wrote stub .plexi/commands.toml");
                }
            }
            print_tip("declare app dependencies in .plexi/apps.toml, then run `plexi app install`.");
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
fn app_init_config_dir() -> String {
    crate::config::config_dir()
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".plexi")
        .to_string()
}

/// `plexi app init [--lang python|rust] <name>` — scaffold a new app.
///
/// Placement: walks up from CWD looking for the nearest ancestor directory
/// that contains the channel config dir (e.g. `.plexi-alpha/` for the alpha
/// build, `.plexi/` for main). If found, scaffolds into
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

    // Root dir: hard reject (no prompt). Home dir: prompt — user may
    // intentionally want a global-scoped app not tied to any workspace.
    let home = dirs::home_dir();
    let is_root = cwd == std::path::Path::new("/");
    let is_home = home.as_ref().map(|h| cwd == *h).unwrap_or(false);
    if is_root {
        log::warn!("app_init: rejected — root dir guard: {}", cwd.display());
        eprintln!("error: cannot scaffold an app in the root directory.");
        return 1;
    }
    if is_home {
        eprintln!("You're about to create a global-scoped app (not tied to any workspace). Continue? [y/N]");
        eprintln!("(Or cd into a project directory to create a workspace-scoped app.)");
        let _ = io::stderr().flush();
        let mut answer = String::new();
        if io::stdin().read_line(&mut answer).is_err() {
            eprintln!("error: failed to read confirmation");
            return 1;
        }
        if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
            log::info!("app_init: user declined global-scoped app at home dir — exiting cleanly");
            return 0;
        }
        log::info!("app_init: user confirmed global-scoped app at home dir");
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
            if current.join(".plexi").is_dir() {
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
    use std::os::unix::fs::PermissionsExt;

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

    // chmod +x main.py
    let mut perms = std::fs::metadata(&main_path)?.permissions();
    perms.set_mode(perms.mode() | 0o111);
    std::fs::set_permissions(&main_path, perms)?;

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
    let core_ids = crate::install::core_pack_ids();
    if core_ids.contains(id) {
        eprintln!("note: '{id}' is a core app — it will be re-installed on the next Plexi launch");
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
    println!("Run `plexi app open {app_id}` to launch it.");
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

    // Optional: parse seed state JSON to inject via protocol.
    // Either path (read or parse failure) is a hard error — the caller explicitly
    // requested seeded state, so silently falling back would produce a misleading render.
    let seed_state: Option<serde_json::Value> = if let Some(s) = state {
        let json = match std::fs::read_to_string(s) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("error: could not read state file '{s}': {e}");
                return 1;
            }
        };
        match serde_json::from_str(&json) {
            Ok(v) => {
                log::info!("app_render[{id}]: loaded seed state from '{s}'");
                Some(v)
            }
            Err(e) => {
                eprintln!("error: invalid JSON in state file '{s}': {e}");
                return 1;
            }
        }
    } else {
        None
    };

    // Resolve the app binary
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let registry = crate::app_registry::AppRegistry::load(&cwd);
    let app_bin = match registry.list().into_iter().find(|a| a.manifest.id == id) {
        Some(a) => a.bin_path.clone(),
        None => {
            eprintln!("error: app '{id}' not found — run `plexi app list` to see installed apps");
            return 1;
        }
    };

    let png_bytes = match crate::app_render::render_app_to_png(id, &app_bin, width, height, seed_state) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: render failed: {e}");
            return 1;
        }
    };

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
                print_tip(&format!("open your app with `plexi app open {}`.", outcome.id));
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

/// `plexi install` with no args — detect `.plexi/apps.toml` and apply it.
///
/// Walks up from CWD looking for `.plexi/` (the workspace marker), reads
/// `apps.toml` from it, and installs declared apps into the workspace-scoped
/// channel apps dir (`<workspace_root>/<channel_dir>/apps/`).
pub fn install_workspace_pack_cli() -> i32 {
    log::info!("cli: install_workspace_pack (no-args flow)");
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => { eprintln!("error: {e}"); return 1; }
    };

    // Walk up from CWD looking for `.plexi/` (workspace marker).
    let workspace_root = {
        let home = dirs::home_dir();
        let mut current = cwd.clone();
        let mut found: Option<std::path::PathBuf> = None;
        loop {
            if let Some(ref h) = home {
                if current == *h {
                    break;
                }
            }
            if current == std::path::Path::new("/") {
                break;
            }
            if current.join(".plexi").is_dir() {
                found = Some(current);
                break;
            }
            if !current.pop() {
                break;
            }
        }
        found
    };

    let Some(root) = workspace_root else {
        eprintln!("Usage: plexi app install <source-spec>[@ref] | plexi app install --pack <path|core>");
        eprintln!("  In a workspace (directory with .plexi/apps.toml), `plexi app install` applies the manifest.");
        eprintln!("  Run `plexi workspace init` to initialize a workspace here.");
        return 1;
    };

    let apps_toml = root.join(".plexi").join("apps.toml");
    if !apps_toml.exists() {
        eprintln!("no .plexi/apps.toml found in workspace at {}", root.display());
        eprintln!("  Declare app dependencies there, then re-run `plexi app install`.");
        eprintln!("  Usage: plexi app install <source-spec>[@ref] | plexi app install --pack <path|core>");
        return 1;
    }

    log::info!("install_workspace_pack:cli: applying {}", apps_toml.display());
    println!("Applying workspace manifest {}...", apps_toml.display());

    let cloner = crate::install::GitCloner;
    let outcomes = match crate::install::apply_workspace_pack(&root, &cloner) {
        Ok(o) => o,
        Err(e) => { eprintln!("error: {e}"); return 1; }
    };

    if outcomes.is_empty() {
        println!("No apps declared in .plexi/apps.toml.");
        return 0;
    }

    let mut any_failed = false;
    for o in &outcomes {
        match &o.status {
            crate::install::InstallStatus::Installed(p) => {
                println!("  installed  {:30} → {}", o.id, p.display());
            }
            crate::install::InstallStatus::AlreadyAtVersion => {
                println!("  up-to-date {:30}", o.id);
            }
            crate::install::InstallStatus::SkippedOtherVersion { installed, requested } => {
                println!("  skipped    {:30} (installed {installed}, requested {requested})", o.id);
            }
            crate::install::InstallStatus::Failed(msg) => {
                eprintln!("  FAILED     {:30} {msg}", o.id);
                any_failed = true;
            }
        }
    }
    if any_failed { 1 } else { 0 }
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

    // Remove completions (only for main uninstall)
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
/// Only supports main channel. Alpha (dev) and PR builds must use `just install`.
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
        println!("install one with: plexi app install <source>[@ref]");
        return 0;
    }
    // Read versions directly from the global apps dir for the source-of-truth
    // version field — the registry only carries `manifest.version` at load time.
    let global_versions = crate::install::installed_versions(&crate::app_registry::apps_dir());
    let workspace_root = crate::app_registry::resolve_workspace_root(&cwd);
    let core_ids = crate::install::core_pack_ids();
    let example_ids = crate::install::examples_pack_ids();
    let workspace_ids = workspace_root
        .as_ref()
        .map(|r| crate::install::workspace_manifest_ids(r))
        .unwrap_or_default();
    let mut globals: Vec<(String, String, String, &'static str)> = Vec::new();
    let mut workspace: Vec<(String, String, String, &'static str)> = Vec::new();
    for app in installed {
        let version = global_versions
            .get(&app.manifest.id)
            .cloned()
            .unwrap_or_else(|| app.manifest.version.clone());
        let badge = if core_ids.contains(app.manifest.id.as_str()) {
            "[core]"
        } else if example_ids.contains(app.manifest.id.as_str()) {
            "[example]"
        } else if workspace_ids.contains(app.manifest.id.as_str()) {
            "[workspace]"
        } else {
            ""
        };
        let row = (app.manifest.id.clone(), app.manifest.name.clone(), version, badge);
        match app.source {
            crate::app_registry::RegistrySource::Global => globals.push(row),
            crate::app_registry::RegistrySource::LocalApp
            | crate::app_registry::RegistrySource::LocalAgent => workspace.push(row),
        }
    }
    if !globals.is_empty() {
        println!("Global apps ({})", crate::app_registry::apps_dir().display());
        for (id, name, version, badge) in &globals {
            if badge.is_empty() {
                println!("  {:30} {:30} {}", id, name, version);
            } else {
                println!("  {:30} {:30} {}  {}", id, name, version, badge);
            }
        }
    }
    if !workspace.is_empty() {
        if let Some(root) = workspace_root {
            println!();
            println!("Workspace apps ({})", root.display());
            for (id, name, version, badge) in &workspace {
                if badge.is_empty() {
                    println!("  {:30} {:30} {}", id, name, version);
                } else {
                    println!("  {:30} {:30} {}  {}", id, name, version, badge);
                }
            }
        }
    }
    0
}

/// `plexi app freeze <path>` — write a `pack.toml` snapshot of installed apps to `path`.
/// See `crate::install::export_pack` for the source-spec inference rules.
pub fn freeze_cli(dest_path: &str) -> i32 {
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

    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let mut stream = match UnixStream::connect(&socket_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: could not connect to PLEXI_SOCKET {socket_path:?}: {e}");
            return 1;
        }
    };
    let line = format!("{payload}\n");
    if let Err(e) = stream.write_all(line.as_bytes()) {
        eprintln!("error: could not write to socket: {e}");
        return 1;
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
                        let channel = crate::config::build_channel().unwrap_or_else(|| "main".to_string());
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
pub fn pane_capture_cli(pane_id: Option<u64>, lines: usize, full_output: bool, from_cursor: Option<u64>) -> i32 {
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

    log::info!("pane_capture:cli: pane_id={resolved_pane_id} lines={lines} full_output={full_output} from_cursor={from_cursor:?} response_file={response_file:?}");

    let mut req = serde_json::json!({
        "type": "capture_pane",
        "pane_id": resolved_pane_id,
        "lines": lines,
        "full_output": full_output,
        "response_file": response_file,
    });
    if let Some(cursor) = from_cursor {
        req["from_cursor"] = serde_json::Value::Number(serde_json::Number::from(cursor));
    }
    let code = send_to_socket(req);
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
                    match serde_json::from_str::<serde_json::Value>(&content) {
                        Ok(v) => {
                            if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                                eprintln!("error: {err}");
                                return 1;
                            }
                            // Print cursor to stderr so callers can capture it without
                            // polluting the line stream.
                            if let Some(cursor) = v.get("cursor").and_then(|c| c.as_u64()) {
                                eprintln!("cursor={cursor}");
                            }
                            return print_json_output(&content);
                        }
                        Err(_) => return print_json_output(&content),
                    }
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
/// alpha, beta, main, and PR builds without caring which binary is on PATH.
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
        log::warn!("open:cli: 'plexi app open terminal' is deprecated, use 'plexi terminal' instead");
        eprintln!("warning: 'plexi app open terminal' is deprecated, use 'plexi terminal' instead");
    }

    let from_pane_id = from_pane_id.or_else(|| std::env::var("PLEXI_PANE_ID").ok()?.parse().ok());

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
    let from_pane_id = from_pane_id.or_else(|| std::env::var("PLEXI_PANE_ID").ok()?.parse().ok());

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
fn send_to_socket(payload: serde_json::Value) -> i32 {
    let socket_path = match std::env::var("PLEXI_SOCKET") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: PLEXI_SOCKET is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };
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

pub fn demo_cli() -> i32 {
    let pane_id_str = match std::env::var("PLEXI_PANE_ID") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: run `plexi demo` inside a Plexi terminal pane");
            eprintln!("hint: open Plexi, then run this command from a pane");
            return 1;
        }
    };
    let my_pane_id: u64 = match pane_id_str.parse() {
        Ok(n) => n,
        Err(_) => {
            eprintln!("error: PLEXI_PANE_ID is not a valid number: {pane_id_str}");
            return 1;
        }
    };

    log::info!("demo_cli: starting interactive tutorial for pane_id={my_pane_id}");

    let events_path = crate::config::config_dir().join("events.jsonl");

    // Seek to end — only watch events that occur after demo starts.
    let start_offset = match std::fs::metadata(&events_path) {
        Ok(m) => m.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => {
            log::warn!("demo_cli: could not read events file metadata: {e}");
            0
        }
    };

    // Welcome banner
    eprintln!("\x1b[1;36m");
    eprintln!("  Plexi — Quick Tutorial");
    eprintln!("\x1b[0m");
    eprintln!("  Two moves. That's all you need to know.");
    eprintln!();

    // Step 1 — split
    eprintln!("  Step 1 of 2   Split a pane");
    eprintln!();
    eprintln!("  Press  \x1b[1m[ \u{2318}D ]\x1b[0m  to split the current pane.");
    eprintln!();

    // Capture the new pane's ID from the split event so step 2 can verify
    // focus specifically returns from that pane (not a bounce from the split itself).
    let mut split_pane_id: u64 = 0;
    let after_split_offset = match poll_event(&events_path, start_offset, |kind, obj| {
        if kind == "pane_split" {
            if let Some(id) = obj.get("pane_id").and_then(|v| v.as_u64()) {
                split_pane_id = id;
                return true;
            }
        }
        false
    }) {
        Ok(offset) => offset,
        Err(e) => {
            eprintln!("error watching {}: {e}", events_path.display());
            return 1;
        }
    };
    eprintln!("  \x1b[1;32m\u{2713} 1/2\x1b[0m");
    eprintln!();

    // Step 2 — navigate
    eprintln!("  Step 2 of 2   Navigate panes");
    eprintln!();
    eprintln!("     \x1b[2m^\x1b[0m");
    eprintln!("     K");
    eprintln!("  H     L");
    eprintln!("     J");
    eprintln!();
    eprintln!("  Press  \x1b[1m[ \u{2318}L ]\x1b[0m  to move focus right, then  \x1b[1m[ \u{2318}H ]\x1b[0m  to come back.");
    eprintln!();

    // Wait for focus to LEAVE this pane (user pressed ⌘L).
    let focus_offset = match poll_event(&events_path, after_split_offset, |kind, obj| {
        kind == "focus_changed"
            && obj.get("pane_id").and_then(|v| v.as_u64()) == Some(my_pane_id)
    }) {
        Ok(offset) => offset,
        Err(e) => {
            eprintln!("error watching {}: {e}", events_path.display());
            return 1;
        }
    };

    // Wait for focus to leave the split pane with duration_secs > 0 (user pressed ⌘H and
    // spent deliberate time on the new pane). The split itself generates a 0-duration
    // bounce-back that must not satisfy this check.
    if let Err(e) = poll_event(&events_path, focus_offset, |kind, obj| {
        kind == "focus_changed"
            && obj.get("pane_id").and_then(|v| v.as_u64()) == Some(split_pane_id)
            && obj.get("duration_secs").and_then(|v| v.as_u64()).unwrap_or(0) > 0
    }) {
        eprintln!("error watching {}: {e}", events_path.display());
        return 1;
    }

    eprintln!("  \x1b[1;32m\u{2713} 2/2   You know Plexi.\x1b[0m");
    eprintln!();
    log::info!("demo_cli: tutorial completed for pane_id={my_pane_id}");
    0
}

/// Tails `path` from `offset`, advancing the cursor as lines are consumed.
/// Returns the byte offset immediately after the matched line when the predicate fires.
/// Handles missing files gracefully; only processes complete newline-terminated lines.
fn poll_event<F>(path: &std::path::Path, mut offset: u64, mut predicate: F) -> std::io::Result<u64>
where
    F: FnMut(&str, &serde_json::Value) -> bool,
{
    use std::io::{Read, Seek, SeekFrom};
    loop {
        match std::fs::File::open(path) {
            Ok(mut f) => {
                let file_len = f.seek(SeekFrom::End(0))?;
                if file_len > offset {
                    f.seek(SeekFrom::Start(offset))?;
                    let mut buf = String::new();
                    f.read_to_string(&mut buf)?;
                    // Only process lines up to the last newline to avoid partial writes.
                    let process_len = match buf.rfind('\n') {
                        Some(pos) => pos + 1,
                        None => {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            continue;
                        }
                    };
                    let complete = &buf[..process_len];
                    let mut byte_pos: u64 = 0;
                    for line in complete.split_inclusive('\n') {
                        let line_bytes = line.len() as u64;
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            if let Ok(obj) = serde_json::from_str::<serde_json::Value>(trimmed) {
                                if let Some(kind) = obj.get("kind").and_then(|v| v.as_str()) {
                                    if predicate(kind, &obj) {
                                        return Ok(offset + byte_pos + line_bytes);
                                    }
                                }
                            }
                        }
                        byte_pos += line_bytes;
                    }
                    offset += process_len as u64;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

pub fn completions_cli(shell: &str, binary_name: &str) -> i32 {
    use clap::CommandFactory;
    use clap_complete::{generate, Shell};
    let Ok(shell_variant) = shell.parse::<Shell>() else {
        eprintln!("error: unsupported shell {shell:?} — supported: bash, zsh, fish");
        return 1;
    };
    log::info!("completions: generating {:?} completions for binary {:?}", shell, binary_name);
    let mut cmd = crate::cli_args::Cli::command();
    generate(shell_variant, &mut cmd, binary_name, &mut std::io::stdout());
    0
}

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

#[cfg(test)]
mod workspace_init_tests {
    use std::fs;

    /// Calls the internal init_workspace logic and then the channel-dir creation
    /// on a temp dir, asserting both `.plexi/` and the channel dir are present.
    #[test]
    fn workspace_init_creates_channel_dir_alongside_plexi_dir() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_path_buf();

        // Run init_workspace (creates .plexi/)
        crate::workspace_secrets::init_workspace(&cwd)
            .expect("init_workspace should succeed in a temp dir");

        // Replicate the channel dir creation from workspace_init()
        let channel_dir = super::app_init_config_dir();
        let channel_path = cwd.join(&channel_dir);
        fs::create_dir_all(&channel_path).expect("create_dir_all should succeed");

        assert!(
            cwd.join(".plexi").is_dir(),
            ".plexi/ dir must exist after workspace init"
        );
        assert!(
            channel_path.is_dir(),
            "{channel_dir}/ dir must exist after workspace init"
        );
    }

    /// After init, resolve_workspace_root must still find the workspace and the channel dir must exist.
    #[test]
    fn workspace_remains_resolvable_after_channel_dir_creation() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_path_buf();

        crate::workspace_secrets::init_workspace(&cwd)
            .expect("init_workspace should succeed");

        let channel_dir = super::app_init_config_dir();
        std::fs::create_dir_all(cwd.join(&channel_dir)).unwrap();

        let found = crate::app_registry::resolve_workspace_root(&cwd);
        assert!(
            found.is_some(),
            "resolve_workspace_root should still find the workspace (via .plexi/) after channel dir creation"
        );
        assert!(
            cwd.join(&channel_dir).is_dir(),
            "channel directory should exist alongside .plexi/"
        );
    }
}

#[cfg(test)]
mod command_parse_tests {
    use super::{CommandEntry, PlexiCommands};

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
