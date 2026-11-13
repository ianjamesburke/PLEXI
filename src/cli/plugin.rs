use std::io::{self, Write};

pub(crate) const SCAFFOLD_METADATA_FILE: &str = "plexi.scaffold.toml";
pub(super) const SCAFFOLD_METADATA_SCHEMA_VERSION: u32 = 1;
pub(super) const PYTHON_SCAFFOLD_TEMPLATE_VERSION: u32 = 6;

/// Detect the channel config dir name from the running binary name.
pub(super) fn plugin_init_config_dir() -> String {
    crate::config::config_dir()
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .unwrap_or_else(crate::config::workspace_channel_dir)
}

/// Build an actionable error message for `plugin init` when `plugin_dir` already
/// exists. Reuses `PluginManifest` (never a hand-rolled TOML parse — see
/// src/cli/AGENTS.md / root AGENTS.md regex-vs-parser rule) to name what's
/// already occupying the slot, then suggests a single alternative name so the
/// caller (including the assistant's `build-plexi-plugin` skill) can retry
/// immediately instead of pivoting to edit a stale/foreign plugin in place
/// (stint 0428).
pub(super) fn describe_plugin_init_collision(plugin_dir: &std::path::Path, name: &str) -> String {
    let manifest_path = plugin_dir.join("manifest.toml");
    let existing = std::fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|raw| toml::from_str::<crate::app::registry::PluginManifest>(&raw).ok());

    let mut msg = match existing {
        Some(manifest) => format!(
            "error: {} already exists (id=\"{}\", type={:?}) — pick a different name",
            plugin_dir.display(),
            manifest.plugin.id,
            manifest.plugin.manifest_type
        ),
        None => format!(
            "error: {} already exists — pick a different name",
            plugin_dir.display()
        ),
    };
    msg.push_str(&format!("\n  Try: plexi plugin init {name}-2"));
    msg
}

/// Where `plugin init <name>` without `--global` scaffolds when run from
/// `cwd`: `<workspace>/<channel_dir>/plugins/<name>`, or `None` outside a
/// workspace.
fn plugin_init_workspace_dir(cwd: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let channel_dir = plugin_init_config_dir();
    // The registry's walk stops at `$HOME` and at the login home, so a
    // redirected `HOME` never makes the installed profile dir a workspace
    // (stint 1337).
    let root = crate::app::registry::resolve_workspace_root_with_channel(cwd, &channel_dir);
    log::info!(
        "plugin_init: workspace walk from {} for {channel_dir}/ found {:?}",
        cwd.display(),
        root.as_ref().map(|r| r.display().to_string())
    );
    root.map(|root| root.join(&channel_dir).join("plugins").join(name))
}

/// `plexi plugin init [--lang python|rust] [--global] <name> [--open]`
///
/// Without `--global`: walks up from CWD looking for the nearest workspace
/// (ancestor with `<channel_dir>/`). If none is found, exits with an error
/// directing the user to pass `--global` or `cd` into a workspace.
///
/// With `--global`: scaffolds directly into the global registry
/// (`~/.plexi-<channel>/plugins/<name>/`).
///
/// Scaffolds without opening by default. `--open` opens the plugin in a
/// split-right pane after creating it.
pub fn plugin_init(
    name: &str,
    lang: &str,
    global: bool,
    open: bool,
    from_pane_id: Option<u64>,
) -> i32 {
    if name.is_empty() {
        eprintln!("Usage: plexi plugin init [--lang python|rust] [--global] <name> [--open]");
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
        log::warn!("plugin_init: rejected at root dir");
        eprintln!("error: cannot scaffold a plugin in the root directory.");
        return 1;
    }

    let channel_dir = plugin_init_config_dir();
    let plugin_dir = if global {
        crate::app::registry::plugins_dir().join(name)
    } else {
        match plugin_init_workspace_dir(&cwd, name) {
            Some(dir) => dir,
            None => {
                eprintln!(
                    "error: no workspace found (no {channel_dir}/ directory in any ancestor)."
                );
                eprintln!(
                    "  Use --global to create a global plugin, or cd into a project with a workspace."
                );
                return 1;
            }
        }
    };

    let placement = if global { "global" } else { "workspace" };
    log::info!(
        "plugin_init: placement={placement} path={}",
        plugin_dir.display()
    );

    if plugin_dir.exists() {
        log::info!(
            "plugin_init: name collision at {} — refusing to overwrite existing directory",
            plugin_dir.display()
        );
        eprintln!("{}", describe_plugin_init_collision(&plugin_dir, name));
        return 1;
    }

    if let Err(e) = std::fs::create_dir_all(&plugin_dir) {
        eprintln!("error: could not create {}: {e}", plugin_dir.display());
        return 1;
    }

    // `lang` is constrained by the CLI `value_parser` (see PluginCmd::Init); an
    // unlisted value never reaches here. The explicit arms keep the mapping
    // total so a newly added language can't silently fall back to Python.
    let result = match lang {
        "wasm" => scaffold_wasm_view(&plugin_dir, name),
        "python" => scaffold_python_plugin(&plugin_dir, name),
        other => {
            log::error!("plugin_init: no scaffold implemented for --lang '{other}'");
            eprintln!("error: no scaffold implemented for --lang '{other}'");
            return 1;
        }
    };

    match result {
        Ok(()) => {
            if lang != "wasm" {
                match crate::app::python_env::ensure_plugin_venv(name, &plugin_dir, &[]) {
                    Ok(python) => {
                        println!("  Python venv: {}", python.display());
                    }
                    Err(e) => {
                        let entry_path = plugin_dir.join("main.py");
                        match crate::app::python_env::resolve_python_runtime(
                            name,
                            &entry_path,
                            false,
                            &[],
                        ) {
                            Ok(runtime) => {
                                log::warn!(
                                    "plugin_init[{name}]: Python venv setup failed: {e}; using {} ({})",
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
                                    "plugin_init[{name}]: Python environment setup failed: {e}; no fallback runtime: {runtime_err}"
                                );
                                eprintln!(
                                    "error: plugin created, but no compatible Python runtime is available."
                                );
                                eprintln!("  venv setup: {e}");
                                eprintln!("  fallback: {runtime_err}");
                                return 1;
                            }
                        }
                    }
                }
            }
            println!("Created plugin '{name}' at {}", plugin_dir.display());
            let (channel, profile_dir) = current_scaffold_channel();
            let explicit_plexi = explicit_plexi_command(&channel);
            let explicit_host_plexi = explicit_host_plexi_command(&channel, &profile_dir);
            if lang == "wasm" {
                println!("\nNext steps:");
                println!("  cd {}", plugin_dir.display());
                println!("  cargo component build --release --target wasm32-wasip2");
                println!("  {explicit_host_plexi} view open {}", plugin_dir.display());
                println!("  SDK docs: {}", plugin_dir.join("AUTHORING.md").display());
            } else {
                if open {
                    let path_str = plugin_dir.to_string_lossy().to_string();
                    log::info!("plugin_init: opening '{name}' split-right path={path_str} from_pane_id={from_pane_id:?}");
                    let exit_code =
                        super::open_cli(&path_str, &[], Some("split_h"), from_pane_id, None);
                    if exit_code != 0 {
                        eprintln!(
                            "warning: plugin created but could not auto-open (exit {exit_code})"
                        );
                        eprintln!(
                            "  Open with: {explicit_host_plexi} view open {}",
                            plugin_dir.display()
                        );
                    }
                } else {
                    log::info!(
                        "plugin_init: created '{name}' without opening path={}",
                        plugin_dir.display()
                    );
                    println!(
                        "  Open with: {explicit_host_plexi} view open {}",
                        plugin_dir.display()
                    );
                }
                for line in
                    python_init_next_steps(&plugin_dir, name, &explicit_plexi, &explicit_host_plexi)
                {
                    println!("{line}");
                }
            }
            0
        }
        Err(e) => {
            eprintln!("error: failed to scaffold plugin: {e}");
            1
        }
    }
}

/// Lines `plugin init` prints after scaffolding a Python plugin.
fn python_init_next_steps(
    plugin_dir: &std::path::Path,
    name: &str,
    explicit_plexi: &str,
    explicit_host_plexi: &str,
) -> Vec<String> {
    let dir = plugin_dir.display();
    vec![
        format!("  Agent loop: read {dir}/AGENTS.md"),
        format!("  Test with: {explicit_plexi} plugin test {dir}"),
        format!("  Check gate: {explicit_plexi} plugin check {dir} --png-dir /tmp/{name}-shots"),
        format!(
            "  Render state: {explicit_plexi} view render {dir} --state {dir}/fixtures/state.json"
        ),
        "  Hot reload: after open, edit source and verify the same pane id updates with pane state"
            .to_string(),
        "  Host probes:".to_string(),
        format!("    {explicit_host_plexi} pane state <pane-id>"),
        format!("    {explicit_host_plexi} pane action <pane-id> <handler-id>"),
        format!("    {explicit_host_plexi} pane key <pane-id> <key>"),
        "  Guide: sdk/python/AUTHORING.md in the Plexi repo (https://github.com/ianjamesburke/PLEXI/blob/alpha/sdk/python/AUTHORING.md)"
            .to_string(),
        "  API reference: https://plexiapp.com/docs/sdk".to_string(),
    ]
}

/// `uv run` arguments for `plexi plugin test`, built for one plugin directory.
///
/// The runner has to provision its own pytest. Neither the maintained plugins
/// under `plugins/` nor a fresh `plexi plugin init` scaffold declares a Python
/// project, so a bare `uv run pytest` resolves against whatever project happens
/// to sit above the plugin — which does not depend on pytest — and dies with
/// `Failed to spawn: pytest`. `--with pytest` makes the command self-contained.
/// `--no-project` keeps uv from adopting (and materializing a `.venv` for) an
/// unrelated ancestor project when the plugin declares none of its own; a plugin that
/// does ship a `pyproject.toml` stays in project mode so its declared
/// dependencies are installed alongside pytest.
///
/// `--python` pins the interpreter to the plugin venv (`python_env::PYTHON_APP_VENV_VERSION`).
/// Without it uv picks whatever `python3` is on PATH, which on a stock Mac is
/// older than the SDK's floor and dies importing `plexi_sdk` (`tomllib`).
fn plugin_test_uv_args(plugin_dir: &std::path::Path, python: &std::path::Path) -> Vec<String> {
    let mut args = vec!["run".to_string()];
    if !plugin_dir.join("pyproject.toml").is_file() {
        args.push("--no-project".to_string());
    }
    args.push("--python".to_string());
    args.push(python.display().to_string());
    args.extend(
        ["--with", "pytest", "pytest", "tests/"]
            .into_iter()
            .map(str::to_string),
    );
    args
}

/// App id and declared Python dependencies for the plugin venv, from
/// `manifest.toml` when it parses, else the directory name and no dependencies.
fn plugin_test_identity(plugin_dir: &std::path::Path) -> (String, Vec<String>) {
    let manifest = std::fs::read_to_string(plugin_dir.join("manifest.toml"))
        .ok()
        .and_then(|text| toml::from_str::<crate::app::registry::PluginManifest>(&text).ok());
    if let Some(manifest) = manifest {
        return (manifest.plugin.id, manifest.plugin.dependencies);
    }
    let id = std::fs::canonicalize(plugin_dir)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "plugin".to_string());
    (id, Vec::new())
}

/// Builds the import path for plugin tests, keeping the plugin's entry module ahead
/// of the injected Plexi SDK.
fn plugin_test_pythonpath(plugin_dir: &std::path::Path) -> String {
    let sdk_paths = crate::config::build_pythonpath(None);
    let mut paths = vec![plugin_dir.to_path_buf()];
    paths.extend(std::env::split_paths(&sdk_paths));
    std::env::join_paths(paths)
        .expect("plugin test PYTHONPATH entries must not contain the platform path separator")
        .to_string_lossy()
        .into_owned()
}

