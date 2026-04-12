use crate::agent_mode::AgentMode;
use crate::app_permissions::AppPermissions;
use crate::app_trait::{App, SurfaceLayer, SurfaceMode};
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
