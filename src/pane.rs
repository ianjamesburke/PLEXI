use crate::agent_mode::AgentMode;
use crate::app_permissions::AppPermissions;
use crate::app_trait::{App, SurfaceLayer, SurfaceMode, DEFAULT_COMPANION_FRACTION};
use crate::tiling::PaneId;
use egui_term::{BackendSettings, PtyEvent, TerminalBackend};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

pub struct TerminalPane {
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
    /// Agent mode state for this pane.
    pub agent_mode: AgentMode,
    /// When true, Cmd+W will not close this pane.
    pub locked: bool,
}

/// Top-level pane primitive. A pane is one of several variants; this enum
/// locks the ADT shape that later PRs will flesh out (AgentPane, AppPane).
/// PR1: single variant wrapping the existing `TerminalPane` struct.
pub enum Pane {
    Terminal(TerminalPane),
    // TODO(PR2): Agent(AgentPane),
    // TODO(PR2): App(AppPane),
}

impl Pane {
    #[inline]
    pub fn as_terminal(&self) -> Option<&TerminalPane> {
        match self {
            Pane::Terminal(p) => Some(p),
        }
    }

    #[inline]
    pub fn as_terminal_mut(&mut self) -> Option<&mut TerminalPane> {
        match self {
            Pane::Terminal(p) => Some(p),
        }
    }

    /// PR1 convenience: until sibling variants exist, every pane is a terminal.
    /// Call sites can use this when they know the variant. In PR2, these
    /// become fallible matches on specific variants.
    #[inline]
    pub fn terminal(&self) -> &TerminalPane {
        match self {
            Pane::Terminal(p) => p,
        }
    }

    #[inline]
    pub fn terminal_mut(&mut self) -> &mut TerminalPane {
        match self {
            Pane::Terminal(p) => p,
        }
    }
}

// TODO(PR2): remove Deref/DerefMut. They exist only to minimize PR1 call-site
// churn while wrapping the single existing variant. When sibling variants
// (Agent, App) land, callers must match the variant explicitly.
impl std::ops::Deref for Pane {
    type Target = TerminalPane;
    #[inline]
    fn deref(&self) -> &TerminalPane {
        match self {
            Pane::Terminal(p) => p,
        }
    }
}

impl std::ops::DerefMut for Pane {
    #[inline]
    fn deref_mut(&mut self) -> &mut TerminalPane {
        match self {
            Pane::Terminal(p) => p,
        }
    }
}

impl TerminalPane {
    pub fn new(
        id: u64,
        ctx: egui::Context,
        tx: Sender<(u64, PtyEvent)>,
        settings: BackendSettings,
        default_font_size: f32,
    ) -> Option<Self> {
        let cwd = settings.working_directory.clone()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
        let backend = match TerminalBackend::new(id, ctx, tx, settings) {
            Ok(b) => b,
            Err(e) => {
                log::error!("Failed to create terminal backend {id}: {e}");
                return None;
            }
        };
        Some(Self {
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
            agent_mode: AgentMode::new(cwd),
            locked: false,
        })
    }

    /// Open an app in legacy `AppActive` mode — the app fills the entire
    /// pane. Used when there's a separate companion terminal in another tile.
    pub fn open_app(&mut self, app: Box<dyn App>, permissions: AppPermissions, scope: PathBuf) {
        self.active_app = Some(app);
        self.surface_mode = SurfaceMode::AppActive;
        self.focused_surface = SurfaceLayer::App;
        self.app_permissions = permissions;
        self.app_scope = Some(scope);
    }

    /// Open an app in `AppWithCompanion` mode — the app overlays the top
    /// portion of the pane while the existing terminal continues to render
    /// below. The terminal instance is preserved (no new TerminalPane created).
    pub fn open_app_with_companion(
        &mut self,
        app: Box<dyn App>,
        permissions: AppPermissions,
        scope: PathBuf,
    ) {
        self.active_app = Some(app);
        self.surface_mode = SurfaceMode::AppWithCompanion {
            companion_fraction: DEFAULT_COMPANION_FRACTION,
        };
        self.focused_surface = SurfaceLayer::App;
        self.app_permissions = permissions;
        self.app_scope = Some(scope);
        // No linked pane — the companion terminal IS this same pane.
        self.linked_terminal_pane = None;
    }

    /// Close the active app and return to full terminal mode.
    /// Returns the linked terminal pane ID if there was one (caller should close it).
    /// In `AppWithCompanion` mode this returns `None` and the existing
    /// terminal simply expands back to full pane — no tile destruction.
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
        if self.surface_mode.has_app() {
            self.focused_surface = match self.focused_surface {
                SurfaceLayer::App => SurfaceLayer::Terminal,
                SurfaceLayer::Terminal => SurfaceLayer::App,
            };
        }
    }
}
