use egui_term::{BackendSettings, PtyEvent, TerminalBackend};
use std::sync::mpsc::Sender;

pub struct TerminalPane {
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
            backend,
            exited: false,
            name: None,
            font_size: default_font_size,
        })
    }
}
