use crate::app::app_trait::{App, AppCommand, AppRenderContext};
use crate::app::permissions::AppPermissions;
use crate::spatial::tiling::PaneId;
use egui_term::{BackendSettings, PtyEvent, TerminalBackend};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

// ---------------------------------------------------------------------------
// Pane ADT (spec §2) — Terminal | App | Portal.
// Issue #1374 added the Portal variant (formerly SubContext).
// ---------------------------------------------------------------------------

pub enum Pane {
    Terminal(Box<TerminalPane>),
    App(Box<AppPane>),
    /// A tile that represents a child context nested inside this one.
    /// Renders a summary card with pane count, status, and per-pane summaries.
    /// Cmd+Enter zooms into the sub-context when this tile has focus.
    Portal(Box<PortalPane>),
}

/// A portal pane points at a child context and caches its rolled-up state.
pub struct PortalPane {
    pub pane_id: PaneId,
    pub target_context_id: u64,
    pub context_state: Option<crate::host::context_state::ContextState>,
    /// When true, the pane is visually deprioritized (outline dot, dimmed tab title).
    pub hidden: bool,
}

impl Pane {
    pub fn id(&self) -> PaneId {
        match self {
            Pane::Terminal(t) => t.id,
            Pane::App(a) => a.id,
            Pane::Portal(p) => p.pane_id,
        }
    }

    pub fn is_hidden(&self) -> bool {
        match self {
            Pane::Terminal(t) => t.hidden,
            Pane::App(a) => a.hidden,
            Pane::Portal(p) => p.hidden,
        }
    }

    pub fn set_hidden(&mut self, val: bool) {
        match self {
            Pane::Terminal(t) => t.hidden = val,
            Pane::App(a) => a.hidden = val,
            Pane::Portal(p) => p.hidden = val,
        }
    }

