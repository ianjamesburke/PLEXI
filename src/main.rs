#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// `delete_context` drops windows on a worker thread. Proving `Window: Send`
// walks the wgpu resource graph, and the combined external-turn + ledger assistant
// state overflows rustc's default recursion limit in test builds.
#![recursion_limit = "256"]
// Ship-time panic-path protection. `todo!()` / `unimplemented!()` compile clean
// but panic at runtime — e.g. 2026-04-18, the audio capture entry point was
// `todo!()` and froze the GUI when a recorder plugin sent AudioCapture without
// PLEXI_AUDIO=mock://. Stubs must return `Err(NotImplemented)` instead.
#![deny(clippy::todo, clippy::unimplemented)]
// Single-authority input ownership (stint 0429): egui focus is a projection
// of the host-derived InputOwner. Direct focus calls are banned everywhere
// except the reconciler (src/app/input_owner.rs) — see clippy.toml.
#![deny(clippy::disallowed_methods)]

mod agent;
mod app;
mod assistant;
mod broker;
mod cli;
mod cloud;

mod config;
mod distribution;
mod editor;
mod file_browser;
mod host;
mod mcp_http;
mod media;
pub mod notes;
mod overlays;
mod pane_ops;
mod platform;
mod plexi_ai;
mod program_status;
mod protocol;
mod release;
mod render;
mod rpc;
#[cfg(test)]
mod scenes;
mod search;
mod spatial;
pub(crate) mod strip;
#[cfg(test)]
mod testing;
mod ui;
#[cfg(test)]
mod ui_tests;
mod workspace;

