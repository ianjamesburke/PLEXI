#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Ship-time panic-path protection. `todo!()` / `unimplemented!()` compile clean
// but panic at runtime — e.g. 2026-04-18, CoreAudioDevice::start_capture was
// `todo!()` and froze the GUI when a recorder app sent AudioCapture without
// PLEXI_AUDIO=mock://. Stubs must return `Err(NotImplemented)` instead.
#![deny(clippy::todo, clippy::unimplemented)]

mod anchor;
mod app;
mod app_permissions;
mod app_protocol;
mod app_render;
mod app_registry;
mod app_registry_watcher;
mod app_trait;
mod audio;
mod cli;

mod command_palette;
mod config;
mod config_watcher;
mod context;
mod context_state;
mod event_log;
mod features;
mod file_browser;
mod host;
mod keys;
mod launch_failed;
mod logging;
#[cfg(target_os = "macos")]
mod finder_service;
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
mod render_components;
mod process_app;
mod headless_renderer;
mod hot_reload;
mod install;
mod protocol;
mod runs;
mod secrets;
mod secrets_app;
mod workspace_router;
mod workspace_secrets;
mod scheduler;
mod shell;
mod minimap;
mod sidebar;
mod sidebar_row;
mod spatial;
mod style;
mod theme;
mod tiling;
mod typed_pipes;
mod updater;
mod video;
mod widgets;
mod workspace;
#[cfg(test)]
mod testing;

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
    let is_first_launch = crate::config::ensure_profile_initialized();

    {
        let apps_dir = crate::app_registry::apps_dir();
        let cloner = crate::install::GitCloner;

        // On first launch, also seed the example apps.
        if is_first_launch {
            if let Some(outcomes) =
                crate::install::apply_examples_pack_if_empty(&cloner, &apps_dir)
            {
                let n_ok = outcomes
                    .iter()
                    .filter(|o| matches!(o.status, crate::install::InstallStatus::Installed(_)))
                    .count();
                log::info!("examples pack: seeded {n_ok} apps to {}", apps_dir.display());
            }
        }

        // Always re-seed any deleted core apps.
        let core_outcomes = crate::install::apply_core_pack_always(&cloner, &apps_dir);
        let n_installed = core_outcomes
            .iter()
            .filter(|o| matches!(o.status, crate::install::InstallStatus::Installed(_)))
            .count();
        if n_installed > 0 {
            log::info!(
                "core pack: re-seeded {n_installed} missing apps to {}",
                apps_dir.display()
            );
        }
        for o in &core_outcomes {
            if let crate::install::InstallStatus::Failed(msg) = &o.status {
                log::warn!("core pack: FAILED {}: {msg}", o.id);
            }
        }
    }

    // Adopt an explicit workspace root from `plexi <path>` if one was given.
    // If the path has no `.plexi/` ancestor, an adopted context path is set
    // instead — the directory opens as a new context on first frame.
    // Bare `plexi` continues to resolve via CWD-walk later in `PlexiApp::new`.
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
    let log_config = crate::config::PlexiConfig::load_with_workspace(merged_config_root.as_deref())
        .log
        .unwrap_or_default();
    let log_level = log_config.level_filter().unwrap_or(log::LevelFilter::Info);
    let retention_days = log_config.retention_days.unwrap_or(30);
    let cli_mode = raw_args.iter().skip(1).any(|a| {
        !a.starts_with('-') && known_subcommands().contains(a.as_str())
    });
    crate::logging::init(log_level, retention_days, cli_mode);
    let frame_tick = crate::logging::new_frame_tick();
    // Note: spawn_heartbeat is deferred to just before eframe::run_native so
    // the shell probes below don't trigger false FREEZE alerts. The heartbeat
    // should only monitor actual eframe operation, not pre-startup work.


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
    use crate::cli::args::{Cli, Commands, WorkspaceCmd, SecretCmd, AppCmd, UpdateCmd, PaneCmd, DescriptorCmd, RegistryCmd, ContextCmd, ConfigCmd, RoutineCmd, NotesCmd};
    use clap::Parser;

    match Cli::try_parse_from(&args) {
        Ok(cli) => {
            if let Some(cmd) = cli.command {
                match cmd {
                    Commands::Run { command } => match command {
                        Some(cmd) => std::process::exit(cli::run_command(&cmd)),
                        None => std::process::exit(cli::run_list_commands()),
                    },
                    Commands::Workspace { cmd } => match cmd {
                        WorkspaceCmd::Init => std::process::exit(cli::workspace_init()),
                    },
                    Commands::Routine { cmd } => match cmd {
                        RoutineCmd::List => std::process::exit(cli::routine_list()),
                        RoutineCmd::Run { name } => std::process::exit(cli::routine_run(&name)),
                    },
                    Commands::Secret { cmd } => match cmd {
                        SecretCmd::Set { friendly_name, from_env, global, alias } => std::process::exit(cli::workspace_secret_set(&friendly_name, from_env, global, alias.as_deref())),
                        SecretCmd::Get { friendly_name, global } => std::process::exit(cli::workspace_secret_get(&friendly_name, global)),
                        SecretCmd::List => std::process::exit(cli::workspace_secret_list()),
                        SecretCmd::Delete { friendly_name } => std::process::exit(cli::workspace_secret_delete(&friendly_name)),
                    },
                    Commands::App { cmd } => match cmd {
                        AppCmd::Open { type_id, mcp, cli: cli_flag, layout, from_pane_id, extra_args } => {
                            let mode_count = type_id.is_some() as u8
                                + (!mcp.is_empty()) as u8
                                + cli_flag.is_some() as u8;
                            if mode_count == 0 {
                                eprintln!("error: one of TYPE_ID, --mcp, or --cli is required");
                                std::process::exit(2);
                            }
                            if mode_count > 1 {
                                eprintln!("error: TYPE_ID, --mcp, and --cli are mutually exclusive");
                                std::process::exit(2);
                            }
                            if let Some(tid) = type_id {
                                log::info!("app_open:cli: opening app type_id={tid}");
                                std::process::exit(cli::open_cli(&tid, &extra_args, layout.as_deref(), from_pane_id, None));
                            } else if !mcp.is_empty() {
                                log::info!("app_open:cli: launching mcp-renderer with command {:?}", mcp);
                                std::process::exit(cli::open_cli("mcp-renderer", &mcp, layout.as_deref(), from_pane_id, None));
                            } else {
                                let binary = cli_flag.unwrap();
                                log::info!("app_open:cli: running --help parser for `{binary}`");
                                match crate::cli::help_parser::parse_help_to_descriptor(&binary) {
                                    Ok(json) => {
                                        let id = uuid::Uuid::new_v4();
                                        let tmp = std::env::temp_dir()
                                            .join(format!("plexi-descriptor-{id}.json"));
                                        if let Err(e) = std::fs::write(&tmp, &json) {
                                            eprintln!("error: could not write descriptor temp file: {e}");
                                            std::process::exit(1);
                                        }
                                        let path = tmp.to_string_lossy().into_owned();
                                        log::info!("app_open:cli: launching descriptor-renderer with descriptor at {path}");
                                        std::process::exit(cli::open_cli(
                                            "descriptor-renderer",
                                            &[path],
                                            layout.as_deref(),
                                            from_pane_id,
                                            None,
                                        ));
                                    }
                                    Err(e) => {
                                        eprintln!("error: could not parse --help output: {e}");
                                        std::process::exit(1);
                                    }
                                }
                            }
                        }
                        AppCmd::Install { spec_or_path, pack } => {
                            if let Some(p) = pack {
                                log::info!("app_install:cli: pack={p}");
                                std::process::exit(cli::install_pack_cli(&p));
                            }
                            match spec_or_path {
                                None => {
                                    log::info!("app_install:cli: workspace pack (no args)");
                                    std::process::exit(cli::install_workspace_pack_cli());
                                }
                                Some(s) => {
                                    // Local path: contains a path separator, starts with . or /, or is an existing directory.
                                    // Using is_dir() (not exists()) avoids misrouting bare app IDs that happen
                                    // to match a file in the current directory.
                                    let is_local = s.contains('/') || s.starts_with('.') || std::path::Path::new(&s).is_dir();
                                    if is_local {
                                        log::info!("app_install:cli: local path={s}");
                                        std::process::exit(cli::app_install(&s));
                                    } else {
                                        log::info!("app_install:cli: remote spec={s}");
                                        std::process::exit(cli::install_cli(&s));
                                    }
                                }
                            }
                        }
                        AppCmd::Init { name, lang, from_pane_id } => std::process::exit(cli::app_init(&name, &lang, from_pane_id)),
                        AppCmd::Uninstall { id, yes } => std::process::exit(cli::app_uninstall(&id, yes)),
                        AppCmd::List => std::process::exit(cli::app_list()),
                        AppCmd::Render { id, size, state, output } => {
                            std::process::exit(cli::app_render(&id, &size, state.as_deref(), output.as_deref()))
                        }
                        AppCmd::Info { id } => std::process::exit(cli::app_info(&id)),
                        AppCmd::Run { path, from_pane_id } => std::process::exit(cli::app_run(&path, from_pane_id)),
                        AppCmd::Validate { path } => {
                            log::info!("app_validate:cli: path={path}");
                            std::process::exit(cli::validate_cli(&path));
                        }
                        AppCmd::Freeze { path } => {
                            log::info!("app_freeze:cli: path={path}");
                            std::process::exit(cli::freeze_cli(&path));
                        }
                    },
                    Commands::Uninstall { keep_data, yes } => std::process::exit(cli::plexi_uninstall_cli(keep_data, yes)),
                    Commands::Update { subcommand } => match subcommand {
                        Some(UpdateCmd::Apps { id }) => std::process::exit(cli::update_cli(id.as_deref())),
                        None => std::process::exit(cli::self_update_cli()),
                    },
                    Commands::Notify { title, body, level, choices, host_actions, timeout, scope } => {
                        // Parse --host-action flags into a key → "action_type:action_arg" map.
                        let mut host_action_map: std::collections::HashMap<String, String> =
                            std::collections::HashMap::new();
                        for raw in &host_actions {
                            let parts: Vec<&str> = raw.splitn(3, ':').collect();
                            match parts.as_slice() {
                                [key, action_type, action_arg] => {
                                    host_action_map.insert(
                                        key.to_string(),
                                        format!("{action_type}:{action_arg}"),
                                    );
                                }
                                _ => {
                                    let msg = format!(
                                        "error: --host-action requires 3 colon-separated segments \
                                         (key:action_type:action_arg) — got {:?}",
                                        raw
                                    );
                                    log::warn!("notify:cli: {msg}");
                                    eprintln!("{msg}");
                                    std::process::exit(1);
                                }
                            }
                        }
                        let mut parsed_choices: Vec<(String, String, Option<String>)> = Vec::new();
                        for raw in &choices {
                            match cli::parse_notify_choice(raw) {
                                Ok((key, label, existing_action)) => {
                                    // --host-action overrides any action embedded in --choice.
                                    let action = host_action_map
                                        .remove(&key)
                                        .map(Some)
                                        .unwrap_or(existing_action);
                                    parsed_choices.push((key, label, action));
                                }
                                Err(msg) => {
                                    log::warn!("notify:cli: {msg}");
                                    eprintln!("{msg}");
                                    std::process::exit(1);
                                }
                            }
                        }
                        // Any --host-action keys not matched to a --choice are an error.
                        for (key, _) in &host_action_map {
                            let msg = format!(
                                "error: --host-action key {:?} does not match any --choice key",
                                key
                            );
                            log::warn!("notify:cli: {msg}");
                            eprintln!("{msg}");
                            std::process::exit(1);
                        }
                        let parsed_scope: Option<crate::app_protocol::NotifyScope> = match scope.as_deref() {
                            None | Some("global") => None,
                            Some("window") => Some(crate::app_protocol::NotifyScope::Window),
                            Some("context") => Some(crate::app_protocol::NotifyScope::Context),
                            Some(other) => {
                                let msg = format!(
                                    "error: --scope must be window, context, or global — got {:?}",
                                    other
                                );
                                log::warn!("notify:cli: {msg}");
                                eprintln!("{msg}");
                                std::process::exit(1);
                            }
                        };
                        log::info!(
                            "notify:cli: host_actions={} merged into choices",
                            host_actions.len()
                        );
                        std::process::exit(cli::notify_cli(&title, &body, &level, &parsed_choices, timeout, parsed_scope));
                    }
                    Commands::Pane { cmd } => match cmd {
                        PaneCmd::Name { first, second } => {
                            let (pane_id, name) = match second {
                                Some(title) => match first.parse::<u64>() {
                                    Ok(id) => (Some(id), title),
                                    Err(_) => {
                                        eprintln!("error: expected a numeric pane ID as first argument, got {:?}", first);
                                        std::process::exit(1);
                                    }
                                },
                                None => (None, first),
                            };
                            std::process::exit(cli::pane_set_title_cli(pane_id, &name))
                        }
                        PaneCmd::SetTitle { first, second } => {
                            eprintln!("warning: `pane set-title` is deprecated — use `pane name` instead");
                            let (pane_id, name) = match second {
                                Some(title) => match first.parse::<u64>() {
                                    Ok(id) => (Some(id), title),
                                    Err(_) => {
                                        eprintln!("error: expected a numeric pane ID as first argument, got {:?}", first);
                                        std::process::exit(1);
                                    }
                                },
                                None => (None, first),
                            };
                            std::process::exit(cli::pane_set_title_cli(pane_id, &name))
                        }
                        PaneCmd::List { context } => {
                            let (context_id, current) = match context.as_deref() {
                                None => (None, false),
                                Some("current") => (None, true),
                                Some(s) => match s.parse::<u64>() {
                                    Ok(id) => (Some(id), false),
                                    Err(_) => {
                                        eprintln!("error: --context value must be a numeric context ID or omitted for current context");
                                        std::process::exit(1);
                                    }
                                },
                            };
                            std::process::exit(cli::pane_list_cli(context_id, current))
                        }
                        PaneCmd::Focus { pane_id } => std::process::exit(cli::pane_focus_cli(pane_id)),
                        PaneCmd::Close { pane_id } => {
                            let id = match pane_id {
                                Some(id) => id,
                                None => {
                                    let s = match std::env::var("PLEXI_PANE_ID") {
                                        Ok(s) => s,
                                        Err(_) => {
                                            eprintln!("error: PLEXI_PANE_ID is not set — run this inside a Plexi terminal pane or pass a pane ID explicitly");
                                            std::process::exit(1);
                                        }
                                    };
                                    match s.parse::<u64>() {
                                        Ok(id) => id,
                                        Err(_) => {
                                            eprintln!("error: PLEXI_PANE_ID is not a valid number: {s}");
                                            std::process::exit(1);
                                        }
                                    }
                                }
                            };
                            log::info!("pane_close:cli: pane_id={id}");
                            std::process::exit(cli::pane_close_cli(id));
                        }
                        PaneCmd::Send { pane_id, text } => std::process::exit(cli::pane_send_cli(pane_id, &text)),
                        PaneCmd::Key { pane_id, key } => std::process::exit(cli::pane_key_cli(pane_id, &key)),
                        PaneCmd::Self_ => std::process::exit(cli::pane_self_cli()),
                        PaneCmd::Info => std::process::exit(cli::pane_info_cli()),
                        PaneCmd::Capture { pane_id, lines, full_output, from_cursor } => std::process::exit(cli::pane_capture_cli(pane_id, lines, full_output, from_cursor)),
                    },
                    Commands::Terminal { cmd, ephemeral, layout, from_pane_id, cwd, no_focus } => {
                        std::process::exit(cli::terminal_cli(cmd.as_deref(), ephemeral, layout.as_deref(), from_pane_id, cwd.as_deref(), no_focus));
                    }
                    Commands::Descriptor { cmd } => match cmd {
                        DescriptorCmd::Probe { command, no_registry, no_crawl, json, extra_args } => {
                            if json {
                                match crate::cli::help_parser::parse_help_to_descriptor(&command) {
                                    Ok(j) => { println!("{j}"); std::process::exit(0); }
                                    Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
                                }
                            }
                            let runner = cli::descriptor::RealRunner;
                            let opts = cli::descriptor::ProbeOptions { use_registry: !no_registry, use_crawl: !no_crawl };
                            let extra: Vec<&str> = extra_args.iter().map(|s| s.as_str()).collect();
                            std::process::exit(cli::descriptor::probe_with_options(&runner, &command, &extra, &opts));
                        }
                    },
                    Commands::Registry { cmd } => match cmd {
                        RegistryCmd::Watch { cli: only } => {
                            std::process::exit(cli::registry::watch_cli(only.as_deref()));
                        }
                    },
                    Commands::Context { cmd } => match cmd {
                        ContextCmd::New { name, path, parent } => std::process::exit(cli::context_new_cli(name.as_deref(), path.as_deref(), parent.as_deref())),
                        ContextCmd::Open { path } => std::process::exit(cli::context_open_cli(path.as_deref())),
                        ContextCmd::SetRoot { path } => std::process::exit(cli::context_set_root_cli(path.as_deref())),
                        ContextCmd::Current => std::process::exit(cli::context_current_cli()),
                        ContextCmd::Describe { text } => std::process::exit(cli::context_describe_cli(&text)),
                        ContextCmd::Zoom { context_id } => std::process::exit(cli::context_zoom_cli(context_id)),
                        ContextCmd::ZoomOut => std::process::exit(cli::context_zoom_out_cli()),
                    },
                    Commands::Completions { shell } => {
                        let s = shell.as_deref().unwrap_or("zsh");
                        let binary_name = std::env::args()
                            .next()
                            .as_deref()
                            .and_then(|p| std::path::Path::new(p).file_name())
                            .and_then(|n| n.to_str())
                            .unwrap_or("plexi")
                            .to_string();
                        std::process::exit(cli::completions_cli(s, &binary_name));
                    }
                    Commands::Config { cmd: config_cmd } => match config_cmd {
                        ConfigCmd::Check => {
                            std::process::exit(cli::config_check());
                        }
                        ConfigCmd::Edit => {
                            std::process::exit(cli::config_edit());
                        }
                        ConfigCmd::Get { key } => {
                            std::process::exit(cli::config_get(&key));
                        }
                        ConfigCmd::Reset => {
                            std::process::exit(cli::config_reset());
                        }
                    },
                    Commands::Notes { cmd } => match cmd {
                        Some(NotesCmd::List) | None => std::process::exit(cli::notes_list_cli()),
                        Some(NotesCmd::Open) => std::process::exit(cli::notes_open_cli()),
                    },
                    Commands::Demo => std::process::exit(cli::demo_cli()),
                }
            }
            // No subcommand — fall through to workspace path check, then GUI
        }
        Err(e) => {
            // --help and --version print and exit 0/2 through this path
            e.exit();
        }
    }

    // Plexi-in-Plexi detection: if already running inside a Plexi terminal,
    // show help rather than attempting to launch a second GUI.
    if std::env::var("PLEXI_RUNNING").as_deref() == Ok("1") {
        use clap::CommandFactory;
        let _ = Cli::command().print_help();
        println!();
        std::process::exit(0);
    }

    // When invoked with no arguments outside Plexi, print a brief launch
    // notice so the terminal isn't silent for first-time users (#1515).
    if !cli_mode && adopted_root.is_none() {
        println!("Starting Plexi — run 'plexi --help' to see available commands.");
    }

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/app-icon.png"))
        .expect("failed to load app icon");

    // Resolve the login-shell PATH and adopt API keys only for the GUI.
    // CLI subcommands already inherit the full shell environment from the
    // calling terminal — running this there corrupts terminal signal state
    // (zsh -i hijacks SIGINT) and spams the user's stdout with log noise.
    crate::shell::install_login_shell_path();
    crate::shell::install_login_shell_env();

    // Shell probes are done. Start the heartbeat now so it only monitors
    // eframe operation — not pre-startup shell work (#588).
    crate::logging::spawn_heartbeat(frame_tick.clone());


    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([400.0, 300.0])
            .with_title(env!("PLEXI_APP_TITLE"))
            .with_icon(icon)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false),
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
/// `Ok(None)` when no path arg was given or the directory has no `.plexi/`
/// ancestor (in which case an adopted context path is set via
/// `config::set_adopted_context_path`), and `Err(_)` only for invalid paths.
///
/// Anything starting with `--` is skipped (flags) and so are values that
/// follow a known value-bearing flag (`--profile`). Recognized CLI
/// subcommands (`run`, `secret`, `app`, `workspace`, `notify`) are also
/// skipped — those are dispatched separately later in `main`.
fn parse_workspace_path_arg(args: &[String]) -> Result<Option<std::path::PathBuf>, String> {
    let known = known_subcommands();
    let mut iter = args.iter().enumerate();
    // Skip argv[0] (binary name).
    let _ = iter.next();
    while let Some((_, a)) = iter.next() {
        if a == "--profile" || a == "--lang" || a == "--title" || a == "--body" || a == "--level" {
            // Skip the value paired with this flag.
            let _ = iter.next();
            continue;
        }
        if a.starts_with('-') {
            continue;
        }
        if known.contains(a.as_str()) {
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
                crate::config::set_adopted_context_path(canonical);
                return Ok(None);
            }
        }
    }
    Ok(None)
}

