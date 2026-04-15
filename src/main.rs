#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// ─────────────────────────────────────────────────────────────────────────────
// Phase 1 of the WASM/PWA split (see docs/specs/proposals/wasm-pwa-deployment.md).
//
// Every module that touches the OS — PTY, filesystem, subprocess, Keychain,
// audio devices, file watching — is gated behind `cfg(not(target_arch = "wasm32"))`.
// The UI rendering modules (`app`, `pane`, `pane_ops`, `tiling`, `theme`,
// `command_palette`, `file_browser`) also depend transitively on `egui_term`
// and `shell`, which are native-only, so they are gated here as well until a
// later phase isolates the rendering core from the PTY/shell layer.
//
// The wasm build currently compiles only the self-contained leaf modules
// (protocol types, trait definitions, pure state structs). A real wasm UI is
// Phase 3 work — this phase just sets up the compile scaffolding.
// ─────────────────────────────────────────────────────────────────────────────

// ── Shared modules — compile on every target ────────────────────────────────
//
// These modules are pure data/protocol definitions with no OS dependencies.
// They're the seed of what will eventually be the WASM-compilable UI layer.
mod app_protocol;

// `agent_mode` pulls in `agent_llm` (ureq) and `secrets` (Keychain) which are
// native-only. Gated until a later phase isolates the state machine from the
// I/O backends.
#[cfg(not(target_arch = "wasm32"))]
mod agent_mode;

// `features` and `app_permissions` transitively reference `config` and
// `app_trait`, which are native-only. Gated until they're decoupled.
#[cfg(not(target_arch = "wasm32"))]
mod features;
#[cfg(all(target_os = "macos", not(target_arch = "wasm32")))]
mod finder_service;
#[cfg(not(target_arch = "wasm32"))]
mod app_permissions;

// ── Native-only modules ──────────────────────────────────────────────────────
//
// Everything below depends on `std::fs`, `std::process`, `egui_term`,
// `rodio`, `notify`, `dirs`, or macOS-specific crates. Gated until Phase 2
// introduces a WebSocket transport that replaces direct OS access.
#[cfg(not(target_arch = "wasm32"))]
mod agent_ansi;
#[cfg(not(target_arch = "wasm32"))]
mod agent_context;
#[cfg(not(target_arch = "wasm32"))]
mod agent_llm;
#[cfg(not(target_arch = "wasm32"))]
mod app;
#[cfg(not(target_arch = "wasm32"))]
mod app_registry;
#[cfg(not(target_arch = "wasm32"))]
mod app_api;
#[cfg(not(target_arch = "wasm32"))]
mod audio_app;
#[cfg(not(target_arch = "wasm32"))]
mod app_trait;
#[cfg(not(target_arch = "wasm32"))]
mod cli;
#[cfg(not(target_arch = "wasm32"))]
mod command_palette;
#[cfg(not(target_arch = "wasm32"))]
mod file_browser;
#[cfg(not(target_arch = "wasm32"))]
mod process_app;
#[cfg(not(target_arch = "wasm32"))]
mod quick_note_app;
#[cfg(not(target_arch = "wasm32"))]
mod secrets_app;
#[cfg(not(target_arch = "wasm32"))]
mod text_editor_app;
#[cfg(not(target_arch = "wasm32"))]
mod config;
#[cfg(not(target_arch = "wasm32"))]
mod context;
#[cfg(not(target_arch = "wasm32"))]
mod cost_tracker;
#[cfg(not(target_arch = "wasm32"))]
mod event_log;
#[cfg(not(target_arch = "wasm32"))]
mod run_store;
#[cfg(not(target_arch = "wasm32"))]
mod plexi_iq;
#[cfg(not(target_arch = "wasm32"))]
mod logging;
#[cfg(not(target_arch = "wasm32"))]
mod notification_log;
#[cfg(not(target_arch = "wasm32"))]
mod notification_palette;
#[cfg(not(target_arch = "wasm32"))]
mod notify_socket;
#[cfg(not(target_arch = "wasm32"))]
mod keys;
#[cfg(not(target_arch = "wasm32"))]
mod media_cache;
#[cfg(all(target_os = "macos", not(target_arch = "wasm32")))]
mod macos_menu;
#[cfg(not(target_arch = "wasm32"))]
mod overlays;
#[cfg(not(target_arch = "wasm32"))]
mod pane;
#[cfg(not(target_arch = "wasm32"))]
mod pane_ops;
#[cfg(not(target_arch = "wasm32"))]
mod secrets;
#[cfg(not(target_arch = "wasm32"))]
mod shell;
#[cfg(not(target_arch = "wasm32"))]
mod sidebar;
#[cfg(not(target_arch = "wasm32"))]
mod theme;
#[cfg(not(target_arch = "wasm32"))]
mod tiling;
#[cfg(target_os = "macos")]
mod trash;
#[cfg(not(target_arch = "wasm32"))]
mod workspace;

// ─────────────────────────────────────────────────────────────────────────────
// Native entry point
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let log_level = crate::config::PlexiConfig::load()
        .log
        .and_then(|l| l.level_filter())
        .unwrap_or(log::LevelFilter::Info);
    crate::logging::init(log_level);

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
            other => {
                // `plexi <path>` — open the given directory as a new context
                // on launch. Same code path as the macOS Finder service:
                // queue the path, and PlexiApp::update will pick it up.
                // Anything that is not a known subcommand and not a directory
                // falls through to the GUI without error.
                let path = std::path::PathBuf::from(other);
                if path.is_dir() {
                    let abs = path.canonicalize().unwrap_or(path);
                    #[cfg(target_os = "macos")]
                    {
                        finder_service::queue_path(abs);
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        let _ = abs;
                    }
                }
            }
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

// ─────────────────────────────────────────────────────────────────────────────
// WASM entry point — stub for Phase 1
// ─────────────────────────────────────────────────────────────────────────────
//
// This compiles the shared modules above (protocol, trait, pure state) for
// wasm32-unknown-unknown so we can verify the dependency gating in Cargo.toml
// is correct. A real WASM UI requires Phase 3 (ws_bridge + WebRunner wiring).
#[cfg(target_arch = "wasm32")]
fn main() {
    // No-op: the real wasm entry point will live in a separate `wasm_main`
    // module once Phase 3 wires up `eframe::WebRunner`. For now, `main` only
    // exists so `cargo build --target wasm32-unknown-unknown` has an entry
    // symbol for the binary crate.
}