fn main() -> eframe::Result {
    config::profile_home_or_exit();
    let process_start = std::time::Instant::now();
    let raw_args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    // Installer and renderer entry points are exact argv shapes, outside
    // ordinary command dispatch.
    let entry_point = match raw_args.get(1..) {
        Some([flag]) => ["--build-info", "--distribution-check", "--render"]
            .into_iter()
            .find(|entry| flag == entry),
        _ => None,
    };
    // The one reader of argv. Everything below acts on its outcome.
    let invocation = entry_point.is_none().then(|| cli::front::parse(&raw_args));
    let host_startup = matches!(
        invocation.as_ref().map(|i| &i.outcome),
        Some(cli::front::Outcome::Launch(_))
    );
    #[cfg(unix)]
    if let Err(error) = configure_cli_sigpipe(host_startup) {
        eprintln!("CLI SIGPIPE setup failed: {error}");
        cli::trace::exit(1);
    }
    match entry_point {
        Some("--render") => {
            render_cli();
            cli::trace::exit(0);
        }
        Some(flag) => {
            if flag == "--distribution-check" {
                if let Err(error) = distribution::check_runtime() {
                    eprintln!("package runtime check: {error}");
                    cli::trace::exit(1);
                }
            }
            println!("{}", distribution::build_info());
            cli::trace::exit(0);
        }
        None => {}
    }
    let Some(cli::front::Invocation { bin, outcome }) = invocation else {
        unreachable!("every argv without an entry point has an invocation")
    };
    // Display outcomes and usage errors exit here: no profile read, no
    // initialization, no tracing, no host contact.
    let startup = match outcome {
        cli::front::Outcome::Help(help) => {
            crate::config::set_profile(help.selection.clone());
            cli::trace::exit(cli::front::emit_help(&bin, &help))
        }
        cli::front::Outcome::Version => cli::trace::exit(cli::front::emit_version(&bin)),
        cli::front::Outcome::UsageError(error) => {
            cli::trace::exit(cli::front::emit_usage_error(&bin, &error))
        }
        cli::front::Outcome::Completions { shell } => cli::trace::exit(cli::completions_cli(
            shell.as_deref().unwrap_or("zsh"),
            &bin,
        )),
        cli::front::Outcome::Run(run) => Startup::Run(run),
        cli::front::Outcome::Launch(launch) => Startup::Launch(launch),
    };
    // Install the validated selection, then settle the caller's identity
    // against the endpoint this command addresses, before profile/workspace
    // setup or any pane default reads inherited identity. This is still
    // single-threaded process startup.
    crate::config::set_profile(match &startup {
        Startup::Run(run) => run.selection.clone(),
        Startup::Launch(launch) => launch.selection.clone(),
    });
    // A lifecycle command addresses its validated profile host and nothing
    // else: a refused target exits here, before any connection or trace.
    let lifecycle_socket = match &startup {
        Startup::Run(run)
            if run
                .cli
                .command
                .as_ref()
                .is_some_and(cli::host::addresses_profile_host) =>
        {
            match cli::host::host_target() {
                Ok(target) => Some(target.socket()),
                Err(error) => {
                    eprintln!("WARN host: {error}");
                    cli::trace::exit(1);
                }
            }
        }
        _ => None,
    };
    let dropped_identity = match &startup {
        Startup::Run(run) => {
            cli::settle_caller_identity(run.cli.socket.as_deref(), lifecycle_socket.as_deref())
        }
        Startup::Launch(_) => Vec::new(),
    };
    let boot_state = crate::workspace::consume_host_boot_state();

    fn exit_if_feature_disabled(feature: crate::release::ReleaseFeature) {
        if !crate::release::feature_enabled(feature) {
            eprintln!(
                "error: {}",
                crate::release::feature_unavailable_message(feature)
            );
            cli::trace::exit(1);
        }
    }

    let surface_path: &[String] = match &startup {
        Startup::Run(run) => &run.surface.path,
        Startup::Launch(_) => &[],
    };
    let is_plugin_prune_command = surface_path.starts_with(&["plugin".into(), "prune".into()]);

    let cli_mode = !host_startup;
    // Classify the launch before the profile is touched. A CLI command
    // initializes the profile only when no host of another build runs on it
    // (stint 1609). A host launch takes the initialization lock here and
    // claims the profile under it below, before it initializes anything.
    let profile_init_failed = |error: crate::config::ProfileInitError| -> ! {
        let show_window = app::startup_error::shows_window(!cli_mode, boot_state.is_background());
        app::startup_error::exit_on_profile_init_failure(&error, show_window)
    };
    let host_lock = match app::host_instance::host_launch(
        cli_mode,
        std::env::var("PLEXI_RUNNING").ok().as_deref(),
    ) {
        app::host_instance::HostLaunch::Cli => None,
        // Plexi-in-Plexi: a bare launch typed in a Plexi pane shows help
        // rather than starting a second GUI, and never touches the profile.
        app::host_instance::HostLaunch::InsidePlexi => cli::trace::exit(cli::front::emit_help(
            &bin,
            &cli::front::Help {
                path: Vec::new(),
                long: false,
                selection: crate::config::ProfileSelection::Default,
            },
        )),
        app::host_instance::HostLaunch::Host => Some(
            crate::config::lock_profile_for_host_launch()
                .unwrap_or_else(|error| profile_init_failed(error)),
        ),
    };
    // A bad [experiments] table fails every command except the ones needed to
    // fix it, so the user is never locked out of repairing the config.
    if let Some(error) = crate::release::experiments_error() {
        let repair = surface_path
            .first()
            .is_some_and(|name| matches!(name.as_str(), "experiments" | "config" | "doctor"));
        if !repair {
            eprintln!("error: {error}");
            cli::trace::exit(1);
        }
    }
    let launch_profile = match host_lock {
        Some(lock) => LaunchProfile::Host(lock),
        None => LaunchProfile::Cli(
            crate::config::lock_profile_initialization()
                .unwrap_or_else(|error| profile_init_failed(error)),
        ),
    };
    // Adopt an explicit workspace root from `plexi <path>` if one was given.
    // If the path has no `.plexi/` ancestor, an adopted context path is set
    // instead — the directory opens as a new context on first frame.
    // Bare `plexi` continues to resolve via CWD-walk later in `PlexiApp::new`.
    let workspace_operand = match &startup {
        Startup::Launch(launch) => launch.cli.workspace_path.as_deref(),
        Startup::Run(_) => None,
    };
    let adopted_root = match resolve_workspace_operand(workspace_operand) {
        Ok(root) => root,
        Err(e) => {
            eprintln!("error: {e}");
            cli::trace::exit(1);
        }
    };
    // Merge global config with the workspace's channel-scoped config if a
    // workspace is in scope. The `log` level needs the merged value so a
    // project-level `[log] level = "debug"` actually takes effect.
    let merged_config_root = adopted_root
        .clone()
        .or_else(crate::config::active_workspace_root);
    let merged_config =
        crate::config::PlexiConfig::load_with_workspace(merged_config_root.as_deref());
    let log_config = merged_config.log.clone().unwrap_or_default();
    // Local history pruning belongs to the cloud experiment. Log archives are
    // always pruned.
    let retain_local_history = crate::cloud::retention::history_pruning_enabled(
        merged_config
            .cloud
            .as_ref()
            .is_some_and(|cloud| cloud.retain_local_history()),
    );
    let log_level = log_config.level_filter().unwrap_or(log::LevelFilter::Info);
    let retention_days = log_config.retention_days.unwrap_or(30);
    crate::platform::logging::init(log_level, retention_days, cli_mode);
    #[cfg(not(test))]
    config::test_hooks::report_ignored();
    log::info!(
        "startup: logger ready {}ms after main entry",
        process_start.elapsed().as_millis()
    );
    log::info!(
        "cli_front: {} {surface_path:?}",
        if host_startup { "launch" } else { "run" }
    );
    // A CLI command's migration ran inside `lock_profile_initialization`,
    // before the logger.
    crate::app::plugin_migration::replay_early_traces();

    // One host per profile (stint 1459). A host claims `host.lock` while it
    // holds the initialization lock and only then migrates and seeds the
    // profile, so a CLI of another build can never migrate under it (stint
    // 1609). Custody, retention, the rpc sweep, the window and the socket
    // have not run yet. A launch from any path (the install, a generation,
    // the app icon) that finds a live host stops here. Held until the
    // process exits.
    //
    // Profile seeding (example and core packs, Python plugin venvs) can take
    // seconds on a fresh profile, so it runs after the logger is up
    // (stint 1567), before the workspace chdir.
    let (_host_instance, foreign_host) = match launch_profile {
        LaunchProfile::Cli(crate::config::ProfileInit::Current) => (None, None),
        LaunchProfile::Cli(crate::config::ProfileInit::Initialize(profile_init_lock)) => {
            seed_initialized_profile(is_plugin_prune_command);
            drop(profile_init_lock);
            (None, None)
        }
        LaunchProfile::Cli(crate::config::ProfileInit::Deferred(host)) => {
            log::info!(
                "profile: migration to build {} deferred to the next host start; {host}",
                env!("PLEXI_BUILD_ID")
            );
            (None, Some(host))
        }
        // A second launch on a live host hands it the canonical path main
        // resolved above (before the chdir) and exits 0 (stint 1543).
        LaunchProfile::Host(profile_init_lock) => {
            let instance = match app::host_instance::claim(
                &config::config_dir(),
                &crate::platform::ipc::endpoint_path(),
                crate::config::adopted_context_path()
                    .or_else(|| adopted_root.clone())
                    .as_deref(),
            ) {
                Ok(app::host_instance::Launch::Host(instance)) => instance,
                Ok(app::host_instance::Launch::HandedOff(line)) => {
                    log::logger().flush();
                    println!("{line}");
                    std::process::exit(0);
                }
                Err(refusal) if refusal.is_another_host() => {
                    log::info!("host_startup: refused second host: {refusal}");
                    log::logger().flush();
                    eprintln!("{refusal}");
                    std::process::exit(app::host_instance::ALREADY_RUNNING_EXIT);
                }
                Err(failure) => {
                    // Not across the error window, which waits for dismissal.
                    drop(profile_init_lock);
                    app::startup_error::exit_with_startup_error(
                        app::startup_error::StartupErrorView::host_lock_failure(
                            &failure.to_string(),
                            &crate::platform::logging::log_path(),
                        ),
                        app::startup_error::shows_window(true, boot_state.is_background()),
                    )
                }
            };
            match crate::config::initialize_claimed_profile(&profile_init_lock) {
                Ok(true) => seed_initialized_profile(is_plugin_prune_command),
                Ok(false) => {}
                Err(error) => app::startup_error::report_claimed_profile_init_failure(
                    &error,
                    app::startup_error::shows_window(true, boot_state.is_background()),
                    (profile_init_lock, instance),
                ),
            }
            drop(profile_init_lock);
            (Some(instance), None)
        }
    };
    crate::config::set_adopted_workspace_root(adopted_root.clone());

    // When the user runs `plexi <path>`, treat that path as the new CWD so
    // every downstream consumer (PluginRegistry, event log, default pane cwd)
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

    // Host start only (foreground, background and headless hosts all start
    // here). Every CLI command, `host start` included, stays without custody.
    if !cli_mode {
        if let Err(error) = crate::broker::custody::install_for_host_start() {
            log::error!("custody: {error}");
            eprintln!("error: {error}");
            cli::trace::exit(1);
        }
    }
    if !dropped_identity.is_empty() {
        log::info!("cli: dropped foreign channel identity variables {dropped_identity:?}");
    }
    if !cli_mode {
        let enabled: Vec<&str> = crate::release::ReleaseFeature::ALL
            .iter()
            .filter(|f| crate::release::feature_enabled(**f))
            .map(|f| f.key())
            .collect();
        log::info!("experiments: enabled={enabled:?}");
    }
    // Host startup only. A CLI invocation must not prune local history as a
    // side effect of printing JSON, and its stderr must stay a clean payload.
    if cli_mode {
        #[cfg(unix)]
        log::info!("cli: default SIGPIPE handling enabled for closed output pipes");
        log::info!("retention: skipped, argv does not start a host");
    } else {
        if let Err(error) = crate::cloud::retention::run_at_startup(
            chrono::Utc::now(),
            chrono::Local::now().date_naive(),
            retention_days,
            retain_local_history,
            merged_config_root.as_deref(),
        ) {
            log::warn!("cloud retention failed: {error}");
        }
    }
    let frame_tick = crate::platform::logging::new_frame_tick();
    // Note: spawn_heartbeat is deferred to just before eframe::run_native so
    // the shell probes below don't trigger false FREEZE alerts. The heartbeat
    // should only monitor actual eframe operation, not pre-startup work.

    // Capture Rust panics into the log file so they survive process death.
    // Without this, panics on the UI thread only appear in Console.plugin and are
    // invisible to the Plexi log (the log writer thread is killed mid-write).
    install_panic_hook(crate::platform::logging::log_path());

    // Handle CLI subcommands before launching the GUI.
    use crate::cli::args::{
        AccountCmd, AgentCmd, AgentsDashboardCmd, AiCmd, AssistantCmd, AssistantPermissionCmd,
        ChangesCmd, CliSetupCmd, Commands, ConfigCmd, ContextCmd, DebugCmd, DebugSessionCmd,
        DescriptorCmd, EventsCmd, FileCmd, HookAction, HostCmd, LedgerCmd, NeedsYouCmd, NotifyCmd, PaneCmd,
        PaneMessageCmd, PaneResultCmd, PaneSlotCmd, PermissionsCmd, PluginCmd, PluginStateCmd,
        RegistryCmd, RoutineCmd, SecretCmd, SkillCmd, ViewCmd, WorkspaceCmd,
    };
    match startup {
        Startup::Launch(launch) => {
            if let Some(socket) = launch.cli.socket {
                cli::set_command_socket_override(socket);
            }
        }
        Startup::Run(run) => {
            let cli::front::Run {
                cli,
                surface,
                tokens,
                owners,
                ..
            } = *run;
            let blocked = cli::help::admission(&surface, crate::release::feature_enabled);
            if let Some(feature) = blocked {
                exit_if_feature_disabled(feature);
            }
            if let Some(socket) = cli.socket {
                cli::set_command_socket_override(socket);
            }
            if let Some(cmd) = cli.command {
                let json = cli.json;
                let mode = cli::OutputMode::from(json);
                let resolve_pane = |t: &cli::PaneTarget| -> u64 {
                    t.resolve(mode).unwrap_or_else(|code| cli::trace::exit(code))
                };
                let resolve_opt_pane = |t: Option<&cli::PaneTarget>| -> Option<u64> {
                    cli::resolve_opt_pane_target(t, mode).unwrap_or_else(|code| cli::trace::exit(code))
                };
                let current_pane = |t: Option<&cli::PaneTarget>, command: &str| -> u64 {
                    let explicit = resolve_opt_pane(t);
                    cli::current_pane_id(explicit, command).unwrap_or_else(|code| cli::trace::exit(code))
                };
                let resolve_ctx = |t: &cli::ContextTarget| -> u64 {
                    t.resolve(mode).unwrap_or_else(|code| cli::trace::exit(code))
                };
                let resolve_opt_ctx = |t: Option<&cli::ContextTarget>| -> Option<u64> {
                    cli::resolve_opt_context_target(t, mode).unwrap_or_else(|code| cli::trace::exit(code))
                };
                if !matches!(&cmd, Commands::Debug { .. } | Commands::Skill { .. }) {
                    cli::trace::begin(&tokens, &owners, lifecycle_socket.clone());
                }
                // The profile still belongs to a live host of another build
                // (stint 1609): commands that read or write its plugins or
                // SDK cannot run until that host restarts.
                if let Some(host) = foreign_host
                    .as_ref()
                    .filter(|_| cmd.needs_current_profile(&config::config_dir()))
                {
                    log::info!(
                        "profile: refused a command that needs the migrated profile; {host}"
                    );
                    // The hint addresses this command's profile host (stint 1655).
                    let refusal = match cli::host::host_target() {
                        Ok(target) => host.command_refusal(&cli::host::restart_hint(&target)),
                        Err(error) => error,
                    };
                    eprintln!("error: {refusal}");
                    cli::trace::exit(1);
                }
                match cmd {
                    Commands::Assistant { cmd } => match cmd {
                        AssistantCmd::Send {
                            text,
                            request_id,
                            pane_id,
                            context_id,
                            conversation,
                            desktop,
                            status_for,
                            client,
                            kind,
                        } => {
                            let pane_id = resolve_opt_pane(pane_id.as_ref());
                            let context_id = resolve_opt_ctx(context_id.as_ref());
                            cli::trace::exit(cli::assistant_send_cli(
                                text.as_deref(),
                                request_id.as_deref(),
                                pane_id,
                                context_id,
                                cli::plugin::AssistantSendRoute {
                                    conversation: conversation.as_deref(),
                                    join_desktop: desktop,
                                    status_for: status_for.as_deref(),
                                    client: client.as_deref(),
                                    kind: kind.as_deref(),
                                },
                            ))
                        }
                        AssistantCmd::Tool { name, input } => {
                            cli::trace::exit(cli::assistant_tool_cli(&name, &input))
                        }
                        AssistantCmd::Permission { cmd } => match cmd {
                            AssistantPermissionCmd::List => {
                                cli::trace::exit(cli::assistant_permission_cli("list", None, None))
                            }
                            AssistantPermissionCmd::Show { id } => cli::trace::exit(
                                cli::assistant_permission_cli("show", Some(&id), None),
                            ),
                            AssistantPermissionCmd::Resolve { id, choice } => cli::trace::exit(
                                cli::assistant_permission_cli("resolve", Some(&id), Some(&choice)),
                            ),
                        },
                    },
                    Commands::NeedsYou { cmd } => match cmd {
                        NeedsYouCmd::List => {
                            cli::trace::exit(cli::needs_you_cli("list", None, None))
                        }
                        NeedsYouCmd::Show { id } => cli::trace::exit(cli::needs_you_api_cli(
                            "show",
                            serde_json::json!({ "id": id }),
                        )),
                        NeedsYouCmd::Ask {
                            summary,
                            resource,
                            choices,
                            allow_free_text,
                            recommend,
                            preview_refs,
                            correlation_id,
                            operation_id,
                            session_id,
                            expected_revision,
                            context_id,
                            pane_id,
                        } => {
                            let pane_id = resolve_opt_pane(pane_id.as_ref()).or_else(|| {
                                std::env::var("PLEXI_PANE_ID")
                                    .ok()
                                    .and_then(|v| v.parse::<u64>().ok())
                            });
                            let context_id = resolve_opt_ctx(context_id.as_ref()).or_else(|| {
                                std::env::var("PLEXI_CONTEXT_ID")
                                    .ok()
                                    .and_then(|v| v.parse::<u64>().ok())
                            });
                            cli::trace::exit(cli::needs_you_api_cli(
                                "ask",
                                serde_json::json!({
                                    "summary": summary,
                                    "resource": resource,
                                    "choices": choices,
                                    "allow_free_text": allow_free_text,
                                    "recommend": recommend,
                                    "preview_refs": preview_refs,
                                    "correlation_id": correlation_id,
                                    "operation_id": operation_id,
                                    "session_id": session_id,
                                    "expected_revision": expected_revision,
                                    "context_id": context_id,
                                    "pane_id": pane_id,
                                }),
                            ))
                        }
                        NeedsYouCmd::Answer { id, answer } => {
                            cli::trace::exit(cli::needs_you_api_cli(
                                "answer",
                                serde_json::json!({ "id": id, "answer": answer }),
                            ))
                        }
                        NeedsYouCmd::Reconcile {
                            session_id,
                            pane_id,
                            context_id,
                            id,
                        } => {
                            let pane_id = resolve_opt_pane(pane_id.as_ref());
                            let context_id = resolve_opt_ctx(context_id.as_ref());
                            cli::trace::exit(cli::needs_you_api_cli(
                                "reconcile",
                                serde_json::json!({
                                    "session_id": session_id,
                                    "pane_id": pane_id,
                                    "context_id": context_id,
                                    "id": id,
                                }),
                            ))
                        }
                        NeedsYouCmd::Complete { id, state, receipt } => {
                            cli::trace::exit(cli::needs_you_api_cli(
                                "complete",
                                serde_json::json!({
                                    "id": id,
                                    "state": state,
                                    "receipt": receipt,
                                }),
                            ))
                        }
                        NeedsYouCmd::Resolve { id, approve, deny } => {
                            // Approve of a click approval is refused in the host. Deny
                            // still settles. Questions and blocked runs settle
                            // from here and do not mint a grant.
                            cli::trace::exit(cli::needs_you_cli(
                                "resolve",
                                Some(&id),
                                Some(approve && !deny),
                            ))
                        }
                    },
                    Commands::Experiments { cmd } => match cmd {
                        cli::args::ExperimentsCmd::List => {
                            cli::trace::exit(cli::experiments::experiments_list(json))
                        }
                        cli::args::ExperimentsCmd::Enable { name } => {
                            cli::trace::exit(cli::experiments::experiments_set(&name, true))
                        }
                        cli::args::ExperimentsCmd::Disable { name } => {
                            cli::trace::exit(cli::experiments::experiments_set(&name, false))
                        }
                    },
                    Commands::Skill { cmd } => match cmd {
                        SkillCmd::Draft {
                            from_session,
                            pane,
                            all_panes,
                            include_internal,
                            include_failed,
                            format,
                            out,
                        } => {
                            let options = cli::skill_draft::DraftOptions {
                                pane: cli::skill_draft::resolve_pane(
                                    pane,
                                    all_panes,
                                    std::env::var("PLEXI_PANE_ID").ok().as_deref(),
                                ),
                                include_internal,
                                include_failed,
                            };
                            cli::trace::exit(cli::skill_draft::skill_draft_cli(
                                &from_session,
                                options,
                                &format,
                                out.as_deref(),
                            ))
                        }
                        SkillCmd::Install { agent } => {
                            cli::trace::exit(cli::skill_install::skill_install_cli(&agent))
                        }
                    },
                    Commands::Permissions { cmd } => match cmd {
                        PermissionsCmd::List => {
                            cli::trace::exit(cli::permissions_cli("list", None, None, mode))
                        }
                        PermissionsCmd::Reset { id } => {
                            cli::trace::exit(cli::permissions_cli("reset", Some(&id), None, cli::OutputMode::Json))
                        }
                        PermissionsCmd::Revoke { id } => {
                            cli::trace::exit(cli::permissions_cli("revoke", Some(&id), None, cli::OutputMode::Json))
                        }
                        PermissionsCmd::Allow { id, duration } => {
                            cli::trace::exit(cli::permissions_cli(
                                "allow",
                                Some(&id),
                                duration.as_deref(),
                                cli::OutputMode::Json,
                            ))
                        }
                    },
                    Commands::AgentsDashboard { cmd, follow } => {
                        if follow && cmd.is_some() {
                            eprintln!("error: --follow prints the live projection and takes no subcommand");
                            cli::trace::exit(2);
                        }
                        if follow {
                            cli::trace::exit(cli::command_view_follow_cli());
                        }
                        match cmd {
                            Some(AgentsDashboardCmd::Open) => {
                                cli::trace::exit(cli::command_view_cli("open", mode))
                            }
                            Some(AgentsDashboardCmd::Cancel { run }) => {
                                cli::trace::exit(cli::command_view_cancel_cli(&run, mode))
                            }
                            Some(AgentsDashboardCmd::Resolve { id: _ }) => {
                                cli::trace::exit(cli::command_view_refused("resolve"))
                            }
                            Some(AgentsDashboardCmd::Allow { tool: _ }) => {
                                cli::trace::exit(cli::command_view_refused("allow"))
                            }
                            None => cli::trace::exit(cli::command_view_cli("list", mode)),
                        }
                    }
                    Commands::Run {
                        command,
                        extra_args,
                    } => match command {
                        Some(cmd) => cli::trace::exit(cli::run_command(&cmd, &extra_args, mode)),
                        None => cli::trace::exit(cli::run_list_commands(json)),
                    },
                    Commands::Workspace { cmd } => match cmd {
                        WorkspaceCmd::Init => cli::trace::exit(cli::workspace_init()),
                        WorkspaceCmd::Clean { dry_run } => {
                            cli::trace::exit(cli::workspace_clean_cli(dry_run, mode))
                        }
                    },
                    Commands::Routine { cmd } => match cmd {
                        RoutineCmd::List => cli::trace::exit(cli::routine_list()),
                        RoutineCmd::Run { name, force } => {
                            cli::trace::exit(cli::routine_run(&name, force))
                        }
                        RoutineCmd::Add {
                            name,
                            command,
                            schedule,
                            context,
                            ephemeral,
                        } => cli::trace::exit(cli::routine_add(
                            &name,
                            &command,
                            &schedule,
                            context.as_deref(),
                            ephemeral,
                        )),
                        RoutineCmd::Remove { name } => cli::trace::exit(cli::routine_remove(&name)),
                        RoutineCmd::Enable { name } => {
                            cli::trace::exit(cli::routine_set_enabled(&name, true))
                        }
                        RoutineCmd::Disable { name } => {
                            cli::trace::exit(cli::routine_set_enabled(&name, false))
                        }
                    },
                    Commands::Lease { cmd } => cli::trace::exit(cli::lease::dispatch(cmd)),
                    Commands::Agent { cmd } => match cmd {
                        AgentCmd::Create {
                            name,
                            cli: harness_cli,
                            model,
                            fallback,
                            description,
                            persona,
                            persona_file,
                            result_schema,
                            ephemeral,
                            display_name,
                            global,
                            cwd,
                            flags,
                        } => cli::trace::exit(cli::agent_create_cli(
                            &name,
                            &harness_cli,
                            model.as_deref(),
                            fallback.as_deref(),
                            description.as_deref(),
                            persona.as_deref(),
                            persona_file.as_deref(),
                            result_schema.as_deref(),
                            ephemeral,
                            display_name.as_deref(),
                            global,
                            cwd.as_deref(),
                            &flags,
                        )),
                        AgentCmd::Flags { target, cwd } => cli::trace::exit(cli::agent_flags_cli(
                            target.as_deref(),
                            cwd.as_deref(),
                            json,
                        )),
                        AgentCmd::List { cwd } => {
                            cli::trace::exit(cli::agent_list_cli(cwd.as_deref(), json))
                        }
                        AgentCmd::Show { name, cwd } => {
                            cli::trace::exit(cli::agent_show_cli(&name, cwd.as_deref(), json))
                        }
                        AgentCmd::Call {
                            role,
                            task,
                            worktree,
                            asynchronous,
                        } => cli::trace::exit(cli::agent_tool_cli(
                            serde_json::json!({"type":"agent_call","role":role,"task":task,"worktree":worktree,"asynchronous":asynchronous}),
                        )),
                        AgentCmd::Message {
                            pane_id,
                            text,
                            wait,
                        } => {
                            let pane_id = resolve_pane(&pane_id);
                            cli::trace::exit(cli::agent_tool_cli(
                                serde_json::json!({"type":"agent_message","pane_id":pane_id,"text":text,"wait":wait}),
                            ))
                        }
                        AgentCmd::Handoff { role, task } => cli::trace::exit(cli::agent_tool_cli(
                            serde_json::json!({"type":"agent_call","role":role,"task":task,"handoff":true}),
                        )),
                        AgentCmd::Start {
                            name,
                            prompt,
                            message,
                            cwd,
                        } => {
                            let initial_message = message.as_deref().or(prompt.as_deref());
                            cli::trace::exit(cli::agent_start_cli(
                                &name,
                                cwd.as_deref(),
                                initial_message,
                                mode,
                            ))
                        }
                        AgentCmd::Spawn {
                            name,
                            task,
                            subcontext,
                            ephemeral,
                            from,
                            cwd,
                        } => {
                            let from = resolve_opt_pane(from.as_ref());
                            cli::trace::exit(cli::agent_spawn_cli(
                                &name,
                                &task,
                                subcontext.as_deref(),
                                ephemeral,
                                from,
                                cwd.as_deref(),
                                mode,
                            ))
                        }
                        AgentCmd::Done {
                            result,
                            summary,
                            pane_id,
                        } => {
                            let pane_id = resolve_opt_pane(pane_id.as_ref());
                            cli::trace::exit(cli::agent_done_cli(
                                &result,
                                summary.as_deref(),
                                pane_id,
                            ))
                        }
                        AgentCmd::Result { pane, pane_flag } => {
                            let target = pane.as_deref().or(pane_flag.as_deref());
                            cli::trace::exit(cli::agent_result_cli(target, json))
                        }
                        AgentCmd::Wait { pane_ids, timeout } => {
                            let resolved: Vec<u64> = pane_ids.iter().map(resolve_pane).collect();
                            cli::trace::exit(cli::agent_wait_cli(&resolved, timeout, mode))
                        }
                        AgentCmd::Ask { question, pane_id } => {
                            let pane_id = resolve_opt_pane(pane_id.as_ref());
                            cli::trace::exit(cli::agent_ask_cli(&question, pane_id, mode))
                        }
                        AgentCmd::Report {
                            state,
                            agent,
                            detail,
                            session_id,
                            event,
                            blocked_reason,
                        } => cli::trace::exit(cli::agent_report_cli(
                            state.as_str(),
                            agent.as_str(),
                            detail.as_deref(),
                            session_id.as_deref(),
                            event.as_deref(),
                            blocked_reason,
                        )),
                        AgentCmd::Status {
                            blocked,
                            working,
                            idle,
                        } => cli::trace::exit(cli::agent_status_cli(blocked, working, idle, mode)),
                        AgentCmd::Next => {
                            cli::trace::exit(cli::agent_next_cli(mode));
                        }
                        AgentCmd::Fleet { cmd, filter } => match cmd {
                            Some(cli::args::AgentFleetCmd::Dismiss { target, all_failed }) => {
                                cli::trace::exit(cli::agent_fleet_dismiss_cli(
                                    target.as_deref(),
                                    all_failed,
                                    mode,
                                ));
                            }
                            None => {
                                cli::trace::exit(cli::agent_fleet_cli(mode, filter));
                            }
                        },
                        AgentCmd::History { limit } => {
                            cli::trace::exit(cli::agent_history_cli(limit, json))
                        }
                        AgentCmd::Open { name, cwd } => cli::trace::exit(cli::agent_open_cli(
                            &name,
                            cwd.as_deref(),
                            mode,
                        )),
                        AgentCmd::NewChat { name, cwd } => cli::trace::exit(
                            cli::agent_new_chat_cli(&name, cwd.as_deref(), mode),
                        ),
                        AgentCmd::Reopen { target, subcontext } => cli::trace::exit(
                            cli::agent_reopen_cli(&target, subcontext.as_deref(), mode),
                        ),
                        AgentCmd::Team { cmd } => {
                            cli::trace::exit(cli::agent_team_dispatch(cmd, mode))
                        }
                        AgentCmd::HookEvent { agent } => {
                            cli::trace::exit(cli::agent_hook_event_cli(agent.as_deref()))
                        }
                        AgentCmd::Hook { action } => match action {
                            HookAction::Install {
                                claude_code,
                                codex,
                                pi,
                            } => cli::trace::exit(cli::agent_hook_install_cli(
                                claude_code,
                                codex,
                                pi,
                            )),
                            HookAction::Uninstall {
                                claude_code,
                                codex,
                                pi,
                            } => cli::trace::exit(cli::agent_hook_uninstall_cli(
                                claude_code,
                                codex,
                                pi,
                            )),
                        },
                    },
                    Commands::Changes { cmd } => cli::trace::exit(match cmd {
                        ChangesCmd::Allow {
                            agent,
                            file,
                            old,
                            new,
                        } => cli::changes_allow_cli(&agent, &file, &old, &new),
                        ChangesCmd::Propose {
                            agent,
                            file,
                            old,
                            new,
                        } => cli::changes_propose_cli(&agent, &file, &old, &new),
                        ChangesCmd::Preview { id } => cli::changes_preview_cli(&id),
                        ChangesCmd::Accept { id } => cli::changes_accept_cli(&id),
                        ChangesCmd::Refresh { id } => cli::changes_refresh_cli(&id),
                        ChangesCmd::Revert { id } => cli::changes_revert_cli(&id),
                        ChangesCmd::Profile => cli::changes_profile_cli(),
                    }),
                    Commands::Secret { cmd } => match cmd {
                        SecretCmd::Set {
                            friendly_name,
                            from_env,
                            global,
                            alias,
                            folder,
                        } => {
                            if let Some(folder) = folder.as_deref() {
                                cli::trace::exit(cli::folder_secret_set(
                                    &friendly_name,
                                    from_env,
                                    global,
                                    alias.as_deref(),
                                    folder,
                                ))
                            }
                            cli::trace::exit(cli::workspace_secret_set(
                                &friendly_name,
                                from_env,
                                global,
                                alias.as_deref(),
                            ))
                        }
                        SecretCmd::Get {
                            friendly_name,
                            global,
                        } => cli::trace::exit(cli::workspace_secret_get(&friendly_name, global)),
                        SecretCmd::List { global, context } => cli::trace::exit(
                            cli::workspace_secret_list(global, context.as_ref(), mode),
                        ),
                        SecretCmd::Delete {
                            friendly_name,
                            global,
                        } => cli::trace::exit(cli::workspace_secret_delete(&friendly_name, global)),
                        SecretCmd::Rm { name, folder } => {
                            cli::trace::exit(cli::folder_secret_rm(&name, &folder))
                        }
                        SecretCmd::Grant {
                            name,
                            agent,
                            plugin,
                            folder,
                        } => cli::trace::exit(cli::folder_secret_grant(
                            &name,
                            agent.as_deref(),
                            plugin.as_deref(),
                            folder.as_deref(),
                        )),
                        SecretCmd::Read { name, folder } => {
                            cli::trace::exit(cli::folder_secret_read(&name, folder.as_deref()))
                        }
                        SecretCmd::Exec { cwd, command } => {
                            cli::trace::exit(cli::folder_secret_exec(&cwd, &command))
                        }
                    },
                    Commands::Open(args)
                    | Commands::File {
                        cmd: FileCmd::Open(args),
                    } => {
                        let from = resolve_opt_pane(args.from.as_ref());
                        cli::trace::exit(cli::file_open_cli(&args, from, mode))
                    }
                    Commands::View { cmd } => match cmd {
                        ViewCmd::List => cli::trace::exit(cli::view_list(json)),
                        ViewCmd::Open {
                            type_id,
                            mcp,
                            cli: cli_flag,
                            down,
                            left,
                            up,
                            right,
                            tab,
                            window,
                            split,
                            from: from_pane_id,
                            extra_args,
                        } => {
                            let from_pane_id = resolve_opt_pane(from_pane_id.as_ref());
                            let layout: Option<String> = match split.as_deref() {
                                Some("down") => Some("split_v".into()),
                                Some("left") => Some("split_left".into()),
                                Some("up") => Some("split_above".into()),
                                Some("right") => Some("split_h".into()),
                                Some("tab") => Some("tab".into()),
                                Some("window") => Some("new_window".into()),
                                _ if down => Some("split_v".into()),
                                _ if left => Some("split_left".into()),
                                _ if up => Some("split_above".into()),
                                _ if right => Some("split_h".into()),
                                _ if tab => Some("tab".into()),
                                _ if window => Some("new_window".into()),
                                _ => None,
                            }; // default: overlay
                            let mode_count = type_id.is_some() as u8
                                + (!mcp.is_empty()) as u8
                                + cli_flag.is_some() as u8;
                            if mode_count == 0 {
                                eprintln!("error: one of TYPE_ID, --mcp, or --cli is required");
                                cli::trace::exit(2);
                            }
                            if mode_count > 1 {
                                eprintln!(
                                    "error: TYPE_ID, --mcp, and --cli are mutually exclusive"
                                );
                                cli::trace::exit(2);
                            }
                            if let Some(tid) = type_id {
                                // Prefix routing: cli:, mcp:, view:, or bare name
                                match cli::parse_prefix(&tid) {
                                    cli::OpenPrefix::Cli(name) => {
                                        log::info!("view_open:cli: prefix-routed cli:{name}");
                                        cli::trace::exit(cli::open_cli_by_name(
                                            &name,
                                            layout.as_deref(),
                                            from_pane_id,
                                            None,
                                        ));
                                    }
                                    cli::OpenPrefix::Mcp(name) => {
                                        exit_if_feature_disabled(
                                            crate::release::ReleaseFeature::McpClient,
                                        );
                                        log::info!("view_open:cli: prefix-routed mcp:{name}");
                                        cli::trace::exit(cli::open_mcp_by_name(
                                            &name,
                                            layout.as_deref(),
                                            from_pane_id,
                                            None,
                                        ));
                                    }
                                    cli::OpenPrefix::View(name) => {
                                        log::info!("view_open:cli: prefix-routed view:{name}");
                                        cli::trace::exit(cli::open_cli(
                                            &name,
                                            &extra_args,
                                            layout.as_deref(),
                                            from_pane_id,
                                            None,
                                        ));
                                    }
                                    cli::OpenPrefix::Bare(name) => {
                                        log::info!("view_open:cli: opening plugin type_id={name}");
                                        cli::trace::exit(cli::open_cli(
                                            &name,
                                            &extra_args,
                                            layout.as_deref(),
                                            from_pane_id,
                                            None,
                                        ));
                                    }
                                }
                            } else if !mcp.is_empty() {
                                let title = cli::mcp_pane_title(&mcp);
                                log::info!("view_open:cli: launching mcp-renderer with server id {:?}, auto-title={title:?}", mcp);
                                cli::trace::exit(cli::pane_new_cli(cli::PaneNewCliArgs {
                                    name: Some(title.as_str()),
                                    layout: layout.as_deref(),
                                    from_pane_id,
                                    plugin: Some("mcp-renderer"),
                                    mcp: &mcp,
                                    ..Default::default()
                                }));
                            } else {
                                let binary = cli_flag.unwrap();
                                log::info!("view_open:cli: --cli routing `{binary}` through unified resolver");
                                cli::trace::exit(cli::open_cli_by_name(
                                    &binary,
                                    layout.as_deref(),
                                    from_pane_id,
                                    None,
                                ));
                            }
                        }
                        ViewCmd::Render {
                            plugin,
                            size,
                            state,
                            output,
                            png,
                        } => cli::trace::exit(cli::view_render(
                            &plugin,
                            &size,
                            state.as_deref(),
                            output.as_deref(),
                            png,
                        )),
                    },
                    Commands::Plugin { cmd } => match cmd {
                        PluginCmd::Install {
                            spec_or_path,
                            pack,
                            refresh,
                            version,
                            yes,
                        } => {
                            if let Some(p) = pack {
                                log::info!("app_install:cli: pack={p} refresh={refresh}");
                                cli::trace::exit(cli::install_pack_cli(&p, refresh));
                            }
                            match spec_or_path {
                                None => {
                                    log::info!("app_install:cli: workspace pack (no args)");
                                    cli::trace::exit(cli::install_workspace_pack_cli());
                                }
                                Some(s) => {
                                    match cli::plugin::classify_plugin_install_spec(&s) {
                                        cli::plugin::PluginInstallSpecKind::Package => {
                                            log::info!(
                                                "app_install:cli: package={s} version={version:?}"
                                            );
                                            cli::trace::exit(cli::plugin_install_package(
                                                &s,
                                                version.as_deref(),
                                                cli::InstallConfirm::Interactive,
                                                yes,
                                            ));
                                        }
                                        cli::plugin::PluginInstallSpecKind::Source => {
                                            log::info!(
                                                    "app_install:cli: source spec={s} version={version:?}"
                                                );
                                            cli::trace::exit(cli::install_cli(&s, yes));
                                        }
                                        cli::plugin::PluginInstallSpecKind::LocalPath => {
                                            log::info!("app_install:cli: local path={s} version={version:?}");
                                            cli::trace::exit(cli::plugin_install_with_pin(
                                                &s,
                                                version.as_deref(),
                                                cli::InstallConfirm::Interactive,
                                                yes,
                                            ));
                                        }
                                        cli::plugin::PluginInstallSpecKind::MarketplaceId => {
                                            exit_if_feature_disabled(
                                                crate::release::ReleaseFeature::Marketplace,
                                            );
                                            log::info!("app_install:cli: marketplace id={s} version={version:?}");
                                            cli::trace::exit(cli::install_cli(&s, yes));
                                        }
                                    }
                                }
                            }
                        }
                        PluginCmd::Init {
                            name,
                            adopt,
                            wasm,
                            lang,
                            global,
                            open,
                            no_open,
                            from: from_pane_id,
                        } => {
                            if adopt {
                                cli::trace::exit(cli::plugin_adopt(name.as_deref().unwrap_or(".")));
                            }
                            let from_pane_id = resolve_opt_pane(from_pane_id.as_ref());
                            cli::trace::exit(cli::plugin_init(
                                wasm.as_deref().or(name.as_deref()).unwrap_or_default(),
                                if wasm.is_some() { "wasm" } else { &lang },
                                global,
                                open && !no_open,
                                from_pane_id,
                            ));
                        }
                        PluginCmd::Uninstall { id, yes } => {
                            cli::trace::exit(cli::plugin_uninstall(&id, yes))
                        }
                        PluginCmd::List => cli::trace::exit(cli::plugin_list(json)),
                        PluginCmd::Prune { dry_run } => {
                            cli::trace::exit(cli::plugin_prune_cli(dry_run))
                        }

                        PluginCmd::Check {
                            path,
                            sizes,
                            png_dir,
                        } => {
                            log::info!(
                                "plugin_check:cli: path_given={} sizes={} png_dir_given={}",
                                !path.is_empty(),
                                sizes.len(),
                                png_dir.is_some()
                            );
                            cli::trace::exit(cli::plugin_check_cli(
                                &path,
                                &sizes,
                                png_dir.as_deref(),
                            ));
                        }
                        PluginCmd::Test { path, snapshot } => {
                            log::info!("app_test:cli: path={path} snapshot={snapshot}");
                            cli::trace::exit(cli::plugin_test_cli(&path, snapshot));
                        }
                        PluginCmd::Info { id } => cli::trace::exit(cli::plugin_info(&id)),
                        PluginCmd::State { cmd } => match cmd {
                            PluginStateCmd::Get { plugin, scope } => {
                                cli::trace::exit(cli::plugin_state::get(&plugin, scope.as_deref(), mode))
                            }
                            PluginStateCmd::Set {
                                plugin,
                                file,
                                scope,
                            } => cli::trace::exit(cli::plugin_state::set(
                                &plugin,
                                file.as_deref(),
                                scope.as_deref(),
                                mode,
                            )),
                        },
                        PluginCmd::Validate { path } => {
                            log::info!("app_validate:cli: path={path}");
                            cli::trace::exit(cli::validate_cli(&path));
                        }
                        PluginCmd::Inspect { path } => {
                            log::info!("app_inspect:cli: path={path}");
                            cli::trace::exit(cli::plugin_inspect_cli(&path));
                        }
                        PluginCmd::Package { path, out } => {
                            log::info!("app_package:cli: path={path} out={out:?}");
                            cli::trace::exit(cli::plugin_package_cli(&path, out.as_deref()));
                        }
                        PluginCmd::Freeze { path } => {
                            log::info!("app_freeze:cli: path={path}");
                            cli::trace::exit(cli::freeze_cli(&path));
                        }
                        PluginCmd::Publish { path } => {
                            cli::trace::exit(cli::plugin_publish_cli(&path));
                        }
                        PluginCmd::Browse => {
                            cli::trace::exit(cli::plugin_browse_cli());
                        }
                        PluginCmd::Search { query } => {
                            cli::trace::exit(cli::plugin_search_cli(&query));
                        }
                        PluginCmd::Update { id } => {
                            log::info!("app_update:cli: id={id:?}");
                            cli::trace::exit(cli::plugin_update_cli(id.as_deref()));
                        }

                        PluginCmd::Call {
                            plugin_id,
                            tool,
                            input,
                            pane,
                            wait_approved,
                        } => {
                            let pane = resolve_opt_pane(pane.as_ref());
                            cli::trace::exit(cli::plugin_call_cli(
                                &plugin_id,
                                &tool,
                                &input,
                                mode,
                                pane,
                                wait_approved,
                            ))
                        }
                        PluginCmd::Tools { plugin, context } => {
                            let context = resolve_opt_ctx(context.as_ref());
                            cli::trace::exit(cli::plugin_tools_cli(
                                plugin.as_deref(),
                                context,
                                mode,
                            ))
                        }
                    },
                    Commands::Uninstall {
                        keep_data,
                        delete_data,
                        yes,
                    } => cli::trace::exit(cli::plexi_uninstall_cli(keep_data, delete_data, yes)),
                    Commands::Update { rollback } => {
                        cli::trace::exit(cli::self_update_cli(rollback))
                    }
                    Commands::Host { cmd } => match cmd {
                        HostCmd::Start {
                            layout,
                            panes,
                            timeout_secs,
                            ephemeral,
                            background,
                        } => cli::trace::exit(cli::host_start_cli(
                            layout.as_deref(),
                            &panes,
                            timeout_secs,
                            ephemeral,
                            background,
                        )),
                        HostCmd::Restart { yes, after_pid } => {
                            cli::trace::exit(cli::host::host_restart_cli(yes, json, after_pid))
                        }
                        HostCmd::Stop => cli::trace::exit(cli::host_stop_cli()),
                        HostCmd::Log { message, source } => {
                            cli::trace::exit(cli::host_log_cli(&source, &message))
                        }
                        HostCmd::Status => cli::trace::exit(cli::host_status_cli(json)),
                        HostCmd::Screenshot { pane, output } => {
                            let pane = resolve_opt_pane(pane.as_ref());
                            cli::trace::exit(cli::host_screenshot_cli(pane, output.as_deref(), mode))
                        }
                        HostCmd::Key { key } => cli::trace::exit(cli::host_key_cli(&key, mode)),
                    },
                    Commands::Notify { cmd, post } => {
                        if let Some(NotifyCmd::Dismiss { notify_id }) = cmd {
                            cli::trace::exit(cli::dismiss_notify_cli(&notify_id));
                        }
                        let Some(title) = post.title else {
                            eprintln!("error: --title is required when posting a notification");
                            cli::trace::exit(1);
                        };
                        let body = post.body;
                        let choices = post.choices;
                        let host_actions = post.host_actions;
                        let no_wait = post.no_wait;
                        let timeout = post.timeout;
                        let wait_timeout = post.wait_timeout;
                        let scope = post.scope;
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
                                    cli::trace::exit(1);
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
                                    cli::trace::exit(1);
                                }
                            }
                        }
                        // Any --host-action keys not matched to a --choice are an error.
                        if let Some(key) = host_action_map.keys().next() {
                            let msg = format!(
                                "error: --host-action key {:?} does not match any --choice key",
                                key
                            );
                            log::warn!("notify:cli: {msg}");
                            eprintln!("{msg}");
                            cli::trace::exit(1);
                        }
                        let parsed_scope = match cli::parse_notify_scope(scope.as_deref()) {
                            Ok(parsed) => parsed,
                            Err(msg) => {
                                log::warn!("notify:cli: {msg}");
                                eprintln!("{msg}");
                                cli::trace::exit(1);
                            }
                        };
                        log::info!(
                            "notify:cli: host_actions={} merged into choices",
                            host_actions.len()
                        );
                        cli::trace::exit(cli::notify_cli(cli::NotifyCliArgs {
                            title: &title,
                            body: &body,
                            choices: &parsed_choices,
                            wait_for_response: !no_wait,
                            display_timeout_secs: timeout,
                            wait_timeout_secs: wait_timeout,
                            scope: parsed_scope,
                            mode,
                        }));
                    }
                    Commands::Pane { cmd } => match cmd {
                        PaneCmd::Action {
                            pane_id,
                            action,
                            args,
                        } => {
                            let pane_id = resolve_pane(&pane_id);
                            log::info!(
                                "pane_action:cli: pane_id={pane_id} action={action:?} argc={}",
                                args.len()
                            );
                            cli::trace::exit(cli::pane_action_cli(pane_id, &action, &args, mode));
                        }
                        PaneCmd::Layout => cli::trace::exit(cli::pane_layout_cli(mode)),
                        PaneCmd::Name { first, second } => {
                            let (pane_id, name) = match second {
                                Some(title) => {
                                    let target = cli::PaneTarget::from(first);
                                    (Some(resolve_pane(&target)), title)
                                }
                                None => (None, first),
                            };
                            cli::trace::exit(cli::pane_set_title_cli(pane_id, &name))
                        }
                        PaneCmd::List { context } => {
                            let (context_id, current) = match context.as_deref() {
                                None => (None, false),
                                Some("current") => (None, true),
                                Some(s) => {
                                    let target = cli::ContextTarget::from(s.to_string());
                                    let id = target
                                        .resolve(mode)
                                        .unwrap_or_else(|code| cli::trace::exit(code));
                                    (Some(id), false)
                                }
                            };
                            cli::trace::exit(cli::pane_list_cli(context_id, current))
                        }
                        PaneCmd::Tree { context } => {
                            let (context_id, current) = match context.as_deref() {
                                None => (None, false),
                                Some("current") => (None, true),
                                Some(s) => {
                                    let target = cli::ContextTarget::from(s.to_string());
                                    let id = target
                                        .resolve(mode)
                                        .unwrap_or_else(|code| cli::trace::exit(code));
                                    (Some(id), false)
                                }
                            };
                            cli::trace::exit(cli::pane_tree_cli(context_id, current, mode))
                        }
                        PaneCmd::Focus {
                            pane_id,
                            back: _,
                            forward,
                        } => {
                            let pane_id = resolve_opt_pane(pane_id.as_ref());
                            cli::trace::exit(match pane_id {
                                Some(id) => cli::pane_focus_cli(id),
                                None => cli::host_action_cli(
                                    serde_json::json!({"type":"focus_history", "forward":forward}),
                                    None,
                                    mode,
                                ),
                            })
                        }
                        PaneCmd::Zoom { pane_id } => {
                            let pane_id = current_pane(pane_id.as_ref(), "pane zoom");
                            cli::trace::exit(cli::host_action_cli(serde_json::json!({"type":"pane_zoom", "pane_id":pane_id}), None, mode))
                        }
                        PaneCmd::Swap { pane_id, direction } => {
                            let pane_id = current_pane(pane_id.as_ref(), "pane swap");
                            cli::trace::exit(cli::host_action_cli(serde_json::json!({"type":"pane_swap", "direction":direction, "pane_id":pane_id}), None, mode))
                        }
                        PaneCmd::Close { pane_id } => {
                            let id = current_pane(pane_id.as_ref(), "pane close");
                            log::info!("pane_close:cli: pane_id={id}");
                            cli::trace::exit(cli::pane_close_cli(id, mode));
                        }
                        PaneCmd::Send {
                            pane_id,
                            text,
                            submit,
                            paste,
                            operation_id,
                        } => {
                            let pane_id = resolve_pane(&pane_id);
                            cli::trace::exit(cli::pane::pane_send_with_operation_cli(
                                pane_id,
                                &text,
                                submit,
                                paste,
                                operation_id.as_deref(),
                            ))
                        }
                        PaneCmd::Command {
                            pane_id,
                            text,
                            enter,
                        } => {
                            let pane_id = resolve_pane(&pane_id);
                            log::info!(
                                "pane_command:cli: pane_id={pane_id} len={} enter={enter}",
                                text.len()
                            );
                            // `--enter` takes the exact host-confirmed submit
                            // path `pane send --submit` uses (settle, Enter,
                            // confirm) instead of racing a raw newline against
                            // the shell's line editor. Without `--enter` this
                            // stays the type-only verb, same as `pane send`.
                            cli::trace::exit(cli::pane_send_cli(pane_id, &text, enter, false));
                        }
                        PaneCmd::Key { pane_id, key } => {
                            let pane_id = resolve_pane(&pane_id);
                            cli::trace::exit(cli::pane_key_cli(pane_id, &key, mode))
                        }
                        PaneCmd::Drop {
                            pane_id,
                            path_or_url,
                        } => {
                            let pane_id = resolve_pane(&pane_id);
                            cli::trace::exit(cli::pane_drop_cli(pane_id, &path_or_url, mode))
                        }
                        PaneCmd::Click {
                            pane_id,
                            x,
                            y,
                            node,
                            button,
                        } => {
                            let pane_id = resolve_pane(&pane_id);
                            cli::trace::exit(match node {
                                Some(node_ref) => cli::pane_click_node_cli(pane_id, &node_ref, mode),
                                None => match (x, y) {
                                    (Some(x), Some(y)) => cli::pane_click_cli(
                                        pane_id,
                                        x,
                                        y,
                                        button.as_deref().unwrap_or("left"),
                                        mode,
                                    ),
                                    _ => {
                                        eprintln!(
                                                "error: pane click requires either <x> <y> or --node <ref>"
                                            );
                                        1
                                    }
                                },
                            })
                        }
                        PaneCmd::Drag {
                            pane_id,
                            from,
                            from_node,
                            to,
                            to_node,
                            steps,
                            button,
                        } => {
                            let pane_id = resolve_pane(&pane_id);
                            cli::trace::exit(cli::pane_drag_cli(
                                pane_id,
                                cli::PaneDragEndpoint {
                                    coords: from.as_deref(),
                                    node: from_node.as_deref(),
                                },
                                cli::PaneDragEndpoint {
                                    coords: to.as_deref(),
                                    node: to_node.as_deref(),
                                },
                                steps,
                                &button,
                                mode,
                            ))
                        }
                        PaneCmd::Self_ => cli::trace::exit(cli::pane_self_cli()),
                        PaneCmd::Info { pane_id, previous } => {
                            let pane_id = resolve_opt_pane(pane_id.as_ref());
                            cli::trace::exit(cli::pane_info_cli(pane_id, previous, mode))
                        }
                        PaneCmd::Capture {
                            pane_id,
                            lines,
                            full_output,
                            from_cursor,
                            plain,
                        } => {
                            let pane_id = resolve_opt_pane(pane_id.as_ref());
                            cli::trace::exit(cli::pane_capture_cli(
                                pane_id,
                                lines,
                                full_output,
                                from_cursor,
                                plain,
                                mode,
                            ))
                        }
                        PaneCmd::Wait { pane_id, until, timeout } => {
                            let pane_id = current_pane(pane_id.as_ref(), "pane wait");
                            cli::trace::exit(cli::pane_lifecycle_wait_cli(pane_id, &until, timeout))
                        }
                        PaneCmd::Events { follow: _, pane } => {
                            let pane = resolve_opt_pane(pane.as_ref());
                            cli::trace::exit(cli::pane_lifecycle_follow_cli(pane))
                        }
                        PaneCmd::Status { pane_id } => {
                            let pane_id = current_pane(pane_id.as_ref(), "pane status");
                            cli::trace::exit(cli::pane_status_cli(pane_id, mode))
                        }
                        PaneCmd::Heartbeat { pane_id, every, text, while_idle_only, off } => {
                            let pane_id = current_pane(pane_id.as_ref(), "pane heartbeat");
                            cli::trace::exit(cli::pane_heartbeat_cli(pane_id, every.as_deref(), text.as_deref(), while_idle_only.then_some(true), off, mode))
                        }
                        PaneCmd::State { pane_id, stale_after } => {
                            let pane_id = current_pane(pane_id.as_ref(), "pane state");
                            cli::trace::exit(cli::pane_state_cli(pane_id, stale_after, mode))
                        }
                        PaneCmd::Slot { cmd } => match cmd {
                            PaneSlotCmd::Write {
                                name,
                                content,
                                pane_id,
                                append,
                                replace,
                            } => {
                                let pane_id = resolve_opt_pane(pane_id.as_ref());
                                cli::trace::exit(cli::pane_slot_write_cli(
                                    &name,
                                    content.as_deref(),
                                    append,
                                    replace,
                                    pane_id,
                                    mode,
                                ));
                            }
                            PaneSlotCmd::Read {
                                name,
                                pane_id,
                                pane_id_flag,
                            } => {
                                let target = pane_id.or(pane_id_flag);
                                let resolved = resolve_opt_pane(target.as_ref());
                                cli::trace::exit(cli::pane_slot_read_cli(&name, resolved))
                            }
                            PaneSlotCmd::Wait {
                                name,
                                pane_id,
                                pane_id_flag,
                                until,
                                timeout,
                            } => {
                                let target = pane_id.or(pane_id_flag);
                                let resolved = resolve_opt_pane(target.as_ref());
                                cli::trace::exit(cli::pane_slot_wait_cli(
                                    &name, resolved, &until, timeout,
                                ))
                            }
                            PaneSlotCmd::List {
                                pane_id,
                                pane_id_flag,
                            } => {
                                let target = pane_id.or(pane_id_flag);
                                let resolved = resolve_opt_pane(target.as_ref());
                                cli::trace::exit(cli::pane_slot_list_cli(resolved, mode))
                            }
                            PaneSlotCmd::Delete {
                                name,
                                pane_id,
                                pane_id_flag,
                            } => {
                                let target = pane_id.or(pane_id_flag);
                                let resolved = resolve_opt_pane(target.as_ref());
                                cli::trace::exit(cli::pane_slot_delete_cli(&name, resolved, mode))
                            }
                        },
                        PaneCmd::Message { cmd } => match cmd {
                            PaneMessageCmd::Send {
                                pane_id,
                                payload,
                                submit,
                            } => {
                                let pane_id = resolve_pane(&pane_id);
                                cli::trace::exit(cli::pane_message_send_cli(
                                    pane_id, &payload, submit,
                                    mode,
                                ))
                            }
                            PaneMessageCmd::Recv {
                                pane_id,
                                pane_id_flag,
                                wait,
                                timeout,
                            } => {
                                let target = pane_id.or(pane_id_flag);
                                let resolved = resolve_opt_pane(target.as_ref());
                                cli::trace::exit(cli::pane_message_recv_cli(
                                    resolved,
                                    wait,
                                    timeout.unwrap_or(300.0),
                                ))
                            }
                        },
                        PaneCmd::Result {
                            cmd,
                            pane_id,
                            pane_id_flag,
                            wait,
                            timeout,
                        } => match cmd {
                            Some(PaneResultCmd::Read {
                                pane_id,
                                pane_id_flag,
                                wait,
                                timeout,
                            }) => {
                                let target = pane_id.or(pane_id_flag);
                                let resolved = resolve_opt_pane(target.as_ref());
                                cli::trace::exit(cli::pane_result_read_cli(resolved, wait, timeout))
                            }
                            Some(PaneResultCmd::Wait {
                                pane_id,
                                pane_id_flag,
                                timeout,
                            }) => {
                                let target = pane_id.or(pane_id_flag);
                                let resolved = resolve_opt_pane(target.as_ref());
                                cli::trace::exit(cli::pane_result_read_cli(resolved, true, timeout))
                            }
                            None => {
                                let target = pane_id.or(pane_id_flag);
                                let resolved = resolve_opt_pane(target.as_ref());
                                cli::trace::exit(cli::pane_result_read_cli(
                                    resolved, wait, timeout,
                                ));
                            }
                        },
                        PaneCmd::New {
                            cmd,
                            name,
                            down,
                            left,
                            up,
                            right,
                            tab,
                            window,
                            overlay,
                            from,
                            ephemeral,
                            no_focus,
                            cwd,
                            worktree,
                            result_schema,
                            on_load,
                            extra_args,
                            agent,
                            boot_timeout,
                        } => {
                            let from = resolve_opt_pane(from.as_ref());
                            let agent = agent.map(|command| cli::AgentBootRequest {
                                command,
                                timeout_secs: boot_timeout,
                            });
                            let layout = if down {
                                "split_v"
                            } else if left {
                                "split_left"
                            } else if up {
                                "split_above"
                            } else if right {
                                "split_h"
                            } else if tab {
                                "tab"
                            } else if window {
                                "new_window"
                            } else if overlay {
                                "overlay"
                            } else {
                                "split_h"
                            };
                            cli::trace::exit(cli::pane_new_cli(cli::PaneNewCliArgs {
                                cmd: cmd.as_deref(),
                                name: name.as_deref(),
                                layout: Some(layout),
                                from_pane_id: from,
                                cwd: cwd.as_deref(),
                                ephemeral,
                                no_focus,
                                extra_args: &extra_args,
                                agent: agent.as_ref(),
                                worktree: worktree.as_deref(),
                                result_schema: result_schema.as_deref(),
                                on_load: &on_load,
                                mode,
                                ..Default::default()
                            }));
                        }
                    },
                    Commands::Events { cmd } => match cmd {
                        EventsCmd::Subscribe {
                            plugin_id,
                            stream,
                            all,
                            payload,
                            trigger,
                            resource,
                        } => cli::trace::exit(cli::events_subscribe_cli(
                            &plugin_id,
                            stream.as_deref(),
                            all,
                            &payload,
                            &trigger,
                            resource.as_deref(),
                        )),
                        EventsCmd::Declare {
                            plugin_id,
                            stream,
                            schema,
                            description,
                            timeout,
                        } => cli::trace::exit(cli::events_declare_cli(
                            &plugin_id,
                            &stream,
                            &schema,
                            description.as_deref(),
                            timeout,
                        )),
                        EventsCmd::Emit {
                            plugin_id,
                            event,
                            summary,
                            resource,
                            revision_after,
                            actor,
                            resource_scope,
                            payload,
                            state_ref,
                            revision_before,
                            rollback_token,
                            changed_resources,
                        } => cli::trace::exit(cli::events_emit_cli(cli::EmitArgs {
                            plugin_id: &plugin_id,
                            event: &event,
                            summary: &summary,
                            resource: &resource,
                            revision_after: &revision_after,
                            actor: &actor,
                            resource_scope: resource_scope.as_deref(),
                            payload: payload.as_deref(),
                            state_ref: state_ref.as_deref(),
                            revision_before: revision_before.as_deref(),
                            rollback_token: rollback_token.as_deref(),
                            changed_resources: &changed_resources,
                        })),
                        EventsCmd::List => cli::trace::exit(cli::events_list_cli(mode)),
                        EventsCmd::McpConfig => cli::trace::exit(cli::events_mcp_config_cli()),
                    },
                    Commands::CompleteOpen { prefix } => {
                        cli::trace::exit(cli::complete_open_cli(&prefix));
                    }
                    Commands::CompleteRun => {
                        cli::trace::exit(cli::complete_run_cli());
                    }
                    Commands::Descriptor { cmd } => match cmd {
                        DescriptorCmd::Probe {
                            command,
                            no_registry,
                            no_crawl,
                            extra_args,
                        } => {
                            if json {
                                let runner = cli::descriptor::RealRunner;
                                match cli::descriptor::resolve_cli_with(
                                    &runner,
                                    &command,
                                    !no_registry,
                                    !no_crawl,
                                ) {
                                    Ok(resolved) => {
                                        match serde_json::to_string_pretty(&resolved.descriptor) {
                                            Ok(j) => {
                                                println!("{j}");
                                                cli::trace::exit(0);
                                            }
                                            Err(e) => {
                                                eprintln!(
                                                    "error: could not serialize descriptor: {e}"
                                                );
                                                cli::trace::exit(1);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("error: {e}");
                                        cli::trace::exit(1);
                                    }
                                }
                            }
                            let runner = cli::descriptor::RealRunner;
                            let opts = cli::descriptor::ProbeOptions {
                                use_registry: !no_registry,
                                use_crawl: !no_crawl,
                            };
                            let extra: Vec<&str> = extra_args.iter().map(|s| s.as_str()).collect();
                            cli::trace::exit(cli::descriptor::probe_with_options(
                                &runner, &command, &extra, &opts,
                            ));
                        }
                    },
                    Commands::Account { cmd } => match cmd {
                        AccountCmd::Status => cli::trace::exit(cli::account_status_cli()),
                        AccountCmd::Login { email } => {
                            cli::trace::exit(cli::account_login_cli(email.as_deref()))
                        }
                        AccountCmd::Logout => cli::trace::exit(cli::account_logout_cli()),
                    },
                    Commands::Registry { cmd } => match cmd {
                        RegistryCmd::Watch { cli: only } => {
                            cli::trace::exit(cli::registry::watch_cli(only.as_deref()));
                        }
                    },
                    Commands::Context { cmd } => match cmd {
                        ContextCmd::New {
                            name,
                            path,
                            parent,
                            window,
                            focus,
                            from,
                            down,
                            left,
                            up,
                            right: _,
                        } => {
                            let from = resolve_opt_pane(from.as_ref());
                            let dir = if down {
                                "down"
                            } else if left {
                                "left"
                            } else if up {
                                "up"
                            } else {
                                "right"
                            };
                            cli::trace::exit(cli::context_new_cli(
                                name.as_deref(),
                                path.as_deref(),
                                parent.as_deref(),
                                &window,
                                dir,
                                cli::ContextSpawnPlacement { focus, from },
                                mode,
                            ))
                        }
                        ContextCmd::Sub {
                            name,
                            path,
                            agents,
                            command,
                            layout,
                            focus,
                            from,
                        } => {
                            let from = resolve_opt_pane(from.as_ref());
                            cli::trace::exit(cli::context_sub_cli(
                                &name,
                                path.as_deref(),
                                agents,
                                &command,
                                layout,
                                cli::ContextSpawnPlacement { focus, from },
                                mode,
                            ))
                        }
                        ContextCmd::Open { path } => {
                            cli::trace::exit(cli::context_open_cli(path.as_deref(), mode))
                        }
                        ContextCmd::SetRoot { path } => {
                            cli::trace::exit(cli::context_set_root_cli(path.as_deref(), mode))
                        }
                        ContextCmd::Current => cli::trace::exit(cli::context_current_cli(mode)),
                        ContextCmd::Describe { text } => {
                            cli::trace::exit(cli::context_describe_cli(&text, mode))
                        }
                        ContextCmd::Zoom { context_id } => {
                            let context_id = resolve_ctx(&context_id);
                            cli::trace::exit(cli::context_zoom_cli(context_id, mode))
                        }
                        ContextCmd::ZoomOut => cli::trace::exit(cli::context_zoom_out_cli()),
                        ContextCmd::Push { name, pane_id } => {
                            let pane_id = resolve_opt_pane(pane_id.as_ref());
                            cli::trace::exit(cli::context_push_cli(name.as_deref(), pane_id))
                        }
                        ContextCmd::Rename { name, context_id } => {
                            let context_id = resolve_opt_ctx(context_id.as_ref());
                            cli::trace::exit(cli::host_action_cli(
                                serde_json::json!({"type":"context_rename", "name":name}),
                                Some(("context_id", context_id, "PLEXI_CONTEXT_ID")),
                                mode,
                            ))
                        }
                        ContextCmd::Close { context_id } => {
                            let context_id = resolve_opt_ctx(context_id.as_ref());
                            cli::trace::exit(cli::host_action_cli(
                                serde_json::json!({"type":"context_close"}),
                                Some(("context_id", context_id, "PLEXI_CONTEXT_ID")),
                                mode,
                            ))
                        }
                        ContextCmd::Park { ref context_id }
                        | ContextCmd::Unpark { ref context_id } => {
                            let parked = matches!(cmd, ContextCmd::Park { .. });
                            let context_id = resolve_opt_ctx(context_id.as_ref());
                            cli::trace::exit(cli::host_action_cli(
                                serde_json::json!({"type":"context_park", "parked":parked}),
                                Some(("context_id", context_id, "PLEXI_CONTEXT_ID")),
                                mode,
                            ))
                        }
                        ContextCmd::List => cli::trace::exit(cli::context_list_cli()),
                    },
                    Commands::Completions { .. } => {
                        unreachable!("completions are a front-end outcome")
                    }
                    Commands::Cli {
                        cmd,
                        modify_path,
                        no_modify_path,
                    } => {
                        let (sub_modify, sub_no_modify) = match cmd {
                            Some(CliSetupCmd::Setup {
                                modify_path: m,
                                no_modify_path: nm,
                            }) => (m, nm),
                            None => (false, false),
                        };
                        let _ = (modify_path, sub_modify);
                        let effective_modify = !(no_modify_path || sub_no_modify);
                        cli::trace::exit(cli::setup::run_cli_setup(effective_modify, json));
                    }
                    Commands::Config { cmd: config_cmd } => match config_cmd {
                        ConfigCmd::Reload => cli::trace::exit(cli::host_action_cli(
                            serde_json::json!({"type":"reload_config"}),
                            None,
                            mode,
                        )),
                        ConfigCmd::Check { scope } => {
                            cli::trace::exit(cli::config_check(scope.scope()));
                        }
                        ConfigCmd::Edit { scope } => {
                            cli::trace::exit(cli::config_edit(scope.scope()));
                        }
                        ConfigCmd::Get { scope, key } => {
                            cli::trace::exit(cli::config_get(&key, scope.scope(), json));
                        }
                        ConfigCmd::Reset { scope } => {
                            cli::trace::exit(cli::config_reset(scope.scope()));
                        }
                        ConfigCmd::List { scope } => {
                            cli::trace::exit(cli::config_list(scope.scope(), json));
                        }
                        ConfigCmd::Set { scope, pairs } => {
                            cli::trace::exit(cli::config_set(&pairs, scope.scope()));
                        }
                    },
                    Commands::Note { cmd, text } => {
                        cli::trace::exit(cli::notes::note_cli(cmd, text))
                    }
                    Commands::Notes { cmd } => cli::trace::exit(cli::notes::notes_compat_cli(cmd)),
                    Commands::Debug { cmd } => match cmd {
                        DebugCmd::Session { cmd } => match cmd {
                            DebugSessionCmd::List => {
                                cli::trace::exit(cli::debug::debug_session_list_cli(json))
                            }
                            DebugSessionCmd::Show {
                                session,
                                pane,
                                join_agents,
                                cli,
                                utc,
                            } => cli::trace::exit(cli::debug::debug_session_show_cli(
                                session.as_deref(),
                                pane,
                                join_agents,
                                cli,
                                json,
                                utc,
                            )),
                        },
                    },
                    Commands::Ledger { cmd } => {
                        let (by, since) = match cmd {
                            Some(LedgerCmd::Summary { by, since }) => (by, since),
                            None => (None, None),
                        };
                        cli::trace::exit(cli::ledger_summary_cli(
                            by.as_deref(),
                            since.as_deref(),
                            json,
                        ));
                    }
                    Commands::Ai { cmd } => match cmd {
                        AiCmd::Onboard => cli::trace::exit(cli::ai_onboard_cli()),
                        AiCmd::Doctor => cli::trace::exit(cli::ai_doctor_cli(json)),
                        AiCmd::Setup => cli::trace::exit(cli::ai_setup_cli()),
                    },
                    Commands::Doctor => cli::trace::exit(cli::doctor_cli(json)),
                    Commands::Demo => cli::trace::exit(cli::demo_cli()),
                }
            }
        }
    }

    // When invoked with no arguments outside Plexi, print a brief launch
    // notice so the terminal isn't silent for first-time users (#1515).
    if !cli_mode && adopted_root.is_none() {
        println!("Starting Plexi — run 'plexi --help' to see available commands.");
    }

    // One-shot RPC response files this host's clients abandoned (died before
    // the answer landed) accumulate in the profile's rpc/ dir; sweep them at
    // boot, mirroring the log rotate-and-prune in `platform::logging::init`.
    rpc::sweep_stale(&config::config_dir());

    if let Err(error) = cli::agent::ensure_agent_state_script() {
        log::error!("host_startup: failed to restore shared agent hook: {error}");
        eprintln!("Could not restore Plexi's agent hook: {error}");
    }
    if let Err(error) = cli::setup::ensure_local_shim() {
        log::warn!("host_startup: could not ensure local CLI shim: {error}");
    }

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/app-icon.png"))
        .expect("failed to load plugin icon");

    // Resolve the login-shell PATH and adopt API keys only for the GUI.
    // CLI subcommands already inherit the full shell environment from the
    // calling terminal — running this there corrupts terminal signal state
    // (zsh -i hijacks SIGINT) and spams the user's stdout with log noise.
    crate::host::shell::scrub_launcher_color_overrides();
    crate::host::shell::install_login_shell_path();
    crate::host::shell::install_login_shell_env();

    // Shell probes are done. Start the heartbeat now so it only monitors
    // eframe operation — not pre-startup shell work (#588).
    let heartbeat_ctx = crate::platform::logging::new_heartbeat_ctx_slot();
    let ui_phase = crate::platform::logging::new_ui_phase_tracker();
    crate::platform::logging::spawn_heartbeat(
        frame_tick.clone(),
        heartbeat_ctx.clone(),
        ui_phase.clone(),
    );

    log::info!("ui_stack: egui=0.34.3 renderer=wgpu");

    let background = boot_state.is_background();
    #[cfg(target_os = "macos")]
    let activation_policy = if background {
        "accessory"
    } else {
        "bundle-default"
    };
    #[cfg(not(target_os = "macos"))]
    let activation_policy = "platform-default";
    #[cfg(target_os = "macos")]
    let activate_ignoring_other_apps = if background {
        "false"
    } else {
        "bundle-default"
    };
    #[cfg(not(target_os = "macos"))]
    let activate_ignoring_other_apps = "not-applicable";
    log::info!(
        "host_startup: background={background} activation_policy={activation_policy} \
         activate_ignoring_other_apps={activate_ignoring_other_apps}"
    );

    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1400.0, 900.0])
        .with_min_inner_size([400.0, 300.0])
        .with_title(env!("PLEXI_APP_TITLE"))
        .with_icon(icon)
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false);
    #[cfg(target_os = "macos")]
    log::info!("titlebar: native title hidden; toolbar renders measured window title");
    #[cfg(target_os = "linux")]
    let viewport = viewport.with_app_id(crate::config::current_exe_basename());
    let native_options = eframe::NativeOptions {
        // eframe 0.34 split root close into CloseRequested followed by Exit.
        // On macOS its run-on-demand loop can stall between those events after
        // destroying the window, leaving a headless process behind.
        run_and_return: false,
        viewport,
        ..Default::default()
    };

    #[cfg(target_os = "macos")]
    let native_options = if background {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};

        let mut options = native_options;
        options.event_loop_builder = Some(Box::new(|builder| {
            builder
                .with_activation_policy(ActivationPolicy::Accessory)
                .with_activate_ignoring_other_apps(false);
        }));
        options
    } else {
        native_options
    };

    eframe::run_native(
        env!("CARGO_PKG_NAME"),
        native_options,
        Box::new(|cc| {
            let mut app = app::PlexiApp::new(cc, frame_tick, heartbeat_ctx, ui_phase);
            app.surface_launch_notices();
            Ok(Box::new(app))
        }),
    )
}

