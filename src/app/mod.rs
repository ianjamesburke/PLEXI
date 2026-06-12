pub mod app_trait;
mod canvas_bindings;
mod dispatch;
mod focus;
mod lifecycle;
pub(crate) mod notification_image;
mod notifications;
pub mod package;
pub mod packs;
pub mod permissions;
pub mod plexi_descriptor;
pub mod registry;
pub mod registry_watcher;
mod render;
pub mod secrets_app;
mod sync;
pub mod text_editor_app;

#[cfg(test)]
pub(crate) use focus::FocusLogOutcome;
pub(crate) use focus::{
    ContextCloseState, FocusLayer, FocusSegmentReason, FOCUS_HEARTBEAT_INTERVAL,
};
pub(crate) use notification_image::NotificationImageState;
#[cfg(test)]
pub(crate) use notifications::save_pending_notifications_to;
pub(crate) use notifications::{load_pending_notifications_from, PendingNotification};

/// Build a shell command string from an args list for passing to `zsh -c <cmd>`.
/// A single arg is used as-is (it's already a shell expression — CLI path).
/// Multiple args are joined with shell quoting so word-splitting is preserved.
fn cmd_from_args(args: &[String]) -> Option<String> {
    match args {
        [] => None,
        [single] => Some(single.clone()),
        multiple => Some(crate::host::shell::shell_join(multiple)),
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
    pub context_root: Option<std::path::PathBuf>,
}

/// What a `TextInputOverlay` commit should do.
#[derive(Clone, Debug)]
pub(crate) enum OverlayTarget {
    /// Set (or clear) the root directory on the context at `idx`.
    ContextRoot(usize),
}

/// Shared text-input overlay state. One instance per open modal.
#[derive(Clone, Debug)]
pub(crate) struct TextInputOverlay {
    pub label: String,
    pub hint: String,
    pub buffer: String,
    /// One-shot guard: true after the first `request_focus()` call.
    pub focus_requested: bool,
}

use crate::app::registry::AppRegistry;
use crate::config;
use crate::host::context::Window;
use crate::host::keys::{self, Action};
use crate::host::pane::{Pane, TerminalPane};
use crate::host::shell;
use crate::spatial::tiling::PaneId;
use crate::ui::theme::{self, Colors};
use crate::workspace::WorkspaceFile;
use egui_term::{BackendSettings, PtyEvent, TerminalTheme};
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
    dir: crate::host::keys::Direction,
    started_at: std::time::Instant,
}

pub(crate) struct ClickFlash {
    pub(crate) window_id: u64,
    pub(crate) tile: egui_tiles::TileId,
    pub(crate) started_at: std::time::Instant,
}

pub struct PlexiApp {
    pub(crate) pty_event_rx: mpsc::Receiver<(u64, PtyEvent)>,
    pub(crate) pty_event_tx: mpsc::Sender<(u64, PtyEvent)>,
    pub(crate) last_notify_poll: std::time::Instant,
    pub(crate) scheduler: crate::host::scheduler::Scheduler,
    pub(crate) theme: TerminalTheme,
    pub(crate) colors: Colors,
    pub(crate) default_font_size: f32,
    pub(crate) ctx: egui::Context,
    pub(crate) router: crate::workspace::router::WorkspaceRouter,
    pub(crate) windows: Vec<Window>,
    pub(crate) active_window: usize,
    pub(crate) sidebar_visible: bool,
    pub(crate) show_shortcuts: bool,
    pub(crate) show_changelog: bool,
    pub(crate) show_ui_gallery: bool,
    pub(crate) ui_gallery_normal_buf: String,
    pub(crate) ui_gallery_focused_buf: String,
    pub(crate) ui_gallery_show_text_modal: bool,
    pub(crate) ui_gallery_modal_buf: String,
    pub(crate) show_cli_setup_prompt: bool,
    /// `None` = idle/success (modal closes on success), `Some(false)` = not found.
    pub(crate) cli_setup_check_result: Option<bool>,
    pub(crate) show_completions_banner: bool,
    pub(crate) quitting: bool,
    pub(crate) quit_press_count: u8,
    pub(crate) quit_last_press: Option<std::time::Instant>,
    pub(crate) pending_close: bool,
    pub(crate) pending_context_close: Option<ContextCloseState>,
    pub(crate) welcome_delete_press_count: u8,
    pub(crate) welcome_delete_last_press: Option<std::time::Instant>,
    pub(crate) frame_tick: crate::platform::logging::FrameTick,
    /// Repaint-cause diagnostics sample window (#2019): start instant and
    /// frame count. `None` until the first frame opens a window.
    pub(crate) frame_diag_window: Option<(std::time::Instant, u32)>,
    /// Directory holding `permissions.toml` for the host-level permission
    /// management handlers (`ListPermissions` / `SetPermission`, stint 0017).
    /// `config_dir()` in production; an isolated temp dir in tests so harness
    /// runs never read or write the developer's real permission store.
    pub(crate) permission_store_dir: std::path::PathBuf,
    /// Cached config so confirmation settings are read through the config
    /// tunnel rather than duplicated as individual bool fields.
    pub(crate) config: crate::config::PlexiConfig,
    pub(crate) key_bindings: crate::host::keys::KeyBindings,
    pub(crate) binding_table: Vec<crate::host::keys::BindingEntry>,
    pub(crate) renaming_window: Option<usize>,
    pub(crate) rename_buffer: String,
    pub(crate) editing_description: Option<usize>,
    pub(crate) description_buffer: String,
    pub(crate) description_focus_requested: bool,
    pub(crate) drag_context: Option<usize>,
    /// Whether the "Parked (N)" section in the sidebar is expanded.
    pub(crate) parked_section_expanded: bool,
    pub(crate) registry: AppRegistry,
    pub(crate) show_command_palette: bool,
    pub(crate) palette_query: String,
    pub(crate) palette_selected: usize,
    /// Cached workspace root for the focused pane at the moment the palette
    /// was opened. Resolved once on open, not per-frame, to avoid repeated
    /// filesystem traversal in the egui draw loop.
    pub(crate) palette_workspace_root: Option<std::path::PathBuf>,
    pub(crate) context_visit_history: Vec<u64>,
    pub(crate) renaming_pane: Option<PaneId>,
    /// One-shot guard: true after `request_focus()` fires on the rename modal's
    /// first render. Prevents the focus from being re-requested every frame,
    /// which lets a later widget steal it on the same frame indefinitely.
    pub(crate) rename_pane_focus_requested: bool,
    /// Active text-input overlay and its dispatch target.
    pub(crate) text_overlay: Option<(TextInputOverlay, OverlayTarget)>,
    /// Receiver for async folder-picker results (Browse button).
    pub(crate) text_overlay_browse_rx: Option<mpsc::Receiver<Option<PathBuf>>>,
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
    /// Notes picker: sorted list of (path, first-line-preview) for the current workspace.
    pub(crate) notes_picker_entries: Vec<(std::path::PathBuf, String)>,
    /// Notes picker: currently highlighted row index.
    pub(crate) notes_picker_selected: usize,
    /// Notes triage: inbox notes loaded when the triage overlay opens.
    pub(crate) notes_triage_notes: Vec<crate::notes::InboxNote>,
    /// Notes triage: configured actions loaded when the triage overlay opens.
    pub(crate) notes_triage_actions: Vec<crate::notes::TriageAction>,
    /// Notes triage: index of the note currently being shown.
    pub(crate) notes_triage_index: usize,
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
    pub(crate) hot_reload: crate::host::hot_reload::HotReloadWatcher,
    pub(crate) hot_reload_rx: std::sync::mpsc::Receiver<crate::host::hot_reload::ReloadRequest>,
    /// Config file watcher (#1115). Watches `config.toml` for saves and fires
    /// a signal so `reload_config()` runs automatically.
    pub(crate) _config_watcher: Option<crate::config::watcher::ConfigWatcher>,
    pub(crate) config_reload_rx: Option<std::sync::mpsc::Receiver<()>>,
    /// App registry filesystem watcher (#1712). Watches the global and workspace-local
    /// apps dirs; signals `registry_reload_rx` on any directory change so the registry
    /// is rescanned without a host restart.
    pub(crate) _registry_watcher: Option<crate::app::registry_watcher::AppRegistryWatcher>,
    pub(crate) registry_reload_rx: Option<std::sync::mpsc::Receiver<()>>,
    /// Watched panes scheduled for crash-restart. Value is the earliest `Instant` at
    /// which the restart fires — giving the developer ~2s to read the crash overlay.
    pub(crate) pending_crash_restarts: HashMap<PaneId, std::time::Instant>,
    /// Spatial-grid minimap overlay state. Controls visibility, fade timer,
    /// and the `Cmd+Shift+M` override-visible flag.
    pub(crate) minimap: crate::render::minimap::MinimapState,
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
    /// Exponential accent flash on focus change (#1141).
    pub(crate) click_flash: Option<ClickFlash>,
    /// Channel receiver fed by the background update-check thread. Sends the
    /// latest version string exactly once if a newer release is available.
    update_rx: Option<std::sync::mpsc::Receiver<String>>,
    /// Latest available version string, set after the background check resolves.
    /// `None` means either the check hasn't completed or we're already current.
    pub(crate) update_available: Option<String>,
    /// Receiver for AppRequests sent over the PLEXI_SOCKET Unix socket listener.
    /// Drained each frame in `drain_pane_cmd_channel`.
    pane_ipc_rx: std::sync::mpsc::Receiver<crate::app_protocol::AppRequest>,
    /// Last (window_id, tile_id) pair that was logged as a FocusChanged event.
    /// Uses stable window_id (u64) not a vector index so removals don't corrupt it.
    /// Compared at end of each frame to detect genuine focus transitions.
    pub(crate) last_logged_focus: Option<(u64, egui_tiles::TileId)>,
    /// When the current focus session started. Reset on each FocusChanged emit.
    pub(crate) focus_started_at: Option<std::time::Instant>,
    /// Last observed system theme for auto-switching catppuccin variants (#1776).
    pub(crate) last_system_theme: Option<egui::Theme>,
    /// App commands held while a modal overlay owns keyboard input. Released and
    /// dispatched on the first frame where `input_captured_by_overlay()` is false.
    /// Only overlay-unsafe side effects are held here; safe commands (ShowNotification,
    /// pipes, queries) are dispatched immediately even during overlay ownership.
    pub(crate) overlay_held_cmds: Vec<crate::app::app_trait::AppCommand>,
    /// Host agent runtime (Phase C, docs/prm/agent-platform.md). Loaded from
    /// the active workspace's `agents/` dir; ticked once per frame in
    /// `update()` to consume agent event deliveries and finished turns.
    pub(crate) agent_host: crate::agent::AgentHost,
}