/// `plexi plugin test [<plugin-path>]` — run a plugin's PluginHarness tests via
/// `uv run` + pytest inside the plugin directory, on the plugin's Python venv.
/// Streams pytest output live and returns its exit code so CI and ship
/// scripts can gate on it.
pub fn plugin_test_cli(path: &str, snapshot: bool) -> i32 {
    let raw_dir = std::path::Path::new(path);
    let tests_dir = raw_dir.join("tests");
    if !tests_dir.is_dir() {
        log::warn!("app_test:cli: no tests/ dir at {}", tests_dir.display());
        eprintln!(
            "error: no tests/ directory in {} — expected {}",
            raw_dir.display(),
            raw_dir.join("tests").join("test_plugin.py").display()
        );
        eprintln!("  `plexi plugin init` scaffolds tests/test_plugin.py for new plugins.");
        return 1;
    }

    let plugin_dir_buf = std::fs::canonicalize(raw_dir).unwrap_or_else(|_| raw_dir.to_path_buf());
    let plugin_dir = plugin_dir_buf.as_path();
    let (plugin_id, deps) = plugin_test_identity(plugin_dir);
    let python = match crate::app::python_env::ensure_plugin_venv(&plugin_id, plugin_dir, &deps) {
        Ok(python) => python,
        Err(e) => {
            log::error!("app_test:cli[{plugin_id}]: plugin Python venv setup failed: {e}");
            eprintln!(
                "error: could not prepare the Python {} plugin venv for {}: {e}",
                crate::app::python_env::PYTHON_APP_VENV_VERSION,
                plugin_dir.display()
            );
            return 1;
        }
    };

    let uv_args = plugin_test_uv_args(plugin_dir, &python);
    log::info!(
        "app_test:cli: running `uv {}` in {} (snapshot={snapshot})",
        uv_args.join(" "),
        plugin_dir.display()
    );

    let mut cmd = std::process::Command::new("uv");
    cmd.args(&uv_args).current_dir(plugin_dir);
    cmd.env("PYTHONPATH", plugin_test_pythonpath(plugin_dir));
    if snapshot {
        cmd.env("PLEXI_UPDATE_SNAPSHOTS", "1");
    }

    match cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            log::error!("app_test:cli: failed to spawn uv: {e}");
            eprintln!("error: could not run `uv run` ({e}). Is uv installed?");
            1
        }
    }
}

/// Commented-out `[marketplace]` section appended to every scaffolded manifest so
/// authors know publishing is one uncomment away. The host validator reads a
/// top-level `[marketplace]` section (see `read_marketplace_manifest` in
/// `src/cli/marketplace.rs`); `publisher` is the field required to publish.
const MARKETPLACE_PLACEHOLDER: &str = "\n# Uncomment and fill in to publish with `plexi plugin publish`:\n# [marketplace]\n# visibility = \"public\"   # public | unlisted | private\n# price = \"free\"          # \"free\", or a price like \"4.99\"\n# publisher = \"your-org\"  # required: your publisher / org slug\n";

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

fn write_python_scaffold_support_files(plugin_dir: &std::path::Path, name: &str) -> io::Result<()> {
    let sdk_version = python_sdk_version();
    let (channel, profile_dir) = current_scaffold_channel();
    let cli_version = env!("CARGO_PKG_VERSION");
    let manifest_schema_version = crate::app::registry::MANIFEST_SCHEMA_VERSION;
    let python_runtime_version = crate::app::python_env::PYTHON_APP_VENV_VERSION;
    let template_version = PYTHON_SCAFFOLD_TEMPLATE_VERSION;

    let agents = include_str!("../../sdk/python/plexi_sdk/templates/plugin-contract.md")
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
    std::fs::write(plugin_dir.join("AGENTS.md"), agents)?;

    let gitignore = include_str!("../../sdk/python/plexi_sdk/templates/gitignore");
    std::fs::write(plugin_dir.join(".gitignore"), gitignore)?;

    write_scaffold_metadata_file(plugin_dir, "plexi plugin init")?;

    Ok(())
}

/// The one writer of `plexi.scaffold.toml`: `plugin init`, `--adopt` and the
/// package migration all stamp metadata through it.
pub(crate) fn write_scaffold_metadata_file(
    plugin_dir: &std::path::Path,
    generated_by: &str,
) -> io::Result<()> {
    let sdk_version = python_sdk_version();
    let (channel, profile_dir) = current_scaffold_channel();
    let cli_version = env!("CARGO_PKG_VERSION");
    let manifest_schema_version = crate::app::registry::MANIFEST_SCHEMA_VERSION;
    let python_runtime_version = crate::app::python_env::PYTHON_APP_VENV_VERSION;
    let template_version = PYTHON_SCAFFOLD_TEMPLATE_VERSION;

    let metadata = format!(
        "schema_version = {schema}\n\
         generated_by = \"{generated_by}\"\n\
         plexi_cli_version = \"{cli_version}\"\n\
         sdk_version = \"{sdk_version}\"\n\
         manifest_schema_version = {manifest_schema_version}\n\
         python_runtime_version = \"{python_runtime_version}\"\n\
         template_version = {template_version}\n\
         channel = \"{channel}\"\n\
         profile_dir = \"{profile_dir}\"\n",
        schema = SCAFFOLD_METADATA_SCHEMA_VERSION,
    );
    let metadata_path = plugin_dir.join(SCAFFOLD_METADATA_FILE);
    std::fs::write(&metadata_path, metadata)?;
    log::info!(
        "plugin_init: wrote scaffold metadata={} generated_by={generated_by} template_version={template_version} sdk_version={sdk_version} channel={channel} profile_dir={profile_dir}",
        metadata_path.display()
    );
    Ok(())
}

/// Write or refresh `plexi.scaffold.toml` for an existing plugin directory without
/// creating or modifying any source files (`plexi plugin init --adopt <dir>`).
pub fn plugin_adopt(path: &str) -> i32 {
    let raw_path = std::path::Path::new(path);
    let (manifest, _entry_path) = match crate::cli::plugin_check::load_local_plugin(raw_path) {
        Ok(loaded) => loaded,
        Err(err) => {
            log::error!(
                "plugin_adopt: invalid plugin directory '{}': {err}",
                raw_path.display()
            );
            eprintln!("error: {err}");
            return 1;
        }
    };
    let plugin_dir = std::fs::canonicalize(raw_path).unwrap_or_else(|_| raw_path.to_path_buf());
    match write_scaffold_metadata_file(&plugin_dir, "plexi plugin init --adopt") {
        Ok(()) => {
            let metadata_path = plugin_dir.join(SCAFFOLD_METADATA_FILE);
            log::info!(
                "plugin_adopt: adopted plugin '{}' at {}",
                manifest.plugin.id,
                metadata_path.display()
            );
            println!(
                "Adopted plugin '{}' — wrote {}",
                manifest.plugin.id,
                metadata_path.display()
            );
            0
        }
        Err(e) => {
            log::error!(
                "plugin_adopt: failed to write {} in {}: {e}",
                SCAFFOLD_METADATA_FILE,
                plugin_dir.display()
            );
            eprintln!("error: could not write {SCAFFOLD_METADATA_FILE}: {e}");
            1
        }
    }
}

/// `chmod +x` a scaffolded entry point.
///
/// Unix needs it: `main.py` is launched directly, and without the bit the plugin
/// never starts. Windows decides executability from the file extension and has
/// no mode bits to set, so there is nothing to do.
#[cfg(unix)]
fn mark_executable(path: &std::path::Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(perms.mode() | 0o111);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn mark_executable(_path: &std::path::Path) -> io::Result<()> {
    Ok(())
}

pub(super) fn scaffold_python_plugin(plugin_dir: &std::path::Path, name: &str) -> io::Result<()> {
    // manifest.toml — shape lives in the template file beside the other scaffold
    // artifacts so it can't silently diverge from the documented manifest. The
    // feature-gated marketplace placeholder is appended in Rust.
    let manifest_template = include_str!("../../sdk/python/plexi_sdk/templates/manifest.toml");
    let manifest = format!(
        "{}{}",
        manifest_template
            .replace("__PLUGIN_ID__", name)
            .replace("__DISPLAY_NAME__", &to_title_case(name)),
        marketplace_placeholder(),
    );
    std::fs::write(plugin_dir.join("manifest.toml"), manifest)?;

    // main.py: plexi_sdk is injected via PYTHONPATH by the host at launch; do
    // NOT copy plexi_sdk alongside (the package uses relative imports that
    // break when imported as a flat single file).
    let display_name = to_title_case(name);
    let main_template = include_str!("../../sdk/python/plexi_sdk/templates/plugin_init.py");
    let main_path = plugin_dir.join("main.py");
    std::fs::write(
        &main_path,
        main_template.replace("__DISPLAY_NAME__", &display_name),
    )?;

    mark_executable(&main_path)?;

    // tests/test_plugin.py: PluginHarness tests that drive the scaffold's one
    // handler, so they fail when it breaks.
    let tests_dir = plugin_dir.join("tests");
    std::fs::create_dir_all(&tests_dir)?;
    let test_template = include_str!("../../sdk/python/plexi_sdk/templates/test_plugin_init.py");
    let test_py = test_template.replace("__DISPLAY_NAME__", &to_title_case(name));
    std::fs::write(tests_dir.join("test_plugin.py"), test_py)?;

    let fixtures_dir = plugin_dir.join("fixtures");
    std::fs::create_dir_all(&fixtures_dir)?;
    std::fs::write(fixtures_dir.join("state.json"), "{\n  \"count\": 3\n}\n")?;

    log::info!(
        "plugin_init: wrote python skeleton main.py tests/test_plugin.py fixtures/state.json at {}",
        plugin_dir.display()
    );

    write_python_scaffold_support_files(plugin_dir, name)?;

    Ok(())
}

fn scaffold_wasm_view(plugin_dir: &std::path::Path, name: &str) -> io::Result<()> {
    let crate_name = name.replace('-', "_");
    std::fs::write(
        plugin_dir.join("manifest.toml"),
        format!(
            "schema_version = 2\n\n[launch]\n\n[plugin]\nid = \"{name}\"\nname = \"{display}\"\nversion = \"0.1.0\"\ndescription = \"A Plexi WASM plugin\"\n\n[plugin.capabilities]\ncapabilities = []\n\n[plugin.capabilities.wasm]\nrequired = []\noptional = []\n\n[runtime]\nkind = \"wasm\"\nentry = \"target/wasm32-wasip1/release/{crate_name}.wasm\"\n\n[[contributes.views]]\nid = \"{name}.main\"\ndefault = true\n",
            display = to_title_case(name),
        ),
    )?;
    std::fs::write(
        plugin_dir.join("Cargo.toml"),
        "[workspace]\nmembers = [\"plugin\"]\nresolver = \"2\"\n",
    )?;
    let crate_dir = plugin_dir.join("plugin");
    std::fs::create_dir_all(crate_dir.join("src"))?;
    std::fs::write(
        crate_dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\ncrate-type = [\"cdylib\"]\n\n[dependencies]\nplexi-wasm-sdk = {{ path = \"../.plexi-sdk/plexi-wasm-sdk\" }}\n\n[package.metadata.component]\npackage = \"plexi:{crate_name}\"\n\n[package.metadata.component.target]\npath = \"../wit\"\nworld = \"plexi-plugin\"\n",
        ),
    )?;
    std::fs::write(
        crate_dir.join("src/lib.rs"),
        format!(
            "use plexi_wasm_sdk::{{effects, export_plugin, ui, View, Effect, InputEvent, KeyEvent, UiActionEvent, UiTree}};\n\nconst INCREMENT: &str = \"increment\";\n\n#[derive(Default)]\nstruct Counter {{ count: u32 }}\n\nimpl View for Counter {{\n    fn init(&mut self, _context: plexi_wasm_sdk::InitContext) -> Vec<Effect> {{\n        vec![effects::set_title(\"{display}\")]\n    }}\n\n    fn update(&mut self, event: InputEvent) -> Vec<Effect> {{\n        let increment = matches!(event, InputEvent::UiAction(UiActionEvent {{ ref handler_id }}) if handler_id == INCREMENT)\n            || matches!(event, InputEvent::Key(KeyEvent {{ ref key, pressed: true, .. }}) if key == \"enter\" || key == \"space\");\n        if increment {{\n            self.count += 1;\n        }}\n        Vec::new()\n    }}\n\n    fn view(&self) -> UiTree {{\n        let mut tree = ui::Tree::new();\n        let count = tree.text(\"count\", format!(\"Count: {{}}\", self.count));\n        let increment = tree.button(\"increment\", \"Increment\", INCREMENT);\n        let root = tree.column(\"root\", [count, increment]);\n        tree.finish(root)\n    }}\n}}\n\nexport_plugin!(Counter::default());\n",
            display = to_title_case(name),
        ),
    )?;
    std::fs::write(
        plugin_dir.join("README.md"),
        "# Build and open\n\n```sh\ncargo install cargo-component\ncargo component build --release --target wasm32-wasip2\nplexi view open .\n```\n\nThe manifest opens the component at `target/wasm32-wasip1/release/`. Click **Increment**, send the `increment` pane action, or press Enter/Space while the pane is focused. Read the local [Plexi WASM authoring guide](AUTHORING.md) for the SDK reference.\n",
    )?;
    std::fs::write(
        plugin_dir.join("AUTHORING.md"),
        include_str!("../../sdk/wasm/AUTHORING.md"),
    )?;
    std::fs::write(
        plugin_dir.join(".gitignore"),
        "/target/\n**/src/bindings.rs\n",
    )?;

    let embedded = plugin_dir.join(".plexi-sdk");
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
        std::os::unix::fs::symlink(".plexi-sdk/wit", plugin_dir.join("wit"))?;
        std::os::unix::fs::symlink("../wit", embedded_sdk.join("wit"))?;
    }
    #[cfg(windows)]
    {
        std::fs::create_dir_all(plugin_dir.join("wit"))?;
        std::fs::write(
            plugin_dir.join("wit/plexi.wit"),
            include_str!("../../wit/plexi.wit"),
        )?;
        std::fs::create_dir_all(embedded_sdk.join("wit"))?;
        std::fs::write(
            embedded_sdk.join("wit/plexi.wit"),
            include_str!("../../wit/plexi.wit"),
        )?;
    }
    log::info!(
        "plugin_init: created self-contained wasm scaffold path={} entry=target/wasm32-wasip1/release/{crate_name}.wasm",
        plugin_dir.display(),
    );
    Ok(())
}

