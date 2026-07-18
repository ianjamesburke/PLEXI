//! Pane / app / agent creation and launch entry points.
//!
//! These methods own the "make a new pane appear" side of the pane API —
//! WASM app runtime wiring, builtin app wiring, single-pane tree creation for new
//! contexts, and the launch helpers called from the command palette and
//! AppRequest routing. The actual tile-tree mutation is delegated to
//! [`PlexiApp::split_with_new_pane`] in [`super::layout`].

use crate::app::app_trait::App;
use crate::app::PlexiApp;
use crate::host::command::{HostAction, OpenPaneRequest, PaneRuntimeKind, Placement, ShareRatio};
use crate::host::effect::HostEffect;
use crate::host::pane::{Pane, TerminalPane};
use crate::spatial::tiling::PaneId;
use egui_tiles::{Tile, TileId, Tree};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Instantiate a builtin `App` by id, checked before the process registry.
///
/// Builtins are compiled-in and never require disk access. Add one line here
/// when introducing a new builtin app type; the dispatch path in
/// `launch_app_by_id_with_layout` requires no further changes.
///
/// `"terminal"` is intentionally absent — it is a PTY pane, not an `App`.
pub(crate) fn builtin_factory(id: &str, cwd: &Path, args: &[String]) -> Option<Box<dyn App>> {
    match id {
        "image-viewer" => {
            let path = args.first().map(PathBuf::from)?;
            Some(Box::new(crate::app::image_viewer_app::ImageViewerApp::new(
                path,
            )))
        }
        "video-player" => {
            let path = args.first().map(PathBuf::from)?;
            Some(Box::new(crate::app::video_player_app::VideoPlayerApp::new(
                path,
            )))
        }
        "audio-player" => {
            let path = args.first().map(PathBuf::from)?;
            Some(Box::new(crate::app::audio_player_app::AudioPlayerApp::new(
                path,
            )))
        }
        "text-editor" => {
            // No explicit path → a fresh scratch note in the inbox. The file is
            // only created once content is typed (empty notes are deleted).
            let path = args
                .first()
                .map(|s| std::path::PathBuf::from(s))
                .unwrap_or_else(new_inbox_note_path);
            Some(Box::new(crate::app::text_editor_app::TextEditorApp::new(
                path,
            )))
        }
        "cli-renderer" => {
            let path = args.first().cloned().unwrap_or_default();
            Some(Box::new(
                crate::render::cli_renderer_app::CliRendererApp::new(path),
            ))
        }
        "file_browser" => Some(Box::new(crate::file_browser::FileBrowserApp::new(
            cwd.to_path_buf(),
        ))),
        "secrets_manager" => Some(Box::new(crate::app::secrets_app::SecretsApp::new(
            cwd.to_path_buf(),
        ))),
        _ => None,
    }
}

fn builtin_default_group(type_id: &str) -> Option<String> {
    match type_id {
        "file_browser" => Some("cwd".to_string()),
        _ => None,
    }
}

fn builtin_restore_args(
    app_id: &str,
    app_state: Option<&serde_json::Value>,
) -> Option<Vec<String>> {
    match app_id {
        "text-editor" | "image-viewer" | "video-player" | "audio-player" => app_state
            .and_then(|state| state.get("path"))
            .and_then(|path| path.as_str())
            .map(|path| vec![path.to_string()]),
        "file_browser" | "secrets_manager" => Some(vec![]),
        _ => None,
    }
}

pub(crate) fn restore_builtin_app_pane(
    app_id: &str,
    pane_id: PaneId,
    cwd: PathBuf,
    app_state: Option<&serde_json::Value>,
) -> Option<Pane> {
    let args = builtin_restore_args(app_id, app_state)?;
    let mut app = builtin_factory(app_id, &cwd, &args)?;
    if let Some(state) = app_state {
        app.restore_state(state);
    }
    let runtime_id = app.type_id().to_string();
    let name = app.display_name();
    log::info!(
        "workspace_restore: restored builtin app_id={app_id} runtime_id={runtime_id} pane_id={pane_id} cwd={}",
        cwd.display()
    );
    Some(Pane::App(Box::new(crate::host::pane::AppPane {
        pip_status: None,
        id: pane_id,
        runtime: crate::host::pane::AppRuntime::Builtin(app),
        workspace_root: cwd,
        permissions: crate::app::permissions::AppPermissions::builtin(),
        manifest_id: runtime_id.clone(),
        name,
        pane_group: builtin_default_group(&runtime_id),
        linked_pane_id: None,
        overlay_replaced: None,
        hidden: false,
        agent: None,
        slots: std::collections::HashMap::new(),
        semantic_state: Default::default(),
    })))
}

