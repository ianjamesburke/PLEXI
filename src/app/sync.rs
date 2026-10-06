//! CWD synchronization + global pane context snapshot.

use super::PlexiApp;
use crate::host::shell;
use crate::plexi_ai::broker::PaneContext;

impl PlexiApp {
    /// Synchronize app panes that join the `cwd` pane group with any terminal's
    /// working directory. Prefers the focused terminal; falls back to any terminal
    /// in the active context, so the sync still happens when an app pane (like
    /// the file browser) is the egui-focused tile. Synchronization is a direct
    /// `workspace_root` reassignment plus `AppRuntime::sync_cwd`; no event is
    /// emitted, and `sync_cwd` is a no-op for Python and WASM panes.
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
    /// the global pane context used by the AI broker (#396). Skips the push
    /// when pane ids, types, editor paths, and dirty flags are unchanged.
    pub(super) fn update_pane_context_snapshot(&mut self) {
        let mut panes = Vec::new();
        for window in &self.windows {
            for pane in window.panes.values() {
                if let Some(app) = pane.as_app() {
                    let buffer = app.runtime.editor_buffer();
                    panes.push(PaneContext {
                        type_id: app.manifest_id.clone(),
                        pane_id: app.id,
                        path: buffer.as_ref().map(|buffer| buffer.path.clone()),
                        dirty: buffer.as_ref().is_some_and(|buffer| buffer.dirty),
                    });
                } else if let Some(term) = pane.as_terminal() {
                    panes.push(PaneContext {
                        type_id: "terminal".to_string(),
                        pane_id: term.id,
                        path: None,
                        dirty: false,
                    });
                }
            }
        }
        let fingerprint = pane_snapshot_fingerprint(&panes);
        if fingerprint == self.pane_snapshot_fp {
            return;
        }
        self.pane_snapshot_fp = fingerprint;
        crate::plexi_ai::broker::update_pane_snapshot(panes);
    }
}

fn pane_snapshot_fingerprint(panes: &[PaneContext]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for pane in panes {
        for byte in pane.type_id.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= pane.pane_id;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        if let Some(path) = &pane.path {
            for byte in path.bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        hash ^= u64::from(pane.dirty);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
