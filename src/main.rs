#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Ship-time panic-path protection. `todo!()` / `unimplemented!()` compile clean
// but panic at runtime — e.g. 2026-04-18, CoreAudioDevice::start_capture was
// `todo!()` and froze the GUI when a recorder app sent AudioCapture without
// PLEXI_AUDIO=mock://. Stubs must return `Err(NotImplemented)` instead.
#![deny(clippy::todo, clippy::unimplemented)]

mod app;
#[allow(dead_code)] // PermissionsLog used via load()/resolve() at permission-prompt sites
mod app_permissions;
#[allow(dead_code)] // STEP-9: AppReply, emit_* wired when PGAP surface completes
mod app_protocol;
#[allow(dead_code)] // STEP-5: app_by_id / permissions_for wired via SpawnService
mod app_registry;
mod app_trait;
mod cli;
mod command_palette;
mod config;
mod context;
#[allow(dead_code)] // STEP-6: emit_* wired when FileEventSink consumes all effects
mod event_log;
mod fd_util;
mod features;
mod file_browser;
#[allow(dead_code)] // STEP-4/6: HostHarness + VecEventSink used by unit tests; kept alongside prod
mod host;
mod keys;
mod logging;
#[cfg(target_os = "macos")]
mod macos_menu;
mod overlays;
mod pane;
mod pane_ops;
#[allow(dead_code)] // STEP-3: plexi_iq simplified/trimmed when stubs resolved
mod plexi_iq;
#[allow(dead_code)] // STEP-5/9: SpawnService + PipeSend routing complete the surface
mod process_app;
mod headless_renderer;
mod protocol;
mod quick_note_app;
#[allow(dead_code)] // STEP-6: Runs palette consumes run state via FileEventSink
mod runs;
#[allow(dead_code)] // STEP-5: secrets flow through SecretsService trait object
mod secrets;
mod secrets_app;
mod shell;
mod sidebar;
#[allow(dead_code)] // text-editor builtin; kept until file_browser refactor
mod text_editor_app;
mod theme;
mod tiling;
#[allow(dead_code)] // STEP-9: PipeSend peer routing + binary pipe surface finish the protocol
mod typed_pipes;
mod workspace;

#[cfg(test)]
#[allow(dead_code)] // STEP-10: harness helpers are test-only; real #[test] fns land next
mod pgap_test_harness;

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

    let log_level = crate::config::PlexiConfig::load()
        .log
        .and_then(|l| l.level_filter())
        .unwrap_or(log::LevelFilter::Info);
    crate::logging::init(log_level);

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
            "secret" => {
                if args.len() < 3 {
                    eprintln!("Usage: plexi secret <set|list|delete> [key]");
                    std::process::exit(1);
                }
                match args[2].as_str() {
                    "set" => {
                        if args.len() < 4 {
                            eprintln!("Usage: plexi secret set <key>");
                            std::process::exit(1);
                        }
                        std::process::exit(cli::set_secret(&args[3]));
                    }
                    "list" => {
                        std::process::exit(cli::list_secrets());
                    }
                    "delete" => {
                        if args.len() < 4 {
                            eprintln!("Usage: plexi secret delete <key>");
                            std::process::exit(1);
                        }
                        std::process::exit(cli::delete_secret_cli(&args[3]));
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
            .with_title("Plexi")
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "plexi",
        native_options,
        Box::new(|cc| Ok(Box::new(app::PlexiApp::new(cc)))),
    )
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
