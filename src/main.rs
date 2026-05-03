#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Ship-time panic-path protection. `todo!()` / `unimplemented!()` compile clean
// but panic at runtime — e.g. 2026-04-18, CoreAudioDevice::start_capture was
// `todo!()` and froze the GUI when a recorder app sent AudioCapture without
// PLEXI_AUDIO=mock://. Stubs must return `Err(NotImplemented)` instead.
#![deny(clippy::todo, clippy::unimplemented)]

mod agent_pane;
mod agent_workspace;
mod agent_workspace_modal;
mod app;
mod app_permissions;
mod app_protocol;
mod app_registry;
mod app_trait;
mod audio;
mod cli;
mod cli_registry;
mod cli_setup;
mod command_palette;
mod config;
mod context;
mod event_log;
mod features;
mod file_browser;
mod host;
mod keys;
mod logging;
#[cfg(target_os = "macos")]
mod macos_menu;
mod midi;
mod overlays;
mod packs;
mod pane;
mod pane_ops;
mod plexi_descriptor;
mod plexi_ai;
mod render;
mod process_app;
mod headless_renderer;
mod hot_reload;
mod install;
mod protocol;
mod quick_note_app;
mod runs;
mod secrets;
mod secrets_app;
mod workspace_router;
mod workspace_secrets;
mod shell;
mod minimap;
mod sidebar;
mod sidebar_row;
mod spatial;
mod style;
mod theme;
mod tiling;
mod typed_pipes;
mod video;
mod widgets;
mod workspace;

