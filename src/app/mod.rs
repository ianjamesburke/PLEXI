mod canvas_bindings;
mod dispatch;
pub(crate) mod notification_image;
mod sync;

/// Returns true for old auto-generated window names ("Page 3,1", "Context 2")
/// written before windows defaulted to an empty name. Stripped on load so they
/// don't appear as user-given names in the command palette.
fn is_auto_window_name(name: &str) -> bool {
    if let Some(rest) = name.strip_prefix("Page ") {
        let mut parts = rest.splitn(2, ',');
        let x = parts.next().unwrap_or("");
        let y = parts.next().unwrap_or("");
        if !x.is_empty()
            && x.chars().all(|c| c.is_ascii_digit())
            && !y.is_empty()
            && y.chars().all(|c| c.is_ascii_digit())
        {
            return true;
        }
    }
    if let Some(rest) = name.strip_prefix("Context ") {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
    }
    false
}

/// Which layer currently owns keyboard input.
///
/// The top of `PlexiApp.focus_stack` is the active layer. When a non-`Pane`
/// layer is on top, keyboard `Event::Key` and `Event::Text` events are drained
/// from `ctx.input` each frame *after* the owning overlay has rendered, so
/// panes and other passive readers see an empty event buffer. Global
/// keybinds (Cmd+Q, Cmd+W, Cmd+Shift+A) are handled in `keys::poll_actions`
/// which runs before the drain and is always live.
///
/// New overlays should push their layer on open and pop on close to inherit
/// input capture. All keyboard-owning overlays live here: notification modal,
/// confirm-close, command palette, run palette, and rename-pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FocusLayer {
    NotificationModal,
    ConfirmClose,
    CommandPalette,
    RunPalette,
    RenamePane,
    /// Context naming modal shown when a new context is created while the
    /// sidebar is hidden. Mirrors the inline sidebar rename but as a centred
    /// overlay so the terminal is immediately usable after dismissal.
    ContextRename,
}

#[derive(Clone)]
pub(crate) struct PendingNotification {
    pub notify_id: String,
    pub sender_pane_id: u64,
    /// Context index the notification originated from (stamped at drain time).
    pub source_context: usize,
    pub level: String,
    pub title: String,
    pub body: String,
    pub kind: crate::app_protocol::NotifyKind,
    pub options: Vec<crate::app_protocol::NotifyOption>,
    pub input_prompt: Option<String>,
    pub required: bool,
    /// Higher = more urgent. Used to pick next after dismiss + to order
    /// Cmd+]/Cmd+[ preview traversal. Arrival order (index in the queue
    /// Vec) breaks ties — oldest wins.
    pub priority: u32,
    /// Visibility scope. Affects which contexts the notification appears in.
    pub scope: crate::app_protocol::NotifyScope,
    /// Optional inline image attachment (#74). Decoded lazily on first
    /// render; oversized payloads (> 50 KB decoded) surface a placeholder
    /// instead of decoding. The decoded texture is cached separately on
    /// `PlexiApp::notification_images` keyed by `notify_id` — this struct
    /// stays Clone-cheap (no GPU handles inside it).
    pub image_inline: Option<crate::app_protocol::NotificationImage>,
    /// Optional pipe-referenced image attachment (#74). The host drains the
    /// matching binary ring on first render and caches the texture under
    /// `PlexiApp::notification_images`.
    pub image_pipe_id: Option<String>,
    /// Path to a file the CLI polls for the chosen key. Set when the
    /// notification was queued by `plexi notify --choice …`. The host writes
    /// the chosen value here when the user picks an option so the blocking CLI
    /// process can read it and exit.
    pub response_file: Option<String>,
    pub timeout_secs: Option<u64>,
    pub on_dismiss: Option<String>,
    /// When the notification was pushed to the queue. Used for timeout tracking.
    pub enqueued_at: std::time::Instant,
    /// True when the originating app pane has exited. The notification stays
    /// in the queue so the user can read it, but action buttons are hidden.
    pub tombstoned: bool,
}

/// Render state for a notification's image attachment. Computed once and
/// cached on `PlexiApp::notification_images` keyed by `notify_id`.
#[derive(Clone)]
pub(crate) enum NotificationImageState {
    /// Image is ready to draw. `(handle, w, h)`.
    Ready(egui::TextureHandle, u32, u32),
    /// Decoded payload exceeded the 50 KB cap, or decoding failed; render a
    /// placeholder badge with the explanation in `reason`.
    Placeholder { reason: String },
    /// Pipe pending — no frame yet drained from the binary ring. Render
    /// nothing this frame; retry next frame.
    Pending,
}

use crate::app_registry::AppRegistry;
use crate::config;
use crate::context::Window;
use crate::keys::{self, Action};
use crate::pane::{Pane, TerminalPane};
use crate::shell;
use crate::theme::{self, Colors};
use crate::tiling::{PaneId, PlexiBehavior};
use crate::workspace::WorkspaceFile;
use egui::{Color32, CornerRadius, Stroke, StrokeKind, Vec2};
use egui_term::{BackendSettings, PtyEvent, TerminalTheme, TerminalView};
use egui_tiles::{Tile, Tree};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;

struct PaneSwapAnim {
    from: egui::Rect,
    to: egui::Rect,
    started_at: std::time::Instant,
}

struct EdgePulse {
    tile: egui_tiles::TileId,
    dir: crate::keys::Direction,
    started_at: std::time::Instant,
}

pub struct PlexiApp {
    pub(crate) pty_event_rx: mpsc::Receiver<(u64, PtyEvent)>,
    pub(crate) pty_event_tx: mpsc::Sender<(u64, PtyEvent)>,
    pub(crate) last_notify_poll: std::time::Instant,
    pub(crate) theme: TerminalTheme,
    pub(crate) colors: Colors,
    pub(crate) default_font_size: f32,
    pub(crate) ctx: egui::Context,
    pub(crate) router: crate::workspace_router::WorkspaceRouter,
    pub(crate) windows: Vec<Window>,
    pub(crate) active_window: usize,
    pub(crate) sidebar_visible: bool,
    pub(crate) show_shortcuts: bool,
    pub(crate) show_changelog: bool,
    pub(crate) show_cli_setup_prompt: bool,
    pub(crate) quitting: bool,
    pub(crate) quit_press_count: u8,
    pub(crate) quit_last_press: Option<std::time::Instant>,
    pub(crate) pending_close: bool,
    pub(crate) frame_tick: crate::logging::FrameTick,
    /// Cached config so confirmation settings are read through the config
    /// tunnel rather than duplicated as individual bool fields.
    pub(crate) config: crate::config::PlexiConfig,
    pub(crate) renaming_window: Option<usize>,
    pub(crate) rename_buffer: String,
    pub(crate) drag_context: Option<usize>,
    pub(crate) registry: AppRegistry,
    pub(crate) show_command_palette: bool,
    pub(crate) palette_query: String,
    pub(crate) palette_selected: usize,
    pub(crate) context_visit_history: Vec<u64>,
    pub(crate) renaming_pane: Option<PaneId>,
    pub(crate) features: crate::features::FeatureFlags,
    /// Whether the Run palette overlay is visible (Cmd+R).
    pub(crate) show_run_palette: bool,
    /// Notifications queued from apps via ShowNotification.
    pub(crate) pending_notifications: Vec<PendingNotification>,
    /// Whether the centered notification modal is visible. Primary (and only)
    /// surface; auto-shown when a new notification arrives unless
    /// `notifications.focus_mode` is on, in which case the modal only opens
    /// on Cmd+Shift+A.
    pub(crate) show_notification_modal: bool,
    /// ID of the notification the modal is currently showing. `None` means
    /// "modal is empty / closed" — at the next render, the highest-priority
    /// notification in the queue becomes current.
    ///
    /// The invariant: **the currently-displayed notification is pinned by
    /// id**. New notifications arriving never change this, so the user can
    /// never be yanked to a different notification without their input.
    /// Cmd+] / Cmd+[ move this id across the priority-sorted queue.
    pub(crate) current_notify_id: Option<String>,
    /// Focused option index for `kind = "choice"` notifications (0-based).
    /// Reset to 0 whenever the front of the queue changes.
    pub(crate) modal_focused_option: usize,
    /// Buffer for `kind = "input"` notifications.
    pub(crate) modal_input_buffer: String,
    /// notify_id of the notification the modal currently has state for. Used to
    /// detect a front-of-queue change and reset focus/input buffer.
    pub(crate) modal_state_notify_id: String,
    /// Lazily-decoded image-render state for notifications that carry an
    /// `image_inline` or `image_pipe_id` attachment (#74). Keyed by
    /// `notify_id`. Populated on first render of the notification; entries
    /// are NOT explicitly evicted on dismiss — egui's TextureHandle drop
    /// cleanup, plus the small upper bound on concurrent visible
    /// notifications, makes this acceptable. A future PR can add eviction
    /// if memory becomes a concern.
    pub(crate) notification_images: HashMap<String, NotificationImageState>,
    /// Cached from `[notifications]` config. See NotificationsConfig for semantics.
    pub(crate) notifications_enabled: bool,
    pub(crate) notifications_focus_mode: bool,
    /// Minimum priority that may auto-open the modal on arrival. Below this,
    /// notifications enter the queue silently (badge only). Defaults to 100.
    pub(crate) notifications_interrupt_threshold: u32,
    /// Input-focus stack. Top layer receives keyboard input; panes see an
    /// empty event buffer while a non-`Pane` layer is on top. See the
    /// `FocusLayer` docs for the invariant.
    pub(crate) focus_stack: Vec<FocusLayer>,
    pub(crate) host: crate::host::model::HostModel,
    pub(crate) host_services: crate::host::services::HostServices,
    /// Parked background ProcessApps — kept alive when their pane is closed.
    /// Keyed by app type_id. Re-attached by B3 when the app is reopened.
    pub(crate) background_apps: HashMap<String, Box<crate::process_app::ProcessApp>>,
    /// Directed inter-agent / inter-app pipes (#286). Keyed by `pipe_id`,
    /// value is the `(sender_pane_id, target_pane_id)` pair the host must
    /// scope `PipeMessage` deliveries to. `DeliverPipeMessage` consults this
    /// map first: hits route ONLY to the non-sender member of the pair;
    /// misses fall back to the legacy peer-broadcast (`has_reader`) path.
    pub(crate) directed_pipes: HashMap<String, (u64, u64)>,
    /// Hot-reload watcher set (#83). Owns one notify watcher per pane that
    /// opted-in via manifest `[app] watch = true` (workspace-local only).
    /// `hot_reload_rx` is drained each frame; pending requests trigger a
    /// `reload_pane` call which replaces the `ProcessApp` inside the
    /// existing `AppPane` envelope.
    pub(crate) hot_reload: crate::hot_reload::HotReloadWatcher,
    pub(crate) hot_reload_rx: std::sync::mpsc::Receiver<crate::hot_reload::ReloadRequest>,
    /// Spatial-grid minimap overlay state. Controls visibility, fade timer,
    /// and the `Cmd+Shift+M` override-visible flag.
    pub(crate) minimap: crate::minimap::MinimapState,
    /// Per-row navigation history for the spatial page grid. Maps `grid_y`
    /// to the `grid_x` of the last page visited on that row. Vertical moves
    /// consult this to land on the most recently accessed page in the target
    /// row rather than the spatially closest one.
    pub(crate) last_page_x_per_row: HashMap<u32, u32>,
    /// context_id → last active window_id for that context.
    pub(crate) context_active_window: HashMap<u64, u64>,
    /// Per-context minimap visibility. Saved on context switch so each
    /// context remembers its own minimap state across window changes.
    /// Absent entry = first visit; defaults to `true` iff context has > 1 page.
    pub(crate) minimap_visible_per_context: HashMap<u64, bool>,
    /// Monotonically increasing counter. Assigned to each new `Window` as its
    /// stable `window_id`. Never reused — only increments.
    pub(crate) next_window_id: u64,
    /// Pane count from the last snapshot push. Avoids rebuilding the global
    /// pane context every frame when no panes were opened or closed.
    pane_snapshot_len: usize,
    /// In-flight pane swap animations. Each entry fades out over 160 ms.
    pane_anims: Vec<PaneSwapAnim>,
    /// Boundary edge pulse — shown when a swap is attempted at the wall.
    edge_pulse: Option<EdgePulse>,
    /// Channel receiver fed by the background update-check thread. Sends the
    /// latest version string exactly once if a newer release is available.
    update_rx: Option<std::sync::mpsc::Receiver<String>>,
    /// Latest available version string, set after the background check resolves.
    /// `None` means either the check hasn't completed or we're already current.
    pub(crate) update_available: Option<String>,
    /// Receiver for HostCommands sent over the PLEXI_SOCKET Unix socket listener.
    /// Drained each frame in `drain_pane_cmd_channel`.
    pane_ipc_rx: std::sync::mpsc::Receiver<crate::app_protocol::HostCommand>,
}

