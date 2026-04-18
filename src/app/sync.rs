//! CWD synchronization — polls linked terminals and broadcasts PathChanged to pane groups.

use crate::tiling::PaneId;
use std::path::PathBuf;

use super::PlexiApp;

impl PlexiApp {
    /// Poll each active app's linked terminal for CWD changes. When a linked
    /// terminal moves, push the new CWD to the owning app AND broadcast
    /// `PathChanged` to every other active app in the same pane group.
    pub(super) fn sync_app_cwd(&mut self) {
        let ctx = &mut self.contexts[self.active_context];

        // 1) Snapshot current (pane_id, linked_id, group, last_cwd) for every
        //    pane with an active app + linked terminal.
        let snapshot: Vec<(PaneId, PaneId, Option<String>, Option<PathBuf>)> = ctx
            .panes
            .iter()
            .filter_map(|(&pane_id, pane)| {
                let t = pane.as_terminal()?;
                let linked = t.linked_terminal_pane?;
                if t.active_app.is_some() {
                    Some((pane_id, linked, t.pane_group.clone(), t.last_synced_cwd.clone()))
                } else {
                    None
                }
            })
            .collect();

        // 2) Poll each linked terminal's CWD via lsof and collect only changes.
        let mut changes: Vec<(PaneId, Option<String>, PathBuf)> = Vec::new();
        for (app_pane_id, linked_id, group, last_cwd) in snapshot {
            let cwd = ctx.panes.get(&linked_id).and_then(|linked_pane| {
                let t = linked_pane.as_terminal()?;
                crate::shell::get_pid_cwd(t.backend.child_pid())
            });
            let Some(cwd) = cwd else { continue };
            let changed = last_cwd.as_ref().map(|prev| prev != &cwd).unwrap_or(true);
            if changed {
                changes.push((app_pane_id, group, cwd));
            }
        }

        // 3) For each change, push to the owning app and broadcast to every
        //    other active app in the same pane group.
        for (source_pane_id, group, cwd) in &changes {
            if let Some(pane) = ctx.panes.get_mut(source_pane_id) {
                if let Some(t) = pane.as_terminal_mut() {
                    t.last_synced_cwd = Some(cwd.clone());
                    if let Some(app) = t.active_app.as_mut() {
                        app.sync_cwd(cwd);
                    }
                }
            }

            let Some(group) = group else { continue };

            let targets: Vec<PaneId> = ctx
                .panes
                .iter()
                .filter_map(|(&pid, p)| {
                    if pid == *source_pane_id { return None; }
                    let t = p.as_terminal()?;
                    t.active_app.as_ref()?;
                    if t.pane_group.as_deref() == Some(group.as_str()) {
                        Some(pid)
                    } else {
                        None
                    }
                })
                .collect();

            for target_id in targets {
                if let Some(pane) = ctx.panes.get_mut(&target_id) {
                    if let Some(t) = pane.as_terminal_mut() {
                        if let Some(app) = t.active_app.as_mut() {
                            app.sync_cwd(cwd);
                        }
                    }
                }
            }
        }
    }
}
