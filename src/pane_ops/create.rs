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
        let active = self.active_window;
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
            let Some(focused_tile) = self.windows[active].focused_pane else {
                log::warn!("app::{app_id}: overlay launch skipped — no focused pane");
                return;
            };
            let Some(Tile::Pane(focused_pane_id)) =
                self.windows[active].tree.tiles.get(focused_tile)
            else {
                log::warn!("app::{app_id}: overlay launch skipped — focused tile is not a pane");
                return;
            };
            let pane_id = *focused_pane_id;
            let Some(replaced_pane) = self.windows[active].panes.remove(&pane_id) else {
                return;
            };
            process.set_pane_id(pane_id);
            self.windows[active].panes.insert(
                pane_id,
                new_app_pane(pane_id, process, workspace_root, group, None, Some(Box::new(replaced_pane))),
            );
            self.windows[active].focused_pane = Some(focused_tile);
            log::info!("app::{app_id}: launched as overlay on pane {pane_id}");
            return;
        }

        if self.windows[active].zoomed_pane.take().is_some() {
            log::info!("app::{app_id}: cleared zoom before launch");
        }

        // Record which terminal we're splitting from before focus moves.
        let linked_pane_id = self.focused_terminal_id(active);
        let share = Self::share_ratio_from_fraction(app_id, self.registry.share_for(app_id));
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
            return;
        }

        let _ = self.split_with_new_pane(new_id, vertical, share, new_pane_first);
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
        if !matches!(app_pane.runtime, crate::pane::AppRuntime::Process(_)) {
            log::debug!("reload_app_pane({pane_id}): builtin runtime — cannot reload");
            return false;
        }
        let manifest_id = app_pane.manifest_id.clone();
        let workspace_root = app_pane.workspace_root.clone();

        log::info!(
            "app::{manifest_id} reload triggered ({reason}) for pane {pane_id}"
        );

        // Launch the replacement first — if launch fails, leave the old
        // subprocess running so the pane stays usable.
        let cwd = workspace_root.clone();
        let Some(mut new_process) = self.registry.launch_process(&manifest_id, &cwd, &[]) else {
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
                let new_perms = new_process.permissions.clone();
                let old_runtime = std::mem::replace(
                    &mut app_pane.runtime,
                    crate::pane::AppRuntime::Process(Box::new(new_process)),
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
                    if let crate::pane::AppRuntime::Process(ref proc) = app_pane.runtime {
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
        permissions: crate::app_permissions::AppPermissions,
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
            self.windows[active].focused_pane = Some(focused_tile);
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
            share.or_else(|| self.registry.share_for(&app_type_id)),
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

        let _ = self.split_with_new_pane(new_id, vertical, share, new_pane_first);
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
        let mut settings = Self::make_backend_settings(new_id, cwd, &self.colors, ctx_id, &ctx_name);
        if let Some(cmd) = initial_cmd {
            log::info!("create_single_pane_tree: initial_cmd={cmd:?} close_on_exit={close_on_exit}");
            let shell_name = std::path::Path::new(&settings.shell)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            let effective_cmd: String = if !close_on_exit {
                let shell_path = &settings.shell;
                let trimmed = cmd.trim().trim_end_matches([';', ' ']);
                let sep = if trimmed.is_empty() { "" } else { "; " };
                match shell_name {
                    "fish" => format!("{trimmed}{sep}exec \"{shell_path}\" --login -i"),
                    _ => format!("{trimmed}{sep}exec \"{shell_path}\" -i -l"),
                }
            } else {
                cmd.to_string()
            };
            settings.args = match shell_name {
                "zsh" | "bash" => vec!["-i".to_string(), "-l".to_string(), "-c".to_string(), effective_cmd],
                "fish" => vec!["--login".to_string(), "-c".to_string(), effective_cmd],
                _ => vec!["-l".to_string(), "-c".to_string(), effective_cmd],
            };
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
            Some("overlay"),
            None,
        );
    }

    /// Launch an installed app by id in the focused pane.
    /// Respects the `layout_hint` from the app's manifest.toml.
    pub(crate) fn launch_app_by_id(&mut self, id: &str) {
        self.launch_app_by_id_with_layout(id, None, &[]);
    }

    /// Launch an installed app with an explicit layout and args override.
    /// `layout` overrides the manifest's `layout_hint` when `Some`.
    ///   "overlay" (default) — full pane takeover; Esc restores the original pane
    ///   "split_v"           — vertical split, new pane below
    ///   "split_h"           — horizontal split, new pane to the right
    pub(crate) fn launch_app_by_id_with_layout(
        &mut self,
        id: &str,
        layout: Option<String>,
        args: &[String],
    ) {
        // "terminal" is a builtin pane type, not in the app registry.
        // Reached via SDK AppCommand::SpawnApp("terminal", ...) and legacy paths.
        // Socket IPC and spawn-queue handle terminal inline in app/mod.rs.
        if id == "terminal" {
            let layout_str = layout.as_deref().unwrap_or("split_v");
            let vertical = matches!(layout_str, "split_h" | "split_above");
            let initial_cmd = if args.is_empty() { None } else { Some(crate::shell::shell_join(args)) };
            log::info!(
                "SpawnPane: terminal layout='{layout_str}' vertical={vertical} initial_cmd={initial_cmd:?}"
            );
            self.split_focused(vertical, initial_cmd.as_deref(), false);
            return;
        }

        let cwd = self.windows[self.active_window]
            .focused_pane
            .and_then(|fp| self.windows[self.active_window].get_focused_pane_cwd(fp))
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));

        // Re-attach a parked background app if one is waiting
        if let Some((_park_context_id, mut parked)) = self.background_apps.remove(id) {
            log::info!("re-attaching background app '{id}'");
            parked.send_event(&crate::app_protocol::PlexiEvent::Resume);
            let group = self.registry.group_for(id);
            let hint = layout
                .or_else(|| self.registry.layout_hint_for(id))
                .or_else(|| {
                    log::info!("app::{id}: no layout_hint — defaulting to overlay");
                    Some("overlay".to_string())
                });
            self.open_process_app_pane(id, *parked, cwd, group, hint.as_deref());
            return;
        }

        // Try registry first; if it returns None, rescan from disk (supports
        // apps created mid-session via `plexi app init`) then fall through to Tier 4.
        let registry_process = self.registry.launch_process(id, &cwd, args);
        let registry_process = if registry_process.is_none() {
            log::info!("launch_app_by_id: '{id}' not in startup registry — rescanning from disk");
            self.registry = crate::app_registry::AppRegistry::load(&cwd);
            self.registry.launch_process(id, &cwd, args)
        } else {
            registry_process
        };
        // Query group/hint after any registry reload so metadata reflects the
        // actual registry that found the app.
        let group = self.registry.group_for(id);
        let hint = layout
            .or_else(|| self.registry.layout_hint_for(id))
            .or_else(|| {
                log::info!("app::{id}: no layout_hint — defaulting to overlay");
                Some("overlay".to_string())
            });
        if let Some(process) = registry_process {
            if cli_binary_in_path(id) {
                log::warn!(
                    "launch_app_by_id: installed app '{id}' is shadowing a CLI of the same name \
                     — installed app takes precedence; uninstall the app to use the CLI's plexi_app"
                );
            }
            self.open_process_app_pane(id, process, cwd, group, hint.as_deref());
            return;
        }

        // Tier 4 — CLI native descriptor (plexi_app field).
        if let Some(process) = self.try_launch_cli_pgap_app(id, &cwd) {
            self.open_process_app_pane(&format!("cli:{id}"), process, cwd, group, hint.as_deref());
        } else {
            log::warn!("launch_app_by_id: app '{id}' not found or failed to launch");
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
        let descriptor = crate::plexi_descriptor::parse(&stdout).ok()?;

        // None = no plexi_app field declared, skip Tier 4.
        let plexi_app_cmd = descriptor.plexi_app.as_deref()?;

        // Step 2: Split the command string into binary + args.
        let mut tokens = plexi_app_cmd.split_whitespace();
        let bin = tokens.next()?;
        let extra_args: Vec<String> = tokens.map(|s| s.to_string()).collect();

        // Step 3: Build permissions from descriptor.capabilities.
        let perms = crate::app_permissions::AppPermissions::from_capability_strings(
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

        let app = Box::new(crate::secrets_app::SecretsApp::new(cwd.clone()));
        let perms = crate::app_permissions::AppPermissions::builtin();
        self.open_builtin_app_pane(app, perms, cwd, None, Some("overlay"), None);
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
        self.quick_note_text = String::new();
        self.quick_note_ctx = crate::app::QuickNoteCtx { cwd, workspace_root, context };
        self.quick_note_pending_parent = None;
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
        dest: &crate::config::QuickNoteDestinationConfig,
    ) -> bool {
        let ctx = self.quick_note_ctx.clone();
        match dest.dest_type.as_deref() {
            Some("backlog") | None if dest.options.is_none() => {
                let path = dest.path.as_deref().unwrap_or("");
                let dir = if path.is_empty() {
                    crate::config::config_dir().join("backlog")
                } else {
                    let expanded = if path.starts_with("~/") {
                        dirs::home_dir()
                            .unwrap_or_else(|| PathBuf::from("/"))
                            .join(&path[2..])
                    } else {
                        PathBuf::from(path)
                    };
                    expanded
                };
                Self::write_backlog_note(text, &dir, &ctx)
            }
            Some("pane") => {
                let cmd_template = match &dest.command {
                    Some(c) => c.clone(),
                    None => {
                        log::warn!("QuickNote: pane destination '{}' has no command", dest.label);
                        return false;
                    }
                };
                let cmd = Self::substitute_note_tokens_static(&cmd_template, text, &ctx);
                let position = dest.position.as_deref().unwrap_or("split");
                log::info!(
                    "QuickNote: committed via '{}' position={:?}",
                    dest.label,
                    position
                );
                match position {
                    "context-end" => self.open_at_context_end(&cmd),
                    "context-start" => self.open_at_context_start(&cmd),
                    _ => self.split_focused(false, Some(&cmd), true),
                }
                true
            }
            _ => {
                log::warn!("QuickNote: unknown dest_type for '{}'", dest.label);
                false
            }
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
            match ws {
                Some(w) => format!("{} · {}", ctx.context, w),
                None => ctx.context.clone(),
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
        let esc = |s: &str| -> String { crate::shell::shell_quote(s) };
        let cwd_str = ctx.cwd.to_string_lossy().to_string();
        cmd.replace("{note}", &esc(note))
           .replace("{cwd}", &esc(&cwd_str))
    }

    /// Spawn a terminal pane as the last child of the root container.
    pub(crate) fn open_at_context_end(&mut self, cmd: &str) {
        let did_insert = self.try_insert_at_root(cmd, false);
        if !did_insert {
            self.split_focused(false, Some(cmd), true);
        }
    }

    /// Spawn a terminal pane as the first child of the root container.
    pub(crate) fn open_at_context_start(&mut self, cmd: &str) {
        let did_insert = self.try_insert_at_root(cmd, true);
        if !did_insert {
            self.split_focused(false, Some(cmd), true);
        }
    }

    fn try_insert_at_root(&mut self, cmd: &str, prepend: bool) -> bool {
        use egui_tiles::{Container, Tile};
        let new_id = self.host.alloc_pane_id();
        let active = self.active_window;
        let cwd = self.windows[active]
            .focused_pane
            .and_then(|t| self.windows[active].get_focused_pane_cwd(t));
        let ctx_id = self.windows.get(active).map(|w| w.context_id).unwrap_or(0);
        let ctx_name = self.context_name_for(ctx_id);
        let mut settings = Self::make_backend_settings(new_id, cwd, &self.colors, ctx_id, &ctx_name);

        let shell_name = std::path::Path::new(&settings.shell)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let trimmed = cmd.trim().trim_end_matches([';', ' ']);
        settings.args = match shell_name {
            "zsh" | "bash" => vec!["-i".to_string(), "-l".to_string(), "-c".to_string(), trimmed.to_string()],
            "fish" => vec!["--login".to_string(), "-c".to_string(), trimmed.to_string()],
            _ => vec!["-l".to_string(), "-c".to_string(), trimmed.to_string()],
        };

        let Some(mut pane) = crate::pane::TerminalPane::new(
            new_id,
            self.ctx.clone(),
            self.pty_event_tx.clone(),
            settings,
            self.default_font_size,
        ) else {
            return false;
        };
        pane.ephemeral = true;
        self.windows[active].panes.insert(new_id, crate::pane::Pane::Terminal(Box::new(pane)));

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
    use crate::app_permissions::AppPermissions;
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

        h.inject_ipc(crate::app_protocol::HostCommand::SpawnPane {
            type_id: "terminal".to_string(),
            layout: Some("split_v".to_string()),
            args: vec![],
            pipe_id: None,
            from_pane_id: None,
            request_id: None,
            response_file: None,
            ephemeral: false,
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

        h.inject_ipc(crate::app_protocol::HostCommand::SpawnPane {
            type_id: "terminal".to_string(),
            layout: Some("split_v".to_string()),
            args: vec!["echo".to_string(), "hello".to_string()],
            pipe_id: None,
            from_pane_id: None,
            request_id: None,
            response_file: None,
            ephemeral: false,
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

        h.inject_ipc(crate::app_protocol::HostCommand::SpawnPane {
            type_id: "terminal".to_string(),
            layout: None,
            args: vec![],
            pipe_id: None,
            from_pane_id: None,
            request_id: None,
            response_file: None,
            ephemeral: false,
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

        h.inject_ipc(crate::app_protocol::HostCommand::SpawnPane {
            type_id: "terminal".to_string(),
            layout: Some("tab".to_string()),
            args: vec![],
            pipe_id: None,
            from_pane_id: None,
            request_id: None,
            response_file: None,
            ephemeral: false,
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

        h.inject_ipc(crate::app_protocol::HostCommand::SpawnPane {
            type_id: "terminal".to_string(),
            layout: Some("new_window".to_string()),
            args: vec![],
            pipe_id: None,
            from_pane_id: None,
            request_id: None,
            response_file: None,
            ephemeral: false,
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
            new_win.panes.values().any(|p| matches!(p, crate::pane::Pane::Terminal(_))),
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
            h.inject_ipc(crate::app_protocol::HostCommand::SpawnPane {
                type_id: "terminal".to_string(),
                layout: Some("new_window".to_string()),
                args: vec![],
                pipe_id: None,
                from_pane_id: None,
                request_id: None,
                response_file: None,
                ephemeral: false,
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
}
