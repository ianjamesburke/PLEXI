use serde::Deserialize;
use std::collections::HashMap;
use std::io::{self, Write};
use std::process::Command;

const APP_ID: &str = "plexi-run";
const COMMANDS_FILE: &str = ".plexi/commands.toml";

// Embed the Python SDK at compile time so `plexi app init` can write it out.
// Source is now sdk/python/plexi_sdk/__init__.py (package layout); the file
// written into scaffolded apps remains plexi_sdk.py for flat single-file import.
const PYTHON_SDK: &str = include_str!("../sdk/python/plexi_sdk/__init__.py");

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

/// Entry point for `plexi run <command_name>`.
/// Returns the exit code.
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

    // Spawn the command via sh -c with secrets injected as env vars
    let mut child_cmd = Command::new("sh");
    child_cmd.arg("-c").arg(&cmd_def.run);
    for (key, value) in &resolved {
        child_cmd.env(key, value);
    }

    match child_cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("error: failed to spawn command: {e}");
            1
        }
    }
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
    match crate::workspace_secrets::init_workspace(&cwd) {
        Ok(cfg) => {
            println!("Initialized workspace at {}", cwd.display());
            println!("  workspace id: {}", cfg.id);
            println!("  router:       .plexi/secrets.toml (fallback = true)");
            0
        }
        Err(e) => {
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

/// `plexi secret set <friendly-name>` — prompt for a value (no echo) and
/// store it under `plexi:<workspace-id>:<friendly-name>`.
pub fn workspace_secret_set(friendly: &str) -> i32 {
    let (root, cfg) = match require_workspace() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    eprint!("Enter value for {friendly}: ");
    let _ = io::stderr().flush();
    let value = match read_secret_from_stdin() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("\nerror: failed to read secret: {e}");
            return 1;
        }
    };
    eprintln!();
    if value.is_empty() {
        eprintln!("error: empty value, nothing stored");
        return 1;
    }
    #[cfg(target_os = "macos")]
    {
        use crate::workspace_secrets::{keychain_workspace_name, MacKeychain, SecretStore};
        let account = keychain_workspace_name(&cfg.id, friendly);
        let store = MacKeychain::new();
        match store.set(&account, &value) {
            Ok(()) => {
                eprintln!(
                    "Stored '{friendly}' for workspace {} ({})",
                    root.display(),
                    cfg.id
                );
                0
            }
            Err(e) => {
                eprintln!("error: keychain write failed: {e}");
                1
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (root, cfg, friendly, value);
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

/// `plexi app init [--lang python|rust] <name>` — scaffold a new app in `.plexi/apps/<name>/`.
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

    let app_dir = cwd.join(".plexi").join("apps").join(name);
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
                println!("  # then open a file in Plexi or run: plexi app launch {name}");
            } else {
                println!("\nNext steps:");
                println!("  edit {}/main.py", app_dir.display());
                println!("  # Plexi will pick it up on next launch (or reload)");
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
        "schema_version = 1\ntype = \"app\"\n\n[app]\nid = \"{name}\"\nname = \"{display}\"\nentry = \"main.py\"\nversion = \"0.1.0\"\ndescription = \"A Plexi app\"\n\n[app.capabilities]\ncapabilities = [\"fs.read\"]\n\n[launch]\nlayout_hint = {{ side = \"right\", split = 0.5 }}\n",
        name = name,
        display = to_title_case(name),
    ))?;

    // plexi_sdk.py — embedded at compile time
    std::fs::write(app_dir.join("plexi_sdk.py"), PYTHON_SDK)?;

    // main.py
    let main_py = format!(
        "#!/usr/bin/env python3\nimport sys, os\nsys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))\nfrom plexi_sdk import App\n\napp = App()\n\n@app.on_render\ndef render(ctx):\n    ctx.rect(0, 0, ctx.width, ctx.height, fill=\"#1e1e2e\")\n    ctx.text(20, 20, \"{display}\", size=16, color=\"#cdd6f4\", bold=True)\n    ctx.text(20, 50, \"Edit main.py to build your app.\", size=13, color=\"#6c7086\")\n\n@app.on_key\ndef on_key(key, mods, emit):\n    pass  # handle key events here\n\napp.run()\n",
        display = to_title_case(name),
    );
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
        "schema_version = 1\ntype = \"app\"\n\n[app]\nid = \"{name}\"\nname = \"{display}\"\nentry = \"bin/plexi-app\"\nversion = \"0.1.0\"\ndescription = \"A Plexi app\"\n\n[app.capabilities]\ncapabilities = [\"fs.read\"]\n\n[launch]\nlayout_hint = {{ side = \"right\", split = 0.5 }}\n",
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

/// `plexi app install <github-shorthand-or-url>` — clone + build an app from GitHub.
pub fn app_install(source: &str) -> i32 {
    let url = if source.starts_with("http://")
        || source.starts_with("https://")
        || source.starts_with("git@")
    {
        source.to_string()
    } else {
        // Treat as github shorthand: "user/repo"
        format!("https://github.com/{source}")
    };

    // Derive app id from repo name (last path segment, strip .git)
    let repo_name = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("app")
        .trim_end_matches(".git");

    let apps_dir = crate::app_registry::apps_dir();
    if let Err(e) = std::fs::create_dir_all(&apps_dir) {
        eprintln!("error: could not create apps dir: {e}");
        return 1;
    }

    let dest = apps_dir.join(repo_name);
    if dest.exists() {
        eprintln!(
            "error: {} already exists. Remove it first to reinstall.",
            dest.display()
        );
        return 1;
    }

    println!("Cloning {url} ...");
    let status = Command::new("git")
        .args(["clone", "--depth", "1", &url, &dest.to_string_lossy()])
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("error: git clone failed (exit {})", s.code().unwrap_or(1));
            return 1;
        }
        Err(e) => {
            eprintln!("error: could not run git: {e}");
            return 1;
        }
    }

    // If the app has a Cargo.toml, build it.
    if dest.join("Cargo.toml").exists() {
        println!("Building Rust app (cargo build --release)...");
        let status = Command::new("cargo")
            .args(["build", "--release"])
            .current_dir(&dest)
            .status();

        match status {
            Ok(s) if s.success() => {
                // Copy the compiled binary to bin/plexi-app
                let bin_dir = dest.join("bin");
                if let Err(e) = std::fs::create_dir_all(&bin_dir) {
                    eprintln!("warning: could not create bin dir: {e}");
                } else {
                    let src_bin = dest.join("target").join("release").join("plexi-app");
                    if src_bin.exists() {
                        if let Err(e) = std::fs::copy(&src_bin, bin_dir.join("plexi-app")) {
                            eprintln!("warning: could not copy binary: {e}");
                        }
                    }
                }
            }
            Ok(s) => {
                eprintln!("error: cargo build failed (exit {})", s.code().unwrap_or(1));
                let _ = std::fs::remove_dir_all(&dest);
                return 1;
            }
            Err(e) => {
                eprintln!("error: could not run cargo: {e}");
                let _ = std::fs::remove_dir_all(&dest);
                return 1;
            }
        }
    } else {
        // Python (or other): chmod +x any executable entry points
        for candidate in ["main.py", "app.py", repo_name] {
            let p = dest.join(candidate);
            if p.exists() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = std::fs::metadata(&p) {
                        let mut perms = meta.permissions();
                        perms.set_mode(perms.mode() | 0o111);
                        let _ = std::fs::set_permissions(&p, perms);
                    }
                }
                break;
            }
        }
    }

    println!("Installed '{repo_name}'. Restart Plexi to load the app.");
    0
}

