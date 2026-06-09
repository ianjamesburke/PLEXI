//! Pane-operation methods on `PlexiApp`, decomposed by concern:
//!
//! - [`create`] — pane / tile / app / agent creation, plus the app-launch
//!   entry points used by the command palette and AppRequest routing.
//! - [`layout`] — splits, tabs, close, navigation, zoom-free tree
//!   manipulation on already-created panes.
//! - [`workspace`] — multi-context management and on-disk workspace
//!   serialization.
//!
//! Each submodule attaches `impl PlexiApp { ... }` blocks. Methods stay on
//! `PlexiApp` regardless of file, so call sites elsewhere in the crate are
//! unchanged.

mod create;
mod layout;
mod workspace;

pub(crate) use layout::insert_split_tile;
pub(crate) use layout::SwapResult;

/// Apply `initial_cmd` to `settings`, using the same shell-suffix injection
/// logic as `split_focused`. Call before `TerminalPane::new`.
pub(super) fn apply_initial_cmd(
    settings: &mut egui_term::BackendSettings,
    cmd: &str,
    close_on_exit: bool,
) {
    let shell_name = std::path::Path::new(&settings.shell)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let effective_cmd: String = if !close_on_exit {
        let shell_path = &settings.shell;
        let trimmed = cmd.trim().trim_end_matches([';', ' ']);
        let sep = if trimmed.is_empty() { "" } else { "; " };
        match shell_name {
            "fish" => format!("{trimmed}{sep}exec \"{shell_path}\" --login -i"),
            _ => format!("{trimmed}{sep}exec \"{shell_path}\" -i -l"),
        }
    } else {
        cmd.to_string()
    };
    settings.args = match shell_name {
        "zsh" | "bash" => vec![
            "-i".to_string(),
            "-l".to_string(),
            "-c".to_string(),
            effective_cmd,
        ],
        "fish" => vec!["--login".to_string(), "-c".to_string(), effective_cmd],
        _ => vec!["-l".to_string(), "-c".to_string(), effective_cmd],
    };
}