/// What startup executes once the front end has ruled out display and usage
/// errors.
enum Startup {
    Run(Box<cli::front::Run>),
    Launch(Box<cli::front::Launch>),
}

/// The `plexi <path>` "open folder" operand, modelled on VS Code. Returns
/// `Ok(Some(workspace_root))` when an ancestor `.plexi/` is found, `Ok(None)`
/// when there is no operand or the directory has no `.plexi/` ancestor (in
/// which case an adopted context path is set via
/// `config::set_adopted_context_path`), and `Err(_)` only for invalid paths.
fn resolve_workspace_operand(operand: Option<&str>) -> Result<Option<std::path::PathBuf>, String> {
    let Some(operand) = operand else {
        return Ok(None);
    };
    let candidate = std::path::PathBuf::from(operand);
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
    let canonical = candidate
        .canonicalize()
        .map_err(|e| format!("canonicalize {}: {e}", candidate.display()))?;
    match crate::app::registry::resolve_workspace_root(&canonical) {
        Some(root) => Ok(Some(root)),
        None => {
            crate::config::set_adopted_context_path(canonical);
            Ok(None)
        }
    }
}

/// Rust ignores SIGPIPE by default, so println! panics on a closed pipe.
/// Restore Unix pipeline behavior before any CLI output, including early
/// help/build-info paths. Never change the long-lived host's disposition.
#[cfg(unix)]
fn configure_cli_sigpipe(host_startup: bool) -> std::io::Result<()> {
    if !host_startup {
        // SAFETY: SIG_DFL is a valid disposition; no Rust signal handler is
        // installed. This runs at startup before application threads exist.
        if unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) } == libc::SIG_ERR {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Best-effort stderr output with the CLI's stdout SIGPIPE policy left intact.
fn write_stderr_diagnostic(message: std::fmt::Arguments<'_>) {
    use std::io::Write;

    let line = format!("{message}\n");
    let mut stderr = std::io::stderr().lock();
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;

        // Darwin sys/fcntl.h. The locked libc does not export these yet.
        const F_SETNOSIGPIPE: libc::c_int = 73;
        const F_GETNOSIGPIPE: libc::c_int = 74;
        let fd = stderr.as_raw_fd();
        // SAFETY: fcntl operates on stderr and takes integer arguments here.
        // Darwin can deliver write-generated SIGPIPE to the process, so
        // suppress it on the descriptor. Restore the flag for any aliases.
        unsafe {
            let previous = libc::fcntl(fd, F_GETNOSIGPIPE);
            if previous < 0 || (previous == 0 && libc::fcntl(fd, F_SETNOSIGPIPE, 1) < 0) {
                return;
            }
            let _ = stderr.write_all(line.as_bytes());
            if previous == 0 {
                let _ = libc::fcntl(fd, F_SETNOSIGPIPE, 0);
            }
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // SAFETY: signal sets are initialized before use, and the mask applies
        // to this thread only. A zero timeout consumes a pending SIGPIPE from
        // our write without waiting. Keep any caller-blocked signal pending.
        unsafe {
            let mut set = std::mem::zeroed();
            let mut previous = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGPIPE);
            if libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut previous) != 0 {
                return;
            }
            if let Err(error) = stderr.write_all(line.as_bytes()) {
                if error.kind() == std::io::ErrorKind::BrokenPipe
                    && libc::sigismember(&previous, libc::SIGPIPE) == 0
                {
                    let zero = libc::timespec {
                        tv_sec: 0,
                        tv_nsec: 0,
                    };
                    while libc::sigtimedwait(&set, std::ptr::null_mut(), &zero) < 0
                        && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                    {
                    }
                }
            }
            let _ = libc::pthread_sigmask(libc::SIG_SETMASK, &previous, std::ptr::null_mut());
        }
    }
    #[cfg(not(unix))]
    let _ = stderr.write_all(line.as_bytes());
}

