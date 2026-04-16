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
mod app_permissions;
#[cfg(not(target_arch = "wasm32"))]
mod features;
#[cfg(all(target_os = "macos", not(target_arch = "wasm32")))]
mod finder_service;

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
mod app_api;
#[cfg(not(target_arch = "wasm32"))]
mod app_registry;
#[cfg(not(target_arch = "wasm32"))]
mod app_trait;
#[cfg(not(target_arch = "wasm32"))]
mod audio_app;
#[cfg(not(target_arch = "wasm32"))]
mod cli;
#[cfg(not(target_arch = "wasm32"))]
mod command_palette;
#[cfg(not(target_arch = "wasm32"))]
mod config;
#[cfg(not(target_arch = "wasm32"))]
mod context;
#[cfg(not(target_arch = "wasm32"))]
mod cost_tracker;
#[cfg(not(target_arch = "wasm32"))]
mod depth_tree_app;
#[cfg(not(target_arch = "wasm32"))]
mod event_log;
#[cfg(not(target_arch = "wasm32"))]
mod run_store;
mod plexi_iq;
mod file_browser;
#[cfg(not(target_arch = "wasm32"))]
mod fractal_depth;
#[cfg(not(target_arch = "wasm32"))]
mod keys;
#[cfg(not(target_arch = "wasm32"))]
mod logging;
#[cfg(all(target_os = "macos", not(target_arch = "wasm32")))]
mod macos_menu;
#[cfg(not(target_arch = "wasm32"))]
mod media_cache;
#[cfg(not(target_arch = "wasm32"))]
mod notification_log;
#[cfg(not(target_arch = "wasm32"))]
mod notification_palette;
#[cfg(not(target_arch = "wasm32"))]
mod notify_socket;
#[cfg(not(target_arch = "wasm32"))]
mod run_palette;
#[cfg(not(target_arch = "wasm32"))]
mod overlays;
#[cfg(not(target_arch = "wasm32"))]
mod pane;
#[cfg(not(target_arch = "wasm32"))]
mod pane_ops;
#[cfg(not(target_arch = "wasm32"))]
mod process_app;
#[cfg(not(target_arch = "wasm32"))]
mod quick_note_app;
#[cfg(not(target_arch = "wasm32"))]
mod secrets;
#[cfg(not(target_arch = "wasm32"))]
mod secrets_app;
#[cfg(not(target_arch = "wasm32"))]
mod shell;
#[cfg(not(target_arch = "wasm32"))]
mod sidebar;
#[cfg(not(target_arch = "wasm32"))]
mod text_editor_app;
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
    let wants_help = args.iter().any(|arg| arg == "--help" || arg == "-h");
    let wants_embedded = args.iter().any(|arg| arg == "--embedded");

    if wants_help {
        print_main_help(wants_embedded);
        return Ok(());
    }

    if wants_embedded {
        run_embedded_stdio();
        return Ok(());
    }

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
            // `plexi secrets` — v2.0 plural form with --global, unset, which
            "secrets" => {
                if args.len() < 3 {
                    eprintln!("Usage: plexi secrets <set|unset|list|which> [--global] [key]");
                    std::process::exit(1);
                }
                match args[2].as_str() {
                    "set" => {
                        // plexi secrets set [--global] <KEY>
                        let mut global = false;
                        let mut key = "";
                        let mut i = 3;
                        while i < args.len() {
                            if args[i] == "--global" {
                                global = true;
                                i += 1;
                            } else {
                                key = args[i].as_str();
                                i += 1;
                            }
                        }
                        if key.is_empty() {
                            eprintln!("Usage: plexi secrets set [--global] <KEY>");
                            std::process::exit(1);
                        }
                        std::process::exit(cli::secrets_set(global, key));
                    }
                    "unset" => {
                        // plexi secrets unset [--global] <KEY>
                        let mut global = false;
                        let mut key = "";
                        let mut i = 3;
                        while i < args.len() {
                            if args[i] == "--global" {
                                global = true;
                                i += 1;
                            } else {
                                key = args[i].as_str();
                                i += 1;
                            }
                        }
                        if key.is_empty() {
                            eprintln!("Usage: plexi secrets unset [--global] <KEY>");
                            std::process::exit(1);
                        }
                        std::process::exit(cli::secrets_unset(global, key));
                    }
                    "list" => {
                        std::process::exit(cli::secrets_list());
                    }
                    "which" => {
                        if args.len() < 4 {
                            eprintln!("Usage: plexi secrets which <KEY>");
                            std::process::exit(1);
                        }
                        std::process::exit(cli::secrets_which(&args[3]));
                    }
                    other => {
                        eprintln!("Unknown secrets subcommand: {other}");
                        eprintln!("Usage: plexi secrets <set|unset|list|which> [--global] [key]");
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
    //
    // Alpha and beta builds are exempt: they need to run alongside the stable
    // daily-driver app for testing. Only same-version re-entry is blocked.
    let is_dev_build = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .map(|name| name.contains("alpha") || name.contains("beta"))
        .unwrap_or(false);

    if !is_dev_build && std::env::var("PLEXI_RUNNING").as_deref() == Ok("1") {
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

#[cfg(not(target_arch = "wasm32"))]
fn print_main_help(embedded_selected: bool) {
    println!("Plexi");
    println!();
    println!("Usage:");
    println!("  plexi [--embedded] [run|secret|app|<path>]");
    println!();
    println!("Flags:");
    println!("  --embedded    Read PGAP JSON lines from stdin and emit DrawCommand JSON");
    println!("  -h, --help    Show this help");
    if embedded_selected {
        println!();
        println!("Embedded mode is available.");
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run_embedded_stdio() {
    use crate::app_protocol::{DrawCommand, PlexiEvent};
    use std::io::BufRead;

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    let started_at = std::time::Instant::now();
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| ".".to_string());

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(err) => {
                let _ = write_embedded_draw(
                    &mut stdout,
                    &DrawCommand::Log {
                        level: "warn".to_string(),
                        message: format!("embedded stdin read error: {err}"),
                    },
                );
                break;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        let event: PlexiEvent = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(err) => {
                let _ = write_embedded_draw(
                    &mut stdout,
                    &DrawCommand::Log {
                        level: "warn".to_string(),
                        message: format!("embedded parse error: {err}"),
                    },
                );
                continue;
            }
        };

        match &event {
            PlexiEvent::Init { .. } | PlexiEvent::Render { .. } => {
                let elapsed = started_at.elapsed().as_secs_f64();
                let now_ms = chrono::Utc::now().timestamp_millis();
                let summary = build_embedded_status_summary(&event, elapsed, now_ms, &cwd);
                if write_embedded_draw(&mut stdout, &summary).is_err() {
                    break;
                }
                // FrameDone terminates the frame so the host's DrawCommand buffer commits.
                if write_embedded_draw(&mut stdout, &DrawCommand::FrameDone).is_err() {
                    break;
                }
            }
            PlexiEvent::Suspend => {
                let _ = write_embedded_draw(
                    &mut stdout,
                    &DrawCommand::Log {
                        level: "info".to_string(),
                        message: "embedded suspended".to_string(),
                    },
                );
            }
            PlexiEvent::Resume => {
                let _ = write_embedded_draw(
                    &mut stdout,
                    &DrawCommand::Log {
                        level: "info".to_string(),
                        message: "embedded resumed".to_string(),
                    },
                );
            }
            PlexiEvent::Shutdown => {
                let _ = write_embedded_draw(
                    &mut stdout,
                    &DrawCommand::Log {
                        level: "info".to_string(),
                        message: "embedded shutdown".to_string(),
                    },
                );
                break;
            }
            _ => {}
        }
    }
}

/// Build a `StatusSummary` DrawCommand for an Init or Render event in embedded mode.
///
/// The summary lets the host display a lightweight status preview of this embedded
/// instance without a full render. Only call this for `Init` and `Render` events —
/// other event types are handled directly in `run_embedded_stdio`.
#[cfg(not(target_arch = "wasm32"))]
fn build_embedded_status_summary(
    event: &crate::app_protocol::PlexiEvent,
    uptime_seconds: f64,
    now_ms: i64,
    cwd: &str,
) -> crate::app_protocol::DrawCommand {
    use crate::app_protocol::{DrawCommand, Health, PaneSummary, StatusSummary};

    let (summary_text, status_text) = match event {
        crate::app_protocol::PlexiEvent::Init {
            width,
            height,
            pixels_per_point,
            protocol_version,
            ..
        } => (
            format!(
                "embedded init {width:.0}x{height:.0} @ {pixels_per_point:.2} (protocol v{protocol_version})"
            ),
            Some("ready".to_string()),
        ),
        crate::app_protocol::PlexiEvent::Render {
            width,
            height,
            delta_time,
            mode,
        } => (
            format!("embedded render {width:.0}x{height:.0} dt={delta_time:.3} mode={mode:?}"),
            Some("rendering".to_string()),
        ),
        _ => unreachable!("build_embedded_status_summary only called for Init/Render"),
    };

    DrawCommand::StatusSummary {
        summary: StatusSummary {
            uptime_seconds,
            process_count: 1,
            last_activity_unix_ms: Some(now_ms),
            summary_text: Some(summary_text),
            health: Health::Running,
            panes: vec![PaneSummary {
                pane_id: 0,
                cwd: cwd.to_string(),
                status_text,
                last_activity_unix_ms: Some(now_ms),
                health: Health::Running,
            }],
        },
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn write_embedded_draw(
    stdout: &mut std::io::StdoutLock<'_>,
    command: &crate::app_protocol::DrawCommand,
) -> std::io::Result<()> {
    use std::io::Write;

    serde_json::to_writer(&mut *stdout, command).map_err(std::io::Error::other)?;
    stdout.write_all(b"\n")?;
    stdout.flush()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_init_and_render_produce_status_summary() {
        use crate::app_protocol::{DrawCommand, Health, PlexiEvent, RenderMode, StatusSummary};

        let init_event = PlexiEvent::Init {
            width: 800.0,
            height: 600.0,
            pixels_per_point: 2.0,
            protocol_version: 2,
            capability_manifest: None,
        };
        let init =
            build_embedded_status_summary(&init_event, 0.25, 1_725_000_000_000, "/tmp/project");
        match init {
            DrawCommand::StatusSummary {
                summary: StatusSummary {
                    process_count,
                    health,
                    ref summary_text,
                    ..
                },
            } => {
                assert_eq!(process_count, 1);
                assert_eq!(health, Health::Running);
                assert!(summary_text
                    .as_deref()
                    .unwrap_or("")
                    .contains("protocol v2"));
            }
            _ => panic!("expected StatusSummary for Init"),
        }

        let render_event = PlexiEvent::Render {
            width: 1024.0,
            height: 768.0,
            delta_time: 0.016,
            mode: RenderMode::Preview,
        };
        let render = build_embedded_status_summary(
            &render_event,
            0.5,
            1_725_000_000_500,
            "/tmp/project",
        );
        match render {
            DrawCommand::StatusSummary {
                summary: StatusSummary {
                    ref summary_text, ..
                },
            } => {
                let text = summary_text.as_deref().unwrap_or("");
                assert!(text.contains("1024x768"), "got: {text}");
                assert!(text.contains("Preview"), "got: {text}");
            }
            _ => panic!("expected StatusSummary for Render"),
        }
    }

    /// Verify that the embedded frame sequence (StatusSummary + FrameDone) serialises
    /// to valid JSON so the host's DrawCommand buffer can commit the frame correctly.
    #[test]
    fn embedded_frame_sequence_serialises_correctly() {
        use crate::app_protocol::{DrawCommand, PlexiEvent, RenderMode};
        use std::io::Write;

        let render_event = PlexiEvent::Render {
            width: 640.0,
            height: 480.0,
            delta_time: 0.016,
            mode: RenderMode::Full,
        };
        let summary =
            build_embedded_status_summary(&render_event, 1.0, 1_725_000_000_000, "/tmp");

        let mut buf = Vec::<u8>::new();
        serde_json::to_writer(&mut buf, &summary).unwrap();
        buf.write_all(b"\n").unwrap();
        serde_json::to_writer(&mut buf, &DrawCommand::FrameDone).unwrap();
        buf.write_all(b"\n").unwrap();

        let output = String::from_utf8(buf).unwrap();
        let mut lines = output.lines();

        let first_line = lines.next().expect("first line");
        let first: DrawCommand = serde_json::from_str(first_line).expect("parse first");
        assert!(
            matches!(first, DrawCommand::StatusSummary { .. }),
            "expected StatusSummary, got: {first_line}"
        );

        let second_line = lines.next().expect("second line");
        let second: DrawCommand = serde_json::from_str(second_line).expect("parse second");
        assert!(
            matches!(second, DrawCommand::FrameDone),
            "expected FrameDone, got: {second_line}"
        );
    }
}
