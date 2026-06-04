//! Layout operations on already-created panes: split, tab, close, navigate,
//! zoom-free tree manipulation, font size, scroll.

use crate::app::PlexiApp;
use crate::host::context::{replace_child, Window};
use crate::host::command::{HostAction, Placement};
use crate::host::effect::HostEffect;
use crate::host::keys::Direction;
use crate::host::pane::{Pane, TerminalPane};
use crate::spatial::tiling::PaneId;
use egui_term::BackendCommand;
use egui_tiles::{Container, SimplificationOptions, Tile, TileId};
use std::collections::HashMap;

pub(crate) enum SwapResult {
    Swapped { rect_a: egui::Rect, rect_b: egui::Rect },
    AtBoundary,
    NoFocus,
}

/// If `pane_id` is an app pane that was opened as an overlay over another
/// pane (`hint = Some("overlay")`), restore the original pane and return
/// `true`. Otherwise return `false` and leave the map unchanged.
/// Restores the pane hidden by an overlay app.
/// Returns `Some(cwd)` if an overlay was restored and the closed app reported a CWD to sync,
/// `Some(None)` if restored with no CWD to sync, or `None` if this was not an overlay.
pub(super) fn restore_overlay_replacement(
    panes: &mut HashMap<PaneId, Pane>,
    pane_id: PaneId,
) -> Option<Option<std::path::PathBuf>> {
    let Some(pane) = panes.remove(&pane_id) else {
        return None;
    };

    match pane {
        Pane::App(mut app) => {
            if let Some(replaced) = app.overlay_replaced.take() {
                let cwd = app.runtime.current_cwd();
                panes.insert(pane_id, *replaced);
                Some(cwd)
            } else {
                panes.insert(pane_id, Pane::App(app));
                None
            }
        }
        other => {
            panes.insert(pane_id, other);
            None
        }
    }
}

/// Insert `new_pane_id` as a new tile next to `split_target` in `tree`.
/// Pure tile/tree manipulation — no PlexiApp state, no focus history.
///
/// If `split_target` is `None` or the tree is empty, inserts as the new root.
/// If `split_target`'s parent is already a Linear container in the requested
/// direction, inserts as a sibling. Otherwise wraps `split_target` and the new
/// tile in a fresh Linear container.
///
/// Returns the `TileId` of the newly inserted pane.
pub(crate) fn insert_split_tile(
    tree: &mut egui_tiles::Tree<PaneId>,
    split_target: Option<egui_tiles::TileId>,
    new_pane_id: PaneId,
    vertical: bool,
    share: crate::host::command::ShareRatio,
    new_pane_first: bool,
) -> egui_tiles::TileId {
    use egui_tiles::LinearDir;

    let new_tile = tree.tiles.insert_pane(new_pane_id);

    // Empty tree or no split target → new pane becomes the root.
    let Some(target) = split_target.or(tree.root) else {
        tree.root = Some(new_tile);
        return new_tile;
    };

    // LinearDir::Horizontal = side-by-side (left/right); Vertical = stacked (top/bottom).
    // `vertical` here means "split vertically" (new pane to the right), so we need Horizontal.
    let split_dir = if vertical { LinearDir::Horizontal } else { LinearDir::Vertical };
    let parent = tree.tiles.parent_of(target);

    let inserted_as_sibling = if let Some(parent_id) = parent {
        if let Some(Tile::Container(Container::Linear(linear))) = tree.tiles.get_mut(parent_id) {
            if linear.dir == split_dir {
                if let Some(pos) = linear.children.iter().position(|&c| c == target) {
                    let insert_pos = if new_pane_first { pos } else { pos + 1 };
                    linear.children.insert(insert_pos, new_tile);
                    true
                } else { false }
            } else { false }
        } else { false }
    } else { false };

    if !inserted_as_sibling {
        let ordered = if new_pane_first {
            vec![new_tile, target]
        } else {
            vec![target, new_tile]
        };
        let container_tile = if vertical {
            tree.tiles.insert_horizontal_tile(ordered)
        } else {
            tree.tiles.insert_vertical_tile(ordered)
        };
        if let Some(Tile::Container(Container::Linear(ref mut lin))) = tree.tiles.get_mut(container_tile) {
            lin.shares.set_share(target, share.denominator);
            lin.shares.set_share(new_tile, share.numerator);
        }
        if let Some(parent_id) = parent {
            if let Some(Tile::Container(parent_container)) = tree.tiles.get_mut(parent_id) {
                replace_child(parent_container, target, container_tile);
            }
        } else {
            tree.root = Some(container_tile);
        }
    }

    new_tile
}

impl PlexiApp {
    /// Route a HostAction through HostModel and return the resulting effects.
    pub(super) fn submit(&mut self, cmd: HostAction) -> Vec<HostEffect> {
        self.host.handle_command(cmd, &mut self.host_services)
    }

    pub(crate) fn split_with_new_pane(
        &mut self,
        new_pane_id: PaneId,
        vertical: bool,
        share: crate::host::command::ShareRatio,
        new_pane_first: bool,
        keep_focus: bool,
    ) -> Option<egui_tiles::TileId> {
        let old_window_id = self.windows[self.active_window].window_id;
        let old_focus = self.windows[self.active_window].focused_pane;
        let focused = self.windows[self.active_window].focused_pane?;
        let split_target = match self.windows[self.active_window].find_ancestor_tabs(focused) {
            Some((tabs_id, _)) => tabs_id,
            None => focused,
        };
        self.push_focus_history(old_window_id, old_focus);

        let ctx = &mut self.windows[self.active_window];
        let new_tile = insert_split_tile(&mut ctx.tree, Some(split_target), new_pane_id, vertical, share, new_pane_first);

        if !keep_focus {
            ctx.focused_pane = Some(new_tile);
        }
        Some(new_tile)
    }

    /// Split the focused pane in the requested direction, creating a new pane
    /// that mirrors the focused pane's type:
    ///
    /// - Terminal → new terminal in the same cwd
    /// - App      → fresh instance of the same app (`launch_app_by_id`)
    ///
    /// If no pane is focused, falls back to creating a full-size terminal.
    pub(crate) fn split_focused_mirror(&mut self, placement: Placement) {
        let active = self.active_window;
        let Some(focused_tile) = self.windows[active].focused_pane else {
            // No focused pane → create a full-size terminal in the active context.
            // The empty-context path: if the context has no panes at all, replace
            // the tree. If it has panes but none focused (rare), drop into the
            // standard terminal split path which will no-op for now.
            if self.windows[active].panes.is_empty() {
                if let Some((tree, panes, root_tile)) = self.create_single_pane_tree(None, None, false) {
                    self.windows[active].tree = tree;
                    self.windows[active].panes = panes;
                    self.set_window_focused_pane(active, root_tile);
                }
            }
            return;
        };

        let Some(Tile::Pane(focused_pane_id)) =
            self.windows[active].tree.tiles.get(focused_tile)
        else {
            return;
        };
        let focused_pane_id = *focused_pane_id;

        // Determine the focused pane's type for the mirror decision.
        // Capture app manifest_id up front so we can drop the immutable borrow
        // before mutating in the App branch.
        enum Kind {
            Terminal,
            App(String),
        }
        let kind = match self.windows[active].panes.get(&focused_pane_id) {
            Some(Pane::Terminal(_)) => Kind::Terminal,
            Some(Pane::App(a)) => Kind::App(a.manifest_id.clone()),
            Some(Pane::Portal(_)) | None => return,
        };

        // `vertical` here follows split_with_new_pane semantics (not split_focused):
        //   true  → split_h (side-by-side, new pane right) → Placement::Right
        //   false → split_v (stacked,      new pane below) → Placement::Below
        let vertical = matches!(placement, Placement::Right);

        match kind {
            Kind::Terminal => {
                // Reuse the existing terminal split path.
                self.split_focused(vertical, None, false, false, None);
            }
            Kind::App(manifest_id) => {
                // Fresh instance of the same app at the requested placement.
                // The layout hint maps directly to the split direction:
                //   Placement::Right → "split_h" (side-by-side, new pane right)
                //   Placement::Below → "split_v" (stacked,      new pane below)
                let layout = if vertical { "split_h" } else { "split_v" };
                let _ = self.launch_app_by_id_with_layout(
                    &manifest_id,
                    Some(layout.to_string()),
                    &[],
                    None,
                );
            }
        }
    }