/// `plexi plugin uninstall <id> [--yes]` — remove an installed plugin (global or workspace-local).
pub fn plugin_uninstall(id: &str, assume_yes: bool) -> i32 {
    let cwd = std::env::current_dir().unwrap_or_default();
    let registry = crate::app::registry::PluginRegistry::load(&cwd);
    let Some(installed) = registry.get(id) else {
        eprintln!(
            "error: plugin '{id}' not found — run `plexi plugin list` to see installed plugins"
        );
        return 1;
    };
    let plugin_dir = installed.plugin_dir.clone();
    let target_root = match plugin_dir.parent() {
        Some(p) => p.to_path_buf(),
        None => {
            eprintln!("error: could not determine parent directory for '{id}'");
            return 1;
        }
    };
    log::info!(
        "plugin_uninstall: resolving '{id}' via registry → {:?}",
        plugin_dir
    );
    let core_ids = crate::cli::install_host::core_pack_ids();
    if core_ids.contains(id) {
        eprintln!(
            "note: '{id}' is a core plugin — it will be re-installed on the next Plexi launch"
        );
    }
    if !assume_yes {
        eprint!("Remove plugin '{id}'? [y/N]: ");
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

/// Write `installed_version.txt` inside `plugin_dir`. Best-effort — logs `warn!` on failure.
pub(super) fn write_installed_version(plugin_dir: &std::path::Path, version: &str) {
    let path = plugin_dir.join("installed_version.txt");
    if let Err(e) = std::fs::write(&path, version) {
        log::warn!(
            "version_pin: could not write installed_version.txt for {}: {e}",
            plugin_dir.display()
        );
    } else {
        log::info!(
            "version_pin: wrote installed_version.txt={version} for {}",
            plugin_dir.display()
        );
    }
}

/// Write `pinned_version.txt` inside `plugin_dir`. Best-effort — logs `warn!` on failure.
pub(super) fn write_pinned_version(plugin_dir: &std::path::Path, version: &str) {
    let path = plugin_dir.join("pinned_version.txt");
    if let Err(e) = std::fs::write(&path, version) {
        log::warn!(
            "version_pin: could not write pinned_version.txt for {}: {e}",
            plugin_dir.display()
        );
    } else {
        log::info!(
            "version_pin: wrote pinned_version.txt={version} for {}",
            plugin_dir.display()
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
/// Print the trust sheet for a validated plugin dir or package: identity,
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
            crate::platform::format::human_size(report.total_size)
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

/// Host-version gate: block install when the host is too old for the plugin (or a
/// declared requirement is malformed); warn when the host is newer than the
/// plugin's declared ceiling. Returns an exit code to abort, or `None` to proceed.
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

/// Run the full trust gate for an interactive install: print the trust sheet,
/// then prompt. `--yes` confirms the install only; it never answers a
/// capability (stint 1291). Returns an exit code when the install stops.
fn run_install_gate(
    report: &crate::app::package::PackageReport,
    assume_yes: bool,
    marketplace_reviewed: bool,
) -> Result<(), i32> {
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
        Ok(true) => Ok(()),
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

fn is_valid_plugin_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// `plexi plugin inspect <path>` — validate a local plugin dir or `.plexipkg` file
/// and print the trust sheet without installing anything (stint 0016).
pub fn plugin_inspect_cli(path: &str) -> i32 {
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

/// `plexi plugin install <path> [--version X.Y.Z] [--yes]`
///
/// When `pin` is `None` (the common case) this is a plain install.
/// When `pin` is `Some(ver)` the version is stored in `pinned_version.txt`.
///
/// `confirm` decides whether the trust sheet + prompt gate runs:
/// `Interactive` for user-typed installs, `PreApproved` for internal callers
/// that already gated (or are first-party bundled content).
pub fn plugin_install_with_pin(
    path: &str,
    pin: Option<&str>,
    confirm: InstallConfirm,
    assume_yes: bool,
) -> i32 {
    plugin_install_with_pin_inner(path, pin, confirm, assume_yes)
}

fn plugin_install_with_pin_inner(
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
    let mut validated_report = None;
    match crate::app::package::validate_dir(&src) {
        Ok(report) => {
            if confirm == InstallConfirm::Interactive {
                if let Err(code) = run_install_gate(&report, assume_yes, false) {
                    return code;
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
                    "plugin::install: pre-approved install of {} proceeding despite validation failure: {e}",
                    src.display()
                );
            }
        },
    }

    let manifest_path = src.join("manifest.toml");
    if !manifest_path.exists() {
        eprintln!("error: no manifest.toml found in {}", src.display());
        eprintln!(
            "  Is this a Plexi plugin directory? Run `plexi plugin init <name>` to scaffold one."
        );
        return 1;
    }
    let manifest_str = match std::fs::read_to_string(&manifest_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: could not read manifest.toml: {e}");
            return 1;
        }
    };
    // Even preapproved installs must obey the schema contract.
    let manifest = match crate::app::registry::PluginManifest::parse(&manifest_str) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("error: {error}");
            return 1;
        }
    };
    let plugin_id = manifest.plugin.id.clone();
    if !is_valid_plugin_id(&plugin_id) {
        eprintln!("error: manifest.toml is missing a valid [plugin].id");
        return 1;
    }
    let plugin_version = &manifest.plugin.version;
    let _command_install = match manifest.lock_command_install(&crate::app::registry::plugins_dir())
    {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("error: {error}");
            return 1;
        }
    };

    let dest = crate::app::registry::plugins_dir().join(&plugin_id);

    // Stage the complete tree outside the plugins directory, mark it complete, then
    // swap it in. An interruption leaves the previous install or the new one,
    // never a missing or half-copied plugin (stint 1175). Grants bind package
    // content, so a changed tree asks for approval again by itself.
    let staging_root = crate::app::install_swap::staging_root();
    crate::app::install_swap::clear_stale_staging(&staging_root, &plugin_id);
    let staging = crate::app::install_swap::new_staging_dir(&staging_root, &plugin_id);
    if let Err(e) = copy_dir_all(&src, &staging) {
        eprintln!(
            "error: could not stage {} at {}: {e}",
            src.display(),
            staging.display()
        );
        let _ = std::fs::remove_dir_all(&staging);
        return 1;
    }
    // Write version tracking files (best-effort — never fail the install).
    write_installed_version(&staging, plugin_version);
    if let Some(pin) = pin {
        if pin != plugin_version {
            eprintln!(
                "warning: manifest version is {plugin_version} but pinning to {pin} as requested"
            );
        }
        write_pinned_version(&staging, pin);
    }
    if let Err(e) = crate::app::install_swap::mark_complete(&staging) {
        eprintln!("error: could not mark the staged install complete: {e}");
        let _ = std::fs::remove_dir_all(&staging);
        return 1;
    }
    if let Err(e) = crate::app::install_swap::replace_dir(&staging, &dest) {
        eprintln!(
            "error: could not install {plugin_id} at {}: {e}",
            dest.display()
        );
        let _ = std::fs::remove_dir_all(&staging);
        return 1;
    }
    if let (Some(report), InstallConfirm::Interactive) = (validated_report.as_ref(), confirm) {
        announce_capabilities_ask_at_launch(report);
    }

    log::info!(
        "plugin::install: installed {plugin_id} v{plugin_version} from {}",
        src.display()
    );
    println!(
        "Installed '{plugin_id}' v{plugin_version} from {}.",
        src.display()
    );
    println!("Run `plexi view open {plugin_id}` to launch it.");
    0
}

/// Install writes no permission (stint 1291): the CLI process is not the
/// host, and `--yes` confirms the install only. Says, per runtime, what the
/// first launch gives automatically and what waits for a person.
fn announce_capabilities_ask_at_launch(report: &crate::app::package::PackageReport) {
    if let Some(line) = install_permission_line(report) {
        log::info!(
            "plugin::install: no grant written for {}: {line}",
            report.id
        );
        println!("{line}");
    }
}

fn install_permission_line(report: &crate::app::package::PackageReport) -> Option<String> {
    use crate::app::package::PackageRuntime;
    // The same rule the host applies at first launch.
    let (needs_person, automatic): (Vec<String>, Vec<String>) = report
        .capabilities
        .iter()
        .map(|cap| cap.as_str().to_string())
        .chain(report.wasm_required_capabilities.iter().cloned())
        .chain(report.wasm_optional_capabilities.iter().cloned())
        .partition(|id| crate::broker::gate::declared_capability_needs_click(id));
    if automatic.is_empty() && needs_person.is_empty() {
        return None;
    }
    let mut line = format!(
        "Installing grants no permissions. When '{}' first runs",
        report.id
    );
    if !automatic.is_empty() {
        line.push_str(&format!(" it gets {} automatically", automatic.join(", ")));
    }
    if !needs_person.is_empty() {
        if !automatic.is_empty() {
            line.push(';');
        }
        match report.runtime {
            PackageRuntime::Wasm => line.push_str(&format!(
                " {} wait for your approval in the Plexi window",
                needs_person.join(", ")
            )),
            PackageRuntime::Python => line.push_str(&format!(
                " {} stay withheld: Python plugins cannot ask for them yet",
                needs_person.join(", ")
            )),
        }
    }
    line.push('.');
    Some(line)
}

/// Shown in the rendered Permissions view and on stderr when `view render`
/// runs outside the host (stint 1291).
const PERMISSION_ROWS_FROM_HOST: &str = "Permission rows come from the running Plexi host and are not available to this command; rendering the view with none.";

/// `plexi plugin package <path> [--out <file>]` — build a `.plexipkg` artifact.
pub fn plugin_package_cli(path: &str, out: Option<&str>) -> i32 {
    log::info!("app_package:cli: path={path} out={out:?}");
    let plugin_dir = match std::path::Path::new(path).canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: could not resolve {path}: {e}");
            return 1;
        }
    };
    match crate::app::package::build_package(&plugin_dir, out.map(std::path::Path::new)) {
        Ok(pkg) => {
            println!("Built package {}", pkg.display());
            println!("  Validate with: plexi plugin validate {}", pkg.display());
            0
        }
        Err(e) => {
            eprintln!("error: package build failed: {e}");
            1
        }
    }
}

/// `plexi plugin install <file.plexipkg>` — validate the package (fail-closed),
/// then extract to a temp dir and run the standard dir install on it.
///
/// The trust gate runs here (against the validated package report); the inner
/// dir install is then `PreApproved` so the user is prompted exactly once.
pub fn plugin_install_package(
    file: &str,
    pin: Option<&str>,
    confirm: InstallConfirm,
    assume_yes: bool,
) -> i32 {
    plugin_install_package_inner(file, pin, confirm, assume_yes, false, None)
}

pub fn plugin_install_marketplace_package(
    file: &str,
    pin: Option<&str>,
    confirm: InstallConfirm,
    assume_yes: bool,
    source_metadata: Option<crate::app::marketplace::InstalledRegistrySource>,
) -> i32 {
    plugin_install_package_inner(file, pin, confirm, assume_yes, true, source_metadata)
}

fn plugin_install_package_inner(
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

    if confirm == InstallConfirm::Interactive {
        if let Err(code) = run_install_gate(&report, assume_yes, marketplace_reviewed) {
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
    let code = plugin_install_with_pin_inner(
        &staging.to_string_lossy(),
        pin,
        InstallConfirm::PreApproved,
        assume_yes,
    );
    if code == 0 && confirm == InstallConfirm::Interactive {
        announce_capabilities_ask_at_launch(&report);
    }
    if code == 0 {
        if let Some(metadata) = source_metadata {
            let plugin_dir = crate::app::registry::plugins_dir().join(&report.id);
            if let Err(e) = metadata.write_to(&plugin_dir) {
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
                "plugin::install: skipping generated dev artifact {}",
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

/// Build the `list_plugin_tools` IPC request payload, attaching caller pane/context
/// env hints when present.
pub(crate) fn build_list_plugin_tools_payload(
    plugin_id: Option<&str>,
    context_id: Option<u64>,
) -> serde_json::Value {
    let caller_pane_id = std::env::var("PLEXI_PANE_ID")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok());
    let caller_context_id = std::env::var("PLEXI_CONTEXT_ID")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok());
    let mut payload = serde_json::json!({
        "type": "list_plugin_tools",
    });
    if let Some(id) = plugin_id.map(str::trim).filter(|s| !s.is_empty()) {
        payload["plugin_id"] = serde_json::json!(id);
    }
    if let Some(ctx) = context_id {
        payload["context_id"] = serde_json::json!(ctx);
    }
    if let Some(pane) = caller_pane_id {
        payload["caller_pane_id"] = serde_json::json!(pane);
    }
    if let Some(ctx) = caller_context_id {
        payload["caller_context_id"] = serde_json::json!(ctx);
    }
    payload
}

/// Best-effort live registry query for `plexi plugin info <id>`. Returns `None`
/// silently when no host socket is reachable so offline `plexi plugin info` never
/// fails or prints a socket error.
fn try_query_live_plugin_tools_for_info(
    plugin_id: &str,
) -> Option<(u64, Vec<crate::plexi_ai::tool_dispatch::LiveToolEntry>)> {
    let socket = super::resolve_command_socket()?;
    if !super::spawn_socket_accepting(&socket) {
        return None;
    }
    let payload = build_list_plugin_tools_payload(Some(plugin_id), None);
    let content = super::request_with(
        payload,
        "app-tools-response",
        "plugin tools",
        std::time::Duration::from_secs(2),
    )
    .ok()?;
    let reply: serde_json::Value = serde_json::from_str(&content).ok()?;
    if reply.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }
    let context_id = reply.get("context_id").and_then(|v| v.as_u64())?;
    let tools: Vec<crate::plexi_ai::tool_dispatch::LiveToolEntry> =
        serde_json::from_value(reply.get("tools")?.clone()).ok()?;
    Some((context_id, tools))
}

/// Format `plexi plugin info <id>` output. Populates `tools` from `live_tools` when
/// at least one pane of the plugin is running in the caller's context; otherwise
/// explicitly labels the section as `manifest-only` (or `static source scan`)
/// so an unopened plugin is never misreported as declaring no tools.
pub(crate) fn format_plugin_info(
    m: &crate::app::registry::PluginIdentity,
    plugin_dir: &std::path::Path,
    live_tools: Option<(u64, &[crate::plexi_ai::tool_dispatch::LiveToolEntry])>,
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "id:          {}", m.id);
    let _ = writeln!(out, "name:        {}", m.name);
    let _ = writeln!(out, "version:     {}", m.version);
    let _ = writeln!(out, "description: {}", m.description);
    if let Some(ref author) = m.author {
        let _ = writeln!(out, "author:      {author}");
    }
    if !m.tags.is_empty() {
        let _ = writeln!(out, "tags:        {}", m.tags.join(", "));
    }
    if let Some(ref repo) = m.repo {
        let _ = writeln!(out, "repo:        {repo}");
    }
    if let Some((context_id, live)) = live_tools.filter(|(_, entries)| !entries.is_empty()) {
        let _ = writeln!(out, "tools (live registry, context {context_id}):");
        for entry in live {
            let ro = if entry.read_only { " [read_only]" } else { "" };
            let _ = writeln!(
                out,
                "  {}  (pane {}){}  {}",
                entry.name, entry.pane_id, ro, entry.description
            );
            let _ = writeln!(out, "    input_schema:  {}", entry.input_schema);
            let _ = writeln!(out, "    output_schema: {}", entry.output_schema);
        }
        return out;
    }

    let static_tools = crate::cli::introspect::declared_tools(plugin_dir);
    if !static_tools.is_empty() {
        let _ = writeln!(
            out,
            "tools (static source scan; manifest-only while not running — open '{}' and run `plexi plugin tools` for live schemas):",
            m.id
        );
        for (name, description) in &static_tools {
            let _ = writeln!(out, "  {name}  {description}");
        }
    } else {
        let _ = writeln!(
            out,
            "tools (manifest-only — manifests do not declare tools; no running pane of '{}' in current context):",
            m.id
        );
        let _ = writeln!(
            out,
            "  (none in manifest; open '{}' and run `plexi plugin tools` to inspect live runtime tools)",
            m.id
        );
    }
    out
}

/// `plexi plugin info <id>` — show manifest info and honest tool status for an installed plugin.
pub fn plugin_info(id: &str) -> i32 {
    if super::host::plugin_mode_core_only() {
        eprintln!("{}", crate::app::core_only::unavailable(id));
        return 1;
    }
    let registry = crate::app::registry::PluginRegistry::load_with_global(
        &std::env::current_dir().unwrap_or_default(),
        &crate::app::registry::plugins_dir(),
    );
    let Some(installed) = registry.get(id) else {
        eprintln!(
            "error: plugin '{id}' not found — run `plexi plugin list` to see installed plugins"
        );
        return 1;
    };
    let plugin_dir = installed
        .bin_path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| installed.bin_path.clone());
    let live = try_query_live_plugin_tools_for_info(id);
    let live_count = live.as_ref().map(|(_, t)| t.len()).unwrap_or(0);
    let formatted = format_plugin_info(
        &installed.manifest,
        &plugin_dir,
        live.as_ref().map(|(ctx, t)| (*ctx, t.as_slice())),
    );
    print!("{formatted}");
    log::info!("plugin_info: id={id} live_tools={live_count}");
    0
}

/// `plexi plugin list` — unified alias for `plexi list`.
pub fn plugin_list() -> i32 {
    log::info!("cli: plugin_list delegating to list_cli (unified)");
    super::list::list_cli()
}

/// Discover declared views without starting a runtime.
pub fn view_list(json: bool) -> i32 {
    if super::host::plugin_mode_core_only() {
        println!(
            "{}",
            if json {
                "[]"
            } else {
                "No views available: core-only mode is on."
            }
        );
        return 0;
    }
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            eprintln!("error: view list could not read cwd: {error}");
            return 1;
        }
    };
    let registry = crate::app::registry::PluginRegistry::load(&cwd);
    let mut rows = Vec::new();
    for plugin in registry.list() {
        for view in &plugin.contributes.views {
            if json {
                rows.push(serde_json::json!({"id": view.id, "plugin_id": plugin.manifest.id, "name": plugin.manifest.name, "default": view.default || plugin.contributes.views.len() == 1}));
            } else {
                println!(
                    "{}\t{}{}",
                    view.id,
                    plugin.manifest.name,
                    if view.default || plugin.contributes.views.len() == 1 {
                        " (default)"
                    } else {
                        ""
                    }
                );
            }
        }
    }
    if json {
        println!("{}", serde_json::Value::Array(rows));
    }
    log::info!("view_list: enumerated declared views");
    0
}

