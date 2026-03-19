#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use egui::{FontId, Vec2};
use egui_term::{
    ColorPalette, FontSettings, PtyEvent, TerminalBackend, TerminalFont, TerminalTheme,
    TerminalView,
};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

const FONT_SIZE: f32 = 14.0;

fn catppuccin_mocha() -> TerminalTheme {
    TerminalTheme::new(Box::new(ColorPalette {
        foreground: "#cdd6f4".into(),
        background: "#1e1e2e".into(),
        black: "#45475a".into(),
        red: "#f38ba8".into(),
        green: "#a6e3a1".into(),
        yellow: "#f9e2af".into(),
        blue: "#89b4fa".into(),
        magenta: "#f5c2e7".into(),
        cyan: "#94e2d5".into(),
        white: "#bac2de".into(),
        bright_black: "#585b70".into(),
        bright_red: "#f38ba8".into(),
        bright_green: "#a6e3a1".into(),
        bright_yellow: "#f9e2af".into(),
        bright_blue: "#89b4fa".into(),
        bright_magenta: "#f5c2e7".into(),
        bright_cyan: "#94e2d5".into(),
        bright_white: "#a6adc8".into(),
        ..Default::default()
    }))
}

struct App {
    terminal_backend: TerminalBackend,
    pty_proxy_receiver: Receiver<(u64, PtyEvent)>,
    theme: TerminalTheme,
    font: TerminalFont,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Load JetBrains Mono Nerd Font
        let font_name = "JetBrainsMono Nerd Font";
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            font_name.to_owned(),
            Arc::new(egui::FontData::from_static(include_bytes!(
                "../fonts/JetBrainsMonoNerdFont-Regular.ttf"
            ))),
        );
        // Insert as first in Proportional so egui_term picks it up
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, font_name.to_owned());
        // Also add to Monospace as fallback
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .insert(0, font_name.to_owned());
        cc.egui_ctx.set_fonts(fonts);

        let system_shell =
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());

        let (pty_proxy_sender, pty_proxy_receiver) = std::sync::mpsc::channel();
        let terminal_backend = TerminalBackend::new(
            0,
            cc.egui_ctx.clone(),
            pty_proxy_sender,
            egui_term::BackendSettings {
                shell: system_shell,
                args: vec!["-l".to_string()],
                ..Default::default()
            },
        )
        .expect("failed to create terminal backend");

        Self {
            terminal_backend,
            pty_proxy_receiver,
            theme: catppuccin_mocha(),
            font: TerminalFont::new(FontSettings {
                font_type: FontId::proportional(FONT_SIZE),
            }),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Ok((_, PtyEvent::Exit)) = self.pty_proxy_receiver.try_recv() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let terminal = TerminalView::new(ui, &mut self.terminal_backend)
                    .set_focus(true)
                    .set_theme(self.theme.clone())
                    .set_font(self.font.clone())
                    .set_size(Vec2::new(ui.available_width(), ui.available_height()));

                ui.add(terminal);
            });
    }
}

fn main() -> eframe::Result {
    env_logger::init();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([400.0, 300.0]),
        ..Default::default()
    };

    eframe::run_native(
        "egui-terminal-poc",
        native_options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
