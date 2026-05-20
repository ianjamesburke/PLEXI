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

pub(crate) use layout::SwapResult;
pub(crate) use layout::insert_split_tile;

/// Apply `initial_cmd` to `settings`, using the same shell-suffix injection
/// logic as `split_focused`. Call before `TerminalPane::new`.
///
/// Branches by shell filename:
/// - **Unix shells** (`zsh`, `bash`, `fish`, generic POSIX): when `close_on_exit`
///   is false, append `exec "<shell>" -i -l` (or `--login -i` for fish) to the
///   command so the shell stays alive after the initial command finishes.
/// - **Windows shells** (`cmd.exe`, `pwsh.exe`, `powershell.exe`): use the
///   shell's native stay-open flags (`/k` for cmd, `-NoExit` for PowerShell) —
///   neither supports the Unix `exec` stay-alive trick, and neither understands
///   `-c` / `-i` / `-l`.
pub(super) fn apply_initial_cmd(
    settings: &mut egui_term::BackendSettings,
    cmd: &str,
    close_on_exit: bool,
) {
    let shell_name_lower = std::path::Path::new(&settings.shell)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    // Windows shells: use native stay-open flags, skip the exec suffix.
    // cmd.exe and pwsh don't understand `-c` / `-i` / `-l`, and they have no
    // analog of the Unix `exec <shell>` stay-alive trick.
    match shell_name_lower.as_str() {
        "cmd.exe" => {
            let trimmed = cmd.trim().trim_end_matches([';', ' ']).to_string();
            settings.args = if close_on_exit {
                vec!["/c".to_string(), trimmed]
            } else {
                vec!["/k".to_string(), trimmed]
            };
            return;
        }
        "pwsh.exe" | "powershell.exe" => {
            let trimmed = cmd.trim().trim_end_matches([';', ' ']).to_string();
            settings.args = if close_on_exit {
                vec!["-Command".to_string(), trimmed]
            } else {
                vec![
                    "-NoExit".to_string(),
                    "-Command".to_string(),
                    trimmed,
                ]
            };
            return;
        }
        _ => {}
    }

    // Unix shells: append `exec <shell>` stay-alive suffix when keeping the
    // shell open after the initial command.
    let effective_cmd: String = if !close_on_exit {
        let shell_path = &settings.shell;
        let trimmed = cmd.trim().trim_end_matches([';', ' ']);
        let sep = if trimmed.is_empty() { "" } else { "; " };
        match shell_name_lower.as_str() {
            "fish" => format!("{trimmed}{sep}exec \"{shell_path}\" --login -i"),
            _ => format!("{trimmed}{sep}exec \"{shell_path}\" -i -l"),
        }
    } else {
        cmd.to_string()
    };
    settings.args = match shell_name_lower.as_str() {
        "zsh" | "bash" => vec!["-i".to_string(), "-l".to_string(), "-c".to_string(), effective_cmd],
        "fish" => vec!["--login".to_string(), "-c".to_string(), effective_cmd],
        _ => vec!["-l".to_string(), "-c".to_string(), effective_cmd],
    };
}
