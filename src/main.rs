#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod app_protocol;
mod app_registry;
mod app_api;
mod app_permissions;
mod audio_app;
mod app_trait;
mod cli;
mod command_palette;
mod file_browser;
mod process_app;
mod quick_note_app;
mod text_editor_app;
mod config;
mod context;
mod features;
mod keys;
#[cfg(target_os = "macos")]
mod macos_menu;
mod overlays;
mod pane;
mod pane_ops;
mod secrets;
mod shell;
mod sidebar;
mod theme;
mod tiling;
mod workspace;

fn main() -> eframe::Result {
    env_logger::init();

    // Handle CLI subcommands before launching the GUI.
    let args: Vec<String> = std::env::args().collect();
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
                eprintln!("plexi: already running inside Plexi. Nearest workspace: {}", dir.join(".plexi").display());
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

    let icon =
        eframe::icon_data::from_png_bytes(include_bytes!("../assets/app-icon.png"))
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