    pub(crate) fn split_focused(&mut self, vertical: bool, initial_cmd: Option<&str>, close_on_exit: bool, new_pane_first: bool, cwd_override: Option<std::path::PathBuf>) {
        let old_window_id = self.windows[self.active_window].window_id;
        let old_focus = self.windows[self.active_window].focused_pane;
        let Some(focused) = self.windows[self.active_window].focused_pane else {
            return;
        };

        let cmd = if vertical {
            HostAction::SplitVertical
        } else {
            HostAction::SplitHorizontal
        };
        let effects = self.submit(cmd);
        log::debug!("split_focused(vertical={vertical} new_pane_first={new_pane_first}) effects: {:?}", effects);
        let (new_id, vertical) = effects
            .iter()
            .find_map(|e| match e {
                HostEffect::SplitOpened {
                    pane_id, placement, ..
                } => Some((*pane_id, !matches!(placement, Placement::Below))),
                _ => None,
            })
            .unwrap_or_else(|| (self.host.alloc_pane_id(), vertical));

        let direction = if vertical { "vertical" } else { "horizontal" }.to_string();
        log::info!("split_focused: emitting PaneSplit pane_id={new_id} direction={direction}");
        crate::host::event_log::emit(crate::host::event_log::HostEvent::PaneSplit {
            pane_id: new_id,
            direction,
            timestamp: crate::host::event_log::now_timestamp(),
        });

        let cwd = self.resolve_new_pane_cwd(cwd_override, Some(focused));
        log::info!("split_focused: cwd={cwd:?} context_root={:?}", self.router.active().root);
        let ctx_id = self.windows.get(self.active_window).map(|w| w.context_id).unwrap_or(0);
        let ctx_name = self.context_name_for(ctx_id);
        let ctx_desc = self.context_description_for(ctx_id);
        let ctx_root = self.context_root_for(ctx_id);
        let ctx_depth = self.context_depth_for(ctx_id);
        let mut settings = Self::make_backend_settings(new_id, cwd, &self.colors, ctx_id, &ctx_name, &ctx_desc, ctx_root.as_ref(), ctx_depth);
        if let Some(cmd) = initial_cmd {
            log::info!("split_focused: initial_cmd={cmd:?} close_on_exit={close_on_exit}");
            super::apply_initial_cmd(&mut settings, cmd, close_on_exit);
        }
        let Some(mut pane) = TerminalPane::new(
            new_id,
            self.ctx.clone(),
            self.pty_event_tx.clone(),
            settings,
            self.default_font_size,
        ) else {
            log::error!("Failed to create new terminal pane");
            return;
        };
        pane.ephemeral = close_on_exit;
        self.windows[self.active_window]
            .panes
            .insert(new_id, Pane::Terminal(Box::new(pane)));

        let split_target = match self.windows[self.active_window].find_ancestor_tabs(focused) {
            Some((tabs_id, _)) => tabs_id,
            None => focused,
        };
        self.push_focus_history(old_window_id, old_focus);

        let ctx = &mut self.windows[self.active_window];
        let parent = ctx.tree.tiles.parent_of(split_target);
        let new_tile = ctx.tree.tiles.insert_pane(new_id);

        // NOTE: split_focused uses INVERTED LinearDir vs split_with_new_pane.
        // Here vertical=true → LinearDir::Vertical → stacked (BELOW);
        //      vertical=false → LinearDir::Horizontal → side-by-side (RIGHT).
        // Any code routing here (e.g. SpawnPane terminal) must account for this.
        let split_dir = if vertical {
            egui_tiles::LinearDir::Vertical
        } else {
            egui_tiles::LinearDir::Horizontal
        };

        let inserted_as_sibling = if let Some(parent_id) = parent {
            if let Some(Tile::Container(Container::Linear(linear))) =
                ctx.tree.tiles.get_mut(parent_id)
            {
                if linear.dir == split_dir {
                    if let Some(pos) = linear.children.iter().position(|&c| c == split_target) {
                        let insert_pos = if new_pane_first { pos } else { pos + 1 };
                        linear.children.insert(insert_pos, new_tile);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        if !inserted_as_sibling {
            let ordered = if new_pane_first {
                vec![new_tile, split_target]
            } else {
                vec![split_target, new_tile]
            };
            let container_tile = if vertical {
                ctx.tree.tiles.insert_vertical_tile(ordered)
            } else {
                ctx.tree.tiles.insert_horizontal_tile(ordered)
            };

            if let Some(parent_id) = parent {
                if let Some(Tile::Container(parent)) = ctx.tree.tiles.get_mut(parent_id) {
                    replace_child(parent, split_target, container_tile);
                }
            } else {
                ctx.tree.root = Some(container_tile);
            }
        }

        ctx.focused_pane = Some(new_tile);
    }

    pub(crate) fn new_tab(&mut self, initial_cmd: Option<&str>, close_on_exit: bool) {
        // Empty context (welcome screen): create the first pane as tree root.
        if self.windows[self.active_window].panes.is_empty() {
            let new_id = self.host.alloc_pane_id();
            let ctx_id = self.windows.get(self.active_window).map(|w| w.context_id).unwrap_or(0);
            let ctx_name = self.context_name_for(ctx_id);
            let ctx_desc = self.context_description_for(ctx_id);
            let ctx_root = self.context_root_for(ctx_id);
            let ctx_depth = self.context_depth_for(ctx_id);
            let cwd = self.cwd_for_welcome_tab();
            log::info!("new_tab (empty context): cwd={cwd:?} context_root={:?}", self.router.active().root);
            let mut settings = Self::make_backend_settings(new_id, Some(cwd), &self.colors, ctx_id, &ctx_name, &ctx_desc, ctx_root.as_ref(), ctx_depth);
            if let Some(cmd) = initial_cmd {
                log::info!("new_tab (empty context): initial_cmd={cmd:?} close_on_exit={close_on_exit}");
                super::apply_initial_cmd(&mut settings, cmd, close_on_exit);
            }
            let Some(mut pane) = TerminalPane::new(
                new_id,
                self.ctx.clone(),
                self.pty_event_tx.clone(),
                settings,
                self.default_font_size,
            ) else {
                log::error!("Failed to create first terminal pane in empty context");
                return;
            };
            pane.ephemeral = close_on_exit;
            let ctx = &mut self.windows[self.active_window];
            ctx.panes.insert(new_id, Pane::Terminal(Box::new(pane)));
            let pane_tile = ctx.tree.tiles.insert_pane(new_id);
            let tab_tile = ctx.tree.tiles.insert_tab_tile(vec![pane_tile]);
            ctx.tree.root = Some(tab_tile);
            ctx.focused_pane = Some(pane_tile);
            return;
        }

        let old_window_id = self.windows[self.active_window].window_id;
        let old_focus = self.windows[self.active_window].focused_pane;
        let Some(focused) = self.windows[self.active_window].focused_pane else {
            return;
        };

        let new_id = self.host.alloc_pane_id();

        let cwd = self.resolve_new_pane_cwd(None, Some(focused));
        log::info!("new_tab: cwd={cwd:?} context_root={:?}", self.router.active().root);
        let ctx_id = self.windows.get(self.active_window).map(|w| w.context_id).unwrap_or(0);
        let ctx_name = self.context_name_for(ctx_id);
        let ctx_desc = self.context_description_for(ctx_id);
        let ctx_root = self.context_root_for(ctx_id);
        let ctx_depth = self.context_depth_for(ctx_id);
        let mut settings = Self::make_backend_settings(new_id, cwd, &self.colors, ctx_id, &ctx_name, &ctx_desc, ctx_root.as_ref(), ctx_depth);
        if let Some(cmd) = initial_cmd {
            log::info!("new_tab: initial_cmd={cmd:?} close_on_exit={close_on_exit}");
            super::apply_initial_cmd(&mut settings, cmd, close_on_exit);
        }
        let Some(mut pane) = TerminalPane::new(
            new_id,
            self.ctx.clone(),
            self.pty_event_tx.clone(),
            settings,
            self.default_font_size,
        ) else {
            log::error!("Failed to create new terminal pane");
            return;
        };
        pane.ephemeral = close_on_exit;
        self.windows[self.active_window]
            .panes
            .insert(new_id, Pane::Terminal(Box::new(pane)));
        self.push_focus_history(old_window_id, old_focus);

        let ctx = &mut self.windows[self.active_window];
        let new_tile = ctx.tree.tiles.insert_pane(new_id);

        if let Some((tabs_id, _)) = ctx.find_ancestor_tabs(focused) {
            if let Some(Tile::Container(Container::Tabs(tabs))) = ctx.tree.tiles.get_mut(tabs_id) {
                tabs.add_child(new_tile);
                tabs.set_active(new_tile);
            }
            ctx.focused_pane = Some(new_tile);
            return;
        }

        let parent = ctx.tree.tiles.parent_of(focused);
        let tab_tile = ctx.tree.tiles.insert_tab_tile(vec![focused, new_tile]);

        if let Some(Tile::Container(Container::Tabs(tabs))) = ctx.tree.tiles.get_mut(tab_tile) {
            tabs.set_active(new_tile);
        }

        if let Some(parent_id) = parent {
            if let Some(Tile::Container(parent_container)) = ctx.tree.tiles.get_mut(parent_id) {
                replace_child(parent_container, focused, tab_tile);
            }
        } else {
            ctx.tree.root = Some(tab_tile);
        }

        ctx.focused_pane = Some(new_tile);
    }

    pub(crate) fn cycle_tab(&mut self, forward: bool) {
        let ctx = &self.windows[self.active_window];
        let Some(focused) = ctx.focused_pane else {
            return;
        };

        let Some((tabs_id, _)) = ctx.find_ancestor_tabs(focused) else {
            return;
        };

        let Some(Tile::Container(Container::Tabs(tabs))) = ctx.tree.tiles.get(tabs_id) else {
            return;
        };

        let children = &tabs.children;
        if children.len() < 2 {
            return;
        }

        let active_idx = tabs
            .active
            .and_then(|a| children.iter().position(|&c| c == a))
            .unwrap_or(0);

        let new_idx = if forward {
            (active_idx + 1) % children.len()
        } else {
            (active_idx + children.len() - 1) % children.len()
        };
        let target = children[new_idx];

        let ctx = &mut self.windows[self.active_window];
        if let Some(Tile::Container(Container::Tabs(tabs))) = ctx.tree.tiles.get_mut(tabs_id) {
            tabs.set_active(target);
        }

        if let Some(pane_tile) = ctx.find_first_pane_in(target) {
            ctx.focused_pane = Some(pane_tile);
            if ctx.zoomed_pane.is_some() {
                ctx.zoomed_pane = Some(pane_tile);
            }
        }
    }

    pub(crate) fn jump_to_tab(&mut self, index: usize) {
        let ctx = &self.windows[self.active_window];
        let Some(focused) = ctx.focused_pane else {
            return;
        };

        let Some((tabs_id, _)) = ctx.find_ancestor_tabs(focused) else {
            return;
        };

        let Some(Tile::Container(Container::Tabs(tabs))) = ctx.tree.tiles.get(tabs_id) else {
            return;
        };

        let children = &tabs.children;
        if children.is_empty() {
            return;
        }

        let clamped = index.min(children.len() - 1);
        let target = children[clamped];

        let ctx = &mut self.windows[self.active_window];
        if let Some(Tile::Container(Container::Tabs(tabs))) = ctx.tree.tiles.get_mut(tabs_id) {
            tabs.set_active(target);
        }

        if let Some(pane_tile) = ctx.find_first_pane_in(target) {
            ctx.focused_pane = Some(pane_tile);
            if ctx.zoomed_pane.is_some() {
                ctx.zoomed_pane = Some(pane_tile);
            }
        }
    }

    pub(crate) fn close_focused(&mut self) {
        let focused = match self.windows[self.active_window].focused_pane {
            Some(f) => f,
            None => return,
        };
        let focused_pane_id = self.windows[self.active_window]
            .tree
            .tiles
            .get(focused)
            .and_then(|tile| match tile {
                Tile::Pane(pane_id) => Some(*pane_id),
                _ => None,
            });
        if let Some(pane_id) = focused_pane_id {
            if let Some(maybe_cwd) =
                restore_overlay_replacement(&mut self.windows[self.active_window].panes, pane_id)
            {
                if let Some(cwd) = maybe_cwd {
                    let escaped = cwd.to_string_lossy().replace('\'', "'\\''");
                    let cd_cmd = format!("\x15cd '{}'\n", escaped);
                    if let Some(t) = self.windows[self.active_window]
                        .panes
                        .get_mut(&pane_id)
                        .and_then(|p| p.as_terminal_mut())
                    {
                        t.backend.process_command(egui_term::BackendCommand::Write(
                            cd_cmd.as_bytes().to_vec(),
                        ));
                        log::info!(
                            "file_browser: synced cwd '{}' to terminal pane {}",
                            cwd.display(),
                            pane_id
                        );
                    }
                }
                return;
            }
        }

        let effects = self.submit(HostAction::CloseFocusedPane);
        log::debug!("close_focused effects: {:?}", effects);
        self.close_tile(self.active_window, focused);
    }

    /// Close a specific pane by its PaneId (the u64 backend ID, not the TileId).
    /// Searches all contexts to find the tile containing this pane.
    pub(crate) fn close_pane_by_id(&mut self, pane_id: PaneId) {
        // Find which context and tile owns this pane_id.
        for ctx_idx in 0..self.windows.len() {
            if let Some(tile_id) = self.windows[ctx_idx].tree.tiles.find_pane(&pane_id) {
                self.close_tile(ctx_idx, tile_id);
                // Mirror the guard in execute_close_pane: if the window is now empty
                // and there are other pages in the same context, delete the zombie window.
                // Without this, the window stays in self.windows[] as a phantom grid cell.
                if self.windows[ctx_idx].panes.is_empty() {
                    let ctx_id = self.windows[ctx_idx].context_id;
                    let pages_in_context =
                        self.windows.iter().filter(|w| w.context_id == ctx_id).count();
                    if pages_in_context > 1 {
                        log::info!(
                            "close_pane_by_id: window {ctx_idx} empty, {pages_in_context} pages in context {ctx_id} — deleting zombie window"
                        );
                        self.delete_window(ctx_idx);
                    }
                }
                return;
            }
        }
    }

    /// Close a tile in a specific context by its TileId. Handles sibling focus
    /// transfer, container cleanup, and pane removal.
    pub(super) fn close_tile(&mut self, ctx_idx: usize, tile_id: TileId) {
        // Snapshot focus/zoom state before any mutation so Phase 2 guards are accurate.
        let is_focused = self.windows[ctx_idx].focused_pane == Some(tile_id);
        let is_zoomed = self.windows[ctx_idx].zoomed_pane == Some(tile_id);

        // Phase 1: Read-only — determine sibling and container type
        let parent_info = self.windows[ctx_idx].find_logical_parent(tile_id);

        let next = if let Some((parent_id, child_in_parent)) = parent_info {
            let sibling_info = {
                let ctx: &Window = &self.windows[ctx_idx];
                if let Some(Tile::Container(container)) = ctx.tree.tiles.get(parent_id) {
                    let children: Vec<TileId> = container.children().copied().collect();
                    children
                        .iter()
                        .position(|&c| c == child_in_parent)
                        .map(|pos| {
                            let sibling = if pos + 1 < children.len() {
                                children[pos + 1]
                            } else {
                                children[pos - 1]
                            };
                            let is_tabs = matches!(container, Container::Tabs(_));
                            let is_linear = matches!(container, Container::Linear(_));
                            // Only treat this as an active-tab close if the closing child
                            // was actually the selected tab. Closing an inactive tab must
                            // not switch the active tab to a different sibling.
                            let is_active_tab = if let Container::Tabs(tabs) = container {
                                tabs.active == Some(child_in_parent)
                            } else {
                                false
                            };
                            (sibling, is_tabs, is_linear, is_active_tab, children)
                        })
                } else {
                    None
                }
            };

            if let Some((sibling, is_tabs, is_linear, is_active_tab, all_children)) = sibling_info {
                // Phase 2: Mutable — update container state.
                // For tabs: only switch the active tab when the closing tile was the active one.
                // Switching away from an already-active sibling would steal focus from a
                // pane the user chose to focus.
                let ctx = &mut self.windows[ctx_idx];
                if is_tabs && is_active_tab {
                    if let Some(Tile::Container(Container::Tabs(tabs))) =
                        ctx.tree.tiles.get_mut(parent_id)
                    {
                        tabs.set_active(sibling);
                    }
                }
                if is_linear {
                    if let Some(Tile::Container(Container::Linear(linear))) =
                        ctx.tree.tiles.get_mut(parent_id)
                    {
                        for &child in &all_children {
                            linear.shares.set_share(child, 1.0);
                        }
                    }
                }

                // Only transfer focus when the closed tile was focused.
                // tabs.set_active was already updated above so egui_tiles stays valid
                // even when closing a background active tab; but that must not move
                // focused_pane to the new active tab's pane.
                if is_focused {
                    self.windows[ctx_idx].find_first_pane_in(sibling)
                } else {
                    None
                }
            } else if is_focused {
                self.windows[ctx_idx].find_next_focus(tile_id)
            } else {
                None
            }
        } else if is_focused {
            self.windows[ctx_idx].find_next_focus(tile_id)
        } else {
            None
        };

        // Phase 3: Remove tile and extract pane — defer drop so background apps can be parked.
        let removed_pane = {
            let ctx = &mut self.windows[ctx_idx];
            if let Some(parent_id) = ctx.tree.tiles.parent_of(tile_id) {
                if let Some(Tile::Container(parent)) = ctx.tree.tiles.get_mut(parent_id) {
                    parent.remove_child(tile_id);
                }
            }

            let removed = if let Some(Tile::Pane(pane_id)) = ctx.tree.tiles.remove(tile_id) {
                log::info!("close_tile: emitting PaneClosed pane_id={pane_id}");
                crate::host::event_log::emit(crate::host::event_log::HostEvent::PaneClosed {
                    pane_id,
                    timestamp: crate::host::event_log::now_timestamp(),
                });
                ctx.panes.remove(&pane_id)
            } else {
                None
            };

            ctx.tree.simplify(&SimplificationOptions {
                all_panes_must_have_tabs: true,
                ..SimplificationOptions::default()
            });

            // Only update focused_pane when focus actually needs to transfer.
            // A background-pane close must not steal focus from the current pane.
            if let Some(new_focus) = next {
                log::info!("close_tile: focus -> {:?}", new_focus);
                ctx.focused_pane = Some(new_focus);
            } else if is_focused {
                // Closed tile was focused but no sibling found — clear focus.
                log::info!("close_tile: focus -> None (no sibling)");
                ctx.focused_pane = None;
            }

            // Clear zoom if the zoomed tile was closed. Render-time validation also does
            // this, but clearing it here avoids a one-frame inconsistency where
            // ToggleZoom sees zoomed_pane pointing at a dead tile.
            if is_zoomed {
                ctx.zoomed_pane = None;
                log::info!("close_tile: cleared stale zoom (closed tile was zoomed)");
            }

            removed
        };

        // ctx borrow is released — park background ProcessApps; drop everything else.
        match removed_pane {
            Some(Pane::App(app_pane)) => {
                let pane_id = app_pane.id;
                // Hot reload (#83): drop any active watcher for this pane.
                // Idempotent — no-op when the pane wasn't being watched.
                self.hot_reload.unwatch(pane_id);
                if let crate::host::pane::AppRuntime::Process(mut process_app) = app_pane.runtime {
                    let type_id = process_app.type_id.clone();
                    if self.registry.is_background(&type_id) {
                        process_app.send_event(&crate::app_protocol::PlexiEvent::Suspend);
                        let park_context_id = self.windows[ctx_idx].context_id;
                        log::info!("parking background app '{type_id}' in context_id {park_context_id}");
                        self.background_apps.insert(type_id, (park_context_id, process_app));
                    }
                    // else: process_app drops here — Drop impl sends Shutdown + kills process
                }
                // Tombstone any notifications this pane posted — they stay visible but
                // their action buttons are hidden since the app can no longer respond.
                self.tombstone_pane_notifications(pane_id);
                // else: builtin app pane drops here
            }
            _ => {}
        }
    }

    pub(crate) fn navigate(&mut self, dir: Direction) {
        let effects = self.submit(HostAction::Navigate(dir));
        log::debug!("navigate({:?}) effects: {:?}", dir, effects);

        let (dx, dy) = match dir {
            Direction::Left  => (-1,  0),
            Direction::Right => ( 1,  0),
            Direction::Up    => ( 0, -1),
            Direction::Down  => ( 0,  1),
        };

        let ctx = &self.windows[self.active_window];
        let pane_neighbor = ctx.focused_pane
            .and_then(|focused| ctx.find_pane_in_direction_from(focused, dir));

        if let Some(target) = pane_neighbor {
            self.windows[self.active_window].focused_pane = Some(target);
            // Signal the newly-focused pane so render_text_inputs auto-focuses
            // the first TextInput on the next frame.
            if let Some(egui_tiles::Tile::Pane(pane_id)) =
                self.windows[self.active_window].tree.tiles.get(target)
            {
                let pane_id = *pane_id;
                if let Some(pane) = self.windows[self.active_window].panes.get_mut(&pane_id) {
                    if let Some(app) = pane.as_app_mut() {
                        if let crate::host::pane::AppRuntime::Process(ref mut proc_app) = app.runtime {
                            proc_app.render_session.pane_just_focused = true;
                        }
                    }
                }
            }
        } else if dy != 0 {
            // Vertical boundary: jump to the first or last window in the current
            // workspace (the minimap list). Down at bottom → last window;
            // Up at top → first window. This is a list-end jump, not a one-step move.
            let ws_id = self.router.active().context_id;
            let jump_idx = if dy < 0 {
                self.windows.iter().enumerate()
                    .filter(|(_, w)| w.context_id == ws_id)
                    .min_by_key(|(_, w)| (w.grid_y, w.grid_x))
                    .map(|(i, _)| i)
            } else {
                self.windows.iter().enumerate()
                    .filter(|(_, w)| w.context_id == ws_id)
                    .max_by_key(|(_, w)| (w.grid_y, w.grid_x))
                    .map(|(i, _)| i)
            };
            if let Some(idx) = jump_idx {
                if idx != self.active_window {
                    log::info!("navigate({:?}): jumping to {} window in workspace", dir, if dy < 0 { "first" } else { "last" });
                    self.active_window = idx;
                    let w = &self.windows[idx];
                    let wid = w.window_id;
                    self.context_active_window.insert(ws_id, wid);
                    self.record_context_visit(wid);
                    // Focus the spatially leftmost pane in the destination window.
                    // If the window has not yet been rendered (no rects), leave focused_pane as-is.
                    let leftmost: Option<TileId> = {
                        let dest = &self.windows[idx];
                        dest.tree.active_tiles().into_iter()
                            .filter_map(|tile_id| {
                                if !matches!(dest.tree.tiles.get(tile_id), Some(Tile::Pane(_))) {
                                    return None;
                                }
                                let rect = dest.tree.tiles.rect(tile_id)?;
                                Some((tile_id, rect))
                            })
                            .min_by(|(_, a), (_, b)| {
                                a.left().partial_cmp(&b.left())
                                    .unwrap_or(std::cmp::Ordering::Equal)
                                    .then(a.top().partial_cmp(&b.top()).unwrap_or(std::cmp::Ordering::Equal))
                            })
                            .map(|(tile_id, _)| tile_id)
                    };
                    if let Some(tile_id) = leftmost {
                        log::info!("navigate({:?}): focused_pane → leftmost {:?}", dir, tile_id);
                        self.windows[idx].focused_pane = Some(tile_id);
                    }
                }
            }
        } else {
            log::info!("navigate({:?}): falling through to page navigation", dir);
            self.navigate_page(dx, dy);
        }
    }

    pub(crate) fn swap_pane(&mut self, dir: Direction) -> SwapResult {
        use egui_tiles::Tile;
        let active = self.active_window;
        let ctx = &mut self.windows[active];

        let Some(focused) = ctx.focused_pane else {
            return SwapResult::NoFocus;
        };

        let Some(neighbor) = ctx.find_pane_in_direction_from(focused, dir) else {
            log::info!("swap_pane({:?}): at boundary, no neighbor", dir);
            return SwapResult::AtBoundary;
        };

        let rect_a = ctx.tree.tiles.rect(focused).unwrap_or(egui::Rect::ZERO);
        let rect_b = ctx.tree.tiles.rect(neighbor).unwrap_or(egui::Rect::ZERO);

        let pane_a = match ctx.tree.tiles.get(focused) {
            Some(Tile::Pane(id)) => *id,
            _ => return SwapResult::NoFocus,
        };
        let pane_b = match ctx.tree.tiles.get(neighbor) {
            Some(Tile::Pane(id)) => *id,
            _ => return SwapResult::NoFocus,
        };

        if let Some(Tile::Pane(id)) = ctx.tree.tiles.get_mut(focused) {
            *id = pane_b;
        }
        if let Some(Tile::Pane(id)) = ctx.tree.tiles.get_mut(neighbor) {
            *id = pane_a;
        }

        // Focus follows content: move focus to the tile now containing this pane.
        ctx.focused_pane = Some(neighbor);

        log::info!(
            "swap_pane({:?}): pane {} ↔ pane {} (tiles {:?} ↔ {:?})",
            dir, pane_a, pane_b, focused, neighbor
        );

        SwapResult::Swapped { rect_a, rect_b }
    }

    /// Move the focused pane to the first (`end=false`, Cmd+Ctrl+K) or last
    /// (`end=true`, Cmd+Ctrl+J) column position in the current context row by
    /// creating a new window at that boundary. Returns `true` if the move
    /// happened; `false` if the pane is already alone at the target boundary
    /// (caller should show the edge pulse).
    pub(crate) fn move_focused_pane_to_row_boundary(&mut self, end: bool) -> bool {
        use egui_tiles::Tile;

        let src_idx = self.active_window;

        let Some(focused_tile) = self.windows[src_idx].focused_pane else {
            return false;
        };

        let src_pane_id = match self.windows[src_idx].tree.tiles.get(focused_tile) {
            Some(Tile::Pane(id)) => *id,
            _ => return false,
        };

        let src_grid_y = self.windows[src_idx].grid_y;
        let src_grid_x = self.windows[src_idx].grid_x;
        let ws_id = self.windows[src_idx].context_id;

        // Edge pulse if pane is already alone at the target boundary.
        if self.windows[src_idx].panes.len() == 1 {
            let boundary_x = self
                .windows
                .iter()
                .filter(|w| w.context_id == ws_id && w.grid_y == src_grid_y)
                .map(|w| w.grid_x)
                .reduce(if end { u32::max } else { u32::min });
            let at_boundary = boundary_x == Some(src_grid_x);
            if at_boundary {
                log::info!(
                    "move_focused_pane_to_row_boundary(end={end}): \
                     pane {src_pane_id} already at boundary — edge pulse"
                );
                return false;
            }
        }

        let cwd = self.windows[src_idx]
            .get_focused_pane_cwd(focused_tile)
            .or_else(|| self.router.active().root.clone())
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")));

        let next_src_focus = self.windows[src_idx].find_next_focus(focused_tile);

        // Detach pane from source window's tile tree.
        let extracted_pane = {
            let ctx = &mut self.windows[src_idx];
            if let Some(parent_id) = ctx.tree.tiles.parent_of(focused_tile) {
                if let Some(Tile::Container(parent)) = ctx.tree.tiles.get_mut(parent_id) {
                    parent.remove_child(focused_tile);
                }
            }
            ctx.tree.tiles.remove(focused_tile);
            let pane = ctx.panes.remove(&src_pane_id);
            ctx.tree.simplify(&SimplificationOptions {
                all_panes_must_have_tabs: true,
                ..SimplificationOptions::default()
            });
            ctx.focused_pane = next_src_focus;
            pane
        };

        let Some(pane) = extracted_pane else {
            return false;
        };

        // Delete source window if empty (compact_workspace_grid runs inside
        // delete_window, reassigning grid_x values).
        if self.windows[src_idx].panes.is_empty() && self.windows.len() > 1 {
            log::info!(
                "move_focused_pane_to_row_boundary(end={end}): \
                 source window ({},{src_grid_y}) empty — deleting",
                self.windows[src_idx].grid_x,
            );
            self.delete_window(src_idx);
        }

        // Compute the target grid_x after any compaction from delete_window.
        let new_x = if end {
            let max_x = self
                .windows
                .iter()
                .filter(|w| w.context_id == ws_id && w.grid_y == src_grid_y)
                .map(|w| w.grid_x)
                .max();
            max_x.map(|x| x + 1).unwrap_or(0)
        } else {
            // Shift all same-row windows right to open grid_x=0.
            for w in self.windows.iter_mut() {
                if w.context_id == ws_id && w.grid_y == src_grid_y {
                    w.grid_x += 1;
                }
            }
            0
        };

        log::info!(
            "move_focused_pane_to_row_boundary(end={end}): \
             pane {src_pane_id} → new window ({new_x},{src_grid_y})"
        );

        let mut new_tree =
            egui_tiles::Tree::empty(format!("tree_boundary_{}", self.next_window_id));
        let new_tile = new_tree.tiles.insert_pane(src_pane_id);
        new_tree.root = Some(new_tile);
        let mut new_panes = HashMap::new();
        new_panes.insert(src_pane_id, pane);

        let win_id = self.next_window_id;
        self.next_window_id += 1;

        self.windows.push(crate::host::context::Window {
            name: String::new(),
            path: cwd,
            tree: new_tree,
            panes: new_panes,
            focused_pane: Some(new_tile),
            zoomed_pane: None,
            grid_x: new_x,
            grid_y: src_grid_y,
            window_id: win_id,
            context_id: ws_id,
        });

        self.active_window = self.windows.len() - 1;
        self.context_active_window.insert(ws_id, win_id);
        self.record_context_visit(win_id);
        self.minimap.visible = true;

        true
    }

    /// Move the focused pane from the active window into the adjacent window in
    /// `dir`. Returns `true` if the move happened; `false` if there is no
    /// adjacent window in that direction (caller should show the edge pulse).
    pub(crate) fn move_focused_pane_to_adjacent_window(&mut self, dir: Direction) -> bool {
        use egui_tiles::Tile;

        let (dx, dy): (i32, i32) = match dir {
            Direction::Left  => (-1,  0),
            Direction::Right => ( 1,  0),
            Direction::Up    => ( 0, -1),
            Direction::Down  => ( 0,  1),
        };

        let src_idx = self.active_window;
        let cur_x = self.windows[src_idx].grid_x;
        let cur_y = self.windows[src_idx].grid_y;
        let ws_id = self.router.active().context_id;

        // Find the index of the adjacent window in the direction of movement.
        let adj_idx = {
            let pages: Vec<(u32, u32, u64)> = self
                .windows
                .iter()
                .map(|w| (w.grid_x, w.grid_y, w.context_id))
                .collect();
            if dx != 0 {
                let tx = cur_x as i32 + dx;
                if tx < 0 {
                    return false;
                }
                let tx = tx as u32;
                pages
                    .iter()
                    .position(|&(gx, gy, ws)| gx == tx && gy == cur_y && ws == ws_id)
            } else {
                let ty = cur_y as i32 + dy;
                if ty < 0 {
                    return false;
                }
                let ty = ty as u32;
                let preferred_x = self.last_page_x_per_row.get(&ty).copied().unwrap_or(cur_x);
                pages
                    .iter()
                    .enumerate()
                    .filter(|(_, &(_, gy, ws))| gy == ty && ws == ws_id)
                    .min_by_key(|(_, &(gx, _, _))| {
                        (gx as i64 - preferred_x as i64).unsigned_abs()
                    })
                    .map(|(i, _)| i)
            }
        };
        let Some(adj_idx) = adj_idx else {
            return false;
        };

        // Resolve focused tile and pane ID.
        let src_focused_tile = match self.windows[src_idx].focused_pane {
            Some(t) => t,
            None => return false,
        };
        let src_pane_id = match self.windows[src_idx].tree.tiles.get(src_focused_tile) {
            Some(Tile::Pane(id)) => *id,
            _ => return false,
        };

        // Determine what focus moves to in the source window after removal.
        let next_src_focus = self.windows[src_idx].find_next_focus(src_focused_tile);

        // Detach the tile from the source window tree.
        let extracted_pane = {
            let ctx = &mut self.windows[src_idx];
            if let Some(parent_id) = ctx.tree.tiles.parent_of(src_focused_tile) {
                if let Some(Tile::Container(parent)) = ctx.tree.tiles.get_mut(parent_id) {
                    parent.remove_child(src_focused_tile);
                }
            }
            ctx.tree.tiles.remove(src_focused_tile);
            let pane = ctx.panes.remove(&src_pane_id);
            ctx.tree.simplify(&SimplificationOptions {
                all_panes_must_have_tabs: true,
                ..SimplificationOptions::default()
            });
            ctx.focused_pane = next_src_focus;
            pane
        };

        let Some(pane) = extracted_pane else {
            return false;
        };

        // If source window is now empty, delete it and adjust adj_idx for the
        // removed slot.
        let adj_idx = if self.windows[src_idx].panes.is_empty() {
            let adjusted = if adj_idx > src_idx { adj_idx - 1 } else { adj_idx };
            self.delete_window(src_idx);
            adjusted
        } else {
            adj_idx
        };

        // Switch active window to the destination.
        self.active_window = adj_idx;
        let new_ws_id = self.windows[adj_idx].context_id;
        let new_win_id = self.windows[adj_idx].window_id;
        self.last_page_x_per_row
            .insert(self.windows[adj_idx].grid_y, self.windows[adj_idx].grid_x);
        self.context_active_window.insert(new_ws_id, new_win_id);
        self.record_context_visit(new_win_id);

        // Insert pane into destination window at the incoming edge.
        // new_pane_first = true when entering from the left/top (pane leads).
        let new_pane_first = matches!(dir, Direction::Right | Direction::Down);
        let split_dir = if dx != 0 {
            egui_tiles::LinearDir::Horizontal
        } else {
            egui_tiles::LinearDir::Vertical
        };

        let ctx = &mut self.windows[adj_idx];
        let new_tile = ctx.tree.tiles.insert_pane(src_pane_id);
        ctx.panes.insert(src_pane_id, pane);

        if ctx.tree.root.is_none() {
            ctx.tree.root = Some(new_tile);
        } else if let Some(root) = ctx.tree.root {
            // If root is already a same-direction linear container, insert inline
            // to avoid unnecessary nesting (e.g. H[H[a,b], c] → H[a,b,c]).
            let inserted_inline = if let Some(Tile::Container(Container::Linear(linear))) =
                ctx.tree.tiles.get_mut(root)
            {
                if linear.dir == split_dir {
                    if new_pane_first {
                        linear.children.insert(0, new_tile);
                    } else {
                        linear.children.push(new_tile);
                    }
                    true
                } else {
                    false
                }
            } else {
                false
            };

            if !inserted_inline {
                let ordered = if new_pane_first {
                    vec![new_tile, root]
                } else {
                    vec![root, new_tile]
                };
                let container_tile = if dx != 0 {
                    ctx.tree.tiles.insert_horizontal_tile(ordered)
                } else {
                    ctx.tree.tiles.insert_vertical_tile(ordered)
                };
                ctx.tree.root = Some(container_tile);
            }
        }
        ctx.focused_pane = Some(new_tile);

        log::info!(
            "move_focused_pane_to_adjacent_window({:?}): pane {} moved from window index {} to {}",
            dir,
            src_pane_id,
            src_idx,
            adj_idx
        );

        true
    }

    pub(crate) fn scroll_focused_pane(&mut self, lines: i32) {
        let ctx = &mut self.windows[self.active_window];
        let Some(focused_tile) = ctx.focused_pane else {
            return;
        };
        let Some(Tile::Pane(pane_id)) = ctx.tree.tiles.get(focused_tile) else {
            return;
        };
        let pane_id = *pane_id;
        if let Some(pane) = ctx.panes.get_mut(&pane_id) {
            if let Some(t) = pane.as_terminal_mut() {
                t.backend.process_command(BackendCommand::Scroll(lines));
            }
        }
    }

    pub(crate) fn adjust_focused_pane_font_size(&mut self, delta: f32) {
        let ctx = &mut self.windows[self.active_window];
        let Some(focused_tile) = ctx.focused_pane else {
            return;
        };
        let Some(Tile::Pane(pane_id)) = ctx.tree.tiles.get(focused_tile) else {
            return;
        };
        let pane_id = *pane_id;
        if let Some(pane) = ctx.panes.get_mut(&pane_id) {
            if let Some(t) = pane.as_terminal_mut() {
                t.font_size = (t.font_size + delta).clamp(8.0, 32.0);
            }
        }
    }

    /// Close the focused app pane.
    pub(crate) fn close_focused_app(&mut self) {
        let active = self.active_window;
        let Some(focused_tile) = self.windows[active].focused_pane else {
            return;
        };
        let Some(pane_id) = self.windows[active]
            .tree
            .tiles
            .get(focused_tile)
            .and_then(|tile| match tile {
                Tile::Pane(pane_id) => Some(*pane_id),
                _ => None,
            })
        else {
            return;
        };
        if let Some(maybe_cwd) =
            restore_overlay_replacement(&mut self.windows[active].panes, pane_id)
        {
            if let Some(cwd) = maybe_cwd {
                let escaped = cwd.to_string_lossy().replace('\'', "'\\''");
                let cd_cmd = format!("\x15cd '{}'\n", escaped);
                if let Some(t) = self.windows[active]
                    .panes
                    .get_mut(&pane_id)
                    .and_then(|p| p.as_terminal_mut())
                {
                    t.backend.process_command(egui_term::BackendCommand::Write(
                        cd_cmd.as_bytes().to_vec(),
                    ));
                    log::info!(
                        "file_browser: synced cwd '{}' to terminal pane {}",
                        cwd.display(),
                        pane_id
                    );
                }
            }
            return;
        }

        let is_app = self.windows[active]
            .panes
            .get(&pane_id)
            .and_then(|p| p.as_app())
            .is_some();
        if is_app {
            self.close_tile(active, focused_tile);
            self.windows[active].zoomed_pane = None;
        }
    }

    /// Execute the close-pane action (called directly when confirm_close is false,
    /// or from the confirm-close dialog when the user confirms).
    pub(crate) fn execute_close_pane(&mut self) -> bool {
        self.windows[self.active_window].zoomed_pane = None;
        if !self.windows[self.active_window].panes.is_empty() {
            self.close_focused();
        }
        // If the window is now empty, only delete it when there are other pages
        // in the same context (i.e. it's one of several pages). When it's the
        // sole page, keep it alive so the welcome screen renders.
        if self.windows[self.active_window].panes.is_empty() {
            let ctx_id = self.windows[self.active_window].context_id;
            let pages_in_context = self.windows.iter().filter(|w| w.context_id == ctx_id).count();
            if pages_in_context > 1 {
                self.delete_window(self.active_window);
            }
        }
        false
    }
}

impl PlexiApp {
    pub(crate) fn resolve_new_pane_cwd(&self, cwd_override: Option<std::path::PathBuf>, focused: Option<TileId>) -> Option<std::path::PathBuf> {
        cwd_override
            .or_else(|| self.router.active().root.clone())
            .or_else(|| focused.and_then(|f| self.windows[self.active_window].get_focused_pane_cwd(f)))
            .or_else(dirs::home_dir)
    }

    /// CWD for the first terminal pane created from the welcome screen (empty window).
    /// Priority: context root → window launch path
    pub(crate) fn cwd_for_welcome_tab(&self) -> std::path::PathBuf {
        self.router
            .active()
            .root
            .clone()
            .unwrap_or_else(|| self.windows[self.active_window].path.clone())
    }
}

/// Test-only helper: pop focused pane unconditionally to a new window at max_x+1,
/// without the boundary check. Used by pop_pane_to_new_window_tests only.
#[cfg(test)]
impl PlexiApp {
    pub(crate) fn pop_focused_pane_to_new_window(&mut self) -> bool {
        use egui_tiles::Tile;

        let src_idx = self.active_window;

        let Some(focused_tile) = self.windows[src_idx].focused_pane else {
            return false;
        };

        let src_pane_id = match self.windows[src_idx].tree.tiles.get(focused_tile) {
            Some(Tile::Pane(id)) => *id,
            _ => return false,
        };

        let src_grid_y = self.windows[src_idx].grid_y;
        let ws_id = self.windows[src_idx].context_id;

        let cwd = self.windows[src_idx]
            .get_focused_pane_cwd(focused_tile)
            .or_else(|| self.router.active().root.clone())
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")));

        let next_src_focus = self.windows[src_idx].find_next_focus(focused_tile);

        let extracted_pane = {
            let ctx = &mut self.windows[src_idx];
            if let Some(parent_id) = ctx.tree.tiles.parent_of(focused_tile) {
                if let Some(Tile::Container(parent)) = ctx.tree.tiles.get_mut(parent_id) {
                    parent.remove_child(focused_tile);
                }
            }
            ctx.tree.tiles.remove(focused_tile);
            let pane = ctx.panes.remove(&src_pane_id);
            ctx.tree.simplify(&SimplificationOptions {
                all_panes_must_have_tabs: true,
                ..SimplificationOptions::default()
            });
            ctx.focused_pane = next_src_focus;
            pane
        };

        let Some(pane) = extracted_pane else {
            return false;
        };

        if self.windows[src_idx].panes.is_empty() && self.windows.len() > 1 {
            self.delete_window(src_idx);
        }

        let max_x = self
            .windows
            .iter()
            .filter(|w| w.context_id == ws_id && w.grid_y == src_grid_y)
            .map(|w| w.grid_x)
            .max();
        let new_x = max_x.map(|x| x + 1).unwrap_or(0);

        let mut new_tree = egui_tiles::Tree::empty(format!("tree_pop_{}", self.next_window_id));
        let new_tile = new_tree.tiles.insert_pane(src_pane_id);
        new_tree.root = Some(new_tile);
        let mut new_panes = HashMap::new();
        new_panes.insert(src_pane_id, pane);

        let win_id = self.next_window_id;
        self.next_window_id += 1;

        self.windows.push(crate::host::context::Window {
            name: String::new(),
            path: cwd,
            tree: new_tree,
            panes: new_panes,
            focused_pane: Some(new_tile),
            zoomed_pane: None,
            grid_x: new_x,
            grid_y: src_grid_y,
            window_id: win_id,
            context_id: ws_id,
        });

        self.active_window = self.windows.len() - 1;
        self.context_active_window.insert(ws_id, win_id);
        self.record_context_visit(win_id);
        self.minimap.visible = true;

        true
    }
}

#[cfg(test)]
mod close_pane_by_id_tests {
    use super::*;
    use std::collections::HashMap;

    fn test_app() -> PlexiApp {
        let ctx = egui::Context::default();
        let ft = crate::platform::logging::new_frame_tick();
        PlexiApp::new_for_test(ctx, ft).0
    }

    fn window_with_pane(context_id: u64, window_id: u64, pane_id: u64, grid_y: u32) -> crate::host::context::Window {
        let mut tree = egui_tiles::Tree::empty("test_tree_wid");
        let tile = tree.tiles.insert_pane(pane_id);
        tree.root = Some(tile);
        crate::host::context::Window {
            name: "test".into(),
            path: std::env::temp_dir(),
            tree,
            panes: HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 0,
            grid_y,
            window_id,
            context_id,
        }
    }

    /// Regression guard for #917: closing the last pane in a non-active window
    /// via IPC must delete the zombie window, not leave it in self.windows[].
    #[test]
    fn close_pane_by_id_removes_zombie_non_active_window() {
        let mut app = test_app();
        // window[0]: context 1, no panes (will show welcome screen — sole page, must stay)
        // window[1]: same context 1, one pane — closing it should delete window[1]
        let pane_id: u64 = 9001;
        app.windows.push(window_with_pane(1, 2, pane_id, 1));

        assert_eq!(app.windows.len(), 2);
        app.close_pane_by_id(pane_id);
        assert_eq!(app.windows.len(), 1, "zombie window must be deleted after IPC close of last pane");
    }

    /// Regression guard for #917: closing the last pane in the active window via IPC
    /// must delete that window and shift active_window to the remaining window.
    #[test]
    fn close_pane_by_id_removes_zombie_active_window() {
        let mut app = test_app();
        // window[0] (active): context 1, one pane
        let pane_id: u64 = 9002;
        let tile = app.windows[0].tree.tiles.insert_pane(pane_id);
        app.windows[0].tree.root = Some(tile);
        // window[1]: same context 1, one pane (stays after active is closed)
        app.windows.push(window_with_pane(1, 2, 9003, 1));

        assert_eq!(app.windows.len(), 2);
        assert_eq!(app.active_window, 0);
        app.close_pane_by_id(pane_id);
        assert_eq!(app.windows.len(), 1, "zombie active window must be deleted after IPC close of last pane");
        assert_eq!(app.active_window, 0, "active_window must point to the remaining window after deletion");
    }

    /// Sole-page window must stay alive when its last pane closes — welcome screen path.
    #[test]
    fn close_pane_by_id_keeps_sole_page_window_alive() {
        let mut app = test_app();
        let pane_id: u64 = 9004;
        let tile = app.windows[0].tree.tiles.insert_pane(pane_id);
        app.windows[0].tree.root = Some(tile);

        assert_eq!(app.windows.len(), 1);
        app.close_pane_by_id(pane_id);
        assert_eq!(app.windows.len(), 1, "sole-page window must remain alive showing welcome screen");
    }

    /// Closing the active tab of a background tab container must not steal focus.
    /// Regression guard for #1547 (Gemini-identified edge case).
    #[test]
    fn close_background_active_tab_does_not_steal_focus() {
        let mut app = test_app();
        let focused_id: u64 = 100;
        let bg_active_id: u64 = 101;
        let bg_inactive_id: u64 = 102;

        let tile_focused    = app.windows[0].tree.tiles.insert_pane(focused_id);
        let tile_bg_active  = app.windows[0].tree.tiles.insert_pane(bg_active_id);
        let tile_bg_inactive = app.windows[0].tree.tiles.insert_pane(bg_inactive_id);

        let tabs_tile = app.windows[0].tree.tiles.insert_tab_tile(
            vec![tile_bg_active, tile_bg_inactive],
        );
        if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))) =
            app.windows[0].tree.tiles.get_mut(tabs_tile)
        {
            tabs.set_active(tile_bg_active);
        }

        let container_tile = app.windows[0].tree.tiles.insert_horizontal_tile(
            vec![tile_focused, tabs_tile],
        );
        app.windows[0].tree.root = Some(container_tile);
        app.windows[0].focused_pane = Some(tile_focused);

        app.close_pane_by_id(bg_active_id);

        assert_eq!(
            app.windows[0].focused_pane,
            Some(tile_focused),
            "focus must remain on the focused pane after closing a background active tab",
        );
    }

    /// Closing a background pane must not steal focus from the currently focused pane.
    /// Regression guard for #1547.
    #[test]
    fn close_background_pane_does_not_steal_focus() {
        let mut app = test_app();
        // Build: Linear(H) -> [pane_focused(100), pane_bg(101), pane_extra(102)]
        let focused_id: u64 = 100;
        let bg_id: u64 = 101;
        let extra_id: u64 = 102;

        let tile_focused = app.windows[0].tree.tiles.insert_pane(focused_id);
        let tile_bg      = app.windows[0].tree.tiles.insert_pane(bg_id);
        let tile_extra   = app.windows[0].tree.tiles.insert_pane(extra_id);

        let container_tile = app.windows[0].tree.tiles.insert_horizontal_tile(
            vec![tile_focused, tile_bg, tile_extra],
        );
        app.windows[0].tree.root = Some(container_tile);
        app.windows[0].focused_pane = Some(tile_focused);

        // Close the non-focused background pane.
        app.close_pane_by_id(bg_id);

        assert_eq!(
            app.windows[0].focused_pane,
            Some(tile_focused),
            "focus must remain on the originally-focused pane after closing a background pane",
        );
    }

    /// Closing a background pane while another pane is zoomed must not corrupt zoom state.
    /// Regression guard for #1547 (cmd+Enter fullscreen bug).
    #[test]
    fn close_background_pane_preserves_zoom() {
        let mut app = test_app();
        let zoomed_id: u64 = 200;
        let bg_id: u64 = 201;

        let tile_zoomed = app.windows[0].tree.tiles.insert_pane(zoomed_id);
        let tile_bg     = app.windows[0].tree.tiles.insert_pane(bg_id);

        let container_tile = app.windows[0].tree.tiles.insert_horizontal_tile(
            vec![tile_zoomed, tile_bg],
        );
        app.windows[0].tree.root = Some(container_tile);
        app.windows[0].focused_pane = Some(tile_zoomed);
        app.windows[0].zoomed_pane = Some(tile_zoomed);

        // Close the background (non-zoomed) pane.
        app.close_pane_by_id(bg_id);

        assert_eq!(
            app.windows[0].zoomed_pane,
            Some(tile_zoomed),
            "zoom must remain on the originally-zoomed pane after closing a background pane",
        );
        assert_eq!(
            app.windows[0].focused_pane,
            Some(tile_zoomed),
            "focus must remain on the zoomed pane",
        );
    }
}

#[cfg(test)]
mod swap_tests {
    use super::*;