/// `plexi plugin prune --dry-run` reports precisely the legacy installs that a
/// normal launch will reconcile. The no-flag form performs the same safe,
/// positively-identified reconciliation for users repairing a stopped profile.
pub fn plugin_prune_cli(dry_run: bool) -> i32 {
    let plugins_dir = crate::app::registry::plugins_dir();
    let candidates = crate::cli::install_host::orphaned_pre_v3_first_party_apps(&plugins_dir);
    log::info!(
        "app_prune: {} candidate(s) at {} dry_run={dry_run}",
        candidates.len(),
        plugins_dir.display()
    );
    if dry_run {
        for candidate in candidates {
            println!("would quarantine {}", candidate.display());
        }
        return 0;
    }
    let removed =
        crate::cli::install_host::reconcile_orphaned_pre_v3_first_party_apps(&plugins_dir);
    let all_removed = removed.len() == candidates.len();
    for path in removed {
        println!("quarantined {}", path.display());
    }
    if !all_removed {
        eprintln!("error: one or more orphaned pre-v3 plugins could not be quarantined");
        return 1;
    }
    0
}

/// `plexi view render <plugin> --size WxH [--state state.json] [--output path] [--png]`
/// Renders a plugin headlessly. `plugin` may be an installed plugin ID or a local directory path.
/// Default output: JSON frame tree. With --png: PNG image.
pub fn view_render(
    plugin: &str,
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

    // Resolve (plugin_id, bin_path): path argument takes priority over registry lookup.
    // A path is detected by prefix (./  ../  /) or by existing as a directory.
    // Path: more than one component (./foo, ../foo, /abs/path) OR an existing directory.
    // Using components() instead of prefix checks is portable across platforms.
    let (
        plugin_id,
        plugin_dir,
        plugin_bin,
        capabilities,
        allowed_hosts,
        state_scopes,
        state_format,
    ) = if std::path::Path::new(plugin).components().count() > 1
        || std::path::Path::new(plugin).is_dir()
    {
        let plugin_dir = match std::path::Path::new(plugin).canonicalize() {
            Ok(p) => p,
            Err(e) => {
                log::warn!("view_render: could not resolve '{plugin}': {e}");
                eprintln!("error: could not resolve '{plugin}': {e}");
                return 1;
            }
        };
        if !plugin_dir.is_dir() {
            eprintln!("error: '{plugin}' is not a directory — expected a plugin directory containing manifest.toml");
            return 1;
        }
        let manifest_path = plugin_dir.join("manifest.toml");
        let manifest_str = match std::fs::read_to_string(&manifest_path) {
            Ok(s) => s,
            Err(e) => {
                log::warn!(
                    "view_render: no manifest.toml in {}: {e}",
                    plugin_dir.display()
                );
                eprintln!("error: no manifest.toml in {}: {e}", plugin_dir.display());
                eprintln!(
                    "  Is this a Plexi plugin directory? Run `plexi plugin init <name>` to scaffold one."
                );
                return 1;
            }
        };
        let manifest = match crate::app::registry::PluginManifest::parse(&manifest_str) {
            Ok(m) => m,
            Err(e) => {
                log::warn!(
                    "view_render: invalid manifest.toml in {}: {e}",
                    plugin_dir.display()
                );
                eprintln!("error: invalid manifest.toml: {e}");
                return 1;
            }
        };
        if manifest.schema_version > crate::app::registry::MANIFEST_SCHEMA_VERSION {
            eprintln!(
                "error: manifest.toml schema_version {} is newer than supported (max {}); update Plexi to render this plugin",
                manifest.schema_version,
                crate::app::registry::MANIFEST_SCHEMA_VERSION
            );
            return 1;
        }
        if let Err(error) = manifest
            .contributes
            .resolve_view(&manifest.plugin.id, &manifest.plugin.id)
        {
            eprintln!("error: {error}");
            return 1;
        }
        let entry = plugin_dir.join(&manifest.plugin.entry);
        if !entry.exists() {
            eprintln!(
                "error: plugin entry '{}' not found in {}",
                manifest.plugin.entry,
                plugin_dir.display()
            );
            return 1;
        }
        let state = manifest
            .state_scopes()
            .map_err(|e| format!("[state] scopes: {e}"))
            .and_then(|scopes| Ok((scopes, manifest.state_format()?)));
        let (state_scopes, state_format) = match state {
            Ok(state) => state,
            Err(e) => {
                eprintln!("error: {}: {e}", manifest_path.display());
                return 1;
            }
        };
        log::info!(
            "view_render[{}]: loaded from path '{plugin}'",
            manifest.plugin.id
        );
        (
            manifest.plugin.id,
            plugin_dir,
            entry,
            manifest.plugin.capabilities.capabilities,
            manifest.plugin.capabilities.allowed_hosts,
            state_scopes,
            state_format,
        )
    } else {
        // ID-based: registry lookup
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let registry = crate::app::registry::PluginRegistry::load(&cwd);
        match registry.list().into_iter().find(|a| {
            a.manifest.id == plugin || a.contributes.views.iter().any(|view| view.id == plugin)
        }) {
            Some(a) => {
                if let Err(error) = a.contributes.resolve_view(&a.manifest.id, plugin) {
                    eprintln!("error: {error}");
                    return 1;
                }
                let plugin_dir = a
                    .bin_path
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_else(|| a.bin_path.clone());
                (
                    a.manifest.id.clone(),
                    plugin_dir,
                    a.bin_path.clone(),
                    a.manifest.capabilities.capabilities.clone(),
                    a.manifest.capabilities.allowed_hosts.clone(),
                    a.state_scopes.clone(),
                    a.state_format,
                )
            }
            None => {
                eprintln!(
                    "error: plugin '{plugin}' not found — run `plexi plugin list` to see installed plugins"
                );
                return 1;
            }
        }
    };

    if plugin_bin.extension().and_then(|ext| ext.to_str()) != Some("py") {
        eprintln!(
            "error: '{}' is not a Python entry — plexi view render only drives SDK v3 Python plugins (CPython-in-WASM)",
            plugin_bin.display()
        );
        return 1;
    }
    let launch_config = crate::cli::plugin_check::python_launch_config_from_parts(
        &plugin_id,
        &plugin_dir,
        &plugin_bin,
        &capabilities,
        &allowed_hosts,
        state_scopes,
        state_format,
    );
    let seed_state = if seed_state.is_none() && plugin_id == "permissions" {
        // Permission rows live in the running host (stint 1291). This command
        // renders the view without them and says so.
        let (entries, notice) =
            match crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
            {
                Ok(monitor) => (monitor.list_entries(), String::new()),
                Err(error) => {
                    log::info!("view_render: permissions inventory unavailable: {error}");
                    eprintln!("{PERMISSION_ROWS_FROM_HOST}");
                    (Vec::new(), PERMISSION_ROWS_FROM_HOST.to_string())
                }
            };
        log::info!(
            "view_render: seeding permissions inventory count={}",
            entries.len()
        );
        Some(serde_json::json!({
            "entries": entries,
            "selected": 0,
            "mode": "list",
            "notice": notice,
        }))
    } else {
        seed_state
    };
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
            1.0,
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
                log::info!("view_render[{plugin_id}]: wrote {width}×{height} PNG to '{path}'");
                eprintln!("Wrote {width}×{height} PNG to '{path}'");
            }
            None => {
                use std::io::Write;
                if let Err(e) = std::io::stdout().write_all(&png_bytes) {
                    eprintln!("error: could not write PNG to stdout: {e}");
                    return 1;
                }
                log::info!(
                    "view_render[{plugin_id}]: wrote {width}×{height} PNG to stdout ({} bytes)",
                    png_bytes.len()
                );
            }
        }
    } else {
        // JSON mode (default): the semantic UI-node tree plus what each node
        // painted at this size.
        let layout =
            crate::host::wasm_render::render_ui_tree_layout(&tree, width as f32, height as f32);
        let json = match serde_json::to_string_pretty(
            &crate::cli::plugin_check::frame_json_with_layout(&tree, &layout),
        ) {
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
                log::info!("view_render[{plugin_id}]: wrote JSON frame to '{path}'");
                eprintln!("Wrote JSON frame to '{path}'");
            }
            None => {
                println!("{json}");
                log::info!("view_render[{plugin_id}]: wrote JSON frame to stdout");
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
pub(crate) enum PluginInstallSpecKind {
    Package,
    Source,
    LocalPath,
    MarketplaceId,
}

/// Classify the user-provided `plexi plugin install <spec>` target before any
/// path canonicalization. Source specs such as `github:owner/repo` contain `/`
/// but are not filesystem paths.
pub(crate) fn classify_plugin_install_spec(s: &str) -> PluginInstallSpecKind {
    if s.ends_with(".plexipkg") {
        return PluginInstallSpecKind::Package;
    }
    if crate::app::packs::parse_source_spec(s).is_ok() {
        return PluginInstallSpecKind::Source;
    }
    if s.contains('/') || s.starts_with('.') || std::path::Path::new(s).is_dir() {
        return PluginInstallSpecKind::LocalPath;
    }
    PluginInstallSpecKind::MarketplaceId
}

/// Returns true if `s` looks like a bare plugin ID (no scheme prefix, no path separators).
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

/// channel plugins dir. Source spec follows `packs::parse_source_spec`.
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

/// `plexi plugin update [<id>]` — canonical plugin update path.
///
/// Pulls git-backed installed plugins in the current workspace-aware registry.
/// `plexi update plugins` delegates to the same implementation for compatibility.
pub fn plugin_update_cli(id: Option<&str>) -> i32 {
    log::info!("app_update:cli: delegating to workspace-aware git updater id={id:?}");
    crate::cli::install::update_cli(id)
}

/// `plexi pane action <pane_id> <action> [args...]`
///
/// Sends a `send_pane_action` message to the host over PLEXI_SOCKET.
/// The host delivers a `PlexiEvent::Action { action, args }` to the target view pane.
/// Returns 0 on success, 1 on error.
pub fn pane_action_cli(pane_id: u64, action: &str, args: &[String]) -> i32 {
    log::info!(
        "pane_action:cli: pane_id={pane_id} action={action:?} argc={}",
        args.len()
    );

    let mut payload = serde_json::json!({
        "type": "send_pane_action",
        "pane_id": pane_id,
        "action": action,
    });
    if !args.is_empty() {
        payload["args"] = serde_json::json!(args);
    }

    let content = match super::request(payload, "app-action-response", "pane action") {
        Ok(content) => content,
        Err(code) => return code,
    };
    if let Err(code) = super::check_reply_error(&content) {
        return code;
    }
    0
}

/// Host deadline for a CLI tool call. Strictly below the CLI's poll window so
/// the caller always sees the host's typed timeout, never a client timeout.
pub(crate) const APP_CALL_HOST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
pub(crate) const APP_CALL_APPROVAL_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(120);

/// `plexi plugin call <plugin_id> <tool> [--input JSON]` — invoke a plugin tool through
/// the host's tool dispatcher and print its JSON output on stdout.
pub fn plugin_call_cli(
    plugin_id: &str,
    tool: &str,
    input: &str,
    json: bool,
    target_pane: Option<u64>,
    wait_approved: bool,
) -> i32 {
    let input_value: serde_json::Value = match serde_json::from_str(input) {
        Ok(value @ serde_json::Value::Object(_)) => value,
        Ok(_) => {
            eprintln!("error: --input must be a JSON object");
            return 1;
        }
        Err(error) => {
            eprintln!("error: --input is not valid JSON: {error}");
            return 1;
        }
    };
    // Provenance, not authority: the host validates the pane id and resolves
    // its live context; it never trusts an identity carried in `input`.
    let caller_pane_id = std::env::var("PLEXI_PANE_ID")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok());
    log::info!(
        "plugin_call:cli: plugin={plugin_id:?} tool={tool:?} caller_pane={caller_pane_id:?}"
    );
    let mut payload = serde_json::json!({
        "type": "call_plugin_tool",
        "plugin_id": plugin_id,
        "tool": tool,
        "input_json": input_value.to_string(),
        "wait_approved": wait_approved,
    });
    if let Some(pane_id) = caller_pane_id {
        payload["caller_pane_id"] = serde_json::json!(pane_id);
    }
    if let Some(pane) = target_pane {
        payload["target_pane_id"] = serde_json::json!(pane);
        log::info!("plugin_call:cli: target_pane={pane}");
    }
    if let Ok(credential) = std::env::var("PLEXI_CALL_CREDENTIAL") {
        if !credential.is_empty() {
            payload["call_credential"] = serde_json::json!(credential);
        }
    }
    let content = match super::request_with(
        payload,
        "app-call-response",
        "plugin call",
        APP_CALL_HOST_TIMEOUT
            + std::time::Duration::from_secs(5)
            + if wait_approved {
                APP_CALL_APPROVAL_TIMEOUT
            } else {
                std::time::Duration::ZERO
            },
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    if !json {
        if let Err(code) = super::check_reply_error(&content) {
            return code;
        }
    }
    match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(reply) => {
            if json {
                println!("{reply}");
                return if reply.get("ok").and_then(|v| v.as_bool()) == Some(false) {
                    1
                } else {
                    0
                };
            }
            let output = reply
                .get("output")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            println!("{output}");
            0
        }
        Err(error) => {
            eprintln!("error: host reply is not JSON: {error}");
            1
        }
    }
}

/// `plexi plugin tools [--plugin <plugin_id>] [--context <context_id>] [--json]` —
/// snapshot the live tool registry exposed by running view panes in the resolved
/// context.
pub fn plugin_tools_cli(plugin_id: Option<&str>, context_id: Option<u64>, json: bool) -> i32 {
    log::info!("plugin_tools:cli: plugin={plugin_id:?} context={context_id:?} json={json}");
    let payload = build_list_plugin_tools_payload(plugin_id, context_id);
    let content = match super::request(payload, "app-tools-response", "plugin tools") {
        Ok(content) => content,
        Err(code) => return code,
    };
    if let Err(code) = super::check_reply_error(&content) {
        return code;
    }
    if json {
        return super::print_json_output(&content);
    }
    let reply: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(error) => {
            eprintln!("error: host reply is not JSON: {error}");
            return 1;
        }
    };
    let resolved_ctx = reply
        .get("context_id")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let tools: Vec<crate::plexi_ai::tool_dispatch::LiveToolEntry> = reply
        .get("tools")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    if tools.is_empty() {
        if let Some(plugin) = plugin_id {
            println!("No live tools registered for plugin '{plugin}' in context {resolved_ctx}.");
        } else {
            println!("No live tools registered in context {resolved_ctx}.");
        }
        return 0;
    }
    println!("context {resolved_ctx} — {} live tool(s):", tools.len());
    for entry in &tools {
        let ro = if entry.read_only { " [read_only]" } else { "" };
        println!(
            "  {}  (plugin: {}, pane: {}){}  {}",
            entry.name, entry.plugin_id, entry.pane_id, ro, entry.description
        );
        println!("    input_schema:  {}", entry.input_schema);
        println!("    output_schema: {}", entry.output_schema);
    }
    0
}

