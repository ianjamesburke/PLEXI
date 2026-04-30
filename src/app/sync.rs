//! CWD synchronization — polls linked terminals and broadcasts PathChanged to pane groups.

use super::PlexiApp;
use crate::shell;

impl PlexiApp {
    /// Synchronize app panes that join the `cwd` pane group with any terminal's
    /// working directory. Prefers the focused terminal; falls back to any terminal
    /// in the active context. This way PathChanged fires even when an app pane
    /// (like the file browser) is the egui-focused tile.
    pub(super) fn sync_app_cwd(&mut self) {
        let active = self.active_window;

        // First try: focused pane if it's a terminal.
        let terminal_cwd = {
            let ctx = &self.windows[active];
            let focused_terminal_cwd = ctx.focused_pane.and_then(|tile| {
                let pane_id = match ctx.tree.tiles.get(tile)? {
                    egui_tiles::Tile::Pane(id) => *id,
                    _ => return None,
                };
                ctx.panes
                    .get(&pane_id)?
                    .as_terminal()
                    .and_then(|t| shell::get_pid_cwd(t.backend.child_pid()))
            });
            if focused_terminal_cwd.is_some() {
                focused_terminal_cwd
            } else {
                // Fallback: any terminal in the context.
                ctx.panes.values().find_map(|p| {
                    p.as_terminal()
                        .and_then(|t| shell::get_pid_cwd(t.backend.child_pid()))
                })
            }
        };
        let Some(new_cwd) = terminal_cwd else {
            return;
        };

        let app_ids: Vec<_> = self.windows[active]
            .panes
            .iter()
            .filter_map(|(&id, pane)| {
                let app = pane.as_app()?;
                if app.pane_group.as_deref() == Some("cwd") {
                    Some(id)
                } else {
                    None
                }
            })
            .collect();

        for id in app_ids {
            if let Some(pane) = self.windows[active].panes.get_mut(&id) {
                if let Some(app) = pane.as_app_mut() {
                    if app.workspace_root != new_cwd {
                        app.workspace_root = new_cwd.clone();
                        app.runtime.sync_cwd(&new_cwd);
                    }
                }
            }
        }
    }
}