/// Rebuild the Assistant pane during workspace restore so a reopened app comes
/// back to the same conversation, focused. The Assistant is a builtin that
/// needs a broker and profile dir, so the broker-free [`builtin_factory`]
/// cannot construct it — the caller supplies the broker (built from
/// `config.ai`). `AssistantApp::new` auto-resumes the active conversation from
/// disk, so no `app_state` is required.
pub(crate) fn restore_assistant_pane(
    pane_id: PaneId,
    workspace_root: PathBuf,
    broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker>,
    profile_dir: &std::path::Path,
) -> Pane {
    let app = Box::new(crate::assistant::AssistantApp::new(
        workspace_root.clone(),
        broker,
        profile_dir,
    ));
    let runtime_id = app.type_id().to_string();
    let name = app.display_name();
    log::info!(
        "workspace_restore: restored assistant pane {pane_id} cwd={}",
        workspace_root.display()
    );
    Pane::App(Box::new(crate::host::pane::AppPane {
        pip_status: None,
        id: pane_id,
        runtime: crate::host::pane::AppRuntime::Builtin(app),
        workspace_root,
        permissions: crate::app::permissions::AppPermissions::builtin(),
        manifest_id: runtime_id.clone(),
        name,
        pane_group: None,
        linked_pane_id: None,
        overlay_replaced: None,
        hidden: false,
        agent: None,
        slots: std::collections::HashMap::new(),
        semantic_state: Default::default(),
    }))
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
        let vertical = matches!(
            hint,
            Some("split_h") | Some("split_right") | Some("split_left")
        );
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

    /// Single placement authority for a freshly-built app pane. Honors the
    /// layout `hint` identically for every runtime (builtin, Python-WASM, native
    /// WASM) so no open path can silently diverge:
    ///
    /// * `Some("overlay")` — replace the focused pane in place, reusing its pane
    ///   id and stashing the displaced pane in `overlay_replaced` so closing the
    ///   app restores it (see `restore_overlay_replacement`). Falls through to
    ///   the root/split path when the context is empty.
    /// * any other hint — split beside the focused pane, or install as the tree
    ///   root when no pane is focused.
    ///
    /// `build(pane_id, linked_pane_id, overlay_replaced) -> Pane` constructs the
    /// runtime-specific pane once its final id is known — overlay reuses the
    /// focused id; every other path allocates a fresh one via
    /// [`Self::open_pane_layout`]. Returns the installed pane id, or `None` when
    /// an overlay was requested against a degenerate focus target (the focused
    /// tile is not a pane, or its pane is missing from the map); callers treat
    /// that as a failed launch.
    fn place_app_pane<F>(
        &mut self,
        app_id: &str,
        group: Option<String>,
        hint: Option<&str>,
        share_hint: Option<f32>,
        build: F,
    ) -> Option<PaneId>
    where
        F: FnOnce(PaneId, Option<PaneId>, Option<Box<Pane>>) -> Pane,
    {
        let active = self.active_window;

        if matches!(hint, Some("overlay")) {
            if let Some(focused_tile) = self.windows[active].focused_pane {
                let Some(Tile::Pane(focused_pane_id)) =
                    self.windows[active].tree.tiles.get(focused_tile)
                else {
                    log::warn!(
                        "app::{app_id}: overlay launch skipped — focused tile is not a pane"
                    );
                    return None;
                };
                let pane_id = *focused_pane_id;
                let Some(replaced_pane) = self.windows[active].panes.remove(&pane_id) else {
                    log::warn!(
                        "app::{app_id}: overlay launch skipped — pane {pane_id} missing from pane map"
                    );
                    return None;
                };
                let pane = build(pane_id, None, Some(Box::new(replaced_pane)));
                self.windows[active].panes.insert(pane_id, pane);
                self.set_window_focused_pane(active, focused_tile);
                log::info!("app::{app_id}: launched as overlay on pane {pane_id}");
                return Some(pane_id);
            }
            log::info!(
                "app::{app_id}: overlay requested but empty context — launching as root pane"
            );
            // fall through to the root/split path below
        }

        if self.windows[active].zoomed_pane.take().is_some() {
            log::info!("app::{app_id}: cleared zoom before launch");
        }

        // Record which terminal we're splitting from before focus moves.
        let linked_pane_id = self.focused_terminal_id(active);
        let share = Self::share_ratio_from_fraction(app_id, share_hint);
        let (new_id, share, vertical, new_pane_first) =
            self.open_pane_layout(app_id, group, hint, share);
        let pane = build(new_id, linked_pane_id, None);
        self.windows[active].panes.insert(new_id, pane);

        // Empty context: no focused pane means no existing tile to split against —
        // install the new pane directly as the tree root.
        if self.windows[active].focused_pane.is_none() {
            let ctx = &mut self.windows[active];
            let root_tile = ctx.tree.tiles.insert_pane(new_id);
            ctx.tree.root = Some(root_tile);
            ctx.focused_pane = Some(root_tile);
            log::info!("app::{app_id}: launched as root pane {new_id} (empty context)");
            return Some(new_id);
        }
        let _ = self.split_with_new_pane(new_id, vertical, share, new_pane_first, false);
        log::info!("app::{app_id}: launched as split pane {new_id}");
        Some(new_id)
    }

    /// Open a built-in error tile pane when a capability pre-flight check fails.
    /// Uses `AppRuntime::Builtin` — no app runtime is spawned.
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
                pip_status: None,
                id: new_id,
                runtime: crate::host::pane::AppRuntime::Builtin(Box::new(
                    crate::host::launch_failed::LaunchFailedApp {
                        app_id: app_id.to_string(),
                        missing,
                    },
                )),
                workspace_root,
                permissions: crate::app::permissions::AppPermissions::default(),
                manifest_id: app_id.to_string(),
                name: format!("Cannot launch {app_id}"),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                hidden: false,
                agent: None,
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
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

    /// Open a sandboxed WASM component app in a new pane (the `run` primitive,
    /// G6). Loads `wasm_path` into a per-pane `wasmtime::Store`, links only the
    /// standard grants, and drives it through the synchronous effect loop.
    /// State is ephemeral — the namespace lives only as long as the pane, per
    /// the spec's default `run` lifecycle (`--persist` is a later addition).
    /// Returns the new pane id, or an error string if the component fails to
    /// load or instantiate (surfaced to the caller rather than crashing).
    pub(crate) fn open_wasm_app_pane(
        &mut self,
        app_id: &str,
        wasm_path: &std::path::Path,
        workspace_root: PathBuf,
        launch_args: Vec<String>,
    ) -> Result<PaneId, String> {
        use crate::host::wasm_app::{StateStore, WasmApp};

        let store = StateStore::ephemeral();
        let snapshot = store.snapshot();
        let permission_store =
            crate::app::permissions::PermissionStore::load_or_default(&crate::config::config_dir());
        let required_grants = WasmApp::inspect_required_grants(wasm_path)
            .map_err(|e| format!("inspect {}: {e}", wasm_path.display()))?;
        let required_caps = required_grants.capability_ids();
        let declared: std::collections::HashSet<String> = required_caps.iter().cloned().collect();
        let (remembered_grants, remembered_blocks) =
            permission_store.build_wasm_permission_sets(app_id, &workspace_root, &declared);
        let missing: Vec<String> = required_caps
            .iter()
            .filter(|cap| !remembered_grants.contains(*cap))
            .cloned()
            .collect();
        if !missing.is_empty() {
            log::warn!(
                "app::{app_id}: raw wasm launch requires review for {missing:?} path={}",
                wasm_path.display()
            );
            return Err(format!(
                "raw WASM launch requires review for: {}",
                missing.join(", ")
            ));
        }
        let app = WasmApp::load_with_grants(app_id, wasm_path, store, required_grants)
            .map_err(|e| format!("load {}: {e}", wasm_path.display()))?;
        self.open_wasm_app_instance(
            app_id,
            app,
            snapshot,
            workspace_root,
            crate::app::permissions::AppPermissions::default(),
            Some((permission_store, remembered_grants, remembered_blocks)),
            None,
            launch_args,
        )
    }

    fn open_python_wasm_app_pane(
        &mut self,
        app_dir: &std::path::Path,
        workspace_root: PathBuf,
        permissions: crate::app::permissions::AppPermissions,
        layout: Option<&str>,
        launch_args: Vec<String>,
    ) -> Result<PaneId, String> {
        let mut config = crate::host::wasm_python::PythonLaunchConfig::from_manifest_file(app_dir)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "manifest does not contain a Python entry".to_string())?;
        config.workspace_root = workspace_root.clone();
        config.launch_args = launch_args;
        config.capabilities = permissions
            .capabilities
            .iter()
            .copied()
            .map(|capability| capability.as_str().to_string())
            .collect();
        config.allowed_hosts = permissions.allowed_hosts.clone();
        let app_id = config.app_id.clone();
        let runtime = crate::host::wasm_python::LivePythonPane::launch(config)
            .map_err(|error| error.to_string())?;
        let manifest_id = app_id.clone();
        let name = app_id.clone();
        let new_id = self
            .place_app_pane(
                &app_id,
                None,
                layout,
                None,
                move |new_id, linked_pane_id, overlay_replaced| {
                    Pane::App(Box::new(crate::host::pane::AppPane {
                        pip_status: None,
                        id: new_id,
                        runtime: crate::host::pane::AppRuntime::Python(Box::new(runtime)),
                        workspace_root,
                        permissions,
                        manifest_id,
                        name,
                        pane_group: None,
                        linked_pane_id,
                        overlay_replaced,
                        hidden: false,
                        agent: None,
                        slots: std::collections::HashMap::new(),
                        semantic_state: Default::default(),
                    }))
                },
            )
            .ok_or_else(|| format!("overlay launch target unavailable for {app_id}"))?;
        self.hot_reload.watch(new_id, app_dir);
        crate::host::event_log::emit(crate::host::event_log::HostEvent::AppSpawned {
            app_id: app_id.clone(),
            type_id: "python-wasm".to_string(),
            pane_id: new_id,
            timestamp: crate::host::event_log::now_timestamp(),
        });
        log::info!("app::{app_id}: launched CPython WASM pane {new_id}");
        Ok(new_id)
    }

    fn review_or_open_raw_wasm_app_pane(
        &mut self,
        app_id: &str,
        wasm_path: &std::path::Path,
        workspace_root: PathBuf,
        launch_args: Vec<String>,
    ) -> Result<(), String> {
        let permission_store =
            crate::app::permissions::PermissionStore::load_or_default(&crate::config::config_dir());
        let required_grants = crate::host::wasm_app::WasmApp::inspect_required_grants(wasm_path)
            .map_err(|e| format!("inspect {}: {e}", wasm_path.display()))?;
        let required_caps = required_grants.capability_ids();
        let declared: HashSet<String> = required_caps.iter().cloned().collect();
        let (remembered_grants, remembered_blocks) =
            permission_store.build_wasm_permission_sets(app_id, &workspace_root, &declared);

        let blocked: Vec<String> = required_caps
            .iter()
            .filter(|cap| remembered_blocks.contains(*cap))
            .cloned()
            .collect();
        if !blocked.is_empty() {
            log::warn!(
                "raw_wasm_review: launch blocked by remembered denial app_id={app_id} path={} blocked={blocked:?}",
                wasm_path.display()
            );
            return Err(format!(
                "raw WASM launch blocked by remembered denial for: {}",
                blocked.join(", ")
            ));
        }

        let missing: Vec<String> = required_caps
            .iter()
            .filter(|cap| !remembered_grants.contains(*cap))
            .cloned()
            .collect();
        if missing.is_empty() {
            return self
                .open_wasm_app_pane(app_id, wasm_path, workspace_root, launch_args)
                .map(|_| ());
        }

        let already_pending = self.pending_raw_wasm_launches.iter().any(|pending| {
            pending.app_id == app_id
                && pending.wasm_path == wasm_path
                && pending.workspace_root == workspace_root
        });
        if !already_pending {
            log::info!(
                "raw_wasm_review: queued app_id={app_id} path={} missing={missing:?}",
                wasm_path.display()
            );
            self.pending_raw_wasm_launches
                .push_back(crate::app::PendingRawWasmLaunch {
                    app_id: app_id.to_string(),
                    wasm_path: wasm_path.to_path_buf(),
                    workspace_root,
                    missing_capabilities: missing,
                    launch_args,
                });
        }
        Ok(())
    }

    fn open_wasm_app_instance(
        &mut self,
        app_id: &str,
        app: crate::host::wasm_app::WasmApp,
        snapshot: crate::host::wasm_app::StateSnapshot,
        workspace_root: PathBuf,
        permissions: crate::app::permissions::AppPermissions,
        remembered: Option<(
            crate::app::permissions::PermissionStore,
            std::collections::HashSet<String>,
            std::collections::HashSet<String>,
        )>,
        layout: Option<&str>,
        launch_args: Vec<String>,
    ) -> Result<PaneId, String> {
        use crate::host::wasm_pane::{LiveWasmPane, SysinfoStats, WasmPane};

        let pane = WasmPane::new(app, Box::new(SysinfoStats::new()));
        let pane = if let Some((store, granted, blocked)) = remembered {
            pane.with_remembered_capabilities(workspace_root.clone(), store, granted, blocked)
        } else {
            pane
        };
        let mut live = LiveWasmPane::new(pane, app_id, snapshot, launch_args);

        let manifest_id = app_id.to_string();
        let name = app_id.to_string();
        let new_id = self
            .place_app_pane(
                app_id,
                None,
                layout,
                None,
                move |new_id, linked_pane_id, overlay_replaced| {
                    live.set_pane_id(new_id);
                    Pane::App(Box::new(crate::host::pane::AppPane {
                        pip_status: None,
                        id: new_id,
                        runtime: crate::host::pane::AppRuntime::Wasm(Box::new(live)),
                        workspace_root,
                        permissions,
                        manifest_id,
                        name,
                        pane_group: None,
                        linked_pane_id,
                        overlay_replaced,
                        hidden: false,
                        agent: None,
                        slots: std::collections::HashMap::new(),
                        semantic_state: Default::default(),
                    }))
                },
            )
            .ok_or_else(|| format!("overlay launch target unavailable for {app_id}"))?;
        crate::host::event_log::emit(crate::host::event_log::HostEvent::AppSpawned {
            app_id: app_id.to_string(),
            type_id: "wasm".to_string(),
            pane_id: new_id,
            timestamp: crate::host::event_log::now_timestamp(),
        });
        log::info!("wasm::{app_id}: spawned pane {new_id}");
        Ok(new_id)
    }

    fn open_installed_wasm_app_pane(
        &mut self,
        installed: crate::app::registry::InstalledApp,
        workspace_root: PathBuf,
        layout: Option<&str>,
        launch_args: Vec<String>,
    ) -> Result<PaneId, String> {
        use crate::app::permissions::{AppPermissions, Capability, PermissionStore};
        use crate::host::wasm_app::{Grants, StateStore, WasmApp};

        let app_id = installed.manifest.id.clone();

        // Cloud/remote execution is future work (stints 0286/0287): no
        // server-side wasmtime runtime or thin-client streaming exists yet.
        // Fail loudly at the launch boundary rather than silently running a
        // `execution = "cloud"`/`"preferred-local"` app locally — the Python
        // launch path already rejects the same shape in
        // `PythonLaunchConfig::from_manifest_file`.
        if installed.runtime.execution != crate::app::registry::RuntimeExecution::Local {
            return Err(format!(
                "app '{app_id}' declares runtime execution '{}', which is not available yet — \
                 only local execution is supported (cloud streaming is future work)",
                installed.runtime.execution.as_str()
            ));
        }
        let config_dir = crate::config::config_dir();
        let permission_store = PermissionStore::load_or_default(&config_dir);
        let declared_caps = crate::app::permissions::parse_capability_strings(
            &installed.manifest.capabilities.capabilities,
        )
        .map_err(|e| format!("manifest lists {e}"))?;
        let (granted_caps, blocked_caps) =
            permission_store.build_permission_sets(&app_id, &workspace_root, &declared_caps);
        let mut permissions = AppPermissions {
            capabilities: granted_caps,
            blocked: blocked_caps,
            is_builtin: false,
            allowed_hosts: installed.manifest.capabilities.allowed_hosts.clone(),
        };
        permissions
            .capabilities
            .retain(|cap| !permissions.blocked.contains(cap));

        let wasm_declared =
            wasm_runtime_capabilities_from_permissions(&permissions, &workspace_root);
        let (remembered_grants, remembered_blocks) =
            permission_store.build_wasm_permission_sets(&app_id, &workspace_root, &wasm_declared);

        let grants = Grants {
            state: true,
            pipes: permissions.capabilities.contains(&Capability::PipeOpen),
            gpu: permissions.capabilities.contains(&Capability::GpuRender),
            audio: permissions
                .capabilities
                .contains(&Capability::AudioPlayback),
        };
        let store = StateStore::persistent(wasm_state_path(&app_id, &workspace_root))
            .map_err(|e| format!("open wasm state for {app_id}: {e}"))?;
        let snapshot = store.snapshot();
        let app = WasmApp::load_with_grants(&app_id, &installed.bin_path, store, grants)
            .map_err(|e| format!("load {}: {e}", installed.bin_path.display()))?;

        log::info!(
            "wasm::{app_id}: manifest-backed launch entry={} remembered_grants={} remembered_blocks={}",
            installed.bin_path.display(),
            remembered_grants.len(),
            remembered_blocks.len()
        );
        self.open_wasm_app_instance(
            &app_id,
            app,
            snapshot,
            workspace_root,
            permissions,
            Some((permission_store, remembered_grants, remembered_blocks)),
            layout,
            launch_args,
        )
    }

    /// Relaunch a CPython-in-WASM pane while preserving its pane envelope.

    pub(crate) fn reload_app_pane(&mut self, pane_id: PaneId, reason: &str) -> bool {
        let active = self.active_window;
        let Some(app_pane) = self.windows[active]
            .panes
            .get_mut(&pane_id)
            .and_then(|pane| pane.as_app_mut())
        else {
            return false;
        };
        let crate::host::pane::AppRuntime::Python(runtime) = &mut app_pane.runtime else {
            return false;
        };
        log::info!(
            "app::{} reload triggered ({reason}) for CPython WASM pane {pane_id}",
            app_pane.manifest_id
        );
        runtime.relaunch().is_ok()
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

    pub(crate) fn drain_crash_restarts(&mut self) {}

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
        let app_type_id = app.type_id().to_string();
        let app_name = app.display_name();
        let manifest_id = app_type_id.clone();
        let group_for_pane = group.clone();
        let placed = self.place_app_pane(
            &app_type_id,
            group,
            hint,
            share,
            move |id, linked_pane_id, overlay_replaced| {
                Pane::App(Box::new(crate::host::pane::AppPane {
                    pip_status: None,
                    id,
                    runtime: crate::host::pane::AppRuntime::Builtin(app),
                    workspace_root,
                    permissions,
                    manifest_id,
                    name: app_name,
                    pane_group: group_for_pane,
                    linked_pane_id,
                    overlay_replaced,
                    hidden: false,
                    agent: None,
                    slots: std::collections::HashMap::new(),
                    semantic_state: Default::default(),
                }))
            },
        );
        if let Some(pane_id) = placed {
            crate::host::event_log::emit(crate::host::event_log::HostEvent::AppSpawned {
                app_id: app_type_id.clone(),
                type_id: app_type_id.clone(),
                pane_id,
                timestamp: crate::host::event_log::now_timestamp(),
            });
        }
    }

    pub(super) fn create_single_pane_tree(
        &mut self,
        cwd: Option<PathBuf>,
        initial_cmd: Option<&str>,
        close_on_exit: bool,
    ) -> Option<(Tree<PaneId>, HashMap<PaneId, Pane>, TileId)> {
        let new_id = self.host.alloc_pane_id();

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
            log::info!(
                "create_single_pane_tree: initial_cmd={cmd:?} close_on_exit={close_on_exit}"
            );
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
        self.windows[win_idx]
            .panes
            .insert(new_id, Pane::Terminal(Box::new(pane)));

        let share =
            crate::host::command::ShareRatio::new(1.0, 1.0).expect("1:1 is a valid ShareRatio");
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

        // Resolve via the canonical new-pane resolver so the file browser
        // honors the context root when set (context.root → focused-pane cwd →
        // home), matching every other new-pane path. A GUI launch whose focused
        // cwd is "/" still falls back to home.
        let cwd = self
            .resolve_new_pane_cwd(None, self.windows[self.active_window].focused_pane)
            .filter(|p| p != &PathBuf::from("/"))
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
        log::info!(
            "open_file_browser: cwd={} context_root={:?}",
            cwd.display(),
            self.router.active().root
        );

        let app: Box<dyn App> = Box::new(crate::file_browser::FileBrowserApp::new(cwd.clone()));

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

    /// Locate an open instance of app `manifest_id`, including one currently
    /// buried under overlay replacements. When `context_id` is `Some`, only
    /// panes whose owning window belongs to that context match; `None` searches
    /// every window in every context. Returns `(window_index, slot_pane_id,
    /// overlay_depth)` — depth 0 means the instance is the visible pane at the
    /// slot, depth N means N overlays sit on top of it and must be popped to
    /// reveal it. Backs the `[launch] on_launch` resolver (#0336).
    ///
    /// Identity is `AppPane::manifest_id`, not `runtime.type_id()`: WASM panes
    /// all report `type_id() == "wasm"`, so only the manifest id is a stable,
    /// runtime-agnostic identity that equals the launch id across process,
    /// WASM, and builtin apps. Pane lookup is global by design (#878); context
    /// scoping comes purely from the owning window's `context_id`.
    pub(crate) fn locate_app_instance(
        &self,
        manifest_id: &str,
        context_id: Option<u64>,
    ) -> Option<(usize, PaneId, usize)> {
        // Walk a pane's overlay-replacement chain, returning how many overlays
        // sit above the first pane whose manifest id matches (`0` = the pane
        // itself matches). The replaced pane an overlay stores can be any
        // variant; only app panes carry an id and a further chain.
        fn overlay_depth(pane: &Pane, manifest_id: &str) -> Option<usize> {
            let mut current = pane;
            let mut depth = 0;
            loop {
                let app = current.as_app()?;
                if app.manifest_id == manifest_id {
                    return Some(depth);
                }
                current = app.overlay_replaced.as_deref()?;
                depth += 1;
            }
        }

        self.windows.iter().enumerate().find_map(|(win_idx, win)| {
            if context_id.is_some_and(|c| c != win.context_id) {
                return None;
            }
            win.panes.iter().find_map(|(slot, pane)| {
                overlay_depth(pane, manifest_id).map(|d| (win_idx, *slot, d))
            })
        })
    }

    /// Convenience wrapper returning `(slot_pane_id, window_index)` for the
    /// first open instance of `manifest_id`, discarding overlay depth. Used by
    /// tests that only assert which instance was matched.
    #[cfg(test)]
    pub(crate) fn find_app_pane_by_type(
        &self,
        manifest_id: &str,
        context_id: Option<u64>,
    ) -> Option<(PaneId, usize)> {
        self.locate_app_instance(manifest_id, context_id)
            .map(|(win_idx, slot, _)| (slot, win_idx))
    }

    /// Resolve an app's `[launch] on_launch` policy (#0336) against currently
    /// open panes before spawning. Returns `Some(existing_pane_id)` when the
    /// launch was satisfied by focusing a live instance — the caller must then
    /// NOT spawn, and must report *that* pane id (never a predicted one).
    /// Returns `None` when no instance matched and the caller should spawn.
    ///
    /// `caller_context_id` is the context the launch originates in (the active
    /// window's context). Cross-context focus for `focus_existing` uses the
    /// sanctioned [`jump_to_context`](Self::jump_to_context) path; it never
    /// mutates `active_window`/`router.active` to steer the focus. An instance
    /// buried under overlays is revealed by popping those overlays via the
    /// sanctioned [`restore_overlay_replacement`](super::layout::restore_overlay_replacement)
    /// path — the same restore Esc performs — before focusing. Revealing a
    /// buried instance therefore pops (and closes) every app overlay stacked
    /// above it, exactly as pressing Esc that many times would; this is
    /// intentional, not incidental.
    ///
    /// `args`/`cwd_override` are the relaunch payload. When a dedup focuses an
    /// existing instance they are *dropped* — the running app never receives
    /// them (delivering relaunch args to a live instance via a bus event is
    /// future work). The drop is logged at `warn` so it is observable.
    pub(crate) fn resolve_on_launch_policy(
        &mut self,
        id: &str,
        caller_context_id: u64,
        args: &[String],
        cwd_override: Option<&Path>,
    ) -> Option<PaneId> {
        use crate::app::registry::OnLaunchPolicy;
        let (scope, target) = match self.registry.on_launch_for(id) {
            OnLaunchPolicy::AlwaysNew => return None,
            OnLaunchPolicy::FocusExisting => ("focus_existing", self.locate_app_instance(id, None)),
            OnLaunchPolicy::FocusExistingInContext => (
                "focus_existing_in_context",
                self.locate_app_instance(id, Some(caller_context_id)),
            ),
        };
        let Some((win_idx, slot, overlay_depth)) = target else {
            log::info!(
                "on_launch[{scope}]: no existing '{id}' instance to focus (context {caller_context_id}) — spawning"
            );
            return None;
        };
        let ctx_id = self.windows[win_idx].context_id;
        let win_id = self.windows[win_idx].window_id;
        log::info!(
            "on_launch[{scope}]: '{id}' already open at slot {slot} in context {ctx_id} (overlay_depth={overlay_depth}) — focusing instead of spawning"
        );
        if !args.is_empty() || cwd_override.is_some() {
            log::warn!(
                "on_launch[{scope}]: '{id}' relaunch carried args={args:?} cwd={cwd_override:?} \
                 but the policy focused the existing instance — these are NOT delivered to it \
                 (relaunch arg delivery to a running instance is future work)"
            );
        }
        // Pop any overlays covering the instance so a relaunch reveals it, then
        // unhide (it may have been hidden rather than overlaid) and focus.
        for _ in 0..overlay_depth {
            super::layout::restore_overlay_replacement(&mut self.windows[win_idx].panes, slot);
        }
        if let Some(pane) = self.windows[win_idx].panes.get_mut(&slot) {
            pane.set_hidden(false);
        }
        self.jump_to_context(win_idx, win_id, Some(slot));
        Some(slot)
    }

    /// Launch an installed app by id in the focused pane.
    pub(crate) fn launch_app_by_id(&mut self, id: &str) {
        let _ = self.launch_app_by_id_with_layout(id, None, &[], None);
    }

    /// Launch an installed app with an explicit layout and args override.
    ///   "overlay" (default) — full pane takeover; Esc restores the original pane
    ///   "split_h"           — horizontal split, new pane to the right
    ///   "split_v"           — vertical split, new pane below
    ///
    /// Honors the app's `[launch] on_launch` dedup policy: returns
    /// `Ok(Some(existing_pane_id))` when a live instance satisfied the launch
    /// (nothing was spawned), and `Ok(None)` when a fresh pane was spawned (the
    /// caller's predicted `next_pane_id()` is then the real id).
    pub(crate) fn launch_app_by_id_with_layout(
        &mut self,
        id: &str,
        layout: Option<String>,
        args: &[String],
        cwd_override: Option<PathBuf>,
    ) -> Result<Option<PaneId>, String> {
        self.launch_app_by_id_with_layout_inner(id, layout, args, cwd_override, false)
    }

    /// Like [`launch_app_by_id_with_layout`](Self::launch_app_by_id_with_layout)
    /// but bypasses the `[launch] on_launch` dedup policy so a fresh instance is
    /// always spawned. Used by the split-mirror path (#0336), whose whole point
    /// is to duplicate a pane — with a singleton app the policy would otherwise
    /// silently no-op the mirror.
    pub(crate) fn launch_app_by_id_with_layout_forced(
        &mut self,
        id: &str,
        layout: Option<String>,
        args: &[String],
        cwd_override: Option<PathBuf>,
    ) -> Result<Option<PaneId>, String> {
        self.launch_app_by_id_with_layout_inner(id, layout, args, cwd_override, true)
    }

    fn launch_app_by_id_with_layout_inner(
        &mut self,
        id: &str,
        layout: Option<String>,
        args: &[String],
        cwd_override: Option<PathBuf>,
        bypass_on_launch: bool,
    ) -> Result<Option<PaneId>, String> {
        // "terminal" is a builtin pane type, not in the app registry.
        // Reached via SDK AppCommand::SpawnApp("terminal", ...) and legacy paths.
        // Socket IPC and spawn-queue handle terminal inline in app/mod.rs.
        if id == "terminal" {
            let layout_str = layout.as_deref().unwrap_or("split_h");
            let vertical = matches!(layout_str, "split_v" | "split_below" | "split_above");
            let new_pane_first = matches!(layout_str, "split_above" | "split_left");
            let initial_cmd = if args.is_empty() {
                None
            } else {
                Some(crate::host::shell::shell_join(args))
            };
            log::info!(
                "SpawnPane: terminal layout='{layout_str}' vertical={vertical} new_pane_first={new_pane_first} initial_cmd={initial_cmd:?}"
            );
            self.split_focused(
                vertical,
                initial_cmd.as_deref(),
                false,
                new_pane_first,
                None,
            );
            return Ok(None);
        }

        // The Assistant is a compiled-in app whose constructor needs the live
        // broker and channel profile, so it cannot participate in the
        // broker-free `builtin_factory`. Keep it on the same generic app-open
        // route as every other builtin so scene and CLI callers do not need an
        // app-specific command.
        if id == "assistant" {
            if !crate::release::feature_enabled(crate::release::ReleaseFeature::Assistant) {
                crate::release::log_feature_blocked(crate::release::ReleaseFeature::Assistant);
                return Err("assistant is not enabled on this release channel".to_string());
            }
            let predicted = self.host.next_pane_id();
            self.open_assistant_pane_with_hint(layout.as_deref().unwrap_or("overlay"));
            let pane_id = self.windows[self.active_window]
                .panes
                .iter()
                .find_map(|(pane_id, pane)| {
                    pane.as_app()
                        .filter(|app| app.runtime.type_id() == "assistant")
                        .map(|_| *pane_id)
                })
                .unwrap_or(predicted);
            log::info!(
                "launch_app_by_id_with_layout: 'assistant' resolved as builtin pane_id={pane_id}"
            );
            return Ok(Some(pane_id));
        }

        // When the caller does not specify a placement, fall back to the app's
        // manifest-declared `[launch] placement` before the host default
        // (`overlay`). One resolution point so every downstream hint inherits
        // it (#2283). Builtins are not in the registry → None → `overlay`.
        let layout = layout.or_else(|| self.registry.placement_for(id));

        // #0336: honor the app's `[launch] on_launch` dedup policy before any
        // spawn path (unless the caller forces a fresh instance, e.g. the
        // split-mirror). If a live instance already satisfies the policy we
        // focus it and return its real pane id; otherwise we fall through and
        // spawn as usual. This sits ahead of the background re-attach below on
        // purpose: the policy only matches *open* panes, so a `focus_existing`
        // app whose pane was closed (its process parked in `background_apps`)
        // finds no pane here, falls through, and re-attaches — the two compose.
        if !bypass_on_launch {
            let caller_context_id = self.windows[self.active_window].context_id;
            if let Some(existing_id) =
                self.resolve_on_launch_policy(id, caller_context_id, args, cwd_override.as_deref())
            {
                return Ok(Some(existing_id));
            }
        }

        let cwd_explicit = cwd_override.is_some();
        let cwd = cwd_override
            .or_else(|| {
                self.resolve_new_pane_cwd(None, self.windows[self.active_window].focused_pane)
            })
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
        log::info!(
            "launch_app_by_id_with_layout: id={id} cwd={cwd:?} cwd_source={} context_root={:?}",
            if cwd_explicit { "explicit" } else { "resolved" },
            self.router.active().root
        );

        // Builtin app factory — resolved before the process registry; builtins never touch disk.
        if let Some(app) = builtin_factory(id, &cwd, args) {
            log::info!("launch_app_by_id_with_layout: '{id}' resolved as builtin");
            let group = self
                .registry
                .group_for(id)
                .or_else(|| builtin_default_group(app.type_id()));
            let hint = layout.unwrap_or_else(|| "overlay".to_string());
            let perms = crate::app::permissions::AppPermissions::builtin();
            self.open_builtin_app_pane(app, perms, cwd, group, Some(&hint), None);
            return Ok(None);
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
            let fail_hint = layout.or_else(|| Some("overlay".to_string()));
            self.open_launch_failed_pane(id, fail_hint.as_deref(), missing, cwd);
            return Ok(None);
        }

        // Query group/hint after any registry reload so metadata reflects the
        // actual registry that found the app.
        let hint = layout.or_else(|| Some("overlay".to_string()));
        if let Some(installed) = self.registry.get(id).cloned() {
            if installed.manifest.manifest_type == crate::app::registry::ManifestType::Wasm {
                let workspace_root = installed
                    .workspace_root
                    .clone()
                    .unwrap_or_else(|| cwd.clone());
                self.open_installed_wasm_app_pane(
                    installed,
                    workspace_root,
                    hint.as_deref(),
                    args.to_vec(),
                )?;
                return Ok(None);
            }
            if matches!(
                installed.bin_path.extension().and_then(|ext| ext.to_str()),
                Some("py" | "pyc")
            ) {
                let workspace_root = installed
                    .workspace_root
                    .clone()
                    .unwrap_or_else(|| cwd.clone());
                let permissions = installed.manifest.capabilities.to_permissions();
                self.open_python_wasm_app_pane(
                    &installed.app_dir,
                    workspace_root,
                    permissions,
                    hint.as_deref(),
                    args.to_vec(),
                )?;
                return Ok(None);
            }
        }

        // Dev-pack apps (apps/dev/*) are intentionally excluded from the
        // id-addressable registry (apps/AGENTS.md) — install.sh never syncs
        // them into a profile. When the caller passed an explicit cwd, `cwd`
        // already is that value; otherwise prefer the focused pane's actual
        // cwd over `cwd` (which may have resolved to the context root ahead
        // of it — see stint 0399) so this fires for a developer sitting in a
        // repo checkout.
        let search_start = if cwd_explicit {
            cwd.clone()
        } else {
            self.windows[self.active_window]
                .focused_pane
                .and_then(|f| self.windows[self.active_window].get_focused_pane_cwd(f))
                .unwrap_or_else(|| cwd.clone())
        };
        if let Some(dev_app_dir) = crate::app::registry::find_dev_pack_app(id, &search_start) {
            log::warn!(
                "launch_app_by_id: '{id}' is a dev-pack app ({dev_app_dir:?}) — not id-openable"
            );
            return Err(format!(
                "'{id}' is a dev-pack app ({}) — dev apps are path-open only. Use `plexi app open {}` instead.",
                dev_app_dir.display(),
                dev_app_dir.display()
            ));
        }

        log::warn!("launch_app_by_id: app '{id}' has no WASM runtime");
        Err(format!("app '{id}' has no WASM runtime"))
    }

    /// Launch an app directly from a filesystem path without looking it up in the registry.
    ///
    /// The app runs in-place; its own directory is used as `workspace_root`.
    #[cfg(test)]
    pub(crate) fn launch_app_by_path_with_layout(
        &mut self,
        app_path: &str,
        layout: Option<String>,
        workspace_root_override: Option<std::path::PathBuf>,
        args: &[String],
    ) -> Result<(), String> {
        self.launch_app_by_path_with_layout_inner(
            app_path,
            layout,
            workspace_root_override,
            args,
            true,
        )
        .map(|_| ())
    }

    pub(crate) fn launch_app_by_path_with_layout_no_review_modal(
        &mut self,
        app_path: &str,
        layout: Option<String>,
        workspace_root_override: Option<std::path::PathBuf>,
        args: &[String],
    ) -> Result<Option<PaneId>, String> {
        self.launch_app_by_path_with_layout_inner(
            app_path,
            layout,
            workspace_root_override,
            args,
            false,
        )
    }

    fn launch_app_by_path_with_layout_inner(
        &mut self,
        app_path: &str,
        layout: Option<String>,
        workspace_root_override: Option<std::path::PathBuf>,
        args: &[String],
        queue_raw_wasm_review: bool,
    ) -> Result<Option<PaneId>, String> {
        let app_dir = PathBuf::from(app_path);
        log::info!("launch_app_by_path_with_layout: path={app_path}");

        // A `.wasm` file is a sandboxed component app (the run primitive, G6),
        // not a manifest-backed process app. Route it to the wasmtime path.
        if app_dir.extension().and_then(|e| e.to_str()) == Some("wasm") {
            let app_id = app_dir
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("wasm")
                .to_string();
            let workspace_root = workspace_root_override.unwrap_or_else(|| {
                app_dir
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| PathBuf::from("."))
            });
            if queue_raw_wasm_review {
                return self
                    .review_or_open_raw_wasm_app_pane(
                        &app_id,
                        &app_dir,
                        workspace_root,
                        args.to_vec(),
                    )
                    .map(|_| None);
            }
            return self
                .open_wasm_app_pane(&app_id, &app_dir, workspace_root, args.to_vec())
                .map(Some);
        }

        let installed = match self.registry.load_app(&app_dir) {
            Ok(a) => a,
            Err(e) => {
                log::warn!(
                    "launch_app_by_path_with_layout: failed to load manifest at {app_path}: {e}"
                );
                return Err(format!("failed to load app at '{app_path}': {e}"));
            }
        };

        let perms = installed.manifest.capabilities.to_permissions();
        let app_id = installed.manifest.id.clone();

        let manifest_placement = installed
            .launch
            .placement
            .clone()
            .filter(|p| crate::app::registry::VALID_PLACEMENTS.contains(&p.as_str()));
        let layout_hint = Some(cli_open_placement(layout, manifest_placement));

        let workspace_root = workspace_root_override.unwrap_or_else(|| app_dir.clone());
        log::info!(
            "launch_app_by_path_with_layout: workspace_root={}",
            workspace_root.display()
        );
        if installed.manifest.manifest_type == crate::app::registry::ManifestType::Wasm {
            let pane_id = self.open_installed_wasm_app_pane(
                installed,
                workspace_root,
                layout_hint.as_deref(),
                args.to_vec(),
            )?;
            return Ok(Some(pane_id));
        }
        if installed.bin_path.extension().and_then(|ext| ext.to_str()) == Some("py")
            || installed.bin_path.extension().and_then(|ext| ext.to_str()) == Some("pyc")
        {
            return self
                .open_python_wasm_app_pane(
                    &app_dir,
                    workspace_root,
                    perms,
                    layout_hint.as_deref(),
                    args.to_vec(),
                )
                .map(Some);
        }
        Err(format!("app '{app_id}' has no WASM runtime"))
    }

    /// Attempt to spawn a PGAP process from the CLI's native `--plexi` descriptor
    /// `plexi_app` field. Returns `None` if the CLI is not found, does not support
    /// `--plexi`, or has no `plexi_app` field.

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

    /// Open (or focus) the host Assistant pane (Phase 1 of
    /// `docs/assistant-host-app.md`, reachable via Cmd+Ctrl+A and the
    /// Cmd+P palette). Opens as an overlay — it overtakes the focused pane
    /// like every other app launch. One Assistant pane per window: if one
    /// already exists it is focused and unhidden instead of spawning a
    /// duplicate. Conversation state is workspace-scoped on disk, so
    /// close-then-reopen resumes the same conversation.
    ///
    /// Builtin exception to the manifest `[launch] on_launch` policy (#0336):
    /// the Assistant is a host builtin with its own entry point and is never
    /// routed through `launch_app_by_id_with_layout`, so the generic resolver
    /// does not see it. Its dedup is per *window* (not per context) and is kept
    /// hardcoded here rather than expressed as `focus_existing_in_context`,
    /// which would change the granularity from window to context.
    pub(crate) fn open_assistant_pane(&mut self) {
        self.open_assistant_pane_with_hint("overlay");
    }

    fn open_assistant_pane_with_hint(&mut self, hint: &str) {
        if !crate::release::feature_enabled(crate::release::ReleaseFeature::Assistant) {
            crate::release::log_feature_blocked(crate::release::ReleaseFeature::Assistant);
            return;
        }
        let active = self.active_window;
        // Focus an existing Assistant pane instead of opening a second one.
        let existing = self.windows[active].panes.iter().find_map(|(id, pane)| {
            pane.as_app()
                .filter(|a| a.runtime.type_id() == "assistant")
                .map(|_| *id)
        });
        if let Some(pane_id) = existing {
            let win = &mut self.windows[active];
            if let Some(pane) = win.panes.get_mut(&pane_id) {
                pane.set_hidden(false);
            }
            let tile_id = win.tree.tiles.iter().find_map(|(tid, tile)| {
                matches!(tile, egui_tiles::Tile::Pane(pid) if *pid == pane_id).then_some(*tid)
            });
            if let Some(tile_id) = tile_id {
                win.focused_pane = Some(tile_id);
            }
            log::info!("assistant: focused existing pane {pane_id}");
            return;
        }

        let workspace_root = crate::config::active_workspace_root()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
        log::info!(
            "assistant: opening pane for workspace {}",
            workspace_root.display()
        );
        let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> = std::sync::Arc::new(
            crate::plexi_ai::broker::LiveAiBroker::new(self.config.ai.clone()),
        );
        let app = Box::new(crate::assistant::AssistantApp::new(
            workspace_root.clone(),
            broker,
            &crate::config::config_dir(),
        ));
        let perms = crate::app::permissions::AppPermissions::builtin();
        self.open_builtin_app_pane(app, perms, workspace_root, None, Some(hint), None);
    }

    /// Create a new scratch note in `notes/inbox/` and open it in a text-editor
    /// pane. Each press creates a fresh timestamped note — scratch notes share
    /// the inbox with quick notes and flow through the same triage. Notes whose
    /// body is still empty on close are deleted by the editor.
    ///
    /// Works from any focused pane type (terminal, app, file browser).
    pub(crate) fn open_scratchpad(&mut self) {
        log::info!("scratchpad: opening new scratch note");
        let active = self.active_window;
        let target_pane_id = self.windows[active].focused_pane.and_then(|tile_id| {
            match self.windows[active].tree.tiles.get(tile_id) {
                Some(Tile::Pane(pane_id)) => Some(*pane_id),
                _ => None,
            }
        });

        // Capture context like a quick note so triage shows where it came from.
        let cwd = self.windows[active]
            .focused_pane
            .and_then(|tile_id| self.windows[active].get_focused_pane_cwd(tile_id))
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
        let ctx = crate::app::QuickNoteCtx {
            cwd,
            workspace_root: crate::config::active_workspace_root(),
            context_root: self.router.active().root.clone(),
        };
        let dir = crate::config::config_dir().join("notes").join("inbox");
        let Some(path) = Self::write_inbox_note("", "scratchpad", &dir, &ctx) else {
            log::warn!("scratchpad: failed to create scratch note in {:?}", dir);
            return;
        };

        let path_str = path.display().to_string();
        log::info!("scratchpad: opening text-editor pane for {:?}", path);
        match self.launch_app_by_id_with_layout("text-editor", None, &[path_str.clone()], None) {
            Ok(_) => {
                let pane_id = target_pane_id.or_else(|| {
                    self.windows[active].focused_pane.and_then(|tile_id| {
                        match self.windows[active].tree.tiles.get(tile_id) {
                            Some(Tile::Pane(pane_id)) => Some(*pane_id),
                            _ => None,
                        }
                    })
                });
                if let Some(pane_id) = pane_id {
                    crate::host::event_log::emit(
                        crate::host::event_log::HostEvent::ScratchpadOpened {
                            pane_id,
                            path: path_str,
                            timestamp: crate::host::event_log::now_timestamp(),
                        },
                    );
                } else {
                    log::warn!("scratchpad: opened note but could not resolve pane id");
                }
            }
            Err(e) => {
                log::warn!("scratchpad: failed to launch text-editor pane: {e}");
            }
        }
    }

    /// Open the quick note modal: capture context and push FocusKind::QuickNote.
    pub(crate) fn open_quick_note_modal(&mut self) {
        let active = self.active_window;
        let cwd = self.windows[active]
            .focused_pane
            .and_then(|tile_id| self.windows[active].get_focused_pane_cwd(tile_id))
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
        let workspace_root = crate::config::active_workspace_root();
        let context_root = self.router.active().root.clone();
        self.quick_note_text = String::new();
        self.quick_note_ctx = crate::app::QuickNoteCtx {
            cwd,
            workspace_root,
            context_root,
        };
        self.push_focus_layer(crate::app::FocusKind::QuickNote);
        log::info!(
            "QuickNote: modal opened — cwd={}, workspace={:?}",
            self.quick_note_ctx.cwd.display(),
            self.quick_note_ctx.workspace_root
        );
    }

    /// Commit a quick note directly to notes/inbox/. Returns `false` if the write fails.
    pub(crate) fn commit_quick_note_to_inbox(&mut self, text: &str) -> bool {
        let ctx = self.quick_note_ctx.clone();
        let dir = crate::config::config_dir().join("notes").join("inbox");
        log::info!("QuickNote: committing to notes/inbox/");
        Self::write_inbox_note(text, "quick-note", &dir, &ctx).is_some()
    }

    /// Write a timestamped note with capture-context frontmatter into `dir`.
    /// Returns the created path, or `None` when the write fails.
    fn write_inbox_note(
        text: &str,
        source: &str,
        dir: &PathBuf,
        ctx: &crate::app::QuickNoteCtx,
    ) -> Option<PathBuf> {
        use chrono::Utc;
        let now = Utc::now();
        let captured_at = now.to_rfc3339();
        let stamp = now.format("%Y%m%d-%H%M%S%.3f").to_string();

        let workspace = ctx
            .workspace_root
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let cwd_str = ctx.cwd.to_string_lossy();
        let context_root_str = ctx
            .context_root
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();

        let frontmatter = format!(
            "---\ntitle: \"\"\ncaptured_at: \"{captured_at}\"\nsource: \"{source}\"\ncwd: \"{cwd_str}\"\nworkspace: \"{workspace}\"\ncontext_root: \"{context_root_str}\"\n---\n\n"
        );
        let content = format!("{frontmatter}{}\n", text.trim());

        if let Err(e) = std::fs::create_dir_all(dir) {
            log::error!(
                "QuickNote: failed to create inbox dir {}: {e}",
                dir.display()
            );
            return None;
        }
        // Use O_CREAT|O_EXCL (create_new) so concurrent scratchpad calls can
        // never overwrite each other — the check-then-write pattern has a TOCTOU
        // window that is safe today but unsafe if notes are written in parallel.
        let mut path = dir.join(format!("note-{stamp}.md"));
        let mut n = 1u32;
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    use std::io::Write as _;
                    match file.write_all(content.as_bytes()) {
                        Ok(()) => {
                            log::info!("QuickNote: saved to {}", path.display());
                            return Some(path);
                        }
                        Err(e) => {
                            // create_new already created the inode; remove it so
                            // a partial write doesn't leave a ghost file blocking
                            // any future attempt at the same path.
                            let _ = std::fs::remove_file(&path);
                            log::error!("QuickNote: save failed {}: {e}", path.display());
                            return None;
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    path = dir.join(format!("note-{stamp}-{n}.md"));
                    n += 1;
                }
                Err(e) => {
                    log::error!("QuickNote: save failed {}: {e}", path.display());
                    return None;
                }
            }
        }
    }
}

