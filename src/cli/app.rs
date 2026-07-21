use std::io::{self, Write};

pub(super) const SCAFFOLD_METADATA_FILE: &str = "plexi.scaffold.toml";
pub(super) const SCAFFOLD_METADATA_SCHEMA_VERSION: u32 = 1;
pub(super) const PYTHON_SCAFFOLD_TEMPLATE_VERSION: u32 = 3;

/// Detect the channel config dir name from the running binary name.
pub(super) fn app_init_config_dir() -> String {
    crate::config::config_dir()
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .unwrap_or_else(crate::config::workspace_channel_dir)
}

/// Build an actionable error message for `app init` when `app_dir` already
/// exists. Reuses `AppManifest` (never a hand-rolled TOML parse — see
/// src/cli/AGENTS.md / root AGENTS.md regex-vs-parser rule) to name what's
/// already occupying the slot, then suggests a single alternative name so the
/// caller (including the assistant's `build-plexi-app` skill) can retry
/// immediately instead of pivoting to edit a stale/foreign app in place
/// (stint 0428).
pub(super) fn describe_app_init_collision(app_dir: &std::path::Path, name: &str) -> String {
    let manifest_path = app_dir.join("manifest.toml");
    let existing = std::fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|raw| toml::from_str::<crate::app::registry::AppManifest>(&raw).ok());

    let mut msg = match existing {
        Some(manifest) => format!(
            "error: {} already exists (id=\"{}\", type={:?}) — pick a different name",
            app_dir.display(),
            manifest.app.id,
            manifest.app.manifest_type
        ),
        None => format!(
            "error: {} already exists — pick a different name",
            app_dir.display()
        ),
    };
    msg.push_str(&format!("\n  Try: plexi app init {name}-2"));
    msg
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
        log::info!(
            "app_init: name collision at {} — refusing to overwrite existing directory",
            app_dir.display()
        );
        eprintln!("{}", describe_app_init_collision(&app_dir, name));
        return 1;
    }

    if let Err(e) = std::fs::create_dir_all(&app_dir) {
        eprintln!("error: could not create {}: {e}", app_dir.display());
        return 1;
    }

    // `lang` is constrained by the CLI `value_parser` (see AppCmd::Init); an
    // unlisted value never reaches here. The explicit arms keep the mapping
    // total so a newly added language can't silently fall back to Python.
    let result = match lang {
        "wasm" => scaffold_wasm_app(&app_dir, name),
        "rust" => scaffold_rust_app(&app_dir, name),
        "python_agent" => scaffold_agent_python_app(&app_dir, name),
        "python" => scaffold_python_app(&app_dir, name),
        other => {
            log::error!("app_init: no scaffold implemented for --lang '{other}'");
            eprintln!("error: no scaffold implemented for --lang '{other}'");
            return 1;
        }
    };

    match result {
        Ok(()) => {
            if !matches!(lang, "rust" | "wasm") {
                match crate::app::python_env::ensure_app_venv(name, &app_dir, &[]) {
                    Ok(python) => {
                        println!("  Python venv: {}", python.display());
                    }
                    Err(e) => {
                        let entry_path = app_dir.join("main.py");
                        match crate::app::python_env::resolve_python_runtime(
                            name,
                            &entry_path,
                            false,
                            &[],
                        ) {
                            Ok(runtime) => {
                                log::warn!(
                                    "app_init[{name}]: Python venv setup failed: {e}; using {} ({})",
                                    runtime.label,
                                    runtime.version
                                );
                                eprintln!(
                                    "warning: Python venv setup failed: {e}; using {} ({})",
                                    runtime.label, runtime.version
                                );
                            }
                            Err(runtime_err) => {
                                log::error!(
                                    "app_init[{name}]: Python environment setup failed: {e}; no fallback runtime: {runtime_err}"
                                );
                                eprintln!(
                                    "error: app created, but no compatible Python runtime is available."
                                );
                                eprintln!("  venv setup: {e}");
                                eprintln!("  fallback: {runtime_err}");
                                return 1;
                            }
                        }
                    }
                }
            }
            println!("Created app '{name}' at {}", app_dir.display());
            let (channel, profile_dir) = current_scaffold_channel();
            let explicit_plexi = explicit_plexi_command(&channel);
            let explicit_host_plexi = explicit_host_plexi_command(&channel, &profile_dir);
            if lang == "wasm" {
                println!("\nNext steps:");
                println!("  cd {}", app_dir.display());
                println!("  cargo component build --release --target wasm32-wasip2");
                println!("  {explicit_host_plexi} app open {}", app_dir.display());
                println!("  SDK docs: {}", app_dir.join("AUTHORING.md").display());
            } else if lang == "rust" {
                println!("\nNext steps:");
                println!("  cd {}", app_dir.display());
                println!("  cargo build --release");
                println!("  {explicit_host_plexi} app open {}", app_dir.display());
            } else {
                if open {
                    let path_str = app_dir.to_string_lossy().to_string();
                    log::info!("app_init: opening '{name}' split-right path={path_str} from_pane_id={from_pane_id:?}");
                    let exit_code =
                        super::open_cli(&path_str, &[], Some("split_h"), from_pane_id, None);
                    if exit_code != 0 {
                        eprintln!(
                            "warning: app created but could not auto-open (exit {exit_code})"
                        );
                        eprintln!(
                            "  Open with: {explicit_host_plexi} app open {}",
                            app_dir.display()
                        );
                    }
                } else {
                    log::info!(
                        "app_init: created '{name}' without opening path={}",
                        app_dir.display()
                    );
                    println!(
                        "  Open with: {explicit_host_plexi} app open {}",
                        app_dir.display()
                    );
                }
                println!("  Agent loop: read {}/AGENTS.md", app_dir.display());
                println!(
                    "  Test with: {explicit_plexi} app test {}",
                    app_dir.display()
                );
                println!(
                    "  Check gate: {explicit_plexi} app check {} --png-dir /tmp/{name}-shots",
                    app_dir.display()
                );
                println!(
                    "  Render state: {explicit_plexi} app render {} --state fixtures/state.json",
                    app_dir.display()
                );
                println!(
                    "  Hot reload: after open, edit source and verify the same pane id updates with pane state"
                );
                println!("  Host probes:");
                println!("    {explicit_host_plexi} pane state <pane-id>");
                println!("    {explicit_host_plexi} app action <pane-id> <handler-id>");
                println!("    {explicit_host_plexi} pane key <pane-id> <key>");
                println!(
                    "  SDK docs: read sdk/python/SDK_V3.md; with Plexi's SDK on PYTHONPATH, run python -c \"import plexi_sdk; help(plexi_sdk)\""
                );
            }
            0
        }
        Err(e) => {
            eprintln!("error: failed to scaffold app: {e}");
            1
        }
    }
}

/// `plexi app test [<app-path>]` — run an app's AppHarness tests via
/// `uv run pytest tests/` inside the app directory. Streams pytest output live
/// and returns its exit code so CI and ship scripts can gate on it.
pub fn app_test_cli(path: &str, snapshot: bool) -> i32 {
    let app_dir = std::path::Path::new(path);
    let tests_dir = app_dir.join("tests");
    if !tests_dir.is_dir() {
        log::warn!("app_test:cli: no tests/ dir at {}", tests_dir.display());
        eprintln!(
            "error: no tests/ directory in {} — expected {}",
            app_dir.display(),
            app_dir.join("tests").join("test_app.py").display()
        );
        eprintln!("  `plexi app init` scaffolds tests/test_app.py for new apps.");
        return 1;
    }

    log::info!(
        "app_test:cli: running `uv run pytest tests/` in {} (snapshot={snapshot})",
        app_dir.display()
    );

    let mut cmd = std::process::Command::new("uv");
    cmd.args(["run", "pytest", "tests/"]).current_dir(app_dir);
    cmd.env("PYTHONPATH", crate::config::build_pythonpath(None));
    if snapshot {
        cmd.env("PLEXI_UPDATE_SNAPSHOTS", "1");
    }

    match cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            log::error!("app_test:cli: failed to spawn uv: {e}");
            eprintln!("error: could not run `uv run pytest` ({e}). Is uv installed?");
            1
        }
    }
}