    fn test_app() -> PlexiApp {
        let ctx = egui::Context::default();
        let ft = crate::platform::logging::new_frame_tick();
        PlexiApp::new_for_test(ctx, ft).0
    }

    #[test]
    fn swap_pane_no_focus_returns_no_focus() {
        let mut app = test_app();
        assert!(matches!(app.swap_pane(Direction::Right), SwapResult::NoFocus));
    }

    #[test]
    fn swap_pane_at_boundary_with_single_pane() {
        let mut app = test_app();
        let tile = app.windows[0].tree.tiles.insert_pane(1u64);
        app.windows[0].focused_pane = Some(tile);
        // No neighbors (rects unset = Rect::ZERO, geometric search returns None)
        assert!(matches!(app.swap_pane(Direction::Right), SwapResult::AtBoundary));
    }
}

#[cfg(test)]
mod move_to_adjacent_window_tests {
    use super::*;
    use std::collections::HashMap;

    fn test_app() -> PlexiApp {
        let ctx = egui::Context::default();
        let ft = crate::platform::logging::new_frame_tick();
        PlexiApp::new_for_test(ctx, ft).0
    }

    fn make_app_pane(id: u64) -> crate::host::pane::Pane {
        use crate::app::permissions::AppPermissions;
        use crate::host::pane::{AppPane, AppRuntime};
        use crate::process_app::ProcessApp;
        let (process_app, _draw_tx) = ProcessApp::new_for_test(id, AppPermissions::builtin());
        crate::host::pane::Pane::App(Box::new(AppPane {
            id,
            runtime: AppRuntime::Process(Box::new(process_app)),
            workspace_root: std::env::temp_dir(),
            permissions: AppPermissions::builtin(),
            manifest_id: "test".to_string(),
            name: "Test".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            hidden: false,
        }))
    }

