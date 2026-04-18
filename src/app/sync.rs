//! CWD synchronization — polls linked terminals and broadcasts PathChanged to pane groups.

use super::PlexiApp;

impl PlexiApp {
    /// Synchronize app panes that join the `cwd` pane group with the currently
    /// focused terminal pane's working directory.
    pub(super) fn sync_app_cwd(&mut self) {
        let active = self.active_context;
        let focused_tile = self.contexts[active].focused_pane;
        let focused_cwd = focused_tile
            .and_then(|tile| self.contexts[active].get_focused_pane_cwd(tile));
        let Some(new_cwd) = focused_cwd else {
            return;
        };

        let app_ids: Vec<_> = self.contexts[active]
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
            if let Some(pane) = self.contexts[active].panes.get_mut(&id) {
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
