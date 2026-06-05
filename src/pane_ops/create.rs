//! Pane / app / agent creation and launch entry points.
//!
//! These methods own the "make a new pane appear" side of the pane API —
//! ProcessApp wiring, builtin app wiring, single-pane tree creation for new
//! contexts, and the launch helpers called from the command palette and
//! AppRequest routing. The actual tile-tree mutation is delegated to
//! [`PlexiApp::split_with_new_pane`] in [`super::layout`].

use crate::app::PlexiApp;
use crate::app::app_trait::App;
use crate::host::command::{HostAction, OpenPaneRequest, PaneRuntimeKind, Placement, ShareRatio};
use crate::host::effect::HostEffect;
use crate::host::pane::{Pane, TerminalPane};
use crate::spatial::tiling::PaneId;
use egui_term::BackendCommand;
use egui_tiles::{Tile, TileId, Tree};
use std::collections::HashMap;
use std::path::PathBuf;

/// Instantiate a builtin `App` by id, checked before the process registry.
///
/// Builtins are compiled-in and never require disk access. Add one line here
/// when introducing a new builtin app type; the dispatch path in
/// `launch_app_by_id_with_layout` requires no further changes.
///
/// `"terminal"` is intentionally absent — it is a PTY pane, not an `App`.
fn builtin_factory(id: &str, args: &[String]) -> Option<Box<dyn App>> {
    match id {
        "text-editor" => {
            let path = args
                .first()
                .map(|s| std::path::PathBuf::from(s))
                .unwrap_or_else(|| crate::config::config_dir().join("notes").join("scratch.md"));
            Some(Box::new(crate::app::text_editor_app::TextEditorApp::new(path)))
        }
        "cli-renderer" => {
            let path = args.first().cloned().unwrap_or_default();
            Some(Box::new(crate::render::cli_renderer_app::CliRendererApp::new(path)))
        }
        _ => None,
    }
}

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
        let ctx = &self.windows[active];
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
        let new_pane_first = matches!(hint, Some("split_above") | Some("split_left"));
        let vertical = matches!(hint, Some("split_h") | Some("split_right") | Some("split_left"));
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
        let effects = self.submit(HostAction::OpenPane(req));
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
    ) -> Option<PaneId> {
        let active = self.active_window;
        let new_app_pane = |id: PaneId,
                            process: crate::process_app::ProcessApp,
                            workspace_root: PathBuf,
                            group: Option<String>,
                            linked_pane_id: Option<PaneId>,
                            overlay_replaced: Option<Box<Pane>>| {
            Pane::App(Box::new(crate::host::pane::AppPane {
                id,
                permissions: process.permissions.clone(),
                runtime: crate::host::pane::AppRuntime::Process(Box::new(process)),
                workspace_root,
                manifest_id: app_id.to_string(),
                name: app_id.to_string(),
                pane_group: group,
                linked_pane_id,
                overlay_replaced,
                hidden: false,
            }))
        };

        if matches!(hint, Some("overlay")) {
            let Some(focused_tile) = self.windows[active].focused_pane else {
                log::warn!("app::{app_id}: overlay launch skipped — no focused pane");
                return None;
            };
            let Some(Tile::Pane(focused_pane_id)) =
                self.windows[active].tree.tiles.get(focused_tile)
            else {
                log::warn!("app::{app_id}: overlay launch skipped — focused tile is not a pane");
                return None;
            };
            let pane_id = *focused_pane_id;
            let Some(replaced_pane) = self.windows[active].panes.remove(&pane_id) else {
                log::warn!(
                    "app::{app_id}: overlay launch skipped — pane {pane_id} missing from pane map"
                );
                return None;
            };
            process.set_pane_id(pane_id);
            self.windows[active].panes.insert(
                pane_id,
                new_app_pane(pane_id, process, workspace_root, group, None, Some(Box::new(replaced_pane))),
            );
            self.set_window_focused_pane(active, focused_tile);
            log::info!("app::{app_id}: launched as overlay on pane {pane_id}");
            return Some(pane_id);
        }

        if self.windows[active].zoomed_pane.take().is_some() {
            log::info!("app::{app_id}: cleared zoom before launch");
        }

        // Record which terminal we're splitting from before focus moves.
        let linked_pane_id = self.focused_terminal_id(active);
        let share = Self::share_ratio_from_fraction(app_id, None);
        let (new_id, share, vertical, new_pane_first) =
            self.open_pane_layout(app_id, group.clone(), hint, share);
        process.set_pane_id(new_id);
        self.windows[active].panes.insert(
            new_id,
            new_app_pane(new_id, process, workspace_root, group, linked_pane_id, None),
        );

        // Hot reload (#83): if the manifest opted in AND the app was
        // discovered from a workspace-local install, begin watching its
        // directory for source-change events.
        if self.registry.watch_eligible(app_id) {
            if let Some(app_dir) = self.registry.app_dir_for(app_id) {
                self.hot_reload.watch(new_id, &app_dir);
            }
        }

        // Empty context: no focused pane means no existing tile to split.
        // Install the new pane directly as the tree root.
        if self.windows[active].focused_pane.is_none() {
            let ctx = &mut self.windows[active];
            let root_tile = ctx.tree.tiles.insert_pane(new_id);
            ctx.tree.root = Some(root_tile);
            ctx.focused_pane = Some(root_tile);
            log::info!("app::{app_id}: launched as root pane {new_id} (empty context)");
            return Some(new_id);
        }

        let _ = self.split_with_new_pane(new_id, vertical, share, new_pane_first, false);

        if let (Some(msg), Some(term_id)) =
            (self.registry.startup_message_for(app_id), linked_pane_id)
        {
            if let Some(term) = self.windows[active]
                .panes
                .get_mut(&term_id)
                .and_then(|p| p.as_terminal_mut())
            {
                let quoted = crate::host::shell::shell_quote(&msg);
                let cmd = format!("\x15printf '%s\\n' {quoted}\n");
                term.backend.process_command(BackendCommand::Write(cmd.into_bytes()));
                log::info!("app::{app_id}: startup message written to terminal pane {term_id}");
            }
        }

        Some(new_id)
    }

    /// Open a built-in error tile pane when a capability pre-flight check fails.
    /// Mirrors `open_process_app_pane` but uses `AppRuntime::Builtin` — no
    /// Python process is spawned.
    fn open_launch_failed_pane(
        &mut self,
        app_id: &str,
        hint: Option<&str>,
        missing: Vec<String>,
        workspace_root: std::path::PathBuf,
    ) {
        let active = self.active_window;
        let share = Self::share_ratio_from_fraction(app_id, None);
        let (new_id, share, vertical, new_pane_first) =
            self.open_pane_layout(app_id, None, hint, share);
        self.windows[active].panes.insert(
            new_id,
            crate::host::pane::Pane::App(Box::new(crate::host::pane::AppPane {
                id: new_id,
                runtime: crate::host::pane::AppRuntime::Builtin(
                    Box::new(crate::host::launch_failed::LaunchFailedApp {
                        app_id: app_id.to_string(),
                        missing,
                    }),
                ),
                workspace_root,
                permissions: crate::app::permissions::AppPermissions::default(),
                manifest_id: app_id.to_string(),
                name: format!("Cannot launch {app_id}"),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                hidden: false,
            })),
        );
        if self.windows[active].focused_pane.is_none() {
            let ctx = &mut self.windows[active];
            let root_tile = ctx.tree.tiles.insert_pane(new_id);
            ctx.tree.root = Some(root_tile);
            ctx.focused_pane = Some(root_tile);
            log::info!("app::{app_id}: launch-failed tile inserted as root pane {new_id}");
            return;
        }
        let _ = self.split_with_new_pane(new_id, vertical, share, new_pane_first, false);
        log::info!("app::{app_id}: launch-failed tile inserted as pane {new_id}");
    }

    /// Reload the `ProcessApp` inside the AppPane at `pane_id` (#83).
    ///
    /// Sends `Shutdown` to the existing subprocess (via `Drop` on the old
    /// `ProcessApp`), relaunches a fresh subprocess via `AppRegistry::launch_process`
    /// using the same `manifest_id` + `workspace_root`, and swaps the
    /// runtime field on the existing `AppPane`. The pane envelope (id,
    /// position, focus, linked terminal) is preserved — only the inner
    /// subprocess is replaced.
    ///
    /// State is not transferred — apps must accept that hot reload starts
    /// fresh. This is documented in the issue body as acceptable for dev.
    ///
    /// Returns true if the reload was attempted (pane found and was a
    /// process-backed AppPane). Returns false otherwise (pane gone, not an
    /// app pane, or builtin runtime).
    pub(crate) fn reload_app_pane(&mut self, pane_id: PaneId, reason: &str) -> bool {
        let active = self.active_window;
        let ctx = &self.windows[active];
        let Some(app_pane) = ctx.panes.get(&pane_id).and_then(|p| p.as_app()) else {
            log::debug!("reload_app_pane({pane_id}): not an app pane — ignoring");
            return false;
        };
        if !matches!(app_pane.runtime, crate::host::pane::AppRuntime::Process(_)) {
            log::debug!("reload_app_pane({pane_id}): builtin runtime — cannot reload");
            return false;
        }
        let manifest_id = app_pane.manifest_id.clone();
        let workspace_root = app_pane.workspace_root.clone();
        // Preserve launch args across hot-reload so the reloaded app gets the
        // same arguments the original was opened with.
        let saved_launch_args: Vec<String> = if let crate::host::pane::AppRuntime::Process(ref proc) = app_pane.runtime {
            proc.launch_args.clone()
        } else {
            Vec::new()
        };

        log::info!(
            "app::{manifest_id} reload triggered ({reason}) for pane {pane_id}"
        );

        // Launch the replacement first — if launch fails, leave the old
        // subprocess running so the pane stays usable.
        let cwd = workspace_root.clone();
        let new_process_opt = self.registry.launch_process(&manifest_id, &cwd, &saved_launch_args);
        // Path-launched apps (app run / app init) are never inserted into the
        // registry's in-memory map, so launch_process returns None. Fall back
        // to loading the manifest directly from workspace_root.
        let new_process_opt = if new_process_opt.is_none()
            && workspace_root.join("manifest.toml").exists()
        {
            match self.registry.load_app(&workspace_root) {
                Ok(installed) => {
                    let perms = installed.manifest.capabilities.to_permissions();
                    let caps = perms.capabilities.clone();
                    let keyboard_capture = installed.launch.keyboard_capture;
                    match crate::process_app::ProcessApp::launch(
                        installed.manifest.id.clone(),
                        installed.manifest.name.clone(),
                        &installed.bin_path,
                        &cwd,
                        &saved_launch_args,
                        workspace_root.clone(),
                        caps,
                        keyboard_capture,
                        installed.manifest.mcp.as_ref(),
                    ) {
                        Ok(mut process) => {
                            process.permissions.allowed_hosts = perms.allowed_hosts;
                            Some(process)
                        }
                        Err(e) => {
                            log::warn!(
                                "reload_app_pane({pane_id}): path-reload launch failed: {e}"
                            );
                            None
                        }
                    }
                }
                Err(e) => {
                    log::warn!("reload_app_pane({pane_id}): path-reload load_app failed: {e}");
                    None
                }
            }
        } else {
            new_process_opt
        };
        let Some(mut new_process) = new_process_opt else {
            log::warn!(
                "reload_app_pane({pane_id}): launch_process returned None — keeping old instance"
            );
            return false;
        };
        new_process.set_pane_id(pane_id);

        // Swap the runtime. The old `ProcessApp` drops at end-of-scope —
        // its `Drop` impl sends `Shutdown` and waits/kills the child.
        let ctx_mut = &mut self.windows[active];
        if let Some(pane) = ctx_mut.panes.get_mut(&pane_id) {
            if let Some(app_pane) = pane.as_app_mut() {
                // Transfer the last committed frame so the pane doesn't flicker
                // blank during hot-reload of a crashed app (#1298).
                if let crate::host::pane::AppRuntime::Process(ref old_proc) = app_pane.runtime {
                    new_process.transfer_frame_from(old_proc);
                }
                let new_perms = new_process.permissions.clone();
                let old_runtime = std::mem::replace(
                    &mut app_pane.runtime,
                    crate::host::pane::AppRuntime::Process(Box::new(new_process)),
                );
                app_pane.permissions = new_perms;
                drop(old_runtime); // explicit — fires Shutdown + reaps child
                return true;
            }
        }
        false
    }

    /// Drain pending `ReloadRequest`s from the watcher channel and reload
    /// the matching panes. Called once per frame from the host update loop.
    pub(crate) fn drain_hot_reload_requests(&mut self) {
        loop {
            match self.hot_reload_rx.try_recv() {
                Ok(req) => {
                    self.reload_app_pane(req.pane_id, "watcher");
                }
                Err(_) => break,
            }
        }
    }

    /// Detect crashes on watched panes and restart them after a brief delay
    /// so the developer can read the crash overlay. Only applies to panes
    /// that opted in via `[app] watch = true` — production installs are
    /// unaffected. Called once per frame alongside `drain_hot_reload_requests`.
    pub(crate) fn drain_crash_restarts(&mut self) {
        use crate::process_app::LifecycleState;
        use std::time::{Duration, Instant};

        const CRASH_RESTART_DELAY: Duration = Duration::from_secs(2);

        let active = self.active_window;

        // Schedule restarts for newly-crashed watched panes.
        for pane_id in self.hot_reload.watched_pane_ids() {
            if self.pending_crash_restarts.contains_key(&pane_id) {
                continue;
            }
            if let Some(pane) = self.windows[active].panes.get(&pane_id) {
                if let Some(app_pane) = pane.as_app() {
                    if let crate::host::pane::AppRuntime::Process(ref proc) = app_pane.runtime {
                        if proc.lifecycle.state() == LifecycleState::Crashed {
                            log::info!(
                                "app::{}: crash detected on watched pane {pane_id} — scheduling restart in {}ms",
                                app_pane.manifest_id,
                                CRASH_RESTART_DELAY.as_millis()
                            );
                            self.pending_crash_restarts
                                .insert(pane_id, Instant::now() + CRASH_RESTART_DELAY);
                        }
                    }
                }
            }
        }

        // Fire elapsed restarts.
        let now = Instant::now();
        let ready: Vec<_> = self
            .pending_crash_restarts
            .iter()
            .filter(|(_, t)| **t <= now)
            .map(|(id, _)| *id)
            .collect();
        for pane_id in ready {
            self.pending_crash_restarts.remove(&pane_id);
            self.reload_app_pane(pane_id, "crash-restart");
        }
    }

    /// Force-reload the focused app pane (manual trigger via Cmd+Option+R).
    /// No-op when the focused pane isn't a process-backed AppPane.
    pub(crate) fn force_reload_focused_app(&mut self) {
        let active = self.active_window;
        let Some(focused_tile) = self.windows[active].focused_pane else {
            return;
        };
        let Some(Tile::Pane(pane_id)) = self.windows[active].tree.tiles.get(focused_tile) else {
            return;
        };
        let pane_id = *pane_id;
        self.reload_app_pane(pane_id, "manual");
    }

    pub(crate) fn open_builtin_app_pane(
        &mut self,
        app: Box<dyn App>,
        permissions: crate::app::permissions::AppPermissions,
        workspace_root: PathBuf,
        group: Option<String>,
        hint: Option<&str>,
        share: Option<f32>,
    ) {
        let active = self.active_window;
        let app_type_id = app.type_id().to_string();
        let app_name = app.display_name();
        let new_app_pane = |id: PaneId,
                            app: Box<dyn App>,
                            workspace_root: PathBuf,
                            group: Option<String>,
                            linked_pane_id: Option<PaneId>,
                            overlay_replaced: Option<Box<Pane>>| {
            Pane::App(Box::new(crate::host::pane::AppPane {
                id,
                runtime: crate::host::pane::AppRuntime::Builtin(app),
                workspace_root,
                permissions,
                manifest_id: app_type_id.clone(),
                name: app_name.clone(),
                pane_group: group,
                linked_pane_id,
                overlay_replaced,
                hidden: false,
            }))
        };

        if matches!(hint, Some("overlay")) {
            let Some(focused_tile) = self.windows[active].focused_pane else {
                log::warn!("builtin::{app_name}: overlay launch skipped — no focused pane");
                return;
            };
            let Some(Tile::Pane(focused_pane_id)) =
                self.windows[active].tree.tiles.get(focused_tile)
            else {
                log::warn!("builtin::{app_name}: overlay launch skipped — focused tile is not a pane");
                return;
            };
            let pane_id = *focused_pane_id;
            let Some(replaced_pane) = self.windows[active].panes.remove(&pane_id) else {
                return;
            };
            self.windows[active].panes.insert(
                pane_id,
                new_app_pane(pane_id, app, workspace_root, group, None, Some(Box::new(replaced_pane))),
            );
            self.set_window_focused_pane(active, focused_tile);
            log::info!("builtin::{app_name}: launched as overlay on pane {pane_id}");
            return;
        }

        if self.windows[active].zoomed_pane.take().is_some() {
            log::info!("builtin::{app_name}: cleared zoom before launch");
        }

        // Record which terminal we're splitting from before focus moves.
        let linked_pane_id = self.focused_terminal_id(active);
        let share = Self::share_ratio_from_fraction(
            &app_type_id,
            share,
        );
        let (new_id, share, vertical, new_pane_first) =
            self.open_pane_layout(&app_type_id, group.clone(), hint, share);
        self.windows[active].panes.insert(
            new_id,
            new_app_pane(new_id, app, workspace_root, group, linked_pane_id, None),
        );

        // Empty context: no focused pane means no existing tile to split.
        // Install the new pane directly as the tree root.
        if self.windows[active].focused_pane.is_none() {
            let ctx = &mut self.windows[active];
            let root_tile = ctx.tree.tiles.insert_pane(new_id);
            ctx.tree.root = Some(root_tile);
            ctx.focused_pane = Some(root_tile);
            log::info!("builtin::{app_name}: launched as root pane {new_id} (empty context)");
            return;
        }

        let _ = self.split_with_new_pane(new_id, vertical, share, new_pane_first, false);
    }

    pub(super) fn create_single_pane_tree(
        &mut self,
        cwd: Option<PathBuf>,
        initial_cmd: Option<&str>,
        close_on_exit: bool,
    ) -> Option<(Tree<PaneId>, HashMap<PaneId, Pane>, TileId)> {
        let new_id = self.host.alloc_pane_id();

        let ctx_id = self.windows.get(self.active_window).map(|w| w.context_id).unwrap_or(0);
        let ctx_name = self.context_name_for(ctx_id);
        let ctx_desc = self.context_description_for(ctx_id);
        let ctx_root = self.context_root_for(ctx_id);
        let ctx_depth = self.context_depth_for(ctx_id);
        let mut settings = Self::make_backend_settings(new_id, cwd, &self.colors, ctx_id, &ctx_name, &ctx_desc, ctx_root.as_ref(), ctx_depth);
        if let Some(cmd) = initial_cmd {
            log::info!("create_single_pane_tree: initial_cmd={cmd:?} close_on_exit={close_on_exit}");
            super::apply_initial_cmd(&mut settings, cmd, close_on_exit);
        }
        let mut pane = TerminalPane::new(
            new_id,
            self.ctx.clone(),
            self.pty_event_tx.clone(),
            settings,
            self.default_font_size,
        )?;
        pane.ephemeral = close_on_exit;

        let mut panes = HashMap::new();
        panes.insert(new_id, Pane::Terminal(Box::new(pane)));

        let mut tiles = egui_tiles::Tiles::default();
        let root_tile = tiles.insert_pane(new_id);
        let tree = Tree::new("plexi", root_tile, tiles);

        Some((tree, panes, root_tile))
    }

    /// Spawn a terminal pane adjacent to `target_tile` in `win_idx`.
    /// Does NOT read or write `active_window` or `focused_pane` — all targeting is explicit.
    /// Returns the newly allocated PaneId.
    /// `keep_focus`: if true, `focused_pane` in the target window is NOT changed.
    pub(crate) fn spawn_terminal_pane_at(
        &mut self,
        win_idx: usize,
        target_tile: egui_tiles::TileId,
        vertical: bool,
        new_pane_first: bool,
        initial_cmd: Option<&str>,
        close_on_exit: bool,
        cwd_override: Option<std::path::PathBuf>,
        keep_focus: bool,
    ) -> crate::spatial::tiling::PaneId {
        let new_id = self.host.alloc_pane_id();
        let ctx_id = self.windows[win_idx].context_id;
        let ctx_name = self.context_name_for(ctx_id);
        let ctx_desc = self.context_description_for(ctx_id);
        let ctx_root = self.context_root_for(ctx_id);
        let ctx_depth = self.context_depth_for(ctx_id);
        let cwd = cwd_override.or_else(|| self.windows[win_idx].get_focused_pane_cwd(target_tile));
        log::info!(
            "spawn_terminal_pane_at: win_idx={win_idx} target_tile={target_tile:?} new_id={new_id} \
             vertical={vertical} keep_focus={keep_focus} initial_cmd={initial_cmd:?}"
        );
        let mut settings = Self::make_backend_settings(new_id, cwd, &self.colors, ctx_id, &ctx_name, &ctx_desc, ctx_root.as_ref(), ctx_depth);
        if let Some(cmd) = initial_cmd {
            super::apply_initial_cmd(&mut settings, cmd, close_on_exit);
        }
        let Some(mut pane) = TerminalPane::new(
            new_id,
            self.ctx.clone(),
            self.pty_event_tx.clone(),
            settings,
            self.default_font_size,
        ) else {
            log::error!("spawn_terminal_pane_at: TerminalPane::new failed for pane_id={new_id}");
            return new_id;
        };
        pane.ephemeral = close_on_exit;
        self.windows[win_idx].panes.insert(new_id, Pane::Terminal(Box::new(pane)));

        let share = crate::host::command::ShareRatio::new(1.0, 1.0)
            .expect("1:1 is a valid ShareRatio");
        let new_tile = super::layout::insert_split_tile(
            &mut self.windows[win_idx].tree,
            Some(target_tile),
            new_id,
            vertical,
            share,
            new_pane_first,
        );

        if !keep_focus {
            self.set_window_focused_pane(win_idx, new_tile);
        }
        new_id
    }

    /// Toggle the file browser: if the focused pane has a file browser open,
    /// close it. Otherwise, open one.
    pub(crate) fn open_file_browser(&mut self) {
        // Check if the focused pane (or its linked app pane above) already has
        // a file browser open. If so, close it.
        let ctx = &self.windows[self.active_window];
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
            let ctx = &self.windows[self.active_window];
            ctx.focused_pane
                .and_then(|tile_id| ctx.get_focused_pane_cwd(tile_id))
                .filter(|p| {
                    if p == &PathBuf::from("/") {
                        log::debug!("open_file_browser: CWD is /, falling back to home_dir (GUI launch)");
                        false
                    } else {
                        true
                    }
                })
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
        };

        let app: Box<dyn App> = self
            .registry
            .launch("file_browser", &cwd, &[])
            .unwrap_or_else(|| Box::new(crate::file_browser::FileBrowserApp::new(cwd.clone())));

        // Built-in file browser gets full permissions, joins the "cwd" group so
        // it follows linked-terminal directory changes.
        let perms = crate::app::permissions::AppPermissions::builtin();
        self.open_builtin_app_pane(
            app,
            perms,
            cwd,
            Some("cwd".to_string()),
            Some("overlay"),
            None,
        );
    }

    /// Launch an installed app by id in the focused pane.
    pub(crate) fn launch_app_by_id(&mut self, id: &str) {
        let _ = self.launch_app_by_id_with_layout(id, None, &[], None);
    }

    /// Launch an installed app with an explicit layout and args override.
    ///   "overlay" (default) — full pane takeover; Esc restores the original pane
    ///   "split_h"           — horizontal split, new pane to the right
    ///   "split_v"           — vertical split, new pane below
    pub(crate) fn launch_app_by_id_with_layout(
        &mut self,
        id: &str,
        layout: Option<String>,
        args: &[String],
        cwd_override: Option<PathBuf>,
    ) -> Result<(), String> {
        // "terminal" is a builtin pane type, not in the app registry.
        // Reached via SDK AppCommand::SpawnApp("terminal", ...) and legacy paths.
        // Socket IPC and spawn-queue handle terminal inline in app/mod.rs.
        if id == "terminal" {
            let layout_str = layout.as_deref().unwrap_or("split_h");
            let vertical = matches!(layout_str, "split_v" | "split_below" | "split_above");
            let new_pane_first = matches!(layout_str, "split_above" | "split_left");
            let initial_cmd = if args.is_empty() { None } else { Some(crate::host::shell::shell_join(args)) };
            log::info!(
                "SpawnPane: terminal layout='{layout_str}' vertical={vertical} new_pane_first={new_pane_first} initial_cmd={initial_cmd:?}"
            );
            self.split_focused(vertical, initial_cmd.as_deref(), false, new_pane_first, None);
            return Ok(());
        }

        let cwd_explicit = cwd_override.is_some();
        let cwd = cwd_override
            .or_else(|| self.resolve_new_pane_cwd(None, self.windows[self.active_window].focused_pane))
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
        log::info!("launch_app_by_id_with_layout: id={id} cwd={cwd:?} cwd_source={} context_root={:?}",
            if cwd_explicit { "explicit" } else { "resolved" },
            self.router.active().root);

        // Builtin app factory — resolved before the process registry; builtins never touch disk.
        if let Some(app) = builtin_factory(id, args) {
            log::info!("launch_app_by_id_with_layout: '{id}' resolved as builtin");
            let group = self.registry.group_for(id);
            let hint = layout.unwrap_or_else(|| "overlay".to_string());
            let perms = crate::app::permissions::AppPermissions::builtin();
            self.open_builtin_app_pane(app, perms, cwd, group, Some(&hint), None);
            return Ok(());
        }

        // Re-attach a parked background app if one is waiting
        if let Some((_park_context_id, mut parked)) = self.background_apps.remove(id) {
            log::info!("re-attaching background app '{id}'");
            parked.send_event(&crate::app_protocol::PlexiEvent::Resume);
            let group = self.registry.group_for(id);
            let hint = layout.unwrap_or_else(|| "overlay".to_string());
            self.open_process_app_pane(id, *parked, cwd, group, Some(&hint));
            return Ok(());
        }

        // Ensure the registry is up-to-date: rescan if the app was added mid-session
        // via `plexi app init` and wasn't present at startup.
        if self.registry.get(id).is_none() {
            log::info!("launch_app_by_id: '{id}' not in startup registry — rescanning from disk");
            self.registry = crate::app::registry::AppRegistry::load(&cwd);
        }

        // Pre-flight: check config-level capability requirements before spawning.
        let missing = self.registry.check_config_capabilities(id, &self.config);
        if !missing.is_empty() {
            log::warn!("pre-flight: '{id}' cannot launch — missing: {missing:?}");
            let fail_hint = layout.clone().or_else(|| Some("overlay".to_string()));
            self.open_launch_failed_pane(id, fail_hint.as_deref(), missing, cwd);
            return Ok(());
        }

        // Try registry first; if it returns None, fall through to Tier 4.
        let registry_process = self.registry.launch_process(id, &cwd, args);
        // Query group/hint after any registry reload so metadata reflects the
        // actual registry that found the app.
        let group = self.registry.group_for(id);
        let hint = layout.or_else(|| Some("overlay".to_string()));
        if let Some(process) = registry_process {
            if cli_binary_in_path(id) {
                log::warn!(
                    "launch_app_by_id: installed app '{id}' is shadowing a CLI of the same name \
                     — installed app takes precedence; uninstall the app to use the CLI's plexi_app"
                );
            }
            self.open_process_app_pane(id, process, cwd, group, hint.as_deref());
            return Ok(());
        }

        // Tier 4 — CLI native descriptor (plexi_app field).
        if let Some(process) = self.try_launch_cli_pgap_app(id, &cwd) {
            self.open_process_app_pane(&format!("cli:{id}"), process, cwd, group, hint.as_deref());
            Ok(())
        } else {
            log::warn!("launch_app_by_id: app '{id}' not found or failed to launch");
            Err(format!("app '{id}' not found"))
        }
    }

    /// Launch an app directly from a filesystem path without looking it up in the registry.
    ///
    /// Used by `plexi app run <path>` and the `SpawnPane` IPC when `path` is set.
    /// The app runs in-place; its own directory is used as `workspace_root`.
    pub(crate) fn launch_app_by_path_with_layout(
        &mut self,
        app_path: &str,
        layout: Option<String>,
        workspace_root_override: Option<std::path::PathBuf>,
    ) -> Result<(), String> {
        let app_dir = PathBuf::from(app_path);
        log::info!("launch_app_by_path_with_layout: path={app_path}");

        let installed = match self.registry.load_app(&app_dir) {
            Ok(a) => a,
            Err(e) => {
                log::warn!("launch_app_by_path_with_layout: failed to load manifest at {app_path}: {e}");
                return Err(format!("failed to load app at '{app_path}': {e}"));
            }
        };

        let perms = installed.manifest.capabilities.to_permissions();
        let caps = perms.capabilities.clone();
        let keyboard_capture = installed.launch.keyboard_capture;
        let app_id = installed.manifest.id.clone();

        let layout_hint = layout.or_else(|| Some("overlay".to_string()));

        let cwd = self
            .resolve_new_pane_cwd(None, self.windows[self.active_window].focused_pane)
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));

        let workspace_root = workspace_root_override.unwrap_or_else(|| app_dir.clone());
        log::info!(
            "launch_app_by_path_with_layout: workspace_root={}",
            workspace_root.display()
        );
        match crate::process_app::ProcessApp::launch(
            installed.manifest.id.clone(),
            installed.manifest.name.clone(),
            &installed.bin_path,
            &cwd,
            &[],
            workspace_root,
            caps,
            keyboard_capture,
            installed.manifest.mcp.as_ref(),
        ) {
            Ok(mut process) => {
                process.permissions.allowed_hosts = perms.allowed_hosts;
                let group = installed.launch.join_group.clone();
                let watch = installed.manifest.watch.unwrap_or(false);
                log::info!(
                    "launch_app_by_path_with_layout: launched '{app_id}' from {app_path} group={group:?}"
                );
                let watch_dir = app_dir.clone();
                let new_pane_id = self.open_process_app_pane(&app_id, process, app_dir, group, layout_hint.as_deref());
                if watch {
                    if let Some(pane_id) = new_pane_id {
                        log::info!(
                            "hot_reload: watching {} for pane {pane_id}",
                            watch_dir.display()
                        );
                        self.hot_reload.watch(pane_id, &watch_dir);
                    }
                }
                Ok(())
            }
            Err(e) => {
                log::error!("launch_app_by_path_with_layout: failed to launch '{app_id}' from {app_path}: {e}");
                Err(format!("failed to launch '{app_id}': {e}"))
            }
        }
    }

    /// Attempt to spawn a PGAP process from the CLI's native `--plexi` descriptor
    /// `plexi_app` field. Returns `None` if the CLI is not found, does not support
    /// `--plexi`, or has no `plexi_app` field.
    fn try_launch_cli_pgap_app(
        &self,
        cli_name: &str,
        cwd: &PathBuf,
    ) -> Option<crate::process_app::ProcessApp> {
        // Step 1: Run `<cli_name> --plexi` to get the native descriptor.
        let output = std::process::Command::new(cli_name)
            .arg("--plexi")
            .output()
            .ok()?;
        if !output.status.success() && output.stdout.is_empty() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let descriptor = crate::app::plexi_descriptor::parse(&stdout).ok()?;

        // None = no plexi_app field declared, skip Tier 4.
        let plexi_app_cmd = descriptor.plexi_app.as_deref()?;

        // Step 2: Split the command string into binary + args.
        let mut tokens = plexi_app_cmd.split_whitespace();
        let bin = tokens.next()?;
        let extra_args: Vec<String> = tokens.map(|s| s.to_string()).collect();

        // Step 3: Build permissions from descriptor.capabilities.
        let perms = crate::app::permissions::AppPermissions::from_capability_strings(
            &descriptor.capabilities,
        );
        let caps = perms.capabilities.clone();

        // Step 4: Spawn the ProcessApp.
        let app_id = format!("cli:{cli_name}");
        let bin_path = std::path::PathBuf::from(bin);
        match crate::process_app::ProcessApp::launch(
            app_id.clone(),
            descriptor.name.clone(),
            &bin_path,
            cwd,
            &extra_args,
            cwd.clone(),
            caps,
            false, // keyboard_capture
            None,  // mcp: CLI-spawned apps have no manifest [app.mcp]
        ) {
            Ok(app) => {
                log::info!(
                    "cli_pgap: spawned `{}` for CLI `{cli_name}` (plexi_app=`{plexi_app_cmd}`)",
                    app_id
                );
                Some(app)
            }
            Err(e) => {
                log::warn!(
                    "cli_pgap: failed to launch `{cli_name}` via plexi_app=`{plexi_app_cmd}`: {e}"
                );
                None
            }
        }
    }

    /// Open the secrets manager (read-only vault viewer, full pane, no terminal split).
    pub(crate) fn open_secrets_manager(&mut self) {
        // Toggle: if already open, close it.
        let ctx = &self.windows[self.active_window];
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
            let ctx = &self.windows[self.active_window];
            ctx.focused_pane
                .and_then(|tile_id| ctx.get_focused_pane_cwd(tile_id))
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
        };

        let app = Box::new(crate::app::secrets_app::SecretsApp::new(cwd.clone()));
        let perms = crate::app::permissions::AppPermissions::builtin();
        self.open_builtin_app_pane(app, perms, cwd, None, Some("overlay"), None);
    }

    /// Open a native text-editor pane for scratchpad editing.
    ///
    /// Launches the built-in `text-editor` app pane with the scratchpad file path.
    /// Works from any focused pane type (terminal, app, file browser).
    pub(crate) fn open_scratchpad(&mut self) {
        let path = scratchpad_file();
        let path_str = path.display().to_string();
        log::info!("scratchpad: opening text-editor pane for {:?}", path);
        if let Err(e) = self.launch_app_by_id_with_layout(
            "text-editor",
            Some("split_h".to_string()),
            &[path_str],
            None,
        ) {
            log::warn!("scratchpad: failed to launch text-editor pane: {e}");
        }
    }

    /// Open the quick note modal: capture context and push FocusLayer::QuickNote.
    pub(crate) fn open_quick_note_modal(&mut self) {
        let active = self.active_window;
        let cwd = self.windows[active]
            .focused_pane
            .and_then(|tile_id| self.windows[active].get_focused_pane_cwd(tile_id))
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
        let workspace_root = crate::config::active_workspace_root();
        let context_id = self.windows.get(active).map(|w| w.context_id).unwrap_or(0);
        let context = self.context_name_for(context_id);
        let context_root = self.router.active().root.clone();
        let context_description = self.router.active().description.clone();
        self.quick_note_text = String::new();
        self.quick_note_ctx = crate::app::QuickNoteCtx { cwd, workspace_root, context, context_root, context_description };
        self.quick_note_children_cache.clear();
        self.quick_note_children_rx = None;
        self.push_focus_layer(crate::app::FocusLayer::QuickNote);
        log::info!(
            "QuickNote: modal opened — cwd={}, workspace={:?}",
            self.quick_note_ctx.cwd.display(),
            self.quick_note_ctx.workspace_root
        );
    }

    /// Commit a quick note: write backlog or spawn a pane.
    /// Returns `false` only for backlog destinations that fail to write.
    pub(crate) fn commit_quick_note(
        &mut self,
        text: &str,
        node: &crate::config::QuickNoteNode,
    ) -> bool {
        let ctx = self.quick_note_ctx.clone();

        // Deprecation warning for old-style fields
        if node.dest_type.as_deref() == Some("pane") || node.position.is_some() {
            log::warn!(
                "QuickNote: '{}' uses deprecated 'type = \"pane\"' or 'position' — migrate to bare command",
                node.label
            );
        }

        if let Some(cmd_template) = &node.command {
            let cmd = Self::substitute_note_tokens_static(cmd_template, text, &ctx);
            // Visible panes default stay_alive=true so the response isn't lost on command exit.
            // Hidden background spawns don't need it (fire-and-forget).
            let stay_alive = node.stay_alive.unwrap_or(!node.hidden);

            if node.hidden {
                log::info!("QuickNote: committed via '{}' (hidden background spawn) cmd={cmd:?}", node.label);
                if let Err(e) = std::process::Command::new("sh")
                    .args(["-c", &cmd])
                    .spawn()
                {
                    log::warn!("QuickNote: hidden spawn failed for '{}': {e}", cmd);
                }
            } else {
                // Legacy position support during migration period
                let position = node.position.as_deref().unwrap_or("new_window");
                log::info!(
                    "QuickNote: committed via '{}' position={:?} stay_alive={stay_alive} cmd={cmd:?}",
                    node.label, position
                );
                match position {
                    "context-end" => self.open_at_context_end(&cmd, stay_alive),
                    "context-start" => self.open_at_context_start(&cmd, stay_alive),
                    "split" => self.split_focused(false, Some(&cmd), !stay_alive, false, None),
                    _ => {
                        // Default: open in a new window to the right and pull focus there.
                        let ws_id = self.router.active().context_id;
                        let active_y = self.windows[self.active_window].grid_y;
                        let new_x = self.windows.iter()
                            .filter(|w| w.context_id == ws_id && w.grid_y == active_y)
                            .map(|w| w.grid_x)
                            .max()
                            .map(|x| x + 1)
                            .unwrap_or(1);
                        log::info!(
                            "QuickNote: opening in new window grid=({new_x},{active_y}) stay_alive={stay_alive}"
                        );
                        self.create_page_at(new_x, active_y, Some(&cmd), !stay_alive);
                    }
                }
            }
            true
        } else if node.dest_type.as_deref() == Some("backlog") ||
                  (node.dest_type.is_none() && node.command.is_none() &&
                   node.options.is_none() && node.children_cmd.is_none()) {
            // Backlog destination (type = "backlog" or bare with just path)
            let path = node.path.as_deref().unwrap_or("");
            let dir = if path.is_empty() {
                crate::config::config_dir().join("backlog")
            } else if path.starts_with("~/") {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/"))
                    .join(&path[2..])
            } else {
                PathBuf::from(path)
            };
            Self::write_backlog_note(text, &dir, &ctx)
        } else {
            log::warn!("QuickNote: leaf '{}' has no command or recognized dest_type", node.label);
            false
        }
    }

    /// Commit the hard-coded destination 0: save to config_dir/backlog.
    /// Returns `false` if the write fails.
    pub(crate) fn commit_quick_note_global_backlog(&mut self, text: &str) -> bool {
        let ctx = self.quick_note_ctx.clone();
        let dir = crate::config::config_dir().join("backlog");
        log::info!("QuickNote: committed via global backlog (destination 0)");
        Self::write_backlog_note(text, &dir, &ctx)
    }

    fn write_backlog_note(text: &str, dir: &PathBuf, ctx: &crate::app::QuickNoteCtx) -> bool {
        use chrono::Local;
        let now = Local::now();
        let timestamp = now.format("%Y-%m-%d-%H%M%S").to_string();
        let display_time = now.format("%Y-%m-%d %H:%M:%S").to_string();
        let filename = format!("note-{timestamp}.md");

        let context_line = {
            let ws = ctx.workspace_root.as_ref()
                .and_then(|p| {
                    let home = dirs::home_dir()?;
                    p.strip_prefix(&home).ok().map(|rel| format!("~/{}", rel.display()))
                })
                .or_else(|| ctx.workspace_root.as_ref().map(|p| p.to_string_lossy().to_string()));
            let name_and_desc = match &ctx.context_description {
                Some(desc) if !desc.is_empty() => format!("{} — \"{}\"", ctx.context, desc.replace('\n', " ").replace('\r', "")),
                _ => ctx.context.clone(),
            };
            match ws {
                Some(w) => format!("{name_and_desc} · {w}"),
                None => name_and_desc,
            }
        };

        let content = format!(
            "# Quick Note — {display_time}\n**Context:** {context_line}\n---\n\n{}\n",
            text.trim()
        );

        if let Err(e) = std::fs::create_dir_all(dir) {
            log::error!("QuickNote: failed to create backlog dir {}: {e}", dir.display());
            return false;
        }
        let path = dir.join(&filename);
        match std::fs::write(&path, &content) {
            Ok(()) => { log::info!("QuickNote: saved to {}", path.display()); true }
            Err(e) => { log::error!("QuickNote: save failed {}: {e}", path.display()); false }
        }
    }

    pub(crate) fn substitute_note_tokens_static(
        cmd: &str,
        note: &str,
        ctx: &crate::app::QuickNoteCtx,
    ) -> String {
        let esc = |s: &str| -> String { crate::host::shell::shell_quote(s) };
        let cwd_str = ctx.cwd.to_string_lossy().to_string();
        let context_root_str = ctx.context_root
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        cmd.replace("{note}", &esc(note))
           .replace("{cwd}", &esc(&cwd_str))
           .replace("{context_root}", &esc(&context_root_str))
    }

    /// Spawn a terminal pane as the last child of the root container.
    pub(crate) fn open_at_context_end(&mut self, cmd: &str, stay_alive: bool) {
        let did_insert = self.try_insert_at_root(cmd, false, stay_alive);
        if !did_insert {
            self.split_focused(false, Some(cmd), !stay_alive, false, None);
        }
    }

    /// Spawn a terminal pane as the first child of the root container.
    pub(crate) fn open_at_context_start(&mut self, cmd: &str, stay_alive: bool) {
        let did_insert = self.try_insert_at_root(cmd, true, stay_alive);
        if !did_insert {
            self.split_focused(false, Some(cmd), !stay_alive, false, None);
        }
    }

    fn try_insert_at_root(&mut self, cmd: &str, prepend: bool, stay_alive: bool) -> bool {
        use egui_tiles::{Container, Tile};
        let new_id = self.host.alloc_pane_id();
        let active = self.active_window;
        let cwd = self.windows[active]
            .focused_pane
            .and_then(|t| self.windows[active].get_focused_pane_cwd(t));
        let ctx_id = self.windows.get(active).map(|w| w.context_id).unwrap_or(0);
        let ctx_name = self.context_name_for(ctx_id);
        let ctx_desc = self.context_description_for(ctx_id);
        let ctx_root = self.context_root_for(ctx_id);
        let ctx_depth = self.context_depth_for(ctx_id);
        let mut settings = Self::make_backend_settings(new_id, cwd, &self.colors, ctx_id, &ctx_name, &ctx_desc, ctx_root.as_ref(), ctx_depth);

        super::apply_initial_cmd(&mut settings, cmd, !stay_alive);

        let Some(mut pane) = crate::host::pane::TerminalPane::new(
            new_id,
            self.ctx.clone(),
            self.pty_event_tx.clone(),
            settings,
            self.default_font_size,
        ) else {
            return false;
        };
        pane.ephemeral = !stay_alive;
        self.windows[active].panes.insert(new_id, crate::host::pane::Pane::Terminal(Box::new(pane)));

        let ctx = &mut self.windows[active];
        let new_tile = ctx.tree.tiles.insert_pane(new_id);
        let root = match ctx.tree.root {
            Some(r) => r,
            None => {
                ctx.tree.root = Some(new_tile);
                ctx.focused_pane = Some(new_tile);
                return true;
            }
        };

        match ctx.tree.tiles.get_mut(root) {
            Some(Tile::Container(Container::Linear(lin))) => {
                if prepend {
                    lin.children.insert(0, new_tile);
                } else {
                    lin.children.push(new_tile);
                }
                ctx.focused_pane = Some(new_tile);
                true
            }
            _ => {
                let ordered = if prepend {
                    vec![new_tile, root]
                } else {
                    vec![root, new_tile]
                };
                let container = ctx.tree.tiles.insert_vertical_tile(ordered);
                ctx.tree.root = Some(container);
                ctx.focused_pane = Some(new_tile);
                true
            }
        }
    }
}