/// `plexi app uninstall <id>` — remove a globally installed app.
pub fn app_uninstall(id: &str) -> i32 {
    let app_dir = crate::app_registry::apps_dir().join(id);
    if !app_dir.exists() {
        eprintln!("error: app '{id}' not found in global apps dir");
        return 1;
    }
    if let Err(e) = std::fs::remove_dir_all(&app_dir) {
        eprintln!("error: could not remove {}: {e}", app_dir.display());
        return 1;
    }
    println!("Uninstalled '{id}'.");
    0
}

/// `plexi app list` — list installed apps.
pub fn app_list() -> i32 {
    let registry =
        crate::app_registry::AppRegistry::load(&std::env::current_dir().unwrap_or_default());
    let apps = registry.list();
    if apps.is_empty() {
        println!("No apps installed.");
        println!("Install one with: plexi app install <github-user/repo>");
    } else {
        for app in apps {
            println!(
                "{:20} {}  {}",
                app.manifest.id, app.manifest.version, app.manifest.description
            );
        }
    }
    0
}

// ── Top-level package manager subcommands (#308 Phase 2) ──────────────────────

/// `plexi install <source-spec>[@ref]` — clone + place one app into the
/// Returns true if `s` looks like a bare app ID (no scheme prefix, no path separators).
fn is_bare_id(s: &str) -> bool {
    !s.contains(':') && !s.contains('/') && !s.is_empty()
}

