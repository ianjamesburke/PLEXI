use egui::FontId;
use egui_term::{ColorPalette, FontSettings, TerminalFont, TerminalTheme};
use std::sync::Arc;

pub const FONT_SIZE: f32 = 14.0;
const FONT_NAME: &str = "JetBrainsMono Nerd Font";

pub fn catppuccin_mocha() -> TerminalTheme {
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

pub fn terminal_font() -> TerminalFont {
    TerminalFont::new(FontSettings {
        font_type: FontId::proportional(FONT_SIZE),
    })
}

pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        FONT_NAME.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../fonts/JetBrainsMonoNerdFont-Regular.ttf"
        ))),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, FONT_NAME.to_owned());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, FONT_NAME.to_owned());
    ctx.set_fonts(fonts);
}