/// `plexi needs-you list --json` and `plexi needs-you resolve <id> --approve|--deny`.
pub fn needs_you_cli(op: &str, id: Option<&str>, approve: Option<bool>) -> i32 {
    let response_file = crate::rpc::response_file("needs-you", "json");
    let payload = match op {
        "list" => serde_json::json!({"type":"list_needs_you","response_file":response_file}),
        "resolve" => serde_json::json!({
            "type": "resolve_needs_you",
            "id": id.unwrap_or(""),
            "approve": approve.unwrap_or(false),
            "response_file": response_file,
        }),
        _ => {
            eprintln!("error: unknown needs-you operation");
            return 1;
        }
    };
    log::info!("needs_you:cli: op={op} id={id:?} approve={approve:?}");
    let content = match super::request_with(
        payload,
        "needs-you",
        "needs-you",
        std::time::Duration::from_secs(15),
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    println!("{content}");
    match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(value) if value.get("ok").and_then(|v| v.as_bool()) == Some(true) => 0,
        Ok(_) => 1,
        Err(_) => 1,
    }
}

/// `plexi needs-you ask|show|answer|reconcile|complete` — rich question and continuation API.
pub fn needs_you_api_cli(op: &str, payload: serde_json::Value) -> i32 {
    let response_file = crate::rpc::response_file("needs-you-api", "json");
    let request = serde_json::json!({
        "type": "needs_you_api",
        "op": op,
        "payload": payload,
        "response_file": response_file,
    });
    log::info!("needs_you:cli: api op={op}");
    let content = match super::request_with(
        request,
        "needs-you",
        "needs-you",
        std::time::Duration::from_secs(15),
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    println!("{content}");
    match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(value) if value.get("ok").and_then(|v| v.as_bool()) == Some(true) => 0,
        Ok(_) => 1,
        Err(_) => 1,
    }
}

