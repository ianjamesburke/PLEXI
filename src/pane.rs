use crate::app_permissions::AppPermissions;
use crate::app_trait::{App, SurfaceLayer, SurfaceMode};
use crate::tiling::PaneId;
use egui_term::{BackendSettings, PtyEvent, TerminalBackend};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

// ---------------------------------------------------------------------------
// Pane ADT (spec §2)
// ---------------------------------------------------------------------------

pub enum Pane {
    Terminal(TerminalPane),
    App(AppPane),
    Agent(AgentPane),
}

impl Pane {
    pub fn id(&self) -> PaneId {
        match self {
            Pane::Terminal(t) => t.id,
            Pane::App(a) => a.id,
            Pane::Agent(a) => a.id,
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

    pub fn as_agent(&self) -> Option<&AgentPane> {
        match self {
            Pane::Agent(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_agent_mut(&mut self) -> Option<&mut AgentPane> {
        match self {
            Pane::Agent(a) => Some(a),
            _ => None,
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            Pane::Terminal(_) => "terminal",
            Pane::App(_) => "app",
            Pane::Agent(_) => "agent",
        }
    }
}

// ---------------------------------------------------------------------------
// TerminalPane — v2-compatible; overlay model preserved for now
// ---------------------------------------------------------------------------

pub struct TerminalPane {
    /// Pane ID — matches the key used in HashMap<PaneId, Pane>.
    pub id: PaneId,
    pub backend: TerminalBackend,
    pub exited: bool,
    pub name: Option<String>,
    pub font_size: f32,
    /// Active app surface overlaid on this terminal, if any.
    pub active_app: Option<Box<dyn App>>,
    pub surface_mode: SurfaceMode,
    /// Which surface has keyboard focus when an app is active.
    pub focused_surface: SurfaceLayer,
    /// Effective permissions for the active app. Reset when app closes.
    pub app_permissions: AppPermissions,
    /// Directory the app was launched from. Commands are scoped to this.
    pub app_scope: Option<PathBuf>,
    /// When an app opens via auto-split, this is the PaneId of the terminal
    /// pane created below. Used to route AppCommands and to collapse the split
    /// on close.
    pub linked_terminal_pane: Option<PaneId>,
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
            font_size: default_font_size,
            active_app: None,
            surface_mode: SurfaceMode::FullTerminal,
            focused_surface: SurfaceLayer::App,
            app_permissions: AppPermissions::default(),
            app_scope: None,
            linked_terminal_pane: None,
        })
    }

    /// Open an app — app gets focus immediately.
    pub fn open_app(&mut self, app: Box<dyn App>, permissions: AppPermissions, scope: PathBuf) {
        self.active_app = Some(app);
        self.surface_mode = SurfaceMode::AppActive;
        self.focused_surface = SurfaceLayer::App;
        self.app_permissions = permissions;
        self.app_scope = Some(scope);
    }

    /// Close the active app and return to full terminal mode.
    /// Returns the linked terminal pane ID if there was one (caller should close it).
    pub fn close_app(&mut self) -> Option<PaneId> {
        self.active_app = None;
        self.surface_mode = SurfaceMode::FullTerminal;
        self.focused_surface = SurfaceLayer::App;
        self.app_permissions = AppPermissions::default();
        self.app_scope = None;
        self.linked_terminal_pane.take()
    }

    /// Toggle keyboard focus between the app and the terminal command bar.
    /// No-op if no app is active.
    pub fn toggle_surface_focus(&mut self) {
        if self.surface_mode == SurfaceMode::AppActive {
            self.focused_surface = match self.focused_surface {
                SurfaceLayer::App => SurfaceLayer::Terminal,
                SurfaceLayer::Terminal => SurfaceLayer::App,
            };
        }
    }
}

// ---------------------------------------------------------------------------
// AppPane — wraps an external PGAP v3 child process (Layer 3b wires this)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub struct AppPane {
    pub id: PaneId,
    pub process: crate::process_app::ProcessApp,
    pub workspace_root: PathBuf,
    pub permissions: AppPermissions,
    pub manifest_id: String,
    pub name: String,
    /// Pane group this app joined at spawn (for PathChanged routing).
    pub pane_group: Option<String>,
}

// ---------------------------------------------------------------------------
// AgentPane — wraps a PlexiIqInstance (Layer 3c wires this)
// ---------------------------------------------------------------------------

pub struct AgentPane {
    pub id: PaneId,
    /// Lazy-initialized; may be None until first turn.
    pub instance: Option<crate::plexi_iq::PlexiIqInstance>,
    /// Human-readable session label for the UI.
    pub label: String,
    /// Accumulated conversation lines (user messages + assistant chunks).
    pub transcript: Vec<String>,
    /// Current user input being typed.
    pub input_buf: String,
    /// Streamed token receiver from the background turn thread.
    /// None when no turn is in progress.
    pub turn_rx: Option<std::sync::mpsc::Receiver<TurnMsg>>,
    /// Session ID from the last completed proxied turn (for --resume).
    pub session_id: Option<String>,
}

/// Message from the background turn thread to the UI thread.
pub enum TurnMsg {
    Token(String),
    Done { session_id: Option<String>, token_count: usize },
    Error(String),
}