    pub fn as_terminal(&self) -> Option<&TerminalPane> {
        match self {
            Pane::Terminal(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_terminal_mut(&mut self) -> Option<&mut TerminalPane> {
        match self {
            Pane::Terminal(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_app(&self) -> Option<&AppPane> {
        match self {
            Pane::App(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_app_mut(&mut self) -> Option<&mut AppPane> {
        match self {
            Pane::App(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_portal(&self) -> Option<&PortalPane> {
        match self {
            Pane::Portal(p) => Some(p),
            _ => None,
        }
    }

    pub fn as_portal_mut(&mut self) -> Option<&mut PortalPane> {
        match self {
            Pane::Portal(p) => Some(p),
            _ => None,
        }
    }

    /// Returns the target context_id if this is a Portal pane.
    pub fn portal_target(&self) -> Option<u64> {
        match self {
            Pane::Portal(p) => Some(p.target_context_id),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// TerminalPane — PTY-only state
// ---------------------------------------------------------------------------

pub struct TerminalPane {
    /// Pane ID — matches the key used in HashMap<PaneId, Pane>.
    pub id: PaneId,
    pub backend: TerminalBackend,
    pub exited: bool,
    pub name: Option<String>,
    /// When true, the name was set explicitly by the user and OSC title sequences must not overwrite it.
    pub name_locked: bool,
    pub font_size: f32,
    /// When true, the pane closes automatically when its process exits (no "[process exited]" prompt).
    /// Set by `plexi terminal --ephemeral`.
    pub ephemeral: bool,
    /// Last OSC 2 title string the process wrote, tracked independently of `name` and `name_locked`.
    /// Used by FocusChanged events to record what was running in the pane.
    pub pty_title: Option<String>,
    /// Cached result for the workspace-scope badge. The actual probe hits the
    /// OS for the child process cwd, so the render path throttles it.
    pub(crate) outside_workspace_cached: bool,
    pub(crate) outside_workspace_checked_at: Option<std::time::Instant>,
    pub(crate) outside_workspace_root: Option<PathBuf>,
    /// When true, the pane is visually deprioritized (outline dot, dimmed tab title).
    pub hidden: bool,
}

impl TerminalPane {
    pub fn new(
        id: u64,
        ctx: egui::Context,
        tx: Sender<(u64, PtyEvent)>,
        settings: BackendSettings,
        default_font_size: f32,
    ) -> Option<Self> {
        let backend = match TerminalBackend::new(id, ctx, tx, settings) {
            Ok(b) => b,
            Err(e) => {
                log::error!("Failed to create terminal backend {id}: {e}");
                return None;
            }
        };
        Some(Self {
            id,
            backend,
            exited: false,
            name: None,
            name_locked: false,
            font_size: default_font_size,
            ephemeral: false,
            pty_title: None,
            outside_workspace_cached: false,
            outside_workspace_checked_at: None,
            outside_workspace_root: None,
            hidden: false,
        })
    }
}

// ---------------------------------------------------------------------------
// AppPane — dedicated app runtime (process or in-process builtin)
// ---------------------------------------------------------------------------

pub enum AppRuntime {
    Process(Box<crate::process_app::ProcessApp>),
    Builtin(Box<dyn App>),
}

impl AppRuntime {
    pub fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        match self {
            AppRuntime::Process(app) => app.ui(ui, ctx),
            AppRuntime::Builtin(app) => app.ui(ui, ctx),
        }
    }

    pub fn handle_key(
        &mut self,
        input: &egui::InputState,
    ) -> crate::app::app_trait::KeyDisposition {
        match self {
            AppRuntime::Process(app) => app.handle_key(input),
            AppRuntime::Builtin(app) => app.handle_key(input),
        }
    }

    pub fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        match self {
            AppRuntime::Process(app) => app.take_pending_commands(),
            AppRuntime::Builtin(app) => app.take_pending_commands(),
        }
    }

    pub fn keyboard_capture(&self) -> bool {
        match self {
            AppRuntime::Process(app) => app.keyboard_capture(),
            AppRuntime::Builtin(app) => app.keyboard_capture(),
        }
    }

    pub fn wants_close(&self) -> bool {
        match self {
            AppRuntime::Process(app) => app.wants_close(),
            AppRuntime::Builtin(app) => app.wants_close(),
        }
    }

    pub fn queue_outbound_event(&mut self, event: crate::app_protocol::PlexiEvent) {
        match self {
            AppRuntime::Process(app) => app.queue_outbound_event(event),
            AppRuntime::Builtin(app) => app.queue_outbound_event(event),
        }
    }

    pub fn sync_cwd(&mut self, new_cwd: &std::path::Path) {
        match self {
            AppRuntime::Process(app) => app.sync_cwd(new_cwd),
            AppRuntime::Builtin(app) => app.sync_cwd(new_cwd),
        }
    }

    pub fn current_cwd(&self) -> Option<std::path::PathBuf> {
        match self {
            AppRuntime::Process(app) => app.current_cwd(),
            AppRuntime::Builtin(app) => app.current_cwd(),
        }
    }

    pub fn type_id(&self) -> &'static str {
        match self {
            AppRuntime::Process(app) => app.type_id(),
            AppRuntime::Builtin(app) => app.type_id(),
        }
    }

    pub fn serialize_state(&self) -> Option<serde_json::Value> {
        match self {
            AppRuntime::Process(app) => app.serialize_state(),
            AppRuntime::Builtin(app) => app.serialize_state(),
        }
    }

    /// Pump event I/O for a pane not in the active context. No-op for builtins.
    pub fn background_tick(&mut self) {
        match self {
            AppRuntime::Process(app) => app.background_tick(),
            AppRuntime::Builtin(_) => {}
        }
    }

    /// Current nav stack depth as reported by the app via `PushNav`/`PopNav`.
    /// Always 0 for builtin apps — they manage their own internal navigation.
    pub fn nav_stack_depth(&self) -> usize {
        match self {
            AppRuntime::Process(app) => app.nav_stack_depth(),
            AppRuntime::Builtin(_) => 0,
        }
    }

    /// Title of the current top-of-stack view for pane chrome display.
    /// `None` when the stack is empty (root view — no back arrow shown).
    pub fn nav_top_title(&self) -> Option<&str> {
        match self {
            AppRuntime::Process(app) => app.nav_top_title(),
            AppRuntime::Builtin(_) => None,
        }
    }

    /// The `view_id` the app should navigate back to (the entry below current
    /// top, or empty string for root). Used to populate `NavBack { view_id }`.
    pub fn nav_back_view_id(&self) -> String {
        match self {
            AppRuntime::Process(app) => app.nav_back_view_id(),
            AppRuntime::Builtin(_) => String::new(),
        }
    }

    pub(crate) fn set_pending_notification_count(&mut self, count: usize) {
        if let AppRuntime::Process(app) = self {
            app.pending_notification_count = count;
        }
    }

    /// Serialize the last-rendered frame (Vec<RenderCommand>) as a JSON array.
    /// Returns `None` for builtin apps (no accessible frame).
    pub(crate) fn frame_json(&self) -> Option<serde_json::Value> {
        match self {
            AppRuntime::Process(app) => serde_json::to_value(&app.frame).ok(),
            AppRuntime::Builtin(_) => None,
        }
    }
}

#[allow(dead_code)]
pub struct AppPane {
    pub id: PaneId,
    pub runtime: AppRuntime,
    pub workspace_root: PathBuf,
    pub permissions: AppPermissions,
    pub manifest_id: String,
    pub name: String,
    /// Pane group this app joined at spawn (for PathChanged routing).
    pub pane_group: Option<String>,
    /// The terminal pane this app was spawned alongside. CdRequest routes here
    /// directly — no tile-tree walk needed.
    pub linked_pane_id: Option<PaneId>,
    /// Pane hidden by an overlay app. Closing the app restores this pane instead
    /// of deleting the tile.
    pub overlay_replaced: Option<Box<Pane>>,
    /// When true, the pane is visually deprioritized (outline dot, dimmed tab title).
    pub hidden: bool,
}
