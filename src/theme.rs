use crate::config::ThemeConfig;
use egui::{Color32, FontId};
use egui_term::{ColorPalette, FontSettings, TerminalFont, TerminalTheme};
use std::collections::HashMap;
use std::sync::Arc;

pub const FONT_SIZE: f32 = 14.0;
const FONT_NAME: &str = "JetBrainsMono Nerd Font";
const FALLBACK_FONT_NAME: &str = "DejaVu Sans";

fn parse_hex_or(s: &Option<String>, default: Color32) -> Color32 {
    let s = match s {
        Some(s) => s.trim_start_matches('#'),
        None => return default,
    };
    if s.len() != 6 {
        return default;
    }
    match (
        u8::from_str_radix(&s[0..2], 16),
        u8::from_str_radix(&s[2..4], 16),
        u8::from_str_radix(&s[4..6], 16),
    ) {
        (Ok(r), Ok(g), Ok(b)) => Color32::from_rgb(r, g, b),
        _ => default,
    }
}

#[derive(Clone, Copy)]
pub struct Colors {
    // Background layers
    pub bg_darkest: Color32,
    pub bg_sidebar: Color32,
    pub bg_toolbar: Color32,
    pub terminal_bg: Color32,
    pub bg_hover: Color32,
    pub bg_active: Color32,
    // Text
    pub text_primary: Color32,
    pub text_dim: Color32,
    pub text_section: Color32,
    // Accent / borders
    pub accent: Color32,
    pub border: Color32,
    // Terminal fg/bg as bytes for dynamic colors
    pub terminal_fg_bytes: [u8; 3],
    pub terminal_bg_bytes: [u8; 3],
}

impl Colors {
    pub fn from_config(cfg: &ThemeConfig) -> Self {
        Self {
            bg_darkest:   parse_hex_or(&cfg.bg_darkest,   Color32::from_rgb(0x11, 0x11, 0x1b)),
            bg_sidebar:   parse_hex_or(&cfg.bg_sidebar,   Color32::from_rgb(0x18, 0x18, 0x25)),
            bg_toolbar:   parse_hex_or(&cfg.bg_toolbar,   Color32::from_rgb(0x18, 0x18, 0x25)),
            terminal_bg:  parse_hex_or(&cfg.terminal_bg,  Color32::from_rgb(0x29, 0x2a, 0x44)),
            bg_hover:     parse_hex_or(&cfg.bg_hover,     Color32::from_rgb(0x2a, 0x2a, 0x3c)),
            bg_active:    parse_hex_or(&cfg.bg_active,    Color32::from_rgb(0x31, 0x31, 0x44)),
            text_primary: parse_hex_or(&cfg.text_primary, Color32::from_rgb(0xcd, 0xd6, 0xf4)),
            text_dim:     parse_hex_or(&cfg.text_dim,     Color32::from_rgb(0x6c, 0x70, 0x86)),
            text_section: parse_hex_or(&cfg.text_section, Color32::from_rgb(0x58, 0x5b, 0x70)),
            accent:       parse_hex_or(&cfg.accent,       Color32::from_rgb(0x89, 0xb4, 0xfa)),
            border:       parse_hex_or(&cfg.border,       Color32::from_rgb(0x2a, 0x2a, 0x3c)),
            terminal_fg_bytes: hex_to_bytes(cfg.foreground.as_deref(), [0xe8, 0xe6, 0xed]),
            terminal_bg_bytes: hex_to_bytes(cfg.background.as_deref(), [0x29, 0x2a, 0x44]),
        }
    }
}

fn hex_to_bytes(s: Option<&str>, default: [u8; 3]) -> [u8; 3] {
    let s = match s {
        Some(s) => s.trim_start_matches('#'),
        None => return default,
    };
    if s.len() != 6 {
        return default;
    }
    match (
        u8::from_str_radix(&s[0..2], 16),
        u8::from_str_radix(&s[2..4], 16),
        u8::from_str_radix(&s[4..6], 16),
    ) {
        (Ok(r), Ok(g), Ok(b)) => [r, g, b],
        _ => default,
    }
}