/// Commented-out `[marketplace]` section appended to every scaffolded manifest so
/// authors know publishing is one uncomment away. The host validator reads a
/// top-level `[marketplace]` section (see `read_marketplace_manifest` in
/// `src/cli/marketplace.rs`); `publisher` is the field required to publish.
const MARKETPLACE_PLACEHOLDER: &str = "\n# Uncomment and fill in to publish with `plexi app publish`:\n# [marketplace]\n# visibility = \"public\"   # public | unlisted | private\n# price = \"free\"          # \"free\", or a price like \"4.99\"\n# publisher = \"your-org\"  # required: your publisher / org slug\n";

fn marketplace_placeholder() -> &'static str {
    if crate::release::feature_enabled(crate::release::ReleaseFeature::Marketplace) {
        MARKETPLACE_PLACEHOLDER
    } else {
        ""
    }
}

pub(super) fn python_sdk_version() -> String {
    const PYPROJECT: &str = include_str!("../../sdk/python/pyproject.toml");
    toml::from_str::<toml::Value>(PYPROJECT)
        .ok()
        .and_then(|value| {
            value
                .get("project")
                .and_then(|project| project.get("version"))
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_string())
}

pub(super) fn current_scaffold_channel() -> (String, String) {
    let profile_dir = crate::config::workspace_channel_dir();
    let channel = profile_dir
        .strip_prefix(".plexi-")
        .map(str::to_owned)
        .or_else(|| {
            if profile_dir == ".plexi" {
                Some("main".to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| profile_dir.trim_start_matches('.').to_string());
    (channel, profile_dir)
}

fn explicit_plexi_command(channel: &str) -> String {
    if channel == "main" {
        "plexi".to_string()
    } else {
        format!("PLEXI_CHANNEL={channel} plexi")
    }
}

fn explicit_host_plexi_command(channel: &str, profile_dir: &str) -> String {
    let socket = format!("PLEXI_SOCKET=$HOME/{profile_dir}/notify.sock");
    if channel == "main" {
        format!("{socket} plexi")
    } else {
        format!("{socket} PLEXI_CHANNEL={channel} plexi")
    }
}

fn write_python_scaffold_support_files(app_dir: &std::path::Path, name: &str) -> io::Result<()> {
    let sdk_version = python_sdk_version();
    let (channel, profile_dir) = current_scaffold_channel();
    let cli_version = env!("CARGO_PKG_VERSION");
    let manifest_schema_version = crate::app::registry::MANIFEST_SCHEMA_VERSION;
    let python_runtime_version = crate::app::python_env::PYTHON_APP_VENV_VERSION;
    let template_version = PYTHON_SCAFFOLD_TEMPLATE_VERSION;

    let agents = include_str!("../../sdk/python/plexi_sdk/templates/AGENTS.md")
        .replace("__APP_NAME__", name)
        .replace("__CLI_VERSION__", cli_version)
        .replace("__SDK_VERSION__", &sdk_version)
        .replace(
            "__MANIFEST_SCHEMA_VERSION__",
            &manifest_schema_version.to_string(),
        )
        .replace("__PYTHON_RUNTIME_VERSION__", python_runtime_version)
        .replace("__TEMPLATE_VERSION__", &template_version.to_string())
        .replace("__CHANNEL__", &channel)
        .replace("__PROFILE_DIR__", &profile_dir);
    std::fs::write(app_dir.join("AGENTS.md"), agents)?;

    let gitignore = include_str!("../../sdk/python/plexi_sdk/templates/gitignore");
    std::fs::write(app_dir.join(".gitignore"), gitignore)?;

    let metadata = format!(
        "schema_version = {schema}\n\
         generated_by = \"plexi app init\"\n\
         plexi_cli_version = \"{cli_version}\"\n\
         sdk_version = \"{sdk_version}\"\n\
         manifest_schema_version = {manifest_schema_version}\n\
         python_runtime_version = \"{python_runtime_version}\"\n\
         template_version = {template_version}\n\
         channel = \"{channel}\"\n\
         profile_dir = \"{profile_dir}\"\n",
        schema = SCAFFOLD_METADATA_SCHEMA_VERSION,
    );
    std::fs::write(app_dir.join(SCAFFOLD_METADATA_FILE), metadata)?;
    log::info!(
        "app_init: wrote scaffold support files metadata={} template_version={template_version} sdk_version={sdk_version} channel={channel} profile_dir={profile_dir}",
        app_dir.join(SCAFFOLD_METADATA_FILE).display()
    );

    Ok(())
}

fn scaffold_python_app(app_dir: &std::path::Path, name: &str) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    // manifest.toml — shape lives in the template file beside the other scaffold
    // artifacts so it can't silently diverge from the documented manifest. The
    // feature-gated marketplace placeholder is appended in Rust.
    let manifest_template = include_str!("../../sdk/python/plexi_sdk/templates/manifest.toml");
    let manifest = format!(
        "{}{}",
        manifest_template
            .replace("__APP_ID__", name)
            .replace("__DISPLAY_NAME__", &to_title_case(name)),
        marketplace_placeholder(),
    );
    std::fs::write(app_dir.join("manifest.toml"), manifest)?;

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

    // tests/test_app.py — a working AppHarness example co-located with the app
    // so agents learn the test pattern from the scaffold, not from docs.
    let tests_dir = app_dir.join("tests");
    std::fs::create_dir_all(&tests_dir)?;
    let test_template = include_str!("../../sdk/python/plexi_sdk/templates/test_app_init.py");
    let test_py = test_template.replace("__DISPLAY_NAME__", &to_title_case(name));
    std::fs::write(tests_dir.join("test_app.py"), test_py)?;

    let fixtures_dir = app_dir.join("fixtures");
    std::fs::create_dir_all(&fixtures_dir)?;
    std::fs::write(fixtures_dir.join("state.json"), "{\n  \"count\": 3\n}\n")?;

    write_python_scaffold_support_files(app_dir, name)?;

    Ok(())
}

fn scaffold_agent_python_app(app_dir: &std::path::Path, name: &str) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    // manifest.toml — ai.query capability pre-configured.
    std::fs::write(app_dir.join("manifest.toml"), format!(
        "schema_version = 1\n\n[app]\nid = \"{name}\"\ntype = \"app\"\nname = \"{display}\"\nentry = \"main.py\"\nversion = \"0.1.0\"\ndescription = \"An agent app\"\nwatch = true\n\n[app.capabilities]\ncapabilities = [\"ai.query\"]\n\n[launch]\n{mp}",
        name = name,
        display = to_title_case(name),
        mp = marketplace_placeholder(),
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
        "schema_version = 1\n\n[app]\nid = \"{name}\"\ntype = \"app\"\nname = \"{display}\"\nentry = \"bin/plexi-app\"\nversion = \"0.1.0\"\ndescription = \"A Plexi app\"\n\n[app.capabilities]\ncapabilities = []\n\n[launch]\n{mp}",
        name = name,
        display = to_title_case(name),
        mp = marketplace_placeholder(),
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
        "use plexi_sdk::{{App, Emitter, Modifiers, RenderContext, run}};\n\nstruct {struct_name};\n\nimpl App for {struct_name} {{\n    fn on_render(&mut self, ctx: &mut RenderContext) {{\n        // Canvas color fields accept \"theme.<token>\" so app chrome tracks the host theme.\n        ctx.rect(0.0, 0.0, ctx.width, ctx.height, \"theme.bg\");\n        ctx.text_bold(20.0, 20.0, \"{display}\", 16.0, \"theme.fg\");\n        ctx.text(20.0, 50.0, \"Edit src/main.rs to build your app.\", 13.0, \"theme.muted\");\n    }}\n\n    fn on_key(&mut self, _key: &str, _mods: &Modifiers, _emit: &mut Emitter) {{}}\n}}\n\nfn main() {{\n    run(&mut {struct_name});\n}}\n",
        struct_name = to_struct_name(name),
        display = to_title_case(name),
    ))?;

    Ok(())
}

fn scaffold_wasm_app(app_dir: &std::path::Path, name: &str) -> io::Result<()> {
    let crate_name = name.replace('-', "_");
    std::fs::write(
        app_dir.join("manifest.toml"),
        format!(
            "schema_version = 1\n\n[app]\nid = \"{name}\"\ntype = \"wasm\"\nname = \"{display}\"\nentry = \"target/wasm32-wasip1/release/{crate_name}.wasm\"\nversion = \"0.1.0\"\ndescription = \"A Plexi WASM app\"\n\n[app.capabilities]\ncapabilities = []\n\n[app.capabilities.wasm]\nrequired = []\noptional = []\n\n[launch]\n",
            display = to_title_case(name),
        ),
    )?;
    std::fs::write(
        app_dir.join("Cargo.toml"),
        "[workspace]\nmembers = [\"app\"]\nresolver = \"2\"\n",
    )?;
    let crate_dir = app_dir.join("app");
    std::fs::create_dir_all(crate_dir.join("src"))?;
    std::fs::write(
        crate_dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\ncrate-type = [\"cdylib\"]\n\n[dependencies]\nplexi-wasm-sdk = {{ path = \"../.plexi-sdk/plexi-wasm-sdk\" }}\n\n[package.metadata.component]\npackage = \"plexi:{crate_name}\"\n\n[package.metadata.component.target]\npath = \"../wit\"\nworld = \"plexi-app\"\n",
        ),
    )?;
    std::fs::write(
        crate_dir.join("src/lib.rs"),
        format!(
            "use plexi_wasm_sdk::{{effects, export_app, ui, App, Effect, InputEvent, KeyEvent, UiActionEvent, UiTree}};\n\nconst INCREMENT: &str = \"increment\";\n\n#[derive(Default)]\nstruct Counter {{ count: u32 }}\n\nimpl App for Counter {{\n    fn init(&mut self, _context: plexi_wasm_sdk::InitContext) -> Vec<Effect> {{\n        vec![effects::set_title(\"{display}\")]\n    }}\n\n    fn update(&mut self, event: InputEvent) -> Vec<Effect> {{\n        let increment = matches!(event, InputEvent::UiAction(UiActionEvent {{ ref handler_id }}) if handler_id == INCREMENT)\n            || matches!(event, InputEvent::Key(KeyEvent {{ ref key, pressed: true, .. }}) if key == \"enter\" || key == \"space\");\n        if increment {{\n            self.count += 1;\n        }}\n        Vec::new()\n    }}\n\n    fn view(&self) -> UiTree {{\n        let mut tree = ui::Tree::new();\n        let count = tree.text(\"count\", format!(\"Count: {{}}\", self.count));\n        let increment = tree.button(\"increment\", \"Increment\", INCREMENT);\n        let root = tree.column(\"root\", [count, increment]);\n        tree.finish(root)\n    }}\n}}\n\nexport_app!(Counter::default());\n",
            display = to_title_case(name),
        ),
    )?;
    std::fs::write(
        app_dir.join("README.md"),
        "# Build and open\n\n```sh\ncargo install cargo-component\ncargo component build --release --target wasm32-wasip2\nplexi app open .\n```\n\nThe manifest opens the component at `target/wasm32-wasip1/release/`. Click **Increment**, send the `increment` app action, or press Enter/Space while the pane is focused. Read the local [Plexi WASM authoring guide](AUTHORING.md) for the SDK reference.\n",
    )?;
    std::fs::write(
        app_dir.join("AUTHORING.md"),
        include_str!("../../sdk/wasm/AUTHORING.md"),
    )?;
    std::fs::write(app_dir.join(".gitignore"), "/target/\n**/src/bindings.rs\n")?;

    let embedded = app_dir.join(".plexi-sdk");
    let embedded_sdk = embedded.join("plexi-wasm-sdk");
    std::fs::create_dir_all(embedded_sdk.join("src"))?;
    std::fs::create_dir_all(embedded.join("wit"))?;
    std::fs::write(
        embedded.join("wit/plexi.wit"),
        include_str!("../../wit/plexi.wit"),
    )?;
    std::fs::write(
        embedded_sdk.join("Cargo.toml"),
        include_str!("../../sdk/wasm/Cargo.toml"),
    )?;
    std::fs::write(
        embedded_sdk.join("src/lib.rs"),
        include_str!("../../sdk/wasm/src/lib.rs"),
    )?;
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(".plexi-sdk/wit", app_dir.join("wit"))?;
        std::os::unix::fs::symlink("../wit", embedded_sdk.join("wit"))?;
    }
    #[cfg(windows)]
    {
        std::fs::create_dir_all(app_dir.join("wit"))?;
        std::fs::write(
            app_dir.join("wit/plexi.wit"),
            include_str!("../../wit/plexi.wit"),
        )?;
        std::fs::create_dir_all(embedded_sdk.join("wit"))?;
        std::fs::write(
            embedded_sdk.join("wit/plexi.wit"),
            include_str!("../../wit/plexi.wit"),
        )?;
    }
    log::info!(
        "app_init: created self-contained wasm scaffold path={} entry=target/wasm32-wasip1/release/{crate_name}.wasm",
        app_dir.display(),
    );
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct InstallGateDecision {
    selected_wasm_optional: Vec<String>,
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
    for line in trust_sheet_lines(report, label) {
        println!("{line}");
    }
}

fn trust_sheet_lines(
    report: &crate::app::package::PackageReport,
    label: crate::app::package::TrustLabel,
) -> Vec<String> {
    let mut lines = vec![
        format!("id:           {}", report.id),
        format!("name:         {}", report.name),
        format!("version:      {}", report.version),
        format!("entry:        {}", report.entry),
        format!(
            "runtime:      {} — {}",
            report.runtime.as_str(),
            label.display_str()
        ),
        format!(
            "files:        {} ({})",
            report.file_count,
            human_size(report.total_size)
        ),
    ];
    if report.capabilities.is_empty()
        && report.wasm_required_capabilities.is_empty()
        && report.wasm_optional_capabilities.is_empty()
    {
        lines.push("capabilities: (none)".to_string());
    } else if !report.capabilities.is_empty() {
        lines.push("capabilities:".to_string());
        push_capability_rows(&mut lines, &report.capabilities);
    }
    if !report.wasm_required_capabilities.is_empty() {
        lines.push("wasm required capabilities:".to_string());
        push_wasm_capability_rows(&mut lines, &report.wasm_required_capabilities);
    }
    if !report.wasm_optional_capabilities.is_empty() {
        lines.push("wasm optional capabilities:".to_string());
        push_wasm_capability_rows(&mut lines, &report.wasm_optional_capabilities);
    }
    if report.requires_plexi_min.is_some() || report.requires_plexi_max.is_some() {
        let min = report.requires_plexi_min.as_deref().unwrap_or("any");
        let max = report.requires_plexi_max.as_deref().unwrap_or("any");
        lines.push(format!("requires:     Plexi {min} .. {max}"));
    }
    lines
}

fn push_capability_rows(lines: &mut Vec<String>, caps: &[crate::app::permissions::Capability]) {
    for cap in caps {
        let sensitive = if cap.is_sensitive() {
            " [sensitive]"
        } else {
            ""
        };
        lines.push(format!(
            "  {:<18}{}{sensitive}",
            cap.as_str(),
            cap.description()
        ));
    }
}

fn push_wasm_capability_rows(lines: &mut Vec<String>, caps: &[String]) {
    for cap in caps {
        let sensitive = if crate::app::permissions::wasm_capability_requires_consent(cap) {
            " [sensitive]"
        } else {
            ""
        };
        lines.push(format!(
            "  {:<28}{}{sensitive}",
            cap,
            crate::app::permissions::wasm_capability_description(cap)
        ));
    }
}

/// Host-version gate: block install when the host is too old for the app (or a
/// declared requirement is malformed); warn when the host is newer than the
/// app's declared ceiling. Returns an exit code to abort, or `None` to proceed.
pub fn host_version_gate(report: &crate::app::package::PackageReport) -> Option<i32> {
    use crate::app::host_version::{check, current};
    let verdict = check(
        report.requires_plexi_min.as_deref(),
        report.requires_plexi_max.as_deref(),
        current(),
    );
    match verdict.message() {
        None => None,
        Some(msg) if verdict.is_blocking() => {
            eprintln!("error: {msg}");
            log::warn!("install: host-version gate blocked '{}': {msg}", report.id);
            Some(1)
        }
        Some(msg) => {
            eprintln!("warning: {msg}");
            log::info!("install: host-version warning for '{}': {msg}", report.id);
            None
        }
    }
}

/// Resolve the trust label for a report against the bundled core pack ids.
fn trust_label_for(
    report: &crate::app::package::PackageReport,
    marketplace_reviewed: bool,
) -> crate::app::package::TrustLabel {
    let core_ids = crate::cli::install_host::core_pack_ids();
    let core_refs: Vec<&str> = core_ids.iter().map(String::as_str).collect();
    crate::app::package::trust_label(report, &core_refs, marketplace_reviewed)
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

fn parse_wasm_optional_selection(answer: &str, optional: &[String]) -> Result<Vec<String>, String> {
    let trimmed = answer.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return Ok(Vec::new());
    }
    if trimmed.eq_ignore_ascii_case("all") {
        return Ok(optional.to_vec());
    }

    let mut selected = Vec::new();
    for raw in trimmed.split(',') {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        let index = token
            .parse::<usize>()
            .map_err(|_| format!("invalid optional WASM grant selection '{token}'"))?;
        if index == 0 || index > optional.len() {
            return Err(format!(
                "optional WASM grant selection {index} is out of range 1..{}",
                optional.len()
            ));
        }
        let capability = optional[index - 1].clone();
        if !selected.contains(&capability) {
            selected.push(capability);
        }
    }
    Ok(selected)
}

fn prompt_wasm_optional_grants(
    report: &crate::app::package::PackageReport,
    assume_yes: bool,
    is_tty: bool,
    reader: &mut impl io::BufRead,
) -> Result<Vec<String>, String> {
    if report.wasm_optional_capabilities.is_empty() || assume_yes {
        return Ok(Vec::new());
    }
    if !is_tty {
        return Ok(Vec::new());
    }

    eprintln!("Optional WASM grants to remember for this launch directory:");
    for (index, capability) in report.wasm_optional_capabilities.iter().enumerate() {
        eprintln!("  {}. {}", index + 1, capability);
    }
    eprint!("Grant optional WASM capabilities? [none/all/1,3] ");
    let _ = io::stderr().flush();
    let mut answer = String::new();
    reader
        .read_line(&mut answer)
        .map_err(|e| format!("failed to read optional WASM grant selection: {e}"))?;
    parse_wasm_optional_selection(&answer, &report.wasm_optional_capabilities)
}

/// Run the full trust gate for an interactive install: print the trust sheet,
/// then prompt. Returns selected install-review decisions or an exit code.
fn run_install_gate(
    report: &crate::app::package::PackageReport,
    assume_yes: bool,
    marketplace_reviewed: bool,
) -> Result<InstallGateDecision, i32> {
    use std::io::IsTerminal;
    let label = trust_label_for(report, marketplace_reviewed);
    print_trust_sheet(report, label);
    // Host-version compatibility: a too-old host (or malformed requirement)
    // aborts before the confirm prompt; a too-new host only warns.
    if let Some(code) = host_version_gate(report) {
        return Err(code);
    }
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
        Ok(true) => match prompt_wasm_optional_grants(report, assume_yes, is_tty, &mut stdin) {
            Ok(selected_wasm_optional) => Ok(InstallGateDecision {
                selected_wasm_optional,
            }),
            Err(e) => {
                eprintln!("error: {e}");
                Err(1)
            }
        },
        Ok(false) => {
            eprintln!("install aborted — '{}' was not installed", report.id);
            Err(1)
        }
        Err(e) => {
            eprintln!("error: {e}");
            Err(1)
        }
    }
}

fn is_valid_app_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
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
            print_trust_sheet(&report, trust_label_for(&report, false));
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
    app_install_with_pin_inner(path, pin, confirm, assume_yes, None)
}

