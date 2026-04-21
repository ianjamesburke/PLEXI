use crate::context::{replace_child, Context};
use crate::host::command::{HostCommand, OpenPaneRequest, PaneRuntimeKind, Placement, ShareRatio};
use crate::host::effect::HostEffect;
use crate::keys::Direction;
use crate::pane::{Pane, TerminalPane};
use crate::shell;
use crate::tiling::PaneId;
use crate::workspace::WorkspaceFile;
use egui_term::BackendCommand;
use egui_tiles::{Container, SimplificationOptions, Tile, TileId, Tree};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::app::PlexiApp;
use crate::app_trait::App;

fn restore_overlay_replacement(panes: &mut HashMap<PaneId, Pane>, pane_id: PaneId) -> bool {
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
    fn submit(&mut self, cmd: HostCommand) -> Vec<HostEffect> {
        self.host.handle_command(cmd, &mut self.host_services)
    }

    /// Convert a manifest-declared share fraction (0.0..1.0 exclusive) to a `ShareRatio`.
    /// Validates the range and falls back to 0.5 (1:1) on invalid input, logging a warning.
    fn share_ratio_from_fraction(app_id: &str, fraction: Option<f32>) -> ShareRatio {
        let f = fraction.unwrap_or(0.5);
        if f <= 0.0 || f >= 1.0 {
            log::warn!(
                "open_pane_layout({app_id}): initial_share {f} out of range (0.0, 1.0); defaulting to 0.5"
            );
            ShareRatio::new(1.0, 1.0).expect("1:1 is valid")
        } else {
            ShareRatio::new(f, 1.0 - f).expect("validated fraction in (0, 1)")
        }
    }

    /// Return the focused terminal's PaneId, if the currently focused pane is a terminal.
    /// Used to record which terminal an app was spawned alongside, so CdRequest can
    /// route directly to it without a tile-tree walk.
    fn focused_terminal_id(&self, active: usize) -> Option<PaneId> {
        let ctx = &self.contexts[active];
        let tile_id = ctx.focused_pane?;
        let pane_id = match ctx.tree.tiles.get(tile_id) {
            Some(egui_tiles::Tile::Pane(id)) => *id,
            _ => return None,
        };
        if ctx.panes.get(&pane_id)?.as_terminal().is_some() {
            Some(pane_id)
        } else {
            None
        }
    }

    /// Submit an `OpenPane` request to HostModel and extract
    /// `(pane_id, share, vertical, new_pane_first)` from the resulting `PaneOpened` effect.
    /// Falls back to `alloc_pane_id()` + 1:1 vertical split if no effect is
    /// returned (should not happen in practice).
    fn open_pane_layout(
        &mut self,
        app_id: &str,
        group: Option<String>,
        hint: Option<&str>,
        share: ShareRatio,
    ) -> (PaneId, ShareRatio, bool, bool) {
        let new_pane_first = matches!(hint, Some("split_above"));
        let vertical = !matches!(hint, Some("split_h") | Some("split_above"));
        let placement = if vertical {
            Placement::Right
        } else {
            Placement::Below
        };
        let req = OpenPaneRequest {
            runtime: PaneRuntimeKind::App {
                app_id: app_id.to_string(),
            },
            placement,
            share,
            group,
            declared_capabilities: vec![],
        };
        let effects = self.submit(HostCommand::OpenPane(req));
        log::debug!("open_pane_layout({app_id}) effects: {:?}", effects);
        effects
            .iter()
            .find_map(|e| {
                if let HostEffect::PaneOpened { pane_id, share, placement, .. } = e {
                    let vert = !matches!(placement, Placement::Below);
                    Some((*pane_id, *share, vert, new_pane_first))
                } else {
                    None
                }
            })
            .unwrap_or_else(|| {
                let fallback = self.host.alloc_pane_id();
                log::warn!(
                    "open_pane_layout({app_id}): no PaneOpened effect — allocating fallback id={fallback}"
                );
                (
                    fallback,
                    ShareRatio::new(1.0, 1.0).expect("1:1 is valid"),
                    vertical,
                    new_pane_first,
                )
            })
    }

    fn split_with_new_pane(
        &mut self,
        new_pane_id: PaneId,
        vertical: bool,
        share: ShareRatio,
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

    fn open_process_app_pane(
        &mut self,
        app_id: &str,
        mut process: crate::process_app::ProcessApp,
        workspace_root: PathBuf,
        group: Option<String>,
        hint: Option<&str>,
    ) {
        let active = self.active_context;
        let new_app_pane = |id: PaneId,
                            process: crate::process_app::ProcessApp,
                            workspace_root: PathBuf,
                            group: Option<String>,
                            linked_pane_id: Option<PaneId>,
                            overlay_replaced: Option<Box<Pane>>| {
            Pane::App(Box::new(crate::pane::AppPane {
                id,
                permissions: process.permissions.clone(),
                runtime: crate::pane::AppRuntime::Process(Box::new(process)),
                workspace_root,
                manifest_id: app_id.to_string(),
                name: app_id.to_string(),
                pane_group: group,
                linked_pane_id,
                overlay_replaced,
            }))
        };

        if matches!(hint, Some("overlay")) {
            let Some(focused_tile) = self.contexts[active].focused_pane else {
                return;
            };
            let Some(Tile::Pane(focused_pane_id)) =
                self.contexts[active].tree.tiles.get(focused_tile)
            else {
                return;
            };
            let pane_id = *focused_pane_id;
            let Some(replaced_pane) = self.contexts[active].panes.remove(&pane_id) else {
                return;
            };
            process.set_pane_id(pane_id);
            self.contexts[active].panes.insert(
                pane_id,
                new_app_pane(pane_id, process, workspace_root, group, None, Some(Box::new(replaced_pane))),
            );
            self.contexts[active].focused_pane = Some(focused_tile);
            return;
        }

        // Record which terminal we're splitting from before focus moves.
        let linked_pane_id = self.focused_terminal_id(active);
        let share = Self::share_ratio_from_fraction(app_id, self.registry.share_for(app_id));
        let (new_id, share, vertical, new_pane_first) =
            self.open_pane_layout(app_id, group.clone(), hint, share);
        process.set_pane_id(new_id);
        self.contexts[active].panes.insert(
            new_id,
            new_app_pane(new_id, process, workspace_root, group, linked_pane_id, None),
        );

        let _ = self.split_with_new_pane(new_id, vertical, share, new_pane_first);
    }

    fn open_builtin_app_pane(
        &mut self,
        app: Box<dyn App>,
        permissions: crate::app_permissions::AppPermissions,
        workspace_root: PathBuf,
        group: Option<String>,
        hint: Option<&str>,
        share: Option<f32>,
    ) {
        let active = self.active_context;
        let app_type_id = app.type_id().to_string();
        let app_name = app.display_name();
        let new_app_pane = |id: PaneId,
                            app: Box<dyn App>,
                            workspace_root: PathBuf,
                            group: Option<String>,
                            linked_pane_id: Option<PaneId>,
                            overlay_replaced: Option<Box<Pane>>| {
            Pane::App(Box::new(crate::pane::AppPane {
                id,
                runtime: crate::pane::AppRuntime::Builtin(app),
                workspace_root,
                permissions,
                manifest_id: app_type_id.clone(),
                name: app_name.clone(),
                pane_group: group,
                linked_pane_id,
                overlay_replaced,
            }))
        };

        if matches!(hint, Some("overlay")) {
            let Some(focused_tile) = self.contexts[active].focused_pane else {
                return;
            };
            let Some(Tile::Pane(focused_pane_id)) =
                self.contexts[active].tree.tiles.get(focused_tile)
            else {
                return;
            };
            let pane_id = *focused_pane_id;
            let Some(replaced_pane) = self.contexts[active].panes.remove(&pane_id) else {
                return;
            };
            self.contexts[active].panes.insert(
                pane_id,
                new_app_pane(pane_id, app, workspace_root, group, None, Some(Box::new(replaced_pane))),
            );
            self.contexts[active].focused_pane = Some(focused_tile);
            return;
        }

        // Record which terminal we're splitting from before focus moves.
        let linked_pane_id = self.focused_terminal_id(active);
        let share = Self::share_ratio_from_fraction(
            &app_type_id,
            share.or_else(|| self.registry.share_for(&app_type_id)),
        );
        let (new_id, share, vertical, new_pane_first) =
            self.open_pane_layout(&app_type_id, group.clone(), hint, share);
        self.contexts[active].panes.insert(
            new_id,
            new_app_pane(new_id, app, workspace_root, group, linked_pane_id, None),
        );

        let _ = self.split_with_new_pane(new_id, vertical, share, new_pane_first);
    }

    fn create_single_pane_tree(
        &mut self,
        cwd: Option<PathBuf>,
    ) -> Option<(Tree<PaneId>, HashMap<PaneId, Pane>, TileId)> {
        let new_id = self.host.alloc_pane_id();

        let settings = Self::make_backend_settings(cwd, &self.colors);
        let pane = TerminalPane::new(
            new_id,
            self.ctx.clone(),
            self.pty_event_tx.clone(),
            settings,
            self.default_font_size,
        )?;

        let mut panes = HashMap::new();
        panes.insert(new_id, Pane::Terminal(Box::new(pane)));

        let mut tiles = egui_tiles::Tiles::default();
        let root_tile = tiles.insert_pane(new_id);
        let tree = Tree::new("plexi", root_tile, tiles);

        Some((tree, panes, root_tile))
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
    fn close_tile(&mut self, ctx_idx: usize, tile_id: TileId) {
        // Phase 1: Read-only — determine sibling and container type
        let parent_info = self.contexts[ctx_idx].find_logical_parent(tile_id);

        let next = if let Some((parent_id, child_in_parent)) = parent_info {
            let sibling_info = {
                let ctx = &self.contexts[ctx_idx];
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

        // Phase 3: Remove tile and pane
        let ctx = &mut self.contexts[ctx_idx];
        if let Some(parent_id) = ctx.tree.tiles.parent_of(tile_id) {
            if let Some(Tile::Container(parent)) = ctx.tree.tiles.get_mut(parent_id) {
                parent.remove_child(tile_id);
            }
        }

        if let Some(Tile::Pane(pane_id)) = ctx.tree.tiles.remove(tile_id) {
            ctx.panes.remove(&pane_id);
        }

        ctx.tree.simplify(&SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..SimplificationOptions::default()
        });
        ctx.focused_pane = next;
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

    pub(crate) fn new_context(&mut self) {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let Some((tree, panes, root_tile)) = self.create_single_pane_tree(Some(home.clone()))
        else {
            log::error!("Failed to create terminal for new context");
            return;
        };

        let name = format!("Context {}", self.contexts.len() + 1);
        self.contexts.push(Context {
            name,
            path: home,
            tree,
            panes,
            focused_pane: Some(root_tile),
            zoomed_pane: None,
        });
        self.active_context = self.contexts.len() - 1;
    }

    pub(crate) fn reset_active_context(&mut self) {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let Some((tree, panes, root_tile)) = self.create_single_pane_tree(Some(home.clone()))
        else {
            log::error!("Failed to create terminal for reset context");
            return;
        };

        let ctx = &mut self.contexts[self.active_context];
        ctx.tree = tree;
        ctx.panes = panes;
        ctx.focused_pane = Some(root_tile);
        ctx.zoomed_pane = None;
    }

    pub(crate) fn delete_context(&mut self, index: usize) {
        if self.contexts.len() <= 1 {
            return;
        }
        self.contexts.remove(index);
        if self.active_context >= self.contexts.len() {
            self.active_context = self.contexts.len() - 1;
        }
        // Clear rename state if it referenced the deleted context
        if self.renaming_context == Some(index) {
            self.renaming_context = None;
        } else if let Some(r) = self.renaming_context {
            if r > index {
                self.renaming_context = Some(r - 1);
            }
        }
    }

    pub(crate) fn save_workspace(&self) {
        let mut saved_contexts = Vec::new();
        for context in &self.contexts {
            let mut saved_panes = Vec::new();
            for (&id, pane) in &context.panes {
                debug_assert_eq!(pane.id(), id);
                if let Some(t) = pane.as_terminal() {
                    let cwd = shell::get_pid_cwd(t.backend.child_pid())
                        .unwrap_or_else(|| context.path.clone());
                    saved_panes.push(crate::workspace::SavedPane {
                        id,
                        kind: crate::workspace::SavedPaneKind::Terminal,
                        cwd,
                        name: t.name.clone(),
                        app_id: None,
                        app_state: None,
                    });
                } else if let Some(a) = pane.as_app() {
                    saved_panes.push(crate::workspace::SavedPane {
                        id,
                        kind: crate::workspace::SavedPaneKind::App,
                        cwd: a.workspace_root.clone(),
                        name: Some(a.name.clone()),
                        app_id: Some(a.runtime.type_id().to_string()),
                        app_state: a.runtime.serialize_state(),
                    });
                } else if let Some(ag) = pane.as_agent() {
                    saved_panes.push(crate::workspace::SavedPane {
                        id,
                        kind: crate::workspace::SavedPaneKind::Agent,
                        cwd: ag.cwd.clone(),
                        name: None,
                        app_id: None,
                        app_state: None,
                    });
                }
            }
            saved_contexts.push(crate::workspace::SavedContext {
                name: context.name.clone(),
                path: context.path.clone(),
                tree: context.tree.clone(),
                panes: saved_panes,
                focused_pane: context.focused_pane,
            });
        }

        let ws = WorkspaceFile {
            version: 1,
            active_context: self.active_context,
            sidebar_visible: self.sidebar_visible,
            next_pane_id: self.host.next_pane_id(),
            contexts: saved_contexts,
        };

        if let Err(e) = ws.save() {
            log::error!("Failed to save workspace: {e}");
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

    /// Toggle the file browser: if the focused pane has a file browser open,
    /// close it. Otherwise, open one.
    pub(crate) fn open_file_browser(&mut self) {
        // Check if the focused pane (or its linked app pane above) already has
        // a file browser open. If so, close it.
        let ctx = &self.contexts[self.active_context];
        if let Some(focused) = ctx.focused_pane {
            if let Some(egui_tiles::Tile::Pane(pane_id)) = ctx.tree.tiles.get(focused) {
                let pane_id = *pane_id;
                if let Some(pane) = ctx.panes.get(&pane_id) {
                    if let Some(a) = pane.as_app() {
                        if a.runtime.type_id() == "file_browser" {
                            self.close_focused_app();
                            return;
                        }
                    }
                }
            }
        }

        let cwd = {
            let ctx = &self.contexts[self.active_context];
            ctx.focused_pane
                .and_then(|tile_id| ctx.get_focused_pane_cwd(tile_id))
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
        };

        let app: Box<dyn App> = self
            .registry
            .launch("file_browser", &cwd, &[])
            .unwrap_or_else(|| Box::new(crate::file_browser::FileBrowserApp::new(cwd.clone())));

        // Built-in file browser gets full permissions, joins the "cwd" group so
        // it follows linked-terminal directory changes.
        let perms = crate::app_permissions::AppPermissions::builtin();
        self.open_builtin_app_pane(
            app,
            perms,
            cwd,
            Some("cwd".to_string()),
            Some("split_above"),
            Some(0.75),
        );
    }

    /// Open the quick note app (full pane, no terminal split).
    pub(crate) fn open_quick_note(&mut self) {
        let cwd = {
            let ctx = &self.contexts[self.active_context];
            ctx.focused_pane
                .and_then(|tile_id| ctx.get_focused_pane_cwd(tile_id))
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
        };

        let app = Box::new(crate::quick_note_app::QuickNoteApp::new(cwd.clone()));
        let perms = crate::app_permissions::AppPermissions::builtin();
        self.open_builtin_app_pane(app, perms, cwd, None, Some("overlay"), None);
    }

    /// Open the Plexi config file in the text editor app.
    pub(crate) fn open_config_editor(&mut self) {
        let config_path = crate::config::config_path();
        // Ensure config file exists with defaults.
        if !config_path.exists() {
            if let Some(parent) = config_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(
                &config_path,
                "# Plexi configuration\n# See docs for options\n",
            );
        }
        let scope = config_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
        let editor = crate::text_editor_app::TextEditorApp::from_file(config_path);
        let perms = crate::app_permissions::AppPermissions::builtin();
        self.open_builtin_app_pane(Box::new(editor), perms, scope, None, Some("overlay"), None);
    }

    /// Launch an installed app by id in the focused pane.
    /// Respects the `layout_hint` from the app's manifest.toml.
    pub(crate) fn launch_app_by_id(&mut self, id: &str) {
        self.launch_app_by_id_with_layout(id, None, &[]);
    }

    /// Launch an installed app with an explicit layout and args override.
    /// `layout` overrides the manifest's `layout_hint` when `Some`.
    ///   "split_v" (default) — vertical split, new pane below
    ///   "split_h"           — horizontal split, new pane to the right
    ///   "overlay"           — full pane, no terminal split
    pub(crate) fn launch_app_by_id_with_layout(
        &mut self,
        id: &str,
        layout: Option<String>,
        args: &[String],
    ) {
        let cwd = self.contexts[self.active_context]
            .focused_pane
            .and_then(|fp| self.contexts[self.active_context].get_focused_pane_cwd(fp))
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));

        let group = self.registry.group_for(id);
        let hint = layout.or_else(|| self.registry.layout_hint_for(id));

        if let Some(process) = self.registry.launch_process(id, &cwd, args) {
            self.open_process_app_pane(id, process, cwd, group, hint.as_deref());
        } else {
            log::warn!("launch_app_by_id: app '{id}' not found or failed to launch");
        }
    }

    /// Open a new agent (Plexi IQ) pane alongside the focused terminal (Cmd+I).
    pub(crate) fn open_agent_pane(&mut self) {
        let active = self.active_context;
        let cwd = {
            let ctx = &self.contexts[active];
            ctx.focused_pane
                .and_then(|tile_id| ctx.get_focused_pane_cwd(tile_id))
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
        };
        let new_id = self.host.alloc_pane_id();
        let pane = crate::agent_pane::AgentPane::new(new_id, cwd);
        self.contexts[active]
            .panes
            .insert(new_id, Pane::Agent(Box::new(pane)));
        let share = crate::host::command::ShareRatio::new(1.0, 1.0).expect("1:1 is valid");
        let _ = self.split_with_new_pane(new_id, true, share, false);
    }

    /// Open the secrets manager (read-only vault viewer, full pane, no terminal split).
    pub(crate) fn open_secrets_manager(&mut self) {
        // Toggle: if already open, close it.
        let ctx = &self.contexts[self.active_context];
        if let Some(focused) = ctx.focused_pane {
            if let Some(egui_tiles::Tile::Pane(pane_id)) = ctx.tree.tiles.get(focused) {
                if let Some(pane) = ctx.panes.get(pane_id) {
                    if let Some(a) = pane.as_app() {
                        if a.runtime.type_id() == "secrets_manager" {
                            self.close_focused_app();
                            return;
                        }
                    }
                }
            }
        }

        let cwd = {
            let ctx = &self.contexts[self.active_context];
            ctx.focused_pane
                .and_then(|tile_id| ctx.get_focused_pane_cwd(tile_id))
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
        };

        let app = Box::new(crate::secrets_app::SecretsApp::new(cwd.clone()));
        let perms = crate::app_permissions::AppPermissions::builtin();
        self.open_builtin_app_pane(app, perms, cwd, None, Some("overlay"), None);
    }

}
