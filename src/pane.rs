use crate::app_permissions::AppPermissions;
use crate::app_trait::{App, AppCommand, AppRenderContext};
use crate::tiling::PaneId;
use egui_term::{BackendSettings, PtyEvent, TerminalBackend};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

// ---------------------------------------------------------------------------
// Pane ADT (spec §2) — Terminal | App | Agent. Adding a variant requires a spec amendment.
// ---------------------------------------------------------------------------

pub enum Pane {
    Terminal(Box<TerminalPane>),
    App(Box<AppPane>),
    Agent(Box<AgentPane>),
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

    pub fn as_agent_mut(&mut self) -> Option<&mut AgentPane> {
        match self {
            Pane::Agent(a) => Some(a),
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
    pub font_size: f32,
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

    pub fn handle_key(&mut self, input: &egui::InputState) -> bool {
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
}

// AgentPane is defined in agent_pane.rs — re-exported here so Pane variants compile.
pub use crate::agent_pane::AgentPane;