fn install_panic_hook(panic_log: std::path::PathBuf) {
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;

        let msg = info.to_string();
        // Best-effort append to the log file directly (logger may be dead).
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&panic_log)
        {
            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            let _ = writeln!(f, "[{now}] [ERROR] [plexi::panic] PANIC: {msg}");
        }
        // Also print to stderr so crash logs / Console.plugin capture it.
        write_stderr_diagnostic(format_args!("PLEXI PANIC: {msg}"));
    }));
}

/// How this process holds the profile after the launch is classified.
enum LaunchProfile {
    /// A CLI command, and what its profile initialization did.
    Cli(crate::config::ProfileInit),
    /// A host launch, holding the initialization lock it will claim under.
    Host(crate::config::ProfileInitLock),
}

/// Seed a profile this process just initialized. The caller holds the
/// initialization lock.
fn seed_initialized_profile(is_plugin_prune_command: bool) {
    let seeding_start = std::time::Instant::now();
    let is_first_launch = crate::config::ensure_profile_initialized();

    if !crate::config::PlexiConfig::load()
        .core_only
        .unwrap_or(false)
    {
        let plugins_dir = crate::app::registry::plugins_dir();
        let cloner = crate::cli::install_host::GitCloner::default();

        // On first launch, also seed the example plugins.
        if is_first_launch {
            if let Some(outcomes) =
                crate::cli::install_host::apply_examples_pack_if_empty(&cloner, &plugins_dir)
            {
                let n_ok = outcomes
                    .iter()
                    .filter(|o| {
                        matches!(
                            o.status,
                            crate::cli::install_host::InstallStatus::Installed(_)
                        )
                    })
                    .count();
                log::info!(
                    "examples pack: seeded {n_ok} plugins to {}",
                    plugins_dir.display()
                );
            }
        }

        // Always re-seed any deleted core plugins.
        let core_outcomes = crate::cli::install_host::apply_core_pack_always(&cloner, &plugins_dir);
        let n_installed = core_outcomes
            .iter()
            .filter(|o| {
                matches!(
                    o.status,
                    crate::cli::install_host::InstallStatus::Installed(_)
                )
            })
            .count();
        if n_installed > 0 {
            log::info!(
                "core pack: re-seeded {n_installed} missing plugins to {}",
                plugins_dir.display()
            );
        }
        for o in &core_outcomes {
            if let crate::cli::install_host::InstallStatus::Failed(msg) = &o.status {
                log::warn!("core pack: FAILED {}: {msg}", o.id);
            }
        }
        if !is_plugin_prune_command {
            let pruned =
                crate::cli::install_host::reconcile_orphaned_pre_v3_first_party_apps(&plugins_dir);
            if !pruned.is_empty() {
                log::info!(
                    "core pack: quarantined {} orphaned pre-v3 plugin(s) from {}",
                    pruned.len(),
                    plugins_dir.display()
                );
            }
        }
        // Packaged plugin definitions stay untouched in core-only mode too.
        if let Err(error) = crate::distribution::seed_profile() {
            log::error!("package profile initialization: {error}");
            eprintln!("package profile initialization: {error}");
            cli::trace::exit(1);
        }
    }
    log::info!(
        "startup: profile initialization took {}ms first_launch={is_first_launch}",
        seeding_start.elapsed().as_millis()
    );
}