#[cfg(test)]
fn configure_egui_ctx(ctx: &egui::Context, colors: &Colors) {
    theme::setup_fonts(ctx);
    ctx.set_visuals(egui::Visuals::dark());
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    theme::setup_style(ctx, colors, true);
}

fn spawn_socket_listener(tx: std::sync::mpsc::Sender<crate::app_protocol::AppRequest>) {
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
                    match serde_json::from_str::<crate::app_protocol::AppRequest>(&line) {
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
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        frame_tick: crate::platform::logging::FrameTick,
    ) -> Self {
        #[cfg(target_os = "macos")]
        crate::platform::macos_menu::customize_app_menu();
        #[cfg(target_os = "macos")]
        crate::platform::finder_service::register();

        // Repaint-cause diagnostics (#2019): route egui_term's repaint labels
        // into the host counters; `update()` flushes a summary every 10s.
        egui_term::set_repaint_diag_hook(|label| {
            use crate::platform::frame_diag::{note, RepaintCause};
            match label {
                "terminal_cursor_blink" => note(RepaintCause::TerminalCursorBlink),
                "terminal_search_blink" => note(RepaintCause::TerminalSearchBlink),
                "terminal_pty_output" => note(RepaintCause::TerminalPtyOutput),
                "pointer_tracking" => note(RepaintCause::PointerTracking),
                other => log::warn!(
                    target: "plexi::frame_diag",
                    "unknown egui_term repaint cause label: {other}"
                ),
            }
        });
        log::info!(target: "plexi::frame_diag", "frame diagnostics active; summary every 10s");

        theme::setup_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);

        // Hot-reload watcher set (#83). Constructed once per host instance.
        // The receiver lives on `self.hot_reload_rx`; `update()` drains it
        // each frame and reloads the matching pane. Both branches of `new()`
        // (workspace-restore and default) use the same instance via shadow
        // names — kept on stack until consumed by `Self {..}`.
        let (hr_watcher, hr_rx) = crate::host::hot_reload::HotReloadWatcher::new();
        let (hr_watcher2, hr_rx2) = crate::host::hot_reload::HotReloadWatcher::new();

        // Config file watcher (#1115). Watches config.toml for saves so the
        // host can hot-reload theme/font/notification settings automatically.
        let (mut cfg_watcher, mut cfg_reload_rx) =
            match crate::config::watcher::start(crate::config::config_path()) {
                Some((w, rx)) => (Some(w), Some(rx)),
                None => (None, None),
            };

        // Resolve the active workspace (explicit `plexi <path>` arg, then
        // CWD-walk fallback) and overlay its channel-scoped config on top of
        // the global config. Project values win on a per-field basis; unset
        // project fields preserve the global value.
        let active_workspace = config::active_workspace_root();
        let config = config::PlexiConfig::load_with_workspace(active_workspace.as_deref());
        let key_bindings = crate::host::keys::build_key_bindings(config.keybindings.as_ref());
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
        let dark_mode = !theme::is_light_preset(
            config.theme.as_ref().and_then(|t| t.preset.as_deref()).unwrap_or(""),
        );
        theme::setup_style(&cc.egui_ctx, &colors, dark_mode);
        let window_theme = if dark_mode {
            egui::SystemTheme::Dark
        } else {
            egui::SystemTheme::Light
        };
        cc.egui_ctx
            .send_viewport_cmd(egui::ViewportCommand::SetTheme(window_theme));
        log::info!("theme: set_window_theme dark_mode={dark_mode}");

        let (tx, rx) = mpsc::channel();

        let cwd = std::env::current_dir().unwrap_or_default();
        let registry = AppRegistry::load(&cwd);

        let (mut reg_watcher, mut reg_reload_rx) = match crate::app::registry_watcher::start(
            crate::app::registry::registry_watch_dirs(&cwd),
        ) {
            Some((w, rx)) => (Some(w), Some(rx)),
            None => (None, None),
        };

        // Initialize the event log. Global log goes to ~/.plexi-*/events.jsonl;
        // workspace log goes to .plexi/events.jsonl if we're inside a workspace.
        {
            let global_path = crate::config::config_dir().join("events.jsonl");
            let workspace_path = crate::host::event_log::find_workspace_events_path(&cwd);
            crate::host::event_log::init_global(global_path, workspace_path);
        }

        // Spawn background update check. Sends the latest version once if newer.
        let (update_tx, update_rx) = std::sync::mpsc::channel::<String>();
        crate::cli::updater::spawn_update_check(crate::config::config_dir(), update_tx);

        let (pane_ipc_tx, pane_ipc_rx) =
            std::sync::mpsc::channel::<crate::app_protocol::AppRequest>();
        spawn_socket_listener(pane_ipc_tx);

        // One-time migration: remove the legacy file-queue directory if it
        // still exists from a previous install. Notify commands now travel
        // over the PLEXI_SOCKET, so the directory is dead weight.
        let _ = std::fs::remove_dir_all(crate::config::config_dir().join("notify-queue"));

        // Try to load saved workspace
        if let Some(ws) = WorkspaceFile::load() {
            let mut windows = Vec::new();
            let ctx_name_map: std::collections::HashMap<u64, String> = ws
                .contexts
                .iter()
                .map(|c| (c.context_id, c.name.clone()))
                .collect();
            let ctx_desc_map: std::collections::HashMap<u64, String> = ws
                .contexts
                .iter()
                .filter_map(|c| c.description.as_ref().map(|d| (c.context_id, d.clone())))
                .collect();
            let ctx_root_map: std::collections::HashMap<u64, PathBuf> = ws
                .contexts
                .iter()
                .filter_map(|c| c.root.as_ref().map(|r| (c.context_id, r.clone())))
                .collect();
            let ctx_depth_map: std::collections::HashMap<u64, u32> = ws
                .contexts
                .iter()
                .map(|c| (c.context_id, c.depth))
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
                        pane_entry = crate::pane_ops::restore_builtin_app_pane(
                            app_type,
                            saved_pane.id,
                            app_cwd.clone(),
                            saved_pane.app_state.as_ref(),
                        );
                        if pane_entry.is_none() {
                            if let Some(process) = registry.launch_process(app_type, &app_cwd, &[])
                            {
                                pane_entry =
                                    Some(Pane::App(Box::new(crate::host::pane::AppPane {
                                        id: saved_pane.id,
                                        permissions: process.permissions.clone(),
                                        runtime: crate::host::pane::AppRuntime::Process(Box::new(
                                            process,
                                        )),
                                        workspace_root: app_cwd,
                                        manifest_id: app_type.to_string(),
                                        name: app_type.to_string(),
                                        pane_group: registry.group_for(app_type),
                                        linked_pane_id: None,
                                        overlay_replaced: None,
                                        hidden: false,
                                        agent: None,
                                        slots: std::collections::HashMap::new(),
                                    })));
                            }
                        }
                    }

                    // Portal panes — restore the tile reference, no process to start.
                    if matches!(
                        saved_pane.kind,
                        crate::workspace::SavedPaneKind::Portal { .. }
                    ) {
                        if let crate::workspace::SavedPaneKind::Portal { context_id } =
                            &saved_pane.kind
                        {
                            pane_entry =
                                Some(Pane::Portal(Box::new(crate::host::pane::PortalPane {
                                    pane_id: saved_pane.id,
                                    target_context_id: *context_id,
                                    context_state: None,
                                    hidden: false,
                                })));
                        }
                    }

                    if pane_entry.is_none() {
                        let ctx_name = ctx_name_map
                            .get(&saved_win.context_id)
                            .cloned()
                            .unwrap_or_default();
                        let ctx_desc = ctx_desc_map
                            .get(&saved_win.context_id)
                            .cloned()
                            .unwrap_or_default();
                        let ctx_root = ctx_root_map.get(&saved_win.context_id);
                        let ctx_depth = ctx_depth_map
                            .get(&saved_win.context_id)
                            .copied()
                            .unwrap_or(0);
                        let settings = Self::make_backend_settings(
                            saved_pane.id,
                            cwd,
                            &colors,
                            saved_win.context_id,
                            &ctx_name,
                            &ctx_desc,
                            ctx_root,
                            ctx_depth,
                        );
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

                    if let Some(mut pane) = pane_entry {
                        if saved_pane.hidden {
                            pane.set_hidden(true);
                        }
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
                let contexts = ws.contexts;
                let active_ctx = ws.active_context.min(contexts.len().saturating_sub(1));
                let active_ctx_id = contexts[active_ctx].context_id;
                let active = ws
                    .context_active_window
                    .get(&active_ctx_id)
                    .and_then(|win_id| windows.iter().position(|w| w.window_id == *win_id))
                    .unwrap_or(0);
                let window_count = windows
                    .iter()
                    .filter(|w| w.context_id == active_ctx_id)
                    .count();
                let mut host = crate::host::model::HostModel::new();
                host.seed_next_pane_id(ws.next_pane_id);
                let mut app = Self {
                    pty_event_rx: rx,
                    pty_event_tx: tx,
                    theme: theme::terminal_theme(&theme_cfg),
                    colors,
                    default_font_size,
                    ctx: cc.egui_ctx.clone(),
                    router: crate::workspace::router::WorkspaceRouter::new(contexts, active_ctx),
                    windows,
                    active_window: active,
                    sidebar_visible: ws.sidebar_visible,
                    show_shortcuts: false,
                    show_changelog: false,
                    show_ui_gallery: false,
                    ui_gallery_normal_buf: "workspace/search".to_string(),
                    ui_gallery_focused_buf: "Focused field".to_string(),
                    ui_gallery_show_text_modal: false,
                    ui_gallery_modal_buf: String::new(),
                    show_cli_setup_prompt: crate::cli::setup::should_prompt(),
                    cli_setup_check_result: None,
                    show_completions_banner: crate::cli::setup::should_prompt_completions(),
                    quitting: false,
                    quit_press_count: 0,
                    quit_last_press: None,
                    welcome_delete_press_count: 0,
                    welcome_delete_last_press: None,
                    config: config.clone(),
                    key_bindings: key_bindings.clone(),
                    binding_table: crate::host::keys::build_binding_table(&key_bindings),
                    pending_close: false,
                    pending_context_close: None,
                    frame_tick: frame_tick.clone(),
                    frame_diag_window: None,
                    permission_store_dir: crate::config::config_dir(),
                    renaming_window: None,
                    rename_buffer: String::new(),
                    editing_description: None,
                    description_buffer: String::new(),
                    description_focus_requested: false,
                    drag_context: None,
                    parked_section_expanded: false,
                    show_command_palette: false,
                    palette_query: String::new(),
                    palette_selected: 0,
                    palette_workspace_root: None,
                    context_visit_history: Vec::new(),
                    renaming_pane: None,
                    rename_pane_focus_requested: false,
                    text_overlay: None,
                    text_overlay_browse_rx: None,
                    registry,
                    features: features.clone(),
                    pending_notifications: load_pending_notifications_from(
                        &crate::config::config_dir().join("notifications.json"),
                    ),
                    show_notification_modal: false,
                    current_notify_id: None,
                    modal_focused_option: 0,
                    modal_input_buffer: String::new(),
                    quick_note_text: String::new(),
                    quick_note_ctx: QuickNoteCtx::default(),
                    notes_picker_entries: Vec::new(),
                    notes_picker_selected: 0,
                    notes_triage_notes: Vec::new(),
                    notes_triage_actions: Vec::new(),
                    notes_triage_index: 0,
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
                    scheduler: crate::host::scheduler::Scheduler::new(),
                    host,
                    host_services: crate::host::services::HostServices::new(),
                    background_apps: HashMap::new(),
                    directed_pipes: HashMap::new(),
                    hot_reload: hr_watcher,
                    hot_reload_rx: hr_rx,
                    _config_watcher: cfg_watcher.take(),
                    config_reload_rx: cfg_reload_rx.take(),
                    _registry_watcher: reg_watcher.take(),
                    registry_reload_rx: reg_reload_rx.take(),
                    pending_crash_restarts: HashMap::new(),
                    minimap: crate::render::minimap::MinimapState::with_visible(window_count >= 2),
                    last_page_x_per_row: HashMap::new(),
                    context_active_window: ws.context_active_window,
                    minimap_visible_per_context: HashMap::new(),
                    next_window_id: next_id,
                    pane_snapshot_len: 0,
                    pane_anims: Vec::new(),
                    edge_pulse: None,
                    click_flash: None,
                    update_rx: Some(update_rx),
                    update_available: None,
                    pane_ipc_rx,
                    last_logged_focus: None,
                    focus_started_at: None,
                    last_system_theme: None,
                    overlay_held_cmds: Vec::new(),
                    agent_host: crate::agent::AgentHost::production(config.ai.clone()),
                };
                app.apply_context_transition_effects();
                return app;
            }
        }

        // Default: empty context — welcome screen is shown until the user creates a pane.
        // If CWD has a .plexi/ anchor, use its context defaults for name/description
        // and set root to the anchor path.
        let panes: HashMap<u64, Pane> = HashMap::new();
        let tree = Tree::empty("plexi");

        let path = {
            let cwd = std::env::current_dir()
                .unwrap_or_else(|_| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
            // macOS GUI apps launch with CWD = /. Use home_dir instead so
            // the initial context and all derived CWDs start at ~.
            if cwd == PathBuf::from("/") {
                dirs::home_dir().unwrap_or(cwd)
            } else {
                cwd
            }
        };

        let anchor = crate::host::anchor::Anchor::detect(&path);
        let (default_name, default_description, default_root) = match anchor.as_ref() {
            Some(a) => {
                let defaults = a.context_defaults.as_ref();
                let name = defaults.and_then(|d| d.name.clone()).unwrap_or_else(|| {
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Default".to_string())
                });
                let desc = defaults.and_then(|d| d.description.clone());
                log::info!(
                    "anchor detected at CWD: name={:?} description={:?} root={}",
                    name,
                    desc,
                    a.root.display()
                );
                (name, desc, Some(a.root.clone()))
            }
            None => ("Default".to_string(), None, None),
        };

        let default_cwd = std::env::current_dir().unwrap_or_default();
        let default_registry = AppRegistry::load(&default_cwd);
        let (mut default_reg_watcher, mut default_reg_reload_rx) =
            match crate::app::registry_watcher::start(crate::app::registry::registry_watch_dirs(
                &default_cwd,
            )) {
                Some((w, rx)) => (Some(w), Some(rx)),
                None => (None, None),
            };

        let agent_host = crate::agent::AgentHost::production(config.ai.clone());
        let mut app = Self {
            pty_event_rx: rx,
            pty_event_tx: tx,
            theme: theme::terminal_theme(&theme_cfg),
            colors,
            default_font_size,
            ctx: cc.egui_ctx.clone(),
            router: crate::workspace::router::WorkspaceRouter::new(
                vec![crate::host::context::Context {
                    name: default_name,
                    path: path.clone(),
                    root: default_root,
                    description: default_description,
                    context_id: 1,
                    parent_id: None,
                    depth: 0,
                    parked: false,
                }],
                0,
            ),
            windows: vec![Window {
                name: String::new(),
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
            show_ui_gallery: false,
            ui_gallery_normal_buf: "workspace/search".to_string(),
            ui_gallery_focused_buf: "Focused field".to_string(),
            ui_gallery_show_text_modal: false,
            ui_gallery_modal_buf: String::new(),
            show_cli_setup_prompt: crate::cli::setup::should_prompt(),
            cli_setup_check_result: None,
            show_completions_banner: crate::cli::setup::should_prompt_completions(),
            quitting: false,
            quit_press_count: 0,
            quit_last_press: None,
            welcome_delete_press_count: 0,
            welcome_delete_last_press: None,
            config,
            binding_table: crate::host::keys::build_binding_table(&key_bindings),
            key_bindings,
            pending_close: false,
            pending_context_close: None,
            frame_tick,
            frame_diag_window: None,
            permission_store_dir: crate::config::config_dir(),
            renaming_window: None,
            rename_buffer: String::new(),
            editing_description: None,
            description_buffer: String::new(),
            description_focus_requested: false,
            drag_context: None,
            parked_section_expanded: false,
            show_command_palette: false,
            palette_query: String::new(),
            palette_selected: 0,
            palette_workspace_root: None,
            context_visit_history: Vec::new(),
            renaming_pane: None,
            rename_pane_focus_requested: false,
            text_overlay: None,
            text_overlay_browse_rx: None,
            registry: default_registry,
            features,
            pending_notifications: load_pending_notifications_from(
                &crate::config::config_dir().join("notifications.json"),
            ),
            show_notification_modal: false,
            current_notify_id: None,
            modal_focused_option: 0,
            modal_input_buffer: String::new(),
            quick_note_text: String::new(),
            quick_note_ctx: QuickNoteCtx::default(),
            notes_picker_entries: Vec::new(),
            notes_picker_selected: 0,
            notes_triage_notes: Vec::new(),
            notes_triage_actions: Vec::new(),
            notes_triage_index: 0,
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
            scheduler: crate::host::scheduler::Scheduler::new(),
            host: crate::host::model::HostModel::new(),
            host_services: crate::host::services::HostServices::new(),
            background_apps: HashMap::new(),
            directed_pipes: HashMap::new(),
            hot_reload: hr_watcher2,
            hot_reload_rx: hr_rx2,
            _config_watcher: cfg_watcher.take(),
            config_reload_rx: cfg_reload_rx.take(),
            _registry_watcher: default_reg_watcher.take(),
            registry_reload_rx: default_reg_reload_rx.take(),
            pending_crash_restarts: HashMap::new(),
            minimap: crate::render::minimap::MinimapState::new(),
            last_page_x_per_row: HashMap::new(),
            context_active_window: HashMap::new(),
            minimap_visible_per_context: HashMap::new(),
            next_window_id: 2,
            pane_snapshot_len: 0,
            pane_anims: Vec::new(),
            edge_pulse: None,
            click_flash: None,
            update_rx: Some(update_rx),
            update_available: None,
            pane_ipc_rx,
            last_logged_focus: None,
            focus_started_at: None,
            last_system_theme: None,
            overlay_held_cmds: Vec::new(),
            agent_host,
        };
        app.apply_context_transition_effects();
        app
    }

    /// Search ALL windows (not just the active one) for a pane by ID.
    /// Returns (window_index, tile_id). O(n*m) but n is typically 1-3.
    pub(crate) fn find_pane_in_any_window(
        &self,
        pane_id: crate::spatial::tiling::PaneId,
    ) -> Option<(usize, egui_tiles::TileId)> {
        self.windows
            .iter()
            .enumerate()
            .find_map(|(idx, win)| win.tree.tiles.find_pane(&pane_id).map(|tile| (idx, tile)))
    }

    /// Set focused pane in a specific window.
    /// Use this everywhere instead of `self.windows[i].focused_pane = Some(...)` directly
    /// so that the grep pattern `windows[active].focused_pane = ` has zero matches outside tests.
    pub(crate) fn set_window_focused_pane(&mut self, win_idx: usize, tile: egui_tiles::TileId) {
        self.windows[win_idx].navigate_to(tile);
        let window_id = self.windows[win_idx].window_id;
        log::info!("focus: flash set — win={window_id} tile={tile:?}");
        self.click_flash = Some(ClickFlash {
            window_id,
            tile,
            started_at: std::time::Instant::now(),
        });
    }

    /// Restore focused pane in a specific window from a saved `Option<TileId>`.
    /// Use instead of `self.windows[i].focused_pane = saved_opt` so the grep
    /// pattern `windows[active].focused_pane = ` has zero matches outside tests.
    pub(crate) fn restore_window_focused_pane(
        &mut self,
        win_idx: usize,
        saved: Option<egui_tiles::TileId>,
    ) {
        self.windows[win_idx].focused_pane = saved;
    }

    /// Create a `PlexiApp` for headless tests. No workspace restore, no macOS
    /// menu setup, no PTY or audio hardware. Initialises a single empty window
    /// so `state().open_panes` is empty and the harness can add panes via
    /// `inject_app_pane`.
    #[cfg(test)]
    pub fn new_for_test(
        ctx: egui::Context,
        frame_tick: crate::platform::logging::FrameTick,
    ) -> (
        Self,
        std::sync::mpsc::Sender<crate::app_protocol::AppRequest>,
    ) {
        let config = config::PlexiConfig::default();
        let key_bindings = crate::host::keys::build_key_bindings(config.keybindings.as_ref());
        let theme_cfg = Self::resolve_theme_config(&config);
        let colors = Colors::from_config(&theme_cfg);
        configure_egui_ctx(&ctx, &colors);
        let (tx, rx) = mpsc::channel();
        let (hr_watcher, hr_rx) = crate::host::hot_reload::HotReloadWatcher::new();
        let path = std::env::temp_dir();
        let features = crate::features::FeatureFlags::from_config(&config);
        let (pane_ipc_tx, pane_ipc_rx) =
            std::sync::mpsc::channel::<crate::app_protocol::AppRequest>();
        (
            Self {
                pty_event_rx: rx,
                pty_event_tx: tx,
                last_notify_poll: std::time::Instant::now(),
                scheduler: crate::host::scheduler::Scheduler::new(),
                theme: theme::terminal_theme(&theme_cfg),
                colors,
                default_font_size: theme::FONT_SIZE,
                ctx: ctx.clone(),
                router: crate::workspace::router::WorkspaceRouter::new(
                    vec![crate::host::context::Context {
                        name: "Test".into(),
                        path: path.clone(),
                        root: None,
                        description: None,
                        context_id: 1,
                        parent_id: None,
                        depth: 0,
                        parked: false,
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
                show_ui_gallery: false,
                ui_gallery_normal_buf: "workspace/search".to_string(),
                ui_gallery_focused_buf: "Focused field".to_string(),
                ui_gallery_show_text_modal: false,
                ui_gallery_modal_buf: String::new(),
                quitting: false,
                quit_press_count: 0,
                quit_last_press: None,
                welcome_delete_press_count: 0,
                welcome_delete_last_press: None,
                config,
                binding_table: crate::host::keys::build_binding_table(&key_bindings),
                key_bindings,
                pending_close: false,
                pending_context_close: None,
                frame_tick,
                frame_diag_window: None,
                permission_store_dir: {
                    let dir = std::env::temp_dir()
                        .join(format!("plexi-test-perms-{}", uuid::Uuid::new_v4()));
                    std::fs::create_dir_all(&dir).expect("create test permission store dir");
                    dir
                },
                renaming_window: None,
                rename_buffer: String::new(),
                editing_description: None,
                description_buffer: String::new(),
                description_focus_requested: false,
                drag_context: None,
                parked_section_expanded: false,
                show_command_palette: false,
                palette_query: String::new(),
                palette_selected: 0,
                palette_workspace_root: None,
                context_visit_history: Vec::new(),
                renaming_pane: None,
                rename_pane_focus_requested: false,
                text_overlay: None,
                text_overlay_browse_rx: None,
                registry: AppRegistry::load_with_global(&path, &path.join("nonexistent-apps-dir")),
                features,
                pending_notifications: Vec::new(),
                show_notification_modal: false,
                current_notify_id: None,
                modal_focused_option: 0,
                modal_input_buffer: String::new(),
                quick_note_text: String::new(),
                quick_note_ctx: QuickNoteCtx::default(),
                notes_picker_entries: Vec::new(),
                notes_picker_selected: 0,
                notes_triage_notes: Vec::new(),
                notes_triage_actions: Vec::new(),
                notes_triage_index: 0,
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
                _registry_watcher: None,
                registry_reload_rx: None,
                pending_crash_restarts: HashMap::new(),
                minimap: crate::render::minimap::MinimapState::new(),
                last_page_x_per_row: HashMap::new(),
                context_active_window: HashMap::new(),
                minimap_visible_per_context: HashMap::new(),
                next_window_id: 2,
                pane_snapshot_len: 0,
                pane_anims: Vec::new(),
                edge_pulse: None,
                click_flash: None,
                show_cli_setup_prompt: false,
                cli_setup_check_result: None,
                show_completions_banner: false,
                update_rx: None,
                update_available: None,
                pane_ipc_rx,
                last_logged_focus: None,
                focus_started_at: None,
                last_system_theme: None,
                overlay_held_cmds: Vec::new(),
                agent_host: crate::agent::AgentHost::inert(),
            },
            pane_ipc_tx,
        )
    }

    /// Add a minimal `ProcessApp` pane directly to window 0 for unit tests.
    /// Returns `(tile_id, pane_id)` — `tile_id` is suitable for `focused_pane` assignments.
    #[cfg(test)]
    pub(crate) fn add_test_pane(&mut self) -> (egui_tiles::TileId, u64) {
        use crate::app::permissions::AppPermissions;
        use crate::host::pane::{AppPane, AppRuntime};
        use crate::process_app::ProcessApp;

        // Use a simple incrementing id; start high to avoid collisions with HostHarness ids.
        static NEXT_PANE_ID: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(10000);
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
            hidden: false,
            agent: None,
            slots: std::collections::HashMap::new(),
        };

        let win = &mut self.windows[0];
        win.panes
            .insert(pane_id, crate::host::pane::Pane::App(Box::new(app_pane)));
        let tile_id = win.tree.tiles.insert_pane(pane_id);
        if win.tree.root.is_none() {
            win.tree.root = Some(tile_id);
        }
        (tile_id, pane_id)
    }

    fn resolve_theme_config(config: &config::PlexiConfig) -> config::ThemeConfig {
        let user_theme = config.theme.clone().unwrap_or_default();
        // Prefer [theme] preset; fall back to legacy top-level theme_preset so
        // existing configs survive the migration without silently losing their theme.
        let preset_name = user_theme.preset.as_deref().or(config.theme_preset.as_deref());
        if let Some(preset_name) = preset_name {
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
        context_description: &str,
        context_root: Option<&PathBuf>,
        context_depth: u32,
    ) -> BackendSettings {
        log::info!(
            "make_backend_settings: pane_id={pane_id} context_id={context_id} \
             context_name={context_name:?} context_root={context_root:?} context_depth={context_depth}"
        );
        let mut env = shell::build_env(working_directory.as_deref());
        env.insert("PLEXI_PANE_ID".into(), pane_id.to_string());
        let socket = crate::config::config_dir()
            .join("notify.sock")
            .to_string_lossy()
            .into_owned();
        env.insert("PLEXI_SOCKET".into(), socket);
        env.insert("PLEXI_CONTEXT_ID".into(), context_id.to_string());
        env.insert("PLEXI_CONTEXT_NAME".into(), context_name.to_string());
        env.insert(
            "PLEXI_CONTEXT_DESCRIPTION".into(),
            context_description.to_string(),
        );
        if let Some(root) = context_root {
            env.insert(
                "PLEXI_CONTEXT_ROOT".into(),
                root.to_string_lossy().into_owned(),
            );
        }
        env.insert("PLEXI_CONTEXT_DEPTH".into(), context_depth.to_string());
        BackendSettings {
            shell: shell::detect_shell(),
            args: vec!["-l".to_string()],
            env,
            dynamic_colors: theme::terminal_dynamic_colors(colors),
            working_directory,
        }
    }

    pub(crate) fn context_name_for(&self, context_id: u64) -> String {
        self.router
            .iter()
            .find(|c| c.context_id == context_id)
            .map(|c| c.name.clone())
            .unwrap_or_default()
    }

    pub(crate) fn context_description_for(&self, context_id: u64) -> String {
        self.router
            .iter()
            .find(|c| c.context_id == context_id)
            .and_then(|c| c.description.clone())
            .unwrap_or_default()
    }

    pub(crate) fn context_root_for(&self, context_id: u64) -> Option<PathBuf> {
        self.router
            .iter()
            .find(|c| c.context_id == context_id)
            .and_then(|c| c.root.clone())
    }

    pub(crate) fn context_depth_for(&self, context_id: u64) -> u32 {
        self.router
            .iter()
            .find(|c| c.context_id == context_id)
            .map(|c| c.depth)
            .unwrap_or(0)
    }
}

/// Returns true for app commands that must NOT execute while a modal overlay
/// owns keyboard input. Safe commands (ShowNotification, pipes, queries) always
/// dispatch immediately. Unsafe commands (layout/terminal mutations, focus changes)
/// are held in `overlay_held_cmds` and released on the next modal-free frame.
fn is_overlay_unsafe_cmd(cmd: &crate::app::app_trait::AppCommand) -> bool {
    use crate::app::app_trait::AppCommand;
    match cmd {
        AppCommand::SpawnApp { .. }
        | AppCommand::SpawnPane { .. }
        | AppCommand::RequestLinkedTerminal { .. }
        | AppCommand::RunInLinkedTerminal { .. }
        | AppCommand::InsertPathToken { .. }
        | AppCommand::OpenArtifact { .. } => true,
        AppCommand::DeliverNotifyAction { host_action, .. } => host_action
            .as_deref()
            .map(|a| a.starts_with("pane_focus:"))
            .unwrap_or(false),
        _ => false,
    }
}

fn overlay_unsafe_cmd_name(cmd: &crate::app::app_trait::AppCommand) -> &'static str {
    use crate::app::app_trait::AppCommand;
    match cmd {
        AppCommand::SpawnApp { .. } => "SpawnApp",
        AppCommand::SpawnPane { .. } => "SpawnPane",
        AppCommand::RequestLinkedTerminal { .. } => "RequestLinkedTerminal",
        AppCommand::RunInLinkedTerminal { .. } => "RunInLinkedTerminal",
        AppCommand::InsertPathToken { .. } => "InsertPathToken",
        AppCommand::OpenArtifact { .. } => "OpenArtifact",
        AppCommand::DeliverNotifyAction { .. } => "DeliverNotifyAction(pane_focus)",
        _ => "unknown",
    }
}

impl eframe::App for PlexiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_tick
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Repaint-cause diagnostics (#2019): count this frame, attribute
        // input-carrying frames to UserInput, and flush one summary per
        // 10s sample window.
        {
            let (started, frames) = self
                .frame_diag_window
                .get_or_insert_with(|| (std::time::Instant::now(), 0));
            *frames += 1;
            let frames = *frames;
            let elapsed = started.elapsed();
            if ctx.input(|i| !i.raw.events.is_empty()) {
                crate::platform::frame_diag::note(
                    crate::platform::frame_diag::RepaintCause::UserInput,
                );
            }
            if elapsed >= std::time::Duration::from_secs(10) {
                let counts = crate::platform::frame_diag::snapshot_and_reset();
                log::info!(
                    target: "plexi::frame_diag",
                    "{}",
                    crate::platform::frame_diag::summary_line(
                        frames,
                        elapsed.as_secs_f32(),
                        &counts
                    )
                );
                self.frame_diag_window = Some((std::time::Instant::now(), 0));
            }
        }
        self.update_preamble(ctx);

        // Host agent runtime (Phase C): consume queued event deliveries and
        // finished agent turns. Cheap no-op when nothing is pending. While a
        // turn runs on a worker thread, keep frames coming so its outcome is
        // collected promptly even when the UI is otherwise idle.
        self.agent_host.tick();
        if self.agent_host.turns_in_flight() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        // Unified overlay dispatch: each overlay owns its complete keyboard
        // contract via a `*_handle_key` method that returns `Consumed`, preventing
        // `dispatch_app_key_events` and `poll_actions` from seeing those events.
        // Global shortcuts (Cmd+Q, Cmd+W, Cmd+P) remain active via `poll_actions`
        // even when an overlay holds focus (see early-return guard in `keys::poll_actions`).
        let mut early_modal_cmds: Vec<crate::app::app_trait::AppCommand> = Vec::new();
        let overlay_key_disposition = if self.input_captured_by_overlay() {
            // Step 0: QuickNote preemption (#1626).
            // Cmd+0 is a high-priority action that must fire even when a non-critical
            // modal is open. Consume the key here — before the overlay draw phase can
            // absorb it via a focused TextEdit widget — then dismiss the non-critical
            // modal and open QuickNote. Critical modals (ConfirmClose, CapabilityModal,
            // ContextCloseConfirm) are not preemptable and fall through to handle_key.
            let qn_binding = self.key_bindings.open_quick_note;
            let qn_pressed = ctx.input_mut(|i| i.consume_key(qn_binding.0, qn_binding.1));
            if qn_pressed && self.is_quick_note_preemptable() {
                self.dismiss_preemptable_modal();
                self.open_quick_note_modal();
                self.sync_notification_modal_focus();
                self.sync_command_palette_focus();
                log::info!("quick_note: opened by preempting non-critical modal");
                crate::app::app_trait::KeyDisposition::Consumed
            } else {
                // Step 1: run the overlay's handle_key (consumes its owned key events).
                let disposition = match self.focus_stack.last() {
                    Some(FocusLayer::NotificationModal) => self.notification_modal_handle_key(ctx),
                    Some(FocusLayer::ConfirmClose) => self.confirm_close_handle_key(ctx),
                    Some(FocusLayer::CommandPalette) => self.command_palette_handle_key(ctx),
                    Some(FocusLayer::RenamePane) => self.rename_pane_handle_key(ctx),
                    Some(FocusLayer::ContextRename) => self.context_rename_handle_key(ctx),
                    Some(FocusLayer::ContextDescription) => {
                        self.context_description_handle_key(ctx)
                    }
                    Some(FocusLayer::QuickNote) => self.quick_note_handle_key(ctx),
                    Some(FocusLayer::CliSetupPrompt) => self.cli_setup_prompt_handle_key(ctx),
                    Some(FocusLayer::TextInput) => self.text_input_handle_key(ctx),
                    Some(FocusLayer::ContextCloseConfirm) => {
                        self.context_close_confirm_handle_key(ctx)
                    }
                    Some(FocusLayer::CapabilityModal) => self.capability_modal_handle_key(ctx),
                    Some(FocusLayer::NotesPicker) => {
                        self.notes_picker_handle_key(ctx);
                        crate::app::app_trait::KeyDisposition::Passthrough
                    }
                    Some(FocusLayer::NotesTriage) => {
                        self.notes_triage_handle_key(ctx);
                        crate::app::app_trait::KeyDisposition::Passthrough
                    }
                    None => crate::app::app_trait::KeyDisposition::Passthrough,
                };
                // Step 2: render the overlay (visual only — key reads already done above).
                match self.focus_stack.last().cloned() {
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
                    Some(FocusLayer::ContextDescription) => {
                        self.draw_edit_description_overlay(ctx);
                    }
                    Some(FocusLayer::QuickNote) => {
                        self.draw_quick_note_modal(ctx);
                    }
                    Some(FocusLayer::CliSetupPrompt) => {
                        self.draw_cli_setup_modal(ctx);
                    }
                    Some(FocusLayer::TextInput) => {
                        self.draw_text_input_overlay(ctx);
                    }
                    Some(FocusLayer::ContextCloseConfirm) => {
                        self.draw_context_close_confirm(ctx);
                    }
                    Some(FocusLayer::CapabilityModal) => {
                        self.draw_capability_modal(ctx);
                    }
                    Some(FocusLayer::NotesPicker) => {
                        self.draw_notes_picker(ctx);
                    }
                    Some(FocusLayer::NotesTriage) => {
                        self.draw_notes_triage(ctx);
                    }
                    None => {}
                }
                // The overlay may have self-closed (notification queue drained,
                // confirm-close confirmed/cancelled, palette picked an entry,
                // rename committed). Re-sync so the layer is accurate for the
                // rest of this frame.
                self.sync_notification_modal_focus();
                self.sync_confirm_close_focus();
                self.sync_context_close_focus();
                self.sync_command_palette_focus();
                self.sync_rename_pane_focus();
                self.sync_context_rename_focus();
                self.sync_cli_setup_prompt_focus();
                self.sync_text_input_focus();
                self.sync_capability_modal_focus();
                disposition
            } // end else (non-preempted path)
        } else {
            crate::app::app_trait::KeyDisposition::Passthrough
        };

        // Apps only receive key input when no overlay holds focus. Double-check
        // input_captured_by_overlay() after the re-syncs above — an overlay may
        // have returned Passthrough for keys it doesn't own (e.g. a Choice/Input
        // notification) while still being open. Re-syncs may also have dismissed
        // the overlay, in which case it's safe to forward keys to the app pane.
        if overlay_key_disposition == crate::app::app_trait::KeyDisposition::Passthrough
            && !self.input_captured_by_overlay()
        {
            self.dispatch_app_key_events(ctx);
        }
        // Drain every app pane's pending_commands every frame — including
        // while a modal holds focus. Background apps emitting notifications
        // must reach the queue *now*, not be buffered until the modal
        // closes (which caused the "ghost queue appears on reopen" bug).
        let fresh_cmds = self.drain_all_app_commands();
        if self.background_processes_need_wake() {
            crate::platform::frame_diag::note(
                crate::platform::frame_diag::RepaintCause::AppIdlePoll,
            );
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        // When the overlay releases, prepend any held unsafe commands so they
        // execute before new commands this frame (FIFO order preserved).
        let deferred_app_cmds: Vec<_> =
            if !self.input_captured_by_overlay() && !self.overlay_held_cmds.is_empty() {
                let released = std::mem::take(&mut self.overlay_held_cmds);
                log::info!(
                    "app_cmd: releasing {} held command(s) — overlay released",
                    released.len()
                );
                let mut all = released;
                all.extend(fresh_cmds);
                all
            } else {
                fresh_cmds
            };
        self.sync_app_cwd();

        // Dispatch any DeliverNotifyAction commands the early modal render
        // produced. Routes back to the originating pane as NotifyAction events.
        self.dispatch_notify_action_cmds(early_modal_cmds);

        // Handle deferred app commands returned from dispatch_app_key_events.
        for cmd in deferred_app_cmds {
            use crate::app::app_trait::AppCommand;
            // Hold overlay-unsafe side effects (layout/terminal mutations, focus changes)
            // while a modal owns input. Safe commands (ShowNotification, pipes, queries)
            // dispatch immediately. Released on the next modal-free frame.
            if self.input_captured_by_overlay() && is_overlay_unsafe_cmd(&cmd) {
                log::info!(
                    "app_cmd: deferring {} — overlay active ({} total held)",
                    overlay_unsafe_cmd_name(&cmd),
                    self.overlay_held_cmds.len() + 1,
                );
                self.overlay_held_cmds.push(cmd);
                continue;
            }
            match cmd {
                // Capability-gated pane read/control request from a PGAP app
                // (stint 0013/0014). routing.rs already checked panes.read /
                // panes.control; execute it through the same handler CLI
                // requests take over PLEXI_SOCKET.
                AppCommand::ForwardPaneRequest { request } => {
                    self.handle_pane_ipc_request(request);
                }
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
                    let _ = self.launch_app_by_id_with_layout(&type_id, layout, &args, None);

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
                    target_context,
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
                                            reason: "layout 'background' not yet implemented"
                                                .to_string(),
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

                    // target_context validation (#1518): if set, verify the
                    // target exists and is a descendant of the requester's
                    // context. Switch active_window temporarily so the spawn
                    // lands in the right context.
                    let original_active_window = self.active_window;
                    if let Some(target_ctx_id) = target_context {
                        let requester_context_id = self.windows[self.active_window].context_id;
                        let target_exists =
                            self.router.iter().any(|c| c.context_id == target_ctx_id);
                        let is_descendant = self
                            .host
                            .ancestors_of(target_ctx_id)
                            .contains(&requester_context_id);

                        if !target_exists
                            || (!is_descendant && requester_context_id != target_ctx_id)
                        {
                            log::warn!(
                                "SpawnPane: target_context={target_ctx_id} invalid or not a descendant of requester context {requester_context_id}"
                            );
                            // Send error back to requesting pane.
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
                                if let Some(pane) = self.windows[active].panes.get_mut(&req_pane_id)
                                {
                                    if let Some(a) = pane.as_app_mut() {
                                        a.runtime.queue_outbound_event(
                                            crate::app_protocol::PlexiEvent::PaneSpawnError {
                                                reason: format!(
                                                    "target_context {target_ctx_id} is not a descendant of context {requester_context_id}"
                                                ),
                                                request_id: request_id.clone(),
                                            },
                                        );
                                    }
                                }
                            }
                            continue;
                        }

                        // Switch to a window in the target context for the spawn.
                        if let Some(win_idx) = self
                            .windows
                            .iter()
                            .position(|w| w.context_id == target_ctx_id)
                        {
                            log::info!(
                                "SpawnPane: target_context={target_ctx_id} — switching to window index {win_idx}"
                            );
                            self.active_window = win_idx;
                        } else {
                            log::warn!(
                                "SpawnPane: target_context={target_ctx_id} exists but has no window"
                            );
                            continue;
                        }
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

                    // Predict the pane id that will be allocated (next_pane_id peeks without allocating).
                    let new_pane_id = self.host.next_pane_id();
                    if type_id == "terminal" {
                        // "terminal" is a builtin pane type, not in the app registry.
                        // Resolve target window+tile from from_pane_id (cross-window search)
                        // or fall back to the active window's focused pane.
                        let vertical =
                            matches!(layout.as_str(), "split_h" | "split_right" | "split_left");
                        let new_pane_first =
                            matches!(layout.as_str(), "split_above" | "split_left");
                        let initial_cmd = cmd_from_args(&effective_args);
                        let close_on_exit = initial_cmd.is_some();
                        let (target_win, target_tile) = if let Some(from_id) = from_pane_id {
                            match self.find_pane_in_any_window(from_id) {
                                Some(loc) => {
                                    log::info!(
                                        "SpawnPane: targeting from_pane_id={from_id} in win_idx={}",
                                        loc.0
                                    );
                                    loc
                                }
                                None => {
                                    log::warn!("SpawnPane: from_pane_id={from_id} not found in any window, using focused pane");
                                    let Some(tile) = self.windows[active]
                                        .focused_pane
                                        .or(self.windows[active].tree.root)
                                    else {
                                        log::warn!(
                                            "SpawnPane: no target tile — window is empty, skipping"
                                        );
                                        self.active_window = original_active_window;
                                        continue;
                                    };
                                    (active, tile)
                                }
                            }
                        } else if let Some(tile) = self.windows[active].focused_pane {
                            (active, tile)
                        } else {
                            let Some(tile) = self.windows[active].tree.root else {
                                log::warn!("SpawnPane: no focused pane and empty tree — skipping");
                                self.active_window = original_active_window;
                                continue;
                            };
                            (active, tile)
                        };
                        log::info!(
                            "SpawnPane: terminal layout='{layout}' vertical={vertical} new_pane_first={new_pane_first} pane_id={new_pane_id} initial_cmd={initial_cmd:?} target_win={target_win}"
                        );
                        // SDK-spawned terminal with a cmd closes on exit (matches historical behavior).
                        // CLI terminal uses the ephemeral flag exclusively — cmd alone does not close.
                        // keep_focus=true: coordinator app always retains focus after spawning a terminal.
                        let _ = self.spawn_terminal_pane_at(
                            target_win,
                            target_tile,
                            vertical,
                            new_pane_first,
                            initial_cmd.as_deref(),
                            close_on_exit,
                            None,
                            true,
                        );
                    } else {
                        if let Some(from_id) = from_pane_id {
                            // When target_context already selected a window, restrict from_pane_id
                            // resolution to that window so it cannot override target_context.
                            let tile_opt = if target_context.is_some() {
                                let ctx_win = self.active_window;
                                self.windows[ctx_win]
                                    .tree
                                    .tiles
                                    .find_pane(&from_id)
                                    .map(|ft| (ctx_win, ft))
                            } else {
                                self.find_pane_in_any_window(from_id)
                            };
                            match tile_opt {
                                Some((fw, ft)) => {
                                    log::info!("SpawnPane: app: targeting from_pane_id={from_id} win_idx={fw}");
                                    self.active_window = fw;
                                    self.set_window_focused_pane(fw, ft);
                                }
                                None => {
                                    log::warn!("SpawnPane: app: from_pane_id={from_id} not found, using focused pane");
                                }
                            }
                        }
                        let _ = self.launch_app_by_id_with_layout(
                            &type_id,
                            Some(layout),
                            &effective_args,
                            None,
                        );
                        log::info!("SpawnPane: launched '{type_id}' pane_id={new_pane_id}");
                    }

                    // Restore active_window if we switched for target_context or from_pane_id.
                    self.active_window = original_active_window;

                    // Send PaneSpawned back to the requesting pane (may be
                    // in a different window than where the spawn landed).
                    if let Some(req_pane_id) = requesting_pane_id {
                        let win_idx = self
                            .windows
                            .iter()
                            .position(|w| w.panes.contains_key(&req_pane_id));
                        if let Some(wi) = win_idx {
                            if let Some(pane) = self.windows[wi].panes.get_mut(&req_pane_id) {
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
                }
                AppCommand::CdRequest {
                    cwd,
                    sender_pane_id,
                } => {
                    // Explicit, app-requested cd only (#2145). Two delivery
                    // targets: a linked terminal pane, or — for overlay apps —
                    // the hidden terminal stored in `overlay_replaced`, whose
                    // PTY stays alive while the overlay covers it.
                    let active = self.active_window;
                    let escaped = cwd.replace('\'', "'\\''");
                    let cd_cmd = format!("\x15cd '{}'\n", escaped);
                    let linked_id = self.windows[active]
                        .panes
                        .get(&sender_pane_id)
                        .and_then(|p| p.as_app())
                        .and_then(|a| a.linked_pane_id);
                    let mut delivered = false;
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
                            delivered = true;
                        }
                    }
                    if !delivered {
                        if let Some(t) = self.windows[active]
                            .panes
                            .get_mut(&sender_pane_id)
                            .and_then(|p| p.as_app_mut())
                            .and_then(|a| a.overlay_replaced.as_deref_mut())
                            .and_then(|p| p.as_terminal_mut())
                        {
                            t.backend.process_command(egui_term::BackendCommand::Write(
                                cd_cmd.as_bytes().to_vec(),
                            ));
                            log::info!(
                                "file_browser: CdRequest synced cwd '{}' to overlay-hidden terminal under pane {}",
                                cwd,
                                sender_pane_id
                            );
                        } else {
                            log::warn!(
                                "file_browser: CdRequest from pane {} found no linked or overlay-hidden terminal; cwd '{}' dropped",
                                sender_pane_id,
                                cwd
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
                    // Capture scope/source_context_id/source_window_id before they move into the struct.
                    let notif_scope = scope;
                    let notif_source_ctx = source_context_id;
                    let notif_source_win_id: u64 = if sender_pane_id != 0 {
                        self.find_pane_in_any_window(sender_pane_id)
                            .map(|(idx, _)| self.windows[idx].window_id)
                            .unwrap_or_else(|| self.windows[self.active_window].window_id)
                    } else {
                        self.windows[self.active_window].window_id
                    };
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
                        source_window_id: notif_source_win_id,
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
                    self.save_notifications();
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
                        crate::app_protocol::NotifyScope::Window => {
                            notif_source_win_id == self.windows[self.active_window].window_id
                        }
                        crate::app_protocol::NotifyScope::Context => {
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
                AppCommand::DeliverNotifyAction {
                    pane_id,
                    notify_id,
                    action_label,
                    value,
                    response_file,
                    host_action,
                } => {
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
                                log::warn!(
                                    "notify:action: pane_focus: invalid pane_id {:?}",
                                    id_str
                                );
                            }
                        } else {
                            log::warn!("notify:action: unknown host_action {:?}", action);
                        }
                    }
                    if let Some(rf) = &response_file {
                        let content = value.as_deref().unwrap_or("");
                        let tmp = format!("{rf}.tmp");
                        match std::fs::write(&tmp, content).and_then(|_| std::fs::rename(&tmp, rf))
                        {
                            Ok(_) => log::info!("notify:action: wrote {:?} to {:?}", content, rf),
                            Err(e) => log::warn!(
                                "notify:action: failed to write response file {:?}: {e}",
                                rf
                            ),
                        }
                    }
                    // Search all windows for the sender pane — it may not be in the
                    // active context (cross-context notification path).
                    let window_idx = self
                        .windows
                        .iter()
                        .position(|w| w.panes.contains_key(&pane_id));
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
                AppCommand::DeliverPipeMessage {
                    sender_pane_id,
                    pipe_id,
                    payload,
                } => {
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
                                crate::host::pane::AppRuntime::Process(pa) => {
                                    pa.pipe_registry.lock().unwrap().has_reader(&pipe_id)
                                }
                                crate::host::pane::AppRuntime::Builtin(_) => false,
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
                    let target_kind =
                        self.windows[active]
                            .panes
                            .get(&target_pane_id)
                            .map(|p| match p {
                                crate::host::pane::Pane::App(_) => "app",
                                crate::host::pane::Pane::Terminal(_) => "terminal",
                                crate::host::pane::Pane::Portal(_) => "portal",
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
                AppCommand::OpenArtifact {
                    sender_pane_id,
                    path,
                    mode,
                } => {
                    self.dispatch_open_artifact(sender_pane_id, path, mode);
                }
                AppCommand::QueryContextState {
                    sender_pane_id,
                    context_id,
                } => {
                    // Visibility check: the requesting pane must be in the
                    // queried context itself or in an ancestor of it.
                    let requester_context_id = self
                        .windows
                        .iter()
                        .find(|w| w.panes.contains_key(&sender_pane_id))
                        .map(|w| w.context_id)
                        .unwrap_or(0);

                    let is_self = requester_context_id == context_id;
                    let is_ancestor = if !is_self {
                        self.host
                            .ancestors_of(context_id)
                            .contains(&requester_context_id)
                    } else {
                        false
                    };

                    if !is_self && !is_ancestor {
                        log::warn!(
                            "QueryContextState: pane {sender_pane_id} in context {requester_context_id} \
                             cannot query context {context_id} — not an ancestor"
                        );
                        continue;
                    }

                    let state = crate::host::context_state::ContextState::compute(
                        context_id,
                        self.router.as_slice(),
                        &self.windows,
                    );
                    log::info!(
                        "QueryContextState: context_id={context_id} pane_count={} children={} status={:?}",
                        state.pane_count,
                        state.children.len(),
                        state.status,
                    );

                    // Send response back to the requesting pane.
                    let window_idx = self
                        .windows
                        .iter()
                        .position(|w| w.panes.contains_key(&sender_pane_id));
                    if let Some(win_idx) = window_idx {
                        if let Some(pane) = self.windows[win_idx].panes.get_mut(&sender_pane_id) {
                            if let Some(app) = pane.as_app_mut() {
                                app.runtime.queue_outbound_event(
                                    crate::app_protocol::PlexiEvent::ContextStateResponse { state },
                                );
                            }
                        }
                    }
                }
                AppCommand::DeliverRunUpdate {
                    originator_type_id,
                    event,
                } => {
                    let active = self.active_window;
                    let pane_ids: Vec<_> = self.windows[active].panes.keys().copied().collect();
                    let mut delivered = false;
                    for pid in pane_ids {
                        let matches = self.windows[active]
                            .panes
                            .get(&pid)
                            .and_then(|p| p.as_app())
                            .map(|a| match &a.runtime {
                                crate::host::pane::AppRuntime::Process(pa) => {
                                    pa.type_id == originator_type_id
                                }
                                crate::host::pane::AppRuntime::Builtin(_) => false,
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
                // The keystroke that triggered the close (e.g. `t` in the file
                // browser) is still in this frame's input queue. The restored
                // terminal renders later this same frame and would forward it
                // to its PTY — swallow all key/text events before closing.
                ctx.input_mut(|i| {
                    i.events.retain(|e| {
                        !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_))
                    });
                });
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
            let window_count = self
                .windows
                .iter()
                .filter(|c| c.context_id == ws_id)
                .count();
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

        // Handle keyboard shortcuts. Global shortcuts (Cmd+Q, Cmd+W, Cmd+P) always
        // fire; all other shortcuts are suppressed when an overlay holds focus via
        // the early-return guard in `keys::poll_actions`.
        let modal_open = self.input_captured_by_overlay();
        for action in keys::poll_actions(
            ctx,
            &self.binding_table,
            app_active,
            keyboard_capture_active,
            modal_open,
            self.show_shortcuts,
        ) {
            match action {
                Action::SplitHorizontal => {
                    self.windows[self.active_window].clear_zoom();
                    self.ctx.memory_mut(|m| {
                        if let Some(id) = m.focused() {
                            m.surrender_focus(id);
                        }
                    });
                    self.split_focused(false, None, false, false, None);
                    self.save_workspace();
                }
                Action::SplitVertical => {
                    self.windows[self.active_window].clear_zoom();
                    self.ctx.memory_mut(|m| {
                        if let Some(id) = m.focused() {
                            m.surrender_focus(id);
                        }
                    });
                    self.split_focused(true, None, false, false, None);
                    self.save_workspace();
                }
                Action::SplitRight => {
                    self.windows[self.active_window].clear_zoom();
                    self.ctx.memory_mut(|m| {
                        if let Some(id) = m.focused() {
                            m.surrender_focus(id);
                        }
                    });
                    self.split_focused_mirror(crate::host::command::Placement::Right);
                    self.save_workspace();
                }
                Action::SplitDown => {
                    self.windows[self.active_window].clear_zoom();
                    self.ctx.memory_mut(|m| {
                        if let Some(id) = m.focused() {
                            m.surrender_focus(id);
                        }
                    });
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
                        if let Some(tile) = new_pane {
                            self.windows[self.active_window].zoom_to(tile);
                        }
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
                Action::SwapPane(dir) => match self.swap_pane(dir) {
                    crate::pane_ops::SwapResult::Swapped { rect_a, rect_b, .. } => {
                        let now = std::time::Instant::now();
                        self.pane_anims = vec![
                            PaneSwapAnim {
                                from: rect_a,
                                to: rect_b,
                                started_at: now,
                            },
                            PaneSwapAnim {
                                from: rect_b,
                                to: rect_a,
                                started_at: now,
                            },
                        ];
                        self.ctx.request_repaint();
                    }
                    crate::pane_ops::SwapResult::AtBoundary => {
                        let moved = match dir {
                            crate::host::keys::Direction::Down => {
                                self.move_focused_pane_to_row_boundary(true)
                            }
                            crate::host::keys::Direction::Up => {
                                self.move_focused_pane_to_row_boundary(false)
                            }
                            _ => self.move_focused_pane_to_adjacent_window(dir),
                        };
                        if moved {
                            self.ctx.request_repaint();
                        } else if let Some(focused) = self.windows[self.active_window].focused_pane
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
                },
                Action::SendPane(dir) => {
                    match self.send_pane(dir) {
                        crate::pane_ops::SwapResult::Swapped { rect_a, rect_b, .. } => {
                            let now = std::time::Instant::now();
                            self.pane_anims = vec![
                                PaneSwapAnim {
                                    from: rect_a,
                                    to: rect_b,
                                    started_at: now,
                                },
                                PaneSwapAnim {
                                    from: rect_b,
                                    to: rect_a,
                                    started_at: now,
                                },
                            ];
                            self.ctx.request_repaint();
                        }
                        crate::pane_ops::SwapResult::AtBoundary => {
                            // For U/D at boundary: edge-pulse only (no row-boundary move).
                            // For L/R at boundary: try cross-window send (focus stays on source).
                            let moved = match dir {
                                crate::host::keys::Direction::Down
                                | crate::host::keys::Direction::Up => false,
                                _ => self.send_pane_to_adjacent_window(dir),
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
                        if let Some(child_ctx_id) = self.get_focused_portal_context_id() {
                            let state = self.build_context_close_state(child_ctx_id);
                            if state.items.is_empty() {
                                // Empty context — close immediately, no dialog needed.
                                let idx = self
                                    .router
                                    .iter()
                                    .position(|c| c.context_id == child_ctx_id);
                                if let Some(i) = idx {
                                    log::info!("context_close: empty ctx={child_ctx_id} — closing immediately");
                                    self.delete_context(i);
                                    self.save_workspace();
                                }
                            } else {
                                log::info!("context_close: ctx={child_ctx_id} has {} panes — showing dialog", state.items.len());
                                self.pending_context_close = Some(state);
                            }
                        } else if self.confirm_close() {
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
                    let (focused_tile, portal_target) = {
                        let win = &self.windows[self.active_window];
                        let tile = win.focused_pane;
                        let target = tile
                            .and_then(|tile_id| win.tree.tiles.get(tile_id))
                            .and_then(|t| {
                                if let egui_tiles::Tile::Pane(p) = t {
                                    Some(*p)
                                } else {
                                    None
                                }
                            })
                            .and_then(|pane_id| win.panes.get(&pane_id))
                            .and_then(|p| p.portal_target());
                        (tile, target)
                    };
                    if let Some(child_ctx_id) = portal_target {
                        // Focused pane is a Portal tile → zoom into its sub-context.
                        // Verify target context exists BEFORE pushing depth state;
                        // push_depth must not fire if there is nothing to switch to.
                        if let Some(ctx_idx) =
                            self.router.position(|c| c.context_id == child_ctx_id)
                        {
                            log::info!(
                                "ToggleZoom on Portal: zooming into context_id={child_ctx_id}"
                            );
                            let current_ctx_id = self.router.active().context_id;
                            let current_win_id = self.windows[self.active_window].window_id;
                            self.router
                                .push_depth(current_ctx_id, current_win_id, focused_tile);
                            self.switch_workspace(ctx_idx);
                        } else {
                            log::warn!("ToggleZoom on Portal: target context_id={child_ctx_id} not found in router");
                        }
                    } else {
                        // Non-Portal pane → toggle fullscreen zoom.
                        let ctx = &mut self.windows[self.active_window];
                        if let Some(focused) = ctx.focused_pane {
                            if ctx.zoomed_pane == Some(focused) {
                                ctx.clear_zoom();
                                log::info!("zoom: toggle off — pane={focused:?}");
                            } else {
                                ctx.zoom_to(focused);
                                log::info!("zoom: toggle on — pane={focused:?}");
                            }
                            self.ctx.memory_mut(|m| {
                                if let Some(id) = m.focused() {
                                    m.surrender_focus(id);
                                }
                            });
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
                        self.ctx.memory_mut(|m| {
                            if let Some(id) = m.focused() {
                                m.surrender_focus(id);
                            }
                        });
                        self.palette_query.clear();
                        self.palette_selected = 0;
                        // Resolve focused pane workspace once at open-time — not per draw-frame —
                        // to avoid filesystem traversal in the egui hot path.
                        let win = &self.windows[self.active_window];
                        self.palette_workspace_root = win
                            .focused_pane
                            .and_then(|tile_id| win.get_focused_pane_cwd(tile_id))
                            .and_then(|cwd| crate::app::registry::resolve_workspace_root(&cwd));
                        log::info!(
                            "palette: opened, focused workspace = {:?}",
                            self.palette_workspace_root
                        );
                    } else {
                        self.palette_workspace_root = None;
                    }
                }
                Action::RenamePane => {
                    self.ctx.memory_mut(|m| {
                        if let Some(id) = m.focused() {
                            m.surrender_focus(id);
                        }
                    });
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
                Action::HidePane => {
                    let win = &mut self.windows[self.active_window];
                    if let Some(focused_tile) = win.focused_pane {
                        if let Some(Tile::Pane(pane_id)) = win.tree.tiles.get(focused_tile) {
                            let pane_id = *pane_id;
                            if let Some(pane) = win.panes.get_mut(&pane_id) {
                                let new_val = !pane.is_hidden();
                                pane.set_hidden(new_val);
                                let name = match pane {
                                    Pane::Terminal(t) => t.name.clone().unwrap_or_default(),
                                    Pane::App(a) => a.name.clone(),
                                    Pane::Portal(_) => String::new(),
                                };
                                log::info!(
                                    "pane_hide: pane={pane_id} name={name:?} hidden={new_val}"
                                );
                                self.save_workspace();
                            }
                        }
                    }
                }
                Action::SwitchContext(n) => {
                    // Map display position n (0-indexed) to the actual router index by
                    // computing the same active display order the sidebar renders.
                    let num = self.router.len();
                    let mut display_order: Vec<usize> = Vec::with_capacity(num);
                    for i in 0..num {
                        if self.router.get(i).parent_id.is_none() {
                            display_order.push(i);
                            let ctx_id = self.router.get(i).context_id;
                            for j in 0..num {
                                if self.router.get(j).parent_id == Some(ctx_id) {
                                    display_order.push(j);
                                }
                            }
                        }
                    }
                    for i in 0..num {
                        if !display_order.contains(&i) {
                            display_order.push(i);
                        }
                    }
                    let active_order: Vec<usize> = display_order
                        .into_iter()
                        .filter(|&i| !self.router.get(i).parked)
                        .collect();
                    if let Some(&router_idx) = active_order.get(n) {
                        let target_parent = self.router.get(router_idx).parent_id;
                        let current_ctx_id = self.router.active().context_id;
                        if target_parent == Some(current_ctx_id) {
                            let current_win_id = self.windows[self.active_window].window_id;
                            let focused_tile = self.windows[self.active_window].focused_pane;
                            self.router
                                .push_depth(current_ctx_id, current_win_id, focused_tile);
                        }
                        log::info!(
                            "SwitchContext: display_pos={} → router_idx={}",
                            n + 1,
                            router_idx
                        );
                        self.switch_workspace(router_idx);
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
                    self.navigate(crate::host::keys::Direction::Down);
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
                Action::OpenAssistant => {
                    self.open_assistant_pane();
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
                Action::NewChildContext => {
                    self.new_child_context_from_keyboard();
                }
                Action::PushPaneToSubcontext => {
                    self.push_pane_to_subcontext(None);
                }
                Action::SetContextRootFromCwd => {
                    let active = self.active_window;
                    if let Some(tile_id) = self.windows[active].focused_pane {
                        if let Some(cwd) = self.windows[active].get_focused_pane_cwd(tile_id) {
                            log::info!("SetContextRootFromCwd: setting root to {}", cwd.display());
                            self.set_active_context_root(cwd);
                        } else {
                            log::warn!("SetContextRootFromCwd: no CWD available for focused pane");
                        }
                    }
                }
                Action::ParkContext => {
                    self.toggle_park_active_context();
                }
                Action::ToggleMinimap => {
                    self.minimap.toggle();
                }
                Action::OpenScratchpad => {
                    log::info!("scratchpad: Cmd+Shift+Space — opening");
                    self.open_scratchpad();
                }
                Action::OpenNotesPicker => {
                    log::info!("notes_picker: Cmd+O — opening picker");
                    self.open_notes_picker();
                }
                Action::OpenNotesTriage => {
                    if !self.focus_stack.contains(&FocusLayer::NotesTriage) {
                        log::info!("notes_triage: opening triage overlay");
                        self.open_notes_triage();
                    } else {
                        self.pop_focus_layer(&FocusLayer::NotesTriage);
                    }
                }
                Action::ContextZoomOut => {
                    log::info!(
                        "ContextZoomOut: popping depth stack (depth={})",
                        self.router.current_depth()
                    );
                    if let Some((parent_ctx_id, parent_win_id, focused_tile)) =
                        self.router.pop_depth()
                    {
                        if let Some(ctx_idx) =
                            self.router.position(|c| c.context_id == parent_ctx_id)
                        {
                            self.switch_workspace(ctx_idx);
                            if let Some(win_idx) = self
                                .windows
                                .iter()
                                .position(|w| w.window_id == parent_win_id)
                            {
                                self.active_window = win_idx;
                                self.windows[win_idx].focused_pane = focused_tile;
                            }
                        }
                    }
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
        crate::platform::macos_menu::apply_version_title_once();
        if crate::platform::macos_menu::take_reload_config_flag() {
            self.reload_config();
        }

        // Config hot-reload (#1115): drain filesystem watcher signals.
        let config_changed = self.config_reload_rx.as_ref().map_or(false, |rx| {
            let hit = rx.try_recv().is_ok();
            if hit {
                while rx.try_recv().is_ok() {}
            }
            hit
        });
        if config_changed {
            self.reload_config();
        }

        // Auto-switch paired theme variant on macOS appearance change (#1776, #1812).
        let current_system_theme = self.ctx.system_theme();
        if current_system_theme != self.last_system_theme {
            self.last_system_theme = current_system_theme;
            if let Some(sys_theme) = current_system_theme {
                self.apply_auto_theme(sys_theme);
            }
        }

        // App registry hot-reload (#1712): drain filesystem watcher signals.
        let registry_changed = self.registry_reload_rx.as_ref().map_or(false, |rx| {
            let hit = rx.try_recv().is_ok();
            if hit {
                while rx.try_recv().is_ok() {}
            }
            hit
        });
        if registry_changed {
            let root = self.router.active().root.clone().unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_else(|_| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
            });
            log::info!(
                "app_registry_watcher: rescanning registry for root={}",
                root.display()
            );
            self.registry = crate::app::registry::AppRegistry::load(&root);
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

        self.render_panels(ctx);

        // Detect genuine pane focus transitions, and periodically bank long
        // same-pane sessions so Stats has live data without keystroke tracking.
        self.reconcile_focus_logging(FOCUS_HEARTBEAT_INTERVAL);
    }

    fn on_exit(&mut self) {
        if let Some((window_id, tile_id)) = self.last_logged_focus {
            let duration_secs = self
                .focus_started_at
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0);
            log::info!(
                "focus_changed: shutdown — banking final session duration_secs={duration_secs}"
            );
            self.emit_focus_changed_for_tile(
                window_id,
                tile_id,
                duration_secs,
                FocusSegmentReason::Shutdown,
            );
        }
    }
}

impl PlexiApp {
    pub(crate) fn open_notes_picker(&mut self) {
        let notes_base = crate::config::config_dir().join("notes");
        let workspace_slug = crate::config::active_workspace_root()
            .and_then(|p| p.file_name().map(|n| n.to_os_string()))
            .map(|n| n.to_string_lossy().into_owned());
        let notes_dir = match workspace_slug {
            Some(ref slug) => notes_base.join(slug),
            None => notes_base,
        };
        let mut with_mtime: Vec<(std::time::SystemTime, std::path::PathBuf, String)> =
            std::fs::read_dir(&notes_dir)
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map_or(false, |x| x == "md"))
                .filter_map(|e| {
                    let path = e.path();
                    let mtime = e.metadata().and_then(|m| m.modified()).ok()?;
                    let preview = std::fs::read_to_string(&path).unwrap_or_default();
                    let first_line = preview
                        .lines()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("")
                        .to_string();
                    Some((mtime, path, first_line))
                })
                .collect();
        with_mtime.sort_by(|a, b| b.0.cmp(&a.0));
        let entries: Vec<(std::path::PathBuf, String)> =
            with_mtime.into_iter().map(|(_, p, l)| (p, l)).collect();
        log::info!(
            "notes_picker: {} notes found in {:?}",
            entries.len(),
            notes_dir
        );
        self.notes_picker_entries = entries;
        self.notes_picker_selected = 0;
        self.push_focus_layer(FocusLayer::NotesPicker);
        // Surrender egui keyboard focus from the active TextEdit so the picker
        // receives j/k and other navigation keys immediately on the first frame.
        self.ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
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
fn register_directed_pipe_on_target(pane: &mut crate::host::pane::Pane, pipe_id: &str) -> bool {
    use crate::host::typed_pipes::PipeDirection;
    let registry = match pane {
        crate::host::pane::Pane::App(app) => match &app.runtime {
            crate::host::pane::AppRuntime::Process(pa) => Some(pa.pipe_registry.clone()),
            crate::host::pane::AppRuntime::Builtin(_) => None,
        },
        crate::host::pane::Pane::Terminal(_) | crate::host::pane::Pane::Portal(_) => None,
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
        Err(crate::host::typed_pipes::PipeError::AlreadyOpen(_)) => true,
        Err(e) => {
            log::warn!("register_directed_pipe_on_target: open_json failed: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests;
