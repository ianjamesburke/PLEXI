use egui::{Color32, FontId};
use egui_term::{ColorPalette, FontSettings, TerminalFont, TerminalTheme};
use std::sync::Arc;

pub const FONT_SIZE: f32 = 14.0;
const FONT_NAME: &str = "JetBrainsMono Nerd Font";

pub struct Colors;

impl Colors {
    // Background layers
    pub const BG_DARKEST: Color32 = Color32::from_rgb(0x11, 0x11, 0x1b);
    pub const BG_SIDEBAR: Color32 = Color32::from_rgb(0x18, 0x18, 0x25);
    pub const BG_TOOLBAR: Color32 = Color32::from_rgb(0x18, 0x18, 0x25);
    pub const BG_HOVER: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x3c);
    pub const BG_ACTIVE: Color32 = Color32::from_rgb(0x31, 0x31, 0x44);
    // Text
    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xcd, 0xd6, 0xf4);
    pub const TEXT_DIM: Color32 = Color32::from_rgb(0x6c, 0x70, 0x86);
    pub const TEXT_SECTION: Color32 = Color32::from_rgb(0x58, 0x5b, 0x70);

    // Accent
    pub const ACCENT: Color32 = Color32::from_rgb(0x89, 0xb4, 0xfa);
    // Borders
    pub const BORDER: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x3c);
}

pub fn setup_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals.dark_mode = true;
    style.visuals.panel_fill = Colors::BG_DARKEST;
    style.visuals.window_fill = Colors::BG_SIDEBAR;
    style.visuals.override_text_color = Some(Colors::TEXT_PRIMARY);
    style.visuals.widgets.noninteractive.bg_fill = Colors::BG_SIDEBAR;
    style.visuals.widgets.inactive.bg_fill = Colors::BG_SIDEBAR;
    style.visuals.widgets.hovered.bg_fill = Colors::BG_HOVER;
    style.visuals.widgets.active.bg_fill = Colors::BG_ACTIVE;
    style.spacing.item_spacing = egui::vec2(8.0, 4.0);
    ctx.set_style(style);
}

pub fn catppuccin_mocha() -> TerminalTheme {
    TerminalTheme::new(Box::new(ColorPalette {
        foreground: "#cdd6f4".into(),
        background: "#1e1e2e".into(),
        // Normal colors
        black: "#45475a".into(),
        red: "#f38ba8".into(),
        green: "#a6e3a1".into(),
        yellow: "#f9e2af".into(),
        blue: "#89b4fa".into(),
        magenta: "#f5c2e7".into(),
        cyan: "#94e2d5".into(),
        white: "#bac2de".into(),
        // Bright colors
        bright_black: "#585b70".into(),
        bright_red: "#f38ba8".into(),
        bright_green: "#a6e3a1".into(),
        bright_yellow: "#f9e2af".into(),
        bright_blue: "#89b4fa".into(),
        bright_magenta: "#f5c2e7".into(),
        bright_cyan: "#94e2d5".into(),
        bright_white: "#a6adc8".into(),
        bright_foreground: Some("#cdd6f4".into()),
        // Dim colors — set to normal colors since view.rs already applies
        // linear_multiply(0.7) when the DIM flag is set. Using pre-dimmed
        // values here would cause double-dimming.
        dim_foreground: "#cdd6f4".into(),
        dim_black: "#45475a".into(),
        dim_red: "#f38ba8".into(),
        dim_green: "#a6e3a1".into(),
        dim_yellow: "#f9e2af".into(),
        dim_blue: "#89b4fa".into(),
        dim_magenta: "#f5c2e7".into(),
        dim_cyan: "#94e2d5".into(),
        dim_white: "#bac2de".into(),
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
