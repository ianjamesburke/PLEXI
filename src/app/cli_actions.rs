//! Shared desktop and CLI handlers for host actions.
use super::{EdgePulse, PaneSwapAnim, PlexiApp};
use crate::host::keys::Direction;

/// The focus hand-off made by the last hide of a focused pane: which pane was
/// hidden and which tile took focus from it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HideHandoff {
    pub(crate) window_id: u64,
    pub(crate) hidden_pane: u64,
    pub(crate) successor: egui_tiles::TileId,
}

impl PlexiApp {
    pub(crate) fn toggle_pane_zoom(&mut self) {
        self.toggle_columns_pane_zoom();
    }

    pub(crate) fn swap_pane_action(&mut self, dir: Direction) {
        log::info!("pane_swap: direction={dir:?}");
        match self.swap_pane(dir) {
            crate::pane_ops::SwapResult::Swapped { rect_a, rect_b, .. } => {
                let now = std::time::Instant::now();
                self.pane_anims = vec![
                    PaneSwapAnim {
                        from: rect_a,
                        to: rect_b,
                        started_at: now,
                    },
                    PaneSwapAnim {
                        from: rect_b,
                        to: rect_a,
                        started_at: now,
                    },
                ];
                self.ctx.request_repaint();
            }
            crate::pane_ops::SwapResult::AtBoundary => {
                let ctx_id = self.windows[self.active_window].context_id;
                let in_strip = self.active_focused_pane_id().is_some_and(|pane| {
                    self.host.strip(ctx_id).is_some_and(|strip| strip.contains_pane(pane))
                });
                let moved = match dir {
                    crate::host::keys::Direction::Down => {
                        self.move_focused_pane_to_row_boundary(true)
                    }
                    crate::host::keys::Direction::Up => {
                        self.move_focused_pane_to_row_boundary(false)
                    }
                    _ if !in_strip => self.move_focused_pane_to_adjacent_window(dir),
                    _ => {
                        log::info!("pane_swap: horizontal strip boundary direction={dir:?}");
                        false
                    }
                };
                if moved {
                    self.ctx.request_repaint();
                } else if let Some(focused) = self.windows[self.active_window].focused_pane {
                    self.edge_pulse = Some(EdgePulse {
                        tile: focused,
                        dir,
                        started_at: std::time::Instant::now(),
                    });
                    self.ctx.request_repaint();
                }
            }
            crate::pane_ops::SwapResult::NoFocus => {}
        }
    }

    /// Cmd+U and the palette's hide command: hide the focused pane, or reveal
    /// it when it is already hidden. When the previous hide handed focus to
    /// the pane that still holds it, reveal the pane hidden last instead, so
    /// the key stays a hide-and-peek toggle (owner ruling 2026-11-12, ledger
    /// row 49).
    pub(crate) fn toggle_focused_pane_hidden(&mut self) {
        if let Some(pane_id) = self.pane_to_reveal_on_toggle() {
            log::info!("pane_hide: toggle reveals pane={pane_id}, the pane hidden last");
            if let Err(error) = self.set_pane_hidden(pane_id, false) {
                log::warn!("pane_hide: {error}");
                return;
            }
            if let Some((win, tile)) = self.find_pane_in_any_window(pane_id) {
                self.windows[win].navigate_to(tile);
            }
            return;
        }
        let win = &self.windows[self.active_window];
        let target = win
            .focused_pane
            .and_then(|tile| Self::find_pane_in_tile(&win.tree, tile))
            .and_then(|id| win.panes.get(&id).map(|pane| (id, !pane.is_hidden())));
        if let Some((pane_id, hidden)) = target {
            if let Err(error) = self.set_pane_hidden(pane_id, hidden) {
                log::warn!("pane_hide: {error}");
            }
        }
    }

    /// The pane the last hide sent away, while focus still sits on the pane
    /// that hide handed it to and the pane is still hidden.
    fn pane_to_reveal_on_toggle(&self) -> Option<u64> {
        let handoff = self.last_hide_handoff?;
        let win = &self.windows[self.active_window];
        (win.window_id == handoff.window_id
            && win.focused_pane == Some(handoff.successor)
            && win
                .panes
                .get(&handoff.hidden_pane)
                .is_some_and(|pane| pane.is_hidden()))
        .then_some(handoff.hidden_pane)
    }

    pub(crate) fn set_pane_hidden(&mut self, pane_id: u64, hidden: bool) -> Result<(), String> {
        let (win, tile) = self
            .find_pane_in_any_window(pane_id)
            .ok_or_else(|| format!("pane {pane_id} not found"))?;
        // A hidden pane never qualifies for input, so hiding the focused pane
        // must hand focus on. The successor is chosen while the pane is still
        // visible: the chooser locates it in its column, and hidden panes are
        // absent from columns.
        let hiding_focused = hidden && self.windows[win].focused_pane == Some(tile);
        let next_focus = hiding_focused
            .then(|| self.choose_focus_after_close(win, tile))
            .flatten();
        self.windows[win]
            .panes
            .get_mut(&pane_id)
            .ok_or_else(|| format!("pane {pane_id} has no runtime"))?
            .set_hidden(hidden);
        log::info!("pane_hide: pane={pane_id} hidden={hidden}");
        if let Some((next, rule)) = next_focus {
            log::info!("pane_hide: focus -> {next:?} rule={rule:?} (pane={pane_id} was focused)");
            self.windows[win].navigate_to(next);
            self.last_hide_handoff = Some(HideHandoff {
                window_id: self.windows[win].window_id,
                hidden_pane: pane_id,
                successor: next,
            });
        } else if hiding_focused {
            // Focus stays so the same shortcut can reveal the pane again.
            log::info!(
                "pane_hide: pane={pane_id} keeps focus, no other visible pane in its window"
            );
        }
        self.mark_workspace_dirty();
        Ok(())
    }

    pub(crate) fn close_context_action(&mut self, context_id: u64) -> Result<bool, String> {
        let idx = self
            .router
            .position(|c| c.context_id == context_id)
            .ok_or_else(|| format!("context {context_id} not found"))?;
        if self.router.len() <= 1 {
            log::info!("close_context: refusing to close only context ctx={context_id}");
            return Err("cannot close the last context".into());
        }
        let state = self.build_context_close_state(context_id);
        let confirm = self.config.confirm_context_close.unwrap_or(true);
        if state.items.is_empty() || !confirm {
            log::info!("close_context: closing ctx={context_id} idx={idx}");
            self.delete_context(idx);
            self.mark_workspace_dirty();
            Ok(false)
        } else {
            log::info!("close_context: ctx={context_id} awaiting confirmation");
            self.pending_context_close = Some(state);
            Ok(true)
        }
    }
}