/// Returns the set of known top-level subcommand names and aliases, cached after first call.
fn known_subcommands() -> &'static std::collections::HashSet<String> {
    use clap::CommandFactory;
    use std::sync::OnceLock;
    static INSTANCE: OnceLock<std::collections::HashSet<String>> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        crate::cli::args::Cli::command()
            .get_subcommands()
            .flat_map(|c| {
                std::iter::once(c.get_name().to_string())
                    .chain(c.get_all_aliases().map(str::to_string))
            })
            .collect()
    })
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
    fn plexi_path_arg_with_no_dotplexi_sets_context_path() {
        let bare = tempfile::tempdir().unwrap();
        let path_str = bare.path().to_string_lossy().to_string();

        let resolved = parse_workspace_path_arg(&argv(&[path_str.as_str()]))
            .expect("non-workspace dir should not error");
        assert!(
            resolved.is_none(),
            "non-workspace dir should return None for workspace root"
        );
        let ctx_path = crate::config::take_adopted_context_path();
        assert!(
            ctx_path.is_some(),
            "adopted context path should be set for non-workspace dir"
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

    #[test]
    fn plexi_path_arg_skips_short_flags() {
        // `plexi -h` must not be treated as a workspace path (fixes issue #1747).
        let resolved = parse_workspace_path_arg(&argv(&["-h"]))
            .expect("-h should not error");
        assert!(resolved.is_none());

        let resolved = parse_workspace_path_arg(&argv(&["-V"]))
            .expect("-V should not error");
        assert!(resolved.is_none());
    }
}
