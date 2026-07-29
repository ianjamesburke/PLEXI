//! Layout operations on already-created panes: split, tab, close, navigate,
//! zoom-free tree manipulation, font size, scroll.

use crate::app::PlexiApp;
use crate::host::command::{HostAction, Placement};
use crate::host::context::{Window, replace_child};
use crate::host::effect::HostEffect;
use crate::host::keys::Direction;
use crate::host::pane::{Pane, TerminalPane};
use crate::spatial::tiling::PaneId;
use egui_term::BackendCommand;
use egui_tiles::{Container, SimplificationOptions, Tile, TileId};
use std::{collections::HashMap, path::PathBuf};

pub(crate) enum SwapResult {
    Swapped {
        rect_a: egui::Rect,
        rect_b: egui::Rect,
    },
    AtBoundary,
    NoFocus,
}

/// If `pane_id` is an app pane that was opened as an overlay over another
/// pane (`hint = Some("overlay")`), restore the original pane and return
/// `true`. Otherwise return `false` and leave the map unchanged.
///
/// Restoring NEVER writes into the restored terminal. Cwd handoff is an
/// explicit `AppCommand::CdRequest` only (#2145) — a coding agent may be
/// running in the terminal behind the overlay.
pub(super) fn restore_overlay_replacement(
    panes: &mut HashMap<PaneId, Pane>,
    pane_id: PaneId,
) -> bool {
    let Some(pane) = panes.remove(&pane_id) else {
        return false;
    };

    match pane {
        Pane::App(mut app) => {
            if let Some(replaced) = app.overlay_replaced.take() {
                let type_id = app.runtime.type_id().to_string();
                crate::host::event_log::emit(crate::host::event_log::HostEvent::AppClosed {
                    app_id: type_id.clone(),
                    type_id,
                    pane_id,
                    reason: Some("overlay_restored".to_string()),
                    timestamp: crate::host::event_log::now_timestamp(),
                });
                panes.insert(pane_id, *replaced);
                true
            } else {
                panes.insert(pane_id, Pane::App(app));
                false
            }
        }
        other => {
            panes.insert(pane_id, other);
            false
        }
    }
}