/// Returns `true` if a binary named `name` exists in any directory on `PATH`.
/// Used to detect when an installed Plexi app shadows a same-named CLI binary.
fn cli_binary_in_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::permissions::AppPermissions;
    use crate::process_app::ProcessApp;
    use crate::testing::HostHarness;

    /// Regression guard for issue #622: launching an app via `open_process_app_pane`
    /// into an empty context (welcome screen) silently did nothing — the process was
    /// inserted into `panes` but `split_with_new_pane` returned None because
    /// `focused_pane` was None. The pane leaked without a tile.
    #[test]
    fn app_launch_in_empty_context_creates_root_pane() {
        let mut h = HostHarness::new();
        assert!(h.state().open_panes.is_empty(), "harness should start empty");
        assert!(
            h.app.windows[0].focused_pane.is_none(),
            "no focused pane in empty context"
        );

        let (process, _tx) = ProcessApp::new_for_test(1, AppPermissions::builtin());
        h.app.open_process_app_pane(
            "test-app",
            process,
            std::path::PathBuf::from("/tmp"),
            None,
            None,
        );

        let snap = h.state();
        assert_eq!(
            snap.open_panes.len(),
            1,
            "app should appear as a pane after launching into empty context"
        );
        assert!(
            h.app.windows[0].focused_pane.is_some(),
            "new root pane should be focused"
        );
        assert!(
            h.app.windows[0].tree.root.is_some(),
            "tree root should be set after launch into empty context"
        );
    }

    /// Regression guard for issue #742: `plexi open terminal` (socket/spawn-queue paths) silently
    /// failed because `launch_app_by_id_with_layout("terminal", …)` fell through to the app
    /// registry, which logged a warn and did nothing. The fix adds an early-return that calls
    /// `split_focused` directly, matching the in-process `AppCommand::SpawnPane` path.
    #[test]
    fn spawn_pane_terminal_via_socket_path_creates_pane() {
        let mut h = HostHarness::new();
        let _pane = h.add_test_pane();
        // add_test_pane does not set focused_pane; wire it up so split_focused can run.
        let root = h.app.windows[0].tree.root.expect("root tile after add_test_pane");
        h.app.windows[0].focused_pane = Some(root);

        h.inject_ipc(crate::app_protocol::AppRequest::SpawnPane {
            type_id: "terminal".to_string(),
            layout: Some("split_v".to_string()),
            args: vec![],
            pipe_id: None,
            from_pane_id: None,
            request_id: None,
            response_file: None,
            ephemeral: false,
            cwd: None,
            no_focus: false,
            path: None,
            workspace_root: None,
            target_context: None,
            name: None,
        });
        h.run_frames(2);

        let snap = h.state();
        assert!(
            snap.open_panes.len() > 1,
            "SpawnPane terminal via socket must open a new pane (got {:?})",
            snap.pane_titles,
        );
        assert!(
            snap.pane_titles.values().any(|t| t == "Terminal"),
            "expected a Terminal pane in snapshot after SpawnPane, got: {:?}",
            snap.pane_titles,
        );
    }

    /// Regression guard for issue #890: terminal panes spawned with an initial_cmd
    /// must persist after the process exits (no auto-close). The pane must appear
    /// in pane list and accept pane send calls.
    #[test]
    fn spawn_pane_terminal_with_initial_cmd_creates_persistent_pane() {
        let mut h = HostHarness::new();
        let _pane = h.add_test_pane();
        let root = h.app.windows[0].tree.root.expect("root tile after add_test_pane");
        h.app.windows[0].focused_pane = Some(root);

        h.inject_ipc(crate::app_protocol::AppRequest::SpawnPane {
            type_id: "terminal".to_string(),
            layout: Some("split_v".to_string()),
            args: vec!["echo".to_string(), "hello".to_string()],
            pipe_id: None,
            from_pane_id: None,
            request_id: None,
            response_file: None,
            ephemeral: false,
            cwd: None,
            no_focus: false,
            path: None,
            workspace_root: None,
            target_context: None,
            name: None,
        });
        h.run_frames(2);

        // add_test_pane creates an App pane; after spawn we have 1 App + 1 Terminal.
        let snap = h.state();
        assert!(
            snap.open_panes.len() > 1,
            "SpawnPane with initial_cmd must create a new pane (got {:?})",
            snap.pane_titles,
        );
        assert!(
            snap.pane_titles.values().any(|t| t == "Terminal"),
            "expected a Terminal pane after SpawnPane with args, got: {:?}",
            snap.pane_titles,
        );
    }

    /// Regression guard for issue #992: SpawnPane with layout=None (omitted from IPC)
    /// must still open a terminal pane — the host must not treat None as a missing
    /// layout and silently skip the pane. Without the fix, the IPC handler would have
    /// called `matches!(layout.as_str(), ...)` on a String "overlay" (the old serde
    /// default), meaning apps always opened as overlay regardless of registry hint.
    #[test]
    fn spawn_pane_terminal_with_null_layout_still_opens_pane() {
        let mut h = HostHarness::new();
        let _pane = h.add_test_pane();
        let root = h.app.windows[0].tree.root.expect("root tile after add_test_pane");
        h.app.windows[0].focused_pane = Some(root);

        h.inject_ipc(crate::app_protocol::AppRequest::SpawnPane {
            type_id: "terminal".to_string(),
            layout: None,
            args: vec![],
            pipe_id: None,
            from_pane_id: None,
            request_id: None,
            response_file: None,
            ephemeral: false,
            cwd: None,
            no_focus: false,
            path: None,
            workspace_root: None,
            target_context: None,
            name: None,
        });
        h.run_frames(2);

        let snap = h.state();
        assert!(
            snap.open_panes.len() > 1,
            "SpawnPane terminal with null layout must open a new pane (got {:?})",
            snap.pane_titles,
        );
        assert!(
            snap.pane_titles.values().any(|t| t == "Terminal"),
            "expected a Terminal pane with null layout, got: {:?}",
            snap.pane_titles,
        );
    }

    /// `plexi terminal --layout tab` must create a new tab pane alongside the
    /// focused pane (wrapping both in a Tabs container) rather than splitting.
    #[test]
    fn spawn_pane_tab_creates_tab_not_split() {
        let mut h = HostHarness::new();
        let _pane = h.add_test_pane();
        let root = h.app.windows[0].tree.root.expect("root tile after add_test_pane");
        h.app.windows[0].focused_pane = Some(root);

        let before_panes = h.app.windows[0].panes.len();
        let before_windows = h.app.windows.len();

        h.inject_ipc(crate::app_protocol::AppRequest::SpawnPane {
            type_id: "terminal".to_string(),
            layout: Some("tab".to_string()),
            args: vec![],
            pipe_id: None,
            from_pane_id: None,
            request_id: None,
            response_file: None,
            ephemeral: false,
            cwd: None,
            no_focus: false,
            path: None,
            workspace_root: None,
            target_context: None,
            name: None,
        });
        h.run_frames(2);

        // No new window — tab stays in the same window.
        assert_eq!(
            h.app.windows.len(),
            before_windows,
            "tab layout must not create a new window"
        );
        // One new pane added to the window.
        assert_eq!(
            h.app.windows[0].panes.len(),
            before_panes + 1,
            "tab layout must add exactly one new pane"
        );
        // The tile tree root must now be a Tabs container.
        let root_tile = h.app.windows[0].tree.root.expect("root tile must exist");
        let root_tile_ref = h.app.windows[0].tree.tiles.get(root_tile);
        assert!(
            matches!(
                root_tile_ref,
                Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(_)))
            ),
            "root tile must be a Tabs container after tab spawn, got: {root_tile_ref:?}"
        );
    }

    /// `plexi terminal --layout new_window` must create a new spatial grid window
    /// in the current context rather than splitting the active pane.
    #[test]
    fn spawn_pane_new_window_creates_separate_window() {
        let mut h = HostHarness::new();
        let _pane = h.add_test_pane();
        let root = h.app.windows[0].tree.root.expect("root tile after add_test_pane");
        h.app.windows[0].focused_pane = Some(root);

        let before_windows = h.app.windows.len();

        h.inject_ipc(crate::app_protocol::AppRequest::SpawnPane {
            type_id: "terminal".to_string(),
            layout: Some("new_window".to_string()),
            args: vec![],
            pipe_id: None,
            from_pane_id: None,
            request_id: None,
            response_file: None,
            ephemeral: false,
            cwd: None,
            no_focus: false,
            path: None,
            workspace_root: None,
            target_context: None,
            name: None,
        });
        h.run_frames(2);

        assert_eq!(
            h.app.windows.len(),
            before_windows + 1,
            "new_window layout must create a new window (not a split)"
        );
        // Original window must still have the same pane count (no split occurred).
        assert_eq!(
            h.app.windows[0].panes.len(),
            1,
            "original window pane count must be unchanged after new_window spawn"
        );
        // New window must be placed to the right of the original at the same grid row.
        let new_win = h.app.windows.last().unwrap();
        assert_eq!(new_win.grid_x, 1, "new window must be at grid_x=1");
        assert_eq!(new_win.grid_y, 0, "new window must be at grid_y=0");
        // New window must contain exactly one Terminal pane.
        assert_eq!(new_win.panes.len(), 1, "new window must have one pane");
        assert!(
            new_win.panes.values().any(|p| matches!(p, crate::host::pane::Pane::Terminal(_))),
            "new window pane must be a Terminal"
        );
    }

    /// A second `new_window` spawn places the next window at grid_x=2, not grid_x=1.
    #[test]
    fn spawn_pane_new_window_stacks_right() {
        let mut h = HostHarness::new();
        let _pane = h.add_test_pane();
        let root = h.app.windows[0].tree.root.expect("root tile");
        h.app.windows[0].focused_pane = Some(root);

        // Spawn two new windows in sequence.
        for _ in 0..2 {
            h.inject_ipc(crate::app_protocol::AppRequest::SpawnPane {
                type_id: "terminal".to_string(),
                layout: Some("new_window".to_string()),
                args: vec![],
                pipe_id: None,
                from_pane_id: None,
                request_id: None,
                response_file: None,
                ephemeral: false,
                cwd: None,
                no_focus: false,
                path: None,
                workspace_root: None,
                target_context: None,
                name: None,
            });
            h.run_frames(2);
        }

        assert_eq!(h.app.windows.len(), 3, "two new_window spawns must create two windows");
        let xs: Vec<u32> = h.app.windows.iter().map(|w| w.grid_x).collect();
        assert!(xs.contains(&0), "original at grid_x=0");
        assert!(xs.contains(&1), "first new_window at grid_x=1");
        assert!(xs.contains(&2), "second new_window at grid_x=2");
    }

    #[test]
    fn cli_binary_in_path_finds_real_binary() {
        // `/bin/sh` is guaranteed to exist on macOS/Linux.
        assert!(
            cli_binary_in_path("sh"),
            "expected to find `sh` on PATH"
        );
    }

    #[test]
    fn cli_binary_in_path_misses_nonexistent() {
        assert!(
            !cli_binary_in_path("plexi-815-nonexistent-binary-zzz"),
            "expected not to find a nonexistent binary on PATH"
        );
    }

    /// Regression guard for issue #920: launching a non-overlay app while a pane is
    /// zoomed must clear the zoom so the new app is immediately visible.
    #[test]
    fn app_launch_clears_zoom() {
        let mut h = HostHarness::new();
        let _pane = h.add_test_pane();
        let root = h.app.windows[0].tree.root.expect("root tile after add_test_pane");
        h.app.windows[0].focused_pane = Some(root);
        h.app.windows[0].zoomed_pane = Some(root);

        let (process, _tx) = ProcessApp::new_for_test(1, AppPermissions::builtin());
        h.app.open_process_app_pane(
            "test-app",
            process,
            std::path::PathBuf::from("/tmp"),
            None,
            None,
        );

        assert!(
            h.app.windows[0].zoomed_pane.is_none(),
            "zoom must be cleared when launching a non-overlay app"
        );
    }

    /// Regression guard for issue #920: launching an overlay app while a pane is
    /// zoomed must NOT clear the zoom — overlay replaces in-place.
    #[test]
    fn overlay_app_launch_preserves_zoom() {
        let mut h = HostHarness::new();
        let _pane = h.add_test_pane();
        let root = h.app.windows[0].tree.root.expect("root tile after add_test_pane");
        h.app.windows[0].focused_pane = Some(root);
        h.app.windows[0].zoomed_pane = Some(root);

        let (process, _tx) = ProcessApp::new_for_test(2, AppPermissions::builtin());
        h.app.open_process_app_pane(
            "test-overlay-app",
            process,
            std::path::PathBuf::from("/tmp"),
            None,
            Some("overlay"),
        );

        assert!(
            h.app.windows[0].zoomed_pane.is_some(),
            "zoom must NOT be cleared when launching an overlay app"
        );
    }

    /// Regression guard for issue #1706: `open_process_app_pane` must return the
    /// created pane ID so callers can start a hot-reload watcher without going
    /// through the registry.
    #[test]
    fn open_process_app_pane_returns_pane_id() {
        let mut h = HostHarness::new();
        let _pane = h.add_test_pane();
        let root = h.app.windows[0].tree.root.expect("root tile");
        h.app.windows[0].focused_pane = Some(root);

        let (process, _tx) = ProcessApp::new_for_test(1, AppPermissions::builtin());
        let result = h.app.open_process_app_pane(
            "test-app",
            process,
            std::path::PathBuf::from("/tmp"),
            None,
            None,
        );
        assert!(result.is_some(), "open_process_app_pane must return Some(pane_id) on success");
    }

    /// Regression guard for issue #1706: overlay launch returns the pane ID of the
    /// replaced pane so the caller can attach a watcher.
    #[test]
    fn open_process_app_pane_overlay_returns_pane_id() {
        let mut h = HostHarness::new();
        let _pane = h.add_test_pane();
        let root = h.app.windows[0].tree.root.expect("root tile");
        h.app.windows[0].focused_pane = Some(root);

        let (process, _tx) = ProcessApp::new_for_test(2, AppPermissions::builtin());
        let result = h.app.open_process_app_pane(
            "test-overlay-app",
            process,
            std::path::PathBuf::from("/tmp"),
            None,
            Some("overlay"),
        );
        assert!(result.is_some(), "overlay launch must return Some(pane_id)");
    }

    /// Regression guard for issue #1706: overlay launch with no focused pane returns
    /// None (nothing was created) so callers do not try to start a watcher.
    #[test]
    fn open_process_app_pane_overlay_no_focused_pane_returns_none() {
        let mut h = HostHarness::new();
        assert!(h.app.windows[0].focused_pane.is_none(), "no focused pane in empty context");

        let (process, _tx) = ProcessApp::new_for_test(3, AppPermissions::builtin());
        let result = h.app.open_process_app_pane(
            "test-overlay-app",
            process,
            std::path::PathBuf::from("/tmp"),
            None,
            Some("overlay"),
        );
        assert!(result.is_none(), "overlay with no focused pane must return None");
    }
}

