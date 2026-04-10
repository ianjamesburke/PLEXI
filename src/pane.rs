use crate::app_trait::{App, SurfaceMode};
use egui_term::{BackendSettings, PtyEvent, TerminalBackend};
use std::sync::mpsc::Sender;

pub struct TerminalPane {
    pub backend: TerminalBackend,
    pub exited: bool,
    pub name: Option<String>,
    pub font_size: f32,
    /// Active app surface overlaid on this terminal, if any.
    pub active_app: Option<Box<dyn App>>,
    pub surface_mode: SurfaceMode,
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
            backend,
            exited: false,
            name: None,
            font_size: default_font_size,
            active_app: None,
            surface_mode: SurfaceMode::FullTerminal,
        })
    }

    /// Open an app in this terminal pane, switching to AppWithCommandBar mode.
    pub fn open_app(&mut self, app: Box<dyn App>) {
        self.active_app = Some(app);
        self.surface_mode = SurfaceMode::AppWithCommandBar;
    }

    /// Close the active app and return to full terminal mode.
    pub fn close_app(&mut self) {
        self.active_app = None;
        self.surface_mode = SurfaceMode::FullTerminal;
    }

    /// Toggle between AppWithCommandBar and AppWithTerminalSplit.
    /// No-op if no app is active.
    pub fn toggle_terminal_split(&mut self) {
        self.surface_mode = match self.surface_mode {
            SurfaceMode::AppWithCommandBar => SurfaceMode::AppWithTerminalSplit,
            SurfaceMode::AppWithTerminalSplit => SurfaceMode::AppWithCommandBar,
            SurfaceMode::FullTerminal => SurfaceMode::FullTerminal,
        };
    }
}