/// `plexi assistant permission list|show|resolve` — observation seam for pending grants.
pub fn assistant_permission_cli(op: &str, id: Option<&str>, choice: Option<&str>) -> i32 {
    let response_file = crate::rpc::response_file("assistant-permission", "json");
    let payload = match op {
        "list" => {
            serde_json::json!({"type":"list_permission_requests","response_file":response_file})
        }
        "show" => {
            serde_json::json!({"type":"show_permission_request","pending_request_id":id.unwrap_or(""),"response_file":response_file})
        }
        "resolve" => {
            serde_json::json!({"type":"resolve_permission_request","pending_request_id":id.unwrap_or(""),"choice":choice.unwrap_or(""),"response_file":response_file})
        }
        _ => {
            eprintln!("error: unknown permission operation");
            return 1;
        }
    };
    log::info!("assistant_permission:cli: op={op} id={id:?}");
    let content = match super::request_with(
        payload,
        "assistant-permission",
        "assistant permission",
        std::time::Duration::from_secs(15),
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    println!("{content}");
    match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(value) if value.get("ok").and_then(|v| v.as_bool()) == Some(true) => 0,
        Ok(_) => 1,
        Err(_) => 1,
    }
}

/// Run one Assistant host tool on the connected host and print the JSON reply.
/// Exit 0 when the tool ran, 2 when the gate is waiting on a decision, 1 otherwise.
pub fn assistant_tool_cli(name: &str, input_json: &str) -> i32 {
    let response_file = crate::rpc::response_file("assistant-tool", "json");
    let payload = serde_json::json!({
        "type": "assistant_host_tool",
        "name": name,
        "input_json": input_json,
        "response_file": response_file,
    });
    log::info!("assistant_tool:cli: name={name}");
    let content = match super::request_with(
        payload,
        "assistant-tool",
        "assistant tool",
        std::time::Duration::from_secs(30),
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    println!("{content}");
    match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(value) if value.get("ok").and_then(|item| item.as_bool()) == Some(true) => 0,
        Ok(value)
            if value.get("error_code").and_then(|item| item.as_str())
                == Some("permission_required") =>
        {
            2
        }
        Ok(_) => 1,
        Err(_) => 1,
    }
}

/// Fields of `submit_assistant_turn`. There is no approval, grant, or
/// permission field: the host permission gate is the only authorizer.
pub(crate) struct AssistantSendFields<'a> {
    pub text: &'a str,
    pub request_id: &'a str,
    pub response_file: &'a str,
    pub pane_id: Option<u64>,
    pub context_id: Option<u64>,
    pub conversation: Option<&'a str>,
    pub join_desktop: bool,
    pub status_for: Option<&'a str>,
    pub client: Option<&'a str>,
    pub kind: Option<&'a str>,
}

pub(crate) fn assistant_send_payload(fields: &AssistantSendFields<'_>) -> serde_json::Value {
    serde_json::json!({
        "type": "submit_assistant_turn",
        "text": fields.text,
        "request_id": fields.request_id,
        "response_file": fields.response_file,
        "pane_id": fields.pane_id,
        "context_id": fields.context_id,
        "conversation_id": fields.conversation,
        "join_desktop": fields.join_desktop,
        "status_for": fields.status_for,
        "client": fields.client,
        "kind": fields.kind,
    })
}

/// Conversation routing and ledger tags for [`assistant_send_result`].
pub struct AssistantSendRoute<'a> {
    pub conversation: Option<&'a str>,
    pub join_desktop: bool,
    pub status_for: Option<&'a str>,
    pub client: Option<&'a str>,
    pub kind: Option<&'a str>,
}

/// Submit one turn, or poll `--status-for`, and return the host JSON envelope.
///
/// `conversation` selects a caller-owned conversation. `join_desktop`
/// opts into the desktop transcript. `status_for` reads a turn that already
/// returned `waiting_for_permission` and does not submit a prompt. A caller
/// turn never carries a grant; the desktop permission gate still admits tools.
pub fn assistant_send_result(
    text: Option<&str>,
    request_id: Option<&str>,
    pane_id: Option<u64>,
    context_id: Option<u64>,
    route: AssistantSendRoute<'_>,
) -> Result<serde_json::Value, String> {
    let AssistantSendRoute {
        conversation,
        join_desktop,
        status_for,
        client,
        kind,
    } = route;
    let request_id = request_id
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let response_file = crate::rpc::response_file("assistant-send", "json");
    if let Some(kind) = kind {
        crate::plexi_ai::ledger::RunKind::parse(kind)?;
    }
    if let Some(turn_id) = status_for {
        log::info!("assistant_send:cli: status poll turn_id={turn_id} request_id={request_id}");
    } else {
        log::info!(
            "assistant_send:cli: submit request_id={request_id} join_desktop={join_desktop} conversation={}",
            conversation.unwrap_or("")
        );
    }
    let payload = assistant_send_payload(&AssistantSendFields {
        text: text.unwrap_or(""),
        request_id: &request_id,
        response_file: &response_file,
        pane_id,
        context_id,
        conversation,
        join_desktop,
        status_for,
        client: client.map(str::trim).filter(|client| !client.is_empty()),
        kind: kind.map(str::trim).filter(|kind| !kind.is_empty()),
    });
    let content = super::request_with(
        payload,
        "assistant-send",
        "assistant send",
        std::time::Duration::from_secs(120),
    )
    .map_err(|code| format!("assistant_send_exit_{code}"))?;
    serde_json::from_str(&content).map_err(|_| "assistant_send_bad_json".to_string())
}

/// Submit one turn to the host Assistant and print its terminal JSON envelope.
pub fn assistant_send_cli(
    text: Option<&str>,
    request_id: Option<&str>,
    pane_id: Option<u64>,
    context_id: Option<u64>,
    route: AssistantSendRoute<'_>,
) -> i32 {
    match assistant_send_result(text, request_id, pane_id, context_id, route) {
        Ok(value) => {
            println!("{value}");
            if value.get("state").and_then(|v| v.as_str()) == Some("succeeded") {
                0
            } else {
                2
            }
        }
        Err(error) => {
            if let Some(code) = error.strip_prefix("assistant_send_exit_") {
                if let Ok(code) = code.parse::<i32>() {
                    return code;
                }
            }
            eprintln!("error: {error}");
            1
        }
    }
}

#[cfg(test)]
mod plugin_install_workspace_tests {
    use super::{classify_plugin_install_spec, PluginInstallSpecKind};
    use tempfile::TempDir;

    fn write_valid_manifest(dir: &std::path::Path, id: &str) {
        std::fs::write(
            dir.join("manifest.toml"),
            format!(
                "schema_version = 2\n\n[launch]\n\n[plugin]\nid = \"{id}\"\nname = \"Test\"\nversion = \"0.1.0\"\ndescription = \"Test\"\n\n[plugin.capabilities]\ncapabilities = []\n\n[runtime]\nkind = \"python\"\nentry = \"main.py\"\n\n[[contributes.views]]\nid = \"{id}.main\"\ndefault = true\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("main.py"), "# stub\n").unwrap();
    }

    /// `plexi plugin install <path>` must succeed from a bare directory with no
    /// `.plexi/` workspace. Install goes to the global channel plugins dir — workspace
    /// resolution is never consulted for path-based installs.
    #[test]
    fn install_from_path_succeeds_without_workspace() {
        let src = TempDir::new().unwrap();
        let plugin_id = "plexi-test-install-no-workspace";
        write_valid_manifest(src.path(), plugin_id);
        let path = src.path().to_string_lossy().to_string();

        // No workspace present in src or any ancestor (temp dir) — must return 0.
        // Interactive + --yes: gate prints the trust sheet but never prompts.
        let code =
            super::plugin_install_with_pin(&path, None, super::InstallConfirm::Interactive, true);

        // Clean up installed plugin to avoid polluting the plugins dir between runs.
        let dest = crate::app::registry::plugins_dir().join(plugin_id);
        let _ = std::fs::remove_dir_all(&dest);

        assert_eq!(
            code, 0,
            "install from path must succeed without a workspace"
        );
    }

    #[test]
    fn install_accepts_reverse_dns_plugin_id() {
        let src = TempDir::new().unwrap();
        let plugin_id = "com.plexi.test-install";
        write_valid_manifest(src.path(), plugin_id);
        let path = src.path().to_string_lossy().to_string();

        let code =
            super::plugin_install_with_pin(&path, None, super::InstallConfirm::Interactive, true);

        let dest = crate::app::registry::plugins_dir().join(plugin_id);
        let _ = std::fs::remove_dir_all(&dest);

        assert_eq!(code, 0, "reverse-DNS plugin ids must install cleanly");
    }

    /// Stint 1291: `--yes` confirms the install only. A WASM package whose
    /// `[plugin.capabilities.wasm]` lists required and optional ids leaves no
    /// capability row in either permission file; the plugin asks when it runs.
    #[test]
    fn install_yes_writes_no_grant() {
        let profile = TempDir::new().unwrap();
        let _profile_guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        let src = TempDir::new().unwrap();
        let plugin_id = "plexi-test-install-yes-no-grant";
        std::fs::write(
            src.path().join("manifest.toml"),
            format!(
                "schema_version = 2\n\n[launch]\n\n[plugin]\nid = \"{plugin_id}\"\nname = \"Test\"\nversion = \"0.1.0\"\ndescription = \"Test\"\n\n[plugin.capabilities]\ncapabilities = []\n\n[plugin.capabilities.wasm]\nrequired = [\"spawn.plugin\", \"state:read-write\"]\noptional = [\"ai.query\"]\n\n[runtime]\nkind = \"wasm\"\nentry = \"app.wasm\"\n\n[[contributes.views]]\nid = \"{plugin_id}.main\"\ndefault = true\n"
            ),
        )
        .unwrap();
        std::fs::write(src.path().join("app.wasm"), b"\0asm").unwrap();

        let code = super::plugin_install_with_pin(
            &src.path().to_string_lossy(),
            None,
            super::InstallConfirm::Interactive,
            true,
        );
        assert_eq!(code, 0, "install --yes must still install the plugin");

        let grants =
            std::fs::read_to_string(profile.path().join("grants.toml")).unwrap_or_default();
        assert!(
            !grants.contains(plugin_id),
            "install --yes must not write a grant row:\n{grants}"
        );
        let legacy =
            std::fs::read_to_string(profile.path().join("permissions.toml")).unwrap_or_default();
        assert!(
            !legacy.contains(plugin_id),
            "install --yes must not write a legacy permission row:\n{legacy}"
        );
    }

    /// Stint 1291: the install line says what each runtime really gets.
    #[test]
    fn install_permission_line_is_true_per_runtime() {
        use crate::app::package::{PackageReport, PackageRuntime};
        use crate::app::permissions::Capability;
        let mut report = PackageReport {
            id: "p".to_string(),
            name: "P".to_string(),
            version: "0.1.0".to_string(),
            runtime: PackageRuntime::Python,
            entry: "main.py".to_string(),
            capabilities: vec![Capability::FsRead, Capability::SpawnView],
            wasm_required_capabilities: Vec::new(),
            wasm_optional_capabilities: Vec::new(),
            file_count: 1,
            total_size: 1,
            requires_plexi_min: None,
            requires_plexi_max: None,
        };
        assert_eq!(
            super::install_permission_line(&report).unwrap(),
            "Installing grants no permissions. When 'p' first runs it gets fs.read automatically; spawn.plugin stay withheld: Python plugins cannot ask for them yet."
        );
        report.runtime = PackageRuntime::Wasm;
        report.capabilities = Vec::new();
        report.wasm_required_capabilities = vec!["fs:write:/w".to_string()];
        assert_eq!(
            super::install_permission_line(&report).unwrap(),
            "Installing grants no permissions. When 'p' first runs fs:write:/w wait for your approval in the Plexi window."
        );
        report.wasm_required_capabilities = Vec::new();
        assert!(super::install_permission_line(&report).is_none());
    }

    #[test]
    fn install_fails_on_missing_manifest_not_workspace_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let code =
            super::plugin_install_with_pin(&path, None, super::InstallConfirm::Interactive, true);
        // Must fail with exit code 1 (manifest missing), not a workspace error.
        assert_eq!(code, 1, "missing manifest must return 1");
    }

    #[test]
    fn github_source_spec_is_not_classified_as_local_path() {
        assert_eq!(
            classify_plugin_install_spec("github:owner/repo"),
            PluginInstallSpecKind::Source
        );
        assert_eq!(
            classify_plugin_install_spec("git+file:///tmp/repo"),
            PluginInstallSpecKind::Source
        );
        assert_eq!(
            classify_plugin_install_spec("owner/repo"),
            PluginInstallSpecKind::LocalPath
        );
    }
}

#[cfg(test)]
mod install_confirm_tests {
    use super::{confirm_install, trust_sheet_lines, InstallConfirm};
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
            line == "runtime:      python — Reviewed Python plugin — human-reviewed"
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
            .any(|line| line == "runtime:      python — Bundled"));
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
}

