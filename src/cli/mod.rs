use serde::Deserialize;
use std::collections::HashMap;

pub(super) const APP_ID: &str = "plexi-run";
pub(super) const COMMANDS_FILE: &str = ".plexi/commands.toml";


/// Parsed .plexi/commands.toml
#[derive(Deserialize)]
pub struct PlexiCommands {
    #[serde(default)]
    pub secrets: SecretsConfig,
    #[serde(default)]
    pub commands: HashMap<String, CommandEntry>,
}

#[derive(Deserialize, Default)]
pub struct SecretsConfig {
    #[serde(default)]
    pub required: Vec<String>,
}

/// A command entry: either a bare string (`build = "cargo build"`) or an inline table
/// (`build = { run = "cargo build", description = "..." }`).
/// The old nested-section form (`[commands.build]\nrun = "..."`) is TOML-equivalent to the
/// inline-table form and parses identically — no migration needed.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum CommandEntry {
    Simple(String),
    Full(CommandDef),
}

impl CommandEntry {
    pub fn run(&self) -> &str {
        match self {
            CommandEntry::Simple(s) => s,
            CommandEntry::Full(d) => &d.run,
        }
    }

    pub fn description(&self) -> Option<&str> {
        match self {
            CommandEntry::Simple(_) => None,
            CommandEntry::Full(d) => d.description.as_deref(),
        }
    }

    pub fn secrets(&self) -> &[String] {
        match self {
            CommandEntry::Simple(_) => &[],
            CommandEntry::Full(d) => &d.secrets,
        }
    }
}

#[derive(Deserialize)]
pub struct CommandDef {
    pub run: String,
    pub description: Option<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
}

/// List executable files in a scripts directory.
pub(super) fn list_global_scripts(scripts_dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(scripts_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let path = e.path();
            path.is_file() && is_executable(&path)
        })
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

pub(super) fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.exists()
    }
}

pub mod args;
pub mod crawl;
pub mod help_parser;
pub mod registry;
pub mod setup;

pub mod agent;
pub mod app;
pub mod completions;
pub mod config_cli;
pub mod context_cli;
pub mod demo;
pub mod descriptor;
pub mod doctor;
pub mod install;
pub mod list;
pub mod notes;
pub mod notify;
pub mod open;
pub mod pane;
pub mod registry_watch;
pub mod routine;
pub mod run;
pub mod validate;
pub mod workspace;

pub(super) fn print_tip(msg: &str) {
    let config = crate::config::PlexiConfig::load_with_workspace(
        crate::config::active_workspace_root().as_deref(),
    );
    let enabled = config.cli.as_ref().and_then(|c| c.tips).unwrap_or(true);
    if enabled {
        log::info!("cli:tip: {msg}");
        if std::env::var_os("NO_COLOR").is_none() {
            eprintln!("\x1b[2mtip: {msg}\x1b[0m");
        } else {
            eprintln!("tip: {msg}");
        }
    }
}

pub(super) fn send_to_socket(payload: serde_json::Value) -> i32 {
    let socket_path = match std::env::var("PLEXI_SOCKET") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: PLEXI_SOCKET is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let mut stream = match UnixStream::connect(&socket_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: could not connect to PLEXI_SOCKET {socket_path:?}: {e}");
            return 1;
        }
    };
    let line = format!("{}\n", payload);
    if let Err(e) = stream.write_all(line.as_bytes()) {
        eprintln!("error: could not write to socket: {e}");
        return 1;
    }
    0
}

pub(super) fn binary_in_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

// ── Public re-exports (preserve crate::cli::fn_name() call sites in main.rs) ──
pub use app::{app_init, app_uninstall, app_install_with_pin, app_run, app_info, app_list, app_render, app_dev, app_publish, app_update_cli};
pub use completions::completions_cli;
pub use config_cli::{config_check, config_edit, config_get, config_reset};
pub use context_cli::{
    context_new_cli, context_zoom_cli, context_zoom_out_cli, context_open_cli,
    context_set_root_cli, context_current_cli, context_describe_cli, context_push_cli,
};
pub use demo::demo_cli;
pub use doctor::doctor_cli;
pub use install::{install_cli, install_pack_cli, install_workspace_pack_cli, plexi_uninstall_cli, update_cli, self_update_cli};
pub use list::{freeze_cli, parse_notify_choice};
pub use notes::{notes_list_cli, notes_open_cli};
pub use notify::notify_cli;
pub use open::{open_cli, terminal_cli, pane_new_cli, mcp_pane_title};
pub use pane::{
    pane_set_title_cli, pane_list_cli, pane_self_cli, pane_info_cli,
    pane_focus_cli, pane_close_cli, pane_send_cli, pane_key_cli, pane_capture_cli,
    pane_state_cli, pane_move_cli,
};
pub use routine::{routine_list, routine_run};
pub use run::{run_list_commands, run_command};
pub use validate::validate_cli;
pub use workspace::{workspace_init, workspace_secret_set, workspace_secret_get, workspace_secret_list, workspace_secret_delete};
pub use agent::{agent_add, agent_update, agent_list};