    fn window_at(context_id: u64, window_id: u64, grid_x: u32, grid_y: u32) -> crate::host::context::Window {
        crate::host::context::Window {
            name: "test".into(),
            path: std::env::temp_dir(),
            tree: egui_tiles::Tree::empty(format!("tree_{window_id}")),
            panes: HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x,
            grid_y,
            window_id,
            context_id,
        }
    }

    #[test]
    fn no_adjacent_window_returns_false() {
        let mut app = test_app();
        let pane_id: u64 = 42;
        let tile = app.windows[0].tree.tiles.insert_pane(pane_id);
        app.windows[0].tree.root = Some(tile);
        app.windows[0].focused_pane = Some(tile);
        app.windows[0].panes.insert(pane_id, make_app_pane(pane_id));

        assert!(!app.move_focused_pane_to_adjacent_window(Direction::Right));
        // State unchanged
        assert_eq!(app.active_window, 0);
        assert!(app.windows[0].panes.contains_key(&pane_id));
    }

    #[test]
    fn moves_pane_to_adjacent_window_deletes_empty_source() {
        let mut app = test_app();
        // Window 0: context 1, grid (0,0), one pane
        let pane_id: u64 = 42;
        let tile = app.windows[0].tree.tiles.insert_pane(pane_id);
        app.windows[0].tree.root = Some(tile);
        app.windows[0].focused_pane = Some(tile);
        app.windows[0].panes.insert(pane_id, make_app_pane(pane_id));
        // Window 1: context 1, grid (1,0), empty
        app.windows.push(window_at(1, 2, 1, 0));

        let moved = app.move_focused_pane_to_adjacent_window(Direction::Right);
        assert!(moved);

        // Source had only 1 pane → deleted; one window remains (destination, compacted to grid_x=0)
        assert_eq!(app.windows.len(), 1, "empty source window must be deleted");
        assert_eq!(app.active_window, 0);
        assert!(app.windows[0].panes.contains_key(&pane_id), "pane must be in destination");
        // compact_workspace_grid renumbers the surviving column to 0
        assert_eq!(app.windows[0].grid_x, 0);
    }

