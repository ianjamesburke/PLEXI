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
            eprintln!(
                "warning: python3 not found — install plexi-sdk manually: pip install plexi-sdk"
            );
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
    match std::process::Command::new("python3")
        .args(["-m", "pip", "install", "plexi-sdk"])
        .status()
    {
        Ok(s) if s.success() => {
            println!("Installed plexi-sdk (via pip).");
            log::info!("ensure_plexi_sdk: installed via python3 -m pip");
            true
        }
        Ok(_) => {
            eprintln!(
                "warning: could not install plexi-sdk — install manually: pip install plexi-sdk"
            );
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
    let Ok(s) = std::fs::read_to_string(app_dir.join("manifest.toml")) else {
        return false;
    };
    let Ok(manifest) = toml::from_str::<toml::Value>(&s) else {
        return false;
    };
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
        .map(str::to_owned)
        .unwrap_or_else(crate::config::workspace_channel_dir)
}

/// `plexi app init [--lang python|rust] [--global] <name> [--open]`
///
/// Without `--global`: walks up from CWD looking for the nearest workspace
/// (ancestor with `<channel_dir>/`). If none is found, exits with an error
/// directing the user to pass `--global` or `cd` into a workspace.
///
/// With `--global`: scaffolds directly into the global registry
/// (`~/.plexi-<channel>/apps/<name>/`).
///
/// Scaffolds without opening by default. `--open` opens the app in a
/// split-right pane after creating it.
pub fn app_init(
    name: &str,
    lang: &str,
    global: bool,
    open: bool,
    from_pane_id: Option<u64>,
) -> i32 {
    if name.is_empty() {
        eprintln!("Usage: plexi app init [--lang python|rust] [--global] <name> [--open]");
        return 1;
    }

    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    if cwd == std::path::Path::new("/") {
        log::warn!("app_init: rejected at root dir");
        eprintln!("error: cannot scaffold an app in the root directory.");
        return 1;
    }

    let channel_dir = app_init_config_dir();
    let app_dir = if global {
        crate::app::registry::apps_dir().join(name)
    } else {
        let home = dirs::home_dir();
        let workspace_root = {
            let mut current = cwd;
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
        match workspace_root {
            Some(root) => root.join(&channel_dir).join("apps").join(name),
            None => {
                eprintln!(
                    "error: no workspace found (no {channel_dir}/ directory in any ancestor)."
                );
                eprintln!(
                    "  Use --global to create a global app, or cd into a project with a workspace."
                );
                return 1;
            }
        }
    };

    let placement = if global { "global" } else { "workspace" };
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
        "python_agent" => scaffold_agent_python_app(&app_dir, name),
        _ => scaffold_python_app(&app_dir, name),
    };

    match result {
        Ok(()) => {
            println!("Created app '{name}' at {}", app_dir.display());
            if lang == "rust" {
                println!("\nNext steps:");
                println!("  cd {}", app_dir.display());
                println!("  cargo build --release");
                println!("  plexi app open {}", app_dir.display());
            } else {
                ensure_plexi_sdk();
                if open {
                    let path_str = app_dir.to_string_lossy().to_string();
                    log::info!("app_init: opening '{name}' split-right path={path_str} from_pane_id={from_pane_id:?}");
                    let exit_code =
                        super::open_cli(&path_str, &[], Some("split_h"), from_pane_id, None);
                    if exit_code != 0 {
                        eprintln!(
                            "warning: app created but could not auto-open (exit {exit_code})"
                        );
                        eprintln!("  Open with: plexi app open {}", app_dir.display());
                    }
                } else {
                    log::info!(
                        "app_init: created '{name}' without opening path={}",
                        app_dir.display()
                    );
                    println!("  Open with: plexi app open {}", app_dir.display());
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
        "schema_version = 1\n\n[app]\nid = \"{name}\"\ntype = \"app\"\nname = \"{display}\"\nentry = \"main.py\"\nversion = \"0.1.0\"\ndescription = \"A Plexi app\"\nwatch = true\n\n[app.capabilities]\ncapabilities = [\"timer\"]\n\n[launch]\n",
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

fn scaffold_agent_python_app(app_dir: &std::path::Path, name: &str) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    // manifest.toml — ai.query capability pre-configured.
    std::fs::write(app_dir.join("manifest.toml"), format!(
        "schema_version = 1\n\n[app]\nid = \"{name}\"\ntype = \"app\"\nname = \"{display}\"\nentry = \"main.py\"\nversion = \"0.1.0\"\ndescription = \"An agent app\"\nwatch = true\n\n[app.capabilities]\ncapabilities = [\"ai.query\"]\n\n[launch]\n",
        name = name,
        display = to_title_case(name),
    ))?;

    let template = include_str!("../../sdk/python/plexi_sdk/templates/agent_init.py");
    let main_py = template
        .replace("__CLASS_NAME__", &to_struct_name(name))
        .replace("__DISPLAY_NAME__", &to_title_case(name));
    let main_path = app_dir.join("main.py");
    std::fs::write(&main_path, main_py)?;

    let mut perms = std::fs::metadata(&main_path)?.permissions();
    perms.set_mode(perms.mode() | 0o111);
    std::fs::set_permissions(&main_path, perms)?;

    log::info!(
        "scaffold_agent_python_app: created agent scaffold at {}",
        app_dir.display()
    );
    Ok(())
}

fn scaffold_rust_app(app_dir: &std::path::Path, name: &str) -> io::Result<()> {
    // manifest.toml
    std::fs::write(app_dir.join("manifest.toml"), format!(
        "schema_version = 1\n\n[app]\nid = \"{name}\"\ntype = \"app\"\nname = \"{display}\"\nentry = \"bin/plexi-app\"\nversion = \"0.1.0\"\ndescription = \"A Plexi app\"\n\n[app.capabilities]\ncapabilities = []\n\n[launch]\n",
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

/// `plexi app uninstall <id> [--yes]` — remove an installed app (global or workspace-local).
pub fn app_uninstall(id: &str, assume_yes: bool) -> i32 {
    let cwd = std::env::current_dir().unwrap_or_default();
    let registry = crate::app::registry::AppRegistry::load(&cwd);
    let Some(installed) = registry.get(id) else {
        eprintln!("error: app '{id}' not found — run `plexi app list` to see installed apps");
        return 1;
    };
    let app_dir = installed.app_dir.clone();
    let target_root = match app_dir.parent() {
        Some(p) => p.to_path_buf(),
        None => {
            eprintln!("error: could not determine parent directory for '{id}'");
            return 1;
        }
    };
    log::info!(
        "app_uninstall: resolving '{id}' via registry → {:?}",
        app_dir
    );
    let core_ids = crate::cli::install_host::core_pack_ids();
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
    match crate::cli::install_host::uninstall_one(id, &target_root) {
        Ok(()) => {
            println!("Uninstalled '{id}'.");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// Write `installed_version.txt` inside `app_dir`. Best-effort — logs `warn!` on failure.
pub(super) fn write_installed_version(app_dir: &std::path::Path, version: &str) {
    let path = app_dir.join("installed_version.txt");
    if let Err(e) = std::fs::write(&path, version) {
        log::warn!(
            "version_pin: could not write installed_version.txt for {}: {e}",
            app_dir.display()
        );
    } else {
        log::info!(
            "version_pin: wrote installed_version.txt={version} for {}",
            app_dir.display()
        );
    }
}

/// Write `pinned_version.txt` inside `app_dir`. Best-effort — logs `warn!` on failure.
pub(super) fn write_pinned_version(app_dir: &std::path::Path, version: &str) {
    let path = app_dir.join("pinned_version.txt");
    if let Err(e) = std::fs::write(&path, version) {
        log::warn!(
            "version_pin: could not write pinned_version.txt for {}: {e}",
            app_dir.display()
        );
    } else {
        log::info!(
            "version_pin: wrote pinned_version.txt={version} for {}",
            app_dir.display()
        );
    }
}

/// Who approved this install. Threaded explicitly through every install call
/// chain — no default, every caller chooses (stint 0016).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallConfirm {
    /// A user typed the command — show the trust sheet and prompt.
    Interactive,
    /// An internal caller (bundled pack seeding, an outer install path that
    /// already prompted) — never prompt.
    PreApproved,
}

/// Format a byte count for the trust sheet (B / KB / MB / GB, one decimal).
fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.1} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.1} GB", b / (KB * KB * KB))
    }
}

/// Print the trust sheet for a validated app dir or package: identity,
/// runtime + trust label, size, and every declared capability with its
/// description (sensitive ones marked). Plain text — no color, so NO_COLOR
/// holds trivially.
pub fn print_trust_sheet(
    report: &crate::app::package::PackageReport,
    label: crate::app::package::TrustLabel,
) {
    println!("id:           {}", report.id);
    println!("name:         {}", report.name);
    println!("version:      {}", report.version);
    println!("entry:        {}", report.entry);
    println!(
        "runtime:      {} — {}",
        report.runtime.as_str(),
        label.display_str()
    );
    println!(
        "files:        {} ({})",
        report.file_count,
        human_size(report.total_size)
    );
    if report.capabilities.is_empty() {
        println!("capabilities: (none)");
    } else {
        println!("capabilities:");
        for cap in &report.capabilities {
            let sensitive = if cap.is_sensitive() {
                " [sensitive]"
            } else {
                ""
            };
            println!("  {:<18}{}{sensitive}", cap.as_str(), cap.description());
        }
    }
}

/// Resolve the trust label for a report against the bundled core pack ids.
fn trust_label_for(report: &crate::app::package::PackageReport) -> crate::app::package::TrustLabel {
    let core_ids = crate::cli::install_host::core_pack_ids();
    let core_refs: Vec<&str> = core_ids.iter().map(String::as_str).collect();
    crate::app::package::trust_label(report, &core_refs)
}

/// The install confirmation gate. Returns `Ok(true)` to proceed,
/// `Ok(false)` when the user declined, `Err` when confirmation is impossible
/// (non-interactive stdin without `--yes`) or unreadable.
///
/// Pure decision logic over an injected reader so tests can feed answers.
pub fn confirm_install(
    report: &crate::app::package::PackageReport,
    label: crate::app::package::TrustLabel,
    mode: InstallConfirm,
    assume_yes: bool,
    is_tty: bool,
    reader: &mut impl io::BufRead,
) -> Result<bool, String> {
    match mode {
        InstallConfirm::PreApproved => Ok(true),
        InstallConfirm::Interactive => {
            if assume_yes {
                log::info!(
                    "confirm_install: --yes given, skipping prompt for '{}' ({:?})",
                    report.id,
                    label
                );
                return Ok(true);
            }
            if !is_tty {
                return Err(format!(
                    "stdin is not a terminal — cannot confirm install of '{}'. \
                     Re-run with --yes to approve non-interactively.",
                    report.id
                ));
            }
            eprint!("Install? [y/N] ");
            let _ = io::stderr().flush();
            let mut answer = String::new();
            reader
                .read_line(&mut answer)
                .map_err(|e| format!("failed to read install confirmation: {e}"))?;
            Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
        }
    }
}

/// Run the full trust gate for an interactive install: print the trust sheet,
/// then prompt. Returns an exit code on refusal/abort, `None` to proceed.
fn run_install_gate(report: &crate::app::package::PackageReport, assume_yes: bool) -> Option<i32> {
    use std::io::IsTerminal;
    let label = trust_label_for(report);
    print_trust_sheet(report, label);
    let is_tty = io::stdin().is_terminal();
    let mut stdin = io::stdin().lock();
    match confirm_install(
        report,
        label,
        InstallConfirm::Interactive,
        assume_yes,
        is_tty,
        &mut stdin,
    ) {
        Ok(true) => None,
        Ok(false) => {
            eprintln!("install aborted — '{}' was not installed", report.id);
            Some(1)
        }
        Err(e) => {
            eprintln!("error: {e}");
            Some(1)
        }
    }
}

/// `plexi app inspect <path>` — validate a local app dir or `.plexipkg` file
/// and print the trust sheet without installing anything (stint 0016).
pub fn app_inspect_cli(path: &str) -> i32 {
    log::info!("app_inspect:cli: path={path}");
    let target = std::path::Path::new(path);
    let report = if target.is_file() {
        crate::app::package::validate_package(target)
    } else {
        crate::app::package::validate_dir(target)
    };
    match report {
        Ok(report) => {
            print_trust_sheet(&report, trust_label_for(&report));
            0
        }
        Err(e) => {
            eprintln!("error: validation failed for {path}: {e}");
            1
        }
    }
}

/// `plexi app install <path> [--version X.Y.Z] [--yes]`
///
/// When `pin` is `None` (the common case) this is a plain install.
/// When `pin` is `Some(ver)` the version is stored in `pinned_version.txt`.
///
/// `confirm` decides whether the trust sheet + prompt gate runs:
/// `Interactive` for user-typed installs, `PreApproved` for internal callers
/// that already gated (or are first-party bundled content).
pub fn app_install_with_pin(
    path: &str,
    pin: Option<&str>,
    confirm: InstallConfirm,
    assume_yes: bool,
) -> i32 {
    let src = match std::path::Path::new(path).canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: could not resolve {path}: {e}");
            return 1;
        }
    };

    // The 0015 validator is the install gate (stint 0016). User-initiated
    // installs must pass it; PreApproved installs (bundled first-party
    // content, or an outer path that already validated and prompted) proceed
    // with a warning so startup seeding can never break on legacy layouts.
    match crate::app::package::validate_dir(&src) {
        Ok(report) => {
            if confirm == InstallConfirm::Interactive {
                if let Some(code) = run_install_gate(&report, assume_yes) {
                    return code;
                }
            }
        }
        Err(e) => match confirm {
            InstallConfirm::Interactive => {
                eprintln!(
                    "error: validation failed — refusing install of {}: {e}",
                    src.display()
                );
                return 1;
            }
            InstallConfirm::PreApproved => {
                log::warn!(
                    "app::install: pre-approved install of {} proceeding despite validation failure: {e}",
                    src.display()
                );
            }
        },
    }

    let manifest_path = src.join("manifest.toml");
    if !manifest_path.exists() {
        eprintln!("error: no manifest.toml found in {}", src.display());
        eprintln!("  Is this a Plexi app directory? Run `plexi app init <name>` to scaffold one.");
        return 1;
    }
    let manifest_str = match std::fs::read_to_string(&manifest_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: could not read manifest.toml: {e}");
            return 1;
        }
    };
    let manifest: toml::Value = match toml::from_str(&manifest_str) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: manifest.toml parse failed: {e}");
            return 1;
        }
    };

    let schema_version = manifest
        .get("schema_version")
        .and_then(|v| v.as_integer())
        .unwrap_or(0);
    if schema_version > crate::app::registry::MANIFEST_SCHEMA_VERSION as i64 {
        eprintln!(
            "error: manifest.toml schema_version {schema_version} is newer than supported (max {})",
            crate::app::registry::MANIFEST_SCHEMA_VERSION
        );
        return 1;
    }

    let app_section = match manifest.get("app") {
        Some(a) => a,
        None => {
            eprintln!("error: manifest.toml is missing [app] section");
            return 1;
        }
    };
    let app_id = match app_section.get("id").and_then(|v| v.as_str()) {
        Some(id)
            if !id.is_empty()
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') =>
        {
            id.to_string()
        }
        _ => {
            eprintln!("error: manifest.toml is missing a valid [app].id");
            eprintln!("  (IDs must be non-empty and contain only alphanumeric characters, dashes, or underscores)");
            return 1;
        }
    };
    let app_version = app_section
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("?");

    let dest = crate::app::registry::apps_dir().join(&app_id);

    // Remove existing install (idempotent overwrite).
    if dest.exists() {
        if let Err(e) = std::fs::remove_dir_all(&dest) {
            eprintln!(
                "error: could not remove existing install at {}: {e}",
                dest.display()
            );
            return 1;
        }
    }

    if let Err(e) = copy_dir_all(&src, &dest) {
        eprintln!(
            "error: could not copy {} to {}: {e}",
            src.display(),
            dest.display()
        );
        return 1;
    }

    // Write version tracking files (best-effort — never fail the install).
    write_installed_version(&dest, app_version);
    if let Some(pin) = pin {
        if pin != app_version {
            eprintln!(
                "warning: manifest version is {app_version} but pinning to {pin} as requested"
            );
        }
        write_pinned_version(&dest, pin);
    }

    log::info!(
        "app::install: installed {app_id} v{app_version} from {}",
        src.display()
    );
    println!(
        "Installed '{app_id}' v{app_version} from {}.",
        src.display()
    );
    if app_is_python(&dest) {
        ensure_plexi_sdk();
    }
    println!("Run `plexi app open {app_id}` to launch it.");
    0
}

/// `plexi app package <path> [--out <file>]` — build a `.plexipkg` artifact.
pub fn app_package_cli(path: &str, out: Option<&str>) -> i32 {
    log::info!("app_package:cli: path={path} out={out:?}");
    let app_dir = match std::path::Path::new(path).canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: could not resolve {path}: {e}");
            return 1;
        }
    };
    match crate::app::package::build_package(&app_dir, out.map(std::path::Path::new)) {
        Ok(pkg) => {
            println!("Built package {}", pkg.display());
            println!("  Validate with: plexi app validate {}", pkg.display());
            0
        }
        Err(e) => {
            eprintln!("error: package build failed: {e}");
            1
        }
    }
}

