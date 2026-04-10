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
        let pane = TerminalPane::new(new_id, self.ctx.clone(), self.pty_event_tx.clone(), settings, self.default_font_size)?;

        let mut panes = HashMap::new();
        panes.insert(new_id, pane);

        let mut tiles = egui_tiles::Tiles::default();
        let root_tile = tiles.insert_pane(new_id);
        let tree = Tree::new("plexi", root_tile, tiles);

        Some((tree, panes, root_tile))
    }

    pub(crate) fn split_focused(&mut self, vertical: bool) {
        let Some(focused) = self.contexts[self.active_context].focused_pane else {
            return;
        };

        let new_id = self.next_pane_id;
        self.next_pane_id += 1;

        let cwd = self.contexts[self.active_context].get_focused_pane_cwd(focused);
        let settings = Self::make_backend_settings(cwd, &self.colors);
        let Some(pane) =
            TerminalPane::new(new_id, self.ctx.clone(), self.pty_event_tx.clone(), settings, self.default_font_size)
        else {
            log::error!("Failed to create new terminal pane");
            return;
        };
        self.contexts[self.active_context]
            .panes
            .insert(new_id, pane);

        let split_target =
            match self.contexts[self.active_context].find_ancestor_tabs(focused) {
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
        let Some(pane) =
            TerminalPane::new(new_id, self.ctx.clone(), self.pty_event_tx.clone(), settings, self.default_font_size)
        else {
            log::error!("Failed to create new terminal pane");
            return;
        };
        self.contexts[self.active_context]
            .panes
            .insert(new_id, pane);

        let ctx = &mut self.contexts[self.active_context];
        let new_tile = ctx.tree.tiles.insert_pane(new_id);

        if let Some((tabs_id, _)) = ctx.find_ancestor_tabs(focused) {
            if let Some(Tile::Container(Container::Tabs(tabs))) =
                ctx.tree.tiles.get_mut(tabs_id)
            {
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

        // Phase 1: Read-only — determine sibling and container type
        let parent_info = self.contexts[self.active_context].find_logical_parent(focused);

        let next = if let Some((parent_id, child_in_parent)) = parent_info {
            let sibling_info = {
                let ctx = &self.contexts[self.active_context];
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
                let ctx = &mut self.contexts[self.active_context];
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

                self.contexts[self.active_context].find_first_pane_in(sibling)
            } else {
                self.contexts[self.active_context].find_next_focus(focused)
            }
        } else {
            self.contexts[self.active_context].find_next_focus(focused)
        };

        // Phase 3: Remove tile and pane
        let ctx = &mut self.contexts[self.active_context];
        if let Some(parent_id) = ctx.tree.tiles.parent_of(focused) {
            if let Some(Tile::Container(parent)) = ctx.tree.tiles.get_mut(parent_id) {
                parent.remove_child(focused);
            }
        }

        if let Some(Tile::Pane(pane_id)) = ctx.tree.tiles.remove(focused) {
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
        let Some(focused_tile) = ctx.focused_pane else { return };
        let Some(Tile::Pane(pane_id)) = ctx.tree.tiles.get(focused_tile) else { return };
        let pane_id = *pane_id;
        if let Some(pane) = ctx.panes.get_mut(&pane_id) {
            pane.backend.process_command(BackendCommand::Scroll(lines));
        }
    }

    pub(crate) fn adjust_focused_pane_font_size(&mut self, delta: f32) {
        let ctx = &mut self.contexts[self.active_context];
        let Some(focused_tile) = ctx.focused_pane else { return };
        let Some(Tile::Pane(pane_id)) = ctx.tree.tiles.get(focused_tile) else { return };
        let pane_id = *pane_id;
        if let Some(pane) = ctx.panes.get_mut(&pane_id) {
            pane.font_size = (pane.font_size + delta).clamp(8.0, 32.0);
        }
    }

    /// Close the active app on the focused terminal pane, returning to full terminal.
    /// Also closes the linked terminal pane and collapses the split.
    pub(crate) fn close_focused_app(&mut self) {
        let ctx = &mut self.contexts[self.active_context];
        let linked = if let Some((_pane_id, pane)) = ctx.focused_pane_mut() {
            pane.close_app()
        } else {
            None
        };

        // Close the linked terminal pane if it exists.
        if let Some(linked_id) = linked {
            // Find the tile ID for the linked pane and remove it.
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

    /// Open an app on the focused pane: auto-splits vertically, app on top,
    /// fresh linked terminal on bottom.
    pub(crate) fn open_app_on_focused(
        &mut self,
        app: Box<dyn App>,
        permissions: crate::app_permissions::AppPermissions,
        scope: PathBuf,
    ) {
        let Some(focused) = self.contexts[self.active_context].focused_pane else {
            return;
        };

        // Create a new terminal pane for the bottom split (same as split_focused).
        let new_term_id = self.next_pane_id;
        self.next_pane_id += 1;

        let cwd = self.contexts[self.active_context]
            .get_focused_pane_cwd(focused)
            .unwrap_or_else(|| scope.clone());

        let settings = Self::make_backend_settings(Some(cwd), &self.colors);
        let Some(new_pane) = TerminalPane::new(
            new_term_id,
            self.ctx.clone(),
            self.pty_event_tx.clone(),
            settings,
            self.default_font_size,
        ) else {
            log::error!("Failed to create linked terminal pane for app");
            return;
        };
        self.contexts[self.active_context]
            .panes
            .insert(new_term_id, new_pane);

        // Split using the exact same logic as split_focused (which works).
        let split_target =
            match self.contexts[self.active_context].find_ancestor_tabs(focused) {
                Some((tabs_id, _)) => tabs_id,
                None => focused,
            };

        let ctx = &mut self.contexts[self.active_context];
        let parent = ctx.tree.tiles.parent_of(split_target);
        let new_tile = ctx.tree.tiles.insert_pane(new_term_id);

        let inserted_as_sibling = if let Some(parent_id) = parent {
            if let Some(Tile::Container(Container::Linear(linear))) =
                ctx.tree.tiles.get_mut(parent_id)
            {
                if linear.dir == egui_tiles::LinearDir::Vertical {
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
            let container_tile = ctx
                .tree
                .tiles
                .insert_vertical_tile(vec![split_target, new_tile]);

            // Set 75/25 ratio for app (top) vs terminal (bottom).
            if let Some(Tile::Container(Container::Linear(ref mut lin))) =
                ctx.tree.tiles.get_mut(container_tile)
            {
                lin.shares.set_share(split_target, 3.0);
                lin.shares.set_share(new_tile, 1.0);
            }

            if let Some(parent_id) = parent {
                if let Some(Tile::Container(parent_container)) =
                    ctx.tree.tiles.get_mut(parent_id)
                {
                    replace_child(parent_container, split_target, container_tile);
                }
            } else {
                ctx.tree.root = Some(container_tile);
            }
        }

        // Set the app on the focused (top) pane and link to the bottom terminal.
        if let Some(egui_tiles::Tile::Pane(pane_id)) = ctx.tree.tiles.get(focused) {
            let pane_id = *pane_id;
            if let Some(pane) = ctx.panes.get_mut(&pane_id) {
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
        if let Some((_pane_id, pane)) = ctx.focused_pane_mut() {
            pane.open_app(app, permissions, scope);
            // No linked terminal — app takes the full pane.
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
            let _ = std::fs::write(&config_path, "# Plexi configuration\n# See docs for options\n");
        }
        let scope = config_path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
        let editor = crate::text_editor_app::TextEditorApp::from_file(config_path);
        let perms = crate::app_permissions::AppPermissions::builtin();
        self.open_app_fullscreen(Box::new(editor), perms, scope);
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
            self.open_app_on_focused(app, perms, cwd.clone());
        } else {
            // No registered app — fall back to writing the path into the terminal.
            let ctx = &mut self.contexts[self.active_context];
            if let Some((_pane_id, pane)) = ctx.focused_pane_mut() {
                let path_str = file_path.display().to_string();
                let escaped = if path_str.contains(|c: char| c.is_whitespace() || "\"'\\()&|;$`!#".contains(c)) {
                    format!("'{}'", path_str.replace('\'', "'\\''"))
                } else {
                    path_str
                };
                pane.backend
                    .process_command(egui_term::BackendCommand::Write(escaped.into_bytes()));
            }
        }
    }
}
