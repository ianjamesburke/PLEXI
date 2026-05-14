mod canvas_bindings;
mod dispatch;
pub(crate) mod notification_image;
mod sync;

/// Returns true for old auto-generated window names ("Page 3,1", "Context 2")
/// Build a shell command string from an args list for passing to `zsh -c <cmd>`.
/// A single arg is used as-is (it's already a shell expression — CLI path).
/// Multiple args are joined with shell quoting so word-splitting is preserved.
fn cmd_from_args(args: &[String]) -> Option<String> {
    match args {
        [] => None,
        [single] => Some(single.clone()),
        multiple => Some(crate::shell::shell_join(multiple)),
    }
}

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

/// Context captured when the quick note modal opens.
#[derive(Default, Clone)]
pub(crate) struct QuickNoteCtx {
    pub cwd: std::path::PathBuf,
    pub workspace_root: Option<std::path::PathBuf>,
    pub context: String,
    pub context_root: Option<std::path::PathBuf>,
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
    RenamePane,
    /// Context naming modal shown when a new context is created while the
    /// sidebar is hidden. Mirrors the inline sidebar rename but as a centred
    /// overlay so the terminal is immediately usable after dismissal.
    ContextRename,
    /// Quick note compose modal (text input phase).
    QuickNote,
    /// Quick note destination picker.
    QuickNoteDestination,
    /// Quick note sub-destination picker. Inner Vec<u8> = key path from root to current node.
    /// E.g. vec![3] = inside destination 3's children; vec![3,2] = destination 3 → child 2.
    QuickNoteSubDestination(Vec<u8>),
    /// First-launch CLI setup prompt. No text input — intercepts keys so they
    /// don't fall through to the active terminal while the modal is visible.
    CliSetupPrompt,
    /// Context inspector modal — shows pane list, allows close/delete.
    ContextInspector,
}

#[derive(Clone)]
pub(crate) struct PendingNotification {
    pub notify_id: String,
    pub sender_pane_id: u64,
    /// Stable context identity the notification originated from (stamped at drain time).
    pub source_context_id: u64,
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
    /// When `Some(t)`, the notification is invisible and exempt from timeout
    /// until `t` has elapsed (snooze). `None` means deliver immediately.
    pub deliver_after: Option<std::time::Instant>,
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
    pub(crate) cli_setup_error: Option<String>,
    pub(crate) quitting: bool,
    pub(crate) quit_press_count: u8,
    pub(crate) quit_last_press: Option<std::time::Instant>,
    pub(crate) pending_close: bool,
    pub(crate) show_context_inspector: bool,
    pub(crate) inspector_selected_pane: usize,
    pub(crate) welcome_delete_press_count: u8,
    pub(crate) welcome_delete_last_press: Option<std::time::Instant>,
    pub(crate) frame_tick: crate::logging::FrameTick,
    /// Cached config so confirmation settings are read through the config
    /// tunnel rather than duplicated as individual bool fields.
    pub(crate) config: crate::config::PlexiConfig,
    pub(crate) key_bindings: crate::keys::KeyBindings,
    pub(crate) voice_config: crate::config::VoiceConfig,
    pub(crate) renaming_window: Option<usize>,
    pub(crate) rename_buffer: String,
    pub(crate) drag_context: Option<usize>,
    pub(crate) registry: AppRegistry,
    pub(crate) show_command_palette: bool,
    pub(crate) palette_query: String,
    pub(crate) palette_selected: usize,
    pub(crate) context_visit_history: Vec<u64>,
    pub(crate) renaming_pane: Option<PaneId>,
    /// One-shot guard: true after `request_focus()` fires on the rename modal's
    /// first render. Prevents the focus from being re-requested every frame,
    /// which lets a later widget steal it on the same frame indefinitely.
    pub(crate) rename_pane_focus_requested: bool,
    pub(crate) features: crate::features::FeatureFlags,
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
    /// Text being composed in the quick note modal.
    pub(crate) quick_note_text: String,
    /// Context captured at the time the quick note modal was opened.
    pub(crate) quick_note_ctx: QuickNoteCtx,
    /// Cursor row in the destination picker (0 = global backlog, 1+ = config destinations).
    pub(crate) quick_note_dest_cursor: usize,
    /// Cursor row in the sub-destination picker.
    pub(crate) quick_note_sub_cursor: usize,
    /// Cache of dynamically loaded children, keyed by full key path. Cleared on modal open.
    pub(crate) quick_note_children_cache: HashMap<Vec<u8>, Vec<crate::config::QuickNoteNode>>,
    /// Pending children_cmd receiver: (key_path, receiver).
    pub(crate) quick_note_children_rx: Option<(Vec<u8>, std::sync::mpsc::Receiver<Result<Vec<crate::config::QuickNoteNode>, String>>)>,
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
    /// Pane focus history for Cmd+[ time-travel. Each entry is (window_id, tile_id)
    /// captured just before a focus change. Capped at 100; oldest evicted from front.
    pub(crate) focus_history_depth: usize,
    pub(crate) pane_focus_history: Vec<(u64, egui_tiles::TileId)>,
    /// Pane focus future — entries undone by back-navigation; cleared on any
    /// organic focus change.
    pub(crate) pane_focus_future: Vec<(u64, egui_tiles::TileId)>,
    /// Set to true during step_focus_history_back/forward to suppress recording
    /// the history-driven focus change as a new history entry.
    pub(crate) navigating_history: bool,
    pub(crate) host: crate::host::model::HostModel,
    pub(crate) host_services: crate::host::services::HostServices,
    /// Parked background ProcessApps — kept alive when their pane is closed.
    /// Keyed by app type_id. Value is `(park_context_id, app)` where
    /// `park_context_id` is the context_id the app was running in when its
    /// pane was closed. Used to route notifications to the correct context.
    pub(crate) background_apps: HashMap<String, (u64, Box<crate::process_app::ProcessApp>)>,
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
    /// Config file watcher (#1115). Watches `config.toml` for saves and fires
    /// a signal so `reload_config()` runs automatically.
    pub(crate) _config_watcher: Option<crate::config_watcher::ConfigWatcher>,
    pub(crate) config_reload_rx: Option<std::sync::mpsc::Receiver<()>>,
    /// Watched panes scheduled for crash-restart. Value is the earliest `Instant` at
    /// which the restart fires — giving the developer ~2s to read the crash overlay.
    pub(crate) pending_crash_restarts: HashMap<PaneId, std::time::Instant>,
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
    /// Last (window_id, tile_id) pair that was logged as a FocusChanged event.
    /// Uses stable window_id (u64) not a vector index so removals don't corrupt it.
    /// Compared at end of each frame to detect genuine focus transitions.
    pub(crate) last_logged_focus: Option<(u64, egui_tiles::TileId)>,
    /// When the current focus session started. Reset on each FocusChanged emit.
    pub(crate) focus_started_at: Option<std::time::Instant>,
    /// Timestamp of the last Space keydown — used to detect double-spacebar
    /// for scratchpad activation. Reset on trigger or when the interval expires.
    pub(crate) last_space_press: Option<std::time::Instant>,
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
        #[cfg(target_os = "macos")]
        crate::finder_service::register();

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

        // Config file watcher (#1115). Watches config.toml for saves so the
        // host can hot-reload theme/font/notification settings automatically.
        let (mut cfg_watcher, mut cfg_reload_rx) =
            match crate::config_watcher::start(crate::config::config_path()) {
                Some((w, rx)) => (Some(w), Some(rx)),
                None => (None, None),
            };

        // Resolve the active workspace (explicit `plexi <path>` arg, then
        // CWD-walk fallback) and overlay its `.plexi/config.toml` on top of
        // the global config. Project values win on a per-field basis; unset
        // project fields preserve the global value.
        let active_workspace = config::active_workspace_root();
        let config = config::PlexiConfig::load_with_workspace(active_workspace.as_deref());
        let key_bindings = crate::keys::build_key_bindings(config.keybindings.as_ref());
        let voice_config = config::VoiceConfig::load_with_workspace(active_workspace.as_deref());
        log::info!(
            "voice: config loaded at startup — enabled={}",
            voice_config.is_enabled()
        );
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
        let focus_history_depth = config.focus_history_depth.unwrap_or(100);
        log::info!("config: focus_history_depth={focus_history_depth}");
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

        // One-time migration: remove the legacy file-queue directory if it
        // still exists from a previous install. Notify commands now travel
        // over the PLEXI_SOCKET, so the directory is dead weight.
        let _ = std::fs::remove_dir_all(crate::config::config_dir().join("notify-queue"));

