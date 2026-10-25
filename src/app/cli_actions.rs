//! Shared desktop and CLI handlers for host actions.
use super::{EdgePulse, PaneSwapAnim, PlexiApp};
use crate::host::keys::Direction;

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
                let moved = match dir {
                    crate::host::keys::Direction::Down => {
                        self.move_focused_pane_to_row_boundary(true)
                    }
                    crate::host::keys::Direction::Up => {
                        self.move_focused_pane_to_row_boundary(false)
                    }
                    _ => self.move_focused_pane_to_adjacent_window(dir),
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

    pub(crate) fn set_pane_hidden(&mut self, pane_id: u64, hidden: bool) -> Result<(), String> {
        let (win, _) = self
            .find_pane_in_any_window(pane_id)
            .ok_or_else(|| format!("pane {pane_id} not found"))?;
        self.windows[win]
            .panes
            .get_mut(&pane_id)
            .ok_or_else(|| format!("pane {pane_id} has no runtime"))?
            .set_hidden(hidden);
        log::info!("pane_hide: pane={pane_id} hidden={hidden}");
        self.mark_workspace_dirty();
        Ok(())
    }

    pub(crate) fn close_context_action(&mut self, context_id: u64) -> Result<bool, String> {
        let idx = self
            .router
            .position(|c| c.context_id == context_id)
            .ok_or_else(|| format!("context {context_id} not found"))?;
        if self.router.len() <= 1 {
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