/// Build a fresh tile tree holding `pane_ids` arranged per `layout`.
/// Pure tile manipulation — no PlexiApp state, no pane creation.
///
/// A single pane becomes the root directly (no container wrapper), matching the
/// shape `create_single_pane_tree` produces. Returns the tree plus the root tile
/// of the *first* pane, which callers use as the window's initial focus.
///
/// Panics if `pane_ids` is empty — every caller seeds at least one pane, and a
/// rootless window is not a state the host can render.
pub(crate) fn build_squad_tree(
    pane_ids: &[PaneId],
    layout: crate::app_protocol::SubContextLayout,
) -> (egui_tiles::Tree<PaneId>, TileId) {
    assert!(
        !pane_ids.is_empty(),
        "build_squad_tree requires at least one pane"
    );
    let mut tiles = egui_tiles::Tiles::default();
    let pane_tiles: Vec<TileId> = pane_ids.iter().map(|id| tiles.insert_pane(*id)).collect();
    let first_tile = pane_tiles[0];
    let root = if pane_tiles.len() == 1 {
        first_tile
    } else {
        match layout {
            crate::app_protocol::SubContextLayout::Tiled => tiles.insert_grid_tile(pane_tiles),
            crate::app_protocol::SubContextLayout::Columns => {
                tiles.insert_horizontal_tile(pane_tiles)
            }
        }
    };
    (egui_tiles::Tree::new("plexi", root, tiles), first_tile)
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
    let split_dir = if vertical {
        LinearDir::Horizontal
    } else {
        LinearDir::Vertical
    };
    let parent = tree.tiles.parent_of(target);

    let inserted_as_sibling = if let Some(parent_id) = parent {
        if let Some(Tile::Container(Container::Linear(linear))) = tree.tiles.get_mut(parent_id) {
            if linear.dir == split_dir {
                if let Some(pos) = linear.children.iter().position(|&c| c == target) {
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
            vec![new_tile, target]
        } else {
            vec![target, new_tile]
        };
        let container_tile = if vertical {
            tree.tiles.insert_horizontal_tile(ordered)
        } else {
            tree.tiles.insert_vertical_tile(ordered)
        };
        if let Some(Tile::Container(Container::Linear(ref mut lin))) =
            tree.tiles.get_mut(container_tile)
        {
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
        let new_tile = insert_split_tile(
            &mut ctx.tree,
            Some(split_target),
            new_pane_id,
            vertical,
            share,
            new_pane_first,
        );

        if !keep_focus {
            ctx.navigate_to(new_tile);
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
                let context = self.pane_context_env_for_window(active);
                if let Some((tree, panes, root_tile)) =
                    self.create_single_pane_tree(&context, None, None, false)
                {
                    self.windows[active].tree = tree;
                    self.windows[active].panes = panes;
                    self.set_window_focused_pane(active, root_tile);
                }
            }
            return;
        };

        let Some(Tile::Pane(focused_pane_id)) = self.windows[active].tree.tiles.get(focused_tile)
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
                //
                // Force a fresh spawn (#0336): a mirror-split of a singleton
                // (`focus_existing`) app must still duplicate the pane, so it
                // bypasses the on_launch dedup policy that would otherwise focus
                // the original and silently no-op the split.
                let layout = if vertical { "split_h" } else { "split_v" };
                let _ = self.launch_app_by_id_with_layout_forced(
                    &manifest_id,
                    Some(layout.to_string()),
                    &[],
                    None,
                );
            }
        }
    }

    /// Split the focused pane and spawn a terminal in the new slot. Returns
    /// the new pane's id, or `None` when nothing was spawned (no focused pane,
    /// or terminal creation failed).
    pub(crate) fn split_focused(
        &mut self,
        vertical: bool,
        initial_cmd: Option<&str>,
        close_on_exit: bool,
        new_pane_first: bool,
        cwd_override: Option<std::path::PathBuf>,
    ) -> Option<PaneId> {
        let old_window_id = self.windows[self.active_window].window_id;
        let old_focus = self.windows[self.active_window].focused_pane;
        let Some(focused) = self.windows[self.active_window].focused_pane else {
            return None;
        };

        let cmd = if vertical {
            HostAction::SplitVertical
        } else {
            HostAction::SplitHorizontal
        };
        let effects = self.submit(cmd);
        log::debug!(
            "split_focused(vertical={vertical} new_pane_first={new_pane_first}) effects: {:?}",
            effects
        );
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
        log::info!(
            "split_focused: cwd={cwd:?} context_root={:?}",
            self.router.active().root
        );
        let ctx_id = self
            .windows
            .get(self.active_window)
            .map(|w| w.context_id)
            .unwrap_or(0);
        let ctx_name = self.context_name_for(ctx_id);
        let ctx_desc = self.context_description_for(ctx_id);
        let ctx_root = self.context_root_for(ctx_id);
        let ctx_depth = self.context_depth_for(ctx_id);
        let mut settings = Self::make_backend_settings(
            new_id,
            cwd,
            &self.colors,
            ctx_id,
            &ctx_name,
            &ctx_desc,
            ctx_root.as_ref(),
            ctx_depth,
        );
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
            return None;
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

        ctx.navigate_to(new_tile);
        Some(new_id)
    }

    pub(crate) fn new_tab(
        &mut self,
        target_win_idx: usize,
        initial_cmd: Option<&str>,
        close_on_exit: bool,
        cwd_override: Option<PathBuf>,
    ) {
        // Empty context (welcome screen): create the first pane as tree root.
        if self.windows[target_win_idx].panes.is_empty() {
            let new_id = self.host.alloc_pane_id();
            let ctx_id = self
                .windows
                .get(target_win_idx)
                .map(|w| w.context_id)
                .unwrap_or(0);
            let ctx_name = self.context_name_for(ctx_id);
            let ctx_desc = self.context_description_for(ctx_id);
            let ctx_root = self.context_root_for(ctx_id);
            let ctx_depth = self.context_depth_for(ctx_id);
            // Resolve against the target window's own context, not the ambient
            // active context — target_win_idx may differ from self.active_window.
            let cwd = cwd_override
                .or_else(|| ctx_root.clone())
                .unwrap_or_else(|| {
                    dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"))
                });
            log::info!(
                "new_tab (empty context): target_win_idx={target_win_idx} cwd={cwd:?} context_root={ctx_root:?}"
            );
            let mut settings = Self::make_backend_settings(
                new_id,
                Some(cwd),
                &self.colors,
                ctx_id,
                &ctx_name,
                &ctx_desc,
                ctx_root.as_ref(),
                ctx_depth,
            );
            if let Some(cmd) = initial_cmd {
                log::info!(
                    "new_tab (empty context): initial_cmd={cmd:?} close_on_exit={close_on_exit}"
                );
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
            let ctx = &mut self.windows[target_win_idx];
            ctx.panes.insert(new_id, Pane::Terminal(Box::new(pane)));
            let pane_tile = ctx.tree.tiles.insert_pane(new_id);
            let tab_tile = ctx.tree.tiles.insert_tab_tile(vec![pane_tile]);
            ctx.tree.root = Some(tab_tile);
            ctx.navigate_to(pane_tile);
            return;
        }

        let old_window_id = self.windows[target_win_idx].window_id;
        let old_focus = self.windows[target_win_idx].focused_pane;
        let Some(focused) = self.windows[target_win_idx].focused_pane else {
            return;
        };

        let new_id = self.host.alloc_pane_id();

        let ctx_id = self
            .windows
            .get(target_win_idx)
            .map(|w| w.context_id)
            .unwrap_or(0);
        let ctx_name = self.context_name_for(ctx_id);
        let ctx_desc = self.context_description_for(ctx_id);
        let ctx_root = self.context_root_for(ctx_id);
        let ctx_depth = self.context_depth_for(ctx_id);

        // Resolve against the target window's own context/pane, not the ambient
        // active window — target_win_idx may differ from self.active_window when
        // anchored to a caller pane in another window.
        let cwd = cwd_override
            .or_else(|| ctx_root.clone())
            .or_else(|| self.windows[target_win_idx].get_focused_pane_cwd(focused))
            .or_else(dirs::home_dir);
        log::info!(
            "new_tab: target_win_idx={target_win_idx} cwd={cwd:?} context_root={ctx_root:?}"
        );
        let mut settings = Self::make_backend_settings(
            new_id,
            cwd,
            &self.colors,
            ctx_id,
            &ctx_name,
            &ctx_desc,
            ctx_root.as_ref(),
            ctx_depth,
        );
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
        self.windows[target_win_idx]
            .panes
            .insert(new_id, Pane::Terminal(Box::new(pane)));
        self.push_focus_history(old_window_id, old_focus);

        let ctx = &mut self.windows[target_win_idx];
        let new_tile = ctx.tree.tiles.insert_pane(new_id);

        if let Some((tabs_id, _)) = ctx.find_ancestor_tabs(focused) {
            if let Some(Tile::Container(Container::Tabs(tabs))) = ctx.tree.tiles.get_mut(tabs_id) {
                tabs.add_child(new_tile);
                tabs.set_active(new_tile);
            }
            ctx.navigate_to(new_tile);
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

        ctx.navigate_to(new_tile);
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
            if ctx.zoomed_pane.is_some() {
                ctx.zoom_to(pane_tile);
            } else {
                ctx.navigate_to(pane_tile);
            }
        }
    }

    pub(crate) fn switch_to_tab(&mut self, container_tile: TileId, idx: usize) {
        let ctx = &self.windows[self.active_window];
        let Some(Tile::Container(Container::Tabs(tabs))) = ctx.tree.tiles.get(container_tile)
        else {
            return;
        };
        let children = &tabs.children;
        if idx >= children.len() {
            return;
        }
        let target = children[idx];

        let ctx = &mut self.windows[self.active_window];
        if let Some(Tile::Container(Container::Tabs(tabs))) = ctx.tree.tiles.get_mut(container_tile)
        {
            tabs.set_active(target);
        }

        if let Some(pane_tile) = ctx.find_first_pane_in(target) {
            if ctx.zoomed_pane.is_some() {
                ctx.zoom_to(pane_tile);
            } else {
                ctx.navigate_to(pane_tile);
            }
        }
        log::info!("tab_click: switched to tab index={idx} in container={container_tile:?}");
    }

    pub(crate) fn reorder_tab(
        &mut self,
        container_tile: TileId,
        from_idx: usize,
        to_idx: usize,
    ) -> bool {
        let ctx = &mut self.windows[self.active_window];
        let Some(Tile::Container(Container::Tabs(tabs))) = ctx.tree.tiles.get_mut(container_tile)
        else {
            return false;
        };
        let len = tabs.children.len();
        if from_idx >= len || to_idx >= len || from_idx == to_idx {
            return false;
        }

        let moved = tabs.children.remove(from_idx);
        tabs.children.insert(to_idx, moved);
        log::info!("tab_reorder: container={container_tile:?} from_idx={from_idx} to_idx={to_idx}");
        true
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
            if restore_overlay_replacement(&mut self.windows[self.active_window].panes, pane_id) {
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
                let ctx_id = self.windows[ctx_idx].context_id;
                self.close_tile(ctx_idx, tile_id);
                // Mirror the guard in execute_close_pane: if the window is now empty
                // and there are other pages in the same context, delete the zombie window.
                // Without this, the window stays in self.windows[] as a phantom grid cell.
                if self.windows[ctx_idx].panes.is_empty() {
                    let pages_in_context = self
                        .windows
                        .iter()
                        .filter(|w| w.context_id == ctx_id)
                        .count();
                    if pages_in_context > 1 {
                        log::info!(
                            "close_pane_by_id: window {ctx_idx} empty, {pages_in_context} pages in context {ctx_id} — deleting zombie window"
                        );
                        self.delete_window(ctx_idx);
                    }
                }
                // An emptied subcontext collapses entirely — delete it and,
                // when it was active, zoom back out like Cmd+Escape.
                self.collapse_subcontext_if_empty(ctx_id);
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
        let mut closed_pane_id = None;
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
                closed_pane_id = Some(pane_id);
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
                ctx.navigate_to(new_focus);
            } else if is_focused {
                // Closed tile was focused but no sibling found — clear focus.
                log::info!("close_tile: focus -> None (no sibling)");
                ctx.focused_pane = None;
            }

            // Clear zoom if the zoomed tile was closed. Render-time validation also does
            // this, but clearing it here avoids a one-frame inconsistency where
            // ToggleZoom sees zoomed_pane pointing at a dead tile.
            if is_zoomed {
                ctx.clear_zoom();
                log::info!("close_tile: cleared stale zoom (closed tile was zoomed)");
            }

            removed
        };

        // ctx borrow is released — reap any routine live-run record for this
        // pane (the destroy half of the overlap guard's register/reap pair;
        // without it a closed run would block its routine forever), then park
        // background WASM app runtimes; drop everything else.
        if let Some(pane_id) = closed_pane_id {
            if self.pane_heartbeats.remove(&pane_id).is_some() {
                log::info!("pane_heartbeat: pane_id={pane_id} removed reason=closed");
            }
            for name in self.scheduler.reap_pane(pane_id) {
                log::info!("scheduler: routine '{name}' run ended — pane {pane_id} closed");
            }
        }
        match removed_pane {
            Some(Pane::App(app_pane)) => {
                let pane_id = app_pane.id;
                // Hot reload (#83): drop any active watcher for this pane.
                // Idempotent — no-op when the pane wasn't being watched.
                self.hot_reload.unwatch(pane_id);
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
            Direction::Left => (-1, 0),
            Direction::Right => (1, 0),
            Direction::Up => (0, -1),
            Direction::Down => (0, 1),
        };

        let ctx = &self.windows[self.active_window];
        let pane_neighbor = ctx
            .focused_pane
            .and_then(|focused| ctx.find_pane_in_direction_from(focused, dir));

        if let Some(target) = pane_neighbor {
            self.windows[self.active_window].navigate_to(target);
            // Signal the newly-focused pane so render_text_inputs auto-focuses
            // the first TextInput on the next frame.
        } else if dy != 0 {
            // Vertical boundary: jump to the first or last window in the current
            // workspace (the minimap list). Down at bottom → last window;
            // Up at top → first window. This is a list-end jump, not a one-step move.
            let ws_id = self.router.active().context_id;
            let jump_idx = if dy < 0 {
                self.windows
                    .iter()
                    .enumerate()
                    .filter(|(_, w)| w.context_id == ws_id)
                    .min_by_key(|(_, w)| (w.grid_y, w.grid_x))
                    .map(|(i, _)| i)
            } else {
                self.windows
                    .iter()
                    .enumerate()
                    .filter(|(_, w)| w.context_id == ws_id)
                    .max_by_key(|(_, w)| (w.grid_y, w.grid_x))
                    .map(|(i, _)| i)
            };
            if let Some(idx) = jump_idx {
                if idx != self.active_window {
                    log::info!(
                        "navigate({:?}): jumping to {} window in workspace",
                        dir,
                        if dy < 0 { "first" } else { "last" }
                    );
                    self.active_window = idx;
                    let w = &self.windows[idx];
                    let wid = w.window_id;
                    self.context_active_window.insert(ws_id, wid);
                    self.record_context_visit(wid);
                    // Focus the spatially leftmost pane in the destination window.
                    // If the window has not yet been rendered (no rects), leave focused_pane as-is.
                    let leftmost: Option<TileId> = {
                        let dest = &self.windows[idx];
                        dest.tree
                            .active_tiles()
                            .into_iter()
                            .filter_map(|tile_id| {
                                if !matches!(dest.tree.tiles.get(tile_id), Some(Tile::Pane(_))) {
                                    return None;
                                }
                                let rect = dest.tree.tiles.rect(tile_id)?;
                                Some((tile_id, rect))
                            })
                            .min_by(|(_, a), (_, b)| {
                                a.left()
                                    .partial_cmp(&b.left())
                                    .unwrap_or(std::cmp::Ordering::Equal)
                                    .then(
                                        a.top()
                                            .partial_cmp(&b.top())
                                            .unwrap_or(std::cmp::Ordering::Equal),
                                    )
                            })
                            .map(|(tile_id, _)| tile_id)
                    };
                    if let Some(tile_id) = leftmost {
                        log::info!("navigate({:?}): focused_pane → leftmost {:?}", dir, tile_id);
                        self.windows[idx].navigate_to(tile_id);
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
        ctx.navigate_to(neighbor);

        log::info!(
            "swap_pane({:?}): pane {} ↔ pane {} (tiles {:?} ↔ {:?})",
            dir,
            pane_a,
            pane_b,
            focused,
            neighbor
        );

        SwapResult::Swapped { rect_a, rect_b }
    }

    /// Send the focused pane in `dir` without following focus.
    ///
    /// Within-window: swaps tile contents like `swap_pane` but keeps focus on
    /// the **original tile** — the neighbor's pane ends up under the cursor.
    ///
    /// At boundary (no neighbor within this window):
    /// - L/R: delegates to `send_pane_to_adjacent_window` — pane moves to the
    ///   adjacent grid window; focus stays on whatever pane remains in the source
    ///   window. If the source becomes empty, returns `AtBoundary` (edge-pulse).
    /// - U/D: always returns `AtBoundary` (edge-pulse only; no row-boundary move).
    pub(crate) fn send_pane(&mut self, dir: Direction) -> SwapResult {
        use egui_tiles::Tile;
        let active = self.active_window;
        let ctx = &mut self.windows[active];

        let Some(focused) = ctx.focused_pane else {
            return SwapResult::NoFocus;
        };

        let Some(neighbor) = ctx.find_pane_in_direction_from(focused, dir) else {
            log::info!("send_pane({:?}): at boundary within window", dir);
            // For L/R, try cross-window send. For U/D, edge-pulse.
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

        // Focus stays on the original tile (now containing pane_b).
        // focused_pane is unchanged — it already points at `focused`.

        log::info!(
            "send_pane({:?}): pane {} ↔ pane {} (tiles {:?} ↔ {:?}), focus stays at {:?}",
            dir,
            pane_a,
            pane_b,
            focused,
            neighbor,
            focused
        );

        SwapResult::Swapped { rect_a, rect_b }
    }

    /// Move the focused pane to an adjacent grid window without following focus.
    ///
    /// Unlike `move_focused_pane_to_adjacent_window`, the active window stays
    /// on the source and focus stays on whatever pane fills the vacated slot.
    ///
    /// Returns `true` if the pane was moved (source still has panes remaining),
    /// `false` if there is no adjacent window or the source would become empty
    /// (caller shows the edge pulse for both cases).
    pub(crate) fn send_pane_to_adjacent_window(&mut self, dir: Direction) -> bool {
        let src_idx = self.active_window;

        // Peek: does the source window have more than one pane?
        // If it has only one pane, moving it out empties the source — no pane
        // to stay focused on, so we edge-pulse instead.
        if self.windows[src_idx].panes.len() <= 1 {
            log::info!(
                "send_pane_to_adjacent_window({:?}): source would become empty — edge pulse",
                dir
            );
            return false;
        }

        // Determine next_src_focus before calling move_focused_pane_to_adjacent_window
        // (the function sets it internally, but we need active_window to remain src_idx).
        let focused_tile = match self.windows[src_idx].focused_pane {
            Some(t) => t,
            None => return false,
        };
        let next_src_focus = self.windows[src_idx].find_next_focus(focused_tile);

        // Delegate the actual move (this sets active_window = adj_idx).
        let moved = self.move_focused_pane_to_adjacent_window(dir);
        if !moved {
            return false;
        }

        // Restore active window to source (which survived since it had >1 pane).
        // find the source by its previously known index — it may have shifted if adj
        // was deleted, but since source had >1 pane it was never deleted.
        self.active_window = src_idx;

        // Restore focus to the pane that filled the vacated slot in the source.
        self.windows[src_idx].focused_pane = next_src_focus;

        log::info!(
            "send_pane_to_adjacent_window({:?}): pane sent, focus stays at source window",
            dir
        );

        true
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
            Direction::Left => (-1, 0),
            Direction::Right => (1, 0),
            Direction::Up => (0, -1),
            Direction::Down => (0, 1),
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
                    .min_by_key(|(_, &(gx, _, _))| (gx as i64 - preferred_x as i64).unsigned_abs())
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
            let adjusted = if adj_idx > src_idx {
                adj_idx - 1
            } else {
                adj_idx
            };
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
        ctx.navigate_to(new_tile);

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
            } else if let Some(a) = pane.as_app_mut() {
                if let crate::host::pane::AppRuntime::Builtin(app) = &mut a.runtime {
                    app.adjust_font_size(delta);
                }
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
        if restore_overlay_replacement(&mut self.windows[active].panes, pane_id) {
            return;
        }

        let is_app = self.windows[active]
            .panes
            .get(&pane_id)
            .and_then(|p| p.as_app())
            .is_some();
        if is_app {
            let ctx_id = self.windows[active].context_id;
            self.close_tile(active, focused_tile);
            self.windows[active].clear_zoom();
            // An emptied subcontext collapses entirely — delete it and zoom
            // back out to the parent, as if Cmd+Escape had been pressed.
            self.collapse_subcontext_if_empty(ctx_id);
        }
    }

    /// Execute the close-pane action (called directly when confirm_close is false,
    /// or from the confirm-close dialog when the user confirms).
    pub(crate) fn execute_close_pane(&mut self) -> bool {
        let ctx_id = self.windows[self.active_window].context_id;
        self.windows[self.active_window].clear_zoom();
        if !self.windows[self.active_window].panes.is_empty() {
            self.close_focused();
        }
        // If the window is now empty, only delete it when there are other pages
        // in the same context (i.e. it's one of several pages). When it's the
        // sole page, keep it alive so the welcome screen renders.
        if self.windows[self.active_window].panes.is_empty() {
            let pages_in_context = self
                .windows
                .iter()
                .filter(|w| w.context_id == ctx_id)
                .count();
            if pages_in_context > 1 {
                self.delete_window(self.active_window);
            }
        }
        // An emptied subcontext collapses entirely — delete it and zoom back
        // out to the parent, as if Cmd+Escape had been pressed.
        self.collapse_subcontext_if_empty(ctx_id);
        false
    }
}

impl PlexiApp {
    pub(crate) fn resolve_new_pane_cwd(
        &self,
        cwd_override: Option<std::path::PathBuf>,
        focused: Option<TileId>,
    ) -> Option<std::path::PathBuf> {
        cwd_override
            .or_else(|| self.router.active().root.clone())
            .or_else(|| {
                focused.and_then(|f| self.windows[self.active_window].get_focused_pane_cwd(f))
            })
            .or_else(dirs::home_dir)
    }

    /// CWD for the first terminal pane created from the welcome screen (empty window).
    /// Priority: context root → window launch path
    pub(crate) fn cwd_for_welcome_tab(&self) -> std::path::PathBuf {
        self.router
            .active()
            .root
            .clone()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")))
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

    fn window_with_pane(
        context_id: u64,
        window_id: u64,
        pane_id: u64,
        grid_y: u32,
    ) -> crate::host::context::Window {
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

    fn install_tab_tree(app: &mut PlexiApp) -> (TileId, Vec<TileId>) {
        let tiles = &mut app.windows[0].tree.tiles;
        let children = vec![
            tiles.insert_pane(100),
            tiles.insert_pane(101),
            tiles.insert_pane(102),
        ];
        let tabs_tile = tiles.insert_tab_tile(children.clone());
        if let Some(Tile::Container(Container::Tabs(tabs))) =
            app.windows[0].tree.tiles.get_mut(tabs_tile)
        {
            tabs.set_active(children[1]);
        }
        app.windows[0].tree.root = Some(tabs_tile);
        app.windows[0].focused_pane = Some(children[1]);
        app.windows[0].zoomed_pane = Some(children[1]);
        (tabs_tile, children)
    }

    #[test]
    fn reorder_tab_moves_child_without_changing_active_focus_or_zoom() {
        let mut app = test_app();
        let (tabs_tile, children) = install_tab_tree(&mut app);

        assert!(app.reorder_tab(tabs_tile, 0, 2));

        let Some(Tile::Container(Container::Tabs(tabs))) = app.windows[0].tree.tiles.get(tabs_tile)
        else {
            panic!("expected tabs container");
        };
        assert_eq!(
            tabs.children,
            vec![children[1], children[2], children[0]],
            "reorder must move only the selected child tile"
        );
        assert_eq!(
            tabs.active,
            Some(children[1]),
            "active tab tile must remain the same after reorder"
        );
        assert_eq!(app.windows[0].focused_pane, Some(children[1]));
        assert_eq!(app.windows[0].zoomed_pane, Some(children[1]));
    }

    #[test]
    fn reorder_tab_rejects_invalid_indices_and_non_tabs() {
        let mut app = test_app();
        let (tabs_tile, children) = install_tab_tree(&mut app);
        assert!(!app.reorder_tab(tabs_tile, 0, 0));
        assert!(!app.reorder_tab(tabs_tile, 0, 99));
        assert!(!app.reorder_tab(children[0], 0, 1));
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
        assert_eq!(
            app.windows.len(),
            1,
            "zombie window must be deleted after IPC close of last pane"
        );
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
        assert_eq!(
            app.windows.len(),
            1,
            "zombie active window must be deleted after IPC close of last pane"
        );
        assert_eq!(
            app.active_window, 0,
            "active_window must point to the remaining window after deletion"
        );
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
        assert_eq!(
            app.windows.len(),
            1,
            "sole-page window must remain alive showing welcome screen"
        );
    }

    /// Closing the active tab of a background tab container must not steal focus.
    /// Regression guard for #1547 (Gemini-identified edge case).
    #[test]
    fn close_background_active_tab_does_not_steal_focus() {
        let mut app = test_app();
        let focused_id: u64 = 100;
        let bg_active_id: u64 = 101;
        let bg_inactive_id: u64 = 102;

        let tile_focused = app.windows[0].tree.tiles.insert_pane(focused_id);
        let tile_bg_active = app.windows[0].tree.tiles.insert_pane(bg_active_id);
        let tile_bg_inactive = app.windows[0].tree.tiles.insert_pane(bg_inactive_id);

        let tabs_tile = app.windows[0]
            .tree
            .tiles
            .insert_tab_tile(vec![tile_bg_active, tile_bg_inactive]);
        if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))) =
            app.windows[0].tree.tiles.get_mut(tabs_tile)
        {
            tabs.set_active(tile_bg_active);
        }

        let container_tile = app.windows[0]
            .tree
            .tiles
            .insert_horizontal_tile(vec![tile_focused, tabs_tile]);
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
        let tile_bg = app.windows[0].tree.tiles.insert_pane(bg_id);
        let tile_extra = app.windows[0].tree.tiles.insert_pane(extra_id);

        let container_tile = app.windows[0].tree.tiles.insert_horizontal_tile(vec![
            tile_focused,
            tile_bg,
            tile_extra,
        ]);
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
        let tile_bg = app.windows[0].tree.tiles.insert_pane(bg_id);

        let container_tile = app.windows[0]
            .tree
            .tiles
            .insert_horizontal_tile(vec![tile_zoomed, tile_bg]);
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
        assert!(matches!(
            app.swap_pane(Direction::Right),
            SwapResult::NoFocus
        ));
    }

    #[test]
    fn swap_pane_at_boundary_with_single_pane() {
        let mut app = test_app();
        let tile = app.windows[0].tree.tiles.insert_pane(1u64);
        app.windows[0].focused_pane = Some(tile);
        // No neighbors (rects unset = Rect::ZERO, geometric search returns None)
        assert!(matches!(
            app.swap_pane(Direction::Right),
            SwapResult::AtBoundary
        ));
    }
}

#[cfg(test)]
mod send_pane_tests {
    use super::*;

    fn test_app() -> PlexiApp {
        let ctx = egui::Context::default();
        let ft = crate::platform::logging::new_frame_tick();
        PlexiApp::new_for_test(ctx, ft).0
    }

    #[test]
    fn send_pane_no_focus_returns_no_focus() {
        let mut app = test_app();
        assert!(matches!(
            app.send_pane(Direction::Right),
            SwapResult::NoFocus
        ));
    }

    #[test]
    fn send_pane_at_boundary_with_single_pane() {
        let mut app = test_app();
        let tile = app.windows[0].tree.tiles.insert_pane(1u64);
        app.windows[0].focused_pane = Some(tile);
        // No neighbors (rects unset = Rect::ZERO, geometric search returns None)
        assert!(matches!(
            app.send_pane(Direction::Right),
            SwapResult::AtBoundary
        ));
    }
}

#[cfg(test)]
mod squad_tree_tests {
    use super::build_squad_tree;
    use crate::app_protocol::SubContextLayout;

    /// `--layout tiled` (the `context sub` default) builds a Grid container, so
    /// an agent squad renders as a near-square block rather than a strip.
    #[test]
    fn tiled_layout_builds_a_grid_container() {
        let (tree, first) = build_squad_tree(&[1, 2, 3, 4], SubContextLayout::Tiled);
        let root = tree.root.expect("root");
        match tree.tiles.get(root) {
            Some(egui_tiles::Tile::Container(egui_tiles::Container::Grid(grid))) => {
                assert_eq!(grid.children().count(), 4);
            }
            other => panic!("tiled layout must be a Grid, got {other:?}"),
        }
        assert!(matches!(
            tree.tiles.get(first),
            Some(egui_tiles::Tile::Pane(1))
        ));
    }

    /// `--layout columns` builds one horizontal linear container: N full-height
    /// columns, left to right in the order the commands were given.
    #[test]
    fn columns_layout_builds_a_horizontal_linear_container() {
        let (tree, first) = build_squad_tree(&[7, 8, 9], SubContextLayout::Columns);
        let root = tree.root.expect("root");
        match tree.tiles.get(root) {
            Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(linear))) => {
                assert_eq!(linear.dir, egui_tiles::LinearDir::Horizontal);
                assert_eq!(linear.children.len(), 3);
                assert_eq!(
                    linear.children[0], first,
                    "the first pane's tile is the window's initial focus"
                );
            }
            other => panic!("columns layout must be a horizontal Linear, got {other:?}"),
        }
    }

    /// A single pane is the root directly — the same tree shape
    /// `create_single_pane_tree` produces, so the historical single-terminal
    /// child context is structurally unchanged.
    #[test]
    fn single_pane_needs_no_container() {
        let (tree, first) = build_squad_tree(&[42], SubContextLayout::Tiled);
        assert_eq!(tree.root, Some(first));
        assert!(matches!(
            tree.tiles.get(first),
            Some(egui_tiles::Tile::Pane(42))
        ));
    }
}
