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

/// Entry point for `plexi secret set <key>` — stores a secret for the current directory.
pub fn set_secret(key: &str) -> i32 {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: could not determine current directory: {e}");
            return 1;
        }
    };

    eprint!("Enter value for {key}: ");
    let _ = io::stderr().flush();

    let value = match read_secret_from_stdin() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("\nerror: failed to read secret: {e}");
            return 1;
        }
    };
    eprintln!(); // newline after hidden input

    if value.is_empty() {
        eprintln!("error: empty value, nothing stored");
        return 1;
    }

    let dir_str = cwd.to_string_lossy();
    if crate::secrets::store_secret(key, &value, APP_ID, &dir_str) {
        eprintln!("Stored secret '{key}' for {}", cwd.display());
        0
    } else {
        eprintln!("error: failed to store secret '{key}'");
        1
    }
}

/// Entry point for `plexi secret delete <key>` — deletes a secret for the current directory.
pub fn delete_secret_cli(key: &str) -> i32 {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: could not determine current directory: {e}");
            return 1;
        }
    };

    let dir_str = cwd.to_string_lossy();
    if crate::secrets::delete_secret(key, APP_ID, &dir_str) {
        eprintln!("Deleted secret '{key}' for {}", cwd.display());
        0
    } else {
        eprintln!("error: failed to delete secret '{key}' (does it exist?)");
        1
    }
}

/// Entry point for `plexi secret list` — lists secrets for the current directory.
pub fn list_secrets() -> i32 {
    let accounts = crate::secrets::list_secrets(APP_ID);

    if accounts.is_empty() {
        eprintln!("No secrets stored for {APP_ID}.");
        return 0;
    }

    // accounts are strings like "plexi-run/dir/key"
    // Group by directory
    let prefix = format!("{APP_ID}/");
    let mut by_dir: HashMap<String, Vec<String>> = HashMap::new();

    for account in &accounts {
        if let Some(rest) = account.strip_prefix(&prefix) {
            // rest = "dir/key" — split on last '/' to separate dir from key
            if let Some(last_slash) = rest.rfind('/') {
                let dir = &rest[..last_slash];
                let key = &rest[last_slash + 1..];
                by_dir
                    .entry(dir.to_string())
                    .or_default()
                    .push(key.to_string());
            }
        }
    }

    let mut dirs: Vec<&String> = by_dir.keys().collect();
    dirs.sort();

    for dir in dirs {
        println!("{}:", dir);
        let keys = by_dir.get(dir).unwrap();
        for key in keys {
            println!("  {key}");
        }
    }

    0
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