#[cfg(test)]
fn configure_egui_ctx(ctx: &egui::Context, colors: &Colors) {
    theme::setup_fonts(ctx);
    ctx.set_visuals(egui::Visuals::dark());
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    theme::setup_style(ctx, colors);
}

fn spawn_socket_listener(
    tx: std::sync::mpsc::Sender<crate::app_protocol::HostCommand>,
) {
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;

    let path = crate::config::config_dir().join("notify.sock");
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            log::error!("pane_ipc: failed to bind {:?}: {e}", path);
            return;
        }
    };
    log::info!("pane_ipc: listening on {:?}", path);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { break };
            let tx = tx.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(stream);
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    let line = line.trim().to_owned();
                    if line.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<crate::app_protocol::HostCommand>(&line) {
                        Ok(cmd) => {
                            let _ = tx.send(cmd);
                        }
                        Err(e) => {
                            log::warn!("pane_ipc: parse error: {e}  line={line:?}");
                        }
                    }
                }
            });
        }
    });
}

impl PlexiApp {
    pub fn new(cc: &eframe::CreationContext<'_>, frame_tick: crate::logging::FrameTick) -> Self {
        #[cfg(target_os = "macos")]
        crate::macos_menu::customize_app_menu();

        theme::setup_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);

        // Hot-reload watcher set (#83). Constructed once per host instance.
        // The receiver lives on `self.hot_reload_rx`; `update()` drains it
        // each frame and reloads the matching pane. Both branches of `new()`
        // (workspace-restore and default) use the same instance via shadow
        // names — kept on stack until consumed by `Self {..}`.
        let (hr_watcher, hr_rx) = crate::hot_reload::HotReloadWatcher::new();
        let (hr_watcher2, hr_rx2) = crate::hot_reload::HotReloadWatcher::new();

        // Resolve the active workspace (explicit `plexi <path>` arg, then
        // CWD-walk fallback) and overlay its `.plexi/config.toml` on top of
        // the global config. Project values win on a per-field basis; unset
        // project fields preserve the global value.
        let active_workspace = config::active_workspace_root();
        let config = config::PlexiConfig::load_with_workspace(active_workspace.as_deref());
        let features = crate::features::FeatureFlags::from_config(&config);
        let notifications_enabled = config
            .notifications
            .as_ref()
            .and_then(|n| n.enabled)
            .unwrap_or(true);
        let notifications_focus_mode = config
            .notifications
            .as_ref()
            .and_then(|n| n.focus_mode)
            .unwrap_or(false);
        let notifications_interrupt_threshold = config
            .notifications
            .as_ref()
            .and_then(|n| n.interrupt_threshold)
            .unwrap_or(100); // PRIORITY_HIGH — only HIGH/CRITICAL interrupt by default
        let default_font_size = config.font_size.unwrap_or(theme::FONT_SIZE);
        let theme_cfg = Self::resolve_theme_config(&config);
        let colors = Colors::from_config(&theme_cfg);
        theme::setup_style(&cc.egui_ctx, &colors);

        let (tx, rx) = mpsc::channel();

        let cwd = std::env::current_dir().unwrap_or_default();
        let registry = AppRegistry::load(&cwd);

        // Initialize the event log. Global log goes to ~/.plexi-*/events.jsonl;
        // workspace log goes to .plexi/events.jsonl if we're inside a workspace.
        {
            let global_path = crate::config::config_dir().join("events.jsonl");
            let workspace_path = crate::event_log::find_workspace_events_path(&cwd);
            crate::event_log::init_global(global_path, workspace_path);
        }

        // Spawn background update check. Sends the latest version once if newer.
        let (update_tx, update_rx) = std::sync::mpsc::channel::<String>();
        crate::updater::spawn_update_check(crate::config::config_dir(), update_tx);

        let (pane_ipc_tx, pane_ipc_rx) = std::sync::mpsc::channel::<crate::app_protocol::HostCommand>();
        spawn_socket_listener(pane_ipc_tx);

        // Try to load saved workspace
        if let Some(ws) = WorkspaceFile::load() {
            let mut windows = Vec::new();
            for saved_win in ws.windows {
                let mut panes = HashMap::new();
                for saved_pane in &saved_win.panes {
                    let cwd = if saved_pane.cwd.is_dir() {
                        Some(saved_pane.cwd.clone())
                    } else if saved_win.path.is_dir() {
                        Some(saved_win.path.clone())
                    } else {
                        dirs::home_dir()
                    };
                    let mut pane_entry: Option<Pane> = None;
                    if matches!(saved_pane.kind, crate::workspace::SavedPaneKind::App) {
                        let Some(app_type) = &saved_pane.app_id else {
                            continue;
                        };
                        let app_cwd = saved_pane.cwd.clone();
                        let builtin_perms = crate::app_permissions::AppPermissions::builtin();
                        match app_type.as_str() {
                            "file_browser" => {
                                let mut app =
                                    crate::file_browser::FileBrowserApp::new(app_cwd.clone());
                                if let Some(state) = &saved_pane.app_state {
                                    use crate::app_trait::App;
                                    app.restore_state(state);
                                }
                                pane_entry = Some(Pane::App(Box::new(crate::pane::AppPane {
                                    id: saved_pane.id,
                                    runtime: crate::pane::AppRuntime::Builtin(Box::new(app)),
                                    workspace_root: app_cwd,
                                    permissions: builtin_perms,
                                    manifest_id: "file_browser".to_string(),
                                    name: "File Browser".to_string(),
                                    pane_group: Some("cwd".to_string()),
                                    linked_pane_id: None,
                                    overlay_replaced: None,
                                })));
                            }
                            "quick_note" => {
                                let mut app =
                                    crate::quick_note_app::QuickNoteApp::new(app_cwd.clone());
                                if let Some(state) = &saved_pane.app_state {
                                    use crate::app_trait::App;
                                    app.restore_state(state);
                                }
                                pane_entry = Some(Pane::App(Box::new(crate::pane::AppPane {
                                    id: saved_pane.id,
                                    runtime: crate::pane::AppRuntime::Builtin(Box::new(app)),
                                    workspace_root: app_cwd,
                                    permissions: builtin_perms,
                                    manifest_id: "quick_note".to_string(),
                                    name: "Quick Note".to_string(),
                                    pane_group: None,
                                    linked_pane_id: None,
                                    overlay_replaced: None,
                                })));
                            }
                            "secrets_manager" => {
                                let mut app = crate::secrets_app::SecretsApp::new(app_cwd.clone());
                                if let Some(state) = &saved_pane.app_state {
                                    use crate::app_trait::App;
                                    app.restore_state(state);
                                }
                                pane_entry = Some(Pane::App(Box::new(crate::pane::AppPane {
                                    id: saved_pane.id,
                                    runtime: crate::pane::AppRuntime::Builtin(Box::new(app)),
                                    workspace_root: app_cwd,
                                    permissions: builtin_perms,
                                    manifest_id: "secrets_manager".to_string(),
                                    name: "Secrets Manager".to_string(),
                                    pane_group: None,
                                    linked_pane_id: None,
                                    overlay_replaced: None,
                                })));
                            }
                            other => {
                                if let Some(process) = registry.launch_process(other, &app_cwd, &[])
                                {
                                    pane_entry = Some(Pane::App(Box::new(crate::pane::AppPane {
                                        id: saved_pane.id,
                                        permissions: process.permissions.clone(),
                                        runtime: crate::pane::AppRuntime::Process(Box::new(
                                            process,
                                        )),
                                        workspace_root: app_cwd,
                                        manifest_id: other.to_string(),
                                        name: other.to_string(),
                                        pane_group: registry.group_for(other),
                                        linked_pane_id: None,
                                        overlay_replaced: None,
                                    })));
                                }
                            }
                        }
                    }

                    if pane_entry.is_none() {
                        let settings = Self::make_backend_settings(saved_pane.id, cwd, &colors);
                        if let Some(mut pane) = TerminalPane::new(
                            saved_pane.id,
                            cc.egui_ctx.clone(),
                            tx.clone(),
                            settings,
                            default_font_size,
                        ) {
                            pane.name = saved_pane.name.clone();
                            pane_entry = Some(Pane::Terminal(Box::new(pane)));
                        }
                    }

                    if let Some(pane) = pane_entry {
                        panes.insert(saved_pane.id, pane);
                    }
                }
                // Skip only if there were saved panes that all failed to restore
                // (corrupted state). An empty saved pane list means the user
                // intentionally closed all panes — restore the empty window so
                // the welcome screen appears on next launch.
                if !saved_win.panes.is_empty() && panes.is_empty() {
                    continue;
                }
                windows.push(Window {
                    // Strip old auto-generated "Page X,Y" names — treat them as
                    // unnamed so they don't pollute the command palette.
                    name: if is_auto_window_name(&saved_win.name) {
                        String::new()
                    } else {
                        saved_win.name
                    },
                    path: saved_win.path,
                    tree: saved_win.tree,
                    panes,
                    focused_pane: saved_win.focused_pane,
                    zoomed_pane: None,
                    grid_x: saved_win.grid_x,
                    grid_y: saved_win.grid_y,
                    window_id: saved_win.window_id,
                    context_id: saved_win.context_id,
                });
            }
            if !windows.is_empty() {
                // Repair window_ids: old workspace files have window_id=0.
                // Assign sequential IDs so every window has a stable unique ID.
                let mut next_id: u64 = 1;
                for win in &mut windows {
                    if win.window_id == 0 {
                        win.window_id = next_id;
                        next_id += 1;
                    } else {
                        next_id = next_id.max(win.window_id + 1);
                    }
                }
                let mut contexts = Vec::new();
                for saved_ctx in ws.contexts {
                    contexts.push(crate::context::Context {
                        name: saved_ctx.name,
                        path: saved_ctx.path,
                        context_id: saved_ctx.context_id,
                    });
                }
                let active_ctx = ws.active_context.min(contexts.len().saturating_sub(1));
                let active_ctx_id = contexts[active_ctx].context_id;
                let active = ws.context_active_window.get(&active_ctx_id)
                    .and_then(|win_id| windows.iter().position(|w| w.window_id == *win_id))
                    .unwrap_or(0);
                let window_count = windows.iter().filter(|w| w.context_id == active_ctx_id).count();
                let mut host = crate::host::model::HostModel::new();
                host.seed_next_pane_id(ws.next_pane_id);
                return Self {
                    pty_event_rx: rx,
                    pty_event_tx: tx,
                    theme: theme::terminal_theme(&theme_cfg),
                    colors,
                    default_font_size,
                    ctx: cc.egui_ctx.clone(),
                    router: crate::workspace_router::WorkspaceRouter::new(contexts, active_ctx),
                    windows,
                    active_window: active,
                    sidebar_visible: ws.sidebar_visible,
                    show_shortcuts: false,
                    show_changelog: false,
                    show_cli_setup_prompt: crate::cli_setup::should_prompt(),
                    quitting: false,
                    quit_press_count: 0,
                    quit_last_press: None,
                    config: config.clone(),
                    pending_close: false,
                    frame_tick: frame_tick.clone(),
                    renaming_window: None,
                    rename_buffer: String::new(),
                    drag_context: None,
                    show_command_palette: false,
                    palette_query: String::new(),
                    palette_selected: 0,
                    context_visit_history: Vec::new(),
                    renaming_pane: None,
                    registry,
                    features: features.clone(),
                    show_run_palette: false,
                    pending_notifications: Vec::new(),
                    show_notification_modal: false,
                    current_notify_id: None,
                    modal_focused_option: 0,
                    modal_input_buffer: String::new(),
                    modal_state_notify_id: String::new(),
                    notification_images: HashMap::new(),
                    notifications_enabled,
                    notifications_focus_mode,
                    notifications_interrupt_threshold,
                    focus_stack: Vec::new(),
                    last_notify_poll: std::time::Instant::now(),
                    host,
                    host_services: crate::host::services::HostServices::new(),
                    background_apps: HashMap::new(),
                    directed_pipes: HashMap::new(),
                    hot_reload: hr_watcher,
                    hot_reload_rx: hr_rx,
                    minimap: crate::minimap::MinimapState::with_visible(window_count >= 2),
                    last_page_x_per_row: HashMap::new(),
                    context_active_window: ws.context_active_window,
                    minimap_visible_per_context: HashMap::new(),
                    next_window_id: next_id,
                    pane_snapshot_len: 0,
                    pane_anims: Vec::new(),
                    edge_pulse: None,
                    update_rx: Some(update_rx),
                    update_available: None,
                    pane_ipc_rx,
                };
            }
        }

        // Default: empty context — welcome screen is shown until the user creates a pane.
        let panes: HashMap<u64, Pane> = HashMap::new();
        let tree = Tree::empty("plexi");

