/// App command dispatch — keyboard routing from focused pane to linked terminal.

use crate::app_permissions::PermissionCheck;
use crate::app_trait::AppCommand;
use crate::pane::TerminalPane;
use egui_term::BackendCommand;

use super::PlexiApp;

impl PlexiApp {
    /// Feed keyboard input to the focused pane's active app and dispatch any
    /// resulting AppCommands. Returns SpawnApp requests to the caller (they
    /// need host-level access that this method can't provide mid-borrow).
    pub(super) fn dispatch_app_key_events(
        &mut self,
        ctx: &egui::Context,
    ) -> Vec<AppCommand> {
        let active = self.active_context;
        let Some(focused_tile) = self.contexts[active].focused_pane else {
            return vec![];
        };
        let Some(egui_tiles::Tile::Pane(pane_id)) =
            self.contexts[active].tree.tiles.get(focused_tile)
        else {
            return vec![];
        };
        let pane_id = *pane_id;

        let (commands, scope, perms, linked_id) = {
            let Some(pane) = self.contexts[active].panes.get_mut(&pane_id) else {
                return vec![];
            };
            let Some(t) = pane.as_terminal_mut() else { return vec![] };
            if t.active_app.is_none() {
                return vec![];
            }
            let app = t.active_app.as_mut().unwrap();
            ctx.input(|i| { app.handle_key(i); });
            let cmds = app.take_pending_commands();
            let scope = t.app_scope.clone().unwrap_or_else(|| std::path::PathBuf::from("/"));
            let perms = t.app_permissions.clone();
            let linked = t.linked_terminal_pane;
            (cmds, scope, perms, linked)
        };

        let mut deferred = Vec::new();

        for cmd in commands {
            if matches!(cmd, AppCommand::SpawnApp { .. }) {
                deferred.push(cmd);
                continue;
            }

            if let Some(linked_id) = linked_id {
                if let Some(target_pane) = self.contexts[active].panes.get_mut(&linked_id) {
                    if let Some(t) = target_pane.as_terminal_mut() {
                        let check = match &cmd {
                            AppCommand::RunInTerminal(_) => {
                                if perms.is_builtin
                                    || perms.capabilities.contains(&crate::app_permissions::Capability::FsWrite)
                                {
                                    PermissionCheck::Allowed
                                } else {
                                    PermissionCheck::Denied(
                                        "RunInTerminal requires fs.write capability".to_string(),
                                    )
                                }
                            }
                            AppCommand::Cd(path) => {
                                crate::app_permissions::check_cd(path, &perms, &scope)
                            }
                            AppCommand::Notify(_) => PermissionCheck::Allowed,
                            AppCommand::SpawnApp { .. } => unreachable!(),
                        };
                        match check {
                            PermissionCheck::Allowed => execute_app_command(cmd, t),
                            PermissionCheck::Denied(reason) => {
                                log::warn!("App command denied: {reason}");
                            }
                        }
                    }
                }
            }
        }

        deferred
    }
}

/// Execute a single AppCommand against a terminal pane.
pub(super) fn execute_app_command(cmd: AppCommand, pane: &mut TerminalPane) {
    match cmd {
        AppCommand::RunInTerminal(command) => {
            let mut bytes = command.into_bytes();
            bytes.push(b'\n');
            pane.backend.process_command(BackendCommand::Write(bytes));
        }
        AppCommand::Cd(path) => {
            let cmd = format!("clear && cd {}\n", shell_escape(&path.display().to_string()));
            pane.backend.process_command(BackendCommand::Write(cmd.into_bytes()));
        }
        AppCommand::Notify(_msg) => {
            // Notification system not yet wired — no-op for now.
        }
        AppCommand::SpawnApp { .. } => {
            log::error!("execute_app_command: SpawnApp must not reach the terminal router");
        }
    }
}

/// Shell-escape a path string for safe embedding in a shell command.
pub(super) fn shell_escape(s: &str) -> String {
    if s.contains(|c: char| c.is_whitespace() || "\"'\\()&|;$`!#".contains(c)) {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_string()
    }
}
