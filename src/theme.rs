use egui::{Color32, FontId};
use egui_term::{ColorPalette, FontSettings, TerminalFont, TerminalTheme};
use std::collections::HashMap;
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
    pub const TERMINAL_BG: Color32 = Color32::from_rgb(0x29, 0x2a, 0x44);
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

pub fn rebecca() -> TerminalTheme {
    TerminalTheme::new(Box::new(ColorPalette {
        foreground: "#e8e6ed".into(),
        background: "#292a44".into(),
        // Ghostty built-in Rebecca theme
        black: "#12131e".into(),
        red: "#dd7755".into(),
        green: "#04dbb5".into(),
        yellow: "#f2e7b7".into(),
        blue: "#7aa5ff".into(),
        magenta: "#bf9cf9".into(),
        cyan: "#56d3c2".into(),
        white: "#e4e3e9".into(),
        bright_black: "#666699".into(),
        bright_red: "#ff92cd".into(),
        bright_green: "#01eac0".into(),
        bright_yellow: "#fffca8".into(),
        bright_blue: "#69c0fa".into(),
        bright_magenta: "#c17ff8".into(),
        bright_cyan: "#8bfde1".into(),
        bright_white: "#f4f2f9".into(),
        bright_foreground: Some("#f4f2f9".into()),
        // Match named dim colors to normal colors; egui_term already applies
        // its own dimming multiplier in view.rs.
        dim_foreground: "#e8e6ed".into(),
        dim_black: "#12131e".into(),
        dim_red: "#dd7755".into(),
        dim_green: "#04dbb5".into(),
        dim_yellow: "#f2e7b7".into(),
        dim_blue: "#7aa5ff".into(),
        dim_magenta: "#bf9cf9".into(),
        dim_cyan: "#56d3c2".into(),
        dim_white: "#e4e3e9".into(),
    }))
}

pub fn terminal_font() -> TerminalFont {
    TerminalFont::new(FontSettings {
        font_type: FontId::proportional(FONT_SIZE),
    })
}

pub fn terminal_dynamic_colors() -> HashMap<usize, [u8; 3]> {
    HashMap::from([
        (256, [0xe8, 0xe6, 0xed]),
        (257, [0x29, 0x2a, 0x44]),
    ])
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
