use crate::context::{replace_child, Context};
use crate::keys::Direction;
use crate::pane::TerminalPane;
use crate::shell;
use crate::tiling::PaneId;
use crate::workspace::WorkspaceFile;
use egui_term::BackendCommand;
use egui_tiles::{Container, SimplificationOptions, Tile, TileId, Tree};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::app::PlexiApp;
use crate::app_trait::App;

impl PlexiApp {
    fn create_single_pane_tree(
        &mut self,
        cwd: Option<PathBuf>,
    ) -> Option<(Tree<PaneId>, HashMap<PaneId, TerminalPane>, TileId)> {
        let new_id = self.next_pane_id;
        self.next_pane_id += 1;

        let settings = Self::make_backend_settings(cwd, &self.colors);
        let pane = TerminalPane::new(
            new_id,
            self.ctx.clone(),
            self.pty_event_tx.clone(),
            settings,
            self.default_font_size,
        )?;

        let mut panes = HashMap::new();
        panes.insert(new_id, pane);

        let mut tiles = egui_tiles::Tiles::default();
        let root_tile = tiles.insert_pane(new_id);
        let tree = Tree::new("plexi", root_tile, tiles);

        Some((tree, panes, root_tile))
    }

    /// Create a two-pane horizontal split tree rooted at `cwd`.
    /// Returns (tree, panes, left_tile_id).
    fn create_split_pane_tree(
        &mut self,
        cwd: Option<PathBuf>,
    ) -> Option<(Tree<PaneId>, HashMap<PaneId, TerminalPane>, TileId)> {
        let id_left = self.next_pane_id;
        self.next_pane_id += 1;
        let id_right = self.next_pane_id;
        self.next_pane_id += 1;

        let settings_left = Self::make_backend_settings(cwd.clone(), &self.colors);
        let pane_left = TerminalPane::new(
            id_left,
            self.ctx.clone(),
            self.pty_event_tx.clone(),
            settings_left,
            self.default_font_size,
        )?;

        let settings_right = Self::make_backend_settings(cwd, &self.colors);
        let pane_right = TerminalPane::new(
            id_right,
            self.ctx.clone(),
            self.pty_event_tx.clone(),
            settings_right,
            self.default_font_size,
        )?;

        let mut panes = HashMap::new();
        panes.insert(id_left, pane_left);
        panes.insert(id_right, pane_right);

        let mut tiles = egui_tiles::Tiles::default();
        let left_tile = tiles.insert_pane(id_left);
        let right_tile = tiles.insert_pane(id_right);
        let root = tiles.insert_horizontal_tile(vec![left_tile, right_tile]);
        let tree = Tree::new("plexi", root, tiles);

        Some((tree, panes, left_tile))
    }

    /// Create a tree with N panes in a horizontal split, all rooted at `cwd`.
    /// Returns (tree, panes, first_tile_id). Falls back to 2 panes if n < 1.
    fn create_n_pane_tree(
        &mut self,
        n: usize,
        cwd: Option<PathBuf>,
    ) -> Option<(Tree<PaneId>, HashMap<PaneId, TerminalPane>, Vec<egui_tiles::TileId>)> {
        let count = n.max(1);
        let mut panes = HashMap::new();
        let mut tiles = egui_tiles::Tiles::default();
        let mut tile_ids = Vec::new();

        for _ in 0..count {
            let id = self.next_pane_id;
            self.next_pane_id += 1;
            let settings = Self::make_backend_settings(cwd.clone(), &self.colors);
            let pane = TerminalPane::new(
                id,
                self.ctx.clone(),
                self.pty_event_tx.clone(),
                settings,
                self.default_font_size,
            )?;
            panes.insert(id, pane);
            let tile = tiles.insert_pane(id);
            tile_ids.push(tile);
        }

        let root = if tile_ids.len() == 1 {
            tile_ids[0]
        } else {
            tiles.insert_horizontal_tile(tile_ids.clone())
        };
        let tree = Tree::new("plexi", root, tiles);
        Some((tree, panes, tile_ids))
    }

    pub(crate) fn split_focused(&mut self, vertical: bool) {
        let Some(focused) = self.contexts[self.active_context].focused_pane else {
            return;
        };

        let new_id = self.next_pane_id;
        self.next_pane_id += 1;

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
        log::info!(
            "pane {new_id}: created via {} split",
            if vertical { "vertical" } else { "horizontal" }
        );
        self.contexts[self.active_context]
            .panes
            .insert(new_id, pane);

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

        let new_id = self.next_pane_id;
        self.next_pane_id += 1;

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
        log::info!("pane {new_id}: created as new tab");
        self.contexts[self.active_context]
            .panes
            .insert(new_id, pane);

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
        // Refuse to close locked panes.
        if let Some(Tile::Pane(pid)) = self.contexts[self.active_context].tree.tiles.get(focused) {
            if self.contexts[self.active_context]
                .panes
                .get(pid)
                .is_some_and(|p| p.locked)
            {
                log::info!("close_focused: pane is locked, refusing to close");
                return;
            }
        }
        self.close_tile(self.active_context, focused);
    }

    /// Close a specific pane by its PaneId (the u64 backend ID, not the TileId).
    /// Searches all contexts to find the tile containing this pane.
    pub(crate) fn close_pane_by_id(&mut self, pane_id: PaneId) {
        // Handle cascade/orphan before removing the tile so children are still
        // findable in the tree.
        self.handle_spawn_pane_close(pane_id);


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
            log::info!("pane {pane_id}: closed");
            ctx.panes.remove(&pane_id);
        }