#[cfg(test)]
mod plugin_inspect_tests {
    use tempfile::TempDir;

    #[test]
    fn inspect_valid_dir_returns_0() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("manifest.toml"),
            "schema_version = 2\n\n[launch]\n\n[plugin]\nid = \"inspect-test\"\nname = \"Inspect Test\"\nversion = \"0.1.0\"\ndescription = \"Test\"\n\n[plugin.capabilities]\ncapabilities = [\"ai.query\"]\n\n[runtime]\nkind = \"python\"\nentry = \"main.py\"\n\n[[contributes.views]]\nid = \"inspect-test.main\"\ndefault = true\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("main.py"), "# stub\n").unwrap();
        let code = super::plugin_inspect_cli(&dir.path().to_string_lossy());
        assert_eq!(code, 0, "valid plugin dir must inspect cleanly");
    }

    #[test]
    fn inspect_wasm_manifest_returns_0() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("manifest.toml"),
            "schema_version = 2\n\n[launch]\n\n[plugin]\nid = \"com.plexi.inspect-wasm\"\nname = \"Inspect WASM\"\nversion = \"0.1.0\"\ndescription = \"Test\"\n\n[plugin.capabilities]\ncapabilities = [\"gpu.render\"]\n\n[runtime]\nkind = \"wasm\"\nentry = \"app.wasm\"\n\n[[contributes.views]]\nid = \"com.plexi.inspect-wasm.main\"\ndefault = true\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("app.wasm"), b"\0asm").unwrap();
        let code = super::plugin_inspect_cli(&dir.path().to_string_lossy());
        assert_eq!(code, 0, "valid wasm plugin dir must inspect cleanly");
    }

    #[test]
    fn inspect_invalid_dir_returns_1() {
        let dir = TempDir::new().unwrap();
        // No manifest — must fail validation, exit non-zero.
        let code = super::plugin_inspect_cli(&dir.path().to_string_lossy());
        assert_eq!(code, 1, "dir without manifest must fail inspect");
    }
}

#[cfg(test)]
mod version_pin_tests {
    use tempfile::TempDir;

    fn write_manifest(dir: &std::path::Path, id: &str, version: &str) {
        let manifest = format!(
            "schema_version = 2\n\n[launch]\n\n[plugin]\nid = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\ndescription = \"Test\"\n\n[plugin.capabilities]\ncapabilities = []\n\n[runtime]\nkind = \"python\"\nentry = \"main.py\"\n\n[[contributes.views]]\nid = \"{id}.main\"\ndefault = true\n"
        );
        std::fs::write(dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(dir.join("main.py"), "# stub\n").unwrap();
    }

    #[test]
    fn installed_version_written_to_file() {
        let src = TempDir::new().unwrap();
        write_manifest(src.path(), "my-app", "1.2.3");

        // Use a temp dir as the plugins_dir root — app_install copies to plugins_dir/my-plugin.
        // We can test write_installed_version directly since app_install calls plugins_dir().
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
            .get("plugin")
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
        let plugin_src = TempDir::new().unwrap();
        write_manifest(plugin_src.path(), "reviewed-notes", "0.1.0");
        let pkg = plugin_src.path().join("reviewed-notes-0.1.0.plexipkg");
        crate::app::package::build_package(plugin_src.path(), Some(&pkg)).unwrap();

        let metadata = crate::app::marketplace::InstalledRegistrySource {
            schema_version: crate::app::marketplace::MARKETPLACE_SCHEMA_VERSION,
            source: "hosted-registry".to_string(),
            registry_url: "https://plexiapp.com/registry/v1/index.json".to_string(),
            plugin_id: "reviewed-notes".to_string(),
            version: "0.1.0".to_string(),
            publisher: "plexi".to_string(),
            checksum: "abc123".to_string(),
            package_url: "https://plexiapp.com/registry/v1/packages/abc123.plexipkg".to_string(),
            reviewed_native: true,
        };

        let code = super::plugin_install_marketplace_package(
            &pkg.to_string_lossy(),
            None,
            super::InstallConfirm::PreApproved,
            true,
            Some(metadata.clone()),
        );

        assert_eq!(code, 0);
        let source_path = profile
            .path()
            .join("plugins")
            .join("reviewed-notes")
            .join(crate::app::marketplace::INSTALLED_REGISTRY_SOURCE_FILE);
        let text = std::fs::read_to_string(source_path).unwrap();
        let parsed: crate::app::marketplace::InstalledRegistrySource =
            toml::from_str(&text).unwrap();
        assert_eq!(parsed, metadata);
    }
}

#[cfg(test)]
mod plugin_test_command_tests {
    use super::*;
    use tempfile::TempDir;

    const VENV_PYTHON: &str = "/apps/todo/.venv/bin/python";

    fn venv_python() -> std::path::PathBuf {
        std::path::PathBuf::from(VENV_PYTHON)
    }

    /// The interpreter must be pinned to the plugin venv. Left to uv, a stock
    /// Mac resolves to system Python 3.9 and `plexi_sdk` dies on `tomllib`.
    #[test]
    fn plugin_test_pins_the_interpreter_to_the_plugin_venv() {
        let dir = TempDir::new().unwrap();
        let args = plugin_test_uv_args(dir.path(), &venv_python());
        let at = args.iter().position(|a| a == "--python").unwrap();
        assert_eq!(args[at + 1], VENV_PYTHON);
    }

    #[test]
    fn plugin_test_pythonpath_puts_the_plugin_root_before_the_sdk() {
        let dir = TempDir::new().unwrap();
        let pythonpath = plugin_test_pythonpath(dir.path());
        let mut entries = std::env::split_paths(&pythonpath);

        assert_eq!(entries.next().as_deref(), Some(dir.path()));
        assert_eq!(
            entries.collect::<Vec<_>>(),
            std::env::split_paths(&crate::config::build_pythonpath(None)).collect::<Vec<_>>(),
            "the SDK path entries follow the plugin root"
        );
    }

    /// Identity falls back to the directory name with no dependencies when
    /// the plugin has no readable manifest, and reads both from one when it does.
    #[test]
    fn plugin_test_identity_reads_manifest_else_dir_name() {
        let dir = TempDir::new().unwrap();
        let (id, deps) = plugin_test_identity(dir.path());
        assert_eq!(
            id,
            dir.path().file_name().unwrap().to_string_lossy(),
            "no manifest → directory name"
        );
        assert!(deps.is_empty());

        std::fs::write(
            dir.path().join("manifest.toml"),
            "schema_version = 2\n\n[plugin]\nid = \"todo\"\nname = \"Todo\"\ndependencies = [\"httpx\"]\n\n[runtime]\nkind = \"python\"\nentry = \"main.py\"\n\n[[contributes.views]]\nid = \"todo.main\"\ndefault = true\n",
        )
        .unwrap();
        let (id, deps) = plugin_test_identity(dir.path());
        assert_eq!(id, "todo");
        assert_eq!(deps, vec!["httpx".to_string()]);
    }

    /// The generated command must ask uv for pytest itself. Plugins do not depend
    /// on pytest, so dropping `--with pytest` puts `plexi plugin test` back to
    /// `Failed to spawn: pytest` for every maintained and scaffolded plugin.
    #[test]
    fn plugin_test_provisions_pytest_for_an_plugin_without_a_project() {
        let dir = TempDir::new().unwrap();
        let args = plugin_test_uv_args(dir.path(), &venv_python());
        assert_eq!(
            args,
            vec![
                "run",
                "--no-project",
                "--python",
                VENV_PYTHON,
                "--with",
                "pytest",
                "pytest",
                "tests/"
            ],
            "a plugin declaring no pyproject.toml runs pytest in an isolated uv environment"
        );
    }