        // Try to load saved workspace
        if let Some(ws) = WorkspaceFile::load() {
            let mut windows = Vec::new();
            let ctx_name_map: std::collections::HashMap<u64, String> = ws.contexts.iter()
                .map(|c| (c.context_id, c.name.clone()))
                .collect();
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
                        let ctx_name = ctx_name_map.get(&saved_win.context_id).cloned().unwrap_or_default();
                        let settings = Self::make_backend_settings(saved_pane.id, cwd, &colors, saved_win.context_id, &ctx_name);
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
                        root: saved_ctx.root,
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
                    cli_setup_error: None,
                    quitting: false,
                    quit_press_count: 0,
                    quit_last_press: None,
                    show_context_inspector: false,
                    inspector_selected_pane: 0,
                    welcome_delete_press_count: 0,
                    welcome_delete_last_press: None,
                    config: config.clone(),
                    key_bindings: key_bindings.clone(),
                    voice_config: voice_config.clone(),
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
                    rename_pane_focus_requested: false,
                    registry,
                    features: features.clone(),
                    pending_notifications: Vec::new(),
                    show_notification_modal: false,
                    current_notify_id: None,
                    modal_focused_option: 0,
                    modal_input_buffer: String::new(),
                    quick_note_text: String::new(),
                    quick_note_ctx: QuickNoteCtx::default(),
                    quick_note_dest_cursor: 0,
                    quick_note_sub_cursor: 0,
                    quick_note_children_cache: HashMap::new(),
                    quick_note_children_rx: None,
                    modal_state_notify_id: String::new(),
                    notification_images: HashMap::new(),
                    notifications_enabled,
                    notifications_focus_mode,
                    notifications_interrupt_threshold,
                    focus_stack: Vec::new(),
                    focus_history_depth,
                    pane_focus_history: Vec::new(),
                    pane_focus_future: Vec::new(),
                    navigating_history: false,
                    last_notify_poll: std::time::Instant::now(),
                    host,
                    host_services: crate::host::services::HostServices::new(),
                    background_apps: HashMap::new(),
                    directed_pipes: HashMap::new(),
                    hot_reload: hr_watcher,
                    hot_reload_rx: hr_rx,
                    _config_watcher: cfg_watcher.take(),
                    config_reload_rx: cfg_reload_rx.take(),
                    pending_crash_restarts: HashMap::new(),
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
                    last_logged_focus: None,
                    focus_started_at: None,
                    last_space_press: None,
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
                    root: None,
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
            show_cli_setup_prompt: crate::cli_setup::should_prompt(),
            cli_setup_error: None,
            quitting: false,
            quit_press_count: 0,
            quit_last_press: None,
            show_context_inspector: false,
            inspector_selected_pane: 0,
            welcome_delete_press_count: 0,
            welcome_delete_last_press: None,
            config,
            key_bindings,
            voice_config,
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
            rename_pane_focus_requested: false,
            registry: AppRegistry::load(&std::env::current_dir().unwrap_or_default()),
            features,
            pending_notifications: Vec::new(),
            show_notification_modal: false,
            current_notify_id: None,
            modal_focused_option: 0,
            modal_input_buffer: String::new(),
            quick_note_text: String::new(),
            quick_note_ctx: QuickNoteCtx::default(),
            quick_note_dest_cursor: 0,
            quick_note_sub_cursor: 0,
            quick_note_children_cache: HashMap::new(),
            quick_note_children_rx: None,
            modal_state_notify_id: String::new(),
            notification_images: HashMap::new(),
            notifications_enabled,
            notifications_focus_mode,
            notifications_interrupt_threshold,
            focus_stack: Vec::new(),
            focus_history_depth,
            pane_focus_history: Vec::new(),
            pane_focus_future: Vec::new(),
            navigating_history: false,
            last_notify_poll: std::time::Instant::now(),
            host: crate::host::model::HostModel::new(),
            host_services: crate::host::services::HostServices::new(),
            background_apps: HashMap::new(),
            directed_pipes: HashMap::new(),
            hot_reload: hr_watcher2,
            hot_reload_rx: hr_rx2,
            _config_watcher: cfg_watcher.take(),
            config_reload_rx: cfg_reload_rx.take(),
            pending_crash_restarts: HashMap::new(),
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
            last_logged_focus: None,
            focus_started_at: None,
            last_space_press: None,
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
        let key_bindings = crate::keys::build_key_bindings(config.keybindings.as_ref());
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
                    root: None,
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
            show_context_inspector: false,
            inspector_selected_pane: 0,
            welcome_delete_press_count: 0,
            welcome_delete_last_press: None,
            config,
            key_bindings,
            voice_config: config::VoiceConfig::default(),
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
            rename_pane_focus_requested: false,
            registry: AppRegistry::load_with_global(
                &path,
                &path.join("nonexistent-apps-dir"),
            ),
            features,
            pending_notifications: Vec::new(),
            show_notification_modal: false,
            current_notify_id: None,
            modal_focused_option: 0,
            modal_input_buffer: String::new(),
            quick_note_text: String::new(),
            quick_note_ctx: QuickNoteCtx::default(),
            quick_note_dest_cursor: 0,
            quick_note_sub_cursor: 0,
            quick_note_children_cache: HashMap::new(),
            quick_note_children_rx: None,
            modal_state_notify_id: String::new(),
            notification_images: HashMap::new(),
            notifications_enabled: false,
            notifications_focus_mode: false,
            notifications_interrupt_threshold: 100,
            focus_stack: Vec::new(),
            focus_history_depth: 100,
            pane_focus_history: Vec::new(),
            pane_focus_future: Vec::new(),
            navigating_history: false,
            host: crate::host::model::HostModel::new(),
            host_services: crate::host::services::HostServices::new(),
            background_apps: HashMap::new(),
            directed_pipes: HashMap::new(),
            hot_reload: hr_watcher,
            hot_reload_rx: hr_rx,
            _config_watcher: None,
            config_reload_rx: None,
            pending_crash_restarts: HashMap::new(),
            minimap: crate::minimap::MinimapState::new(),
            last_page_x_per_row: HashMap::new(),
            context_active_window: HashMap::new(),
            minimap_visible_per_context: HashMap::new(),
            next_window_id: 2,
            pane_snapshot_len: 0,
            pane_anims: Vec::new(),
            edge_pulse: None,
            show_cli_setup_prompt: false,
            cli_setup_error: None,
            update_rx: None,
            update_available: None,
            pane_ipc_rx,
            last_logged_focus: None,
            focus_started_at: None,
            last_space_press: None,
        }, pane_ipc_tx)
    }

    /// Add a minimal `ProcessApp` pane directly to window 0 for unit tests.
    /// Returns `(tile_id, pane_id)` — `tile_id` is suitable for `focused_pane` assignments.
    #[cfg(test)]
    pub(crate) fn add_test_pane(&mut self) -> (egui_tiles::TileId, u64) {
        use crate::app_permissions::AppPermissions;
        use crate::process_app::ProcessApp;
        use crate::pane::{AppPane, AppRuntime};

        // Use a simple incrementing id; start high to avoid collisions with HostHarness ids.
        static NEXT_PANE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(10000);
        let pane_id = NEXT_PANE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let (process_app, _draw_tx) = ProcessApp::new_for_test(pane_id, AppPermissions::builtin());
        let app_pane = AppPane {
            id: pane_id,
            runtime: AppRuntime::Process(Box::new(process_app)),
            workspace_root: std::env::temp_dir(),
            permissions: AppPermissions::builtin(),
            manifest_id: "test".to_string(),
            name: "Test App".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
        };

        let win = &mut self.windows[0];
        win.panes.insert(pane_id, crate::pane::Pane::App(Box::new(app_pane)));
        let tile_id = win.tree.tiles.insert_pane(pane_id);
        if win.tree.root.is_none() {
            win.tree.root = Some(tile_id);
        }
        (tile_id, pane_id)
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
        context_id: u64,
        context_name: &str,
    ) -> BackendSettings {
        log::info!("make_backend_settings: pane_id={pane_id} context_id={context_id} context_name={context_name:?}");
        let mut env = shell::build_env();
        env.insert("PLEXI_PANE_ID".into(), pane_id.to_string());
        let socket = crate::config::config_dir()
            .join("notify.sock")
            .to_string_lossy()
            .into_owned();
        env.insert("PLEXI_SOCKET".into(), socket);
        env.insert("PLEXI_CONTEXT_ID".into(), context_id.to_string());
        env.insert("PLEXI_CONTEXT_NAME".into(), context_name.to_string());
        BackendSettings {
            shell: shell::detect_shell(),
            args: vec!["-l".to_string()],
            env,
            dynamic_colors: theme::terminal_dynamic_colors(colors),
            working_directory,
        }
    }

    pub(crate) fn context_name_for(&self, context_id: u64) -> String {
        self.router.iter()
            .find(|c| c.context_id == context_id)
            .map(|c| c.name.clone())
            .unwrap_or_default()
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
                                t.name_locked = !name.is_empty();
                                t.name = if name.is_empty() { None } else { Some(name.clone()) };
                                found = true;
                                break;
                            }
                        }
                    }
                    if !found {
                        log::warn!("pane_ipc: set_pane_title: pane_id={pane_id} not found");
                    }
                }
                crate::app_protocol::HostCommand::ListPanes { response_file } => {
                    log::info!("pane_ipc: kind=list_panes response_file={:?}", response_file);
                    let active_win = self.active_window;
                    let mut entries: Vec<serde_json::Value> = Vec::new();
                    for (win_idx, win) in self.windows.iter().enumerate() {
                        let focused_pane_id = win.focused_pane
                            .and_then(|t| win.tree.tiles.get(t))
                            .and_then(|tile| {
                                if let egui_tiles::Tile::Pane(id) = tile { Some(*id) } else { None }
                            });
                        let context_name = self.router.iter()
                            .find(|ctx| ctx.context_id == win.context_id)
                            .map(|ctx| ctx.name.clone())
                            .unwrap_or_default();
                        for (pane_id, pane) in &win.panes {
                            // Only emit panes that have a corresponding tile in the tree.
                            // win.panes and the tile tree can desync (e.g. from corrupted
                            // restore state); omitting orphaned entries ensures every id
                            // returned here is navigable via pane_focus. (#996)
                            if win.tree.tiles.find_pane(pane_id).is_none() {
                                log::warn!(
                                    "pane_list: pane_id={pane_id} in win.panes but absent \
                                     from tile tree — skipping (desync)"
                                );
                                continue;
                            }
                            let (pane_type, title, cwd) = match pane {
                                crate::pane::Pane::Terminal(t) => {
                                    let name = t.name.clone().unwrap_or_else(|| "terminal".to_string());
                                    let cwd = crate::shell::get_pid_cwd(t.backend.child_pid())
                                        .map(|p| p.to_string_lossy().into_owned());
                                    ("terminal", name, cwd)
                                }
                                crate::pane::Pane::App(a) => {
                                    let cwd = Some(a.workspace_root.to_string_lossy().into_owned());
                                    ("app", a.name.clone(), cwd)
                                }
                            };
                            let focused = win_idx == active_win && focused_pane_id == Some(*pane_id);
                            entries.push(serde_json::json!({
                                "id": pane_id,
                                "type": pane_type,
                                "title": title,
                                "focused": focused,
                                "context_id": win.context_id,
                                "context_name": context_name,
                                "window_id": win.window_id,
                                "cwd": cwd,
                            }));
                        }
                    }
                    let json_str = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string());
                    if let Err(e) = std::fs::write(response_file, &json_str) {
                        log::error!("pane_ipc: list_panes: could not write response file {response_file:?}: {e}");
                    }
                }
                crate::app_protocol::HostCommand::GetPaneInfo { pane_id, response_file } => {
                    log::info!("pane_ipc: kind=get_pane_info pane_id={pane_id} response_file={:?}", response_file);
                    let active_win = self.active_window;
                    let mut found = false;
                    'outer: for (win_idx, win) in self.windows.iter().enumerate() {
                        let focused_pane_id = win.focused_pane
                            .and_then(|t| win.tree.tiles.get(t))
                            .and_then(|tile| {
                                if let egui_tiles::Tile::Pane(id) = tile { Some(*id) } else { None }
                            });
                        if let Some(pane) = win.panes.get(pane_id) {
                            let focused = win_idx == active_win && focused_pane_id == Some(*pane_id);
                            let info = match pane {
                                crate::pane::Pane::Terminal(t) => {
                                    let cwd = crate::shell::get_pid_cwd(t.backend.child_pid())
                                        .map(|p| p.to_string_lossy().into_owned());
                                    serde_json::json!({
                                        "id": pane_id,
                                        "type": "terminal",
                                        "title": t.name.clone().unwrap_or_else(|| "terminal".to_string()),
                                        "focused": focused,
                                        "context_id": win.context_id,
                                        "window_id": win.window_id,
                                        "cwd": cwd,
                                    })
                                }
                                crate::pane::Pane::App(a) => {
                                    serde_json::json!({
                                        "id": pane_id,
                                        "type": "app",
                                        "title": a.name.clone(),
                                        "focused": focused,
                                        "context_id": win.context_id,
                                        "window_id": win.window_id,
                                        "cwd": a.workspace_root.to_string_lossy().as_ref(),
                                        "manifest_id": a.manifest_id.clone(),
                                    })
                                }
                            };
                            let json_str = serde_json::to_string(&info).unwrap_or_else(|_| "{}".to_string());
                            if let Err(e) = std::fs::write(response_file, &json_str) {
                                log::error!("pane_ipc: get_pane_info: could not write response file {:?}: {e}", response_file);
                            }
                            found = true;
                            break 'outer;
                        }
                    }
                    if !found {
                        log::warn!("pane_ipc: get_pane_info: pane_id={pane_id} not found");
                        let json_str = format!("{{\"error\":\"pane {pane_id} not found\"}}");
                        if let Err(e) = std::fs::write(response_file, &json_str) {
                            log::error!("pane_ipc: get_pane_info: could not write error response: {e}");
                        }
                    }
                }
                crate::app_protocol::HostCommand::FocusPane { pane_id } => {
                    log::info!("pane_ipc: kind=focus_pane pane_id={pane_id}");
                    if !self.pane_navigate(*pane_id) {
                        log::warn!("pane_ipc: focus_pane: pane_id={pane_id} not found");
                    }
                }
                crate::app_protocol::HostCommand::ClosePane { pane_id } => {
                    log::info!("pane_ipc: kind=close_pane pane_id={pane_id}");
                    let before: usize = self.windows.iter().map(|w| w.panes.len()).sum();
                    self.close_pane_by_id(*pane_id);
                    let after: usize = self.windows.iter().map(|w| w.panes.len()).sum();
                    if before == after {
                        log::warn!("pane_ipc: close_pane: pane_id={pane_id} not found");
                    }
                }
                crate::app_protocol::HostCommand::SpawnPane { type_id, layout, args, ephemeral, response_file, from_pane_id, cwd, no_focus, .. } => {
                    log::info!("pane_ipc: kind=spawn_pane type_id={type_id} layout={layout:?} ephemeral={ephemeral} no_focus={no_focus} from_pane_id={from_pane_id:?} cwd={cwd:?} response_file={response_file:?}");
                    let new_pane_id = self.host.next_pane_id();

                    // Override focused_pane for the split if from_pane_id is specified,
                    // so the new pane splits the origin pane regardless of which pane has focus.
                    let active = self.active_window;
                    let original_focused = self.windows[active].focused_pane;
                    if let Some(from_id) = from_pane_id {
                        if let Some(tile) = self.windows[active].tree.tiles.find_pane(from_id) {
                            log::info!("pane_ipc: spawn_pane: splitting relative to from_pane_id={from_id}");
                            self.windows[active].focused_pane = Some(tile);
                        } else {
                            log::warn!("pane_ipc: spawn_pane: from_pane_id={from_id} not found, using focused pane");
                        }
                    }

                    if type_id == "terminal" {
                        let layout_str = layout.as_deref().unwrap_or("split_v");
                        let initial_cmd = cmd_from_args(args);
                        let cwd_override: Option<std::path::PathBuf> = cwd.as_deref().map(std::path::PathBuf::from);
                        if layout_str == "new_window" {
                            // Create a new spatial grid window to the right of the
                            // current context row instead of splitting the active pane.
                            let ws_id = self.router.active().context_id;
                            let active_y = self.windows[self.active_window].grid_y;
                            let max_x = self.windows.iter()
                                .filter(|w| w.context_id == ws_id && w.grid_y == active_y)
                                .map(|w| w.grid_x)
                                .max();
                            let new_x = max_x.map(|x| x + 1).unwrap_or(1);
                            log::info!("pane_ipc: spawn_pane terminal layout=new_window grid=({new_x},{active_y}) initial_cmd={initial_cmd:?} ephemeral={ephemeral}");
                            self.create_page_at(new_x, active_y, initial_cmd.as_deref(), *ephemeral);
                        } else if layout_str == "tab" {
                            log::info!("pane_ipc: spawn_pane terminal layout=tab initial_cmd={initial_cmd:?} ephemeral={ephemeral}");
                            self.new_tab(initial_cmd.as_deref(), *ephemeral);
                        } else {
                            let vertical = matches!(layout_str, "split_h" | "split_above");
                            log::info!("pane_ipc: spawn_pane terminal layout={layout_str} vertical={vertical} initial_cmd={initial_cmd:?} ephemeral={ephemeral}");
                            self.split_focused(vertical, initial_cmd.as_deref(), *ephemeral, cwd_override);
                        }
                    } else {
                        self.launch_app_by_id_with_layout(type_id, layout.clone(), args);
                    }

                    // Restore original focus when no_focus is requested or from_pane_id overrode it.
                    if *no_focus || from_pane_id.is_some() {
                        let reason = if *no_focus { "no_focus=true" } else { "from_pane_id override" };
                        log::info!("pane_ipc: spawn_pane: {reason}, retaining focus on pane_id={original_focused:?}");
                        // Also restore active_window — new_window layout switches it to the new window.
                        if *no_focus {
                            self.active_window = active;
                        }
                        self.windows[active].focused_pane = original_focused;
                    }
                    if let Some(rf) = response_file {
                        let json = format!("{{\"pane_id\":{new_pane_id}}}");
                        if let Err(e) = std::fs::write(rf, &json) {
                            log::error!("pane_ipc: spawn_pane: could not write response file: {e}");
                        }
                    }
                }
                crate::app_protocol::HostCommand::SendToPane { pane_id, text, response_file } => {
                    log::info!("pane_ipc: kind=send_to_pane pane_id={pane_id} len={} windows={} response_file={response_file:?}", text.len(), self.windows.len());
                    let text_with_newlines = text.replace("\\n", "\n");
                    let result = match self.windows.iter_mut().find_map(|win| win.panes.get_mut(pane_id)) {
                        None => {
                            log::warn!("pane_ipc: send_to_pane: pane_id={pane_id} not found in any window");
                            Err(format!("pane {pane_id} not found"))
                        }
                        Some(pane) => match pane.as_terminal_mut() {
                            None => {
                                log::warn!("pane_ipc: send_to_pane: pane_id={pane_id} is not a terminal pane");
                                Err(format!("pane {pane_id} is not a terminal pane"))
                            }
                            Some(term) => {
                                term.backend.process_command(egui_term::BackendCommand::Write(
                                    text_with_newlines.into_bytes(),
                                ));
                                Ok(())
                            }
                        },
                    };
                    if let Some(rf) = response_file {
                        let json = match result {
                            Ok(()) => r#"{"ok":true}"#.to_string(),
                            Err(ref msg) => format!("{{\"error\":{}}}", serde_json::to_string(msg).unwrap_or_else(|_| format!("\"{msg}\""))),
                        };
                        if let Err(e) = std::fs::write(rf, &json) {
                            log::error!("pane_ipc: send_to_pane: could not write response file: {e}");
                        }
                    }
                }
                crate::app_protocol::HostCommand::KeyPane { pane_id, key, response_file } => {
                    log::info!("pane_ipc: kind=key_pane pane_id={pane_id} key={key:?}");
                    let result = match self.windows.iter_mut().find_map(|win| win.panes.get_mut(pane_id)) {
                        None => {
                            log::warn!("pane_ipc: key_pane: pane_id={pane_id} not found");
                            Err(format!("pane {pane_id} not found"))
                        }
                        Some(pane) => {
                            if let Some(term) = pane.as_terminal_mut() {
                                let bytes = key_str_to_pty_bytes(key);
                                term.backend.process_command(egui_term::BackendCommand::Write(bytes));
                                Ok(())
                            } else if let Some(app_pane) = pane.as_app_mut() {
                                let (key_str, modifiers) = parse_key_str_to_event(key);
                                app_pane.runtime.queue_outbound_event(
                                    crate::app_protocol::PlexiEvent::Key { key: key_str, modifiers }
                                );
                                Ok(())
                            } else {
                                Err(format!("pane {pane_id}: unknown pane type"))
                            }
                        }
                    };
                    if let Some(rf) = response_file {
                        let json = match &result {
                            Ok(()) => serde_json::json!({"ok": true}).to_string(),
                            Err(msg) => serde_json::json!({"error": msg}).to_string(),
                        };
                        if let Err(e) = std::fs::write(rf, &json) {
                            log::error!("pane_ipc: key_pane: could not write response file: {e}");
                        }
                    }
                }
                crate::app_protocol::HostCommand::CapturePane { pane_id, lines, response_file } => {
                    log::info!("pane_ipc: kind=capture_pane pane_id={pane_id} lines={lines} response_file={:?}", response_file);
                    let result = match self.windows.iter().find_map(|win| win.panes.get(pane_id)) {
                        None => {
                            log::warn!("pane_ipc: capture_pane: pane_id={pane_id} not found");
                            Err(format!("pane {pane_id} not found"))
                        }
                        Some(pane) => match pane.as_terminal() {
                            None => {
                                log::warn!("pane_ipc: capture_pane: pane_id={pane_id} is not a terminal pane");
                                Err(format!("pane {pane_id} is not a terminal pane"))
                            }
                            Some(term) => Ok(term.backend.capture_lines(*lines)),
                        },
                    };
                    let json_str = match result {
                        Ok(captured) => serde_json::to_string(&captured).unwrap_or_else(|_| "[]".to_string()),
                        Err(msg) => serde_json::json!({"error": msg}).to_string(),
                    };
                    if let Err(e) = std::fs::write(response_file, &json_str) {
                        log::error!("pane_ipc: capture_pane: could not write response file {response_file:?}: {e}");
                    }
                }
                crate::app_protocol::HostCommand::Notify {
                    level, title, body, kind, options, input_prompt,
                    required, priority, image_inline, image_pipe_id,
                    timeout_secs, on_dismiss, response_file, scope, ..
                } => {
                    if !self.notifications_enabled {
                        log::info!("pane_ipc: notify dropped — notifications disabled");
                        continue;
                    }
                    let internal_id = format!(
                        "__host__:{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_nanos())
                            .unwrap_or(0)
                    );
                    log::info!(
                        "pane_ipc: kind=notify title={:?} choices={} scope={:?} response_file={:?}",
                        title, options.len(), scope, response_file
                    );
                    self.pending_notifications.push(PendingNotification {
                        notify_id: internal_id.clone(),
                        sender_pane_id: 0,
                        source_context_id: self.router.active().context_id,
                        scope: scope.unwrap_or(crate::app_protocol::NotifyScope::Global),
                        level: level.clone(),
                        title: title.clone(),
                        body: body.clone(),
                        kind: kind.clone(),
                        options: options.clone(),
                        input_prompt: input_prompt.clone(),
                        required: *required,
                        priority: *priority,
                        image_inline: image_inline.clone(),
                        image_pipe_id: image_pipe_id.clone(),
                        response_file: response_file.clone(),
                        timeout_secs: *timeout_secs,
                        on_dismiss: on_dismiss.clone(),
                        enqueued_at: std::time::Instant::now(),
                        tombstoned: false,
                        deliver_after: None,
                    });
                    let should_auto_open = !self.notifications_focus_mode
                        && *priority >= self.notifications_interrupt_threshold;
                    if should_auto_open {
                        self.show_notification_modal = true;
                        if self.current_notify_id.is_none() {
                            self.current_notify_id = Some(internal_id);
                        }
                    }
                }
                crate::app_protocol::HostCommand::CreateContext { root, name } => {
                    log::info!("pane_ipc: kind=create_context root={:?} name={:?}", root, name);
                    if let Some(r) = root {
                        self.new_context_at_path(r.clone());
                    } else {
                        self.new_context();
                    }
                    if let Some(n) = name {
                        let idx = self.router.len() - 1;
                        self.router.get_mut(idx).name = n.clone();
                    }
                    self.save_workspace();
                }
                crate::app_protocol::HostCommand::FocusContext { root } => {
                    log::warn!(
                        "pane_ipc: FocusContext ignored — CWD-based auto-switch removed (root={})",
                        root.display()
                    );
                }
                crate::app_protocol::HostCommand::SetContextRoot { root } => {
                    log::info!("pane_ipc: kind=set_context_root root={}", root.display());
                    self.set_active_context_root(root.clone());
                    self.save_workspace();
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
            let ephemeral = val["ephemeral"].as_bool().unwrap_or(false);
            let args: Vec<String> = val["args"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let cwd_override: Option<std::path::PathBuf> = val["cwd"].as_str().map(std::path::PathBuf::from);
            let no_focus = val["no_focus"].as_bool().unwrap_or(false);
            log::info!("spawn-queue: launching '{type_id}' layout={layout:?} ephemeral={ephemeral} no_focus={no_focus} cwd={cwd_override:?}");
            let active = self.active_window;
            let original_focused = self.windows[active].focused_pane;
            if type_id == "terminal" {
                let layout_str = layout.as_deref().unwrap_or("split_v");
                let vertical = matches!(layout_str, "split_h" | "split_above");
                let initial_cmd = cmd_from_args(&args);
                self.split_focused(vertical, initial_cmd.as_deref(), ephemeral, cwd_override);
            } else {
                self.launch_app_by_id_with_layout(&type_id, layout, &args);
            }
            if no_focus {
                log::info!("spawn-queue: no_focus=true, retaining focus on pane_id={original_focused:?}");
                self.active_window = active;
                self.windows[active].focused_pane = original_focused;
            }
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
                                log::info!("pty: pane {id} process exited ephemeral={}", t.ephemeral);
                                if t.ephemeral {
                                    panes_to_close.push(id);
                                }
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
                    } else {
                        let title_trimmed = title.trim();
                        let osc_enabled = self.config.beta.as_ref()
                            .and_then(|b| b.osc_pane_title)
                            .unwrap_or(false);
                        for win in &mut self.windows {
                            if let Some(pane) = win.panes.get_mut(&id) {
                                if let Some(t) = pane.as_terminal_mut() {
                                    // Always track the raw OSC 2 title for event logging,
                                    // independent of osc_enabled and name_locked.
                                    t.pty_title = if title_trimmed.is_empty() { None } else { Some(title_trimmed.to_string()) };
                                    if osc_enabled {
                                        if t.name_locked {
                                            log::debug!("osc_title: pane {id} name locked, skipping");
                                        } else {
                                            let is_empty = title_trimmed.is_empty();
                                            let already_matches = match &t.name {
                                                None => is_empty,
                                                Some(curr) => !is_empty && curr == title_trimmed,
                                            };
                                            if !already_matches {
                                                t.name = if is_empty { None } else { Some(title_trimmed.to_string()) };
                                                log::debug!("osc_title: pane {id} name set to {:?}", t.name);
                                            }
                                        }
                                    }
                                }
                                break;
                            }
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
    // Visibility by scope:
    //   Window  — only when source_context == active context (default; most restrictive).
    //             Equivalent to Context in today's single-window-per-context model;
    //             the distinction will matter when multi-window contexts land.
    //   Context — same as Window today; reserved for the multi-window distinction.
    //   Global  — always visible.
    // The raw `pending_notifications` Vec stays flat; only the *view* changes
    // with the active workspace.

    /// True when this notification should appear in the current workspace view.
    pub(crate) fn notification_is_visible(&self, n: &PendingNotification) -> bool {
        if n.deliver_after.map_or(false, |t| t > std::time::Instant::now()) {
            return false;
        }
        match n.scope {
            crate::app_protocol::NotifyScope::Global => true,
            crate::app_protocol::NotifyScope::Window
            | crate::app_protocol::NotifyScope::Context => {
                n.source_context_id == self.router.active().context_id
            }
        }
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
    /// Also wakes snoozed notifications whose `deliver_after` has elapsed and
    /// auto-reopens the modal when a high-priority one wakes. Called once per
    /// second from `update()`.
    pub(crate) fn tick_notification_timeouts(&mut self) {
        let now = std::time::Instant::now();
        let threshold = self.notifications_interrupt_threshold;
        let focus_mode = self.notifications_focus_mode;
        let mut expired_ids: Vec<String> = Vec::new();
        let mut woken_priority_met = false;
        // Single mutable pass: wake snoozed entries, collect expired ids.
        for n in &mut self.pending_notifications {
            if let Some(t) = n.deliver_after {
                if t > now {
                    continue; // still snoozed — skip timeout check too
                }
                if !focus_mode && n.priority >= threshold {
                    woken_priority_met = true;
                }
                log::info!("notify:snooze: woke notify_id={}", n.notify_id);
                n.deliver_after = None;
            }
            if let Some(timeout) = n.timeout_secs {
                if n.enqueued_at.elapsed() >= std::time::Duration::from_secs(timeout) {
                    expired_ids.push(n.notify_id.clone());
                }
            }
        }
        if woken_priority_met {
            self.show_notification_modal = true;
            if self.current_notify_id.is_none() {
                self.current_notify_id = self.select_highest_priority();
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
                    host_action: None,
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

    /// Count of window- or context-scoped notifications whose source_context_id == the id of ctx_idx.
    /// Used for per-context sidebar badges on inactive contexts. Global notifications
    /// are excluded — they already appear everywhere via notification_is_visible.
    pub(crate) fn context_notification_count(&self, ctx_idx: usize) -> usize {
        let ctx_id = self.router.get(ctx_idx).context_id;
        self.pending_notifications
            .iter()
            .filter(|n| {
                matches!(
                    n.scope,
                    crate::app_protocol::NotifyScope::Window
                        | crate::app_protocol::NotifyScope::Context
                )
                && n.source_context_id == ctx_id
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
            self.drain_spawn_queue();
            if let Some(ctx_path) = crate::config::take_adopted_context_path() {
                log::info!("adopted context path: {}", ctx_path.display());
                self.new_context_at_path(ctx_path);
                self.save_workspace();
            }
            #[cfg(target_os = "macos")]
            {
                let finder_paths = crate::finder_service::drain();
                if !finder_paths.is_empty() {
                    for path in finder_paths {
                        log::info!("finder_service: opening context for {}", path.display());
                        self.new_context_at_path(path);
                    }
                    self.save_workspace();
                }
            }
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
        self.sync_rename_pane_focus();
        self.sync_context_rename_focus();
        self.sync_cli_setup_prompt_focus();
        self.sync_context_inspector_focus();

        // If an overlay owns input, render it FIRST so its widgets (the
        // notification modal's TextEdit for the `input` kind, the palette's
        // search field, the rename input) can read keystrokes before we
        // drain. Then drain the keyboard buffer so downstream readers —
        // focused app (`dispatch_app_key_events`), terminal backends,
        // `keys::poll_actions` — see only the global allowlist (Cmd+Q,
        // Cmd+W, Cmd+Shift+A, Cmd+Shift+L/H).
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
                Some(FocusLayer::RenamePane) => {
                    self.draw_rename_pane_overlay(ctx);
                }
                Some(FocusLayer::ContextRename) => {
                    self.draw_rename_context_overlay(ctx);
                }
                Some(FocusLayer::QuickNote) => {
                    self.draw_quick_note_modal(ctx);
                }
                Some(FocusLayer::QuickNoteDestination) => {
                    self.draw_quick_note_destination(ctx);
                }
                Some(FocusLayer::QuickNoteSubDestination(path)) => {
                    let path = path.clone();
                    self.draw_quick_note_menu(ctx, &path);
                }
                Some(FocusLayer::CliSetupPrompt) => {
                    self.draw_cli_setup_modal(ctx);
                }
                Some(FocusLayer::ContextInspector) => {
                    self.draw_context_inspector(ctx);
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
            self.sync_rename_pane_focus();
            self.sync_context_rename_focus();
            self.sync_cli_setup_prompt_focus();
            self.sync_context_inspector_focus();
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
                    from_pane_id,
                    request_id,
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
                                            request_id: request_id.clone(),
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

                    // Capture requesting_pane_id from the CURRENT focused pane (the calling app pane).
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

                    // Override focused_pane for the split if from_pane_id is specified.
                    let original_focused = self.windows[active].focused_pane;
                    if let Some(from_id) = from_pane_id {
                        if let Some(tile) = self.windows[active].tree.tiles.find_pane(&from_id) {
                            log::info!("SpawnPane: splitting relative to pane_id={from_id}");
                            self.windows[active].focused_pane = Some(tile);
                        } else {
                            log::warn!("SpawnPane: from_pane_id={from_id} not found, using focused pane");
                        }
                    }

                    // Predict the pane id that will be allocated (next_pane_id peeks without allocating).
                    let new_pane_id = self.host.next_pane_id();
                    if type_id == "terminal" {
                        // "terminal" is a builtin pane type, not in the app registry.
                        // split_focused uses inverted LinearDir vs split_with_new_pane:
                        //   split_focused(false) → insert_horizontal_tile → side-by-side (RIGHT)
                        //   split_focused(true)  → insert_vertical_tile   → stacked (BELOW)
                        // So: split_v (right) → false, split_h/split_above (below) → true.
                        let vertical = matches!(layout.as_str(), "split_h" | "split_above");
                        let initial_cmd = cmd_from_args(&effective_args);
                        log::info!(
                            "SpawnPane: terminal layout='{layout}' vertical={vertical} pane_id={new_pane_id} initial_cmd={initial_cmd:?}"
                        );
                        // SDK-spawned terminal with a cmd closes on exit (matches historical behavior).
                        // CLI terminal uses the ephemeral flag exclusively — cmd alone does not close.
                        self.split_focused(vertical, initial_cmd.as_deref(), initial_cmd.is_some(), None);
                    } else {
                        self.launch_app_by_id_with_layout(&type_id, Some(layout), &effective_args);
                        log::info!("SpawnPane: launched '{type_id}' pane_id={new_pane_id}");
                    }

                    // Restore focused_pane after the split so the coordinator app keeps focus.
                    self.windows[active].focused_pane = original_focused;

                    // Send PaneSpawned back to the requesting pane.
                    if let Some(req_pane_id) = requesting_pane_id {
                        let active = self.active_window;
                        if let Some(pane) = self.windows[active].panes.get_mut(&req_pane_id) {
                            if let Some(a) = pane.as_app_mut() {
                                a.runtime.queue_outbound_event(
                                    crate::app_protocol::PlexiEvent::PaneSpawned {
                                        pane_id: new_pane_id,
                                        request_id,
                                    },
                                );
                            }
                        }
                    }
                }
                AppCommand::CdRequest { cwd, sender_pane_id } => {
                    let active = self.active_window;
                    let escaped = cwd.replace('\'', "'\\''");
                    let cd_cmd = format!("\x15cd '{}'\n", escaped);
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
                            log::info!(
                                "file_browser: CdRequest synced cwd '{}' to terminal pane {}",
                                cwd,
                                tid
                            );
                        }
                    }
                }
                AppCommand::Notify(_) => {}
                AppCommand::ShowNotification {
                    notify_id,
                    sender_pane_id,
                    source_context_id,
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
                    // Capture scope/source_context_id before they move into the struct.
                    let notif_scope = scope;
                    let notif_source_ctx = source_context_id;
                    // Strip any per-option shortcut that conflicts with navigation keys.
                    let options: Vec<crate::app_protocol::NotifyOption> = options.into_iter().map(|mut opt| {
                        if let Some(ref sc) = opt.shortcut.clone() {
                            if crate::app_protocol::is_reserved_shortcut(sc) {
                                log::warn!(
                                    "notify:shortcut: app pane {} sent reserved shortcut {:?} on option {:?} — stripped",
                                    sender_pane_id, sc, opt.label
                                );
                                opt.shortcut = None;
                            }
                        }
                        opt
                    }).collect();
                    self.pending_notifications.push(PendingNotification {
                        notify_id,
                        sender_pane_id,
                        source_context_id,
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
                        deliver_after: None,
                    });
                    // Auto-open rules:
                    //   1. Visibility — Global always; Window/Context only when
                    //      source_context_id == active context id.
                    //   2. focus_mode off — the global mute gate.
                    //   3. priority >= interrupt_threshold — don't
                    //      auto-open low-urgency notifications like
                    //      "note saved".
                    // If any gate fails, the notification still queues
                    // (badge ticks) but the modal doesn't pop.
                    let is_visible = match notif_scope {
                        crate::app_protocol::NotifyScope::Global => true,
                        crate::app_protocol::NotifyScope::Window
                        | crate::app_protocol::NotifyScope::Context => {
                            notif_source_ctx == self.router.active().context_id
                        }
                    };
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
                AppCommand::DeliverNotifyAction { pane_id, notify_id, action_label, value, response_file, host_action } => {
                    log::info!(
                        "notify:action: pane_id={pane_id} notify_id={notify_id:?} value={value:?} host_action={host_action:?}"
                    );
                    // Execute host-side action synchronously before writing the response
                    // file so the navigation is complete before the shell unblocks.
                    if let Some(ref action) = host_action {
                        if let Some(id_str) = action.strip_prefix("pane_focus:") {
                            if let Ok(pane_id_target) = id_str.parse::<u64>() {
                                self.pane_navigate(pane_id_target);
                            } else {
                                log::warn!("notify:action: pane_focus: invalid pane_id {:?}", id_str);
                            }
                        } else {
                            log::warn!("notify:action: unknown host_action {:?}", action);
                        }
                    }
                    if let Some(rf) = &response_file {
                        let content = value.as_deref().unwrap_or("");
                        let tmp = format!("{rf}.tmp");
                        match std::fs::write(&tmp, content).and_then(|_| std::fs::rename(&tmp, rf)) {
                            Ok(_) => log::info!("notify:action: wrote {:?} to {:?}", content, rf),
                            Err(e) => log::warn!("notify:action: failed to write response file {:?}: {e}", rf),
                        }
                    }
                    // Search all windows for the sender pane — it may not be in the
                    // active context (cross-context notification path).
                    let window_idx = self.windows.iter().position(|w| w.panes.contains_key(&pane_id));
                    if let Some(win_idx) = window_idx {
                        if let Some(pane) = self.windows[win_idx].panes.get_mut(&pane_id) {
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
                    sender_pane_id,
                    terminal_pane_id,
                    command,
                    echo,
                } => {
                    self.dispatch_run_in_linked_terminal(
                        sender_pane_id,
                        terminal_pane_id,
                        command,
                        echo,
                    );
                }
                AppCommand::InsertPathToken {
                    sender_pane_id,
                    terminal_pane_id,
                    path,
                    mode,
                } => {
                    self.dispatch_insert_path_token(sender_pane_id, terminal_pane_id, path, mode);
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
                AppCommand::OpenArtifact { sender_pane_id, path, mode } => {
                    self.dispatch_open_artifact(sender_pane_id, path, mode);
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

        // Double-spacebar scratchpad trigger — first Space passes through to the terminal;
        // the second Space within 250ms is consumed and opens the scratchpad.
        if !app_active && !keyboard_capture_active {
            let space_pressed = ctx.input(|i| {
                i.events.iter().any(|e| matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::Space,
                        pressed: true,
                        repeat: false,
                        ..
                    }
                ))
            });
            if space_pressed {
                let now = std::time::Instant::now();
                if let Some(last) = self.last_space_press {
                    if now.duration_since(last) < std::time::Duration::from_millis(250) {
                        log::info!("scratchpad: double-spacebar detected — opening");
                        self.last_space_press = None;
                        // Consume ALL pending Space keydown events so none leak into
                        // the TextEdit on its first frame.
                        ctx.input_mut(|i| {
                            i.events.retain(|e| !matches!(
                                e,
                                egui::Event::Key {
                                    key: egui::Key::Space,
                                    pressed: true,
                                    repeat: false,
                                    ..
                                }
                            ));
                        });
                        self.open_scratchpad();
                    } else {
                        self.last_space_press = Some(now);
                    }
                } else {
                    self.last_space_press = Some(now);
                }
            }
        }

        // Handle keyboard shortcuts
        let modal_open = self.input_captured_by_overlay();
        for action in keys::poll_actions(ctx, &self.key_bindings, app_active, keyboard_capture_active, modal_open, self.show_shortcuts) {
            match action {
                Action::SplitHorizontal => {
                    self.windows[self.active_window].zoomed_pane = None;
                    self.ctx.memory_mut(|m| { if let Some(id) = m.focused() { m.surrender_focus(id); } });
                    self.split_focused(false, None, false, None);
                    self.save_workspace();
                }
                Action::SplitVertical => {
                    self.windows[self.active_window].zoomed_pane = None;
                    self.ctx.memory_mut(|m| { if let Some(id) = m.focused() { m.surrender_focus(id); } });
                    self.split_focused(true, None, false, None);
                    self.save_workspace();
                }
                Action::SplitRight => {
                    self.windows[self.active_window].zoomed_pane = None;
                    self.ctx.memory_mut(|m| { if let Some(id) = m.focused() { m.surrender_focus(id); } });
                    self.split_focused_mirror(crate::host::command::Placement::Right);
                    self.save_workspace();
                }
                Action::SplitDown => {
                    self.windows[self.active_window].zoomed_pane = None;
                    self.ctx.memory_mut(|m| { if let Some(id) = m.focused() { m.surrender_focus(id); } });
                    self.split_focused_mirror(crate::host::command::Placement::Below);
                    self.save_workspace();
                }
                Action::Navigate(dir) => {
                    let was_zoomed = self.windows[self.active_window].zoomed_pane.is_some();
                    let old_focus = self.windows[self.active_window].focused_pane;
                    let old_window_id = self.windows[self.active_window].window_id;
                    self.navigate(dir);
                    if was_zoomed {
                        let new_pane = self.windows[self.active_window].focused_pane;
                        self.windows[self.active_window].zoomed_pane = new_pane;
                        log::info!("zoom: navigate — new zoomed pane={new_pane:?}");
                        self.ctx.memory_mut(|m| {
                            if let Some(id) = m.focused() {
                                m.surrender_focus(id);
                            }
                        });
                    }
                    let new_window_id = self.windows[self.active_window].window_id;
                    let new_focus = self.windows[self.active_window].focused_pane;
                    if new_window_id != old_window_id || new_focus != old_focus {
                        self.push_focus_history(old_window_id, old_focus);
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
                            let moved = match dir {
                                crate::keys::Direction::Down => {
                                    self.move_focused_pane_to_row_boundary(true)
                                }
                                crate::keys::Direction::Up => {
                                    self.move_focused_pane_to_row_boundary(false)
                                }
                                _ => self.move_focused_pane_to_adjacent_window(dir),
                            };
                            if moved {
                                self.ctx.request_repaint();
                            } else if let Some(focused) =
                                self.windows[self.active_window].focused_pane
                            {
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
                    if !self.try_nav_back_focused() {
                        self.step_focus_history_back();
                    }
                    log::info!("nav: back-app — window={}", self.active_window);
                }
                Action::FocusHistoryForward => {
                    self.step_focus_history_forward();
                }
                Action::NewTab => {
                    self.new_tab(None, false);
                    self.save_workspace();
                }
                Action::ToggleZoom => {
                    let ctx = &mut self.windows[self.active_window];
                    if let Some(focused) = ctx.focused_pane {
                        if ctx.zoomed_pane == Some(focused) {
                            ctx.zoomed_pane = None;
                            log::info!("zoom: toggle off — pane={focused:?}");
                        } else {
                            ctx.zoomed_pane = Some(focused);
                            log::info!("zoom: toggle on — pane={focused:?}");
                        }
                        self.ctx.memory_mut(|m| {
                            if let Some(id) = m.focused() {
                                m.surrender_focus(id);
                            }
                        });
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
                            self.rename_pane_focus_requested = false;
                            // Sync the focus layer immediately so `input_captured_by_overlay()`
                            // is accurate for the rest of this frame — without this, there is a
                            // one-frame window where `renaming_pane` is Some but the focus layer
                            // has not been pushed yet.
                            self.sync_rename_pane_focus();
                            log::info!("rename_pane: opened for pane {pane_id:?}");
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
                    log::info!("tab: next — window={}", self.active_window);
                }
                Action::PrevTab => {
                    self.cycle_tab(false);
                    log::info!("tab: prev — window={}", self.active_window);
                }
                Action::FirstTab => {
                    self.jump_to_tab(0);
                    log::info!("tab: first — window={}", self.active_window);
                }
                Action::LastTab => {
                    self.jump_to_tab(usize::MAX);
                    log::info!("tab: last — window={}", self.active_window);
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
                    self.open_quick_note_modal();
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
                Action::RenameContext => {
                    let ctx_idx = self.router.active_idx();
                    self.rename_buffer = self.router.active().name.clone();
                    self.renaming_window = Some(ctx_idx);
                    log::info!(
                        "RenameContext: opening rename for context {:?} (idx {})",
                        self.router.active().name,
                        ctx_idx
                    );
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
                Action::ContextInspector => {
                    self.show_context_inspector = !self.show_context_inspector;
                    self.inspector_selected_pane = 0;
                    log::info!("ContextInspector: toggled to {}", self.show_context_inspector);
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

        // Crash-resilient dev reload (#1055): auto-restart watched panes that
        // have crashed, after a 2s delay so the developer can read the traceback.
        self.drain_crash_restarts();

        // Reload configuration from disk when the user clicks
        // "Reload Configuration" in the app menu.
        crate::macos_menu::apply_version_title_once();
        if crate::macos_menu::take_reload_config_flag() {
            self.reload_config();
        }

        // Config hot-reload (#1115): drain filesystem watcher signals.
        let config_changed = self
            .config_reload_rx
            .as_ref()
            .map_or(false, |rx| {
                let hit = rx.try_recv().is_ok();
                if hit {
                    while rx.try_recv().is_ok() {}
                }
                hit
            });
        if config_changed {
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
                        left: 80,
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
                    if self.router.len() > 1 {
                        let delete_pressed = ctx.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
                                || input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                        });
                        if delete_pressed {
                            let now = std::time::Instant::now();
                            let elapsed = self
                                .welcome_delete_last_press
                                .map(|t| now.duration_since(t))
                                .unwrap_or(std::time::Duration::MAX);
                            if elapsed > std::time::Duration::from_millis(1500) {
                                self.welcome_delete_press_count = 0;
                            }
                            self.welcome_delete_press_count += 1;
                            self.welcome_delete_last_press = Some(now);
                            if self.welcome_delete_press_count >= 3 {
                                let ctx_idx = self.router.active_idx();
                                log::info!(
                                    "welcome_delete: triple-tap delete context idx={ctx_idx} name={:?}",
                                    self.router.active().name
                                );
                                self.welcome_delete_press_count = 0;
                                self.welcome_delete_last_press = None;
                                self.delete_context(ctx_idx);
                                self.save_workspace();
                                return;
                            }
                        }
                    }
                    self.draw_welcome_screen(ui);
                    if self.welcome_delete_press_count > 0 {
                        let timed_out = self
                            .welcome_delete_last_press
                            .map(|t| t.elapsed() > std::time::Duration::from_millis(1500))
                            .unwrap_or(false);
                        if timed_out {
                            self.welcome_delete_press_count = 0;
                            self.welcome_delete_last_press = None;
                        } else {
                            self.draw_welcome_delete_overlay(ctx);
                            ctx.request_repaint_after(std::time::Duration::from_millis(100));
                        }
                    }
                    return;
                }

                // Build per-pane notification counts before `ctx` mutably borrows
                // `self.windows`. Inline the notification_is_visible logic to
                // avoid a borrow conflict with the mutable ctx reference below.
                let notify_counts: std::collections::HashMap<u64, usize> = {
                    let active_context_id = self.router.active().context_id;
                    let mut counts = std::collections::HashMap::new();
                    for n in &self.pending_notifications {
                        let visible = match n.scope {
                            crate::app_protocol::NotifyScope::Global => true,
                            crate::app_protocol::NotifyScope::Window
                            | crate::app_protocol::NotifyScope::Context => {
                                n.source_context_id == active_context_id
                            }
                        };
                        if visible {
                            *counts.entry(n.sender_pane_id).or_insert(0) += 1;
                        }
                    }
                    counts
                };

                // Capture focus before rendering so we can record a history entry
                // if the user clicks a different pane this frame.
                let canvas_old_focus = self.windows[self.active_window].focused_pane;
                let canvas_old_window_id = self.windows[self.active_window].window_id;

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
                        self.ctx.memory_mut(|m| {
                            if let Some(id) = m.focused() {
                                m.surrender_focus(id);
                            }
                        });
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

                // Propagate pre-computed notification counts into each app pane so
                // ProcessApp can render the per-pane chrome badge without
                // holding a reference to PlexiApp.
                for pane in ctx.panes.values_mut() {
                    if let Some(app_pane) = pane.as_app_mut() {
                        let count = notify_counts.get(&app_pane.id).copied().unwrap_or(0);
                        app_pane.runtime.set_pending_notification_count(count);
                    }
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
                    workspace_root: self.router.active().root.clone().or_else(crate::config::active_workspace_root),
                };
                log::debug!("[DRAG] tiling: start (zoomed={}, hovered_files={hovered_files})", zoomed_pane.is_some());
                ui.scope(|ui| {
                    // When a pane is zoomed, the resize handles inside egui_tiles' linear
                    // container are still rendered unconditionally. Disabling the UI prevents
                    // their interact() calls from returning hovered/dragged, blocking background
                    // pane resizing while the zoom overlay is active.
                    if zoomed_pane.is_some() {
                        ui.disable();
                    }
                    ctx.tree.ui(&mut behavior, ui);
                });
                log::debug!("[DRAG] tiling: done");

                // Paint the active pane focus outline using the parent painter which
                // has the full window clip rect. paint_on_top_of_tile cannot do this —
                // its painter is clipped to the tile rect, making Outside strokes invisible.
                if zoomed_pane.is_none() && !suppress_focus {
                    if let Some(tile_id) = ctx.focused_pane {
                        if let Some(rect) = ctx.tree.tiles.rect(tile_id) {
                            let gap = 4.0;
                            let stroke = egui::Stroke::new(gap, self.colors.accent);
                            ui.painter().rect_stroke(
                                rect,
                                0.0,
                                stroke,
                                egui::StrokeKind::Outside,
                            );
                        }
                    }
                }

                let canvas_focus_changed = if let Some(new) = behavior.new_focused {
                    let changed = Some(new) != canvas_old_focus;
                    ctx.focused_pane = Some(new);
                    changed
                } else {
                    false
                };

                let should_close_exited = behavior.close_exited.is_some();

                // Draw zoom overlay if a pane is zoomed
                if let Some(zoomed_tile) = zoomed_pane {
                    if let Some(Tile::Pane(pane_id)) = ctx.tree.tiles.get(zoomed_tile) {
                        let pane_id = *pane_id;
                        let panel_rect = ui.max_rect();
                        let zoomed_tab_info = behavior.tab_info.get(&zoomed_tile).copied();
                        let zoomed_pane_name = behavior.pane_names.get(&pane_id).cloned();

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
                        // Render name bar outside the Frame so the terminal's background
                        // fill inside the Frame cannot paint over it. The Frame gets its
                        // own child_ui scoped to the area below the name bar, so there
                        // is no cursor/spacing gap between them.
                        let has_name = zoomed_pane_name.is_some();
                        const NAME_BAR_HEIGHT: f32 = 20.0;
                        if has_name {
                            let bar_rect = egui::Rect::from_min_size(
                                inner_rect.min,
                                egui::vec2(inner_rect.width(), NAME_BAR_HEIGHT),
                            );
                            child_ui.painter().rect_filled(bar_rect, 0.0, self.colors.terminal_bg);
                            if let Some((active_idx, count)) = zoomed_tab_info {
                                crate::tiling::paint_tab_dots(
                                    child_ui.painter(),
                                    bar_rect.left(),
                                    bar_rect.center().y,
                                    active_idx,
                                    count,
                                    self.colors.accent,
                                    self.colors.bg_active,
                                );
                            }
                            if let Some(ref name) = zoomed_pane_name {
                                child_ui.painter().text(
                                    bar_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    name,
                                    egui::FontId::proportional(11.0),
                                    self.colors.text_dim,
                                );
                            }
                        }
                        // Frame rect starts exactly where the name bar ends (or at the
                        // top of inner_rect when there is no name bar), so no gap appears.
                        let frame_rect = if has_name {
                            inner_rect.with_min_y(inner_rect.min.y + NAME_BAR_HEIGHT)
                        } else {
                            inner_rect
                        };
                        let mut frame_ui = ui.new_child(egui::UiBuilder::new().max_rect(frame_rect));
                        frame_ui.set_opacity(0.88);
                        egui::Frame::new()
                            .fill(self.colors.terminal_bg)
                            .inner_margin(egui::Margin::same(8))
                            .show(&mut frame_ui, |ui| {
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
                                            // Reserve space for tab dots when no name bar
                                            if !has_name && zoomed_tab_info.is_some() {
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

                                // Draw tab indicator dots for unnamed panes in a tab group
                                if !has_name {
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

                // Record canvas click focus change in pane history (ctx borrow released above).
                if canvas_focus_changed {
                    self.push_focus_history(canvas_old_window_id, canvas_old_focus);
                }
            });

        // Shortcuts overlay
        self.draw_shortcuts_overlay(ctx);

        // Changelog overlay
        self.draw_changelog_overlay(ctx);

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

        // Re-request focus for the palette search field after all pane rendering.
        // App panes call request_focus() on their TextInput widgets during
        // CentralPanel rendering, and egui focus is last-write-wins — without
        // this, a keyboard-capture app pane steals focus from the palette every
        // frame, making the search field non-typeable even though it's visible.
        if self.show_command_palette {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("palette_search")));
        }

        // Same pattern: QuickNote compose mode needs re-focus every frame so
        // pane TextInput widgets rendered in CentralPanel can't steal it.
        if matches!(self.focus_stack.last(), Some(FocusLayer::QuickNote)) {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("quick_note_text")));
        }

        // Detect genuine pane focus transitions and emit FocusChanged events.
        // Comparing at frame-end means temporary save/restore patterns in
        // canvas_bindings are invisible — focused_pane holds the settled value here.
        // Uses stable window_id (not the vector index) so the key survives window removal.
        let current_focus = self.windows
            .get(self.active_window)
            .and_then(|win| win.focused_pane.map(|tile| (win.window_id, tile)));
        if current_focus != self.last_logged_focus {
            if let Some((window_id, tile_id)) = self.last_logged_focus {
                let duration_secs = self.focus_started_at
                    .map(|t| t.elapsed().as_secs())
                    .unwrap_or(0);
                self.emit_focus_changed_for_tile(window_id, tile_id, duration_secs);
            }
            self.last_logged_focus = current_focus;
            self.focus_started_at = Some(std::time::Instant::now());
        }

        let frame_ms = _frame_start.elapsed().as_millis();
        if frame_ms > 50 {
            log::warn!("slow frame: {}ms", frame_ms);
        }
    }

    fn on_exit(&mut self) {
        if let Some((window_id, tile_id)) = self.last_logged_focus {
            let duration_secs = self.focus_started_at
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0);
            log::info!("focus_changed: shutdown — banking final session duration_secs={duration_secs}");
            self.emit_focus_changed_for_tile(window_id, tile_id, duration_secs);
        }
    }
}

impl PlexiApp {
    /// Collect metadata for the pane at `tile_id` in the window identified by
    /// stable `window_id` and emit a `FocusChanged` event. Called when the
    /// focused pane changes and on shutdown.
    fn emit_focus_changed_for_tile(&self, window_id: u64, tile_id: egui_tiles::TileId, duration_secs: u64) {
        use egui_tiles::Tile;
        let Some(win) = self.windows.iter().find(|w| w.window_id == window_id) else { return };
        let pane_id = match win.tree.tiles.get(tile_id) {
            Some(Tile::Pane(id)) => *id,
            _ => return,
        };
        let Some(pane) = win.panes.get(&pane_id) else { return };
        let context_name = self.context_name_for(win.context_id);

        let (cwd, pty_title, pane_name, app_type_id) = match pane {
            crate::pane::Pane::Terminal(t) => {
                let cwd = crate::shell::get_pid_cwd(t.backend.child_pid())
                    .map(|p| p.to_string_lossy().into_owned());
                (cwd, t.pty_title.clone(), t.name.clone(), None)
            }
            crate::pane::Pane::App(a) => {
                let cwd = Some(a.workspace_root.to_string_lossy().into_owned());
                let type_id = Some(a.manifest_id.clone());
                (cwd, None, None, type_id)
            }
        };

        log::info!(
            "focus_changed: pane_id={pane_id} context={context_name:?} duration_secs={duration_secs} pty_title={pty_title:?} pane_name={pane_name:?} app_type_id={app_type_id:?}"
        );
        crate::event_log::emit(crate::event_log::HostEvent::FocusChanged {
            pane_id,
            context_name,
            cwd,
            pty_title,
            pane_name,
            app_type_id,
            duration_secs,
            timestamp: crate::event_log::now_timestamp(),
        });
    }

    /// True when a modal overlay owns keyboard input. Used by `update()` to
    /// drain remaining key events after the overlay has rendered so panes see
    /// an empty input buffer this frame.
    pub(crate) fn input_captured_by_overlay(&self) -> bool {
        matches!(
            self.focus_stack.last(),
            Some(FocusLayer::NotificationModal)
                | Some(FocusLayer::ConfirmClose)
                | Some(FocusLayer::CommandPalette)
                | Some(FocusLayer::RenamePane)
                | Some(FocusLayer::ContextRename)
                | Some(FocusLayer::QuickNote)
                | Some(FocusLayer::QuickNoteDestination)
                | Some(FocusLayer::QuickNoteSubDestination(_))
                | Some(FocusLayer::CliSetupPrompt)
                | Some(FocusLayer::ContextInspector)
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

    pub(crate) fn push_focus_history(&mut self, window_id: u64, old_focus: Option<egui_tiles::TileId>) {
        if self.navigating_history {
            return;
        }
        let Some(tile_id) = old_focus else { return };
        self.pane_focus_history.push((window_id, tile_id));
        if self.pane_focus_history.len() > self.focus_history_depth {
            self.pane_focus_history.remove(0);
        }
        self.pane_focus_future.clear();
        log::info!("focus_history: recorded window={window_id} tile={tile_id:?} history_len={}", self.pane_focus_history.len());
    }

    /// Step backward through pane focus history (Cmd+[).
    /// Skips stale entries where the window or tile no longer exists.
    pub(crate) fn step_focus_history_back(&mut self) {
        self.navigating_history = true;
        loop {
            let Some((window_id, tile_id)) = self.pane_focus_history.pop() else {
                log::info!("focus_history: back exhausted");
                self.navigating_history = false;
                return;
            };
            let window_idx = self.windows.iter().position(|w| w.window_id == window_id);
            let Some(idx) = window_idx else {
                log::info!("focus_history: skipping stale entry window={window_id} tile={tile_id:?} (window gone)");
                continue;
            };
            if self.windows[idx].tree.tiles.get(tile_id).is_none() {
                log::info!("focus_history: skipping stale entry window={window_id} tile={tile_id:?} (tile gone)");
                continue;
            }
            // Save current focus to future stack before navigating.
            let current_window_id = self.windows[self.active_window].window_id;
            if let Some(current_tile) = self.windows[self.active_window].focused_pane {
                self.pane_focus_future.push((current_window_id, current_tile));
                if self.pane_focus_future.len() > self.focus_history_depth {
                    self.pane_focus_future.remove(0);
                }
            }
            self.windows[idx].focused_pane = Some(tile_id);
            self.active_window = idx;
            log::info!("focus_history: back — to window={window_id} tile={tile_id:?} history_len={}", self.pane_focus_history.len());
            break;
        }
        self.navigating_history = false;
    }

    /// Step forward through pane focus future (Cmd+]).
    /// Skips stale entries where the window or tile no longer exists.
    pub(crate) fn step_focus_history_forward(&mut self) {
        self.navigating_history = true;
        loop {
            let Some((window_id, tile_id)) = self.pane_focus_future.pop() else {
                log::info!("focus_history: forward exhausted");
                self.navigating_history = false;
                return;
            };
            let window_idx = self.windows.iter().position(|w| w.window_id == window_id);
            let Some(idx) = window_idx else {
                log::info!("focus_history: skipping stale entry window={window_id} tile={tile_id:?} (window gone)");
                continue;
            };
            if self.windows[idx].tree.tiles.get(tile_id).is_none() {
                log::info!("focus_history: skipping stale entry window={window_id} tile={tile_id:?} (tile gone)");
                continue;
            }
            // Save current focus to history stack before navigating.
            let current_window_id = self.windows[self.active_window].window_id;
            if let Some(current_tile) = self.windows[self.active_window].focused_pane {
                self.pane_focus_history.push((current_window_id, current_tile));
                if self.pane_focus_history.len() > self.focus_history_depth {
                    self.pane_focus_history.remove(0);
                }
            }
            self.windows[idx].focused_pane = Some(tile_id);
            self.active_window = idx;
            log::info!("focus_history: forward — to window={window_id} tile={tile_id:?} future_len={}", self.pane_focus_future.len());
            break;
        }
        self.navigating_history = false;
    }

    /// Re-read configuration from disk and apply changes that can take
    /// effect without a restart (theme, font size, notification settings,
    /// confirmation toggles). Logs the reload so the user knows it worked.
    pub(crate) fn reload_config(&mut self) {
        let active_workspace = crate::config::active_workspace_root();

        let mut all_diags = crate::config::validate_from_path(&crate::config::config_path());
        if let Some(root) = active_workspace.as_ref() {
            let project_path = root.join(".plexi").join("config.toml");
            all_diags.extend(crate::config::validate_from_path(&project_path));
        }

        let has_errors = all_diags.iter().any(|d| d.is_error());

        let warnings: Vec<_> = all_diags.iter().filter(|d| !d.is_error()).collect();
        if !warnings.is_empty() {
            let body = warnings.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n");
            log::warn!("config: unknown keys found:\n{body}");
        }

        if has_errors {
            let error_msg = all_diags
                .iter()
                .filter(|d| d.is_error())
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            log::warn!("config: parse error, keeping current config:\n{error_msg}");
            let notify_id = format!(
                "config-error-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0)
            );
            self.pending_notifications.push(PendingNotification {
                notify_id: notify_id.clone(),
                sender_pane_id: 0,
                source_context_id: 0,
                level: "error".to_string(),
                title: "Config Error".to_string(),
                body: error_msg,
                kind: crate::app_protocol::NotifyKind::Message,
                options: vec![],
                input_prompt: None,
                required: false,
                priority: 100,
                scope: crate::app_protocol::NotifyScope::Global,
                image_inline: None,
                image_pipe_id: None,
                response_file: None,
                timeout_secs: None,
                on_dismiss: None,
                enqueued_at: std::time::Instant::now(),
                tombstoned: false,
                deliver_after: None,
            });
            if !self.notifications_focus_mode {
                self.show_notification_modal = true;
                if self.current_notify_id.is_none() {
                    self.current_notify_id = Some(notify_id);
                }
            }
            return;
        }

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

        self.focus_history_depth = fresh.focus_history_depth.unwrap_or(100);

        // Feature flags
        self.features = crate::features::FeatureFlags::from_config(&fresh);

        // Replace the cached config
        self.config = fresh;
        self.key_bindings = crate::keys::build_key_bindings(self.config.keybindings.as_ref());
        log::info!("keybindings: rebuilt after config reload");

        // Voice config
        let fresh_voice =
            crate::config::VoiceConfig::load_with_workspace(active_workspace.as_deref());
        log::info!(
            "voice: config reloaded — enabled={}",
            fresh_voice.is_enabled()
        );
        self.voice_config = fresh_voice;

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

    pub(crate) fn sync_cli_setup_prompt_focus(&mut self) {
        let should_own = self.show_cli_setup_prompt;
        let has_layer = self
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::CliSetupPrompt);
        if should_own && !has_layer {
            log::info!("cli_setup: focus captured by CliSetupPrompt layer");
            self.push_focus_layer(FocusLayer::CliSetupPrompt);
        } else if !should_own && has_layer {
            log::info!("cli_setup: CliSetupPrompt focus layer released");
            // Use retain rather than pop_focus_layer so stale entries are removed
            // even if another layer was pushed on top (e.g. via rapid state change).
            self.focus_stack.retain(|l| *l != FocusLayer::CliSetupPrompt);
        }
    }

    pub(crate) fn sync_context_inspector_focus(&mut self) {
        let should_own = self.show_context_inspector;
        let has_layer = self
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::ContextInspector);
        if should_own && !has_layer {
            self.push_focus_layer(FocusLayer::ContextInspector);
        } else if !should_own && has_layer {
            self.focus_stack
                .retain(|l| *l != FocusLayer::ContextInspector);
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
            // Explicitly surrender egui focus from palette_search so AccessKit
            // doesn't hold a stale focused node ID after the widget is gone.
            self.ctx.memory_mut(|m| {
                let palette_id = egui::Id::new("palette_search");
                if m.focused() == Some(palette_id) {
                    log::info!("palette: surrendering palette_search focus on dismiss");
                    m.surrender_focus(palette_id);
                }
            });
        }
    }

    /// Navigate to a pane by id, updating both `focused_pane` on its window and
    /// `active_window`. Returns `true` if the pane was found.
    pub(crate) fn pane_navigate(&mut self, pane_id: u64) -> bool {
        // Read-only pass: find the window index, tile_id, and context_id before mutating.
        // Using iter() instead of iter_mut() so self.windows borrow ends before we call
        // push_focus_history (which needs &mut self).
        let found_read = self.windows.iter().enumerate().find_map(|(idx, win)| {
            win.tree.tiles.find_pane(&pane_id).map(|tile_id| (idx, tile_id, win.context_id))
        });
        let Some((idx, tile_id, ctx_id)) = found_read else {
            log::warn!("notify:action: pane_navigate pane_id={pane_id} not found");
            return false;
        };
        let old_focus = self.windows[self.active_window].focused_pane;
        let old_window_id = self.windows[self.active_window].window_id;
        self.windows[idx].focused_pane = Some(tile_id);
        self.push_focus_history(old_window_id, old_focus);
        let prev = self.active_window;
        self.active_window = idx;
        // Sync the router so the sidebar context switcher reflects the
        // new active context immediately (router.active_idx() drives the highlight).
        if let Some(ctx_idx) = self.router.position(|ctx| ctx.context_id == ctx_id) {
            self.router.set_active(ctx_idx);
            log::info!(
                "notify:action: pane_navigate active_window {prev}→{idx} ctx_idx={ctx_idx} pane_id={pane_id}"
            );
        } else {
            log::warn!(
                "notify:action: pane_navigate active_window {prev}→{idx} pane_id={pane_id} ctx_id={ctx_id} not found in router"
            );
        }
        true
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

    /// Drain input-intent events from `ctx.input` so downstream widgets (panes,
    /// terminal backends, `keys::poll_actions`) see only the global allowlist.
    ///
    /// Uses `input_intent::classify` to identify events that carry user input
    /// (Key, Text, Paste, Ime, Copy, Cut). Non-input events (pointer, scroll,
    /// window focus, etc.) pass through unconditionally. New egui::Event
    /// variants are dropped by default — promoting one requires adding it to
    /// `InputIntent`.
    pub(crate) fn drain_captured_keyboard_input(&self, ctx: &egui::Context) {
        ctx.input_mut(|i| {
            i.events.retain(|e| {
                if crate::input_intent::classify(e).is_none() {
                    return true;
                }
                match e {
                    egui::Event::Key { key, modifiers, .. } => {
                        let cmd = modifiers.command;
                        let shift = modifiers.shift;
                        if !cmd || modifiers.alt || modifiers.ctrl {
                            return false;
                        }
                        if !shift && matches!(key, egui::Key::Q | egui::Key::W) {
                            return true;
                        }
                        if shift && matches!(key, egui::Key::A) {
                            return true;
                        }
                        if shift && matches!(key, egui::Key::L | egui::Key::H) {
                            return true;
                        }
                        false
                    }
                    _ => false,
                }
            });
        });
    }

    /// Route `DeliverNotifyAction` commands back to the originating app pane as
    /// `NotifyAction` events. Shared by the modal and the sidebar panel so both
    /// surfaces dispatch identically.
    pub(crate) fn dispatch_notify_action_cmds(&mut self, cmds: Vec<crate::app_trait::AppCommand>) {
        use crate::app_trait::AppCommand;
        for cmd in cmds {
            if let AppCommand::DeliverNotifyAction { pane_id, notify_id, action_label, value, response_file, host_action } = cmd {
                log::info!(
                    "notify:action: pane_id={pane_id} notify_id={notify_id:?} value={value:?} host_action={host_action:?}"
                );
                // Execute host-side action synchronously before writing the response
                // file so the navigation is complete before the shell unblocks.
                if let Some(ref action) = host_action {
                    if let Some(id_str) = action.strip_prefix("pane_focus:") {
                        if let Ok(pane_id_target) = id_str.parse::<u64>() {
                            self.pane_navigate(pane_id_target);
                        } else {
                            log::warn!("notify:action: pane_focus: invalid pane_id {:?}", id_str);
                        }
                    } else {
                        log::warn!("notify:action: unknown host_action {:?}", action);
                    }
                }
                if let Some(rf) = &response_file {
                    let content = value.as_deref().unwrap_or("");
                    let tmp = format!("{rf}.tmp");
                    match std::fs::write(&tmp, content).and_then(|_| std::fs::rename(&tmp, rf)) {
                        Ok(_) => log::info!("notify:action: wrote {:?} to {:?}", content, rf),
                        Err(e) => log::warn!("notify:action: failed to write response file {:?}: {e}", rf),
                    }
                }
                // Search all windows for the sender pane — it may not be in the
                // active context (cross-context notification path).
                let window_idx = self.windows.iter().position(|w| w.panes.contains_key(&pane_id));
                if let Some(win_idx) = window_idx {
                    if let Some(pane) = self.windows[win_idx].panes.get_mut(&pane_id) {
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

/// Translate a key string (e.g. "enter", "ctrl+c", "h") to PTY bytes.
fn key_str_to_pty_bytes(key: &str) -> Vec<u8> {
    let key_lower = key.to_lowercase();
    // Handle ctrl+X chords using bit-mask to support any ASCII char ([, ], \, /, @, etc.)
    if let Some(rest) = key_lower.strip_prefix("ctrl+") {
        if let Some(ch) = rest.chars().next() {
            if ch.is_ascii() && !ch.is_ascii_control() {
                return vec![(ch as u8) & 0x1F];
            }
        }
    }
    match key_lower.as_str() {
        "enter" => b"\r".to_vec(),
        "escape" | "esc" => b"\x1b".to_vec(),
        "space" => b" ".to_vec(),
        "backspace" => b"\x7f".to_vec(),
        "tab" => b"\t".to_vec(),
        "up" | "arrowup" => b"\x1b[A".to_vec(),
        "down" | "arrowdown" => b"\x1b[B".to_vec(),
        "right" | "arrowright" => b"\x1b[C".to_vec(),
        "left" | "arrowleft" => b"\x1b[D".to_vec(),
        _ => {
            // single printable char
            let mut chars = key.chars();
            if let Some(ch) = chars.next() {
                if chars.next().is_none() {
                    let mut buf = [0u8; 4];
                    return ch.encode_utf8(&mut buf).as_bytes().to_vec();
                }
            }
            log::warn!("pane_ipc: key_pane: unrecognized key string {key:?}, sending raw bytes");
            key.as_bytes().to_vec()
        }
    }
}

/// Parse a key string into a (key_name, Modifiers) pair for PGAP app panes.
fn parse_key_str_to_event(key: &str) -> (String, crate::app_protocol::Modifiers) {
    let mut parts: Vec<&str> = key.split('+').collect();
    let key_part = parts.pop().unwrap_or(key);
    let mut modifiers = crate::app_protocol::Modifiers::default();
    for m in &parts {
        match m.to_lowercase().as_str() {
            "ctrl" | "control" => modifiers.ctrl = true,
            "shift" => modifiers.shift = true,
            "alt" => modifiers.alt = true,
            "cmd" | "command" | "meta" => modifiers.cmd = true,
            _ => {}
        }
    }
    let key_str = match key_part.to_lowercase().as_str() {
        "enter" | "return" => "Enter".to_string(),
        "escape" | "esc" => "Escape".to_string(),
        "space" => " ".to_string(),
        "backspace" => "Backspace".to_string(),
        "tab" => "Tab".to_string(),
        "up" | "arrowup" => "ArrowUp".to_string(),
        "down" | "arrowdown" => "ArrowDown".to_string(),
        "right" | "arrowright" => "ArrowRight".to_string(),
        "left" | "arrowleft" => "ArrowLeft".to_string(),
        _ => {
            // Preserve original case for single chars (e.g. "A" stays "A", not "a").
            // Multi-word named keys get title-case.
            if key_part.chars().count() == 1 {
                key_part.to_string()
            } else {
                let mut s = key_part.to_string();
                if let Some(c) = s.get_mut(0..1) {
                    c.make_ascii_uppercase();
                }
                s
            }
        }
    };
    (key_str, modifiers)
}

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

#[cfg(test)]
mod tests;
