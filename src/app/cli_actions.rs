//! Shared desktop and CLI handlers for host actions.
use super::{EdgePulse, PaneSwapAnim, PlexiApp};
use crate::host::keys::Direction;

impl PlexiApp {
    pub(crate) fn toggle_pane_zoom(&mut self) {
        if let Some(parent) = self.fleet_focused_pane() {
            if self.dive_into_agent_children(parent) {
                return;
            }
        }
        let (focused_tile, portal_target) = {
            let win = &self.windows[self.active_window];
            let tile = win.focused_pane;
            let target = tile
                .and_then(|tile_id| win.tree.tiles.get(tile_id))
                .and_then(|t| {
                    if let egui_tiles::Tile::Pane(p) = t {
                        Some(*p)
                    } else {
                        None
                    }
                })
                .and_then(|pane_id| win.panes.get(&pane_id))
                .and_then(|p| p.portal_target());
            (tile, target)
        };
        if let Some(child_ctx_id) = portal_target {
            // Focused pane is a Portal tile → zoom into its sub-context.
            // Verify target context exists BEFORE pushing depth state;
            // push_depth must not fire if there is nothing to switch to.
            if let Some(ctx_idx) = self.router.position(|c| c.context_id == child_ctx_id) {
                log::info!("ToggleZoom on Portal: zooming into context_id={child_ctx_id}");
                let current_ctx_id = self.router.active().context_id;
                let current_win_id = self.windows[self.active_window].window_id;
                self.router
                    .push_depth(current_ctx_id, current_win_id, focused_tile);
                self.switch_workspace(ctx_idx);
            } else {
                log::warn!(
                    "ToggleZoom on Portal: target context_id={child_ctx_id} not found in router"
                );
            }
        } else {
            // Non-Portal pane → toggle fullscreen zoom.
            let ctx = &mut self.windows[self.active_window];
            if let Some(focused) = ctx.focused_pane {
                if ctx.zoomed_pane == Some(focused) {
                    ctx.clear_zoom();
                    log::info!("zoom: toggle off — pane={focused:?}");
                } else if ctx.panes.len() <= 1 {
                    log::info!(
                        "zoom: toggle ignored — pane={focused:?} is the only pane in window={}",
                        ctx.window_id,
                    );
                } else {
                    ctx.zoom_to(focused);
                    log::info!("zoom: toggle on — pane={focused:?}");
                }
            }
        }
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
