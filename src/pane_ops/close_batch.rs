//! Close every selected pane (stint 1458). The batch lives in
//! [`crate::host::model::PaneCloseBatch`]; this file drives it: each pane goes
//! through its own close veto and dialog, in selection order, and closes
//! through `close_pane_by_id`, the same route as a single close.

use std::collections::HashSet;

use crate::app::PlexiApp;
use crate::host::model::{CloseBatchWait, PaneCloseBatch, PaneTargetSource, PaneTargets};
use crate::spatial::tiling::PaneId;

/// What a paused batch learned from its pane's dialog this pass.
enum WaitOutcome {
    Pending,
    /// The dialog allowed the close; the batch closes the pane.
    Close,
    /// The pane is already gone (its own dialog closed it).
    Gone,
    Cancelled,
}

impl PlexiApp {
    /// The panes `action` applies to in the active window: the selection, else
    /// the focused pane. See [`crate::host::model::HostModel::resolve_pane_targets`].
    pub(crate) fn resolve_pane_targets(&self, action: &str) -> Option<PaneTargets> {
        let window = &self.windows[self.active_window];
        self.host
            .resolve_pane_targets(action, window.window_id, self.focused_pane_id(), |id| {
                window.panes.contains_key(&id) && window.tree.tiles.find_pane(&id).is_some()
            })
    }

    /// Start closing the active window's selection when it holds two or more
    /// panes. Returns false, changing nothing, when the close shortcut should
    /// act on the focused pane as before.
    pub(crate) fn start_close_selected_panes(&mut self, action: &str) -> bool {
        if self.host.pane_close_batch().is_some() {
            log::info!("close_batch: {action} ignored, a batch is already waiting on a dialog");
            return true;
        }
        let Some(targets) = self
            .resolve_pane_targets(action)
            .filter(|t| t.source == PaneTargetSource::Selection && t.panes.len() >= 2)
        else {
            return false;
        };
        let window = &self.windows[self.active_window];
        let closing: HashSet<PaneId> = targets.panes.iter().copied().collect();
        let focus_after = window
            .tree
            .tiles
            .find_pane(&targets.anchor)
            .and_then(|tile| self.choose_focus_after_closing(self.active_window, tile, &closing))
            .and_then(|(tile, rule)| {
                let pane = match window.tree.tiles.get(tile) {
                    Some(egui_tiles::Tile::Pane(pane)) => *pane,
                    _ => return None,
                };
                log::info!(
                    "close_batch: focus after rule={} -> pane {pane}",
                    rule.as_str()
                );
                Some(pane)
            });
        log::info!(
            "close_batch: start action={action} window={} anchor={} panes={:?}",
            window.window_id,
            targets.anchor,
            targets.panes
        );
        self.host.start_pane_close_batch(PaneCloseBatch {
            window: window.window_id,
            anchor: targets.anchor,
            queue: targets.panes.into(),
            waiting: None,
            focus_after,
        });
        self.service_pane_close_batch();
        true
    }

    /// Advance the batch until it finishes or pauses on a pane's dialog. Runs
    /// at start and before the frame's self-close poll, which is where the
    /// dialogs it waits on resolve.
    pub(crate) fn service_pane_close_batch(&mut self) {
        while let Some(batch) = self.host.pane_close_batch() {
            let window_id = batch.window;
            let waiting = batch.waiting;
            let Some(win_idx) = self.windows.iter().position(|w| w.window_id == window_id) else {
                log::info!("close_batch: end window={window_id} gone");
                self.host.take_pane_close_batch();
                return;
            };
            if let Some(wait) = waiting {
                let pane = wait.pane();
                match self.close_batch_wait_outcome(win_idx, wait) {
                    WaitOutcome::Pending => return,
                    WaitOutcome::Cancelled => {
                        self.cancel_pane_close_batch(pane);
                        return;
                    }
                    WaitOutcome::Gone => {
                        log::info!("close_batch: pane {pane} closed by its dialog");
                    }
                    WaitOutcome::Close => {
                        log::info!("close_batch: pane {pane} closed after its dialog");
                        self.close_pane_by_id(pane);
                    }
                }
                if let Some(batch) = self.host.pane_close_batch_mut() {
                    batch.waiting = None;
                }
                continue;
            }
            let Some(pane) = self
                .host
                .pane_close_batch_mut()
                .and_then(|b| b.queue.pop_front())
            else {
                self.finish_pane_close_batch();
                return;
            };
            if let Some(wait) = self.close_batch_ask(win_idx, pane) {
                if let Some(batch) = self.host.pane_close_batch_mut() {
                    batch.waiting = Some(wait);
                }
                log::info!("close_batch: pane {pane} asks first ({wait:?})");
                return;
            }
        }
    }

