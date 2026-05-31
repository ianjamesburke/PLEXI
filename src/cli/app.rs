use std::io::{self, Write};

pub(super) fn ensure_plexi_sdk() -> bool {
    let check = std::process::Command::new("python3")
        .args(["-c", "import plexi_sdk"])
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .status();

    match check {
        Ok(s) if s.success() => {
            log::info!("ensure_plexi_sdk: already importable");
            return true;
        }
        Err(e) => {
            log::warn!("ensure_plexi_sdk: python3 not found: {e}");
            eprintln!("warning: python3 not found — install plexi-sdk manually: pip install plexi-sdk");
            return false;
        }
        Ok(_) => {}
    }

    log::info!("ensure_plexi_sdk: plexi_sdk not importable, attempting install");

    // Try uv pip install first; suppress output so noisy venv warnings don't surface.
    let uv_ok = std::process::Command::new("uv")
        .args(["pip", "install", "plexi-sdk"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if uv_ok {
        println!("Installed plexi-sdk (via uv).");
        log::info!("ensure_plexi_sdk: installed via uv pip");
        return true;
    }

    // Fallback: python3 -m pip to guarantee the same environment that python3 checked.
    match std::process::Command::new("python3").args(["-m", "pip", "install", "plexi-sdk"]).status() {
        Ok(s) if s.success() => {
            println!("Installed plexi-sdk (via pip).");
            log::info!("ensure_plexi_sdk: installed via python3 -m pip");
            true
        }
        Ok(_) => {
            eprintln!("warning: could not install plexi-sdk — install manually: pip install plexi-sdk");
            log::warn!("ensure_plexi_sdk: python3 -m pip install failed");
            false
        }
        Err(e) => {
            eprintln!("warning: pip not available ({e}) — install manually: pip install plexi-sdk");
            log::warn!("ensure_plexi_sdk: python3 -m pip not available: {e}");
            false
        }
    }
}

/// Returns true if the app at `app_dir` has a Python entry point (entry ending in `.py`).
pub(super) fn app_is_python(app_dir: &std::path::Path) -> bool {
    let Ok(s) = std::fs::read_to_string(app_dir.join("manifest.toml")) else { return false; };
    let Ok(manifest) = toml::from_str::<toml::Value>(&s) else { return false; };
    let entry = manifest
        .get("app")
        .and_then(|a| a.get("entry"))
        .and_then(|e| e.as_str())
        .unwrap_or("");
    entry.ends_with(".py")
}

/// Detect the channel config dir name from the running binary name.
pub(super) fn app_init_config_dir() -> String {
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
pub fn app_init(name: &str, lang: &str, from_pane_id: Option<u64>) -> i32 {
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
                ensure_plexi_sdk();
                // Auto-open the app if PLEXI_SOCKET is set (running inside a pane).
                if std::env::var("PLEXI_SOCKET").is_ok() {
                    let path_str = app_dir.to_string_lossy().to_string();
                    log::info!("app_init: auto-opening '{name}' via app_run from_path={path_str} from_pane_id={from_pane_id:?}");
                    let exit_code = app_run(&path_str, from_pane_id);
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
    let template = include_str!("../../sdk/python/plexi_sdk/templates/app_init.py");
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
    if app_is_python(&dest) {
        ensure_plexi_sdk();
    }
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
pub fn app_run(path: &str, from_pane_id: Option<u64>) -> i32 {
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
        let from_pane_id = from_pane_id.or_else(|| {
            std::env::var("PLEXI_PANE_ID").ok()?.parse::<u64>().ok()
        });
        let mut payload = serde_json::json!({
            "type": "spawn_pane",
            "type_id": "",
            "path": abs_path,
            "response_file": response_file,
        });
        if let Some(pid) = from_pane_id {
            payload["from_pane_id"] = serde_json::Value::Number(pid.into());
        }
        log::info!("app_run:cli: sending via socket path={abs_path} from_pane_id={from_pane_id:?} response_file={response_file}");
        let code = super::send_to_socket(payload);
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
    super::list::list_cli()
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
pub(super) fn is_bare_id(s: &str) -> bool {
    !s.contains(':') && !s.contains('/') && !s.is_empty()
}

/// Returns true if `s` looks like a bare GitHub shorthand (`owner/repo`): no scheme,
/// exactly one `/`, non-empty owner and repo segments.
pub(super) fn is_github_shorthand(s: &str) -> bool {
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
pub(super) fn resolve_registry_id(id: &str) -> Result<String, String> {
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
#[cfg(test)]
mod app_run_tests {
    use tempfile::TempDir;

    #[test]
    fn app_run_nonexistent_path_returns_1() {
        let code = super::app_run("/tmp/plexi-test-nonexistent-path-xyzzy-12345", None);
        assert_eq!(code, 1);
    }

    #[test]
    fn app_run_dir_without_manifest_returns_1() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let code = super::app_run(&path, None);
        assert_eq!(code, 1);
    }

    #[test]
    fn app_run_invalid_manifest_returns_1() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("manifest.toml"), "this is not valid toml ][[[").unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let code = super::app_run(&path, None);
        assert_eq!(code, 1);
    }
}
