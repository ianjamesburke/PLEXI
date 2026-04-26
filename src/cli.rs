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
        "[app]\nid = \"{name}\"\nname = \"{display}\"\nentry = \"main.py\"\nversion = \"0.1.0\"\ndescription = \"A Plexi app\"\n\n[app.capabilities]\ncapabilities = [\"fs.read\"]\n\n[launch]\nlayout_hint = {{ side = \"right\", split = 0.5 }}\n",
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
        "[app]\nid = \"{name}\"\nname = \"{display}\"\nentry = \"bin/plexi-app\"\nversion = \"0.1.0\"\ndescription = \"A Plexi app\"\n\n[app.capabilities]\ncapabilities = [\"fs.read\"]\n\n[launch]\nlayout_hint = {{ side = \"right\", split = 0.5 }}\n",
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
/// channel apps dir. Source spec follows `packs::parse_source_spec`.
pub fn install_cli(spec: &str) -> i32 {
    let (source_str, git_ref) = crate::install::split_source_and_ref(spec);
    let source = match crate::packs::parse_source_spec(&source_str) {
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
                     uninstall first or use `plexi update`",
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

/// `plexi update [<id>]` — git-pull one installed app, or all of them.
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

/// Entry point for `plexi notify --title <text> --body <text> [--level info|warn|error]`.
/// Writes a JSON file to the notify queue dir; the running host polls and ingests it.
pub fn notify_cli(title: &str, body: &str, level: &str) -> i32 {
    let queue_dir = crate::config::config_dir().join("notify-queue");
    if let Err(e) = std::fs::create_dir_all(&queue_dir) {
        eprintln!("error: could not create notify queue: {e}");
        return 1;
    }
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
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
    0
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
        pub stderr: String,
    }

    pub struct RealRunner;

    impl DescriptorRunner for RealRunner {
        fn run(&self, command: &str, args: &[&str]) -> std::io::Result<RunOutput> {
            let out = Command::new(command).args(args).output()?;
            Ok(RunOutput {
                status_success: out.status.success(),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            })
        }
    }

    /// Run `<command> <args...> --plexi`, parse + summarize. Returns the
    /// process exit code suitable for `std::process::exit`.
    pub fn probe<R: DescriptorRunner>(runner: &R, command: &str, args: &[&str]) -> i32 {
        let mut full_args: Vec<&str> = args.to_vec();
        full_args.push("--plexi");

        let output = match runner.run(command, &full_args) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("error: failed to spawn `{command}`: {e}");
                return 1;
            }
        };

        if !output.status_success {
            eprintln!(
                "error: `{command} {}` exited non-zero",
                full_args.join(" ")
            );
            if !output.stderr.is_empty() {
                eprintln!("--- stderr ---\n{}", output.stderr.trim_end());
            }
            return 1;
        }

        let descriptor = match plexi_descriptor::parse(&output.stdout) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("error: descriptor from `{command}` failed to parse:");
                eprintln!("  {e}");
                return 1;
            }
        };

        print_summary(&descriptor);
        0
    }

    fn print_summary(d: &PlexiDescriptor) {
        let icon = d.icon.as_deref().unwrap_or("");
        println!(
            "{}{}{} v{}  (descriptor {})",
            icon,
            if icon.is_empty() { "" } else { " " },
            d.name,
            d.version,
            d.plexi_version
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
        pub stderr: String,
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
                stderr: self.stderr.clone(),
            })
        }
    }
}

#[cfg(test)]
mod descriptor_tests {
    use super::descriptor::*;
    use std::cell::RefCell;

    #[test]
    fn probe_invokes_command_with_plexi_flag() {
        let mock = MockRunner {
            stdout: r#"{
                "plexi_version": "0.1",
                "name": "fake",
                "version": "0.0.1",
                "commands": []
            }"#
            .into(),
            stderr: String::new(),
            success: true,
            captured: RefCell::new(None),
        };
        let code = probe(&mock, "fake-cli", &[]);
        assert_eq!(code, 0);
        let captured = mock.captured.borrow();
        let (cmd, args) = captured.as_ref().expect("runner was invoked");
        assert_eq!(cmd, "fake-cli");
        assert_eq!(args.last().map(|s| s.as_str()), Some("--plexi"));
    }

    #[test]
    fn probe_appends_plexi_after_user_args() {
        let mock = MockRunner {
            stdout: r#"{
                "plexi_version": "0.1",
                "name": "fake",
                "version": "0.0.1",
                "commands": []
            }"#
            .into(),
            stderr: String::new(),
            success: true,
            captured: RefCell::new(None),
        };
        let code = probe(&mock, "fake-cli", &["sub", "--verbose"]);
        assert_eq!(code, 0);
        let captured = mock.captured.borrow();
        let (_, args) = captured.as_ref().expect("runner was invoked");
        assert_eq!(args.as_slice(), &["sub", "--verbose", "--plexi"]);
    }

    #[test]
    fn probe_surfaces_parse_error_with_path() {
        // Mock returns JSON with an unknown top-level field. We assert the
        // probe surfaces a non-zero exit; the specific error message reaches
        // stderr (not assertable cleanly here without a buffer-capture
        // harness — the field-path content is covered by the parser's own
        // `parse_rejects_unknown_top_level_field` test).
        let mock = MockRunner {
            stdout: r#"{
                "plexi_version": "0.1",
                "name": "x",
                "version": "0.0.1",
                "commands": [],
                "rogue": 1
            }"#
            .into(),
            stderr: String::new(),
            success: true,
            captured: RefCell::new(None),
        };
        let code = probe(&mock, "fake-cli", &[]);
        assert_eq!(code, 1);
    }

    #[test]
    fn probe_surfaces_nonzero_exit_when_command_fails() {
        let mock = MockRunner {
            stdout: String::new(),
            stderr: "boom".into(),
            success: false,
            captured: RefCell::new(None),
        };
        let code = probe(&mock, "fake-cli", &[]);
        assert_eq!(code, 1);
    }
}