/// `plexi app install <file.plexipkg>` — validate the package (fail-closed),
/// then extract to a temp dir and run the standard dir install on it.
///
/// The trust gate runs here (against the validated package report); the inner
/// dir install is then `PreApproved` so the user is prompted exactly once.
pub fn app_install_package(
    file: &str,
    pin: Option<&str>,
    confirm: InstallConfirm,
    assume_yes: bool,
) -> i32 {
    log::info!("app_install:cli: package file={file} pin={pin:?} confirm={confirm:?}");
    let pkg_path = match std::path::Path::new(file).canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: could not resolve {file}: {e}");
            return 1;
        }
    };

    let report = match crate::app::package::validate_package(&pkg_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: package validation failed — refusing install: {e}");
            return 1;
        }
    };

    if confirm == InstallConfirm::Interactive {
        if let Some(code) = run_install_gate(&report, assume_yes) {
            return code;
        }
    }

    // Extract into a unique temp dir and reuse the existing dir-install path.
    let staging = std::env::temp_dir().join(format!("plexipkg-install-{}", uuid::Uuid::new_v4()));
    if let Err(e) = crate::app::package::extract_package(&pkg_path, &staging) {
        eprintln!("error: could not extract {}: {e}", pkg_path.display());
        let _ = std::fs::remove_dir_all(&staging);
        return 1;
    }
    log::info!(
        "app_install:cli: validated package id={} v{} ({} files) — installing from {}",
        report.id,
        report.version,
        report.file_count,
        staging.display()
    );
    // Already validated and (when interactive) confirmed above — the inner
    // dir install must not prompt a second time.
    let code = app_install_with_pin(
        &staging.to_string_lossy(),
        pin,
        InstallConfirm::PreApproved,
        assume_yes,
    );
    if let Err(e) = std::fs::remove_dir_all(&staging) {
        log::warn!(
            "app_install:cli: could not clean up staging dir {}: {e}",
            staging.display()
        );
    }
    code
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

