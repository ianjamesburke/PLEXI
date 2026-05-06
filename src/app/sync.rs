//! CWD synchronization + global pane context snapshot.

use super::PlexiApp;
use crate::plexi_ai::broker::PaneContext;
use crate::shell;
use egui_tiles::Tile;

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
            for (_, pane) in &window.panes {
                if let Some(app) = pane.as_app() {
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

    /// On every frame, if the focused pane changed and is a terminal, walk its CWD
    /// up the directory tree looking for a matching context root. If found and not
    /// already active, switch to it automatically.
    ///
    /// Skips when `skip_auto_switch_frames > 0` — set by manual context switches
    /// to prevent immediately overriding the user's intent.
    pub(super) fn check_context_auto_switch(&mut self) {
        if self.skip_auto_switch_frames > 0 {
            self.skip_auto_switch_frames -= 1;
            return;
        }

        let active = self.active_window;
        let Some(focused_tile) = self.windows[active].focused_pane else {
            self.last_focused_tile_for_auto_switch = None;
            return;
        };

        if self.last_focused_tile_for_auto_switch == Some(focused_tile) {
            return;
        }
        self.last_focused_tile_for_auto_switch = Some(focused_tile);

        let pane_id = match self.windows[active].tree.tiles.get(focused_tile) {
            Some(Tile::Pane(id)) => *id,
            _ => return,
        };

        let cwd = {
            let Some(terminal) = self.windows[active]
                .panes
                .get(&pane_id)
                .and_then(|p| p.as_terminal())
            else {
                return;
            };
            shell::get_pid_cwd(terminal.backend.child_pid())
        };
        let Some(cwd) = cwd else { return; };

        let current_ctx_idx = self.router.active_idx();
        let mut path = cwd.as_path();
        loop {
            if let Some(idx) = self.router.position(|ctx| ctx.root.as_deref() == Some(path)) {
                if idx != current_ctx_idx {
                    log::info!(
                        "context:auto-switch: cwd={} → context_id={}",
                        cwd.display(),
                        self.router.get(idx).context_id
                    );
                    self.switch_workspace(idx);
                }
                break;
            }
            match path.parent() {
                Some(parent) if parent != path => path = parent,
                _ => break,
            }
        }
    }
}