fn app_install_with_pin_inner(
    path: &str,
    pin: Option<&str>,
    confirm: InstallConfirm,
    assume_yes: bool,
    preapproved_gate: Option<InstallGateDecision>,
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
    let mut validated_report = None;
    let mut gate_decision = preapproved_gate.unwrap_or_default();
    match crate::app::package::validate_dir(&src) {
        Ok(report) => {
            if confirm == InstallConfirm::Interactive {
                match run_install_gate(&report, assume_yes, false) {
                    Ok(decision) => {
                        gate_decision = decision;
                    }
                    Err(code) => return code,
                }
            }
            validated_report = Some(report);
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
        Some(id) if is_valid_app_id(id) => id.to_string(),
        _ => {
            eprintln!("error: manifest.toml is missing a valid [app].id");
            eprintln!("  (IDs must be non-empty and contain only alphanumeric characters, dots, dashes, or underscores)");
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
    if let Some(report) = validated_report.as_ref() {
        remember_wasm_install_review(report, &gate_decision.selected_wasm_optional);
    }

    log::info!(
        "app::install: installed {app_id} v{app_version} from {}",
        src.display()
    );
    println!(
        "Installed '{app_id}' v{app_version} from {}.",
        src.display()
    );
    println!("Run `plexi app open {app_id}` to launch it.");
    0
}

fn remember_wasm_install_review(
    report: &crate::app::package::PackageReport,
    selected_wasm_optional: &[String],
) {
    let workspace_root = match std::env::current_dir() {
        Ok(path) => path,
        Err(e) => {
            log::warn!(
                "app::install: could not resolve cwd for wasm install review '{}': {e}",
                report.id
            );
            return;
        }
    };
    let Some(summary) = persist_wasm_install_review(
        report,
        selected_wasm_optional,
        &crate::config::config_dir(),
        &workspace_root,
    ) else {
        return;
    };
    log::info!(
        "app::install: wasm review app={} workspace={} required_granted={} optional_granted={} optional_deferred={}",
        report.id,
        workspace_root.display(),
        summary.required_granted,
        summary.optional_granted,
        summary.optional_deferred
    );
    if summary.required_granted > 0 || summary.optional_granted > 0 {
        println!(
            "Remembered WASM grants for '{}': {} required, {} optional.",
            report.id, summary.required_granted, summary.optional_granted
        );
    }
}

fn persist_wasm_install_review(
    report: &crate::app::package::PackageReport,
    selected_wasm_optional: &[String],
    config_dir: &std::path::Path,
    workspace_root: &std::path::Path,
) -> Option<crate::app::permissions::WasmInstallGrantSummary> {
    if report.wasm_required_capabilities.is_empty() && report.wasm_optional_capabilities.is_empty()
    {
        return None;
    }
    let selected: std::collections::HashSet<String> =
        selected_wasm_optional.iter().cloned().collect();
    let mut store = crate::app::permissions::PermissionStore::load_or_default(config_dir);
    let summary = store.apply_wasm_install_review(
        &report.id,
        workspace_root,
        &report.wasm_required_capabilities,
        &report.wasm_optional_capabilities,
        &selected,
    );
    store.save();
    Some(summary)
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
    app_install_package_inner(file, pin, confirm, assume_yes, false, None)
}

pub fn app_install_marketplace_package(
    file: &str,
    pin: Option<&str>,
    confirm: InstallConfirm,
    assume_yes: bool,
    source_metadata: Option<crate::app::marketplace::InstalledRegistrySource>,
) -> i32 {
    app_install_package_inner(file, pin, confirm, assume_yes, true, source_metadata)
}

fn app_install_package_inner(
    file: &str,
    pin: Option<&str>,
    confirm: InstallConfirm,
    assume_yes: bool,
    marketplace_reviewed: bool,
    source_metadata: Option<crate::app::marketplace::InstalledRegistrySource>,
) -> i32 {
    log::info!("app_install:cli: package file={file} pin={pin:?} confirm={confirm:?}");
    let pkg_path = match std::path::Path::new(file).canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: could not resolve {file}: {e}");
            return 1;
        }
    };

    let report = match if marketplace_reviewed {
        crate::app::package::validate_package_reviewed_native(&pkg_path)
    } else {
        crate::app::package::validate_package(&pkg_path)
    } {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: package validation failed — refusing install: {e}");
            return 1;
        }
    };

    let gate_decision = if confirm == InstallConfirm::Interactive {
        match run_install_gate(&report, assume_yes, marketplace_reviewed) {
            Ok(decision) => Some(decision),
            Err(code) => return code,
        }
    } else {
        None
    };

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
    let code = app_install_with_pin_inner(
        &staging.to_string_lossy(),
        pin,
        InstallConfirm::PreApproved,
        assume_yes,
        gate_decision,
    );
    if code == 0 {
        if let Some(metadata) = source_metadata {
            let app_dir = crate::app::registry::apps_dir().join(&report.id);
            if let Err(e) = metadata.write_to(&app_dir) {
                log::warn!(
                    "app_install:cli: could not record marketplace source for '{}': {e}",
                    report.id
                );
            }
        }
    }
    if let Err(e) = std::fs::remove_dir_all(&staging) {
        log::warn!(
            "app_install:cli: could not clean up staging dir {}: {e}",
            staging.display()
        );
    }
    code
}

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> io::Result<()> {
    copy_dir_all_inner(src, dst, std::path::Path::new(""))
}

