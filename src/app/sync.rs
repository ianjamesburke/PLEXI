//! CWD synchronization + global pane context snapshot.

use super::PlexiApp;
use crate::host::shell;
use crate::plexi_ai::broker::PaneContext;

impl PlexiApp {
    /// Synchronize view panes that join the `cwd` pane group with any terminal's
    /// working directory. Prefers the focused terminal; falls back to any terminal
    /// in the active context, so the sync still happens when an view pane is
    /// the egui-focused tile. Synchronization is a direct
    /// `workspace_root` reassignment plus `ViewRuntime::sync_cwd`; no event is
    /// emitted, and `sync_cwd` is a no-op for Python and WASM panes.
    pub(super) fn sync_plugin_cwd(&mut self) {
        self.sync_file_browser_cwd();
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

        let plugin_ids: Vec<_> = self.windows[active]
            .panes
            .iter()
            .filter_map(|(&id, pane)| {
                let app = pane.as_view()?;
                if app.pane_group.as_deref() == Some("cwd") {
                    Some(id)
                } else {
                    None
                }
            })
            .collect();

        for id in plugin_ids {
            if let Some(pane) = self.windows[active].panes.get_mut(&id) {
                if let Some(app) = pane.as_view_mut() {
                    if app.workspace_root != new_cwd {
                        app.workspace_root = new_cwd.clone();
                        app.runtime.sync_cwd(&new_cwd);
                    }
                }
            }
        }
    }

    /// Explorers follow their bound terminal, never whichever terminal happens
    /// to be focused. Missing/dead owners leave navigation unchanged. Restored
    /// Explorers have no persisted caller link, so they retain saved navigation.
    fn sync_file_browser_cwd(&mut self) {
        use crate::host::pane::{Pane, TerminalPane};

        fn underlying_terminal(mut pane: &Pane) -> Option<&TerminalPane> {
            loop {
                if let Pane::Terminal(terminal) = pane {
                    return Some(terminal);
                }
                pane = pane.as_view()?.overlay_replaced.as_deref()?;
            }
        }

        let active = self.active_window;
        let context_id = self.windows[active].context_id;
        let updates: Vec<_> = self.windows[active]
            .panes
            .iter()
            .filter_map(|(&id, pane)| {
                let app = pane.as_view()?;
                if app.pane_group.as_deref()
                    != Some(crate::pane_ops::create::FILE_BROWSER_CWD_GROUP)
                {
                    return None;
                }
                let terminal = if let Some(owner) = app.linked_pane_id {
                    self.windows
                        .iter()
                        .filter(|window| window.context_id == context_id)
                        .find_map(|window| window.panes.get(&owner))
                        .and_then(underlying_terminal)?
                } else {
                    // An overlay launched directly from a terminal owns that terminal.
                    // An overlay launched from an plugin has no invoking terminal.
                    app.overlay_replaced.as_deref()?.as_terminal()?
                };
                if terminal.exited || terminal.standby_session.is_some() {
                    return None;
                }
                let cwd = shell::get_pid_cwd(terminal.backend.child_pid())?;
                (app.workspace_root != cwd).then_some((id, cwd))
            })
            .collect();
        for (id, cwd) in updates {
            if let Some(app) = self.windows[active]
                .panes
                .get_mut(&id)
                .and_then(Pane::as_view_mut)
            {
                log::info!(
                    "file_browser: linked terminal cwd changed pane={id} cwd={}",
                    cwd.display()
                );
                app.workspace_root = cwd.clone();
                app.runtime.sync_cwd(&cwd);
            }
        }
    }

    /// Build a snapshot of all open panes across all windows and push it into
    /// the global pane context used by the AI broker (#396). Skips the rebuild
    /// when the total pane count hasn't changed since the last push.
    pub(super) fn update_pane_context_snapshot(&mut self) {
        let current_len: usize = self.windows.iter().map(|w| w.panes.len()).sum();
        if current_len == self.pane_snapshot_len {
            return;
        }
        self.pane_snapshot_len = current_len;

        let mut panes = Vec::with_capacity(current_len);
        for window in &self.windows {
            for pane in window.panes.values() {
                if let Some(app) = pane.as_view() {
                    panes.push(PaneContext {
                        type_id: app.manifest_id.clone(),
                        pane_id: app.id,
                    });
                } else if let Some(term) = pane.as_terminal() {
                    panes.push(PaneContext {
                        type_id: "terminal".to_string(),
                        pane_id: term.id,
                    });
                }
            }
        }
        crate::plexi_ai::broker::update_pane_snapshot(panes);
    }
}