    #[test]
    fn moves_pane_source_survives_with_remaining_pane() {
        let mut app = test_app();
        // Window 0: context 1, grid (0,0), two panes
        let pane_a: u64 = 10;
        let pane_b: u64 = 20;
        let tile_a = app.windows[0].tree.tiles.insert_pane(pane_a);
        let tile_b = app.windows[0].tree.tiles.insert_pane(pane_b);
        let container = app.windows[0].tree.tiles.insert_horizontal_tile(vec![tile_a, tile_b]);
        app.windows[0].tree.root = Some(container);
        app.windows[0].focused_pane = Some(tile_a);
        app.windows[0].panes.insert(pane_a, make_app_pane(pane_a));
        app.windows[0].panes.insert(pane_b, make_app_pane(pane_b));
        // Window 1: context 1, grid (1,0), empty
        app.windows.push(window_at(1, 2, 1, 0));

        let moved = app.move_focused_pane_to_adjacent_window(Direction::Right);
        assert!(moved);

        // Two windows remain
        assert_eq!(app.windows.len(), 2);
        // Active window is the destination (index 1 is unchanged since source survived)
        assert_eq!(app.active_window, 1);
        assert_eq!(app.windows[1].grid_x, 1);
        assert!(app.windows[1].panes.contains_key(&pane_a), "moved pane in destination");
        // Source still has pane_b
        assert!(app.windows[0].panes.contains_key(&pane_b), "remaining pane still in source");
        assert!(!app.windows[0].panes.contains_key(&pane_a), "moved pane no longer in source");
    }
}

#[cfg(test)]
mod pop_pane_to_new_window_tests {
    use super::*;
    use std::collections::HashMap;