#[cfg(test)]
mod quick_note_tests {
    use super::*;
    use crate::app::QuickNoteCtx;

    fn ctx(cwd: &str) -> QuickNoteCtx {
        QuickNoteCtx {
            cwd: std::path::PathBuf::from(cwd),
            workspace_root: None,
            context: "test".to_string(),
            context_root: None,
            context_description: None,
        }
    }

    /// Run `sh -c <cmd>` and return stdout. Panics if the command exits non-zero.
    fn sh(cmd: &str) -> String {
        let out = std::process::Command::new("sh")
            .args(["-c", cmd])
            .output()
            .expect("sh -c failed");
        if !out.status.success() {
            panic!(
                "sh command failed (exit {}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
        }
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    #[test]
    fn substitute_tokens_escapes_shell_special_chars() {
        let result = PlexiApp::substitute_note_tokens_static(
            "gh issue create --title {note} --body ''",
            "it's a note \"with quotes\"",
            &ctx("/tmp/test dir"),
        );
        assert!(!result.contains("it's"), "unescaped single quote found: {result}");
        assert!(result.contains("it"), "note text missing from: {result}");
    }

    /// `$(...)` in a note must not be evaluated by the shell.
    #[test]
    fn no_command_substitution_dollar_paren() {
        let note = "$(printf INJECTED)";
        let cmd = PlexiApp::substitute_note_tokens_static("printf '%s' {note}", note, &ctx("/tmp"));
        let out = sh(&cmd);
        assert_eq!(out, note, "dollar-paren injection executed: got {out:?}");
    }

    /// Backtick command substitution in a note must not execute.
    #[test]
    fn no_command_substitution_backtick() {
        let note = "`printf INJECTED`";
        let cmd = PlexiApp::substitute_note_tokens_static("printf '%s' {note}", note, &ctx("/tmp"));
        let out = sh(&cmd);
        assert_eq!(out, note, "backtick injection executed: got {out:?}");
    }

    /// A note containing `'; <command>; echo '` must not break out of the substitution context.
    #[test]
    fn no_injection_via_single_quote_break() {
        let note = "'; printf INJECTED; printf '";
        let cmd = PlexiApp::substitute_note_tokens_static("printf '%s' {note}", note, &ctx("/tmp"));
        let out = sh(&cmd);
        assert_eq!(out, note, "single-quote break injection executed: got {out:?}");
    }

    /// Backslash in a note is passed through as a literal character.
    #[test]
    fn backslash_in_note_is_literal() {
        let note = r"a\b";
        let cmd = PlexiApp::substitute_note_tokens_static("printf '%s' {note}", note, &ctx("/tmp"));
        let out = sh(&cmd);
        assert_eq!(out, note, "backslash mangled: got {out:?}");
    }

    /// Newlines in a note are preserved literally (not treated as command separators).
    #[test]
    fn newline_in_note_is_literal() {
        let note = "first\nsecond";
        let cmd = PlexiApp::substitute_note_tokens_static("printf '%s' {note}", note, &ctx("/tmp"));
        let out = sh(&cmd);
        assert_eq!(out, note, "newline not preserved literally: got {out:?}");
    }

    /// `{cwd}` with a directory name containing a single quote expands safely.
    #[test]
    fn cwd_with_apostrophe_expands_safely() {
        let path = "/tmp/it's a dir";
        let cmd = PlexiApp::substitute_note_tokens_static("printf '%s' {cwd}", "note", &ctx(path));
        let out = sh(&cmd);
        assert_eq!(out, path, "cwd apostrophe mangled: got {out:?}");
    }

    /// A note consisting entirely of apostrophes must survive the round-trip.
    #[test]
    fn note_all_apostrophes_round_trips() {
        let note = "'''";
        let cmd = PlexiApp::substitute_note_tokens_static("printf '%s' {note}", note, &ctx("/tmp"));
        let out = sh(&cmd);
        assert_eq!(out, note, "all-apostrophe note mangled: got {out:?}");
    }

    #[test]
    fn context_root_token_is_shell_escaped() {
        let mut c = ctx("/tmp");
        c.context_root = Some(std::path::PathBuf::from("/projects/it's a dir"));
        let cmd = PlexiApp::substitute_note_tokens_static("printf '%s' {context_root}", "note", &c);
        let out = sh(&cmd);
        assert_eq!(out, "/projects/it's a dir", "context_root apostrophe mangled: got {out:?}");
    }

    #[test]
    fn context_root_empty_when_unset() {
        let c = ctx("/tmp");
        let result = PlexiApp::substitute_note_tokens_static("echo {context_root}", "note", &c);
        // When context_root is None, {context_root} substitutes to shell_quote("") = ''
        assert!(result.contains("''") || result.ends_with("echo "), "unexpected result: {result}");
    }

    // ── Agent context spawning tests (#1518) ─────────────────────────────────

    /// Verify that QueryContextState for the pane's own context produces
    /// the correct AppCommand via route_command.
    #[test]
    fn query_context_state_routes_to_pending_commands() {
        use crate::app_protocol::{AppRequest, DrawCommand};
        use crate::host::pane::{AppRuntime, Pane};
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        let pane = h.add_test_pane();

        h.inject(pane, DrawCommand::Host(
            AppRequest::QueryContextState { context_id: 1 },
        ));
        // Drain the draw channel via background_tick.
        {
            let win = &mut h.app.windows[0];
            let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
                panic!("expected App pane");
            };
            let AppRuntime::Process(ref mut proc) = app_pane.runtime else {
                panic!("expected Process runtime");
            };
            proc.background_tick();
        }

        // Verify pending_commands contains QueryContextState with the
        // correct sender_pane_id.
        let win = &mut h.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(ref mut proc) = app_pane.runtime else {
            panic!("expected Process runtime");
        };
        let has_cmd = proc.pending_commands.iter().any(|c| {
            matches!(c, crate::app::app_trait::AppCommand::QueryContextState {
                sender_pane_id, context_id
            } if *sender_pane_id == pane && *context_id == 1)
        });
        assert!(
            has_cmd,
            "QueryContextState should be in pending_commands after route_command"
        );
    }

    #[test]
    fn spawn_pane_target_context_invalid_returns_error() {
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        let pane = h.add_test_pane();
        let root = h.app.windows[0].tree.root.expect("root tile");
        h.app.windows[0].focused_pane = Some(root);

        // Spawn into a non-existent context via the PGAP draw channel
        // (inject, not inject_ipc) so it routes through ProcessApp ->
        // pending_commands -> deferred dispatch with target_context validation.
        h.inject(pane, crate::app_protocol::DrawCommand::Host(
            crate::app_protocol::AppRequest::SpawnPane {
                type_id: "terminal".to_string(),
                layout: Some("split_v".to_string()),
                args: vec![],
                pipe_id: None,
                from_pane_id: None,
                request_id: Some("req-ctx-bad".to_string()),
                response_file: None,
                ephemeral: false,
                cwd: None,
                no_focus: false,
                path: None,
                workspace_root: None,
                target_context: Some(999),
                name: None,
            },
        ));
        h.run_frames(1);

        // The pane count should NOT have increased (spawn was rejected).
        let snap = h.state();
        assert_eq!(
            snap.open_panes.len(), 1,
            "SpawnPane with invalid target_context must not create a pane"
        );
    }

    /// Regression guard for issue #1705: IPC terminal spawn with `from_pane_id` must split
    /// from the originating pane and keep focus there, not from the UI-focused pane.
    #[test]
    fn spawn_pane_terminal_ipc_from_pane_id_splits_from_origin() {
        use crate::testing::HostHarness;
        let mut h = HostHarness::new();
        let pane_a = h.add_test_pane();
        let root_a = h.app.windows[0].tree.root.expect("root tile after add_test_pane");
        h.app.windows[0].focused_pane = Some(root_a);

        // Add pane B: split from A so focus moves to B.
        h.inject_ipc(crate::app_protocol::AppRequest::SpawnPane {
            type_id: "terminal".to_string(),
            layout: Some("split_h".to_string()),
            args: vec![],
            pipe_id: None,
            from_pane_id: None,
            request_id: None,
            response_file: None,
            ephemeral: false,
            cwd: None,
            no_focus: false,
            path: None,
            workspace_root: None,
            target_context: None,
            name: None,
        });
        h.run_frames(2);

        let pane_b_tile = h.app.windows[0].focused_pane.expect("focused pane B after split");

        // Spawn pane C via IPC with from_pane_id=pane_a while UI focus is on pane B.
        h.inject_ipc(crate::app_protocol::AppRequest::SpawnPane {
            type_id: "terminal".to_string(),
            layout: Some("split_v".to_string()),
            args: vec![],
            pipe_id: None,
            from_pane_id: Some(pane_a),
            request_id: None,
            response_file: None,
            ephemeral: false,
            cwd: None,
            no_focus: false,
            path: None,
            workspace_root: None,
            target_context: None,
            name: None,
        });
        h.run_frames(2);

        let snap = h.state();
        assert!(
            snap.open_panes.len() >= 3,
            "from_pane_id terminal spawn via IPC must create a third pane (got {:?})",
            snap.open_panes,
        );
        // keep_focus=true when from_pane_id is set: focus must not jump to the new pane.
        assert_ne!(
            h.app.windows[0].focused_pane,
            None,
            "focused_pane must not be None after from_pane_id spawn",
        );
        assert_eq!(
            h.app.windows[0].focused_pane,
            Some(pane_b_tile),
            "focus must remain on the pre-existing UI-focused pane (pane B) after IPC terminal from_pane_id spawn",
        );
    }
}

/// Static scratchpad file path: `<config_dir>/notes/scratch.md`.
fn scratchpad_file() -> PathBuf {
    crate::config::config_dir().join("notes").join("scratch.md")
}


