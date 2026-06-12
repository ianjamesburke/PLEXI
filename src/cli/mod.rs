use serde::Deserialize;
use std::collections::HashMap;

pub(super) const APP_ID: &str = "plexi-run";
pub(super) fn commands_file() -> String {
    format!("{}/commands.toml", crate::config::workspace_channel_dir())
}

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
pub mod ai;
pub mod app;
pub mod app_check;
pub mod completions;
pub mod config_cli;
pub mod context_cli;
pub mod demo;
pub mod descriptor;
pub mod doctor;
pub mod install;
pub mod install_host;
pub mod list;
pub mod notes;
pub mod notify;
pub mod open;
pub mod pane;
pub mod registry_watch;
pub mod routine;
pub mod run;
pub mod updater;
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
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            let _ = std::fs::remove_file(&socket_path);
            eprintln!("error: Plexi is not responding (stale socket removed). Is Plexi running?");
            return 1;
        }
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

/// Best-effort wake of a running instance after a spawn-queue file write.
///
/// A fully idle Plexi produces zero frames and only drains the spawn-queue
/// during a frame, so without this nudge an outside-shell `plexi terminal`
/// hangs until the user touches the window. Connects to the channel
/// profile's `notify.sock` and sends a no-op `AppRequest::Wake`; the socket
/// listener requests a repaint, the resulting frame drains the queue.
///
/// All errors are ignored silently and intentionally: no instance running
/// means the queue is drained at next startup — the designed fallback.
pub(super) fn nudge_running_instance() {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let path = crate::config::config_dir().join("notify.sock");
    let Ok(mut stream) = UnixStream::connect(&path) else {
        return;
    };
    let Ok(payload) = serde_json::to_string(&crate::app_protocol::AppRequest::Wake) else {
        return;
    };
    if stream.write_all(format!("{payload}\n").as_bytes()).is_ok() {
        log::debug!("cli: sent wake nudge to running instance at {path:?}");
    }
}

pub(super) fn binary_in_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

// ── Public re-exports (preserve crate::cli::fn_name() call sites in main.rs) ──
pub use agent::{
    agent_add, agent_hook_install_cli, agent_hook_uninstall_cli, agent_init, agent_list,
    agent_report_cli, agent_status_cli, agent_update,
};
pub use ai::{ai_doctor_cli, ai_setup_cli};
pub use app::{
    app_action_cli, app_info, app_init, app_inspect_cli, app_install_package, app_install_with_pin,
    app_list, app_package_cli, app_publish, app_render, app_uninstall, app_update_cli,
    InstallConfirm,
};
pub use app_check::app_check_cli;
pub use completions::{complete_open_cli, completions_cli};
pub use config_cli::{config_check, config_edit, config_get, config_reset};
pub use context_cli::{
    context_current_cli, context_describe_cli, context_list_cli, context_new_cli, context_open_cli,
    context_push_cli, context_set_root_cli, context_zoom_cli, context_zoom_out_cli,
};
pub use demo::demo_cli;
pub use doctor::doctor_cli;
pub use install::{
    install_cli, install_pack_cli, install_workspace_pack_cli, plexi_uninstall_cli,
    self_update_cli, update_cli,
};
pub use list::{freeze_cli, parse_notify_choice};
pub use notes::{notes_list_cli, notes_open_cli};
pub use notify::notify_cli;
pub use open::{
    mcp_pane_title, open_cli, open_cli_by_name, open_mcp_by_name, pane_new_cli, parse_prefix,
    OpenPrefix,
};
pub use pane::{
    pane_capture_cli, pane_close_cli, pane_focus_cli, pane_info_cli, pane_key_cli, pane_list_cli,
    pane_self_cli, pane_send_cli, pane_set_title_cli, pane_slot_delete_cli, pane_slot_list_cli,
    pane_slot_read_cli, pane_slot_write_cli, pane_state_cli,
};
pub use routine::{routine_list, routine_run};
pub use run::{run_command, run_list_commands};
pub use validate::validate_cli;
pub use workspace::{
    workspace_clean_cli, workspace_init, workspace_secret_delete, workspace_secret_get,
    workspace_secret_list, workspace_secret_set,
};