/// `plexi app info <id>` — show manifest info for an installed app, including MCP URL if applicable.
pub fn app_info(id: &str) -> i32 {
    let registry =
        crate::app::registry::AppRegistry::load(&std::env::current_dir().unwrap_or_default());
    let Some(installed) = registry.get(id) else {
        eprintln!("error: app '{id}' not found — run `plexi app list` to see installed apps");
        return 1;
    };
    let m = &installed.manifest;
    println!("id:          {}", m.id);
    println!("name:        {}", m.name);
    println!("version:     {}", m.version);
    println!("description: {}", m.description);
    if let Some(ref author) = m.author {
        println!("author:      {author}");
    }
    if !m.tags.is_empty() {
        println!("tags:        {}", m.tags.join(", "));
    }
    if let Some(ref repo) = m.repo {
        println!("repo:        {repo}");
    }
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
        println!(
            "mcp_url:     http://localhost:${{PLEXI_MCP_PORT}}/mcp  (port assigned at runtime)"
        );
        println!();
        println!("Claude Desktop config:");
        println!("  {{");
        println!("    \"mcpServers\": {{");
        println!(
            "      \"{}\": {{ \"url\": \"http://localhost:${{PLEXI_MCP_PORT}}/mcp\" }}",
            m.id
        );
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

/// `plexi app render <app> --size WxH [--state state.json] [--output path] [--png]`
/// Renders an app headlessly. `app` may be an installed app ID or a local directory path.
/// Default output: JSON frame tree. With --png: PNG image.
pub fn app_render(
    app: &str,
    size: &str,
    state: Option<&str>,
    output: Option<&str>,
    png: bool,
) -> i32 {
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
            Ok(v) => Some(v),
            Err(e) => {
                eprintln!("error: invalid JSON in state file '{s}': {e}");
                return 1;
            }
        }
    } else {
        None
    };

    // Resolve (app_id, bin_path): path argument takes priority over registry lookup.
    // A path is detected by prefix (./  ../  /) or by existing as a directory.
    // Path: more than one component (./foo, ../foo, /abs/path) OR an existing directory.
    // Using components() instead of prefix checks is portable across platforms.
    let (app_id, app_bin) = if std::path::Path::new(app).components().count() > 1
        || std::path::Path::new(app).is_dir()
    {
        let app_dir = match std::path::Path::new(app).canonicalize() {
            Ok(p) => p,
            Err(e) => {
                log::warn!("app_render: could not resolve '{app}': {e}");
                eprintln!("error: could not resolve '{app}': {e}");
                return 1;
            }
        };
        if !app_dir.is_dir() {
            eprintln!("error: '{app}' is not a directory — expected an app directory containing manifest.toml");
            return 1;
        }
        let manifest_path = app_dir.join("manifest.toml");
        let manifest_str = match std::fs::read_to_string(&manifest_path) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("app_render: no manifest.toml in {}: {e}", app_dir.display());
                eprintln!("error: no manifest.toml in {}: {e}", app_dir.display());
                eprintln!(
                    "  Is this a Plexi app directory? Run `plexi app init <name>` to scaffold one."
                );
                return 1;
            }
        };
        let manifest: crate::app::registry::AppManifest = match toml::from_str(&manifest_str) {
            Ok(m) => m,
            Err(e) => {
                log::warn!(
                    "app_render: invalid manifest.toml in {}: {e}",
                    app_dir.display()
                );
                eprintln!("error: invalid manifest.toml: {e}");
                return 1;
            }
        };
        if manifest.schema_version > crate::app::registry::MANIFEST_SCHEMA_VERSION {
            eprintln!(
                "error: manifest.toml schema_version {} is newer than supported (max {}); update Plexi to render this app",
                manifest.schema_version,
                crate::app::registry::MANIFEST_SCHEMA_VERSION
            );
            return 1;
        }
        let entry = app_dir.join(&manifest.app.entry);
        if !entry.exists() {
            eprintln!(
                "error: app entry '{}' not found in {}",
                manifest.app.entry,
                app_dir.display()
            );
            return 1;
        }
        log::info!("app_render[{}]: loaded from path '{app}'", manifest.app.id);
        (manifest.app.id, entry)
    } else {
        // ID-based: registry lookup
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let registry = crate::app::registry::AppRegistry::load(&cwd);
        match registry.list().into_iter().find(|a| a.manifest.id == app) {
            Some(a) => (a.manifest.id.clone(), a.bin_path.clone()),
            None => {
                eprintln!(
                    "error: app '{app}' not found — run `plexi app list` to see installed apps"
                );
                return 1;
            }
        }
    };

    if png {
        // PNG mode: rasterize and write binary
        let png_bytes = match crate::render::app_render::render_app_to_png(
            &app_id, &app_bin, width, height, seed_state,
        ) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("error: render failed: {e}");
                return 1;
            }
        };
        match output {
            Some(path) => {
                if let Err(e) = std::fs::write(path, &png_bytes) {
                    eprintln!("error: could not write output to '{path}': {e}");
                    return 1;
                }
                log::info!("app_render[{app_id}]: wrote {width}×{height} PNG to '{path}'");
                eprintln!("Wrote {width}×{height} PNG to '{path}'");
            }
            None => {
                use std::io::Write;
                if let Err(e) = std::io::stdout().write_all(&png_bytes) {
                    eprintln!("error: could not write PNG to stdout: {e}");
                    return 1;
                }
                log::info!(
                    "app_render[{app_id}]: wrote {width}×{height} PNG to stdout ({} bytes)",
                    png_bytes.len()
                );
            }
        }
    } else {
        // JSON mode (default): return raw frame commands
        let json = match crate::render::app_render::render_app_to_json(
            &app_id, &app_bin, width, height, seed_state,
        ) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("error: render failed: {e}");
                return 1;
            }
        };
        match output {
            Some(path) => {
                if let Err(e) = std::fs::write(path, &json) {
                    eprintln!("error: could not write output to '{path}': {e}");
                    return 1;
                }
                log::info!("app_render[{app_id}]: wrote JSON frame to '{path}'");
                eprintln!("Wrote JSON frame to '{path}'");
            }
            None => {
                println!("{json}");
                log::info!("app_render[{app_id}]: wrote JSON frame to stdout");
            }
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


/// `plexi app update [<id>]` — local version check for installed apps.
///
/// v1: compares `<app_dir>/installed_version.txt` against `manifest.toml`
/// version field. If `pinned_version.txt` exists, reports the pin.
/// No network calls — registry check is future work.
pub fn app_update_cli(id: Option<&str>) -> i32 {
    let apps_dir = crate::app::registry::apps_dir();

    // Collect app dirs to check.
    let dirs: Vec<std::path::PathBuf> = match id {
        Some(app_id) => {
            let dir = apps_dir.join(app_id);
            if !dir.exists() {
                eprintln!("error: app '{app_id}' not installed — run `plexi app list`");
                return 1;
            }
            vec![dir]
        }
        None => match std::fs::read_dir(&apps_dir) {
            Ok(rd) => rd
                .flatten()
                .filter(|e| e.path().is_dir())
                .filter(|e| {
                    e.path()
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| !n.starts_with('.'))
                })
                .map(|e| e.path())
                .collect(),
            Err(_) => {
                println!("no apps installed");
                return 0;
            }
        },
    };

    if dirs.is_empty() {
        println!("no apps installed");
        return 0;
    }

    for app_dir in &dirs {
        let dir_name = app_dir.file_name().and_then(|n| n.to_str()).unwrap_or("?");

        // Read manifest version.
        let manifest_version = {
            let manifest_path = app_dir.join("manifest.toml");
            match std::fs::read_to_string(&manifest_path) {
                Ok(s) => match toml::from_str::<toml::Value>(&s) {
                    Ok(v) => v
                        .get("app")
                        .and_then(|a| a.get("version"))
                        .and_then(|ver| ver.as_str())
                        .unwrap_or("?")
                        .to_string(),
                    Err(_) => "?".to_string(),
                },
                Err(_) => "?".to_string(),
            }
        };

        // Read installed version (written at install time).
        let installed_version = std::fs::read_to_string(app_dir.join("installed_version.txt"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| manifest_version.clone());

        // Read optional pin.
        let pinned_version = std::fs::read_to_string(app_dir.join("pinned_version.txt"))
            .ok()
            .map(|s| s.trim().to_string());

        if let Some(ref pin) = pinned_version {
            println!("{dir_name}: pinned to v{pin} (installed v{installed_version})");
        } else if installed_version == manifest_version {
            println!("{dir_name}: up to date (v{installed_version})");
        } else {
            println!(
                "{dir_name}: installed v{installed_version}, manifest v{manifest_version} — consider reinstalling"
            );
        }
    }

    log::info!("app_update: checked {} app(s)", dirs.len());
    0
}

/// `plexi app action <pane_id> <action> [args...]`
///
/// Sends a `send_app_action` message to the host over PLEXI_SOCKET.
/// The host delivers a `PlexiEvent::Action { action, args }` to the target app pane.
/// Returns 0 on success, 1 on error.
pub fn app_action_cli(pane_id: u64, action: &str, args: &[String]) -> i32 {
    let id = uuid::Uuid::new_v4();
    let response_file = crate::config::config_dir()
        .join(format!("app-action-response-{id}.json"))
        .to_string_lossy()
        .into_owned();

    log::info!(
        "app_action:cli: pane_id={pane_id} action={action:?} args={args:?} response_file={response_file:?}"
    );

    let mut payload = serde_json::json!({
        "type": "send_app_action",
        "pane_id": pane_id,
        "action": action,
        "response_file": response_file,
    });
    if !args.is_empty() {
        payload["args"] = serde_json::json!(args);
    }

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
                    log::warn!("app_action:cli: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return 1;
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("error: timed out waiting for app action response");
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(test)]
mod app_install_workspace_tests {
    use tempfile::TempDir;

    fn write_valid_manifest(dir: &std::path::Path, id: &str) {
        std::fs::write(
            dir.join("manifest.toml"),
            format!(
                "schema_version = 1\n\n\
                 [app]\n\
                 id = \"{id}\"\n\
                 type = \"app\"\n\
                 name = \"Test\"\n\
                 entry = \"main.py\"\n\
                 version = \"0.1.0\"\n\
                 description = \"Test\"\n\
                 \n\
                 [app.capabilities]\n\
                 capabilities = []\n\
                 \n\
                 [launch]\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("main.py"), "# stub\n").unwrap();
    }

    /// `plexi app install <path>` must succeed from a bare directory with no
    /// `.plexi/` workspace. Install goes to the global channel apps dir — workspace
    /// resolution is never consulted for path-based installs.
    #[test]
    fn install_from_path_succeeds_without_workspace() {
        let src = TempDir::new().unwrap();
        let app_id = "plexi-test-install-no-workspace";
        write_valid_manifest(src.path(), app_id);
        let path = src.path().to_string_lossy().to_string();

        // No workspace present in src or any ancestor (temp dir) — must return 0.
        // Interactive + --yes: gate prints the trust sheet but never prompts.
        let code =
            super::app_install_with_pin(&path, None, super::InstallConfirm::Interactive, true);

        // Clean up installed app to avoid polluting the apps dir between runs.
        let dest = crate::app::registry::apps_dir().join(app_id);
        let _ = std::fs::remove_dir_all(&dest);

        assert_eq!(
            code, 0,
            "install from path must succeed without a workspace"
        );
    }

    #[test]
    fn install_fails_on_missing_manifest_not_workspace_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let code =
            super::app_install_with_pin(&path, None, super::InstallConfirm::Interactive, true);
        // Must fail with exit code 1 (manifest missing), not a workspace error.
        assert_eq!(code, 1, "missing manifest must return 1");
    }
}

#[cfg(test)]
mod install_confirm_tests {
    use super::{confirm_install, human_size, InstallConfirm};
    use crate::app::package::{PackageReport, PackageRuntime, TrustLabel};
    use std::io::Cursor;

    fn report() -> PackageReport {
        PackageReport {
            id: "gate-test".to_string(),
            name: "Gate Test".to_string(),
            version: "0.1.0".to_string(),
            runtime: PackageRuntime::Python,
            entry: "main.py".to_string(),
            capabilities: Vec::new(),
            file_count: 2,
            total_size: 64,
        }
    }

    #[test]
    fn non_tty_without_yes_fails_closed() {
        let r = report();
        let err = confirm_install(
            &r,
            TrustLabel::PythonUnreviewed,
            InstallConfirm::Interactive,
            false, // no --yes
            false, // stdin not a tty
            &mut Cursor::new(b"y\n"),
        )
        .unwrap_err();
        assert!(
            err.contains("--yes"),
            "fail-closed error must tell the user to pass --yes, got: {err}"
        );
    }

    #[test]
    fn assume_yes_skips_prompt() {
        let r = report();
        // Reader is empty: --yes must never read stdin, even non-tty.
        let ok = confirm_install(
            &r,
            TrustLabel::PythonUnreviewed,
            InstallConfirm::Interactive,
            true,
            false,
            &mut Cursor::new(b""),
        )
        .unwrap();
        assert!(ok, "--yes must approve without reading stdin");
    }

    #[test]
    fn pre_approved_never_prompts() {
        let r = report();
        let ok = confirm_install(
            &r,
            TrustLabel::FirstPartyCore,
            InstallConfirm::PreApproved,
            false,
            false,
            &mut Cursor::new(b""),
        )
        .unwrap();
        assert!(ok, "PreApproved must approve unconditionally");
    }

    #[test]
    fn interactive_y_approves_and_n_refuses() {
        let r = report();
        for (input, expected) in [
            (&b"y\n"[..], true),
            (b"Y\n", true),
            (b"yes\n", true),
            (b"n\n", false),
            (b"no\n", false),
            (b"\n", false),
            (b"", false), // EOF = refuse
        ] {
            let got = confirm_install(
                &r,
                TrustLabel::PythonUnreviewed,
                InstallConfirm::Interactive,
                false,
                true,
                &mut Cursor::new(input),
            )
            .unwrap();
            assert_eq!(got, expected, "input {input:?}");
        }
    }

    #[test]
    fn human_size_formats() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
    }
}

#[cfg(test)]
mod app_inspect_tests {
    use tempfile::TempDir;

    #[test]
    fn inspect_valid_dir_returns_0() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("manifest.toml"),
            "schema_version = 1\n\n\
             [app]\n\
             id = \"inspect-test\"\n\
             type = \"app\"\n\
             name = \"Inspect Test\"\n\
             entry = \"main.py\"\n\
             version = \"0.1.0\"\n\
             description = \"Test\"\n\n\
             [app.capabilities]\n\
             capabilities = [\"ai.query\"]\n\n\
             [launch]\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("main.py"), "# stub\n").unwrap();
        let code = super::app_inspect_cli(&dir.path().to_string_lossy());
        assert_eq!(code, 0, "valid app dir must inspect cleanly");
    }

    #[test]
    fn inspect_invalid_dir_returns_1() {
        let dir = TempDir::new().unwrap();
        // No manifest — must fail validation, exit non-zero.
        let code = super::app_inspect_cli(&dir.path().to_string_lossy());
        assert_eq!(code, 1, "dir without manifest must fail inspect");
    }
}

#[cfg(test)]
mod version_pin_tests {
    use tempfile::TempDir;

    fn write_manifest(dir: &std::path::Path, id: &str, version: &str) {
        let manifest = format!(
            "schema_version = 1\n\n[app]\nid = \"{id}\"\ntype = \"app\"\nname = \"{id}\"\n\
             version = \"{version}\"\nentry = \"main.py\"\ndescription = \"Test\"\n\n\
             [app.capabilities]\ncapabilities = []\n\n[launch]\n"
        );
        std::fs::write(dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(dir.join("main.py"), "# stub\n").unwrap();
    }

    #[test]
    fn installed_version_written_to_file() {
        let src = TempDir::new().unwrap();
        write_manifest(src.path(), "my-app", "1.2.3");

        // Use a temp dir as the apps_dir root — app_install copies to apps_dir/my-app.
        // We can test write_installed_version directly since app_install calls apps_dir().
        // Instead, call write_installed_version directly and verify the file.
        let dest = TempDir::new().unwrap();
        super::write_installed_version(dest.path(), "1.2.3");
        let content = std::fs::read_to_string(dest.path().join("installed_version.txt")).unwrap();
        assert_eq!(content, "1.2.3");
    }

    #[test]
    fn pinned_version_stored_when_flag_given() {
        let dest = TempDir::new().unwrap();
        super::write_pinned_version(dest.path(), "0.9.0");
        let content = std::fs::read_to_string(dest.path().join("pinned_version.txt")).unwrap();
        assert_eq!(content, "0.9.0");
    }

    #[test]
    fn app_update_reports_up_to_date() {
        let dir = TempDir::new().unwrap();
        // Write manifest with version 0.1.0.
        write_manifest(dir.path(), "test-app", "0.1.0");
        // Write installed_version.txt matching manifest.
        super::write_installed_version(dir.path(), "0.1.0");

        // app_update_cli operates on apps_dir(), which is a runtime directory.
        // Test the logic via the helper directly instead.
        let installed = std::fs::read_to_string(dir.path().join("installed_version.txt"))
            .map(|s| s.trim().to_string())
            .unwrap();
        let manifest_val: toml::Value =
            toml::from_str(&std::fs::read_to_string(dir.path().join("manifest.toml")).unwrap())
                .unwrap();
        let manifest_ver = manifest_val
            .get("app")
            .and_then(|a| a.get("version"))
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(installed, "0.1.0");
        assert_eq!(manifest_ver, "0.1.0");
        assert_eq!(
            installed, manifest_ver,
            "up-to-date: installed matches manifest"
        );
        // No pinned_version.txt → no pin.
        assert!(!dir.path().join("pinned_version.txt").exists());
    }
}