/// Resolve the pane placement for a CLI- or spawn-request-initiated app open.
///
/// The caller's pane is typically the agent's own terminal, so a CLI open must
/// never take over it (`overlay`). Precedence: an explicit layout from the
/// command flag, then the app's manifest `[launch] placement`, then a sibling
/// split (`split_h`). Manifest placement still overrides the default; only the
/// fallback changed from `overlay` to `split_h` (stint 0330). `manifest_placement`
/// must already be validated against [`crate::app::registry::VALID_PLACEMENTS`].
pub(crate) fn cli_open_placement(
    explicit: Option<String>,
    manifest_placement: Option<String>,
) -> String {
    explicit
        .or(manifest_placement)
        .unwrap_or_else(|| "split_h".to_string())
}

/// Build a fresh timestamped note path in `notes/inbox/`. Does not create the file.
fn new_inbox_note_path() -> PathBuf {
    let filename = format!("note-{}.md", chrono::Utc::now().format("%Y%m%d-%H%M%S%.3f"));
    crate::config::config_dir()
        .join("notes")
        .join("inbox")
        .join(filename)
}

fn wasm_runtime_capabilities_from_permissions(
    permissions: &crate::app::permissions::AppPermissions,
    workspace_root: &std::path::Path,
) -> std::collections::HashSet<String> {
    use crate::app::permissions::Capability;

    let mut caps = std::collections::HashSet::new();
    if permissions.capabilities.contains(&Capability::FsRead) {
        caps.insert(format!("fs:read:{}", workspace_root.display()));
    }
    if permissions.capabilities.contains(&Capability::FsWrite) {
        caps.insert(format!("fs:write:{}", workspace_root.display()));
    }
    if permissions.capabilities.contains(&Capability::NetHttp) {
        if permissions.allowed_hosts.is_empty() {
            caps.insert("net:fetch:*".to_string());
        } else {
            caps.extend(
                permissions
                    .allowed_hosts
                    .iter()
                    .map(|host| format!("net:fetch:{host}")),
            );
        }
    }
    if permissions.capabilities.contains(&Capability::AiQuery) {
        caps.insert("ai.query".to_string());
    }
    caps
}