fn main() -> eframe::Result {
    if std::env::args().nth(1).as_deref() == Some("--render") {
        render_cli();
        std::process::exit(0);
    }
    // Parse --profile <name> early so config_dir() resolves correctly for
    // both logging and all downstream I/O.
    let raw_args: Vec<String> = std::env::args().collect();
    let profile = parse_profile_flag(&raw_args);
    crate::config::set_profile(profile);
    crate::config::ensure_profile_initialized();

    // #308 Phase 2: idempotently apply the bundled core pack to a freshly-
    // empty apps dir. `ensure_profile_initialized` already seeds the dir on
    // first profile creation; this catches the secondary case where someone
    // wiped their apps dir but kept the rest of the profile config.
    {
        let apps_dir = crate::app_registry::apps_dir();
        let cloner = crate::install::GitCloner;
        if let Some(outcomes) = crate::install::apply_core_pack_if_empty(&cloner, &apps_dir) {
            let n_ok = outcomes
                .iter()
                .filter(|o| matches!(o.status, crate::install::InstallStatus::Installed(_)))
                .count();
            eprintln!("core pack: applied {} apps to {}", n_ok, apps_dir.display());
            for o in &outcomes {
                if let crate::install::InstallStatus::Failed(msg) = &o.status {
                    eprintln!("core pack: FAILED {}: {msg}", o.id);
                }
            }
        }
    }

    // Adopt an explicit workspace root from `plexi <path>` if one was given.
    // Errors out if the path exists but has no `.plexi/` ancestor — the user
    // can run `plexi workspace init` to create one. Bare `plexi` continues to
    // resolve via CWD-walk later in `PlexiApp::new`.
    let adopted_root = match parse_workspace_path_arg(&raw_args) {
        Ok(root) => root,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    crate::config::set_adopted_workspace_root(adopted_root.clone());

    // When the user runs `plexi <path>`, treat that path as the new CWD so
    // every downstream consumer (AppRegistry, event log, default pane cwd)
    // sees the adopted workspace as its starting point. Mirrors VS Code's
    // "open folder" semantics. Bare `plexi` leaves CWD untouched.
    if let Some(root) = adopted_root.as_ref() {
        if let Err(e) = std::env::set_current_dir(root) {
            eprintln!(
                "warning: failed to chdir into workspace {}: {e}",
                root.display()
            );
        }
    }

    // Merge global config with the workspace's `.plexi/config.toml` if a
    // workspace is in scope. The `log` level needs the merged value so a
    // project-level `[log] level = "debug"` actually takes effect.
    let merged_config_root = adopted_root
        .clone()
        .or_else(|| crate::config::active_workspace_root());
    let log_level = crate::config::PlexiConfig::load_with_workspace(merged_config_root.as_deref())
        .log
        .and_then(|l| l.level_filter())
        .unwrap_or(log::LevelFilter::Info);
    crate::logging::init(log_level);
    let frame_tick = crate::logging::new_frame_tick();
    crate::logging::spawn_heartbeat(frame_tick.clone());

    // Resolve the login-shell PATH once for the whole process. Fixes
    // `gh`/`rg`/`fd`/any-homebrew-tool-not-found bugs when Plexi is launched
    // as a GUI bundle (which inherits only `/usr/bin:/bin:/usr/sbin:/sbin`).
    crate::shell::install_login_shell_path();
    // Adopt API keys and other user secrets set in ~/.zshrc / ~/.zsh_secrets.
    // GUI bundles don't inherit these — OPENROUTER_API_KEY, ANTHROPIC_API_KEY,
    // etc. are invisible without this call.
    crate::shell::install_login_shell_env();

    // One-shot migration from the v3.0 global-namespace secrets index to the
    // workspace-namespaced layout (issue #322). Idempotent: a no-op once the
    // index is in the new flat-string form.
    #[cfg(target_os = "macos")]
    {
        let store = crate::workspace_secrets::MacKeychain::new();
        let migrated = crate::workspace_secrets::migrate_legacy_global_secrets(&store);
        if migrated > 0 {
            log::info!(
                "workspace_secrets: migrated {migrated} legacy global secrets to plexi:user:* namespace"
            );
        }
    }

    // Capture Rust panics into the log file so they survive process death.
    // Without this, panics on the UI thread only appear in Console.app and are
    // invisible to the Plexi log (the log writer thread is killed mid-write).
    {
        let panic_log = crate::logging::log_path();
        std::panic::set_hook(Box::new(move |info| {
            let msg = info.to_string();
            // Best-effort append to the log file directly (logger may be dead).
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&panic_log)
            {
                use std::io::Write;
                let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                let _ = writeln!(f, "[{now}] [ERROR] [plexi::panic] PANIC: {msg}");
            }
            // Also print to stderr so crash logs / Console.app capture it.
            eprintln!("PLEXI PANIC: {msg}");
        }));
    }

    // Handle CLI subcommands before launching the GUI.
    let args: Vec<String> = raw_args
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            // strip --profile and its value from downstream arg parsing
            if a == "--profile" {
                return None;
            }
            if i > 0 && raw_args.get(i - 1).map(|x| x.as_str()) == Some("--profile") {
                return None;
            }
            Some(a.clone())
        })
        .collect();
    if args.len() >= 2 {
        match args[1].as_str() {
            "run" => {
                if args.len() < 3 {
                    eprintln!("Usage: plexi run <command>");
                    std::process::exit(1);
                }
                std::process::exit(cli::run_command(&args[2]));
            }
            "workspace" => {
                if args.len() < 3 {
                    eprintln!("Usage: plexi workspace <init>");
                    std::process::exit(1);
                }
                match args[2].as_str() {
                    "init" => {
                        std::process::exit(cli::workspace_init());
                    }
                    other => {
                        eprintln!("Unknown workspace subcommand: {other}");
                        eprintln!("Usage: plexi workspace <init>");
                        std::process::exit(1);
                    }
                }
            }
            "secret" => {
                if args.len() < 3 {
                    eprintln!("Usage: plexi secret <set|list|delete> [friendly-name]");
                    std::process::exit(1);
                }
                match args[2].as_str() {
                    "set" => {
                        if args.len() < 4 {
                            eprintln!("Usage: plexi secret set <friendly-name>");
                            std::process::exit(1);
                        }
                        std::process::exit(cli::workspace_secret_set(&args[3]));
                    }
                    "list" => {
                        std::process::exit(cli::workspace_secret_list());
                    }
                    "delete" => {
                        if args.len() < 4 {
                            eprintln!("Usage: plexi secret delete <friendly-name>");
                            std::process::exit(1);
                        }
                        std::process::exit(cli::workspace_secret_delete(&args[3]));
                    }
                    other => {
                        eprintln!("Unknown secret subcommand: {other}");
                        eprintln!("Usage: plexi secret <set|list|delete>");
                        std::process::exit(1);
                    }
                }
            }
            "app" => {
                if args.len() < 3 {
                    eprintln!("Usage: plexi app <init|install|uninstall|list> [options]");
                    std::process::exit(1);
                }
                match args[2].as_str() {
                    "init" => {
                        // plexi app init [--lang python|rust] <name>
                        let mut lang = "python";
                        let mut name = "";
                        let mut i = 3;
                        while i < args.len() {
                            if args[i] == "--lang" && i + 1 < args.len() {
                                lang = args[i + 1].as_str();
                                i += 2;
                            } else {
                                name = args[i].as_str();
                                i += 1;
                            }
                        }
                        std::process::exit(cli::app_init(name, lang));
                    }
                    "install" => {
                        if args.len() < 4 {
                            eprintln!("Usage: plexi app install <github-user/repo>");
                            std::process::exit(1);
                        }
                        std::process::exit(cli::app_install(&args[3]));
                    }
                    "uninstall" => {
                        if args.len() < 4 {
                            eprintln!("Usage: plexi app uninstall <id>");
                            std::process::exit(1);
                        }
                        std::process::exit(cli::app_uninstall(&args[3]));
                    }
                    "list" => {
                        std::process::exit(cli::app_list());
                    }
                    other => {
                        eprintln!("Unknown app subcommand: {other}");
                        eprintln!("Usage: plexi app <init|install|uninstall|list>");
                        std::process::exit(1);
                    }
                }
            }
            "install" => {
                // `plexi install <source-spec>[@ref]`
                // `plexi install --pack <path|core>`
                let mut pack: Option<String> = None;
                let mut positional: Option<String> = None;
                let mut i = 2;
                while i < args.len() {
                    match args[i].as_str() {
                        "--pack" if i + 1 < args.len() => {
                            pack = Some(args[i + 1].clone());
                            i += 2;
                        }
                        other if !other.starts_with("--") && positional.is_none() => {
                            positional = Some(other.to_string());
                            i += 1;
                        }
                        _ => {
                            i += 1;
                        }
                    }
                }
                if let Some(p) = pack {
                    std::process::exit(cli::install_pack_cli(&p));
                }
                match positional {
                    Some(spec) => std::process::exit(cli::install_cli(&spec)),
                    None => {
                        eprintln!(
                            "Usage: plexi install <source-spec>[@ref] | plexi install --pack <path|core>"
                        );
                        std::process::exit(1);
                    }
                }
            }
            "uninstall" => {
                if args.len() < 3 {
                    eprintln!("Usage: plexi uninstall <id> [--yes]");
                    std::process::exit(1);
                }
                let mut id: Option<String> = None;
                let mut assume_yes = false;
                for a in &args[2..] {
                    if a == "--yes" || a == "-y" {
                        assume_yes = true;
                    } else if !a.starts_with("--") && id.is_none() {
                        id = Some(a.clone());
                    }
                }
                match id {
                    Some(id) => std::process::exit(cli::uninstall_cli(&id, assume_yes)),
                    None => {
                        eprintln!("Usage: plexi uninstall <id> [--yes]");
                        std::process::exit(1);
                    }
                }
            }
            "update" => {
                let id = args
                    .iter()
                    .skip(2)
                    .find(|a| !a.starts_with("--"))
                    .cloned();
                std::process::exit(cli::update_cli(id.as_deref()));
            }
            "list" => {
                std::process::exit(cli::list_cli());
            }
            "pack" => {
                // `plexi pack export <path>` (#308 Phase 3) — write a
                // pack.toml describing the currently-installed app set.
                if args.len() < 3 {
                    eprintln!("Usage: plexi pack export <path>");
                    std::process::exit(1);
                }
                match args[2].as_str() {
                    "export" => {
                        if args.len() < 4 {
                            eprintln!("Usage: plexi pack export <path>");
                            std::process::exit(1);
                        }
                        std::process::exit(cli::pack_export_cli(&args[3]));
                    }
                    other => {
                        eprintln!("Unknown pack subcommand: {other}");
                        eprintln!("Usage: plexi pack export <path>");
                        std::process::exit(1);
                    }
                }
            }
            "notify" => {
                let mut title = String::new();
                let mut body = String::new();
                let mut level = "info";
                let mut i = 2;
                while i < args.len() {
                    match args[i].as_str() {
                        "--title" if i + 1 < args.len() => {
                            title = args[i + 1].clone();
                            i += 2;
                        }
                        "--body" if i + 1 < args.len() => {
                            body = args[i + 1].clone();
                            i += 2;
                        }
                        "--level" if i + 1 < args.len() => {
                            level = args[i + 1].as_str();
                            i += 2;
                        }
                        _ => {
                            i += 1;
                        }
                    }
                }
                if title.is_empty() {
                    eprintln!(
                        "Usage: plexi notify --title <text> --body <text> [--level info|warn|error]"
                    );
                    std::process::exit(1);
                }
                std::process::exit(cli::notify_cli(&title, &body, level));
            }
            "descriptor" => {
                // `plexi descriptor probe <cmd> [args...] [--no-registry]` —
                // runs `<cmd> [args...] --plexi`, parses the result against
                // the descriptor schema, and prints a human-readable
                // summary. On Tier-1 failure, falls back to the Tier-2
                // registry (issue #321) unless `--no-registry` is set.
                if args.len() < 3 {
                    eprintln!(
                        "Usage: plexi descriptor probe <command> [args...] [--no-registry]"
                    );
                    std::process::exit(1);
                }
                match args[2].as_str() {
                    "probe" => {
                        if args.len() < 4 {
                            eprintln!(
                                "Usage: plexi descriptor probe <command> [args...] [--no-registry]"
                            );
                            std::process::exit(1);
                        }
                        let cmd = &args[3];
                        // Split the tail into positional args + flags. `--no-registry`
                        // is the only flag we consume; anything else is forwarded
                        // verbatim to the target command.
                        let mut use_registry = true;
                        let mut extra: Vec<&str> = Vec::new();
                        for a in &args[4..] {
                            if a == "--no-registry" {
                                use_registry = false;
                            } else {
                                extra.push(a.as_str());
                            }
                        }
                        let runner = cli::descriptor::RealRunner;
                        let opts = cli::descriptor::ProbeOptions { use_registry };
                        std::process::exit(cli::descriptor::probe_with_options(
                            &runner, cmd, &extra, &opts,
                        ));
                    }
                    other => {
                        eprintln!("Unknown descriptor subcommand: {other}");
                        eprintln!(
                            "Usage: plexi descriptor probe <command> [args...] [--no-registry]"
                        );
                        std::process::exit(1);
                    }
                }
            }
            "registry" => {
                // `plexi registry watch [<cli>]` — diff installed CLIs
                // against their registered descriptors. Issue #321 substrate.
                if args.len() < 3 {
                    eprintln!("Usage: plexi registry watch [<cli>]");
                    std::process::exit(1);
                }
                match args[2].as_str() {
                    "watch" => {
                        let only = args.get(3).map(|s| s.as_str());
                        std::process::exit(cli::registry::watch_cli(only));
                    }
                    other => {
                        eprintln!("Unknown registry subcommand: {other}");
                        eprintln!("Usage: plexi registry watch [<cli>]");
                        std::process::exit(1);
                    }
                }
            }
            _ => {} // Not a CLI subcommand — fall through to GUI
        }
    }

    // Plexi-in-Plexi detection: if already running inside a Plexi terminal, don't
    // launch a second GUI — just report the nearest .plexi/ workspace.
    if std::env::var("PLEXI_RUNNING").as_deref() == Ok("1") {
        let cwd = std::env::current_dir().unwrap_or_default();
        let home = dirs::home_dir().unwrap_or_default();
        let mut dir = cwd.as_path();
        loop {
            if dir.join(".plexi").is_dir() {
                eprintln!(
                    "plexi: already running inside Plexi. Nearest workspace: {}",
                    dir.join(".plexi").display()
                );
                std::process::exit(0);
            }
            if dir == home || dir.parent().is_none() {
                break;
            }
            dir = dir.parent().unwrap();
        }
        eprintln!("plexi: already running inside Plexi. Use Cmd+T to open a new pane.");
        std::process::exit(0);
    }

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/app-icon.png"))
        .expect("failed to load app icon");

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([400.0, 300.0])
            .with_title(env!("PLEXI_APP_TITLE"))
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        env!("CARGO_PKG_NAME"),
        native_options,
        Box::new(|cc| Ok(Box::new(app::PlexiApp::new(cc, frame_tick)))),
    )
}

