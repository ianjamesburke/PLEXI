//! App command dispatch — keyboard routing from focused pane to linked terminal.

use crate::app_trait::AppCommand;

use super::PlexiApp;

impl PlexiApp {
    /// Feed keyboard input to the focused app pane and dispatch any resulting
    /// AppCommands. Spawn requests are deferred to the outer update loop.
    pub(super) fn dispatch_app_key_events(&mut self, ctx: &egui::Context) -> Vec<AppCommand> {
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

        let commands = {
            let Some(pane) = self.contexts[active].panes.get_mut(&pane_id) else {
                return vec![];
            };
            let Some(app_pane) = pane.as_app_mut() else {
                return vec![];
            };
            ctx.input(|i| {
                app_pane.runtime.handle_key(i);
            });
            app_pane.runtime.take_pending_commands()
        };

        let mut deferred = Vec::new();

        for cmd in commands {
            match cmd {
                AppCommand::SpawnApp { .. }
                | AppCommand::DeliverPipeMessage { .. }
                | AppCommand::DeliverRunUpdate { .. } => deferred.push(cmd),
                // Always stamp the focused pane's ID — builtins don't know their own pane_id.
                AppCommand::CdRequest { cwd, .. } => {
                    deferred.push(AppCommand::CdRequest { cwd, sender_pane_id: pane_id });
                }
                AppCommand::Notify(msg) => {
                    log::info!("app notify: {msg}");
                }
                AppCommand::ShowNotification { .. } => {
                    // No-op until A5 wires the notification panel.
                }
            }
        }

        deferred
    }
}
