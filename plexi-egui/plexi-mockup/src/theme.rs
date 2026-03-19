use egui::Color32;
use std::sync::Arc;

const FONT_NAME: &str = "JetBrainsMono Nerd Font";

pub struct Colors;

impl Colors {
    // Background layers
    pub const BG_DARKEST: Color32 = Color32::from_rgb(0x11, 0x11, 0x1b);
    pub const BG_SIDEBAR: Color32 = Color32::from_rgb(0x18, 0x18, 0x25);
    pub const BG_MAIN: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x2e);
    pub const BG_TOOLBAR: Color32 = Color32::from_rgb(0x18, 0x18, 0x25);
    pub const BG_HOVER: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x3c);
    pub const BG_ACTIVE: Color32 = Color32::from_rgb(0x31, 0x31, 0x44);

    // Text
    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xcd, 0xd6, 0xf4);
    pub const TEXT_DIM: Color32 = Color32::from_rgb(0x6c, 0x70, 0x86);
    pub const TEXT_SECTION: Color32 = Color32::from_rgb(0x58, 0x5b, 0x70);

    // Accent
    pub const ACCENT: Color32 = Color32::from_rgb(0x89, 0xb4, 0xfa);
    pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x45, 0x5a, 0x7d);

    // Minimap
    pub const MINIMAP_PANE: Color32 = Color32::from_rgb(0x31, 0x31, 0x44);
    pub const MINIMAP_BORDER: Color32 = Color32::from_rgb(0x45, 0x47, 0x5a);

    // Borders
    pub const BORDER: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x3c);
}

pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        FONT_NAME.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../fonts/JetBrainsMonoNerdFont-Regular.ttf"
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

pub fn setup_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals.dark_mode = true;
    style.visuals.panel_fill = Colors::BG_MAIN;
    style.visuals.window_fill = Colors::BG_SIDEBAR;
    style.visuals.override_text_color = Some(Colors::TEXT_PRIMARY);
    style.visuals.widgets.noninteractive.bg_fill = Colors::BG_SIDEBAR;
    style.visuals.widgets.inactive.bg_fill = Colors::BG_SIDEBAR;
    style.visuals.widgets.hovered.bg_fill = Colors::BG_HOVER;
    style.visuals.widgets.active.bg_fill = Colors::BG_ACTIVE;
    style.spacing.item_spacing = egui::vec2(8.0, 4.0);
    ctx.set_style(style);
}