/// Scan argv for the first positional that points at an existing directory
/// — the `plexi <path>` "open folder" arg, modelled on VS Code. Returns
/// `Ok(Some(workspace_root))` when an ancestor `.plexi/` is found,
/// `Ok(None)` when no path arg was given, and `Err(_)` when the user passed
/// a path that has no `.plexi/` workspace anywhere up the tree.
///
/// Anything starting with `--` is skipped (flags) and so are values that
/// follow a known value-bearing flag (`--profile`). Recognized CLI
/// subcommands (`run`, `secret`, `app`, `workspace`, `notify`) are also
/// skipped — those are dispatched separately later in `main`.
fn parse_workspace_path_arg(args: &[String]) -> Result<Option<std::path::PathBuf>, String> {
    const SUBCOMMANDS: &[&str] = &[
        "run",
        "secret",
        "app",
        "workspace",
        "notify",
        "--render",
        // #308 Phase 2 — top-level package manager subcommands
        "install",
        "uninstall",
        "update",
        "list",
        "pack",
        // #188 — `plexi descriptor probe <cmd>` for the --plexi standard.
        "descriptor",
        // #321 — `plexi registry watch [<cli>]` for the CLI wrapper registry.
        "registry",
    ];
    let mut iter = args.iter().enumerate();
    // Skip argv[0] (binary name).
    let _ = iter.next();
    while let Some((_, a)) = iter.next() {
        if a == "--profile" || a == "--lang" || a == "--title" || a == "--body" || a == "--level" {
            // Skip the value paired with this flag.
            let _ = iter.next();
            continue;
        }
        if a.starts_with("--") {
            continue;
        }
        if SUBCOMMANDS.contains(&a.as_str()) {
            return Ok(None);
        }
        // First positional that isn't a known subcommand — interpret as a
        // workspace path.
        let candidate = std::path::PathBuf::from(a);
        if !candidate.exists() {
            return Err(format!(
                "workspace path does not exist: {}",
                candidate.display()
            ));
        }
        if !candidate.is_dir() {
            return Err(format!(
                "workspace path is not a directory: {}",
                candidate.display()
            ));
        }
        let canonical = match candidate.canonicalize() {
            Ok(p) => p,
            Err(e) => return Err(format!("canonicalize {}: {e}", candidate.display())),
        };
        match crate::app_registry::resolve_workspace_root(&canonical) {
            Some(root) => return Ok(Some(root)),
            None => {
                return Err(format!(
                    "no .plexi/ workspace found at or above {}.\n\
                     Run `plexi workspace init` in that directory first.",
                    canonical.display()
                ));
            }
        }
    }
    Ok(None)
}

