#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod app_protocol;
mod app_registry;
mod app_api;
mod app_permissions;
mod app_trait;
mod cli;
mod command_palette;
mod file_browser_app;
mod process_app;
mod quick_note_app;
mod config;
mod context;
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
                    eprintln!("Usage: plexi secret <set|list> [key]");
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
                    other => {
                        eprintln!("Unknown secret subcommand: {other}");
                        eprintln!("Usage: plexi secret <set|list>");
                        std::process::exit(1);
                    }
                }
            }
            _ => {} // Not a CLI subcommand — fall through to GUI
        }
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