fn copy_dir_all_inner(
    src: &std::path::Path,
    dst: &std::path::Path,
    rel: &std::path::Path,
) -> io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let entry_path = entry.path();
        let rel_path = rel.join(entry.file_name());
        if crate::app::package::is_generated_dev_artifact_rel(&rel_path) {
            log::info!(
                "app::install: skipping generated dev artifact {}",
                entry_path.display()
            );
            continue;
        }
        let dst_path = dst.join(entry.file_name());
        if entry_path.is_dir() {
            copy_dir_all_inner(&entry_path, &dst_path, &rel_path)?;
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
        match serde_json::from_str::<serde_json::Value>(&json) {
            Ok(v) if v.is_object() => Some(v),
            Ok(_) => {
                eprintln!(
                    "error: --state must be a plain JSON object, for example {{\"count\": 3}}"
                );
                return 1;
            }
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
    let (app_id, app_dir, app_bin, capabilities, allowed_hosts) = if std::path::Path::new(app)
        .components()
        .count()
        > 1
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
        (
            manifest.app.id,
            app_dir,
            entry,
            manifest.app.capabilities.capabilities,
            manifest.app.capabilities.allowed_hosts,
        )
    } else {
        // ID-based: registry lookup
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let registry = crate::app::registry::AppRegistry::load(&cwd);
        match registry.list().into_iter().find(|a| a.manifest.id == app) {
            Some(a) => {
                let app_dir = a
                    .bin_path
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_else(|| a.bin_path.clone());
                (
                    a.manifest.id.clone(),
                    app_dir,
                    a.bin_path.clone(),
                    a.manifest.capabilities.capabilities.clone(),
                    a.manifest.capabilities.allowed_hosts.clone(),
                )
            }
            None => {
                eprintln!(
                    "error: app '{app}' not found — run `plexi app list` to see installed apps"
                );
                return 1;
            }
        }
    };

    if app_bin.extension().and_then(|ext| ext.to_str()) != Some("py") {
        eprintln!(
            "error: '{}' is not a Python entry — plexi app render only drives SDK v3 Python apps (CPython-in-WASM)",
            app_bin.display()
        );
        return 1;
    }
    let launch_config = crate::cli::app_check::python_launch_config_from_parts(
        &app_id,
        &app_dir,
        &app_bin,
        &capabilities,
        &allowed_hosts,
    );
    let tree = match crate::host::wasm_python::run_headless_frame(
        &launch_config,
        (width as f32, height as f32),
        seed_state,
    ) {
        Ok(tree) => tree,
        Err(e) => {
            eprintln!("error: render failed: {e}");
            return 1;
        }
    };

    if png {
        // PNG mode: rasterize and write binary
        let png_bytes = match crate::host::wasm_render::render_ui_tree_to_png(
            &tree,
            width as f32,
            height as f32,
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
        // JSON mode (default): return the semantic UI-node tree
        let json =
            match serde_json::to_string_pretty(&crate::cli::app_check::ui_tree_to_json(&tree)) {
                Ok(j) => j,
                Err(e) => {
                    eprintln!("error: could not serialize frame: {e}");
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppInstallSpecKind {
    Package,
    Source,
    LocalPath,
    MarketplaceId,
}

/// Classify the user-provided `plexi app install <spec>` target before any
/// path canonicalization. Source specs such as `github:owner/repo` contain `/`
/// but are not filesystem paths.
pub(crate) fn classify_app_install_spec(s: &str) -> AppInstallSpecKind {
    if s.ends_with(".plexipkg") {
        return AppInstallSpecKind::Package;
    }
    if crate::app::packs::parse_source_spec(s).is_ok() {
        return AppInstallSpecKind::Source;
    }
    if s.contains('/') || s.starts_with('.') || std::path::Path::new(s).is_dir() {
        return AppInstallSpecKind::LocalPath;
    }
    AppInstallSpecKind::MarketplaceId
}

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

/// `plexi app update [<id>]` — canonical app update path.
///
/// Pulls git-backed installed apps in the current workspace-aware registry.
/// `plexi update apps` delegates to the same implementation for compatibility.
pub fn app_update_cli(id: Option<&str>) -> i32 {
    log::info!("app_update:cli: delegating to workspace-aware git updater id={id:?}");
    crate::cli::install::update_cli(id)
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
    use super::{classify_app_install_spec, AppInstallSpecKind};
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
    fn install_accepts_reverse_dns_app_id() {
        let src = TempDir::new().unwrap();
        let app_id = "com.plexi.test-install";
        write_valid_manifest(src.path(), app_id);
        let path = src.path().to_string_lossy().to_string();

        let code =
            super::app_install_with_pin(&path, None, super::InstallConfirm::Interactive, true);

        let dest = crate::app::registry::apps_dir().join(app_id);
        let _ = std::fs::remove_dir_all(&dest);

        assert_eq!(code, 0, "reverse-DNS app ids must install cleanly");
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

    #[test]
    fn github_source_spec_is_not_classified_as_local_path() {
        assert_eq!(
            classify_app_install_spec("github:owner/repo"),
            AppInstallSpecKind::Source
        );
        assert_eq!(
            classify_app_install_spec("git+file:///tmp/repo"),
            AppInstallSpecKind::Source
        );
        assert_eq!(
            classify_app_install_spec("owner/repo"),
            AppInstallSpecKind::LocalPath
        );
    }
}

#[cfg(test)]
mod install_confirm_tests {
    use super::{
        confirm_install, human_size, parse_wasm_optional_selection, prompt_wasm_optional_grants,
        trust_sheet_lines, InstallConfirm,
    };
    use crate::app::package::{PackageReport, PackageRuntime, TrustLabel};
    use crate::app::permissions::{PermissionState, PermissionStore};
    use std::io::Cursor;

    fn report() -> PackageReport {
        PackageReport {
            id: "gate-test".to_string(),
            name: "Gate Test".to_string(),
            version: "0.1.0".to_string(),
            runtime: PackageRuntime::Python,
            entry: "main.py".to_string(),
            capabilities: Vec::new(),
            wasm_required_capabilities: Vec::new(),
            wasm_optional_capabilities: Vec::new(),
            file_count: 2,
            total_size: 64,
            requires_plexi_min: None,
            requires_plexi_max: None,
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

    #[test]
    fn trust_sheet_lists_wasm_required_and_optional_capabilities() {
        let mut r = report();
        r.runtime = PackageRuntime::Wasm;
        r.entry = "app.wasm".to_string();
        r.wasm_required_capabilities = vec!["state:read-write".to_string()];
        r.wasm_optional_capabilities = vec!["ai.query".to_string()];

        let lines = trust_sheet_lines(&r, TrustLabel::SandboxedWasm);
        assert!(lines
            .iter()
            .any(|line| line == "wasm required capabilities:"));
        assert!(lines.iter().any(|line| line.contains("state:read-write")));
        assert!(lines
            .iter()
            .any(|line| line == "wasm optional capabilities:"));
        assert!(lines.iter().any(|line| line.contains("ai.query")));
    }

    #[test]
    fn trust_sheet_uses_honest_runtime_labels() {
        let mut r = report();
        let reviewed_lines = trust_sheet_lines(&r, TrustLabel::ReviewedNative);
        assert!(reviewed_lines.iter().any(|line| {
            line == "runtime:      python — Reviewed native process — human-reviewed; not sandboxed"
        }));

        r.runtime = PackageRuntime::Wasm;
        r.entry = "app.wasm".to_string();
        let wasm_lines = trust_sheet_lines(&r, TrustLabel::SandboxedWasm);
        assert!(wasm_lines.iter().any(|line| {
            line == "runtime:      wasm — Sandboxed WASM — scoped host imports are capability-gated"
        }));

        let core_lines = trust_sheet_lines(&report(), TrustLabel::FirstPartyCore);
        assert!(core_lines
            .iter()
            .any(|line| line == "runtime:      python — First-party core — bundled with Plexi"));
    }

    #[test]
    fn marketplace_reviewed_helper_uses_reviewed_native_label() {
        let r = report();
        assert_eq!(
            super::trust_label_for(&r, false),
            TrustLabel::PythonUnreviewed
        );
        assert_eq!(super::trust_label_for(&r, true), TrustLabel::ReviewedNative);
    }

    #[test]
    fn wasm_optional_selection_accepts_none_all_and_indices() {
        let optional = vec![
            "ai.query".to_string(),
            "net:fetch:api.example.com".to_string(),
            "state:read-write".to_string(),
        ];

        assert_eq!(
            parse_wasm_optional_selection("", &optional).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            parse_wasm_optional_selection("none", &optional).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            parse_wasm_optional_selection("all", &optional).unwrap(),
            optional
        );
        assert_eq!(
            parse_wasm_optional_selection("2, 1, 2", &optional).unwrap(),
            vec![
                "net:fetch:api.example.com".to_string(),
                "ai.query".to_string()
            ]
        );
        assert!(parse_wasm_optional_selection("4", &optional).is_err());
        assert!(parse_wasm_optional_selection("ai.query", &optional).is_err());
    }

    #[test]
    fn wasm_optional_prompt_skips_assume_yes_and_reads_tty_selection() {
        let mut r = report();
        r.runtime = PackageRuntime::Wasm;
        r.wasm_optional_capabilities = vec![
            "ai.query".to_string(),
            "net:fetch:api.example.com".to_string(),
        ];

        let skipped =
            prompt_wasm_optional_grants(&r, true, true, &mut Cursor::new(b"all\n")).unwrap();
        assert!(
            skipped.is_empty(),
            "--yes must not silently grant optional WASM capabilities"
        );

        let selected =
            prompt_wasm_optional_grants(&r, false, true, &mut Cursor::new(b"2\n")).unwrap();
        assert_eq!(selected, vec!["net:fetch:api.example.com".to_string()]);
    }

    #[test]
    fn wasm_install_review_persistence_writes_selected_scope() {
        let config = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut r = report();
        r.id = "wasm-install-review".to_string();
        r.runtime = PackageRuntime::Wasm;
        r.wasm_required_capabilities = vec!["state:read-write".to_string()];
        r.wasm_optional_capabilities = vec![
            "ai.query".to_string(),
            "net:fetch:api.example.com".to_string(),
        ];

        let summary = super::persist_wasm_install_review(
            &r,
            &["net:fetch:api.example.com".to_string()],
            config.path(),
            workspace.path(),
        )
        .expect("wasm review should persist");

        assert_eq!(summary.required_granted, 1);
        assert_eq!(summary.optional_granted, 1);
        assert_eq!(summary.optional_deferred, 1);

        let store = PermissionStore::load_or_default(config.path());
        assert_eq!(
            store.get_wasm("wasm-install-review", workspace.path(), "state:read-write"),
            Some(PermissionState::Green)
        );
        assert_eq!(
            store.get_wasm("wasm-install-review", workspace.path(), "ai.query"),
            Some(PermissionState::Yellow)
        );
        assert_eq!(
            store.get_wasm(
                "wasm-install-review",
                workspace.path(),
                "net:fetch:api.example.com"
            ),
            Some(PermissionState::Green)
        );
        assert_eq!(
            store.get_wasm(
                "wasm-install-review",
                tempfile::tempdir().unwrap().path(),
                "net:fetch:api.example.com"
            ),
            None,
            "raw WASM install grants must remain workspace-scoped"
        );
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
    fn inspect_wasm_manifest_returns_0() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("manifest.toml"),
            "schema_version = 1\n\n\
             [app]\n\
             id = \"com.plexi.inspect-wasm\"\n\
             type = \"wasm\"\n\
             name = \"Inspect WASM\"\n\
             entry = \"app.wasm\"\n\
             version = \"0.1.0\"\n\
             description = \"Test\"\n\n\
             [app.capabilities]\n\
             capabilities = [\"gpu.render\"]\n\n\
             [launch]\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("app.wasm"), b"\0asm").unwrap();
        let code = super::app_inspect_cli(&dir.path().to_string_lossy());
        assert_eq!(code, 0, "valid wasm app dir must inspect cleanly");
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
    fn installed_version_matches_manifest_version_when_written() {
        let dir = TempDir::new().unwrap();
        // Write manifest with version 0.1.0.
        write_manifest(dir.path(), "test-app", "0.1.0");
        // Write installed_version.txt matching manifest.
        super::write_installed_version(dir.path(), "0.1.0");

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

    #[test]
    fn marketplace_package_install_records_source_metadata() {
        let profile = TempDir::new().unwrap();
        let _profile_guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        let app_src = TempDir::new().unwrap();
        write_manifest(app_src.path(), "reviewed-notes", "0.1.0");
        let pkg = app_src.path().join("reviewed-notes-0.1.0.plexipkg");
        crate::app::package::build_package(app_src.path(), Some(&pkg)).unwrap();

        let metadata = crate::app::marketplace::InstalledRegistrySource {
            schema_version: crate::app::marketplace::MARKETPLACE_SCHEMA_VERSION,
            source: "hosted-registry".to_string(),
            registry_url: "https://plexiapp.com/registry/v1/index.json".to_string(),
            app_id: "reviewed-notes".to_string(),
            version: "0.1.0".to_string(),
            publisher: "plexi".to_string(),
            checksum: "abc123".to_string(),
            package_url: "https://plexiapp.com/registry/v1/packages/abc123.plexipkg".to_string(),
            reviewed_native: true,
        };

        let code = super::app_install_marketplace_package(
            &pkg.to_string_lossy(),
            None,
            super::InstallConfirm::PreApproved,
            true,
            Some(metadata.clone()),
        );

        assert_eq!(code, 0);
        let source_path = profile
            .path()
            .join("apps")
            .join("reviewed-notes")
            .join(crate::app::marketplace::INSTALLED_REGISTRY_SOURCE_FILE);
        let text = std::fs::read_to_string(source_path).unwrap();
        let parsed: crate::app::marketplace::InstalledRegistrySource =
            toml::from_str(&text).unwrap();
        assert_eq!(parsed, metadata);
    }
}

#[cfg(test)]
mod scaffold_marketplace_tests {
    use super::*;
    use tempfile::TempDir;

    /// Beta/alpha scaffolds emit a commented top-level `[marketplace]`
    /// placeholder naming `publisher`. The comment must neither break parsing
    /// nor produce a live `[marketplace]` section.
    fn assert_placeholder(manifest: &str) {
        assert!(
            manifest.contains("# [marketplace]"),
            "manifest should carry a commented [marketplace] placeholder"
        );
        assert!(
            manifest.contains("# publisher"),
            "placeholder should name the required publisher field"
        );
        let parsed: crate::app::registry::AppManifest =
            toml::from_str(manifest).expect("scaffolded manifest must parse");
        assert!(
            parsed.marketplace.is_none(),
            "commented placeholder must not produce a live [marketplace] section"
        );
    }

    fn assert_no_placeholder(manifest: &str) {
        assert!(
            !manifest.contains("# [marketplace]"),
            "stable scaffolds should not advertise marketplace publishing"
        );
        let parsed: crate::app::registry::AppManifest =
            toml::from_str(manifest).expect("scaffolded manifest must parse");
        assert!(parsed.marketplace.is_none());
    }

    fn manifest_for(scaffold: fn(&std::path::Path, &str) -> io::Result<()>) -> String {
        let dir = TempDir::new().unwrap();
        let app_dir = dir.path().join("myapp");
        std::fs::create_dir_all(&app_dir).unwrap();
        scaffold(&app_dir, "myapp").unwrap();
        std::fs::read_to_string(app_dir.join("manifest.toml")).unwrap()
    }

    #[test]
    fn stable_python_scaffold_omits_marketplace_placeholder() {
        assert_no_placeholder(&manifest_for(scaffold_python_app));
    }

    #[test]
    fn python_scaffold_writes_appharness_test() {
        let dir = TempDir::new().unwrap();
        let app_dir = dir.path().join("myapp");
        std::fs::create_dir_all(&app_dir).unwrap();
        scaffold_python_app(&app_dir, "myapp").unwrap();

        let test_path = app_dir.join("tests").join("test_app.py");
        assert!(
            test_path.is_file(),
            "python scaffold must write tests/test_app.py"
        );
        let test_src = std::fs::read_to_string(&test_path).unwrap();
        assert!(
            test_src.contains("AppHarness"),
            "generated test must exemplify AppHarness"
        );
        assert!(
            test_src.contains("assert_no_overlap"),
            "generated test must assert no layout overlap"
        );
        assert!(
            !test_src.contains("__DISPLAY_NAME__"),
            "generated test must substitute the display-name placeholder"
        );
    }

    #[test]
    fn python_scaffold_writes_agent_contract_gitignore_and_metadata() {
        let dir = TempDir::new().unwrap();
        let app_dir = dir.path().join("myapp");
        std::fs::create_dir_all(&app_dir).unwrap();
        scaffold_python_app(&app_dir, "myapp").unwrap();

        let agents = std::fs::read_to_string(app_dir.join("AGENTS.md")).unwrap();
        assert!(
            agents.contains("Use TDD"),
            "AGENTS.md must tell agents to use TDD"
        );
        assert!(
            agents.contains("plexi app test ."),
            "AGENTS.md must teach regular app tests"
        );
        assert!(
            agents.contains("PLEXI_CHANNEL=alpha plexi app check . --png-dir"),
            "AGENTS.md must teach explicit-channel check gate"
        );
        assert!(
            agents.contains("plexi-pr-123 app check . --png-dir"),
            "AGENTS.md must teach direct PR-channel check syntax"
        );
        assert!(
            agents.contains("plexi app action <pane-id>"),
            "AGENTS.md must teach exercising app actions"
        );
        assert!(
            agents.contains("Hot reload is part of the dev loop"),
            "AGENTS.md must teach same-pane hot reload validation"
        );
        assert!(
            agents.contains("padding=SPACE_MD"),
            "AGENTS.md must teach the semantic shell padding contract"
        );
        assert!(
            agents.contains("log.debug"),
            "AGENTS.md must teach SDK log levels"
        );

        let gitignore = std::fs::read_to_string(app_dir.join(".gitignore")).unwrap();
        assert!(gitignore.contains(".venv/"));
        assert!(gitignore.contains("__pycache__/"));
        assert!(gitignore.contains("*.pyc"));
        assert!(gitignore.contains(".pytest_cache/"));
        assert!(gitignore.contains("render-output/"));
        assert!(gitignore.contains("agent-run-logs/"));
        assert!(
            !gitignore.contains("manifest.toml"),
            ".gitignore must not hide the manifest"
        );
        assert!(
            !gitignore.contains("tests/"),
            ".gitignore must not hide tests"
        );
        assert!(
            !gitignore.contains("fixtures/"),
            ".gitignore must not hide fixtures"
        );
        assert_eq!(
            std::fs::read_to_string(app_dir.join("fixtures/state.json")).unwrap(),
            "{\n  \"count\": 3\n}\n"
        );

        let metadata_raw = std::fs::read_to_string(app_dir.join(SCAFFOLD_METADATA_FILE)).unwrap();
        let metadata: toml::Value = toml::from_str(&metadata_raw).unwrap();
        assert_eq!(
            metadata
                .get("schema_version")
                .and_then(toml::Value::as_integer),
            Some(SCAFFOLD_METADATA_SCHEMA_VERSION as i64)
        );
        assert_eq!(
            metadata
                .get("plexi_cli_version")
                .and_then(toml::Value::as_str),
            Some(env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(
            metadata.get("sdk_version").and_then(toml::Value::as_str),
            Some(python_sdk_version().as_str())
        );
        assert_eq!(
            metadata
                .get("manifest_schema_version")
                .and_then(toml::Value::as_integer),
            Some(crate::app::registry::MANIFEST_SCHEMA_VERSION as i64)
        );
        assert_eq!(
            metadata
                .get("python_runtime_version")
                .and_then(toml::Value::as_str),
            Some(crate::app::python_env::PYTHON_APP_VENV_VERSION)
        );
        assert_eq!(
            metadata
                .get("template_version")
                .and_then(toml::Value::as_integer),
            Some(PYTHON_SCAFFOLD_TEMPLATE_VERSION as i64)
        );
        assert!(metadata
            .get("channel")
            .and_then(toml::Value::as_str)
            .is_some());
        assert!(metadata
            .get("profile_dir")
            .and_then(toml::Value::as_str)
            .is_some());
    }

    #[test]
    fn python_scaffold_writes_self_documenting_main() {
        let dir = TempDir::new().unwrap();
        let app_dir = dir.path().join("myapp");
        std::fs::create_dir_all(&app_dir).unwrap();
        scaffold_python_app(&app_dir, "myapp").unwrap();

        let main_src = std::fs::read_to_string(app_dir.join("main.py")).unwrap();
        assert!(
            main_src.contains("SDK v3 app generated by `plexi app init`"),
            "generated main.py should identify the SDK v3 scaffold"
        );
        assert!(
            main_src.contains("init(size, args)"),
            "generated main.py should document the lifecycle function names"
        );
        assert!(
            main_src.contains("Do not mutate Plexi state in-place"),
            "generated main.py should explain effect-returned state changes"
        );
        assert!(
            main_src.contains("Components describe UI. Effects describe host work."),
            "generated main.py should explain component/effect separation"
        );
        assert!(
            main_src.contains("ActionBar("),
            "generated main.py should demonstrate the standard action-row primitive"
        );
        assert!(
            main_src.contains("Card("),
            "generated main.py should demonstrate the standard surface primitive"
        );
        assert!(
            main_src.contains("Section("),
            "generated main.py should demonstrate semantic section chrome"
        );
        assert!(
            main_src.contains("Badge("),
            "generated main.py should demonstrate semantic badges"
        );
        assert!(
            main_src.contains("Divider("),
            "generated main.py should demonstrate semantic dividers"
        );
        assert!(
            main_src.contains("TextEdit("),
            "generated main.py should demonstrate the host-rendered text edit primitive"
        );
        assert!(
            main_src.contains("SelectList("),
            "generated main.py should demonstrate the host-rendered select list primitive"
        );
        assert!(
            main_src.contains("Scrollable("),
            "generated main.py should keep proof components inside a scroll body"
        );
        assert!(
            main_src.contains("UiValueChange"),
            "generated main.py should handle editable component value changes"
        );
        assert!(
            main_src.contains("FooterKeys("),
            "generated main.py should keep shortcut hints in the footer"
        );
        assert!(
            main_src.contains("SPACE_MD"),
            "generated main.py should keep semantic shell content inset"
        );
        assert!(
            main_src.contains("padding=SPACE_MD"),
            "generated main.py must keep root semantic shell content padding"
        );
        assert!(main_src.contains("log.debug"));
        assert!(main_src.contains("log.info"));
        assert!(main_src.contains("log.warn"));
        assert!(main_src.contains("log.error"));
    }

    #[test]
    fn stable_agent_scaffold_omits_marketplace_placeholder() {
        assert_no_placeholder(&manifest_for(scaffold_agent_python_app));
    }

    #[test]
    fn stable_rust_scaffold_omits_marketplace_placeholder() {
        assert_no_placeholder(&manifest_for(scaffold_rust_app));
    }

    #[test]
    fn wasm_scaffold_has_component_workspace_manifest_and_wit_link() {
        let temp = tempfile::tempdir().unwrap();
        let app_dir = temp.path().join("my-counter");
        std::fs::create_dir(&app_dir).unwrap();

        scaffold_wasm_app(&app_dir, "my-counter").unwrap();

        let manifest: toml::Value =
            toml::from_str(&std::fs::read_to_string(app_dir.join("manifest.toml")).unwrap())
                .unwrap();
        assert_eq!(manifest["app"]["type"].as_str(), Some("wasm"));
        assert_eq!(
            manifest["app"]["entry"].as_str(),
            Some("target/wasm32-wasip1/release/my_counter.wasm")
        );
        assert!(app_dir.join("app/src/lib.rs").is_file());
        assert_eq!(
            std::fs::read_link(app_dir.join("wit")).unwrap(),
            std::path::Path::new(".plexi-sdk/wit")
        );
        assert!(app_dir
            .join(".plexi-sdk/plexi-wasm-sdk/src/lib.rs")
            .is_file());
        let readme = std::fs::read_to_string(app_dir.join("README.md")).unwrap();
        assert!(readme.contains("[Plexi WASM authoring guide](AUTHORING.md)"));
        assert!(app_dir.join("AUTHORING.md").is_file());
        let source = std::fs::read_to_string(app_dir.join("app/src/lib.rs")).unwrap();
        assert!(source.contains("InputEvent::UiAction"));
        assert!(source.contains("InputEvent::Key"));
    }

    #[test]
    fn beta_scaffolds_keep_marketplace_placeholder() {
        let _channel = crate::config::set_test_channel("beta");
        assert_placeholder(&manifest_for(scaffold_python_app));
        assert_placeholder(&manifest_for(scaffold_agent_python_app));
        assert_placeholder(&manifest_for(scaffold_rust_app));
    }
}

#[cfg(test)]
mod app_init_collision_tests {
    use super::describe_app_init_collision;
    use tempfile::TempDir;

    /// stint 0428: a name collision on `app init` must name the occupying
    /// app's id/type and suggest a concrete alternative name, so an agent
    /// (or the assistant's build-plexi-app skill) never pivots to silently
    /// editing whatever it finds there instead.
    #[test]
    fn collision_with_valid_manifest_names_id_and_type() {
        let dir = TempDir::new().unwrap();
        let app_dir = dir.path().join("counter");
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::write(
            app_dir.join("manifest.toml"),
            "schema_version = 1\n\n\
             [app]\n\
             id = \"counter\"\n\
             type = \"wasm\"\n\
             name = \"Counter\"\n\
             entry = \"app.wasm\"\n\
             version = \"0.1.0\"\n\
             description = \"Test\"\n\n\
             [app.capabilities]\n\
             capabilities = []\n\n\
             [launch]\n",
        )
        .unwrap();

        let msg = describe_app_init_collision(&app_dir, "counter");
        assert!(msg.contains("already exists"), "message: {msg}");
        assert!(msg.contains("id=\"counter\""), "message: {msg}");
        assert!(msg.contains("Wasm"), "message: {msg}");
        assert!(
            msg.contains("plexi app init counter-2"),
            "must suggest a concrete alternative name: {msg}"
        );
    }

    /// A colliding directory with no parseable manifest still gets an
    /// actionable message — never a bare "already exists" with no next step.
    #[test]
    fn collision_without_manifest_falls_back_to_generic_message() {
        let dir = TempDir::new().unwrap();
        let app_dir = dir.path().join("stale-dir");
        std::fs::create_dir_all(&app_dir).unwrap();

        let msg = describe_app_init_collision(&app_dir, "stale-dir");
        assert!(msg.contains("already exists"), "message: {msg}");
        assert!(
            msg.contains("plexi app init stale-dir-2"),
            "must suggest a concrete alternative name: {msg}"
        );
    }
}