    /// An plugin that declares its own project keeps project mode, so uv installs
    /// the author's dependencies next to pytest.
    #[test]
    fn plugin_test_keeps_project_mode_when_the_plugin_declares_one() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = \"x\"\n",
        )
        .unwrap();
        let args = plugin_test_uv_args(dir.path(), &venv_python());
        assert_eq!(
            args,
            vec![
                "run",
                "--python",
                VENV_PYTHON,
                "--with",
                "pytest",
                "pytest",
                "tests/"
            ],
            "a plugin with its own project must not be run with --no-project"
        );
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
        let parsed: crate::app::registry::PluginManifest =
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
        let parsed: crate::app::registry::PluginManifest =
            toml::from_str(manifest).expect("scaffolded manifest must parse");
        assert!(parsed.marketplace.is_none());
    }

    fn manifest_for(scaffold: fn(&std::path::Path, &str) -> io::Result<()>) -> String {
        let dir = TempDir::new().unwrap();
        let plugin_dir = dir.path().join("myapp");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        scaffold(&plugin_dir, "myapp").unwrap();
        std::fs::read_to_string(plugin_dir.join("manifest.toml")).unwrap()
    }

    #[test]
    fn stable_python_scaffold_omits_marketplace_placeholder() {
        assert_no_placeholder(&manifest_for(scaffold_python_plugin));
    }

    #[test]
    fn python_scaffold_writes_appharness_test() {
        let dir = TempDir::new().unwrap();
        let plugin_dir = dir.path().join("myapp");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        scaffold_python_plugin(&plugin_dir, "myapp").unwrap();

        let test_path = plugin_dir.join("tests").join("test_plugin.py");
        assert!(
            test_path.is_file(),
            "python scaffold must write tests/test_plugin.py"
        );
        let test_src = std::fs::read_to_string(&test_path).unwrap();
        assert!(
            test_src.contains("PluginHarness"),
            "generated test must exemplify PluginHarness"
        );
        assert!(
            test_src.contains("h.ui_action(\"increment\")") && test_src.contains("h.key(\"+\")"),
            "generated test must drive the handler it asserts on, so breaking the handler fails it"
        );
        assert!(
            !test_src.contains("__DISPLAY_NAME__"),
            "generated test must substitute the display-name placeholder"
        );
    }

    #[test]
    fn python_scaffold_writes_agent_contract_gitignore_and_metadata() {
        let dir = TempDir::new().unwrap();
        let plugin_dir = dir.path().join("myapp");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        scaffold_python_plugin(&plugin_dir, "myapp").unwrap();

        let agents = std::fs::read_to_string(plugin_dir.join("AGENTS.md")).unwrap();
        assert!(
            agents.contains("Use TDD"),
            "AGENTS.md must tell agents to use TDD"
        );
        assert!(
            agents.contains("plexi plugin test ."),
            "AGENTS.md must teach regular plugin tests"
        );
        assert!(
            agents.contains("PLEXI_CHANNEL=alpha plexi plugin check . --png-dir"),
            "AGENTS.md must teach explicit-channel check gate"
        );
        assert!(
            agents.contains("plexi-pr-123 plugin check . --png-dir"),
            "AGENTS.md must teach direct PR-channel check syntax"
        );
        assert!(
            agents.contains("plexi pane action <pane-id>"),
            "AGENTS.md must teach exercising pane actions"
        );
        assert!(
            agents.contains("Hot reload is part of the dev loop"),
            "AGENTS.md must teach same-pane hot reload validation"
        );
        assert!(
            agents.contains("log.debug"),
            "AGENTS.md must teach SDK log levels"
        );

        let gitignore = std::fs::read_to_string(plugin_dir.join(".gitignore")).unwrap();
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
            std::fs::read_to_string(plugin_dir.join("fixtures/state.json")).unwrap(),
            "{\n  \"count\": 3\n}\n"
        );

        let metadata_raw =
            std::fs::read_to_string(plugin_dir.join(SCAFFOLD_METADATA_FILE)).unwrap();
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

    fn scaffold_files(root: &std::path::Path) -> Vec<String> {
        fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(root, &path, out);
                } else {
                    out.push(
                        path.strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }
        let mut out = Vec::new();
        walk(root, root, &mut out);
        out.sort();
        out
    }

    /// Stint 1330: the scaffold is a skeleton to extend, not a demo to delete.
    #[test]
    fn python_scaffold_is_a_minimal_skeleton() {
        let dir = TempDir::new().unwrap();
        let plugin_dir = dir.path().join("myapp");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        scaffold_python_plugin(&plugin_dir, "myapp").unwrap();

        let files = scaffold_files(&plugin_dir);
        assert_eq!(
            files,
            [
                ".gitignore",
                "AGENTS.md",
                "fixtures/state.json",
                "main.py",
                "manifest.toml",
                "plexi.scaffold.toml",
                "tests/test_plugin.py",
            ],
            "the scaffold writes exactly these files"
        );

        for file in &files {
            let src = std::fs::read_to_string(plugin_dir.join(file)).unwrap();
            for demo in ["demo.", "LEFT_COUNTER", "timer"] {
                assert!(
                    !src.contains(demo),
                    "{file} must not carry demo content ({demo:?})"
                );
            }
            assert!(
                !src.contains("__DISPLAY_NAME__")
                    && !src.contains("__PLUGIN_ID__")
                    && !src.contains("__APP_NAME__"),
                "{file} must substitute its placeholders"
            );
        }

        let manifest: crate::app::registry::PluginManifest =
            toml::from_str(&std::fs::read_to_string(plugin_dir.join("manifest.toml")).unwrap())
                .expect("scaffolded manifest must parse");
        assert!(
            manifest.plugin.capabilities.capabilities.is_empty(),
            "a skeleton that returns no gated effect declares no capability"
        );

        let main_src = std::fs::read_to_string(plugin_dir.join("main.py")).unwrap();
        for needle in [
            "def init(size, args) -> list:",
            "def update(event) -> list:",
            "def view() -> Component:",
            "isinstance(event, Resize)",
            "on_click=\"increment\"",
            "event.key == \"+\"",
            "log.info(",
        ] {
            assert!(
                main_src.contains(needle),
                "generated main.py should contain {needle}"
            );
        }
        assert!(
            !main_src.contains("tools."),
            "the skeleton exposes no assistant tools"
        );

        let agents = std::fs::read_to_string(plugin_dir.join("AGENTS.md")).unwrap();
        for file in &files {
            assert!(
                agents.contains(&format!("`{file}`")),
                "AGENTS.md must say what {file} is for"
            );
        }
    }

    /// Stint 1330: the init output names the guide and only real commands.
    #[test]
    fn python_init_next_steps_point_at_the_authoring_guide() {
        let lines = python_init_next_steps(
            std::path::Path::new("/ws/.plexi-alpha/plugins/myapp"),
            "myapp",
            "PLEXI_CHANNEL=alpha plexi",
            "PLEXI_SOCKET=$HOME/.plexi-alpha/notify.sock PLEXI_CHANNEL=alpha plexi",
        )
        .join("\n");

        assert!(
            lines.contains("sdk/python/AUTHORING.md"),
            "init must point at the canonical guide: {lines}"
        );
        assert!(
            lines.contains("https://plexiapp.com/docs/sdk"),
            "init must point at the docs site API reference: {lines}"
        );
        assert!(
            !lines.contains("SDK_V3.md"),
            "the design spec is partly historical and is not the guide: {lines}"
        );
        assert!(
            !lines.contains("help(plexi_sdk"),
            "plexi_sdk does not import in a bare interpreter: {lines}"
        );
        assert!(
            lines.contains(
                "view render /ws/.plexi-alpha/plugins/myapp --state /ws/.plexi-alpha/plugins/myapp/fixtures/state.json"
            ),
            "--state is read relative to the cwd, so the printed path must be complete: {lines}"
        );
    }

    #[test]
    fn wasm_scaffold_has_component_workspace_manifest_and_wit_link() {
        let temp = tempfile::tempdir().unwrap();
        let plugin_dir = temp.path().join("my-counter");
        std::fs::create_dir(&plugin_dir).unwrap();

        scaffold_wasm_view(&plugin_dir, "my-counter").unwrap();

        let manifest: toml::Value =
            toml::from_str(&std::fs::read_to_string(plugin_dir.join("manifest.toml")).unwrap())
                .unwrap();
        assert_eq!(manifest["runtime"]["kind"].as_str(), Some("wasm"));
        assert_eq!(
            manifest["runtime"]["entry"].as_str(),
            Some("target/wasm32-wasip1/release/my_counter.wasm")
        );
        assert!(plugin_dir.join("plugin/src/lib.rs").is_file());
        assert_eq!(
            std::fs::read_link(plugin_dir.join("wit")).unwrap(),
            std::path::Path::new(".plexi-sdk/wit")
        );
        assert!(plugin_dir
            .join(".plexi-sdk/plexi-wasm-sdk/src/lib.rs")
            .is_file());
        let readme = std::fs::read_to_string(plugin_dir.join("README.md")).unwrap();
        assert!(readme.contains("[Plexi WASM authoring guide](AUTHORING.md)"));
        assert!(plugin_dir.join("AUTHORING.md").is_file());
        let source = std::fs::read_to_string(plugin_dir.join("plugin/src/lib.rs")).unwrap();
        assert!(source.contains("InputEvent::UiAction"));
        assert!(source.contains("InputEvent::Key"));
    }

    #[test]
    fn scaffolds_keep_marketplace_placeholder_when_the_experiment_is_on() {
        let _on =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::Marketplace]);
        assert_placeholder(&manifest_for(scaffold_python_plugin));
    }
}

#[cfg(test)]
mod plugin_init_workspace_walk_tests {
    use super::{plugin_init_config_dir, plugin_init_workspace_dir};

    /// Stint 1337: `$HOME` is not the login home (a sandbox or test profile
    /// redirected it), the cwd is under the login home, and the login home
    /// holds the installed channel profile dir. `plugin init` must not take
    /// that profile dir for a workspace.
    #[test]
    fn plugin_init_does_not_scaffold_into_the_login_home_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let login_home = tmp.path().join("login-home");
        let cwd = login_home.join("projects").join("demo");
        let profile = login_home.join(plugin_init_config_dir());
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&profile).unwrap();
        let _login = crate::app::registry::set_test_login_home(&login_home);

        assert_eq!(
            plugin_init_workspace_dir(&cwd, "myapp"),
            None,
            "the login home's profile dir is not a workspace"
        );
    }

    #[test]
    fn plugin_init_scaffolds_into_a_workspace_under_the_login_home() {
        let tmp = tempfile::tempdir().unwrap();
        let login_home = tmp.path().join("login-home");
        let workspace = login_home.join("projects").join("demo");
        let cwd = workspace.join("src").join("deep");
        let channel_dir = plugin_init_config_dir();
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(workspace.join(&channel_dir)).unwrap();
        std::fs::create_dir_all(login_home.join(&channel_dir)).unwrap();
        let _login = crate::app::registry::set_test_login_home(&login_home);

        assert_eq!(
            plugin_init_workspace_dir(&cwd, "myapp"),
            Some(workspace.join(&channel_dir).join("plugins").join("myapp"))
        );
    }
}

#[cfg(test)]
mod plugin_init_collision_tests {
    use super::describe_plugin_init_collision;
    use tempfile::TempDir;

    /// stint 0428: a name collision on `plugin init` must name the occupying
    /// plugin's id/type and suggest a concrete alternative name, so an agent
    /// (or the assistant's build-plexi-plugin skill) never pivots to silently
    /// editing whatever it finds there instead.
    #[test]
    fn collision_with_valid_manifest_names_id_and_type() {
        let dir = TempDir::new().unwrap();
        let plugin_dir = dir.path().join("counter");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("manifest.toml"),
            "schema_version = 2\n\n[launch]\n\n[plugin]\nid = \"counter\"\nname = \"Counter\"\nversion = \"0.1.0\"\ndescription = \"Test\"\n\n[plugin.capabilities]\ncapabilities = []\n\n[runtime]\nkind = \"wasm\"\nentry = \"app.wasm\"\n\n[[contributes.views]]\nid = \"counter.main\"\ndefault = true\n",
        )
        .unwrap();

        let msg = describe_plugin_init_collision(&plugin_dir, "counter");
        assert!(msg.contains("already exists"), "message: {msg}");
        assert!(msg.contains("id=\"counter\""), "message: {msg}");
        assert!(msg.contains("Wasm"), "message: {msg}");
        assert!(
            msg.contains("plexi plugin init counter-2"),
            "must suggest a concrete alternative name: {msg}"
        );
    }

    /// A colliding directory with no parseable manifest still gets an
    /// actionable message — never a bare "already exists" with no next step.
    #[test]
    fn collision_without_manifest_falls_back_to_generic_message() {
        let dir = TempDir::new().unwrap();
        let plugin_dir = dir.path().join("stale-dir");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let msg = describe_plugin_init_collision(&plugin_dir, "stale-dir");
        assert!(msg.contains("already exists"), "message: {msg}");
        assert!(
            msg.contains("plexi plugin init stale-dir-2"),
            "must suggest a concrete alternative name: {msg}"
        );
    }
}

#[cfg(test)]
mod plugin_info_honesty_tests {
    use super::format_plugin_info;
    use crate::app::registry::{ManifestType, PluginCapabilities, PluginIdentity};
    use crate::plexi_ai::tool_dispatch::LiveToolEntry;

    fn sample_manifest(id: &str) -> PluginIdentity {
        PluginIdentity {
            id: id.to_string(),
            name: "Calculator".to_string(),
            entry: "main.py".to_string(),
            manifest_type: ManifestType::Python,
            version: "0.1.0".to_string(),
            min_sdk_version: None,
            description: "Scientific calculator".to_string(),
            capabilities: PluginCapabilities::default(),
            watch: None,
            author: None,
            tags: Vec::new(),
            repo: None,
            dependencies: Vec::new(),
        }
    }

    #[test]
    fn view_info_populates_live_tools_when_pane_is_running() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = sample_manifest("calc");
        let live = vec![LiveToolEntry {
            name: "calc.add".to_string(),
            provider_name: "calc.add".to_string(),
            plugin_id: "calc".to_string(),
            pane_id: 42,
            context_id: 7,
            description: "Add two numbers".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"a": {"type": "number"}, "b": {"type": "number"}},
                "required": ["a", "b"]
            }),
            output_schema: serde_json::json!({
                "type": "object",
                "properties": {"result": {"type": "number"}},
                "required": ["result"]
            }),
            read_only: false,
            timeout_ms: None,
        }];
        let out = format_plugin_info(&manifest, dir.path(), Some((7, &live)));
        assert!(
            out.contains("tools (live registry, context 7):"),
            "expected live registry header: {out}"
        );
        assert!(
            out.contains("calc.add  (pane 42)  Add two numbers"),
            "{out}"
        );
        assert!(out.contains("input_schema:"), "{out}");
        assert!(out.contains("output_schema:"), "{out}");
    }

    #[test]
    fn plugin_info_labels_manifest_only_when_not_running_and_no_static_literals() {
        let dir = tempfile::tempdir().unwrap();
        // Dynamic tool registration in a loop (like plugins/calc) has no literal string first arg.
        std::fs::write(
            dir.path().join("main.py"),
            "for name in ['add', 'sub']:\n    @tools.tool(f'calc.{name}', 'desc')\n    def _op(): pass\n",
        )
        .unwrap();
        let manifest = sample_manifest("calc");
        let out = format_plugin_info(&manifest, dir.path(), None);
        assert!(
            out.contains("manifest-only"),
            "must explicitly label manifest-only when not running: {out}"
        );
        assert!(
            out.contains("plexi plugin tools"),
            "must signpost `plexi plugin tools` for live runtime tools: {out}"
        );
        assert!(
            !out.contains("(none declared)"),
            "must never print the misleading '(none declared)': {out}"
        );
    }

    #[test]
    fn plugin_info_labels_static_source_scan_when_not_running_with_static_decorators() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.py"),
            "@tools.tool(\"chess.state\", \"Return current board state.\")\ndef _state(): pass\n",
        )
        .unwrap();
        let manifest = sample_manifest("chess");
        let out = format_plugin_info(&manifest, dir.path(), Some((1, &[])));
        assert!(
            out.contains("static source scan; manifest-only while not running"),
            "must label static scan honestly: {out}"
        );
        assert!(
            out.contains("chess.state  Return current board state."),
            "{out}"
        );
    }
}
