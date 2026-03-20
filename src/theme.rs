use egui::{Color32, FontId};
use egui_term::{ColorPalette, FontSettings, TerminalFont, TerminalTheme};
use std::sync::Arc;

pub const FONT_SIZE: f32 = 14.0;
const FONT_NAME: &str = "JetBrainsMono Nerd Font";
const FALLBACK_FONT_NAME: &str = "DejaVu Sans";

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

pub fn font_definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        FONT_NAME.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../fonts/JetBrainsMonoNerdFont-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        FALLBACK_FONT_NAME.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../fonts/DejaVuSans.ttf"
        ))),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, FONT_NAME.to_owned());
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(1, FALLBACK_FONT_NAME.to_owned());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, FONT_NAME.to_owned());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(1, FALLBACK_FONT_NAME.to_owned());
    fonts
}

pub fn setup_fonts(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_data_loads() {
        let defs = font_definitions();
        assert!(
            defs.font_data.contains_key(FONT_NAME),
            "JetBrainsMono Nerd Font data missing"
        );
        assert!(
            defs.font_data.contains_key(FALLBACK_FONT_NAME),
            "DejaVu Sans font data missing"
        );
    }

    #[test]
    fn font_families_have_fallback_chain() {
        let defs = font_definitions();

        let proportional = defs.families.get(&egui::FontFamily::Proportional).unwrap();
        assert_eq!(proportional[0], FONT_NAME);
        assert_eq!(proportional[1], FALLBACK_FONT_NAME);

        let monospace = defs.families.get(&egui::FontFamily::Monospace).unwrap();
        assert_eq!(monospace[0], FONT_NAME);
        assert_eq!(monospace[1], FALLBACK_FONT_NAME);
    }

    #[test]
    fn font_data_is_valid_ttf() {
        let defs = font_definitions();
        for (name, data) in &defs.font_data {
            // TrueType fonts start with 0x00010000 or 'true' (0x74727565)
            let bytes = &data.font;
            assert!(
                bytes.len() > 4,
                "Font {name} is too small to be a valid TTF"
            );
            let magic = &bytes[0..4];
            let is_ttf = magic == [0x00, 0x01, 0x00, 0x00] || magic == b"true";
            assert!(is_ttf, "Font {name} has invalid TTF magic bytes: {magic:?}");
        }
    }
}