pub fn setup_style(ctx: &egui::Context, colors: &Colors) {
    let mut style = (*ctx.style()).clone();
    style.visuals.dark_mode = true;
    style.visuals.panel_fill = colors.bg_darkest;
    style.visuals.window_fill = colors.bg_sidebar;
    style.visuals.override_text_color = Some(colors.text_primary);
    style.visuals.widgets.noninteractive.bg_fill = colors.bg_sidebar;
    style.visuals.widgets.inactive.bg_fill = colors.bg_sidebar;
    style.visuals.widgets.hovered.bg_fill = colors.bg_hover;
    style.visuals.widgets.active.bg_fill = colors.bg_active;
    style.spacing.item_spacing = egui::vec2(8.0, 4.0);
    ctx.set_style(style);
}

pub fn terminal_theme(cfg: &ThemeConfig) -> TerminalTheme {
    let fg = cfg.foreground.as_deref().unwrap_or("#e8e6ed");
    let bg = cfg.background.as_deref().unwrap_or("#292a44");
    TerminalTheme::new(Box::new(ColorPalette {
        foreground: fg.into(),
        background: bg.into(),
        black:   cfg.black.as_deref().unwrap_or("#12131e").into(),
        red:     cfg.red.as_deref().unwrap_or("#dd7755").into(),
        green:   cfg.green.as_deref().unwrap_or("#04dbb5").into(),
        yellow:  cfg.yellow.as_deref().unwrap_or("#f2e7b7").into(),
        blue:    cfg.blue.as_deref().unwrap_or("#7aa5ff").into(),
        magenta: cfg.magenta.as_deref().unwrap_or("#bf9cf9").into(),
        cyan:    cfg.cyan.as_deref().unwrap_or("#56d3c2").into(),
        white:   cfg.white.as_deref().unwrap_or("#e4e3e9").into(),
        bright_black:   cfg.bright_black.as_deref().unwrap_or("#666699").into(),
        bright_red:     cfg.bright_red.as_deref().unwrap_or("#ff92cd").into(),
        bright_green:   cfg.bright_green.as_deref().unwrap_or("#01eac0").into(),
        bright_yellow:  cfg.bright_yellow.as_deref().unwrap_or("#fffca8").into(),
        bright_blue:    cfg.bright_blue.as_deref().unwrap_or("#69c0fa").into(),
        bright_magenta: cfg.bright_magenta.as_deref().unwrap_or("#c17ff8").into(),
        bright_cyan:    cfg.bright_cyan.as_deref().unwrap_or("#8bfde1").into(),
        bright_white:   cfg.bright_white.as_deref().unwrap_or("#f4f2f9").into(),
        bright_foreground: Some(cfg.bright_foreground.as_deref().unwrap_or("#f4f2f9").into()),
        dim_foreground: fg.into(),
        dim_black:   cfg.black.as_deref().unwrap_or("#12131e").into(),
        dim_red:     cfg.red.as_deref().unwrap_or("#dd7755").into(),
        dim_green:   cfg.green.as_deref().unwrap_or("#04dbb5").into(),
        dim_yellow:  cfg.yellow.as_deref().unwrap_or("#f2e7b7").into(),
        dim_blue:    cfg.blue.as_deref().unwrap_or("#7aa5ff").into(),
        dim_magenta: cfg.magenta.as_deref().unwrap_or("#bf9cf9").into(),
        dim_cyan:    cfg.cyan.as_deref().unwrap_or("#56d3c2").into(),
        dim_white:   cfg.white.as_deref().unwrap_or("#e4e3e9").into(),
    }))
}

pub fn terminal_dynamic_colors(colors: &Colors) -> HashMap<usize, [u8; 3]> {
    HashMap::from([
        (256, colors.terminal_fg_bytes),
        (257, colors.terminal_bg_bytes),
    ])
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
            "../fonts/JetBrainsMonoNerdFont-Light.ttf"
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