    fn test_app() -> PlexiApp {
        let ctx = egui::Context::default();
        let ft = crate::platform::logging::new_frame_tick();
        PlexiApp::new_for_test(ctx, ft).0
    }

    fn make_app_pane(id: u64) -> crate::host::pane::Pane {
        use crate::app::permissions::AppPermissions;
        use crate::host::pane::{AppPane, AppRuntime};
        use crate::process_app::ProcessApp;
        let (process_app, _draw_tx) = ProcessApp::new_for_test(id, AppPermissions::builtin());
        crate::host::pane::Pane::App(Box::new(AppPane {
            id,
            runtime: AppRuntime::Process(Box::new(process_app)),
            workspace_root: std::env::temp_dir(),
            permissions: AppPermissions::builtin(),
            manifest_id: "test".to_string(),
            name: "Test".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            hidden: false,
        }))
    }

    fn window_at(context_id: u64, window_id: u64, grid_x: u32, grid_y: u32) -> crate::host::context::Window {
        crate::host::context::Window {
            name: String::new(),
            path: std::env::temp_dir(),
            tree: egui_tiles::Tree::empty(format!("tree_{window_id}")),
            panes: HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x,
            grid_y,
            window_id,
            context_id,
        }
    }

    /// Single window with one pane: pop creates a second window; original stays alive (welcome screen).
    #[test]
    fn pop_single_pane_leaves_source_alive() {
        let mut app = test_app();
        let pane_id: u64 = 1;
        let tile = app.windows[0].tree.tiles.insert_pane(pane_id);
        app.windows[0].tree.root = Some(tile);
        app.windows[0].focused_pane = Some(tile);
        app.windows[0].panes.insert(pane_id, make_app_pane(pane_id));

        let popped = app.pop_focused_pane_to_new_window();
        assert!(popped);
        assert_eq!(app.windows.len(), 2, "new window must be created");
        // New window is active and has the pane.
        let new_win = &app.windows[app.active_window];
        assert!(new_win.panes.contains_key(&pane_id), "pane must be in new window");
        assert_eq!(new_win.grid_y, 0, "new window must be in the same row");
        assert!(new_win.grid_x > 0, "new window must be to the right of grid_x=0");
    }