fn wasm_state_path(app_id: &str, workspace_root: &std::path::Path) -> PathBuf {
    crate::config::config_dir()
        .join("wasm-state")
        .join(sanitize_wasm_path_component(app_id))
        .join(format!(
            "{}.json",
            sanitize_wasm_path_component(&workspace_root.display().to_string())
        ))
}

fn sanitize_wasm_path_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "root".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::permissions::AppPermissions;
    use crate::host::wasm_app::{StateSnapshot, StateStore, WasmApp};
    use crate::testing::HostHarness;

    fn counter_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/counter.wasm")
    }

    /// A loadable native-WASM component and the state snapshot the pane opens
    /// with. Snapshot is taken before the store moves into the app, mirroring the
    /// production launch path.
    fn load_counter() -> (WasmApp, StateSnapshot) {
        let store = StateStore::ephemeral();
        let snapshot = store.snapshot();
        let app = WasmApp::load_ephemeral_run("counter-pane", &counter_fixture(), store)
            .expect("load counter fixture");
        (app, snapshot)
    }

    /// Stint 0431: the native-WASM open path must honor the default `"overlay"`
    /// placement hint (the layout Cmd+P uses) — replace the focused pane in
    /// place, reusing its id and stashing the displaced pane for restore — not
    /// unconditionally split a new pane below. Regression:
    /// `open_wasm_app_instance` ignored the hint and always split.
    #[test]
    fn open_wasm_app_instance_overlay_replaces_focused_pane_in_place() {
        let mut h = HostHarness::new();
        let focused = h.add_test_pane();
        h.focus_pane(focused);
        let count_before = h.pane_count();

        let (app, snapshot) = load_counter();
        let pane_id = h
            .app
            .open_wasm_app_instance(
                "counter-pane",
                app,
                snapshot,
                std::env::temp_dir(),
                AppPermissions::default(),
                None,
                Some("overlay"),
                Vec::new(),
            )
            .expect("overlay launch returns the reused pane id");

        assert_eq!(pane_id, focused, "overlay reuses the focused pane's id");
        assert_eq!(
            h.pane_count(),
            count_before,
            "overlay replaces in place — no extra pane is split"
        );
        let stashed_replacement = h.app.windows[0]
            .panes
            .get(&focused)
            .and_then(Pane::as_app)
            .map(|app| app.overlay_replaced.is_some())
            .unwrap_or(false);
        assert!(
            stashed_replacement,
            "overlay stashes the displaced pane in overlay_replaced for restore-on-close"
        );
    }

    /// Stint 0431: an overlay launch into an empty context (no focused pane)
    /// must fall through to the root-pane path and install the app as the tree
    /// root — never a silent no-op.
    #[test]
    fn open_wasm_app_instance_overlay_no_focused_pane_launches_root() {
        let mut h = HostHarness::new();
        assert!(
            h.app.windows[0].focused_pane.is_none(),
            "empty context has no focused pane"
        );

        let (app, snapshot) = load_counter();
        let pane_id = h
            .app
            .open_wasm_app_instance(
                "counter-pane",
                app,
                snapshot,
                std::env::temp_dir(),
                AppPermissions::default(),
                None,
                Some("overlay"),
                Vec::new(),
            )
            .expect("overlay with empty context launches as root pane");

        let root = h.app.windows[0].tree.root.expect("root tile installed");
        assert_eq!(
            h.app.windows[0].focused_pane,
            Some(root),
            "new root pane must be focused"
        );
        assert_eq!(
            h.app.windows[0].tree.tiles.get(root),
            Some(&Tile::Pane(pane_id)),
            "root tile must hold the launched wasm pane"
        );
    }
}