        ctx.tree.simplify(&SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..SimplificationOptions::default()
        });
        ctx.focused_pane = next;
    }

    pub(crate) fn navigate(&mut self, dir: Direction) {
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
        log::info!("context '{}': created", name);
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

    /// Open a folder as a new context — used by the macOS Finder service
    /// ("Open in Plexi") and by `plexi <path>` on the CLI. The context is
    /// named after the folder's last path component.
    pub(crate) fn open_path_as_context(&mut self, path: PathBuf) {
        if !path.is_dir() {
            log::warn!(
                "open_path_as_context: path is not a directory, ignoring: {}",
                path.display()
            );
            return;
        }

        let Some((tree, panes, root_tile)) = self.create_single_pane_tree(Some(path.clone()))
        else {
            log::error!(
                "open_path_as_context: failed to create terminal for {}",
                path.display()
            );
            return;
        };

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());

        self.contexts.push(Context {
            name,
            path,
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
                let cwd = shell::get_pid_cwd(pane.backend.child_pid())
                    .unwrap_or_else(|| context.path.clone());
                let (app_type, app_state) = if let Some(app) = &pane.active_app {
                    (Some(app.type_id().to_string()), app.serialize_state())
                } else {
                    (None, None)
                };
                saved_panes.push(crate::workspace::SavedPane {
                    id,
                    cwd,
                    name: pane.name.clone(),
                    active_app_type: app_type,
                    active_app_state: app_state,
                    linked_terminal_pane: pane.linked_terminal_pane,
                    locked: pane.locked,
                });
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
            next_pane_id: self.next_pane_id,
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
            pane.backend.process_command(BackendCommand::Scroll(lines));
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
            pane.font_size = (pane.font_size + delta).clamp(8.0, 32.0);
        }
    }

    /// Close the active app on the focused terminal pane, returning to full terminal.
    ///
    /// Behavior depends on the mode the app was opened in:
    ///
    /// * `AppWithCompanion` — the existing terminal simply expands back to
    ///   full pane. No tile destruction. The terminal instance is unchanged
    ///   (history, running processes, agent conversations all preserved).
    /// * `AppActive` (legacy auto-split) — closes the linked sibling terminal
    ///   pane (which `open_app_on_focused` created) and collapses the split.
    ///
    /// If the app was tracking a directory (e.g. the file browser) and the user
    /// navigated away from the opening directory, we propagate the final cwd back
    /// to the underlying terminal by writing `cd '<path>'\n` to its PTY. The shell
    /// is the source of truth for cwd, so we let it run `cd` rather than mutating
    /// any in-memory cwd field.
    pub(crate) fn close_focused_app(&mut self) {
        // Find the focused pane's tile id so we can collapse the split when
        // it was a legacy auto-split. If the app was opened with a companion
        // (no separate tile), `linked` will be None and we're done.
        let ctx = &mut self.contexts[self.active_context];
        let linked = if let Some((pane_id, pane)) = ctx.focused_pane_mut() {
            // Before closing, if the app reports a current directory that differs
            // from the scope it was launched in, cd the underlying terminal to it.
            let final_dir = pane
                .active_app
                .as_ref()
                .and_then(|app| app.current_dir().map(|p| p.to_path_buf()));
            let closing_app_id = pane
                .active_app
                .as_ref()
                .map(|a| a.type_id())
                .unwrap_or("unknown");
            log::info!("app '{closing_app_id}': closed");
            crate::event_log::emit_app_closed(closing_app_id, closing_app_id, pane_id, None);
            if let (Some(final_dir), Some(scope)) = (final_dir, pane.app_scope.clone()) {
                if final_dir != scope {
                    let cmd = format!(
                        "cd {}\n",
                        crate::app::shell_escape(&final_dir.display().to_string())
                    );
                    pane.backend
                        .process_command(BackendCommand::Write(cmd.into_bytes()));
                }
            }
            pane.close_app()
        } else {
            None
        };

        if let Some(linked_id) = linked {
            // Legacy path: there was a separate terminal tile. Remove it
            // from the tree and drop the pane. (Same cleanup as before this
            // refactor — kept verbatim to avoid changing legacy behavior.)
            let ctx = &mut self.contexts[self.active_context];
            let tile_to_remove = ctx
                .tree
                .tiles
                .iter()
                .find(|(_, tile)| matches!(tile, Tile::Pane(id) if *id == linked_id))
                .map(|(tile_id, _)| tile_id);
            if let Some(tile_id) = tile_to_remove {
                ctx.tree.tiles.remove(*tile_id);
            }
            ctx.panes.remove(&linked_id);
        }
    }

    /// Toggle keyboard focus between app surface and terminal command bar.
    pub(crate) fn toggle_focused_surface(&mut self) {
        let ctx = &mut self.contexts[self.active_context];
        if let Some((_pane_id, pane)) = ctx.focused_pane_mut() {
            pane.toggle_surface_focus();
        }
    }

    /// Open an app on the focused pane.
    ///
    /// Preferred path: if the focused tile is already a plain terminal pane
    /// with no app currently active, the app is opened *in the same pane*
    /// using `SurfaceMode::AppWithCompanion`. The user's existing terminal —
    /// including history, running processes, and any agent conversations —
    /// is preserved, and the app overlays the top portion of the same pane.
    /// No new `TerminalPane` is created and the tile tree is unchanged.
    ///
    /// Fallback path (when the focused pane is already showing an app, or
    /// the focused tile is not a plain pane): legacy auto-split behavior —
    /// create a fresh linked terminal pane below and put the app on top.
    pub(crate) fn open_app_on_focused(
        &mut self,
        app: Box<dyn App>,
        permissions: crate::app_permissions::AppPermissions,
        scope: PathBuf,
    ) {
        self.open_app_on_focused_with_launch(app, permissions, scope, None);
    }

    /// Open an app on the focused pane with an optional manifest-declared
    /// launch configuration that overrides the split direction, size, and
    /// companion terminal cwd.
    ///
    /// When `launch` is `None`, the legacy 75/25 vertical split is used and the
    /// companion terminal inherits the previous pane's cwd (unchanged behavior).
    /// When `launch` is `Some`, the split direction, fraction, and companion
    /// cwd come from the config. `{launch_dir}` in `companion_cwd` is resolved
    /// to the app's `scope`.
    pub(crate) fn open_app_on_focused_with_launch(
        &mut self,
        app: Box<dyn App>,
        permissions: crate::app_permissions::AppPermissions,
        scope: PathBuf,
        launch: Option<crate::app_registry::AppLaunchConfig>,
    ) {
        let Some(focused) = self.contexts[self.active_context].focused_pane else {
            return;
        };

        // Fast path: focused tile is a plain terminal pane with no app yet.
        // Open the app in-place, preserving the terminal instance. This
        // preserves shell history, running processes, and agent conversations.
        let can_embed = {
            let ctx = &self.contexts[self.active_context];
            if let Some(Tile::Pane(pane_id)) = ctx.tree.tiles.get(focused) {
                ctx.panes
                    .get(pane_id)
                    .map(|p| p.active_app.is_none())
                    .unwrap_or(false)
            } else {
                false
            }
        };

        // Honor the manifest's [app.launch] mode. When an app declares
        // mode = "fullscreen" (or doesn't opt into a companion terminal),
        // open it in AppActive surface mode — the app owns the whole pane,
        // no embedded terminal. Apps that want the 75/25 split must declare
        // a companion explicitly.
        let wants_fullscreen = launch
            .as_ref()
            .map(|l| l.mode == "fullscreen" || l.companion == "none")
            .unwrap_or(true);

        if can_embed {
            let ctx = &mut self.contexts[self.active_context];
            if let Some(Tile::Pane(pane_id)) = ctx.tree.tiles.get(focused) {
                let pane_id = *pane_id;
                if let Some(pane) = ctx.panes.get_mut(&pane_id) {
                    let type_id = app.type_id().to_string();
                    if wants_fullscreen {
                        log::info!("app '{}': opened on pane {pane_id}", type_id);
                        pane.open_app(app, permissions, scope);
                    } else {
                        log::info!("app '{}': opened on pane {pane_id} with companion", type_id);
                        pane.open_app_with_companion(app, permissions, scope);
                        // Write manifest-declared startup message into this same
                        // pane's terminal grid — it IS the companion.
                        if let Some(msg) =
                            launch.as_ref().and_then(|l| l.startup_message.as_deref())
                        {
                            let bytes = format_startup_message(msg);
                            pane.backend.write_agent_bytes(&bytes);
                        }
                    }
                    crate::event_log::emit_app_spawned(&type_id, &type_id, pane_id);
                }
            }
            ctx.focused_pane = Some(focused);
            return;
        }

        // When the focused pane already has an app and the new app wants
        // fullscreen, replace it in place instead of splitting. This matches
        // the user expectation that fullscreen apps never grow a companion
        // terminal they didn't ask for.
        if wants_fullscreen {
            let ctx = &mut self.contexts[self.active_context];
            if let Some(Tile::Pane(pane_id)) = ctx.tree.tiles.get(focused) {
                let pane_id = *pane_id;
                if let Some(pane) = ctx.panes.get_mut(&pane_id) {
                    let type_id = app.type_id().to_string();
                    log::info!(
                        "app '{}': replacing existing app on pane {pane_id}",
                        type_id
                    );
                    pane.close_app();
                    pane.open_app(app, permissions, scope);
                    crate::event_log::emit_app_spawned(&type_id, &type_id, pane_id);
                }
            }
            ctx.focused_pane = Some(focused);
            return;
        }

        // Fallback: legacy auto-split. Create a new terminal pane below and
        // put the app in the existing focused pane.

        // Resolve launch options (defaults match legacy behavior).
        let vertical = match launch.as_ref().map(|l| l.companion_position.as_str()) {
            Some("right") => false,
            _ => true, // "bottom" or unset → vertical split (app on top)
        };
        // Fraction allocated to the companion terminal.
        let companion_frac = launch
            .as_ref()
            .map(|l| l.companion_size.clamp(0.05, 0.95))
            .unwrap_or(0.25);

        // Companion terminal cwd:
        //   - with launch config + `{launch_dir}` template → app's scope
        //   - otherwise → previous focused pane's cwd (legacy)
        let companion_cwd = if let Some(cfg) = launch.as_ref() {
            if cfg.companion_cwd.contains("{launch_dir}") {
                scope.clone()
            } else {
                PathBuf::from(&cfg.companion_cwd)
            }
        } else {
            self.contexts[self.active_context]
                .get_focused_pane_cwd(focused)
                .unwrap_or_else(|| scope.clone())
        };

        // Create the companion terminal pane.
        let new_term_id = self.next_pane_id;
        self.next_pane_id += 1;

        let settings = Self::make_backend_settings(Some(companion_cwd), &self.colors);
        let Some(mut new_pane) = TerminalPane::new(
            new_term_id,
            self.ctx.clone(),
            self.pty_event_tx.clone(),
            settings,
            self.default_font_size,
        ) else {
            log::error!("Failed to create linked terminal pane for app");
            return;
        };
        // Write manifest-declared startup message into the new companion
        // terminal's grid before inserting it into the context.
        if let Some(msg) = launch.as_ref().and_then(|l| l.startup_message.as_deref()) {
            let bytes = format_startup_message(msg);
            new_pane.backend.write_agent_bytes(&bytes);
        }
        self.contexts[self.active_context]
            .panes
            .insert(new_term_id, new_pane);

        // Split using the exact same logic as split_focused (which works).
        let split_target = match self.contexts[self.active_context].find_ancestor_tabs(focused) {
            Some((tabs_id, _)) => tabs_id,
            None => focused,
        };

        let split_dir = if vertical {
            egui_tiles::LinearDir::Vertical
        } else {
            egui_tiles::LinearDir::Horizontal
        };

        let ctx = &mut self.contexts[self.active_context];
        let parent = ctx.tree.tiles.parent_of(split_target);
        let new_tile = ctx.tree.tiles.insert_pane(new_term_id);

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

            // Set app / companion share ratio from config (or 0.75 / 0.25 default).
            if let Some(Tile::Container(Container::Linear(ref mut lin))) =
                ctx.tree.tiles.get_mut(container_tile)
            {
                let app_share = (1.0 - companion_frac).max(0.05);
                lin.shares.set_share(split_target, app_share);
                lin.shares.set_share(new_tile, companion_frac);
            }

            if let Some(parent_id) = parent {
                if let Some(Tile::Container(parent_container)) = ctx.tree.tiles.get_mut(parent_id) {
                    replace_child(parent_container, split_target, container_tile);
                }
            } else {
                ctx.tree.root = Some(container_tile);
            }
        }

        // Set the app on the focused (top) pane and link to the companion terminal.
        if let Some(egui_tiles::Tile::Pane(pane_id)) = ctx.tree.tiles.get(focused) {
            let pane_id = *pane_id;
            if let Some(pane) = ctx.panes.get_mut(&pane_id) {
                log::info!(
                    "app '{}': opened on pane {pane_id} with companion terminal {new_term_id}",
                    app.type_id()
                );
                pane.open_app(app, permissions, scope);
                pane.linked_terminal_pane = Some(new_term_id);
            }
        }

        // Focus stays on the app pane (not the new terminal).
        ctx.focused_pane = Some(focused);
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
                    if let Some(app) = &pane.active_app {
                        if app.type_id() == "file_browser" {
                            // Close the file browser.
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

        // Built-in file browser gets full permissions.
        let perms = crate::app_permissions::AppPermissions::builtin();
        self.open_app_on_focused(app, perms, cwd);
    }

    /// Toggle the depth tree browser in the focused pane.
    pub(crate) fn open_depth_tree(&mut self) {
        let ctx = &self.contexts[self.active_context];
        if let Some(focused) = ctx.focused_pane {
            if let Some(egui_tiles::Tile::Pane(pane_id)) = ctx.tree.tiles.get(focused) {
                let pane_id = *pane_id;
                if let Some(pane) = ctx.panes.get(&pane_id) {
                    if let Some(app) = &pane.active_app {
                        if app.type_id() == "depth_tree" {
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

        let app = Box::new(crate::depth_tree_app::DepthTreeApp::new(cwd.clone()));
        let perms = crate::app_permissions::AppPermissions::builtin();
        self.open_app_fullscreen(app, perms, cwd);
    }

    /// Open an app on the focused pane WITHOUT creating a linked terminal split.
    /// The app takes the full pane. Used for apps like Quick Note that don't
    /// need a terminal.
    pub(crate) fn open_app_fullscreen(
        &mut self,
        app: Box<dyn App>,
        permissions: crate::app_permissions::AppPermissions,
        scope: PathBuf,
    ) {
        let ctx = &mut self.contexts[self.active_context];
        if let Some((pane_id, pane)) = ctx.focused_pane_mut() {
            let type_id = app.type_id().to_string();
            log::info!("app '{}': opened fullscreen on pane {pane_id}", type_id);
            pane.open_app(app, permissions, scope);
            // No linked terminal — app takes the full pane.
            crate::event_log::emit_app_spawned(&type_id, &type_id, pane_id);
        }
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
        self.open_app_fullscreen(app, perms, cwd);
    }

    /// Open the audio player app scoped to the current directory.
    pub(crate) fn open_audio_player(&mut self) {
        let cwd = {
            let ctx = &self.contexts[self.active_context];
            ctx.focused_pane
                .and_then(|tile_id| ctx.get_focused_pane_cwd(tile_id))
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
        };

        // Toggle: if audio player already open, close it.
        let ctx = &self.contexts[self.active_context];
        if let Some(focused) = ctx.focused_pane {
            if let Some(egui_tiles::Tile::Pane(pane_id)) = ctx.tree.tiles.get(focused) {
                if let Some(pane) = ctx.panes.get(pane_id) {
                    if let Some(app) = &pane.active_app {
                        if app.type_id() == "audio_player" {
                            self.close_focused_app();
                            return;
                        }
                    }
                }
            }
        }

        let app = Box::new(crate::audio_app::AudioApp::new(cwd.clone()));
        // Audio player: read-only filesystem, no terminal write.
        let mut perms = crate::app_permissions::AppPermissions::default();
        perms.filesystem = crate::app_permissions::FsPermission::ReadOnly;
        self.open_app_fullscreen(app, perms, cwd);
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
        self.open_app_fullscreen(Box::new(editor), perms, scope);
    }

    /// Launch an installed app by id in the focused pane.
    pub(crate) fn launch_app_by_id(&mut self, id: &str) {
        let cwd = self.contexts[self.active_context]
            .focused_pane
            .and_then(|fp| self.contexts[self.active_context].get_focused_pane_cwd(fp))
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));

        // Snapshot the manifest-declared launch config BEFORE calling into the
        // registry (keeps the borrow of `self.registry` short).
        let launch_cfg = self
            .registry
            .app_by_id(id)
            .and_then(|installed| installed.manifest.launch.clone());

        if let Some(app) = self.registry.launch(id, &cwd, &[]) {
            let perms = crate::app_permissions::AppPermissions::default();
            self.open_app_on_focused_with_launch(app, perms, cwd, launch_cfg);
            self.record_app_visit(id);
        } else {
            log::warn!("launch_app_by_id: app '{id}' not found or failed to launch");
        }
    }

    /// Open the secrets manager (read-only vault viewer, full pane, no terminal split).
    pub(crate) fn open_secrets_manager(&mut self) {
        // Toggle: if already open, close it.
        let ctx = &self.contexts[self.active_context];
        if let Some(focused) = ctx.focused_pane {
            if let Some(egui_tiles::Tile::Pane(pane_id)) = ctx.tree.tiles.get(focused) {
                if let Some(pane) = ctx.panes.get(pane_id) {
                    if let Some(app) = &pane.active_app {
                        if app.type_id() == "secrets_manager" {
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
        self.open_app_fullscreen(app, perms, cwd);
    }

    /// Open the appropriate app for a file, based on its extension.
    /// Falls back to opening the file path in the terminal if no app is registered.
    pub(crate) fn open_file_with_app(&mut self, file_path: PathBuf) {
        let cwd = file_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));

        if let Some(app) = self.registry.launch_for_file(&file_path, &cwd) {
            // Third-party app — sandboxed by default, scoped to launch directory.
            let perms = crate::app_permissions::AppPermissions::default();
            // If the app declared [app.launch], honor its companion split config.
            let launch_cfg = self
                .registry
                .app_by_id(app.type_id())
                .and_then(|installed| installed.manifest.launch.clone());
            self.open_app_on_focused_with_launch(app, perms, cwd.clone(), launch_cfg);
        } else {
            // No registered app — fall back to writing the path into the terminal.
            let ctx = &mut self.contexts[self.active_context];
            if let Some((_pane_id, pane)) = ctx.focused_pane_mut() {
                let path_str = file_path.display().to_string();
                let escaped = if path_str
                    .contains(|c: char| c.is_whitespace() || "\"'\\()&|;$`!#".contains(c))
                {
                    format!("'{}'", path_str.replace('\'', "'\\''"))
                } else {
                    path_str
                };
                pane.backend
                    .process_command(egui_term::BackendCommand::Write(escaped.into_bytes()));
            }
        }
    }

    /// Toggle agent mode on the focused pane.
    pub(crate) fn toggle_agent_mode(&mut self) {
        let ctx = &mut self.contexts[self.active_context];
        if let Some((_pane_id, pane)) = ctx.focused_pane_mut() {
            // Don't activate agent mode if an app is already open
            if pane.active_app.is_some() {
                return;
            }
            if pane.agent_mode.is_active() {
                log::info!("agent mode: deactivated");
                pane.agent_mode.deactivate();
                // Write \r to the PTY so ZSH redraws its prompt cleanly at
                // the current cursor position. Without this, ZSH doesn't know
                // agent mode ended and the cursor lands visually displaced.
                pane.backend
                    .process_command(BackendCommand::Write(b"\r".to_vec()));
            } else {
                // Update directory scope from terminal CWD before activating
                if let Some(cwd) = crate::shell::get_pid_cwd(pane.backend.child_pid()) {
                    pane.agent_mode.directory_scope = cwd;
                }
                log::info!("agent mode: activated");
                pane.agent_mode.activate();
            }
        }
    }

    /// Trigger undo on the focused app.
    pub(crate) fn app_undo(&mut self) {
        let ctx = &mut self.contexts[self.active_context];
        if let Some((_pane_id, pane)) = ctx.focused_pane_mut() {
            if let Some(app) = &mut pane.active_app {
                app.undo();
            }
        }
    }

    /// Trigger redo on the focused app.
    pub(crate) fn app_redo(&mut self) {
        let ctx = &mut self.contexts[self.active_context];
        if let Some((_pane_id, pane)) = ctx.focused_pane_mut() {
            if let Some(app) = &mut pane.active_app {
                app.redo();
            }
        }
    }

    // ─── Spawn dispatcher ────────────────────────────────────────────────────

    /// Drain pending spawn requests from every active app and execute them.
    /// Called once per frame from `update()`, after key dispatch and cwd sync.
    pub(crate) fn dispatch_pending_spawns(&mut self) {
        let active = self.active_context;

        // Two-pass: collect first so we don't hold a mutable borrow while
        // calling execute_spawn (which also borrows self mutably).
        let mut to_spawn: Vec<(PaneId, crate::app_protocol::PendingSpawn)> = Vec::new();
        for (&pane_id, pane) in self.contexts[active].panes.iter_mut() {
            if let Some(app) = pane.active_app.as_mut() {
                for spawn in app.take_pending_spawns() {
                    to_spawn.push((pane_id, spawn));
                }
            }
        }

        for (requester_pane_id, spawn) in to_spawn {
            self.execute_spawn(requester_pane_id, spawn);
        }
    }

    // ─── Pipe dispatcher ─────────────────────────────────────────────────────

    /// Route pending PipeWrite commands from all active apps to their connected
    /// peers (parent and/or children via spawn_relationships).
    ///
    /// Called once per frame from `update()`, after `dispatch_pending_spawns()`.
    /// Uses the standard two-phase collect-then-route pattern to avoid holding
    /// a mutable borrow on `panes` across the routing pass.
    pub(crate) fn dispatch_pipe_writes(&mut self) {
        let active = self.active_context;

        // Phase 1: collect all pending pipe writes with sender context.
        // Each entry: (sender_pane_id, sender_app_id, channel, value)
        let mut pipe_writes: Vec<(PaneId, String, String, serde_json::Value)> = Vec::new();
        for (&pane_id, pane) in self.contexts[active].panes.iter_mut() {
            if let Some(app) = pane.active_app.as_mut() {
                for (channel, value) in app.take_pipe_writes() {
                    let app_id = app.type_id().to_string();
                    pipe_writes.push((pane_id, app_id, channel, value));
                }
            }
        }

        if pipe_writes.is_empty() {
            return;
        }

        // Phase 2: for each write, route to connected peers and log to event bus.
        for (sender_pane_id, sender_app_id, channel, value) in pipe_writes {
            // Emit to event log (no payload value — summary only).
            crate::event_log::emit_pipe_write(&sender_app_id, &channel);

            // Route to parent (if this pane is a child).
            let parent_pane_id = self.spawn_relationships.parent_of(sender_pane_id);
            if let Some(parent_id) = parent_pane_id {
                if let Some(pane) = self.contexts[active].panes.get_mut(&parent_id) {
                    if let Some(app) = pane.active_app.as_mut() {
                        app.send_pipe_data(&sender_app_id, &channel, &value);
                    }
                }
            }

            // Route to all children (if this pane is a parent).
            let child_pane_ids: Vec<PaneId> = self
                .spawn_relationships
                .children_of(sender_pane_id)
                .iter()
                .map(|r| r.child_pane)
                .collect();
            for child_id in child_pane_ids {
                if let Some(pane) = self.contexts[active].panes.get_mut(&child_id) {
                    if let Some(app) = pane.active_app.as_mut() {
                        app.send_pipe_data(&sender_app_id, &channel, &value);
                    }
                }
            }
        }
    }

    /// Resolve and execute one `PendingSpawn`, creating a new pane in the tree.
    fn execute_spawn(
        &mut self,
        requester_pane_id: PaneId,
        spawn: crate::app_protocol::PendingSpawn,
    ) {
        use crate::app_protocol::{SpawnLayout, SpawnLifecycle};

        // 1. Gather context from the requester pane.
        let (requester_app_id, scope) = {
            let ctx = &self.contexts[self.active_context];
            let pane = match ctx.panes.get(&requester_pane_id) {
                Some(p) => p,
                None => {
                    log::warn!(
                        "execute_spawn: requester pane {} not found",
                        requester_pane_id
                    );
                    return;
                }
            };
            let app_id = pane
                .active_app
                .as_ref()
                .map(|a| a.type_id().to_string())
                .unwrap_or_default();
            let scope = pane
                .app_scope
                .clone()
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
            (app_id, scope)
        };

        // 2. Launch the child app subprocess.
        // For built-in apps not in the registry (file_browser), fall back to
        // the in-process implementation so spawning always works.
        let child_app: Box<dyn crate::app_trait::App> = if let Some(a) = self
            .registry
            .launch_as_child(&spawn.app_id, &scope, &spawn.args, &requester_app_id)
        {
            a
        } else if spawn.app_id == "file_browser" || spawn.app_id == "file-browser" {
            Box::new(crate::file_browser::FileBrowserApp::from_args(
                scope.clone(),
                &spawn.args,
            ))
        } else {
            log::warn!(
                "execute_spawn: app '{}' not found in registry",
                spawn.app_id
            );
            return;
        };

        // 3. Create a new TerminalPane for the child and open the app in it.
        let child_pane_id = self.next_pane_id;
        self.next_pane_id += 1;

        let settings = Self::make_backend_settings(Some(scope.clone()), &self.colors);
        let mut child_pane = match TerminalPane::new(
            child_pane_id,
            self.ctx.clone(),
            self.pty_event_tx.clone(),
            settings,
            self.default_font_size,
        ) {
            Some(p) => p,
            None => {
                log::error!(
                    "execute_spawn: failed to create terminal pane for '{}'",
                    spawn.app_id
                );
                return;
            }
        };
        let perms = crate::app_permissions::AppPermissions::default();
        child_pane.open_app_with_companion(child_app, perms, scope);

        // 4. Find the anchor tile in the tree (the tile containing the requester pane).
        let anchor_tile = {
            let ctx = &self.contexts[self.active_context];
            // SpawnParent::SelfPane and NamedMark (v1: falls back to SelfPane) anchor
            // to the requester's own tile. SpawnParent::Root anchors to the tree root.
            match &spawn.parent {
                crate::app_protocol::SpawnParent::Root => ctx.tree.root,
                _ => ctx.tree.tiles.find_pane(&requester_pane_id),
            }
        };
        let anchor_tile = match anchor_tile {
            Some(t) => t,
            None => {
                log::warn!(
                    "execute_spawn: could not find anchor tile for pane {}",
                    requester_pane_id
                );
                return;
            }
        };

        // 5. Insert pane into the panes map and create a tile.
        self.contexts[self.active_context]
            .panes
            .insert(child_pane_id, child_pane);
        let new_tile = self.contexts[self.active_context]
            .tree
            .tiles
            .insert_pane(child_pane_id);

        // 6. Determine split direction, new-pane position, and size ratio.
        let (split_dir, new_before_anchor, ratio) = match &spawn.layout {
            SpawnLayout::Cols { slot, ratio } => {
                (egui_tiles::LinearDir::Horizontal, *slot == 0, *ratio)
            }
            SpawnLayout::Rows { slot, ratio } => {
                (egui_tiles::LinearDir::Vertical, *slot == 0, *ratio)
            }
            // Fill and unsupported variants: default to right column at 50 %.
            _ => (egui_tiles::LinearDir::Horizontal, false, 0.5),
        };

        // Share fractions: anchor keeps `anchor_share`, new pane gets `new_share`.
        let (anchor_share, new_share) = if new_before_anchor {
            (1.0 - ratio, ratio)
        } else {
            (ratio, 1.0 - ratio)
        };

        // 7. Insert the new tile next to the anchor in the tile tree.
        let ctx = &mut self.contexts[self.active_context];
        let parent = ctx.tree.tiles.parent_of(anchor_tile);

        let inserted_as_sibling = if let Some(parent_id) = parent {
            if let Some(Tile::Container(Container::Linear(linear))) =
                ctx.tree.tiles.get_mut(parent_id)
            {
                if linear.dir == split_dir {
                    if let Some(pos) = linear.children.iter().position(|&c| c == anchor_tile) {
                        if new_before_anchor {
                            linear.children.insert(pos, new_tile);
                        } else {
                            linear.children.insert(pos + 1, new_tile);
                        }
                        // Update share ratios.
                        linear.shares.set_share(anchor_tile, anchor_share);
                        linear.shares.set_share(new_tile, new_share);
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
            // Wrap anchor + new pane in a fresh linear container.
            let children = if new_before_anchor {
                vec![new_tile, anchor_tile]
            } else {
                vec![anchor_tile, new_tile]
            };
            let container_tile = if split_dir == egui_tiles::LinearDir::Vertical {
                ctx.tree.tiles.insert_vertical_tile(children)
            } else {
                ctx.tree.tiles.insert_horizontal_tile(children)
            };
            // Set ratios on the new container.
            if let Some(Tile::Container(Container::Linear(lin))) =
                ctx.tree.tiles.get_mut(container_tile)
            {
                lin.shares.set_share(anchor_tile, anchor_share);
                lin.shares.set_share(new_tile, new_share);
            }
            if let Some(parent_id) = parent {
                if let Some(Tile::Container(parent_container)) = ctx.tree.tiles.get_mut(parent_id) {
                    replace_child(parent_container, anchor_tile, container_tile);
                }
            } else {
                ctx.tree.root = Some(container_tile);
            }
        }

        // 8. Record the relationship so cascade/orphan close works.
        let lifecycle = spawn.lifecycle;
        let wire_channels = spawn.wire_channels.clone();
        self.spawn_relationships
            .add(requester_pane_id, child_pane_id, lifecycle, wire_channels);

        log::info!(
            "execute_spawn: spawned '{}' (pane {}) from '{}' (pane {}) lifecycle={:?}",
            spawn.app_id,
            child_pane_id,
            requester_app_id,
            requester_pane_id,
            lifecycle,
        );
    }

    /// Wire cascade/orphan semantics into a pane close. Call this BEFORE the
    /// tile is actually removed from the tree so children can still be found.
    ///
    /// - `Cascade`: close all transitive children alongside the closing pane.
    /// - `Orphan`/`Prompt`: drop the relationship; children survive as top-level panes.
    pub(crate) fn handle_spawn_pane_close(&mut self, closing_pane_id: PaneId) {
        use crate::app_protocol::SpawnLifecycle;

        // Collect direct children and their lifecycles.
        let children: Vec<(PaneId, SpawnLifecycle)> = self
            .spawn_relationships
            .children_of(closing_pane_id)
            .iter()
            .map(|r| (r.child_pane, r.lifecycle))
            .collect();

        // Remove all relationships involving this pane.
        self.spawn_relationships.remove_pane(closing_pane_id);

        for (child_pane_id, lifecycle) in children {
            match lifecycle {
                SpawnLifecycle::Cascade => {
                    // Recurse: close children of children first.
                    self.handle_spawn_pane_close(child_pane_id);
                    self.close_pane_by_id(child_pane_id);
                }
                SpawnLifecycle::Orphan | SpawnLifecycle::Prompt => {
                    // Drop the relationship — child survives.
                    // (Prompt is stubbed as Orphan in v1.)
                }
            }
        }
    }

    /// Focus a pane by its `PaneId` (u64). Searches all contexts.
    /// Optionally enters fullscreen (zoomed) mode on the target pane.
    /// No-op if no pane with that id is found.
    pub(crate) fn focus_pane_by_id(&mut self, pane_id: u64, fullscreen: bool) {
        for ci in 0..self.contexts.len() {
            if let Some(tile_id) = self.contexts[ci].tree.tiles.find_pane(&pane_id) {
                self.active_context = ci;
                self.contexts[ci].focused_pane = Some(tile_id);
                if fullscreen {
                    self.contexts[ci].zoomed_pane = Some(tile_id);
                }
                self.contexts[ci].activate_tab_for(tile_id);
                return;
            }
        }
        log::debug!("focus_pane_by_id: pane {pane_id} not found");
    }

    // ── Z-axis depth navigation ────────────────────────────────────────────

    /// Descend into a child `.plexi` depth boundary. Creates a new context
    /// rooted at `path`, pushes the current context onto the depth stack,
    /// and switches to the new context.
    pub(crate) fn descend_to_depth(&mut self, path: PathBuf) {
        if !path.is_dir() {
            log::warn!(
                "descend_to_depth: path is not a directory: {}",
                path.display()
            );
            return;
        }
        if !path.join(".plexi").is_dir() {
            log::warn!(
                "descend_to_depth: no .plexi boundary at {}",
                path.display()
            );
            return;
        }

        // Guard: don't descend into the current context's own path.
        if self.contexts[self.active_context].path == path {
            log::debug!(
                "descend_to_depth: already at {}, ignoring",
                path.display()
            );
            return;
        }

        let prev_depth = self.depth_stack.len() as u32;
        let prev_ctx = self.active_context;
        let prev_path = self.contexts[prev_ctx].path.clone();

        // Push current context onto the depth stack.
        // Clear the zoom — the zoom was the entry gesture, not a state to
        // preserve. The parent should show its full layout on return.
        self.contexts[prev_ctx].zoomed_pane = None;
        self.depth_stack
            .push((prev_ctx, prev_path.clone()));

        // Reuse an existing context for this path if one exists (preserves
        // whatever pane layout the user built there previously).
        if let Some(existing_idx) = self.contexts.iter().position(|c| c.path == path) {
            log::info!(
                "depth: re-entering existing context for {} (depth {})",
                path.display(),
                self.depth_stack.len()
            );
            self.active_context = existing_idx;
        } else {
            // First visit — read .plexi/config.toml for workspace config.
            let depth_config = read_depth_config(&path);
            let pane_count = depth_config
                .as_ref()
                .map(|c| c.apps.len().max(1))
                .unwrap_or(2);

            let Some((tree, panes, tile_ids)) =
                self.create_n_pane_tree(pane_count, Some(path.clone()))
            else {
                log::error!(
                    "descend_to_depth: failed to create terminal for {}",
                    path.display()
                );
                self.depth_stack.pop();
                return;
            };

            let config_name = depth_config.as_ref().and_then(|c| c.name.clone());
            let name = config_name.unwrap_or_else(|| {
                format!(
                    "depth {} — {}",
                    self.depth_stack.len(),
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string())
                )
            });
            log::info!(
                "depth: descending to {} (depth {})",
                path.display(),
                self.depth_stack.len()
            );

            let first_tile = tile_ids.first().copied();
            self.contexts.push(Context {
                name,
                path: path.clone(),
                tree,
                panes,
                focused_pane: first_tile,
                zoomed_pane: None,
            });
            self.active_context = self.contexts.len() - 1;

            // Launch configured apps into the panes.
            if let Some(config) = depth_config {
                for (i, app_id) in config.apps.iter().enumerate() {
                    if let Some(&tile_id) = tile_ids.get(i) {
                        // Focus this pane, then launch the app.
                        self.contexts[self.active_context].focused_pane = Some(tile_id);
                        self.launch_app_by_id(app_id);
                    }
                }
                // Restore focus to first pane.
                self.contexts[self.active_context].focused_pane = first_tile;
            }
        }

        // Emit depth transition event.
        crate::event_log::emit_depth_transition(
            Some(prev_path.display().to_string()),
            path.display().to_string(),
            Some(prev_depth),
            self.depth_stack.len() as u32,
            "descend",
        );
    }

    /// Ascend one depth level. Switches back to the parent context and
    /// removes the depth entry from the stack.
    pub(crate) fn ascend_depth(&mut self) {
        let Some((parent_ctx_idx, parent_path)) = self.depth_stack.pop() else {
            log::debug!("ascend_depth: already at root depth, nothing to do");
            return;
        };

        let current_path = self.contexts[self.active_context].path.clone();
        let prev_depth = (self.depth_stack.len() + 1) as u32;

        // Switch back to the parent context. Clear zoom so the parent
        // shows its full layout — ascending = zooming out.
        if parent_ctx_idx < self.contexts.len() {
            log::info!(
                "depth: ascending to {} (depth {})",
                parent_path.display(),
                self.depth_stack.len()
            );
            self.active_context = parent_ctx_idx;
            self.contexts[parent_ctx_idx].zoomed_pane = None;
        } else {
            log::warn!(
                "ascend_depth: parent context {} no longer exists, staying at current",
                parent_ctx_idx
            );
            return;
        }

        // Emit depth transition event.
        crate::event_log::emit_depth_transition(
            Some(current_path.display().to_string()),
            parent_path.display().to_string(),
            Some(prev_depth),
            self.depth_stack.len() as u32,
            "ascend",
        );
    }
}

// ─── Depth config ─────────────────────────────────────────────────────────

/// Configuration read from `.plexi/config.toml` at a depth boundary.
struct DepthConfig {
    name: Option<String>,
    apps: Vec<String>,
}

/// Read `.plexi/config.toml` from a directory if it exists.
/// Expected format:
/// ```toml
/// name = "Dev Environment"
/// apps = ["git-log", "file-browser"]
/// ```
fn read_depth_config(dir: &std::path::Path) -> Option<DepthConfig> {
    let config_path = dir.join(".plexi").join("config.toml");
    let content = std::fs::read_to_string(&config_path).ok()?;
    let table: toml::Table = content.parse().ok()?;
    let name = table
        .get("name")
        .and_then(|v| v.as_str())
        .map(String::from);
    let apps = table
        .get("apps")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Some(DepthConfig { name, apps })
}

/// Format a manifest-declared `[app.launch].startup_message` into dim italic
/// ANSI bytes suitable for `TerminalBackend::write_agent_bytes`. Wrapped in
/// leading/trailing CRLF so the message always lands on its own line
/// regardless of where the cursor was.
fn format_startup_message(msg: &str) -> Vec<u8> {
    format!("\r\n\x1b[2;3m{msg}\x1b[0m\r\n").into_bytes()
}