/// Headless render subcommand: reads `{viewport, draw_commands}` JSON from stdin,
/// writes PNG bytes to stdout. Used by the Python snapshot test harness.
/// Invoked via `plexi --render`. Zero GUI init cost.
fn render_cli() {
    use std::io::{Read, Write};
    let mut input = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut input) {
        eprintln!("plexi --render: failed to read stdin: {e}");
        cli::trace::exit(1);
    }
    let req: serde_json::Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("plexi --render: invalid JSON: {e}");
            cli::trace::exit(1);
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
            cli::trace::exit(1);
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
    let renderer = crate::render::headless_renderer::HeadlessRenderer::new();
    let png_bytes = renderer.render_pgap_frame(&all_commands, width, height);
    if let Err(e) = std::io::stdout().write_all(&png_bytes) {
        eprintln!("plexi --render: failed to write PNG to stdout: {e}");
        cli::trace::exit(1);
    }
}

#[cfg(test)]
mod cli_tests {
    use super::resolve_workspace_operand;
    use std::fs;

    #[cfg(unix)]
    fn wait_for_cli_child(mut child: std::process::Child) -> std::process::Output {
        use std::time::{Duration, Instant};

        let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(30));
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!("CLI child hung: {output:?}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        child.wait_with_output().unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn panic_hook_logs_first_panic_with_closed_stderr() {
        use std::process::{Command, Stdio};

        const LOG_PATH: &str = "PLEXI_PANIC_TEST_LOG_PATH";
        const FIRST_PANIC: &str = "intentional first panic";
        if let Some(path) = std::env::var_os(LOG_PATH) {
            super::install_panic_hook(path.into());
            // libtest ignores SIGPIPE. The closed pipe returns EPIPE even
            // during the hook, where another panic would abort the process.
            assert!(std::panic::catch_unwind(|| panic!("{FIRST_PANIC}")).is_err());
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("panic.log");
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cli_tests::panic_hook_logs_first_panic_with_closed_stderr",
                "--nocapture",
            ])
            .env_clear()
            .env(LOG_PATH, &path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(writer)
            .spawn()
            .unwrap();
        let output = wait_for_cli_child(child);
        let log = fs::read_to_string(path).unwrap();
        assert!(output.status.success(), "{output:?}: {log}");
        assert_eq!(log.matches("PANIC:").count(), 1, "{log}");
        assert!(log.contains(FIRST_PANIC), "{log}");
    }

