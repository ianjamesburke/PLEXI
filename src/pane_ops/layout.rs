//! Layout operations on already-created panes: split, tab, close, navigate,
//! zoom-free tree manipulation, font size, scroll.

use crate::app::PlexiApp;
use crate::context::{replace_child, Context};
use crate::host::command::{HostCommand, Placement};
use crate::host::effect::HostEffect;
use crate::keys::Direction;
use crate::pane::{Pane, TerminalPane};
use crate::tiling::PaneId;
use egui_term::BackendCommand;
use egui_tiles::{Container, SimplificationOptions, Tile, TileId};
use std::collections::HashMap;

/// If `pane_id` is an app pane that was opened as an overlay over another
/// pane (`hint = Some("overlay")`), restore the original pane and return
/// `true`. Otherwise return `false` and leave the map unchanged.
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

impl PlexiApp {
    /// Route a HostCommand through HostModel and return the resulting effects.
    pub(super) fn submit(&mut self, cmd: HostCommand) -> Vec<HostEffect> {
        self.host.handle_command(cmd, &mut self.host_services)
    }

    pub(super) fn split_with_new_pane(
        &mut self,
        new_pane_id: PaneId,
        vertical: bool,
        share: crate::host::command::ShareRatio,
        new_pane_first: bool,
    ) -> Option<egui_tiles::TileId> {
        let focused = self.contexts[self.active_context].focused_pane?;
        let split_target = match self.contexts[self.active_context].find_ancestor_tabs(focused) {
            Some((tabs_id, _)) => tabs_id,
            None => focused,
        };

        let ctx = &mut self.contexts[self.active_context];
        let parent = ctx.tree.tiles.parent_of(split_target);
        let new_tile = ctx.tree.tiles.insert_pane(new_pane_id);

        // LinearDir::Horizontal = side-by-side (left/right); Vertical = stacked (top/bottom).
        // `vertical` here means "split vertically" (new pane to the right), so we need Horizontal.
        let split_dir = if vertical {
            egui_tiles::LinearDir::Horizontal
        } else {
            egui_tiles::LinearDir::Vertical
        };

        let inserted_as_sibling = if let Some(parent_id) = parent {
            if let Some(Tile::Container(Container::Linear(linear))) =
                ctx.tree.tiles.get_mut(parent_id)
            {
                if linear.dir == split_dir {
                    if let Some(pos) = linear.children.iter().position(|&c| c == split_target) {
                        linear.children.insert(pos + 1, new_tile);
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
                ctx.tree.tiles.insert_horizontal_tile(ordered)
            } else {
                ctx.tree.tiles.insert_vertical_tile(ordered)
            };

            if let Some(Tile::Container(Container::Linear(ref mut lin))) =
                ctx.tree.tiles.get_mut(container_tile)
            {
                lin.shares.set_share(split_target, share.denominator);
                lin.shares.set_share(new_tile, share.numerator);
            }

            if let Some(parent_id) = parent {
                if let Some(Tile::Container(parent_container)) = ctx.tree.tiles.get_mut(parent_id) {
                    replace_child(parent_container, split_target, container_tile);
                }
            } else {
                ctx.tree.root = Some(container_tile);
            }
        }

        ctx.focused_pane = Some(new_tile);
        Some(new_tile)
    }

    /// Split the focused pane in the requested direction, creating a new pane
    /// that mirrors the focused pane's type:
    ///
    /// - Terminal → new terminal in the same cwd
    /// - App      → fresh instance of the same app (`launch_app_by_id`)
    /// - Agent    → new agent
    ///
    /// If no pane is focused, falls back to creating a full-size terminal.
    pub(crate) fn split_focused_mirror(&mut self, placement: Placement) {
        let active = self.active_context;
        let Some(focused_tile) = self.contexts[active].focused_pane else {
            // No focused pane → create a full-size terminal in the active context.
            // The empty-context path: if the context has no panes at all, replace
            // the tree. If it has panes but none focused (rare), drop into the
            // standard terminal split path which will no-op for now.
            if self.contexts[active].panes.is_empty() {
                if let Some((tree, panes, root_tile)) = self.create_single_pane_tree(None) {
                    self.contexts[active].tree = tree;
                    self.contexts[active].panes = panes;
                    self.contexts[active].focused_pane = Some(root_tile);
                }
            }
            return;
        };

        let Some(Tile::Pane(focused_pane_id)) =
            self.contexts[active].tree.tiles.get(focused_tile)
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
            Agent,
            /// Mirror-split for an Agent Workspace pane is a no-op in this PR
            /// (#348). The modal picker lands in #349 and is the right surface
            /// for "spawn another agent" — silently dropping the request here
            /// would be confusing, so we fall back to a plain terminal split.
            AgentWorkspace,
        }
        let kind = match self.contexts[active].panes.get(&focused_pane_id) {
            Some(Pane::Terminal(_)) => Kind::Terminal,
            Some(Pane::App(a)) => Kind::App(a.manifest_id.clone()),
            Some(Pane::Agent(_)) => Kind::Agent,
            Some(Pane::AgentWorkspace(_)) => Kind::AgentWorkspace,
            None => return,
        };

        // `vertical` parameter for `split_with_new_pane` / `split_focused`:
        //   true  → side-by-side (new pane on the right) → Placement::Right
        //   false → stacked      (new pane below)         → Placement::Below
        let vertical = matches!(placement, Placement::Right);

        match kind {
            Kind::Terminal => {
                // Reuse the existing terminal split path.
                self.split_focused(vertical);
            }
            Kind::App(manifest_id) => {
                // Fresh instance of the same app at the requested placement.
                // The layout hint maps directly to the split direction:
                //   Placement::Right → "split_v" (side-by-side, new pane right)
                //   Placement::Below → "split_h" (stacked,      new pane below)
                let layout = if vertical { "split_v" } else { "split_h" };
                self.launch_app_by_id_with_layout(
                    &manifest_id,
                    Some(layout.to_string()),
                    &[],
                );
            }
            Kind::Agent => {
                let cwd = self.contexts[active]
                    .get_focused_pane_cwd(focused_tile)
                    .unwrap_or_else(|| {
                        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"))
                    });
                let new_id = self.host.alloc_pane_id();
                let pane = crate::agent_pane::AgentPane::new(new_id, cwd);
                self.contexts[active]
                    .panes
                    .insert(new_id, Pane::Agent(Box::new(pane)));
                let share = crate::host::command::ShareRatio::new(1.0, 1.0)
                    .expect("1:1 is valid");
                let _ = self.split_with_new_pane(new_id, vertical, share, false);
            }
            Kind::AgentWorkspace => {
                // Substrate-only: mirror-split of an Agent Workspace falls
                // through to a plain terminal split. The modal picker (#349)
                // owns "spawn another agent in a new worktree".
                self.split_focused(vertical);
            }
        }
    }

    pub(crate) fn split_focused(&mut self, vertical: bool) {
        let Some(focused) = self.contexts[self.active_context].focused_pane else {
            return;
        };

        let cmd = if vertical {
            HostCommand::SplitVertical
        } else {
            HostCommand::SplitHorizontal
        };
        let effects = self.submit(cmd);
        log::debug!("split_focused(vertical={vertical}) effects: {:?}", effects);
        let (new_id, vertical) = effects
            .iter()
            .find_map(|e| match e {
                HostEffect::SplitOpened {
                    pane_id, placement, ..
                } => Some((*pane_id, !matches!(placement, Placement::Below))),
                _ => None,
            })
            .unwrap_or_else(|| (self.host.alloc_pane_id(), vertical));

        let cwd = self.contexts[self.active_context].get_focused_pane_cwd(focused);
        let settings = Self::make_backend_settings(cwd, &self.colors);
        let Some(pane) = TerminalPane::new(
            new_id,
            self.ctx.clone(),
            self.pty_event_tx.clone(),
            settings,
            self.default_font_size,
        ) else {
            log::error!("Failed to create new terminal pane");
            return;
        };
        self.contexts[self.active_context]
            .panes
            .insert(new_id, Pane::Terminal(Box::new(pane)));

        let split_target = match self.contexts[self.active_context].find_ancestor_tabs(focused) {
            Some((tabs_id, _)) => tabs_id,
            None => focused,
        };

        let ctx = &mut self.contexts[self.active_context];
        let parent = ctx.tree.tiles.parent_of(split_target);
        let new_tile = ctx.tree.tiles.insert_pane(new_id);

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
                        linear.children.insert(pos + 1, new_tile);
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
            let container_tile = if vertical {
                ctx.tree
                    .tiles
                    .insert_vertical_tile(vec![split_target, new_tile])
            } else {
                ctx.tree
                    .tiles
                    .insert_horizontal_tile(vec![split_target, new_tile])
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

    pub(crate) fn new_tab(&mut self) {
        let Some(focused) = self.contexts[self.active_context].focused_pane else {
            return;
        };

        let new_id = self.host.alloc_pane_id();

        let cwd = self.contexts[self.active_context].get_focused_pane_cwd(focused);
        let settings = Self::make_backend_settings(cwd, &self.colors);
        let Some(pane) = TerminalPane::new(
            new_id,
            self.ctx.clone(),
            self.pty_event_tx.clone(),
            settings,
            self.default_font_size,
        ) else {
            log::error!("Failed to create new terminal pane");
            return;
        };
        self.contexts[self.active_context]
            .panes
            .insert(new_id, Pane::Terminal(Box::new(pane)));

        let ctx = &mut self.contexts[self.active_context];
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
        let ctx = &self.contexts[self.active_context];
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

        let ctx = &mut self.contexts[self.active_context];
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
        let focused = match self.contexts[self.active_context].focused_pane {
            Some(f) => f,
            None => return,
        };
        let focused_pane_id = self.contexts[self.active_context]
            .tree
            .tiles
            .get(focused)
            .and_then(|tile| match tile {
                Tile::Pane(pane_id) => Some(*pane_id),
                _ => None,
            });
        if let Some(pane_id) = focused_pane_id {
            if restore_overlay_replacement(&mut self.contexts[self.active_context].panes, pane_id) {
                return;
            }
        }

        let effects = self.submit(HostCommand::CloseFocusedPane);
        log::debug!("close_focused effects: {:?}", effects);
        self.close_tile(self.active_context, focused);
    }

    /// Close a specific pane by its PaneId (the u64 backend ID, not the TileId).
    /// Searches all contexts to find the tile containing this pane.
    pub(crate) fn close_pane_by_id(&mut self, pane_id: PaneId) {
        // Find which context and tile owns this pane_id.
        for ctx_idx in 0..self.contexts.len() {
            if let Some(tile_id) = self.contexts[ctx_idx].tree.tiles.find_pane(&pane_id) {
                self.close_tile(ctx_idx, tile_id);
                return;
            }
        }
    }

    /// Close a tile in a specific context by its TileId. Handles sibling focus
    /// transfer, container cleanup, and pane removal.
    pub(super) fn close_tile(&mut self, ctx_idx: usize, tile_id: TileId) {
        // Phase 1: Read-only — determine sibling and container type
        let parent_info = self.contexts[ctx_idx].find_logical_parent(tile_id);

        let next = if let Some((parent_id, child_in_parent)) = parent_info {
            let sibling_info = {
                let ctx: &Context = &self.contexts[ctx_idx];
                if let Some(Tile::Container(container)) = ctx.tree.tiles.get(parent_id) {
                    let children: Vec<TileId> = container.children().copied().collect();
                    children
                        .iter()
                        .position(|&c| c == child_in_parent)
                        .map(|pos| {
                            let sibling = if pos > 0 {
                                children[pos - 1]
                            } else {
                                children[pos + 1]
                            };
                            let is_tabs = matches!(container, Container::Tabs(_));
                            let is_linear = matches!(container, Container::Linear(_));
                            (sibling, is_tabs, is_linear, children)
                        })
                } else {
                    None
                }
            };

            if let Some((sibling, is_tabs, is_linear, all_children)) = sibling_info {
                // Phase 2: Mutable — update container state
                let ctx = &mut self.contexts[ctx_idx];
                if is_tabs {
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

                self.contexts[ctx_idx].find_first_pane_in(sibling)
            } else {
                self.contexts[ctx_idx].find_next_focus(tile_id)
            }
        } else {
            self.contexts[ctx_idx].find_next_focus(tile_id)
        };

        // Phase 3: Remove tile and extract pane — defer drop so background apps can be parked.
        let removed_pane = {
            let ctx = &mut self.contexts[ctx_idx];
            if let Some(parent_id) = ctx.tree.tiles.parent_of(tile_id) {
                if let Some(Tile::Container(parent)) = ctx.tree.tiles.get_mut(parent_id) {
                    parent.remove_child(tile_id);
                }
            }

            let removed = if let Some(Tile::Pane(pane_id)) = ctx.tree.tiles.remove(tile_id) {
                ctx.panes.remove(&pane_id)
            } else {
                None
            };

            ctx.tree.simplify(&SimplificationOptions {
                all_panes_must_have_tabs: true,
                ..SimplificationOptions::default()
            });
            ctx.focused_pane = next;

            removed
        };

        // ctx borrow is released — park background ProcessApps; drop everything else.
        match removed_pane {
            Some(Pane::App(app_pane)) => {
                if let crate::pane::AppRuntime::Process(mut process_app) = app_pane.runtime {
                    let type_id = process_app.type_id.clone();
                    if self.registry.is_background(&type_id) {
                        process_app.send_event(&crate::app_protocol::PlexiEvent::Suspend);
                        log::info!("parking background app '{type_id}'");
                        self.background_apps.insert(type_id, process_app);
                    }
                    // else: process_app drops here — Drop impl sends Shutdown + kills process
                }
                // else: builtin app pane drops here
            }
            Some(Pane::AgentWorkspace(workspace)) => {
                // Tear down the worktree so the directory disappears. The
                // branch survives — review/merge happens after pane close.
                let repo = workspace.repo_path.clone();
                let wt = workspace.worktree_path.clone();
                let branch = workspace.branch_name.clone();
                drop(workspace); // release PTY (TerminalBackend Drop kills child)
                match crate::agent_workspace::remove_worktree(&repo, &wt) {
                    Ok(()) => log::info!(
                        "agent_workspace: removed worktree {} (branch '{branch}' kept for review)",
                        wt.display()
                    ),
                    Err(e) => log::warn!(
                        "agent_workspace: failed to remove worktree {}: {e}",
                        wt.display()
                    ),
                }
            }
            _ => {}
        }
    }

    pub(crate) fn navigate(&mut self, dir: Direction) {
        let effects = self.submit(HostCommand::Navigate(dir));
        log::debug!("navigate({:?}) effects: {:?}", dir, effects);

        let ctx = &self.contexts[self.active_context];
        if let Some(focused) = ctx.focused_pane {
            if let Some(target) = ctx.find_pane_in_direction_from(focused, dir) {
                self.contexts[self.active_context].focused_pane = Some(target);
            }
        }
    }

    pub(crate) fn scroll_focused_pane(&mut self, lines: i32) {
        let ctx = &mut self.contexts[self.active_context];
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
        let ctx = &mut self.contexts[self.active_context];
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
            } else if let Some(a) = pane.as_agent_mut() {
                a.font_size = (a.font_size + delta).clamp(8.0, 32.0);
            }
        }
    }

    /// Close the focused app pane.
    pub(crate) fn close_focused_app(&mut self) {
        let active = self.active_context;
        let Some(focused_tile) = self.contexts[active].focused_pane else {
            return;
        };
        let Some(pane_id) = self.contexts[active]
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
        if restore_overlay_replacement(&mut self.contexts[active].panes, pane_id) {
            return;
        }

        let is_app = self.contexts[active]
            .panes
            .get(&pane_id)
            .and_then(|p| p.as_app())
            .is_some();
        if is_app {
            self.close_tile(active, focused_tile);
        }
    }

    /// Execute the close-pane action (called directly when confirm_close is false,
    /// or from the confirm-close dialog when the user confirms).
    pub(crate) fn execute_close_pane(&mut self) {
        self.contexts[self.active_context].zoomed_pane = None;
        let active_panes = self.contexts[self.active_context].panes.len();
        if active_panes > 1 {
            self.close_focused();
        } else {
            self.reset_active_context();
        }
    }
}
