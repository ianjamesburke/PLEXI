//! Pane / app / agent creation and launch entry points.
//!
//! These methods own the "make a new pane appear" side of the pane API —
//! ProcessApp wiring, builtin app wiring, single-pane tree creation for new
//! contexts, and the launch helpers called from the command palette and
//! HostCommand routing. The actual tile-tree mutation is delegated to
//! [`PlexiApp::split_with_new_pane`] in [`super::layout`].

use crate::app::PlexiApp;
use crate::app_trait::App;
use crate::host::command::{HostCommand, OpenPaneRequest, PaneRuntimeKind, Placement, ShareRatio};
use crate::host::effect::HostEffect;
use crate::pane::{Pane, TerminalPane};
use crate::tiling::PaneId;
use egui_tiles::{Tile, TileId, Tree};
use std::collections::HashMap;
use std::path::PathBuf;

impl PlexiApp {
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

    pub(crate) fn open_builtin_app_pane(
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

    pub(super) fn create_single_pane_tree(
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
    ///
    /// Manifests declaring `[app] type = "agent"` (#338) bypass the normal
    /// app-canvas pane and land in `Pane::Agent` with the subprocess backend.
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

        // Agent path (#338) — manifest type=agent gets the conversation UI
        // backed by the subprocess. Background re-attach is intentionally
        // not supported for agents in this PR (parking + resume of an
        // active conversation is its own design problem; tracked under
        // v3.3.5+).
        if matches!(
            self.registry.manifest_type(id),
            Some(crate::app_registry::ManifestType::Agent)
        ) {
            if let Some(process) = self.registry.launch_process(id, &cwd, args) {
                let system_prompt = self.registry.system_prompt_for(id);
                self.open_subprocess_agent_pane(id, process, system_prompt);
            } else {
                log::warn!(
                    "launch_app_by_id: agent '{id}' not found or failed to launch"
                );
            }
            return;
        }

        // Re-attach a parked background app if one is waiting
        if let Some(mut parked) = self.background_apps.remove(id) {
            log::info!("re-attaching background app '{id}'");
            parked.send_event(&crate::app_protocol::PlexiEvent::Resume);
            self.open_process_app_pane(id, *parked, cwd, group, hint.as_deref());
            return;
        }

        if let Some(process) = self.registry.launch_process(id, &cwd, args) {
            self.open_process_app_pane(id, process, cwd, group, hint.as_deref());
        } else {
            log::warn!("launch_app_by_id: app '{id}' not found or failed to launch");
        }
    }

    /// Open a `Pane::Agent` whose backend is the freshly launched subprocess
    /// (#338). Vertical 1:1 split alongside the focused pane — same default as
    /// `open_agent_pane` (Cmd+I path) for now; layout hints are deferred to
    /// v3.3.5+ so the agent UI lands in a predictable place every time.
    fn open_subprocess_agent_pane(
        &mut self,
        manifest_id: &str,
        mut process: crate::process_app::ProcessApp,
        system_prompt: Option<String>,
    ) {
        let active = self.active_context;
        let new_id = self.host.alloc_pane_id();
        process.set_pane_id(new_id);
        let pane = crate::agent_pane::AgentPane::new_subprocess(
            new_id,
            Box::new(process),
            system_prompt,
            manifest_id.to_string(),
        );
        self.contexts[active]
            .panes
            .insert(new_id, Pane::Agent(Box::new(pane)));
        let share = ShareRatio::new(1.0, 1.0).expect("1:1 is valid");
        let _ = self.split_with_new_pane(new_id, true, share, false);
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
        let share = ShareRatio::new(1.0, 1.0).expect("1:1 is valid");
        let _ = self.split_with_new_pane(new_id, true, share, false);
    }

    /// Open an Agent Workspace pane (#348): create a git worktree, spawn the
    /// CLI inside it, drop the pane into a vertical split alongside the
    /// focused pane.
    ///
    /// On `Err`, no pane is inserted — the worktree was either never created
    /// (non-git repo) or has been rolled back (PTY spawn failure). The error
    /// is logged; a higher layer (the modal in #349) is responsible for any
    /// user-facing surfacing.
    pub(crate) fn open_agent_workspace_pane(
        &mut self,
        cli: crate::agent_workspace::AgentCli,
        task_label: String,
    ) -> Result<(), crate::agent_workspace::AgentWorkspaceError> {
        let active = self.active_context;
        let cwd = self.contexts[active]
            .focused_pane
            .and_then(|fp| self.contexts[active].get_focused_pane_cwd(fp))
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));

        let repo_root = crate::agent_workspace::find_git_repo_root(&cwd).ok_or_else(|| {
            crate::agent_workspace::AgentWorkspaceError::NotAGitRepository(cwd.clone())
        })?;

        let new_id = self.host.alloc_pane_id();
        let env = crate::shell::build_env();
        let dynamic_colors = crate::theme::terminal_dynamic_colors(&self.colors);
        let pane = crate::agent_workspace::AgentWorkspacePane::create(
            new_id,
            cli,
            repo_root,
            task_label,
            self.ctx.clone(),
            self.pty_event_tx.clone(),
            env,
            dynamic_colors,
            self.default_font_size,
        )?;

        self.contexts[active]
            .panes
            .insert(new_id, Pane::AgentWorkspace(Box::new(pane)));
        let share = crate::host::command::ShareRatio::new(1.0, 1.0).expect("1:1 is valid");
        let _ = self.split_with_new_pane(new_id, true, share, false);
        Ok(())
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