    #[cfg(unix)]
    #[test]
    fn cli_closed_stdout_uses_sigpipe() {
        use std::os::fd::{FromRawFd, OwnedFd};
        use std::os::unix::process::{CommandExt, ExitStatusExt};
        use std::process::{Command, Stdio};

        // The gate builds release before tests (including macOS/Linux CI).
        // Never re-exec libtest: its extra threads can print an EPIPE panic
        // while macOS is still delivering process-wide SIGPIPE termination.
        let test_exe = std::env::current_exe().unwrap();
        let binary = test_exe
            .parent()
            .unwrap() // deps
            .parent()
            .unwrap() // debug or release
            .with_file_name("release")
            .join("plexi");
        assert!(
            binary.is_file(),
            "run `just build` before tests: {}",
            binary.display()
        );
        let run = |stdout: Stdio| {
            let child = Command::new(&binary)
                .arg("--build-info")
                .stdin(Stdio::null())
                .stdout(stdout)
                .stderr(Stdio::piped())
                .process_group(0)
                .spawn()
                .unwrap();
            wait_for_cli_child(child)
        };

        // Reject a stale release executable instead of silently validating
        // another checkout or an earlier production implementation.
        let open = run(Stdio::piped());
        assert!(open.status.success(), "{open:?}");
        assert!(open.stderr.is_empty(), "{open:?}");
        let identity: serde_json::Value = serde_json::from_slice(&open.stdout).unwrap();
        assert_eq!(
            identity["build_id"],
            env!("PLEXI_BUILD_ID"),
            "re-run `just build`"
        );

        let mut fds = [-1; 2];
        // SAFETY: pipe initializes both descriptors; OwnedFd takes ownership
        // exactly once. Only the writer is passed to the dedicated CLI child.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let reader = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        let writer = unsafe { OwnedFd::from_raw_fd(fds[1]) };
        drop(reader);
        let closed = run(Stdio::from(writer));
        assert_eq!(closed.status.signal(), Some(libc::SIGPIPE), "{closed:?}");
        assert!(closed.stderr.is_empty(), "{closed:?}");
    }

    #[test]
    fn plexi_path_arg_adopts_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        fs::create_dir_all(workspace.path().join(&channel_dir)).unwrap();
        let path_str = workspace.path().to_string_lossy().to_string();

        let resolved = resolve_workspace_operand(Some(&path_str))
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

        let resolved = resolve_workspace_operand(Some(&path_str))
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
    fn no_operand_and_missing_paths() {
        assert!(resolve_workspace_operand(None).unwrap().is_none());
        assert!(resolve_workspace_operand(Some("./definitely-not-here-1663")).is_err());
    }
}