/// Fetch the plexi app registry and resolve a bare app ID to a source spec string.
///
/// Registry entries in `ianjamesburke/PLEXI` with a `path` field resolve to `local:<dir>`
/// so the bundled copy is used without a network clone. Third-party repos resolve to
/// `github:owner/repo`.
fn resolve_registry_id(id: &str) -> Result<String, String> {
    const REGISTRY_URL: &str =
        "https://raw.githubusercontent.com/ianjamesburke/plexi-registry/main/registry.json";

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

/// `plexi uninstall <id> [--yes]` — remove a globally installed app after
/// a confirmation prompt (skipped with `--yes`).
pub fn uninstall_cli(id: &str, assume_yes: bool) -> i32 {
    let target_root = crate::app_registry::apps_dir();
    let dest = target_root.join(id);
    if !dest.exists() {
        eprintln!("error: '{id}' is not installed at {}", dest.display());
        return 1;
    }
    if !assume_yes {
        eprint!("Remove {} ? [y/N]: ", dest.display());
        let _ = io::stderr().flush();
        let mut answer = String::new();
        if io::stdin().read_line(&mut answer).is_err() {
            eprintln!("error: failed to read confirmation");
            return 1;
        }
        let trimmed = answer.trim().to_lowercase();
        if trimmed != "y" && trimmed != "yes" {
            eprintln!("aborted");
            return 1;
        }
    }
    match crate::install::uninstall_one(id, &target_root) {
        Ok(()) => {
            println!("uninstalled '{id}'");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
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
        eprintln!("Self-update for beta builds is not yet supported.");
        eprintln!(
            "Download the latest beta from: https://github.com/ianjamesburke/PLEXI/releases"
        );
        return 1;
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
            eprintln!("error: binary does not appear to be inside a .app bundle");
            eprintln!("Self-update requires a bundled installation.");
            return 1;
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
            | crate::app_registry::RegistrySource::LocalAgent => workspace.push(row),
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

/// Entry point for `plexi notify --title <text> --body <text> [--level info|warn|error]
///   [--choice key:Label]... [--timeout N]`.
///
/// With no choices: fire-and-forget (writes queue file, prints "notification queued", exits 0).
/// With choices: writes queue file with `choices` + `response_file`, polls the response file
/// until the user selects an option, prints the chosen key to stdout, exits 0.
/// On timeout: exits 2. On queue write error: exits 1.
pub fn notify_cli(
    title: &str,
    body: &str,
    level: &str,
    choices: &[(String, String)],
    timeout_secs: u64,
) -> i32 {
    let queue_dir = crate::config::config_dir().join("notify-queue");
    if let Err(e) = std::fs::create_dir_all(&queue_dir) {
        eprintln!("error: could not create notify queue: {e}");
        return 1;
    }
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    if choices.is_empty() {
        let file = queue_dir.join(format!("{id}.json"));
        let payload = serde_json::json!({
            "level": level,
            "title": title,
            "body": body,
        });
        if let Err(e) = std::fs::write(&file, payload.to_string()) {
            eprintln!("error: could not write notification: {e}");
            return 1;
        }
        println!("notification queued");
        return 0;
    }

    // Blocking path: create a response file path and poll for it.
    let response_file = crate::config::config_dir().join(format!("notify-response-{id}.txt"));
    let choices_json: Vec<serde_json::Value> = choices
        .iter()
        .map(|(key, label)| serde_json::json!({"key": key, "label": label}))
        .collect();
    let payload = serde_json::json!({
        "level": level,
        "title": title,
        "body": body,
        "choices": choices_json,
        "response_file": response_file.to_string_lossy(),
    });
    let queue_file = queue_dir.join(format!("{id}.json"));
    log::info!(
        "notify:cli: writing queue file {:?} choices={} response_file={:?}",
        queue_file, choices.len(), response_file
    );
    if let Err(e) = std::fs::write(&queue_file, payload.to_string()) {
        eprintln!("error: could not write notification: {e}");
        return 1;
    }
    log::info!("notify:cli: queue file written, polling for response");

    let deadline = if timeout_secs > 0 {
        Some(
            std::time::Instant::now()
                + std::time::Duration::from_secs(timeout_secs),
        )
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

/// `plexi open <type_id> [args...] [--layout=X]`
///
/// Writes a spawn request to the spawn-queue directory. The running Plexi host
/// drains this queue each second and launches the app.
/// Returns 0 on success, 1 on error.
pub fn open_cli(type_id: &str, args: &[String], layout: Option<&str>) -> i32 {
    let queue_dir = crate::config::config_dir().join("spawn-queue");
    if let Err(e) = std::fs::create_dir_all(&queue_dir) {
        eprintln!("error: could not create spawn queue: {e}");
        return 1;
    }
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let payload = serde_json::json!({
        "type_id": type_id,
        "args": args,
        "layout": layout.unwrap_or("split_v"),
    });
    let file = queue_dir.join(format!("{id}.json"));
    if let Err(e) = std::fs::write(&file, payload.to_string()) {
        eprintln!("error: could not write spawn request: {e}");
        return 1;
    }
    println!("queued: open {type_id}");
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

#[cfg(test)]
mod notify_tests {
    use super::notify_cli;

    /// Fire-and-forget path: no choices → exit 0, queue file written.
    #[test]
    fn notify_cli_fire_and_forget_returns_zero() {
        // Uses the real config_dir() path; just verifies the function exits 0
        // without blocking. The queue file creation may fail in sandboxed
        // environments, but the function must not panic.
        let code = notify_cli("Test title", "Test body", "info", &[], 0);
        assert_eq!(code, 0);
    }
}