    /// Single pane in window with a sibling window: source is deleted after pop.
    #[test]
    fn pop_sole_pane_deletes_empty_source() {
        let mut app = test_app();
        // Window 0 (active): context 1, grid (0,0), one pane.
        let pane_id: u64 = 42;
        let tile = app.windows[0].tree.tiles.insert_pane(pane_id);
        app.windows[0].tree.root = Some(tile);
        app.windows[0].focused_pane = Some(tile);
        app.windows[0].panes.insert(pane_id, make_app_pane(pane_id));
        // Window 1: context 1, grid (1,0), empty sibling so delete_window is allowed.
        app.windows.push(window_at(1, 2, 1, 0));

        let popped = app.pop_focused_pane_to_new_window();
        assert!(popped);
        // Original had 1 pane → deleted; the empty sibling + new window → 2 windows total.
        // After compact_workspace_grid the empty sibling at old x=1 becomes x=0.
        let pane_win = app.windows.iter().find(|w| w.panes.contains_key(&pane_id));
        assert!(pane_win.is_some(), "pane must be in one of the surviving windows");
        assert_eq!(app.active_window, app.windows.len() - 1, "active must be the newly created window");
    }

    /// Window with two panes: source survives with the remaining pane.
    #[test]
    fn pop_pane_source_survives_with_remaining_pane() {
        let mut app = test_app();
        let pane_a: u64 = 10;
        let pane_b: u64 = 20;
        let tile_a = app.windows[0].tree.tiles.insert_pane(pane_a);
        let tile_b = app.windows[0].tree.tiles.insert_pane(pane_b);
        let container = app.windows[0].tree.tiles.insert_horizontal_tile(vec![tile_a, tile_b]);
        app.windows[0].tree.root = Some(container);
        app.windows[0].focused_pane = Some(tile_a);
        app.windows[0].panes.insert(pane_a, make_app_pane(pane_a));
        app.windows[0].panes.insert(pane_b, make_app_pane(pane_b));

        let popped = app.pop_focused_pane_to_new_window();
        assert!(popped);
        assert_eq!(app.windows.len(), 2, "new window added, original survives");
        let new_win = &app.windows[app.active_window];
        assert!(new_win.panes.contains_key(&pane_a), "popped pane in new window");
        assert!(!new_win.panes.contains_key(&pane_b), "other pane stays in source");
        let src_win = app.windows.iter().find(|w| w.panes.contains_key(&pane_b)).unwrap();
        assert!(!src_win.panes.contains_key(&pane_a), "source no longer has popped pane");
    }

    /// New window is appended at max_grid_x + 1, not at a fixed offset.
    #[test]
    fn pop_appends_at_max_grid_x_plus_one() {
        let mut app = test_app();
        // Two existing windows at x=0 and x=1 in row 0.
        // Active is at x=0 with two panes (so it survives the pop).
        let pane_a: u64 = 1;
        let pane_b: u64 = 2;
        let tile_a = app.windows[0].tree.tiles.insert_pane(pane_a);
        let tile_b = app.windows[0].tree.tiles.insert_pane(pane_b);
        let container = app.windows[0].tree.tiles.insert_horizontal_tile(vec![tile_a, tile_b]);
        app.windows[0].tree.root = Some(container);
        app.windows[0].focused_pane = Some(tile_a);
        app.windows[0].panes.insert(pane_a, make_app_pane(pane_a));
        app.windows[0].panes.insert(pane_b, make_app_pane(pane_b));
        // Sibling at x=1
        app.windows.push(window_at(1, 2, 1, 0));

        let popped = app.pop_focused_pane_to_new_window();
        assert!(popped);
        let new_win = &app.windows[app.active_window];
        assert_eq!(new_win.grid_x, 2, "new window must be at max_x+1 = 2");
        assert_eq!(new_win.grid_y, 0);
    }

    /// No-op (returns false) when there is no focused pane.
    #[test]
    fn pop_no_focus_returns_false() {
        let mut app = test_app();
        // focused_pane is None by default in the initial window.
        assert!(!app.pop_focused_pane_to_new_window());
        assert_eq!(app.windows.len(), 1);
    }
}

#[cfg(test)]
mod move_to_row_boundary_tests {
    use super::*;

    fn test_app() -> PlexiApp {
        let ctx = egui::Context::default();
        let ft = crate::platform::logging::new_frame_tick();
        PlexiApp::new_for_test(ctx, ft).0
    }

    fn make_app_pane(id: u64) -> crate::host::pane::Pane {
        use crate::app::permissions::AppPermissions;
        use crate::host::pane::{AppPane, AppRuntime};
        use crate::process_app::ProcessApp;
        let (process_app, _draw_tx) = ProcessApp::new_for_test(id, AppPermissions::builtin());
        crate::host::pane::Pane::App(Box::new(AppPane {
            id,
            runtime: AppRuntime::Process(Box::new(process_app)),
            workspace_root: std::env::temp_dir(),
            permissions: AppPermissions::builtin(),
            manifest_id: "test".to_string(),
            name: "Test".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            hidden: false,
        }))
    }

    fn window_at(context_id: u64, window_id: u64, grid_x: u32, grid_y: u32) -> crate::host::context::Window {
        crate::host::context::Window {
            name: String::new(),
            path: std::env::temp_dir(),
            tree: egui_tiles::Tree::empty(format!("tree_{window_id}")),
            panes: HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x,
            grid_y,
            window_id,
            context_id,
        }
    }

    fn app_with_two_pane_window() -> (PlexiApp, u64, u64) {
        let mut app = test_app();
        let pane_a: u64 = 10;
        let pane_b: u64 = 20;
        let tile_a = app.windows[0].tree.tiles.insert_pane(pane_a);
        let tile_b = app.windows[0].tree.tiles.insert_pane(pane_b);
        let container = app.windows[0].tree.tiles.insert_horizontal_tile(vec![tile_a, tile_b]);
        app.windows[0].tree.root = Some(container);
        app.windows[0].focused_pane = Some(tile_a);
        app.windows[0].panes.insert(pane_a, make_app_pane(pane_a));
        app.windows[0].panes.insert(pane_b, make_app_pane(pane_b));
        (app, pane_a, pane_b)
    }

    /// Cmd+Ctrl+J (end=true): pane with siblings moves to new window at max_x+1.
    #[test]
    fn end_true_moves_pane_to_rightmost() {
        let (mut app, pane_a, _) = app_with_two_pane_window();
        // Add sibling window at x=1 so there's a clear max.
        app.windows.push(window_at(1, 99, 1, 0));

        let moved = app.move_focused_pane_to_row_boundary(true);
        assert!(moved);
        let new_win = &app.windows[app.active_window];
        assert!(new_win.panes.contains_key(&pane_a));
        assert_eq!(new_win.grid_x, 2, "new window at max_x+1");
    }

    /// Cmd+Ctrl+J (end=true): pane alone at rightmost position → edge pulse (returns false).
    #[test]
    fn end_true_already_alone_at_rightmost_returns_false() {
        let mut app = test_app();
        // Window 0 (active, grid_x=0) with one pane; window 1 at grid_x=1 with one pane.
        let pane_a: u64 = 1;
        let tile_a = app.windows[0].tree.tiles.insert_pane(pane_a);
        app.windows[0].tree.root = Some(tile_a);
        app.windows[0].focused_pane = Some(tile_a);
        app.windows[0].panes.insert(pane_a, make_app_pane(pane_a));

        let pane_b: u64 = 2;
        app.windows.push(window_at(1, 99, 1, 0));
        let tile_b = app.windows[1].tree.tiles.insert_pane(pane_b);
        app.windows[1].tree.root = Some(tile_b);
        app.windows[1].focused_pane = Some(tile_b);
        app.windows[1].panes.insert(pane_b, make_app_pane(pane_b));

        // Make window 1 (grid_x=1, the rightmost) the active window.
        app.active_window = 1;

        let moved = app.move_focused_pane_to_row_boundary(true);
        assert!(!moved, "already alone at rightmost — should edge pulse");
    }

    /// Cmd+Ctrl+K (end=false): pane with siblings moves to new window at grid_x=0.
    #[test]
    fn end_false_moves_pane_to_leftmost() {
        let (mut app, pane_a, _) = app_with_two_pane_window();
        // Add sibling window at x=1.
        app.windows.push(window_at(1, 99, 1, 0));

        let moved = app.move_focused_pane_to_row_boundary(false);
        assert!(moved);
        let new_win = &app.windows[app.active_window];
        assert!(new_win.panes.contains_key(&pane_a));
        assert_eq!(new_win.grid_x, 0, "new window at grid_x=0");
        // All other same-row windows must have shifted right by 1.
        for w in &app.windows {
            if !w.panes.contains_key(&pane_a) {
                assert!(w.grid_x >= 1, "other windows shifted right");
            }
        }
    }

    /// Cmd+Ctrl+K (end=false): pane alone at grid_x=0 → edge pulse (returns false).
    #[test]
    fn end_false_already_alone_at_leftmost_returns_false() {
        let mut app = test_app();
        let pane_a: u64 = 1;
        let tile_a = app.windows[0].tree.tiles.insert_pane(pane_a);
        app.windows[0].tree.root = Some(tile_a);
        app.windows[0].focused_pane = Some(tile_a);
        app.windows[0].panes.insert(pane_a, make_app_pane(pane_a));
        // Add sibling window at x=1.
        app.windows.push(window_at(1, 99, 1, 0));
        // Active is window 0 (grid_x=0) — already alone at leftmost.

        let moved = app.move_focused_pane_to_row_boundary(false);
        assert!(!moved, "already alone at leftmost — should edge pulse");
    }