/// Scan argv for `--profile <name>`. Returns the name if present.
fn parse_profile_flag(args: &[String]) -> Option<String> {
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        if a == "--profile" {
            return iter.next().cloned();
        }
        if let Some(rest) = a.strip_prefix("--profile=") {
            return Some(rest.to_string());
        }
    }
    None
}

/// Headless render subcommand: reads `{viewport, draw_commands}` JSON from stdin,
/// writes PNG bytes to stdout. Used by the Python snapshot test harness.
/// Invoked via `plexi --render`. Zero GUI init cost.
fn render_cli() {
    use std::io::{Read, Write};
    let mut input = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut input) {
        eprintln!("plexi --render: failed to read stdin: {e}");
        std::process::exit(1);
    }
    let req: serde_json::Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("plexi --render: invalid JSON: {e}");
            std::process::exit(1);
        }
    };
    let viewport = &req["viewport"];
    let width = viewport["width"].as_u64().unwrap_or(800) as u32;
    let height = viewport["height"].as_u64().unwrap_or(600) as u32;
    let background = viewport["background"].as_str().unwrap_or("#000000");
    let commands: Vec<serde_json::Value> = match req["draw_commands"].as_array() {
        Some(arr) => arr.clone(),
        None => {
            eprintln!("plexi --render: 'draw_commands' must be a JSON array");
            std::process::exit(1);
        }
    };
    let mut all_commands = Vec::with_capacity(commands.len() + 1);
    all_commands.push(serde_json::json!({
        "type": "rect",
        "x": 0.0,
        "y": 0.0,
        "w": width as f64,
        "h": height as f64,
        "fill": background,
        "radius": 0.0,
    }));
    all_commands.extend(commands);
    let renderer = crate::headless_renderer::HeadlessRenderer::new();
    let png_bytes = renderer.render_pgap_frame(&all_commands, width, height);
    if let Err(e) = std::io::stdout().write_all(&png_bytes) {
        eprintln!("plexi --render: failed to write PNG to stdout: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod cli_tests {
    use super::parse_workspace_path_arg;
    use std::fs;

    fn argv(parts: &[&str]) -> Vec<String> {
        std::iter::once("plexi")
            .chain(parts.iter().copied())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn plexi_path_arg_adopts_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join(".plexi")).unwrap();
        let path_str = workspace.path().to_string_lossy().to_string();

        let resolved = parse_workspace_path_arg(&argv(&[path_str.as_str()]))
            .expect("path arg should resolve")
            .expect("workspace root should be Some");
        assert_eq!(
            resolved.canonicalize().unwrap(),
            workspace.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn plexi_path_arg_with_no_dotplexi_errors() {
        let bare = tempfile::tempdir().unwrap();
        let path_str = bare.path().to_string_lossy().to_string();

        let err = parse_workspace_path_arg(&argv(&[path_str.as_str()]))
            .expect_err("missing .plexi/ should error");
        assert!(
            err.contains("no .plexi/ workspace found"),
            "expected workspace-not-found error, got: {err}"
        );
    }

    #[test]
    fn plexi_path_arg_skips_known_subcommands() {
        // `plexi workspace init` must NOT be interpreted as a path arg.
        let resolved = parse_workspace_path_arg(&argv(&["workspace", "init"]))
            .expect("subcommand path should resolve");
        assert!(resolved.is_none());
    }

    #[test]
    fn plexi_with_no_args_returns_none() {
        let resolved = parse_workspace_path_arg(&argv(&[]))
            .expect("no args should resolve");
        assert!(resolved.is_none());
    }

    #[test]
    fn plexi_path_arg_skips_profile_flag_value() {
        // `plexi --profile alpha` must not treat "alpha" as a path.
        let resolved = parse_workspace_path_arg(&argv(&["--profile", "alpha"]))
            .expect("flag-only argv should resolve");
        assert!(resolved.is_none());
    }
}