        let path = std::env::current_dir()
            .unwrap_or_else(|_| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));

        Self {
            pty_event_rx: rx,
            pty_event_tx: tx,
            theme: theme::terminal_theme(&theme_cfg),
            colors,
            default_font_size,
            ctx: cc.egui_ctx.clone(),
            router: crate::workspace_router::WorkspaceRouter::new(
                vec![crate::context::Context {
                    name: "Default".into(),
                    path: path.clone(),
                    context_id: 1,
                }],
                0,
            ),
            windows: vec![Window {
                name: "Default".into(),
                path,
                tree,
                panes,
                focused_pane: None,
                zoomed_pane: None,
                grid_x: 0,
                grid_y: 0,
                window_id: 1,
                context_id: 1,
            }],
            active_window: 0,
            sidebar_visible: true,
            show_shortcuts: false,
            show_changelog: false,
            show_cli_setup_prompt: !crate::cli_setup::was_prompted()
                && !crate::cli_setup::is_installed(),
            quitting: false,
            quit_press_count: 0,
            quit_last_press: None,
            config,
            pending_close: false,
            frame_tick,
            renaming_window: None,
            rename_buffer: String::new(),
            drag_context: None,
            show_command_palette: false,
            palette_query: String::new(),
            palette_selected: 0,
            context_visit_history: Vec::new(),
            renaming_pane: None,
            registry: AppRegistry::load(&std::env::current_dir().unwrap_or_default()),
            features,
            show_run_palette: false,
            pending_notifications: Vec::new(),
            show_notification_modal: false,
            current_notify_id: None,
            modal_focused_option: 0,
            modal_input_buffer: String::new(),
            modal_state_notify_id: String::new(),
            notification_images: HashMap::new(),
            notifications_enabled,
            notifications_focus_mode,
            notifications_interrupt_threshold,
            focus_stack: Vec::new(),
            last_notify_poll: std::time::Instant::now(),
            host: crate::host::model::HostModel::new(),
            host_services: crate::host::services::HostServices::new(),
            background_apps: HashMap::new(),
            directed_pipes: HashMap::new(),
            hot_reload: hr_watcher2,
            hot_reload_rx: hr_rx2,
            minimap: crate::minimap::MinimapState::new(),
            last_page_x_per_row: HashMap::new(),
            context_active_window: HashMap::new(),
            minimap_visible_per_context: HashMap::new(),
            next_window_id: 2,
            pane_snapshot_len: 0,
            pane_anims: Vec::new(),
            edge_pulse: None,
            update_rx: Some(update_rx),
            update_available: None,
            pane_ipc_rx,
        }
    }

    /// Create a `PlexiApp` for headless tests. No workspace restore, no macOS
    /// menu setup, no PTY or audio hardware. Initialises a single empty window
    /// so `state().open_panes` is empty and the harness can add panes via
    /// `inject_app_pane`.
    #[cfg(test)]
    pub fn new_for_test(
        ctx: egui::Context,
        frame_tick: crate::logging::FrameTick,
    ) -> (Self, std::sync::mpsc::Sender<crate::app_protocol::HostCommand>) {
        let config = config::PlexiConfig::default();
        let theme_cfg = Self::resolve_theme_config(&config);
        let colors = Colors::from_config(&theme_cfg);
        configure_egui_ctx(&ctx, &colors);
        let (tx, rx) = mpsc::channel();
        let (hr_watcher, hr_rx) = crate::hot_reload::HotReloadWatcher::new();
        let path = std::env::temp_dir();
        let features = crate::features::FeatureFlags::from_config(&config);
        let (pane_ipc_tx, pane_ipc_rx) = std::sync::mpsc::channel::<crate::app_protocol::HostCommand>();
        (Self {
            pty_event_rx: rx,
            pty_event_tx: tx,
            last_notify_poll: std::time::Instant::now(),
            theme: theme::terminal_theme(&theme_cfg),
            colors,
            default_font_size: theme::FONT_SIZE,
            ctx: ctx.clone(),
            router: crate::workspace_router::WorkspaceRouter::new(
                vec![crate::context::Context {
                    name: "Test".into(),
                    path: path.clone(),
                    context_id: 1,
                }],
                0,
            ),
            windows: vec![Window {
                name: "Test".into(),
                path: path.clone(),
                tree: egui_tiles::Tree::empty("test_tree"),
                panes: HashMap::new(),
                focused_pane: None,
                zoomed_pane: None,
                grid_x: 0,
                grid_y: 0,
                window_id: 1,
                context_id: 1,
            }],
            active_window: 0,
            sidebar_visible: false,
            show_shortcuts: false,
            show_changelog: false,
            quitting: false,
            quit_press_count: 0,
            quit_last_press: None,
            config,
            pending_close: false,
            frame_tick,
            renaming_window: None,
            rename_buffer: String::new(),
            drag_context: None,
            show_command_palette: false,
            palette_query: String::new(),
            palette_selected: 0,
            context_visit_history: Vec::new(),
            renaming_pane: None,
            registry: AppRegistry::load_with_global(
                &path,
                &path.join("nonexistent-apps-dir"),
            ),
            features,
            show_run_palette: false,
            pending_notifications: Vec::new(),
            show_notification_modal: false,
            current_notify_id: None,
            modal_focused_option: 0,
            modal_input_buffer: String::new(),
            modal_state_notify_id: String::new(),
            notification_images: HashMap::new(),
            notifications_enabled: false,
            notifications_focus_mode: false,
            notifications_interrupt_threshold: 100,
            focus_stack: Vec::new(),
            host: crate::host::model::HostModel::new(),
            host_services: crate::host::services::HostServices::new(),
            background_apps: HashMap::new(),
            directed_pipes: HashMap::new(),
            hot_reload: hr_watcher,
            hot_reload_rx: hr_rx,
            minimap: crate::minimap::MinimapState::new(),
            last_page_x_per_row: HashMap::new(),
            context_active_window: HashMap::new(),
            minimap_visible_per_context: HashMap::new(),
            next_window_id: 2,
            pane_snapshot_len: 0,
            pane_anims: Vec::new(),
            edge_pulse: None,
            show_cli_setup_prompt: false,
            update_rx: None,
            update_available: None,
            pane_ipc_rx,
        }, pane_ipc_tx)
    }

    fn resolve_theme_config(config: &config::PlexiConfig) -> config::ThemeConfig {
        let user_theme = config.theme.clone().unwrap_or_default();
        if let Some(preset_name) = &config.theme_preset {
            if let Some(preset) = theme::preset_colors(preset_name) {
                log::info!("Applying theme preset: {}", preset_name.trim());
                return theme::apply_preset(&preset, &user_theme);
            }
            log::warn!("Unknown theme preset: {preset_name}");
        }
        user_theme
    }

    pub(crate) fn make_backend_settings(
        pane_id: u64,
        working_directory: Option<PathBuf>,
        colors: &Colors,
    ) -> BackendSettings {
        let mut env = shell::build_env();
        env.insert("PLEXI_PANE_ID".into(), pane_id.to_string());
        let socket = crate::config::config_dir()
            .join("notify.sock")
            .to_string_lossy()
            .into_owned();
        env.insert("PLEXI_SOCKET".into(), socket);
        BackendSettings {
            shell: shell::detect_shell(),
            args: vec!["-l".to_string()],
            env,
            dynamic_colors: theme::terminal_dynamic_colors(colors),
            working_directory,
        }
    }

    fn drain_notify_queue(&mut self) {
        let queue_dir = crate::config::config_dir().join("notify-queue");
        let Ok(entries) = std::fs::read_dir(&queue_dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            let _ = std::fs::remove_file(&path);
            let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) else { continue };
            if !self.notifications_enabled {
                continue;
            }
            let level = val["level"].as_str().unwrap_or("info").to_string();
            let title = val["title"].as_str().unwrap_or("").to_string();
            let body = val["body"].as_str().unwrap_or("").to_string();
            let choices_json = val["choices"].as_array();
            let options: Vec<crate::app_protocol::NotifyOption> = choices_json
                .map(|arr| {
                    arr.iter()
                        .map(|item| crate::app_protocol::NotifyOption {
                            label: item["label"].as_str().unwrap_or("").to_string(),
                            value: item["key"].as_str().unwrap_or("").to_string(),
                            shortcut: Some(
                                item["key"].as_str().unwrap_or("").to_string(),
                            ),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let kind = if options.is_empty() {
                crate::app_protocol::NotifyKind::Message
            } else {
                crate::app_protocol::NotifyKind::Choice
            };
            let response_file = val["response_file"].as_str().map(|s| s.to_string());
            log::info!(
                "notify:drain: title={:?} choices={} response_file={:?}",
                title, options.len(), response_file
            );
            let internal_id = format!(
                "__host__:{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            );
            self.pending_notifications.push(PendingNotification {
                notify_id: internal_id.clone(),
                sender_pane_id: 0,
                // Host-originated notifications are global (they originate
                // outside any context) and always visible.
                source_context: self.router.active_idx(),
                scope: crate::app_protocol::NotifyScope::Global,
                level,
                title,
                body,
                kind,
                options,
                input_prompt: None,
                required: false,
                priority: 0,
                image_inline: None,
                image_pipe_id: None,
                response_file,
                timeout_secs: None,
                on_dismiss: None,
                enqueued_at: std::time::Instant::now(),
                tombstoned: false,
            });
            // Host-originated notifications are always LOW (priority 0) —
            // below any reasonable interrupt threshold — so they queue
            // silently by default. They still ride focus_mode as a hard
            // gate for consistency.
            let should_auto_open = !self.notifications_focus_mode
                && 0 >= self.notifications_interrupt_threshold;
            if should_auto_open {
                self.show_notification_modal = true;
                if self.current_notify_id.is_none() {
                    self.current_notify_id = Some(internal_id);
                }
            }
        }
    }

    fn drain_pane_cmd_channel(&mut self) {
        while let Ok(cmd) = self.pane_ipc_rx.try_recv() {
            match &cmd {
                crate::app_protocol::HostCommand::SetPaneTitle { pane_id, name } => {
                    log::info!("pane_ipc: kind=set_pane_title pane_id={pane_id}");
                    let mut found = false;
                    for win in &mut self.windows {
                        if let Some(pane) = win.panes.get_mut(pane_id) {
                            if let Some(t) = pane.as_terminal_mut() {
                                t.name = Some(name.clone());
                                found = true;
                                break;
                            }
                        }
                    }
                    if !found {
                        log::warn!("pane_ipc: set_pane_title: pane_id={pane_id} not found");
                    }
                }
                crate::app_protocol::HostCommand::SpawnPane { type_id, layout, args, .. } => {
                    log::info!("pane_ipc: kind=spawn_pane type_id={type_id}");
                    self.launch_app_by_id_with_layout(type_id, Some(layout.clone()), args);
                }
                _ => {
                    log::warn!("pane_ipc: unsupported command kind, dropping");
                }
            }
        }
    }

    fn drain_spawn_queue(&mut self) {
        let queue_dir = crate::config::config_dir().join("spawn-queue");
        let Ok(entries) = std::fs::read_dir(&queue_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let _ = std::fs::remove_file(&path);
            let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) else {
                continue;
            };
            let type_id = val["type_id"].as_str().unwrap_or("").to_string();
            if type_id.is_empty() {
                log::warn!("spawn-queue: entry missing type_id, skipping");
                continue;
            }
            let layout = val["layout"].as_str().map(|s| s.to_string());
            let args: Vec<String> = val["args"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            log::info!("spawn-queue: launching '{type_id}' layout={layout:?}");
            self.launch_app_by_id_with_layout(&type_id, layout, &args);
        }
    }

    fn drain_pty_events(&mut self) {
        let mut panes_to_close: Vec<u64> = Vec::new();

        while let Ok((id, event)) = self.pty_event_rx.try_recv() {
            match &event {
                PtyEvent::Exit => {
                    for win in &mut self.windows {
                        if let Some(pane) = win.panes.get_mut(&id) {
                            if let Some(t) = pane.as_terminal_mut() {
                                t.exited = true;
                            }
                            break;
                        }
                    }
                }
                PtyEvent::Title(title) => {
                    if let Some(cmd) = title.strip_prefix("plexi:") {
                        match cmd {
                            "close" => panes_to_close.push(id),
                            _ => log::debug!("unknown plexi command: {}", cmd),
                        }
                    }
                }
                _ => {}
            }
        }

        for pane_id in panes_to_close {
            self.close_pane_by_id(pane_id);
        }
    }

    // ── Notification-queue helpers ──────────────────────────────────────────
    //
    // The notification modal tracks the currently-displayed entry by
    // `notify_id`, not by index. These helpers centralise the
    // priority-sort / selection logic so callers can't accidentally reach
    // past the end of the Vec or pick by stale offset.
    //
    // Sort order: `priority DESC, arrival-index ASC`. Arrival index = the
    // entry's current position in `pending_notifications`, which reflects
    // push order (we never reorder the Vec; dismissal removes by id).
    //
    // Visibility: Context-scoped notifications are only visible when
    // `source_context == self.router.active_idx()`. Global notifications are
    // always visible. The raw `pending_notifications` Vec stays flat;
    // only the *view* changes with the active workspace.

    /// True when this notification should appear in the current workspace view.
    pub(crate) fn notification_is_visible(&self, n: &PendingNotification) -> bool {
        matches!(n.scope, crate::app_protocol::NotifyScope::Global)
            || n.source_context == self.router.active_idx()
    }

    /// Return ids of all *visible* notifications (for the current context),
    /// ordered by (required desc, priority desc, arrival asc). Empty Vec when none visible.
    pub(crate) fn sorted_notification_ids(&self) -> Vec<String> {
        let mut indexed: Vec<(usize, u32, bool, &str)> = self
            .pending_notifications
            .iter()
            .enumerate()
            .filter(|(_, n)| self.notification_is_visible(n))
            .map(|(i, n)| (i, n.priority, n.required, n.notify_id.as_str()))
            .collect();
        // required pins to top, then priority DESC, ties broken by arrival ASC.
        indexed.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then(b.1.cmp(&a.1))
                .then(a.0.cmp(&b.0))
        });
        indexed.into_iter().map(|(_, _, _, id)| id.to_string()).collect()
    }

    /// Return the id of the highest-priority *visible* notification,
    /// breaking ties by oldest arrival. `None` when none visible.
    pub(crate) fn select_highest_priority(&self) -> Option<String> {
        self.sorted_notification_ids().into_iter().next()
    }

    /// (1-based position-in-sort-order, total visible len) for the current
    /// notify id, or `None` when modal is empty / current id is missing from
    /// visible queue. Renderer uses this for the "X of N" indicator.
    pub(crate) fn position_of_current(&self) -> Option<(usize, usize)> {
        let current = self.current_notify_id.as_ref()?;
        let sorted = self.sorted_notification_ids();
        let pos = sorted.iter().position(|id| id == current)?;
        Some((pos + 1, sorted.len()))
    }

    /// Move `current_notify_id` forward (`direction = 1`) or backward
    /// (`direction = -1`) through the visible priority-sorted queue. No wrap
    /// at the ends. Called by Cmd+] / Cmd+[.
    pub(crate) fn cycle_notification(&mut self, direction: i32) {
        if !self.show_notification_modal {
            return;
        }
        let sorted = self.sorted_notification_ids();
        if sorted.is_empty() {
            return;
        }
        let Some(current) = self.current_notify_id.as_ref() else {
            // Queue has entries but nothing is current — pick highest.
            self.current_notify_id = sorted.into_iter().next();
            return;
        };
        let Some(pos) = sorted.iter().position(|id| id == current) else {
            // Current id not in visible queue any more (context switch or dismiss).
            // Fall back to highest-priority visible.
            self.current_notify_id = sorted.into_iter().next();
            return;
        };
        let next_pos = match direction {
            d if d > 0 && pos + 1 < sorted.len() => pos + 1,
            d if d < 0 && pos > 0 => pos - 1,
            _ => return, // clamp at both ends
        };
        self.current_notify_id = Some(sorted[next_pos].clone());
    }

    /// Check every pending notification for expiry. For each that has exceeded
    /// its `timeout_secs`, deliver a `NotifyAction` dismiss event and remove it.
    /// Called once per second from `update()`.
    pub(crate) fn tick_notification_timeouts(&mut self) {
        let mut expired_ids: Vec<String> = Vec::new();
        for n in &self.pending_notifications {
            if let Some(timeout) = n.timeout_secs {
                if n.enqueued_at.elapsed() >= std::time::Duration::from_secs(timeout) {
                    expired_ids.push(n.notify_id.clone());
                }
            }
        }
        for id in expired_ids {
            let Some(pos) = self.pending_notifications.iter().position(|n| n.notify_id == id) else {
                continue;
            };
            let n = self.pending_notifications.remove(pos);
            let dismiss_value = n.on_dismiss.clone().unwrap_or_else(|| "timeout".to_string());
            log::info!(
                "notification '{}' timed out after {}s — delivering on_dismiss='{}'",
                n.title,
                n.timeout_secs.unwrap_or(0),
                dismiss_value
            );
            if !n.notify_id.is_empty() && !n.notify_id.starts_with("__host__:") {
                let cmds = vec![crate::app_trait::AppCommand::DeliverNotifyAction {
                    pane_id: n.sender_pane_id,
                    notify_id: n.notify_id.clone(),
                    action_label: "timeout".to_string(),
                    value: Some(dismiss_value),
                    response_file: n.response_file.clone(),
                }];
                self.dispatch_notify_action_cmds(cmds);
            }
            // If this was the pinned notification, clear it so the next highest
            // becomes current on the next frame.
            if self.current_notify_id.as_deref() == Some(&n.notify_id) {
                self.current_notify_id = None;
            }
        }
    }

    /// Mark all pending notifications from `pane_id` as tombstoned. Called
    /// when an app pane is closed. Tombstoned notifications remain in the queue
    /// so the user can read them, but their action buttons are hidden.
    pub(crate) fn tombstone_pane_notifications(&mut self, pane_id: crate::tiling::PaneId) {
        for n in &mut self.pending_notifications {
            if n.sender_pane_id == pane_id {
                n.tombstoned = true;
                log::info!("notification '{}' tombstoned (pane {pane_id} closed)", n.title);
            }
        }
    }

    /// Count of context-scoped notifications whose source_context == ctx_idx.
    /// Used for per-context sidebar badges on inactive contexts.
    pub(crate) fn context_notification_count(&self, ctx_idx: usize) -> usize {
        self.pending_notifications
            .iter()
            .filter(|n| {
                matches!(n.scope, crate::app_protocol::NotifyScope::Context)
                    && n.source_context == ctx_idx
            })
            .count()
    }

    /// Count of visible notifications for the active context (context-scoped
    /// from active + all globals). Used for the toolbar badge.
    pub(crate) fn visible_notification_count(&self) -> usize {
        self.pending_notifications
            .iter()
            .filter(|n| self.notification_is_visible(n))
            .count()
    }

}

impl eframe::App for PlexiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_tick.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let _frame_start = std::time::Instant::now();
        if self.last_notify_poll.elapsed() >= std::time::Duration::from_secs(1) {
            self.last_notify_poll = std::time::Instant::now();
            self.drain_notify_queue();
            self.drain_spawn_queue();
            self.tick_notification_timeouts();
        }
        self.drain_pane_cmd_channel();
        if let Some(rx) = &self.update_rx {
            if let Ok(version) = rx.try_recv() {
                log::info!("update check: badge set to v{version}");
                self.update_available = Some(version);
            }
        }
        self.drain_pty_events();

        // Update the global pane context snapshot so that AiQuery dispatches
        // include all open panes in the workspace context (#396).
        self.update_pane_context_snapshot();

        // Focus stack: reconcile layer state BEFORE any input routing so
        // `input_captured_by_overlay()` answers correctly this frame.
        self.sync_notification_modal_focus();
        self.sync_confirm_close_focus();
        self.sync_command_palette_focus();
        self.sync_run_palette_focus();
        self.sync_rename_pane_focus();
        self.sync_context_rename_focus();

        // If an overlay owns input, render it FIRST so its widgets (the
        // notification modal's TextEdit for the `input` kind, the palette's
        // search field, the rename input) can read keystrokes before we
        // drain. Then drain the keyboard buffer so downstream readers —
        // focused app (`dispatch_app_key_events`), terminal backends,
        // `keys::poll_actions` — see only the global allowlist (Cmd+Q,
        // Cmd+W, Cmd+Shift+A, Cmd+]/Cmd+[).
        let mut early_modal_cmds: Vec<crate::app_trait::AppCommand> = Vec::new();
        if self.input_captured_by_overlay() {
            match self.focus_stack.last() {
                Some(FocusLayer::NotificationModal) => {
                    early_modal_cmds = self.draw_notification_modal(ctx);
                }
                Some(FocusLayer::ConfirmClose) => {
                    self.draw_confirm_close(ctx);
                }
                Some(FocusLayer::CommandPalette) => {
                    self.draw_command_palette(ctx);
                }
                Some(FocusLayer::RunPalette) => {
                    self.draw_run_palette(ctx);
                }
                Some(FocusLayer::RenamePane) => {
                    self.draw_rename_pane_overlay(ctx);
                }
                Some(FocusLayer::ContextRename) => {
                    self.draw_rename_context_overlay(ctx);
                }
                None => {}
            }
            self.drain_captured_keyboard_input(ctx);
            // The overlay may have self-closed (notification queue drained,
            // confirm-close confirmed/cancelled, palette picked an entry,
            // rename committed). Re-sync so the layer is accurate for the
            // rest of this frame.
            self.sync_notification_modal_focus();
            self.sync_confirm_close_focus();
            self.sync_command_palette_focus();
            self.sync_run_palette_focus();
            self.sync_rename_pane_focus();
            self.sync_context_rename_focus();
        }

        // Apps only receive key input if nothing is capturing above them.
        // (Key input is focus-scoped; command drain below is not.)
        if !self.input_captured_by_overlay() {
            self.dispatch_app_key_events(ctx);
        }
        // Drain every app pane's pending_commands every frame — including
        // while a modal holds focus. Background apps emitting notifications
        // must reach the queue *now*, not be buffered until the modal
        // closes (which caused the "ghost queue appears on reopen" bug).
        let deferred_app_cmds = self.drain_all_app_commands();
        self.sync_app_cwd();

        // Dispatch any DeliverNotifyAction commands the early modal render
        // produced. Routes back to the originating pane as NotifyAction events.
        self.dispatch_notify_action_cmds(early_modal_cmds);

        // Handle deferred app commands returned from dispatch_app_key_events.
        for cmd in deferred_app_cmds {
            use crate::app_trait::AppCommand;
            match cmd {
                AppCommand::SpawnApp {
                    type_id,
                    layout,
                    args,
                } => {
                    // Capture requesting pane before launch changes focused_pane.
                    let active = self.active_window;
                    let requesting_pane_id = self.windows[active]
                        .focused_pane
                        .and_then(|tile| self.windows[active].tree.tiles.get(tile))
                        .and_then(|tile| {
                            if let egui_tiles::Tile::Pane(pid) = tile {
                                Some(*pid)
                            } else {
                                None
                            }
                        });

                    let new_pane_id = self.host.next_pane_id();
                    self.launch_app_by_id_with_layout(&type_id, layout, &args);

                    // Confirm back to the requesting app.
                    if let Some(req_pane_id) = requesting_pane_id {
                        let active = self.active_window;
                        if let Some(pane) = self.windows[active].panes.get_mut(&req_pane_id) {
                            let event = crate::app_protocol::PlexiEvent::AppSpawned {
                                pane_id: new_pane_id,
                                type_id: type_id.clone(),
                            };
                            if let Some(a) = pane.as_app_mut() {
                                a.runtime.queue_outbound_event(event);
                            }
                        }
                    }
                }
                AppCommand::SpawnPane {
                    type_id,
                    layout,
                    args,
                    pipe_id,
                } => {
                    // "background" layout is not yet implemented (blocked on #291).
                    if layout == "background" {
                        let active = self.active_window;
                        let requesting_pane_id = self.windows[active]
                            .focused_pane
                            .and_then(|tile| self.windows[active].tree.tiles.get(tile))
                            .and_then(|tile| {
                                if let egui_tiles::Tile::Pane(pid) = tile {
                                    Some(*pid)
                                } else {
                                    None
                                }
                            });
                        if let Some(req_pane_id) = requesting_pane_id {
                            let active = self.active_window;
                            if let Some(pane) = self.windows[active].panes.get_mut(&req_pane_id) {
                                if let Some(a) = pane.as_app_mut() {
                                    a.runtime.queue_outbound_event(
                                        crate::app_protocol::PlexiEvent::PaneSpawnError {
                                            reason: "layout 'background' not yet implemented".to_string(),
                                        },
                                    );
                                }
                            }
                        }
                        log::warn!("SpawnPane: layout='background' not implemented, rejected");
                        continue;
                    }

                    // If pipe_id is set, append --pipe=<id> to args so the spawned app knows
                    // which pipe_id to send its result on.
                    let mut effective_args = args;
                    if let Some(ref pid) = pipe_id {
                        effective_args.push(format!("--pipe={pid}"));
                    }

                    // Predict the pane id that will be allocated (next_pane_id peeks without allocating).
                    let active = self.active_window;
                    let requesting_pane_id = self.windows[active]
                        .focused_pane
                        .and_then(|tile| self.windows[active].tree.tiles.get(tile))
                        .and_then(|tile| {
                            if let egui_tiles::Tile::Pane(pid) = tile {
                                Some(*pid)
                            } else {
                                None
                            }
                        });
                    let new_pane_id = self.host.next_pane_id();
                    if type_id == "terminal" {
                        // "terminal" is a builtin pane type, not in the app registry.
                        // split_focused uses inverted LinearDir vs split_with_new_pane:
                        //   split_focused(false) → insert_horizontal_tile → side-by-side (RIGHT)
                        //   split_focused(true)  → insert_vertical_tile   → stacked (BELOW)
                        // So: split_v (right) → false, split_h/split_above (below) → true.
                        let vertical = matches!(layout.as_str(), "split_h" | "split_above");
                        log::info!(
                            "SpawnPane: terminal layout='{layout}' vertical={vertical} pane_id={new_pane_id}"
                        );
                        self.split_focused(vertical);
                    } else {
                        self.launch_app_by_id_with_layout(&type_id, Some(layout), &effective_args);
                        log::info!("SpawnPane: launched '{type_id}' pane_id={new_pane_id}");
                    }

                    // Send PaneSpawned back to the requesting pane.
                    if let Some(req_pane_id) = requesting_pane_id {
                        let active = self.active_window;
                        if let Some(pane) = self.windows[active].panes.get_mut(&req_pane_id) {
                            if let Some(a) = pane.as_app_mut() {
                                a.runtime.queue_outbound_event(
                                    crate::app_protocol::PlexiEvent::PaneSpawned {
                                        pane_id: new_pane_id,
                                    },
                                );
                            }
                        }
                    }
                }
                AppCommand::CdRequest { cwd, sender_pane_id } => {
                    let active = self.active_window;
                    let escaped = cwd.replace('\'', "'\\''");
                    let cd_cmd = format!("cd '{}'\n", escaped);
                    let linked_id = self.windows[active]
                        .panes
                        .get(&sender_pane_id)
                        .and_then(|p| p.as_app())
                        .and_then(|a| a.linked_pane_id);
                    if let Some(tid) = linked_id {
                        if let Some(t) = self.windows[active]
                            .panes
                            .get_mut(&tid)
                            .and_then(|p| p.as_terminal_mut())
                        {
                            t.backend.process_command(egui_term::BackendCommand::Write(
                                cd_cmd.as_bytes().to_vec(),
                            ));
                        }
                    }
                }
                AppCommand::Notify(_) => {}
                AppCommand::ShowNotification {
                    notify_id,
                    sender_pane_id,
                    source_context,
                    level,
                    title,
                    body,
                    kind,
                    options,
                    input_prompt,
                    required,
                    priority,
                    scope,
                    image_inline,
                    image_pipe_id,
                    timeout_secs,
                    on_dismiss,
                } => {
                    if !self.notifications_enabled {
                        // Silently drop — master switch off.
                        continue;
                    }
                    let new_id = notify_id.clone();
                    // Capture scope/source_context before they move into the struct.
                    let is_global = matches!(scope, crate::app_protocol::NotifyScope::Global);
                    let notif_source_ctx = source_context;
                    self.pending_notifications.push(PendingNotification {
                        notify_id,
                        sender_pane_id,
                        source_context,
                        level,
                        title,
                        body,
                        kind,
                        options,
                        input_prompt,
                        required,
                        priority,
                        scope,
                        image_inline,
                        image_pipe_id,
                        response_file: None,
                        timeout_secs,
                        on_dismiss,
                        enqueued_at: std::time::Instant::now(),
                        tombstoned: false,
                    });
                    // Auto-open rules:
                    //   1. Visibility (Global or in active context) — else
                    //      it stays invisible in the queue until the user
                    //      switches to its context.
                    //   2. focus_mode off — the global mute gate.
                    //   3. priority >= interrupt_threshold — don't
                    //      auto-open low-urgency notifications like
                    //      "note saved".
                    // If any gate fails, the notification still queues
                    // (badge ticks) but the modal doesn't pop.
                    let is_visible = is_global || notif_source_ctx == self.active_window;
                    let should_auto_open = is_visible
                        && !self.notifications_focus_mode
                        && priority >= self.notifications_interrupt_threshold;
                    if should_auto_open {
                        self.show_notification_modal = true;
                    }
                    // Only set the new notification as current if nothing is
                    // already pinned AND the new notification is visible AND
                    // it would auto-open. Low-priority passive notifications
                    // shouldn't become the pinned front-most until the user
                    // actually opens the modal (then the highest-priority
                    // remaining is picked at modal-open time).
                    if self.current_notify_id.is_none() && should_auto_open {
                        self.current_notify_id = Some(new_id);
                    }
                }
                AppCommand::DeliverNotifyAction { pane_id, notify_id, action_label, value, response_file } => {
                    log::info!(
                        "notify:action: pane_id={pane_id} notify_id={notify_id:?} value={value:?}"
                    );
                    if let Some(rf) = &response_file {
                        let content = value.as_deref().unwrap_or("");
                        match std::fs::write(rf, content) {
                            Ok(_) => log::info!("notify:action: wrote {:?} to {:?}", content, rf),
                            Err(e) => log::warn!("notify:action: failed to write response file {:?}: {e}", rf),
                        }
                    }
                    let active = self.active_window;
                    if let Some(pane) = self.windows[active].panes.get_mut(&pane_id) {
                        if let Some(app) = pane.as_app_mut() {
                            app.runtime.queue_outbound_event(
                                crate::app_protocol::PlexiEvent::NotifyAction {
                                    notify_id,
                                    action_label,
                                    value,
                                },
                            );
                        }
                    }
                }
                AppCommand::DeliverPipeMessage { sender_pane_id, pipe_id, payload } => {
                    let active = self.active_window;
                    // Directed pipe (#286) — only the non-sender member of
                    // the pair receives. Falls through to the legacy peer
                    // broadcast when the pipe was not opened directed.
                    if let Some(&(a, b)) = self.directed_pipes.get(&pipe_id) {
                        let target_pid = if sender_pane_id == a {
                            Some(b)
                        } else if sender_pane_id == b {
                            Some(a)
                        } else {
                            // Neither side — wire mismatch. Log + drop.
                            log::warn!(
                                "DeliverPipeMessage: directed pipe '{pipe_id}' \
                                 sender {sender_pane_id} not in pair ({a}, {b}); dropping"
                            );
                            None
                        };
                        if let Some(tid) = target_pid {
                            if let Some(pane) = self.windows[active].panes.get_mut(&tid) {
                                let event = crate::app_protocol::PlexiEvent::PipeMessage {
                                    pipe_id: pipe_id.clone(),
                                    payload: payload.clone(),
                                };
                                if let Some(app) = pane.as_app_mut() {
                                    app.runtime.queue_outbound_event(event);
                                }
                            }
                        }
                        continue;
                    }
                    let pane_ids: Vec<_> = self.windows[active].panes.keys().copied().collect();
                    for pid in pane_ids {
                        if pid == sender_pane_id {
                            continue; // don't echo back to sender
                        }
                        let is_reader = self.windows[active]
                            .panes
                            .get(&pid)
                            .and_then(|p| p.as_app())
                            .map(|a| match &a.runtime {
                                crate::pane::AppRuntime::Process(pa) => {
                                    pa.pipe_registry.lock().unwrap().has_reader(&pipe_id)
                                }
                                crate::pane::AppRuntime::Builtin(_) => false,
                            })
                            .unwrap_or(false);
                        if is_reader {
                            if let Some(pane) = self.windows[active].panes.get_mut(&pid) {
                                if let Some(app) = pane.as_app_mut() {
                                    app.runtime.queue_outbound_event(
                                        crate::app_protocol::PlexiEvent::PipeMessage {
                                            pipe_id: pipe_id.clone(),
                                            payload: payload.clone(),
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
                AppCommand::OpenDirectedPipe {
                    sender_pane_id,
                    pipe_id,
                    target_pane_id,
                } => {
                    // Subscribe both sides + record the pair so subsequent
                    // `DeliverPipeMessage` for this pipe routes ONLY between
                    // them (#286). The sender already registered the pipe
                    // locally inside its own ProcessApp; we need to register
                    // it on the target so its `has_reader` returns true and
                    // its SDK has a Pipe handle if it sends in reverse.
                    let active = self.active_window;
                    let target_kind = self.windows[active]
                        .panes
                        .get(&target_pane_id)
                        .map(|p| match p {
                            crate::pane::Pane::App(_) => "app",
                            crate::pane::Pane::Terminal(_) => "terminal",
                        });
                    match target_kind {
                        Some("app") => {
                            // Register pipe on target's registry so it can
                            // PipeSend back through the same id.
                            let registered = if let Some(pane) =
                                self.windows[active].panes.get_mut(&target_pane_id)
                            {
                                register_directed_pipe_on_target(pane, &pipe_id)
                            } else {
                                false
                            };
                            if !registered {
                                log::warn!(
                                    "OpenDirectedPipe: failed to register '{pipe_id}' on target {target_pane_id}"
                                );
                                continue;
                            }
                            self.directed_pipes
                                .insert(pipe_id.clone(), (sender_pane_id, target_pane_id));
                            log::info!(
                                "OpenDirectedPipe: '{pipe_id}' subscribed pane {sender_pane_id} ↔ pane {target_pane_id}"
                            );
                        }
                        Some(other) => log::warn!(
                            "OpenDirectedPipe: target {target_pane_id} is a {other}; expected app — pipe '{pipe_id}' not subscribed"
                        ),
                        None => log::warn!(
                            "OpenDirectedPipe: target pane {target_pane_id} not found; pipe '{pipe_id}' dropped"
                        ),
                    }
                }
                AppCommand::RequestLinkedTerminal {
                    sender_pane_id,
                    request_id,
                    cwd,
                    label: _label,
                } => {
                    self.dispatch_request_linked_terminal(sender_pane_id, request_id, cwd);
                }
                AppCommand::RunInLinkedTerminal {
                    terminal_pane_id,
                    command,
                    echo,
                } => {
                    self.dispatch_run_in_linked_terminal(terminal_pane_id, command, echo);
                }
                AppCommand::InsertPathToken {
                    terminal_pane_id,
                    path,
                    mode,
                } => {
                    self.dispatch_insert_path_token(terminal_pane_id, path, mode);
                }
                AppCommand::RequestCommandPreview {
                    sender_pane_id,
                    request_id,
                    terminal_pane_id,
                    command,
                } => {
                    self.dispatch_command_preview(
                        sender_pane_id,
                        request_id,
                        terminal_pane_id,
                        command,
                    );
                }
                AppCommand::OpenArtifact { path, mode } => {
                    self.dispatch_open_artifact(path, mode);
                }
                AppCommand::DeliverRunUpdate { originator_type_id, event } => {
                    let active = self.active_window;
                    let pane_ids: Vec<_> = self.windows[active].panes.keys().copied().collect();
                    let mut delivered = false;
                    for pid in pane_ids {
                        let matches = self.windows[active]
                            .panes
                            .get(&pid)
                            .and_then(|p| p.as_app())
                            .map(|a| match &a.runtime {
                                crate::pane::AppRuntime::Process(pa) => {
                                    pa.type_id == originator_type_id
                                }
                                crate::pane::AppRuntime::Builtin(_) => false,
                            })
                            .unwrap_or(false);
                        if matches {
                            if let Some(pane) = self.windows[active].panes.get_mut(&pid) {
                                if let Some(app) = pane.as_app_mut() {
                                    app.runtime.queue_outbound_event(event.clone());
                                    delivered = true;
                                    break;
                                }
                            }
                        }
                    }
                    if !delivered {
                        log::warn!(
                            "DeliverRunUpdate: no pane found for type_id='{originator_type_id}'"
                        );
                    }
                }
            }
        }

        // Check if the focused app wants to close itself (e.g. after saving).
        {
            let ctx_ref = &self.windows[self.active_window];
            let should_close = ctx_ref
                .focused_pane
                .and_then(|tile| ctx_ref.tree.tiles.get(tile))
                .and_then(|tile| {
                    if let egui_tiles::Tile::Pane(pid) = tile {
                        Some(*pid)
                    } else {
                        None
                    }
                })
                .and_then(|pid| ctx_ref.panes.get(&pid))
                .map(|pane| {
                    pane.as_app()
                        .map(|a| a.runtime.wants_close())
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            if should_close {
                self.close_focused_app();
            }
        }

        // Update window title to reflect active pane — readable by AppleScript / OS scripts
        {
            let context = &self.windows[self.active_window];
            let pane_name = context
                .focused_pane
                .and_then(|tile_id| context.tree.tiles.get(tile_id))
                .and_then(|tile| {
                    if let egui_tiles::Tile::Pane(pane_id) = tile {
                        context.panes.get(pane_id)
                    } else {
                        None
                    }
                })
                .and_then(|pane| {
                    if let Some(t) = pane.as_terminal() {
                        t.name.clone()
                    } else {
                        pane.as_app().map(|a| a.name.clone())
                    }
                });
            let ws = self.router.active();
            let ws_id = ws.context_id;
            let window_count = self.windows.iter().filter(|c| c.context_id == ws_id).count();
            let context_label = if window_count > 1 {
                format!("{} ({},{})", ws.name, context.grid_x, context.grid_y)
            } else {
                ws.name.clone()
            };
            let title = match pane_name {
                Some(name) => format!("{} — {}", context_label, name),
                None => context_label,
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        // Determine if the focused pane has an active app surface, and whether
        // that app has declared keyboard_capture mode.
        let (app_active, keyboard_capture_active) = {
            let context = &self.windows[self.active_window];
            let focused_pane = context.focused_pane.and_then(|tile_id| {
                if let Some(egui_tiles::Tile::Pane(pane_id)) = context.tree.tiles.get(tile_id) {
                    context.panes.get(pane_id)
                } else {
                    None
                }
            });
            let active = focused_pane
                .map(|pane| pane.as_app().is_some())
                .unwrap_or(false);
            let capture = if active {
                focused_pane
                    .and_then(|p| p.as_app())
                    .map(|a| a.runtime.keyboard_capture())
                    .unwrap_or(false)
            } else {
                false
            };
            (active, capture)
        };

        // Handle keyboard shortcuts
        let modal_open = self.show_notification_modal;
        for action in keys::poll_actions(ctx, app_active, keyboard_capture_active, modal_open, self.show_shortcuts) {
            match action {
                Action::SplitHorizontal => {
                    self.windows[self.active_window].zoomed_pane = None;
                    self.split_focused(false);
                    self.save_workspace();
                }
                Action::SplitVertical => {
                    self.windows[self.active_window].zoomed_pane = None;
                    self.split_focused(true);
                    self.save_workspace();
                }
                Action::SplitRight => {
                    self.windows[self.active_window].zoomed_pane = None;
                    self.split_focused_mirror(crate::host::command::Placement::Right);
                    self.save_workspace();
                }
                Action::SplitDown => {
                    self.windows[self.active_window].zoomed_pane = None;
                    self.split_focused_mirror(crate::host::command::Placement::Below);
                    self.save_workspace();
                }
                Action::Navigate(dir) => {
                    let was_zoomed = self.windows[self.active_window].zoomed_pane.is_some();
                    self.navigate(dir);
                    if was_zoomed {
                        self.windows[self.active_window].zoomed_pane =
                            self.windows[self.active_window].focused_pane;
                    }
                }
                Action::SwapPane(dir) => {
                    match self.swap_pane(dir) {
                        crate::pane_ops::SwapResult::Swapped {
                            rect_a, rect_b, ..

                        } => {
                            let now = std::time::Instant::now();
                            self.pane_anims = vec![
                                PaneSwapAnim { from: rect_a, to: rect_b, started_at: now },
                                PaneSwapAnim { from: rect_b, to: rect_a, started_at: now },
                            ];
                            self.ctx.request_repaint();
                        }
                        crate::pane_ops::SwapResult::AtBoundary => {
                            if let Some(focused) = self.windows[self.active_window].focused_pane {
                                self.edge_pulse = Some(EdgePulse {
                                    tile: focused,
                                    dir,
                                    started_at: std::time::Instant::now(),
                                });
                                self.ctx.request_repaint();
                            }
                        }
                        crate::pane_ops::SwapResult::NoFocus => {}
                    }
                }
                Action::ClosePane => {
                    // If the focused app pane has a non-empty nav stack
                    // (via PushNav), Escape routes NavBack to the app
                    // instead of closing the pane.
                    if !self.try_nav_back_focused() {
                        if self.confirm_close() {
                            self.pending_close = true;
                        } else if self.execute_close_pane() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        } else {
                            self.save_workspace();
                        }
                    }
                }
                Action::NavBackApp => {
                    // Cmd+[ when a nav-active app pane is focused: go back one
                    // level. Falls through to cycling tabs backwards if no nav is active.
                    if !self.try_nav_back_focused() {
                        self.cycle_tab(false);
                    }
                }
                Action::NewTab => {
                    self.new_tab();
                    self.save_workspace();
                }
                Action::ToggleZoom => {
                    let ctx = &mut self.windows[self.active_window];
                    if let Some(focused) = ctx.focused_pane {
                        if ctx.zoomed_pane == Some(focused) {
                            ctx.zoomed_pane = None;
                        } else {
                            ctx.zoomed_pane = Some(focused);
                        }
                    }
                }
                Action::Quit => {
                    if !self.confirm_quit() {
                        self.quitting = true;
                        self.save_workspace();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    } else {
                        let now = std::time::Instant::now();
                        let elapsed = self
                            .quit_last_press
                            .map(|t| now.duration_since(t))
                            .unwrap_or(std::time::Duration::MAX);
                        if elapsed > std::time::Duration::from_millis(1500) {
                            self.quit_press_count = 0;
                        }
                        self.quit_press_count += 1;
                        self.quit_last_press = Some(now);
                        if self.quit_press_count >= 3 {
                            self.quit_press_count = 0;
                            self.quit_last_press = None;
                            self.quitting = true;
                            self.save_workspace();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                }
                Action::ToggleSidebar => self.sidebar_visible = !self.sidebar_visible,
                Action::ToggleShortcuts => self.show_shortcuts = !self.show_shortcuts,
                Action::ToggleCommandPalette => {
                    self.show_command_palette = !self.show_command_palette;
                    if self.show_command_palette {
                        self.palette_query.clear();
                        self.palette_selected = 0;
                    }
                }
                Action::RenamePane => {
                    let active_ctx = &self.windows[self.active_window];
                    if let Some(focused_tile) = active_ctx.focused_pane {
                        if let Some(Tile::Pane(pane_id)) = active_ctx.tree.tiles.get(focused_tile) {
                            let pane_id = *pane_id;
                            self.rename_buffer = active_ctx
                                .panes
                                .get(&pane_id)
                                .and_then(|p| p.as_terminal())
                                .and_then(|t| t.name.clone())
                                .unwrap_or_default();
                            self.renaming_pane = Some(pane_id);
                        }
                    }
                }
                Action::SwitchContext(n) => {
                    if n < self.router.len() {
                        self.switch_workspace(n);
                    }
                }
                Action::NextTab => {
                    self.cycle_tab(true);
                }
                Action::IncreasePaneFontSize => {
                    self.adjust_focused_pane_font_size(1.0);
                }
                Action::DecreasePaneFontSize => {
                    self.adjust_focused_pane_font_size(-1.0);
                }
                Action::ScrollUp => {
                    self.scroll_focused_pane(3);
                }
                Action::ScrollDown => {
                    self.scroll_focused_pane(-3);
                }
                Action::CloseApp => {
                    self.close_focused_app();
                }
                Action::ToggleAppFocus => {
                    // Tab navigates between the app pane and the linked
                    // terminal pane below (they're separate tiles now).
                    self.navigate(crate::keys::Direction::Down);
                }
                Action::OpenFileBrowser => {
                    self.open_file_browser();
                }
                Action::OpenQuickNote => {
                    self.open_quick_note();
                }
                Action::OpenConfig => {
                    crate::config::open_config_file();
                }
                Action::ReloadConfig => {
                    self.reload_config();
                }
                Action::OpenSecretsManager => {
                    self.open_secrets_manager();
                }
                Action::ToggleRunPalette => {
                    self.show_run_palette = !self.show_run_palette;
                }
                Action::ToggleNotificationModal => {
                    if self.show_notification_modal {
                        self.show_notification_modal = false;
                    } else {
                        self.show_notification_modal = true;
                        // Pick highest-priority when re-opening the modal and
                        // nothing is currently pinned. If something IS pinned
                        // (user closed+reopened), keep showing that one.
                        if self.current_notify_id.is_none() {
                            self.current_notify_id = self.select_highest_priority();
                        }
                    }
                }
                Action::NotificationCycleNext => {
                    self.cycle_notification(1);
                }
                Action::NotificationCyclePrev => {
                    self.cycle_notification(-1);
                }
                Action::ForceReloadApp => {
                    self.force_reload_focused_app();
                }
                Action::NewPageRight => {
                    if self.windows[self.active_window].panes.is_empty()
                        || self.windows[self.active_window].tree.root.is_none()
                    {
                        self.reset_active_context();
                    } else {
                        self.new_page_right();
                    }
                    self.save_workspace();
                }
                Action::NewContext => {
                    self.new_context();
                    self.save_workspace();
                }
                Action::PageLeft => {
                    self.navigate_or_create_page(-1, 0);
                }
                Action::PageRight => {
                    self.navigate_or_create_page(1, 0);
                }
                Action::PageUp => {
                    self.navigate_or_create_page(0, -1);
                }
                Action::PageDown => {
                    self.navigate_or_create_page(0, 1);
                }
                Action::ToggleMinimap => {
                    self.minimap.toggle();
                }
            }
        }

        // Hot reload (#83): drain any pending file-watcher reload requests.
        // Each `ReloadRequest` causes the matching pane's ProcessApp to be
        // dropped (sending Shutdown + reaping the child) and replaced with
        // a fresh subprocess. Idempotent if the pane was closed since.
        self.drain_hot_reload_requests();

        // Reload configuration from disk when the user clicks
        // "Reload Configuration" in the app menu.
        crate::macos_menu::apply_version_title_once();
        if crate::macos_menu::take_reload_config_flag() {
            self.reload_config();
        }

        // Handle window close request (X button or macOS Cmd+Q OS event).
        //
        // On macOS, Cmd+Q fires BOTH a keyboard event (consumed by keys.rs →
        // Action::Quit → triple-tap) AND a close_requested viewport event in the
        // same frame. Without CancelClose here, the OS close wins the race,
        // quitting immediately and bypassing the triple-tap entirely.
        //
        // We distinguish the two sources by whether a keyboard quit flow is in
        // progress (quit_press_count > 0):
        //   - quit_press_count > 0 → Cmd+Q initiated; cancel the OS close and let
        //     the triple-tap flow own the quit path.
        //   - quit_press_count == 0 → X button or system quit; save and allow close
        //     immediately (no triple-tap for deliberate window close gestures).
        //   - quitting == true → triple-tap completed; save and allow close.
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.quitting {
                self.save_workspace();
            } else if self.confirm_quit() && self.quit_press_count > 0 {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else {
                self.save_workspace();
            }
        }

        // Toolbar
        egui::TopBottomPanel::top("toolbar")
            .exact_height(28.0)
            .frame(
                egui::Frame::new()
                    .fill(self.colors.bg_toolbar)
                    .inner_margin(egui::Margin {
                        left: 8,
                        right: 8,
                        top: 4,
                        bottom: 4,
                    }),
            )
            .show(ctx, |ui| {
                self.draw_toolbar(ui);
            });

        // Separator line under toolbar
        egui::TopBottomPanel::top("toolbar_sep")
            .exact_height(1.0)
            .frame(egui::Frame::new().fill(self.colors.border))
            .show(ctx, |_ui| {});

        // Sidebar
        if self.sidebar_visible {
            egui::SidePanel::left("sidebar")
                .exact_width(220.0)
                .resizable(false)
                .show_separator_line(false)
                .frame(
                    egui::Frame::new()
                        .fill(self.colors.bg_sidebar)
                        .inner_margin(egui::Margin::same(0)),
                )
                .show(ctx, |ui| {
                    self.draw_sidebar(ui);
                });
        }

        // Central panel — terminal tiles (or welcome screen when context is empty)
        egui::CentralPanel::default()
            .frame(egui::Frame {
                fill: self.colors.bg_darkest,
                inner_margin: egui::Margin::same(4),
                outer_margin: egui::Margin::ZERO,
                ..Default::default()
            })
            .show(ctx, |ui| {
                let active = self.active_window;
                if self.windows[active].panes.is_empty() || self.windows[active].tree.root.is_none() {
                    self.draw_welcome_screen(ui);
                    return;
                }

                let ctx = &mut self.windows[self.active_window];

                // Resolve focused_pane if simplifier moved the tile
                if let Some(fp) = ctx.focused_pane {
                    if !matches!(ctx.tree.tiles.get(fp), Some(Tile::Pane(_))) {
                        ctx.focused_pane = ctx.find_first_pane_in(fp);
                    }
                }

                // Validate zoomed pane still exists
                if let Some(zp) = ctx.zoomed_pane {
                    if !matches!(ctx.tree.tiles.get(zp), Some(Tile::Pane(_))) {
                        ctx.zoomed_pane = None;
                    }
                }

                let zoomed_pane = ctx.zoomed_pane;
                let tab_info = ctx.compute_tab_info();
                let pane_names: HashMap<PaneId, String> = ctx
                    .panes
                    .iter()
                    .filter_map(|(&id, p)| p.as_terminal()?.name.as_ref().map(|n| (id, n.clone())))
                    .collect();
                let suppress_focus = self.show_command_palette
                    || self.renaming_pane.is_some()
                    || self.renaming_window.is_some();

                // When a pane is zoomed, drag targeting is moot (the whole
                // window targets one pane), so skip the unsafe ObjC cursor
                // probe entirely. Otherwise, throttle the repaint to 100ms
                // — 60fps polling of NSApplication APIs from the render
                // loop is wasteful and a candidate cause of the file-drag
                // spinning ball seen in production.
                #[cfg(target_os = "macos")]
                let drag_cursor_pos: Option<egui::Pos2> = if zoomed_pane.is_some() {
                    None
                } else {
                    let has_drag = ui.input(|i| {
                        !i.raw.hovered_files.is_empty() || !i.raw.dropped_files.is_empty()
                    });
                    if has_drag {
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(100));
                        use objc2_app_kit::NSApplication;
                        use objc2_foundation::MainThreadMarker;
                        MainThreadMarker::new()
                            .and_then(|mtm| {
                                let app = NSApplication::sharedApplication(mtm);
                                app.keyWindow().or_else(|| unsafe { app.mainWindow() })
                            })
                            .map(|w| {
                                let p = unsafe { w.mouseLocationOutsideOfEventStream() };
                                let content_height = ui.ctx().screen_rect().height();
                                egui::pos2(p.x as f32, content_height - p.y as f32)
                            })
                    } else {
                        None
                    }
                };
                #[cfg(not(target_os = "macos"))]
                let drag_cursor_pos: Option<egui::Pos2> = None;

                // Cache once per frame — used by PlexiBehavior to avoid O(n) ui.input() reads.
                let hovered_files = ui.input(|i| !i.raw.hovered_files.is_empty());

                // Edge-trigger: log the first frame a file drag enters the window (zoomed path).
                // Subsequent frames are silent. Used to pinpoint freeze location in the log.
                if hovered_files && zoomed_pane.is_some() {
                    let hover_id = egui::Id::new("drag_hover_was_active");
                    let was_hovering: bool =
                        ui.ctx().data(|d| d.get_temp(hover_id).unwrap_or(false));
                    if !was_hovering {
                        log::info!("[DRAG] hover: hovered_files became non-empty on zoomed pane");
                    }
                    ui.ctx().data_mut(|d| d.insert_temp(hover_id, true));
                } else {
                    let hover_id = egui::Id::new("drag_hover_was_active");
                    ui.ctx().data_mut(|d| d.insert_temp(hover_id, false));
                }

                let mut behavior = PlexiBehavior {
                    panes: &mut ctx.panes,
                    focused_tile: if suppress_focus {
                        None
                    } else {
                        ctx.focused_pane
                    },
                    theme: self.theme.clone(),
                    new_focused: None,
                    close_exited: None,
                    tab_info,
                    zoomed_pane,
                    colors: self.colors,
                    pane_names,
                    drag_cursor_pos,
                    hovered_files,
                    workspace_root: crate::config::active_workspace_root(),
                };
                log::debug!("[DRAG] tiling: start (zoomed={}, hovered_files={hovered_files})", zoomed_pane.is_some());
                ctx.tree.ui(&mut behavior, ui);
                log::debug!("[DRAG] tiling: done");

                if let Some(new) = behavior.new_focused {
                    ctx.focused_pane = Some(new);
                }

                let should_close_exited = behavior.close_exited.is_some();

                // Draw zoom overlay if a pane is zoomed
                if let Some(zoomed_tile) = zoomed_pane {
                    if let Some(Tile::Pane(pane_id)) = ctx.tree.tiles.get(zoomed_tile) {
                        let pane_id = *pane_id;
                        let panel_rect = ui.max_rect();
                        let zoomed_tab_info = behavior.tab_info.get(&zoomed_tile).copied();

                        // Drop behavior to release the mutable borrow on ctx.panes
                        drop(behavior);

                        // Semi-transparent scrim over the entire central panel
                        ui.painter()
                            .rect_filled(panel_rect, 0.0, Color32::from_black_alpha(75));

                        // Inset rect for the zoomed pane
                        let inset = 10.0;
                        let zoom_rect = panel_rect.shrink(inset);

                        // Thicker accent border (2px)
                        ui.painter().rect_stroke(
                            zoom_rect,
                            CornerRadius::same(4),
                            Stroke::new(2.0, self.colors.accent),
                            StrokeKind::Inside,
                        );

                        // Render zoomed terminal in the inset rect
                        let inner_rect = zoom_rect.shrink(2.0); // inside the border
                        let mut child_ui =
                            ui.new_child(egui::UiBuilder::new().max_rect(inner_rect));
                        // Files dropped onto a zoomed pane must go to the
                        // zoomed terminal, NOT a background tile. The
                        // per-tile drop path in `tiling.rs` is gated off
                        // while zoomed; we handle the drop here instead.
                        let has_drop =
                            child_ui.input(|i| !i.raw.dropped_files.is_empty());
                        // When zoomed there is only one pane covering the entire overlay —
                        // no ambiguity about the target. Skip rect_contains_pointer: egui's
                        // pointer position is stale during macOS OS-level file drags (we skip
                        // the NSApplication cursor query when zoomed), so rect_contains_pointer
                        // would only return true if the last known position happened to fall
                        // inside inner_rect (typically the original split-pane location).
                        let dropped_to_zoom = has_drop;
                        if has_drop {
                            log::info!(
                                "drop: zoomed overlay received drop event — pane_id={pane_id:?}"
                            );
                        }
                        child_ui.set_opacity(0.88);
                        egui::Frame::new()
                            .fill(self.colors.terminal_bg)
                            .inner_margin(egui::Margin::same(8))
                            .show(&mut child_ui, |ui| {
                                if let Some(pane) = ctx.panes.get_mut(&pane_id) {
                                    if let Some(t) = pane.as_terminal_mut() {
                                        if dropped_to_zoom {
                                            crate::tiling::write_dropped_paths_to_terminal(ui, t);
                                        }
                                        if t.exited {
                                            let rect = ui.max_rect();
                                            ui.painter().rect_filled(
                                                rect,
                                                0.0,
                                                self.colors.terminal_bg,
                                            );
                                            ui.allocate_new_ui(
                                                egui::UiBuilder::new().max_rect(rect),
                                                |ui| {
                                                    ui.centered_and_justified(|ui| {
                                                        ui.colored_label(
                                                            self.colors.text_dim,
                                                            "[process exited]",
                                                        );
                                                    });
                                                },
                                            );
                                        } else if hovered_files {
                                            // Skip TerminalView render while a file is being
                                            // dragged over the zoomed pane. TerminalView::show()
                                            // calls backend.sync() which clones the full grid
                                            // under FairMutex contention — on a large terminal
                                            // (e.g. a full-window Claude Code session) this
                                            // blocked the main thread for several seconds.
                                            // The drop itself is handled above (dropped_to_zoom
                                            // guard), so we skip only the hover-frame renders.
                                            log::debug!("[DRAG] zoom overlay: skipping TerminalView render during file hover");
                                            let rect = ui.max_rect();
                                            ui.allocate_new_ui(
                                                egui::UiBuilder::new().max_rect(rect),
                                                |ui| {
                                                    ui.centered_and_justified(|ui| {
                                                        ui.colored_label(
                                                            self.colors.text_dim,
                                                            "Drop to paste path",
                                                        );
                                                    });
                                                },
                                            );
                                        } else {
                                            // Reserve space for tab dots if in a tab group
                                            if zoomed_tab_info.is_some() {
                                                ui.add_space(
                                                    crate::tiling::TAB_DOT_RESERVED_HEIGHT,
                                                );
                                            }
                                            let font_size = t.font_size;
                                            log::debug!("[DRAG] zoom overlay: TerminalView render start");
                                            let terminal = TerminalView::new(ui, &mut t.backend)
                                                .set_focus(true)
                                                .set_theme(self.theme.clone())
                                                .set_font(theme::terminal_font(font_size))
                                                .set_size(Vec2::new(
                                                    ui.available_width(),
                                                    ui.available_height(),
                                                ));
                                            ui.add(terminal);
                                            log::debug!("[DRAG] zoom overlay: TerminalView render done");
                                        }
                                    } else if let Some(a) = pane.as_app_mut() {
                                        let app_ctx = crate::app_trait::AppRenderContext {
                                            colors: &self.colors,
                                        };
                                        a.runtime.ui(ui, &app_ctx);
                                    }
                                }

                                // Draw tab indicator dots (same style as tiling.rs)
                                if let Some((active_idx, count)) = zoomed_tab_info {
                                    let rect = ui.max_rect();
                                    crate::tiling::paint_tab_dots(
                                        ui.painter(),
                                        rect.left(),
                                        rect.top() + 2.0 + 4.0, // 4.0 = dot radius
                                        active_idx,
                                        count,
                                        self.colors.accent,
                                        self.colors.bg_active,
                                    );
                                }
                            });
                    } else {
                        drop(behavior);
                    }
                } else {
                    drop(behavior);
                }

                // ── Pane swap animation overlays ────────────────────────────────────────
                {
                    let anim_dur = std::time::Duration::from_millis(160);
                    let edge_dur = std::time::Duration::from_millis(120);
                    let now_anim = std::time::Instant::now();

                    self.pane_anims.retain(|a| now_anim.duration_since(a.started_at) < anim_dur);
                    if let Some(ref pulse) = self.edge_pulse {
                        if now_anim.duration_since(pulse.started_at) >= edge_dur {
                            self.edge_pulse = None;
                        }
                    }

                    for anim in &self.pane_anims {
                        let elapsed = now_anim.duration_since(anim.started_at).as_secs_f32();
                        let t = (elapsed / anim_dur.as_secs_f32()).clamp(0.0, 1.0);
                        let t_eased = 1.0 - (1.0 - t).powi(3);
                        let animated_rect = egui::Rect {
                            min: anim.from.min.lerp(anim.to.min, t_eased),
                            max: anim.from.max.lerp(anim.to.max, t_eased),
                        };
                        ui.painter().rect_filled(
                            animated_rect,
                            egui::CornerRadius::same(4),
                            self.colors.accent.gamma_multiply(0.25),
                        );
                        self.ctx.request_repaint();
                    }

                    if let Some(ref pulse) = self.edge_pulse {
                        if let Some(pane_rect) = self.windows[self.active_window].tree.tiles.rect(pulse.tile) {
                            let elapsed = now_anim.duration_since(pulse.started_at).as_secs_f32();
                            let alpha = (1.0 - elapsed / edge_dur.as_secs_f32()).max(0.0);
                            let edge_color = self.colors.accent.gamma_multiply(alpha);
                            let (p1, p2) = match pulse.dir {
                                crate::keys::Direction::Left => (
                                    egui::pos2(pane_rect.left(), pane_rect.top()),
                                    egui::pos2(pane_rect.left(), pane_rect.bottom()),
                                ),
                                crate::keys::Direction::Right => (
                                    egui::pos2(pane_rect.right(), pane_rect.top()),
                                    egui::pos2(pane_rect.right(), pane_rect.bottom()),
                                ),
                                crate::keys::Direction::Up => (
                                    egui::pos2(pane_rect.left(), pane_rect.top()),
                                    egui::pos2(pane_rect.right(), pane_rect.top()),
                                ),
                                crate::keys::Direction::Down => (
                                    egui::pos2(pane_rect.left(), pane_rect.bottom()),
                                    egui::pos2(pane_rect.right(), pane_rect.bottom()),
                                ),
                            };
                            ui.painter().line_segment([p1, p2], egui::Stroke::new(3.0, edge_color));
                            self.ctx.request_repaint();
                        }
                    }
                }

                if should_close_exited {
                    self.close_focused();
                }
            });

        // Shortcuts overlay
        self.draw_shortcuts_overlay(ctx);

        // Changelog overlay
        self.draw_changelog_overlay(ctx);

        // First-launch CLI setup prompt
        if self.show_cli_setup_prompt {
            self.draw_cli_setup_modal(ctx);
        }

        // Minimap overlay — auto-hidden when current workspace has <2 windows.
        let ws_id = self.router.active().context_id;
        let window_count = self.windows.iter().filter(|c| c.context_id == ws_id).count();
        if window_count >= 2 {
            self.draw_minimap_overlay(ctx);
        } else {
            self.minimap.visible = false;
        }

        // Command palette, run palette, rename-pane overlay, notification
        // modal, and confirm-close are all drawn by the early input-capture
        // path at the top of `update()` — they own a `FocusLayer` and render
        // their own keystrokes before the drain. Drawing again here would
        // double-dispatch Enter/Escape after keys have been drained.
        // Quit confirmation overlay
        if self.confirm_quit() && self.quit_press_count > 0 {
            // Reset on Escape or timeout
            let timed_out = self
                .quit_last_press
                .map(|t| t.elapsed() > std::time::Duration::from_millis(1500))
                .unwrap_or(false);
            if timed_out || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.quit_press_count = 0;
                self.quit_last_press = None;
            } else {
                self.draw_quit_confirm_overlay(ctx);
                // Keep repainting so the timeout dismissal fires promptly
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        self.draw_feature_effects(ctx);

        let frame_ms = _frame_start.elapsed().as_millis();
        if frame_ms > 50 {
            log::warn!("slow frame: {}ms", frame_ms);
        }
    }
}

impl PlexiApp {
    /// True when a modal overlay owns keyboard input. Used by `update()` to
    /// drain remaining key events after the overlay has rendered so panes see
    /// an empty input buffer this frame.
    pub(crate) fn input_captured_by_overlay(&self) -> bool {
        matches!(
            self.focus_stack.last(),
            Some(FocusLayer::NotificationModal)
                | Some(FocusLayer::ConfirmClose)
                | Some(FocusLayer::CommandPalette)
                | Some(FocusLayer::RunPalette)
                | Some(FocusLayer::RenamePane)
                | Some(FocusLayer::ContextRename)
        )
    }

    /// Push a focus layer. Idempotent — if the same layer is already on top,
    /// it's a no-op. Callers should pair with `pop_focus_layer`.
    pub(crate) fn push_focus_layer(&mut self, layer: FocusLayer) {
        if self.focus_stack.last() != Some(&layer) {
            self.focus_stack.push(layer);
        }
    }

    /// Pop the given layer if it's currently on top. No-op otherwise; this
    /// prevents out-of-order pops from corrupting the stack.
    pub(crate) fn pop_focus_layer(&mut self, layer: &FocusLayer) {
        if self.focus_stack.last() == Some(layer) {
            self.focus_stack.pop();
        }
    }

    /// Reconcile the focus stack with the notification modal visibility. Called
    /// once per frame — the modal can open/close from many paths (arrival,
    /// Cmd+Shift+A, queue drains to empty mid-frame), so the source of truth is
    /// `show_notification_modal && !pending_notifications.is_empty()`.
    /// Read `confirm_quit` from the config tunnel. Defaults to `true` so
    /// users get the safer triple-tap behavior unless they explicitly opt out.
    pub(crate) fn confirm_quit(&self) -> bool {
        self.config.confirm_quit.unwrap_or(true)
    }

    /// Read `confirm_close` from the config tunnel. Defaults to `false` so
    /// pane close is instant unless the user explicitly enables the dialog.
    pub(crate) fn confirm_close(&self) -> bool {
        self.config.confirm_close.unwrap_or(false)
    }

    /// If the currently focused pane is an app with a non-empty nav stack,
    /// emit `PlexiEvent::NavBack` to it and return `true`. Returns `false`
    /// when there is no nav-active focused pane (caller may fall back to
    /// default behaviour such as closing the pane or cycling tabs).
    pub(crate) fn try_nav_back_focused(&mut self) -> bool {
        let active = self.active_window;
        let active_ctx = &self.windows[active];

        // Read nav state under shared borrow first.
        let nav_result = active_ctx.focused_pane.and_then(|tile_id| {
            if let Some(egui_tiles::Tile::Pane(pane_id)) = active_ctx.tree.tiles.get(tile_id) {
                active_ctx.panes.get(pane_id).and_then(|pane| {
                    pane.as_app().and_then(|app| {
                        if app.runtime.nav_stack_depth() > 0 {
                            Some((*pane_id, app.runtime.nav_back_view_id()))
                        } else {
                            None
                        }
                    })
                })
            } else {
                None
            }
        });

        if let Some((pane_id, view_id)) = nav_result {
            if let Some(pane) = self.windows[active].panes.get_mut(&pane_id) {
                if let Some(app) = pane.as_app_mut() {
                    app.runtime.queue_outbound_event(
                        crate::app_protocol::PlexiEvent::NavBack { view_id },
                    );
                }
            }
            true
        } else {
            false
        }
    }

    /// Re-read configuration from disk and apply changes that can take
    /// effect without a restart (theme, font size, notification settings,
    /// confirmation toggles). Logs the reload so the user knows it worked.
    pub(crate) fn reload_config(&mut self) {
        let active_workspace = crate::config::active_workspace_root();
        let fresh = crate::config::PlexiConfig::load_with_workspace(active_workspace.as_deref());

        // Theme
        let theme_cfg = Self::resolve_theme_config(&fresh);
        let new_colors = crate::theme::Colors::from_config(&theme_cfg);
        if self.colors != new_colors {
            self.colors = new_colors.clone();
            crate::theme::setup_style(&self.ctx, &new_colors);
        }

        // Terminal theme
        self.theme = crate::theme::terminal_theme(&theme_cfg);

        // Font size
        if let Some(size) = fresh.font_size {
            if (size - self.default_font_size).abs() > 0.01 {
                self.default_font_size = size;
            }
        }

        // Notifications
        self.notifications_enabled = fresh
            .notifications
            .as_ref()
            .and_then(|n| n.enabled)
            .unwrap_or(true);
        self.notifications_focus_mode = fresh
            .notifications
            .as_ref()
            .and_then(|n| n.focus_mode)
            .unwrap_or(false);
        self.notifications_interrupt_threshold = fresh
            .notifications
            .as_ref()
            .and_then(|n| n.interrupt_threshold)
            .unwrap_or(100);

        // Feature flags
        self.features = crate::features::FeatureFlags::from_config(&fresh);

        // Replace the cached config
        self.config = fresh;

        log::info!("Configuration reloaded from disk.");
    }

    /// Reconcile the confirm-close focus layer with `pending_close`. Mirrors
    /// `sync_notification_modal_focus` — the source of truth is a boolean
    /// toggled from multiple paths, and the focus stack must follow it
    /// deterministically each frame.
    pub(crate) fn sync_confirm_close_focus(&mut self) {
        let should_own = self.pending_close;
        let has_layer = self
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::ConfirmClose);
        if should_own && !has_layer {
            self.push_focus_layer(FocusLayer::ConfirmClose);
        } else if !should_own && has_layer {
            self.pop_focus_layer(&FocusLayer::ConfirmClose);
        }
    }

    pub(crate) fn sync_notification_modal_focus(&mut self) {
        let should_own = self.show_notification_modal;
        let has_layer = self
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::NotificationModal);
        if should_own && !has_layer {
            self.push_focus_layer(FocusLayer::NotificationModal);
        } else if !should_own && has_layer {
            self.pop_focus_layer(&FocusLayer::NotificationModal);
        }
    }

    /// Reconcile the command-palette focus layer with `show_command_palette`.
    /// Same pattern as the notification modal: boolean visibility flag is the
    /// source of truth, focus stack follows it deterministically each frame.
    pub(crate) fn sync_command_palette_focus(&mut self) {
        let should_own = self.show_command_palette;
        let has_layer = self
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::CommandPalette);
        if should_own && !has_layer {
            self.push_focus_layer(FocusLayer::CommandPalette);
        } else if !should_own && has_layer {
            self.pop_focus_layer(&FocusLayer::CommandPalette);
        }
    }

    /// Reconcile the run-palette focus layer with `show_run_palette`.
    pub(crate) fn sync_run_palette_focus(&mut self) {
        let should_own = self.show_run_palette;
        let has_layer = self
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::RunPalette);
        if should_own && !has_layer {
            self.push_focus_layer(FocusLayer::RunPalette);
        } else if !should_own && has_layer {
            self.pop_focus_layer(&FocusLayer::RunPalette);
        }
    }

    /// Reconcile the rename-pane focus layer with `renaming_pane`.
    pub(crate) fn sync_rename_pane_focus(&mut self) {
        let should_own = self.renaming_pane.is_some();
        let has_layer = self
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::RenamePane);
        if should_own && !has_layer {
            self.push_focus_layer(FocusLayer::RenamePane);
        } else if !should_own && has_layer {
            self.pop_focus_layer(&FocusLayer::RenamePane);
        }
    }

    /// Reconcile the context-rename focus layer. Active when `renaming_window`
    /// is set AND the sidebar is hidden — in that case the inline sidebar row
    /// never renders, so we promote the rename to a modal overlay instead.
    pub(crate) fn sync_context_rename_focus(&mut self) {
        let should_own = self.renaming_window.is_some() && !self.sidebar_visible;
        let has_layer = self
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::ContextRename);
        if should_own && !has_layer {
            self.push_focus_layer(FocusLayer::ContextRename);
        } else if !should_own && has_layer {
            self.pop_focus_layer(&FocusLayer::ContextRename);
        }
    }

    /// Drain keyboard events from `ctx.input` so downstream widgets (panes,
    /// terminal backends, `keys::poll_actions`) see only the global allowlist.
    /// Called after the owning overlay has read what it needs. The allowlist
    /// lets a small set of keybinds (Quit, Close, hide-modal, queue-cycle)
    /// remain live even while an overlay owns focus — users always need a way
    /// to quit or dismiss the overlay.
    pub(crate) fn drain_captured_keyboard_input(&self, ctx: &egui::Context) {
        ctx.input_mut(|i| {
            i.events.retain(|e| match e {
                egui::Event::Key { key, modifiers, .. } => {
                    let cmd = modifiers.command;
                    let shift = modifiers.shift;
                    let alt = modifiers.alt;
                    let ctrl_only = modifiers.ctrl;
                    // Only pass modifier-bearing keys; drop bare key presses.
                    if !cmd || alt || ctrl_only {
                        return false;
                    }
                    // Cmd+Q (quit), Cmd+W (close pane / hide-modal fallback).
                    if !shift && matches!(key, egui::Key::Q | egui::Key::W) {
                        return true;
                    }
                    // Cmd+Shift+A — toggle notification modal (global escape
                    // hatch, survives even required notifs).
                    if shift && matches!(key, egui::Key::A) {
                        return true;
                    }
                    // Cmd+] / Cmd+[ — cycle the notification queue without
                    // acknowledging. Only meaningful while the modal is open.
                    if !shift
                        && matches!(key, egui::Key::CloseBracket | egui::Key::OpenBracket)
                    {
                        return true;
                    }
                    false
                }
                egui::Event::Text(_) => false,
                _ => true,
            });
        });
    }

    /// Route `DeliverNotifyAction` commands back to the originating app pane as
    /// `NotifyAction` events. Shared by the modal and the sidebar panel so both
    /// surfaces dispatch identically.
    pub(crate) fn dispatch_notify_action_cmds(&mut self, cmds: Vec<crate::app_trait::AppCommand>) {
        use crate::app_trait::AppCommand;
        for cmd in cmds {
            if let AppCommand::DeliverNotifyAction { pane_id, notify_id, action_label, value, response_file } = cmd {
                log::info!(
                    "notify:action: pane_id={pane_id} notify_id={notify_id:?} value={value:?}"
                );
                if let Some(rf) = &response_file {
                    let content = value.as_deref().unwrap_or("");
                    match std::fs::write(rf, content) {
                        Ok(_) => log::info!("notify:action: wrote {:?} to {:?}", content, rf),
                        Err(e) => log::warn!("notify:action: failed to write response file {:?}: {e}", rf),
                    }
                }
                let active = self.active_window;
                if let Some(pane) = self.windows[active].panes.get_mut(&pane_id) {
                    if let Some(app) = pane.as_app_mut() {
                        app.runtime.queue_outbound_event(
                            crate::app_protocol::PlexiEvent::NotifyAction {
                                notify_id,
                                action_label,
                                value,
                            },
                        );
                    }
                }
            }
        }
    }

    pub(crate) fn record_context_visit(&mut self, context_id: u64) {
        self.context_visit_history.retain(|&id| id != context_id);
        self.context_visit_history.insert(0, context_id);
        self.context_visit_history.truncate(50);
    }

    fn draw_feature_effects(&self, ctx: &egui::Context) {
        use egui::{Color32, Stroke};

        // CRT effect — scanlines + green phosphor tint
        if self.features.is_enabled("crt") {
            egui::Area::new(egui::Id::new("crt_overlay"))
                .fixed_pos(egui::pos2(0.0, 0.0))
                .order(egui::Order::Foreground)
                .interactable(false)
                .show(ctx, |ui| {
                    let screen = ctx.screen_rect();
                    let painter = ui.painter();

                    // Green phosphor tint
                    painter.rect_filled(screen, 0.0, Color32::from_rgba_unmultiplied(0, 40, 0, 18));

                    // Scanlines every 3 pixels
                    let mut y = screen.top();
                    while y < screen.bottom() {
                        painter.line_segment(
                            [egui::pos2(screen.left(), y), egui::pos2(screen.right(), y)],
                            Stroke::new(0.5, Color32::from_black_alpha(38)),
                        );
                        y += 3.0;
                    }

                    ctx.request_repaint_after(std::time::Duration::from_millis(16));
                });
        }

    }

}


// ── Directed pipe helpers (#286) ─────────────────────────────────────────────

/// Register a duplex JSON pipe on the target pane's typed-pipe registry so
/// `has_reader` returns `true` and the SDK can `pipe_send` back through the
/// same id. Returns `true` on success, `false` if the target pane has no
/// process-app registry to register against (terminals — should never reach
/// this path; logged at the call site).
fn register_directed_pipe_on_target(pane: &mut crate::pane::Pane, pipe_id: &str) -> bool {
    use crate::typed_pipes::PipeDirection;
    let registry = match pane {
        crate::pane::Pane::App(app) => match &app.runtime {
            crate::pane::AppRuntime::Process(pa) => Some(pa.pipe_registry.clone()),
            crate::pane::AppRuntime::Builtin(_) => None,
        },
        crate::pane::Pane::Terminal(_) => None,
    };
    let Some(registry) = registry else {
        return false;
    };
    let result = registry
        .lock()
        .unwrap()
        .open_json(pipe_id.to_string(), PipeDirection::Duplex);
    match result {
        Ok(()) => true,
        // Already open is acceptable — agents may have called `pipe_open` or
        // received a prior directed pipe with the same id; treat as success.
        Err(crate::typed_pipes::PipeError::AlreadyOpen(_)) => true,
        Err(e) => {
            log::warn!("register_directed_pipe_on_target: open_json failed: {e}");
            false
        }
    }
}