    /// Source window with a single pane is deleted after the boundary jump.
    #[test]
    fn source_deleted_when_empty_after_jump() {
        let mut app = test_app();
        let pane_a: u64 = 1;
        let tile_a = app.windows[0].tree.tiles.insert_pane(pane_a);
        app.windows[0].tree.root = Some(tile_a);
        app.windows[0].focused_pane = Some(tile_a);
        app.windows[0].panes.insert(pane_a, make_app_pane(pane_a));
        // Add a sibling with its own pane so we can distinguish source deletion.
        let pane_b: u64 = 2;
        app.windows.push(window_at(1, 99, 1, 0));
        let tile_b = app.windows[1].tree.tiles.insert_pane(pane_b);
        app.windows[1].tree.root = Some(tile_b);
        app.windows[1].focused_pane = Some(tile_b);
        app.windows[1].panes.insert(pane_b, make_app_pane(pane_b));
        // Window 0 at x=0 is NOT rightmost (x=1 exists) and alone → move triggers.
        let moved = app.move_focused_pane_to_row_boundary(true);
        assert!(moved);
        // Source (x=0, sole pane) was deleted; sibling + new window remain (2 windows).
        assert_eq!(app.windows.len(), 2, "source deleted → 1 sibling + 1 new = 2 windows");
        // No window should still hold pane_a at a non-rightmost position.
        let new_win = app.windows.iter().find(|w| w.panes.contains_key(&pane_a)).unwrap();
        let max_x = app.windows.iter().map(|w| w.grid_x).max().unwrap();
        assert_eq!(new_win.grid_x, max_x, "pane_a must be in the rightmost window");
    }
}

#[cfg(test)]
mod context_root_cwd_tests {
    use super::*;

    fn test_app() -> PlexiApp {
        let ctx = egui::Context::default();
        let ft = crate::platform::logging::new_frame_tick();
        PlexiApp::new_for_test(ctx, ft).0
    }

    #[test]
    fn context_root_used_when_set() {
        let mut app = test_app();
        let root = std::path::PathBuf::from("/projects/myapp");
        app.router.get_mut(0).root = Some(root.clone());
        let cwd = app.resolve_new_pane_cwd(None, None);
        assert_eq!(cwd, Some(root));
    }

    #[test]
    fn cwd_override_takes_priority_over_context_root() {
        let mut app = test_app();
        let root = std::path::PathBuf::from("/projects/myapp");
        let override_dir = std::path::PathBuf::from("/override/dir");
        app.router.get_mut(0).root = Some(root);
        let cwd = app.resolve_new_pane_cwd(Some(override_dir.clone()), None);
        assert_eq!(cwd, Some(override_dir));
    }

    #[test]
    fn context_root_beats_focused_pane_cwd() {
        use crate::app::permissions::AppPermissions;
        use crate::host::pane::{AppPane, AppRuntime};
        use crate::process_app::ProcessApp;

        let mut app = test_app();
        let root = std::path::PathBuf::from("/projects/myapp");
        app.router.get_mut(0).root = Some(root.clone());
        let pane_id: u64 = 42;
        let (process_app, _tx) = ProcessApp::new_for_test(pane_id, AppPermissions::builtin());
        let app_pane = AppPane {
            id: pane_id,
            runtime: AppRuntime::Process(Box::new(process_app)),
            workspace_root: std::path::PathBuf::from("/other/workspace"),
            permissions: AppPermissions::builtin(),
            manifest_id: "test".to_string(),
            name: "Test".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            hidden: false,
        };
        let tile = app.windows[0].tree.tiles.insert_pane(pane_id);
        app.windows[0].tree.root = Some(tile);
        app.windows[0].focused_pane = Some(tile);
        app.windows[0].panes.insert(pane_id, Pane::App(Box::new(app_pane)));
        let pane_cwd = app.windows[0].get_focused_pane_cwd(tile);
        assert_eq!(pane_cwd, Some(std::path::PathBuf::from("/other/workspace")), "pane has its own CWD");
        let cwd = app.resolve_new_pane_cwd(None, Some(tile));
        assert_eq!(cwd, Some(root), "context root must take priority over focused pane CWD");
    }

    #[test]
    fn no_cwd_sources_falls_back_to_home_dir() {
        let app = test_app();
        assert!(app.router.active().root.is_none());
        let cwd = app.resolve_new_pane_cwd(None, None);
        assert_eq!(cwd, dirs::home_dir(), "no context root and no focused pane falls back to home dir");
    }

    #[test]
    fn split_from_portal_tile_falls_back_to_home_dir() {
        use crate::host::pane::PortalPane;

        let mut app = test_app();
        assert!(app.router.active().root.is_none());
        let pane_id: u64 = 99;
        let portal = PortalPane { pane_id, target_context_id: 1, context_state: None, hidden: false };
        let tile = app.windows[0].tree.tiles.insert_pane(pane_id);
        app.windows[0].tree.root = Some(tile);
        app.windows[0].focused_pane = Some(tile);
        app.windows[0].panes.insert(pane_id, Pane::Portal(Box::new(portal)));
        let cwd = app.resolve_new_pane_cwd(None, Some(tile));
        assert_eq!(cwd, dirs::home_dir(), "splitting while a Portal tile is focused must fall back to home, not /");
    }

    #[test]
    fn welcome_tab_falls_back_to_home_dir_when_no_root() {
        let mut app = test_app();
        app.windows[0].path = std::path::PathBuf::from("/tmp/test-dir");
        assert!(app.router.active().root.is_none());
        let cwd = app.cwd_for_welcome_tab();
        assert_eq!(cwd, dirs::home_dir().unwrap(), "welcome tab uses home dir when context root is None");
    }

    #[test]
    fn welcome_tab_prefers_context_root_over_window_path() {
        let mut app = test_app();
        let root_dir = std::path::PathBuf::from("/tmp/root-dir");
        app.router.get_mut(0).root = Some(root_dir.clone());
        app.windows[0].path = std::path::PathBuf::from("/tmp/window-dir");
        let cwd = app.cwd_for_welcome_tab();
        assert_eq!(cwd, root_dir, "context root must take priority over window.path");
    }

    #[test]
    fn welcome_tab_never_returns_root_slash() {
        let app = test_app();
        // root is None, window.path is temp_dir (set by new_for_test)
        assert!(app.router.active().root.is_none());
        let cwd = app.cwd_for_welcome_tab();
        assert_ne!(cwd, std::path::PathBuf::from("/"), "should never fall through to filesystem root");
    }
}

#[cfg(test)]
mod navigate_boundary_tests {
    use super::*;
    use std::collections::HashMap;

    fn test_app() -> PlexiApp {
        let ctx = egui::Context::default();
        let ft = crate::platform::logging::new_frame_tick();
        PlexiApp::new_for_test(ctx, ft).0
    }

    fn make_app_pane(id: u64) -> crate::host::pane::Pane {
        use crate::app::permissions::AppPermissions;
        use crate::host::pane::{AppPane, AppRuntime};
        use crate::process_app::ProcessApp;
        let (process_app, _draw_tx) = ProcessApp::new_for_test(id, AppPermissions::builtin());
        crate::host::pane::Pane::App(Box::new(AppPane {
            id,
            runtime: AppRuntime::Process(Box::new(process_app)),
            workspace_root: std::env::temp_dir(),
            permissions: AppPermissions::builtin(),
            manifest_id: "test".to_string(),
            name: "Test".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            hidden: false,
        }))
    }

    fn window_at(context_id: u64, window_id: u64, grid_x: u32, grid_y: u32) -> crate::host::context::Window {
        crate::host::context::Window {
            name: String::new(),
            path: std::env::temp_dir(),
            tree: egui_tiles::Tree::empty(format!("tree_{window_id}")),
            panes: HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x,
            grid_y,
            window_id,
            context_id,
        }
    }

    // Navigate UP from window at grid_y=1 → active_window switches to grid_y=0 window.
    // No rects set → focused_pane in destination left as None (fallback path).
    #[test]
    fn up_boundary_jumps_to_first_window() {
        let mut app = test_app();
        // Window 0 (initial, context 1, grid_y=0) — destination after UP boundary jump
        let dest_pane: u64 = 1;
        let dest_tile = app.windows[0].tree.tiles.insert_pane(dest_pane);
        app.windows[0].tree.root = Some(dest_tile);
        app.windows[0].panes.insert(dest_pane, make_app_pane(dest_pane));
        app.windows[0].grid_x = 0;
        app.windows[0].grid_y = 0;

        // Window 1 (active source, grid_y=1)
        let mut src_win = window_at(1, 2, 0, 1);
        let src_pane: u64 = 2;
        let src_tile = src_win.tree.tiles.insert_pane(src_pane);
        src_win.tree.root = Some(src_tile);
        src_win.focused_pane = Some(src_tile);
        src_win.panes.insert(src_pane, make_app_pane(src_pane));
        app.windows.push(src_win);
        app.active_window = 1;

        app.navigate(Direction::Up);

        assert_eq!(app.active_window, 0, "should have jumped to first window");
        // No rects available → focused_pane is unchanged (still None from setup)
        assert_eq!(app.windows[0].focused_pane, None, "no rects — focused_pane left unchanged");
    }

    // Same jump but the destination already has a focused_pane set.
    // No rects → focused_pane is preserved (not overwritten).
    #[test]
    fn up_boundary_preserves_focused_pane_when_no_rects() {
        let mut app = test_app();
        // Window 0 (dest, grid_y=0): two tiles, focused_pane pre-set to tile_b
        let pane_a: u64 = 10;
        let pane_b: u64 = 20;
        let tile_a = app.windows[0].tree.tiles.insert_pane(pane_a);
        let tile_b = app.windows[0].tree.tiles.insert_pane(pane_b);
        let container = app.windows[0].tree.tiles.insert_horizontal_tile(vec![tile_a, tile_b]);
        app.windows[0].tree.root = Some(container);
        app.windows[0].focused_pane = Some(tile_b); // previously-focused pane
        app.windows[0].panes.insert(pane_a, make_app_pane(pane_a));
        app.windows[0].panes.insert(pane_b, make_app_pane(pane_b));
        app.windows[0].grid_x = 0;
        app.windows[0].grid_y = 0;

        // Window 1 (active source, grid_y=1)
        let mut src_win = window_at(1, 2, 0, 1);
        let src_pane: u64 = 30;
        let src_tile = src_win.tree.tiles.insert_pane(src_pane);
        src_win.tree.root = Some(src_tile);
        src_win.focused_pane = Some(src_tile);
        src_win.panes.insert(src_pane, make_app_pane(src_pane));
        app.windows.push(src_win);
        app.active_window = 1;

        app.navigate(Direction::Up);

        assert_eq!(app.active_window, 0, "should jump to first window");
        // No rects available → fallback, focused_pane unchanged at tile_b
        assert_eq!(app.windows[0].focused_pane, Some(tile_b), "no rects — focused_pane left unchanged");
    }
}