    /// Run `pane` through its close veto. Returns what to wait for when it
    /// asks; otherwise closes it (or drops it when it is already gone).
    fn close_batch_ask(&mut self, win_idx: usize, pane: PaneId) -> Option<CloseBatchWait> {
        let window = &self.windows[win_idx];
        let Some(tile) = window.tree.tiles.find_pane(&pane) else {
            log::info!("close_batch: pane {pane} already gone, skipped");
            return None;
        };
        let Some(entry) = window.panes.get(&pane) else {
            log::info!("close_batch: pane {pane} already gone, skipped");
            return None;
        };
        // A portal closes like a single portal close: its subcontext goes too,
        // asking through the context-close dialog when it holds panes.
        if let Some(child) = entry.portal_target() {
            let state = self.build_context_close_state(child);
            if state.items.is_empty() {
                if let Some(index) = self.router.position(|c| c.context_id == child) {
                    log::info!("close_batch: pane {pane} portal to empty ctx={child} closed");
                    self.delete_context(index);
                    self.mark_workspace_dirty();
                }
                return None;
            }
            self.pending_context_close = Some(state);
            return Some(CloseBatchWait::ContextClose(pane));
        }
        let vetoed = self.windows[win_idx]
            .panes
            .get_mut(&pane)
            .and_then(|p| p.as_view_mut())
            .is_some_and(|view| !view.runtime.request_close());
        if vetoed {
            self.focus_close_batch_pane(win_idx, tile);
            return Some(CloseBatchWait::Unsaved(pane));
        }
        if self.confirm_close() {
            self.focus_close_batch_pane(win_idx, tile);
            self.pending_close = true;
            return Some(CloseBatchWait::ConfirmClose {
                pane,
                confirmed: false,
            });
        }
        log::info!("close_batch: pane {pane} closed");
        self.close_pane_by_id(pane);
        self.mark_workspace_dirty();
        None
    }

    /// A pane's dialog renders for the focused pane, so the asking pane takes focus.
    fn focus_close_batch_pane(&mut self, win_idx: usize, tile: egui_tiles::TileId) {
        let window = &mut self.windows[win_idx];
        window.clear_zoom();
        window.navigate_to(tile);
    }

    fn close_batch_wait_outcome(&self, win_idx: usize, wait: CloseBatchWait) -> WaitOutcome {
        let window = &self.windows[win_idx];
        let pane = wait.pane();
        if !window.panes.contains_key(&pane) {
            return WaitOutcome::Gone;
        }
        match wait {
            CloseBatchWait::Unsaved(_) => {
                let view = window.panes.get(&pane).and_then(|p| p.as_view());
                if view.is_some_and(|v| v.runtime.wants_close()) {
                    WaitOutcome::Close
                } else if view.is_some_and(|v| match &v.runtime {
                    crate::host::pane::ViewRuntime::Builtin(app) => {
                        app.unsaved_changes_dialog().is_some()
                    }
                    _ => false,
                }) {
                    WaitOutcome::Pending
                } else {
                    WaitOutcome::Cancelled
                }
            }
            CloseBatchWait::ConfirmClose {
                confirmed: true, ..
            } => WaitOutcome::Close,
            CloseBatchWait::ConfirmClose { .. } if self.pending_close => WaitOutcome::Pending,
            CloseBatchWait::ContextClose(_) if self.pending_context_close.is_some() => {
                WaitOutcome::Pending
            }
            CloseBatchWait::ConfirmClose { .. } | CloseBatchWait::ContextClose(_) => {
                WaitOutcome::Cancelled
            }
        }
    }

    /// The confirm-close dialog's Close button. True when a batch owns the
    /// dialog; the batch then closes the pane itself.
    pub(crate) fn confirm_close_batch_pane(&mut self) -> bool {
        match self
            .host
            .pane_close_batch_mut()
            .and_then(|b| b.waiting.as_mut())
        {
            Some(CloseBatchWait::ConfirmClose { confirmed, .. }) => {
                *confirmed = true;
                self.service_pane_close_batch();
                true
            }
            _ => false,
        }
    }

    /// Cancel stops the batch; the panes it did not close stay selected.
    fn cancel_pane_close_batch(&mut self, pane: PaneId) {
        let Some(batch) = self.host.take_pane_close_batch() else {
            return;
        };
        let remaining = batch.remaining();
        log::info!("close_batch: pane {pane} cancelled; end, left open and selected {remaining:?}");
        self.host
            .set_pane_selection(batch.window, batch.anchor, remaining);
    }

    fn finish_pane_close_batch(&mut self) {
        let Some(batch) = self.host.take_pane_close_batch() else {
            return;
        };
        self.host.clear_pane_selection(batch.window);
        let target = batch.focus_after.and_then(|pane| {
            let idx = self
                .windows
                .iter()
                .position(|w| w.window_id == batch.window)?;
            Some((idx, self.windows[idx].tree.tiles.find_pane(&pane)?))
        });
        if let Some((idx, tile)) = target {
            self.windows[idx].navigate_to(tile);
        }
        log::info!(
            "close_batch: end window={} focus={:?}",
            batch.window,
            batch.focus_after.filter(|_| target.is_some())
        );
        self.mark_workspace_dirty();
    }
}
