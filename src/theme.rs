use crate::config::ThemeConfig;
use egui::{Color32, FontId};
use egui_term::{ColorPalette, FontSettings, TerminalFont, TerminalTheme};
use std::collections::HashMap;
use std::sync::Arc;

pub const FONT_SIZE: f32 = 14.0;
const FONT_NAME: &str = "JetBrainsMono Nerd Font";
const FALLBACK_FONT_NAME: &str = "DejaVu Sans";
const UNICODE_FALLBACK_FONT_NAME: &str = "Noto Sans";

fn parse_hex_or(s: &Option<String>, default: Color32) -> Color32 {
    let [r, g, b] = hex_to_bytes(s.as_deref(), [default.r(), default.g(), default.b()]);
    Color32::from_rgb(r, g, b)
}

#[derive(Clone, Copy, PartialEq)]
pub struct Colors {
    // Background layers
    pub bg_darkest: Color32,
    pub bg_sidebar: Color32,
    pub bg_toolbar: Color32,
    pub terminal_bg: Color32,
    pub bg_hover: Color32,
    pub bg_sidebar_hover: Color32,
    pub bg_active: Color32,
    // Text
    pub text_primary: Color32,
    pub text_dim: Color32,
    pub text_section: Color32,
    // Accent / borders
    pub accent: Color32,
    pub border: Color32,
    // Semantic state colors
    pub danger: Color32,
    pub success: Color32,
    pub warning: Color32,
    // Terminal fg/bg as bytes for dynamic colors
    pub terminal_fg_bytes: [u8; 3],
    pub terminal_bg_bytes: [u8; 3],
}

impl Colors {
    pub fn from_config(cfg: &ThemeConfig) -> Self {
        Self {
            bg_darkest: parse_hex_or(&cfg.bg_darkest, Color32::from_rgb(0x11, 0x11, 0x1b)),
            bg_sidebar: parse_hex_or(&cfg.bg_sidebar, Color32::from_rgb(0x18, 0x18, 0x25)),
            bg_toolbar: parse_hex_or(&cfg.bg_toolbar, Color32::from_rgb(0x18, 0x18, 0x25)),
            terminal_bg: parse_hex_or(&cfg.terminal_bg, Color32::from_rgb(0x29, 0x2a, 0x44)),
            bg_hover: parse_hex_or(&cfg.bg_hover, Color32::from_rgb(0x2a, 0x2a, 0x3c)),
            bg_sidebar_hover: parse_hex_or(&cfg.bg_sidebar_hover, Color32::from_rgb(0x2e, 0x2e, 0x48)),
            bg_active: parse_hex_or(&cfg.bg_active, Color32::from_rgb(0x31, 0x31, 0x44)),
            text_primary: parse_hex_or(&cfg.text_primary, Color32::from_rgb(0xcd, 0xd6, 0xf4)),
            text_dim: parse_hex_or(&cfg.text_dim, Color32::from_rgb(0x6c, 0x70, 0x86)),
            text_section: parse_hex_or(&cfg.text_section, Color32::from_rgb(0x58, 0x5b, 0x70)),
            accent: parse_hex_or(&cfg.accent, Color32::from_rgb(0x89, 0xb4, 0xfa)),
            border: parse_hex_or(&cfg.border, Color32::from_rgb(0x2a, 0x2a, 0x3c)),
            // Derive from the theme's ANSI red; fallback matches the Dracula red used in event_log.rs.
            danger: parse_hex_or(&cfg.red, Color32::from_rgb(0xff, 0x55, 0x55)),
            success: parse_hex_or(&cfg.green, Color32::from_rgb(0xa6, 0xe3, 0xa1)),
            warning: parse_hex_or(&cfg.yellow, Color32::from_rgb(0xf9, 0xe2, 0xaf)),
            terminal_fg_bytes: hex_to_bytes(cfg.foreground.as_deref(), [0xe8, 0xe6, 0xed]),
            terminal_bg_bytes: hex_to_bytes(cfg.background.as_deref(), [0x29, 0x2a, 0x44]),
        }
    }

    /// Serialize the semantic color roles to a `role -> #rrggbb` map for the
    /// SDK `Init` payload. Apps read these via `ctx.theme.<role>` so app-drawn
    /// chrome tracks the host theme (light/dark + user `[theme]` overrides).
    /// Both the SDK-semantic names (bg/surface/muted/...) and ANSI aliases
    /// (red/green/yellow) are emitted so apps can pull whichever they need.
    pub fn to_theme_map(&self) -> std::collections::HashMap<String, String> {
        fn hex(c: Color32) -> String {
            format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
        }
        // `bg` is the color the host paints behind the app pane (terminal_bg),
        // so an app that clears to theme.bg matches its container in any theme.
        [
            ("bg", self.terminal_bg),
            ("bg_darkest", self.bg_darkest),
            ("surface", self.bg_active),
            ("highlight", self.bg_hover),
            ("border", self.border),
            ("fg", self.text_primary),
            ("muted", self.text_dim),
            ("text_section", self.text_section),
            ("accent", self.accent),
            ("danger", self.danger),
            ("red", self.danger),
            ("success", self.success),
            ("green", self.success),
            ("warning", self.warning),
            ("yellow", self.warning),
        ]
        .into_iter()
        .map(|(k, c)| (k.to_string(), hex(c)))
        .collect()
    }
}

/// Resolve the active `Colors` from a loaded config: merges the user's
/// `[theme]` overrides over the named preset (or pure user config if no
/// preset). Shared by the GUI and the headless `app render` path.
pub fn colors_from_config(config: &crate::config::PlexiConfig) -> Colors {
    let user_theme = config.theme.clone().unwrap_or_default();
    let cfg = match &config.theme_preset {
        Some(preset_name) => match preset_colors(preset_name) {
            Some(preset) => apply_preset(&preset, &user_theme),
            None => user_theme,
        },
        None => user_theme,
    };
    Colors::from_config(&cfg)
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

fn canonical_preset_name(name: &str) -> Option<&'static str> {
    let normalized = name.trim().to_lowercase().replace([' ', '_'], "-");
    match normalized.as_str() {
        "catppuccin" | "catppuccin-mocha" | "mocha" => Some("catppuccin-mocha"),
        "catppuccin-latte" | "latte" => Some("catppuccin-latte"),
        "dracula" => Some("dracula"),
        "tokyo-night" | "tokyonight" | "tokyo" => Some("tokyo-night"),
        "gruvbox" | "gruvbox-dark" => Some("gruvbox-dark"),
        "nord" => Some("nord"),
        "solarized" | "solarized-dark" => Some("solarized-dark"),
        "solarized-light" => Some("solarized-light"),
        _ => None,
    }
}

/// Returns true for presets with a light background — used to flip egui's dark_mode flag.
pub fn is_light_preset(name: &str) -> bool {
    matches!(canonical_preset_name(name), Some("catppuccin-latte") | Some("solarized-light"))
}

/// Returns true if the preset is any catppuccin variant (latte or mocha).
pub fn is_catppuccin_preset(name: &str) -> bool {
    matches!(canonical_preset_name(name), Some("catppuccin-mocha" | "catppuccin-latte"))
}

/// Returns the list of available preset names.
#[allow(dead_code)] // theme-picker palette is future; preset list stays ready
pub fn preset_names() -> &'static [&'static str] {
    &[
        "catppuccin-mocha",
        "catppuccin-latte",
        "dracula",
        "tokyo-night",
        "gruvbox-dark",
        "nord",
        "solarized-dark",
        "solarized-light",
    ]
}

/// Returns a fully-populated ThemeConfig for the named preset, or None if unknown.
pub fn preset_colors(name: &str) -> Option<ThemeConfig> {
    let s = |v: &str| Some(v.to_string());
    let canonical = canonical_preset_name(name)?;
    match canonical {
        "catppuccin-mocha" => Some(ThemeConfig {
            bg_darkest: s("#11111b"),
            bg_sidebar: s("#181825"),
            bg_toolbar: s("#181825"),
            terminal_bg: s("#292a44"),
            bg_hover: s("#2a2a3c"),
            bg_sidebar_hover: s("#2e2e48"),
            bg_active: s("#313144"),
            text_primary: s("#cdd6f4"),
            text_dim: s("#6c7086"),
            text_section: s("#585b70"),
            accent: s("#89b4fa"),
            border: s("#2a2a3c"),
            foreground: s("#e8e6ed"),
            background: s("#292a44"),
            black: s("#12131e"),
            red: s("#dd7755"),
            green: s("#04dbb5"),
            yellow: s("#f2e7b7"),
            blue: s("#7aa5ff"),
            magenta: s("#bf9cf9"),
            cyan: s("#56d3c2"),
            white: s("#e4e3e9"),
            bright_black: s("#666699"),
            bright_red: s("#ff92cd"),
            bright_green: s("#01eac0"),
            bright_yellow: s("#fffca8"),
            bright_blue: s("#69c0fa"),
            bright_magenta: s("#c17ff8"),
            bright_cyan: s("#8bfde1"),
            bright_white: s("#f4f2f9"),
            bright_foreground: s("#f4f2f9"),
        }),
        "dracula" => Some(ThemeConfig {
            bg_darkest: s("#282a36"),
            bg_sidebar: s("#21222c"),
            bg_toolbar: s("#21222c"),
            terminal_bg: s("#282a36"),
            bg_hover: s("#343746"),
            bg_sidebar_hover: s("#393c50"),
            bg_active: s("#3e4154"),
            text_primary: s("#f8f8f2"),
            text_dim: s("#6272a4"),
            text_section: s("#545876"),
            accent: s("#bd93f9"),
            border: s("#44475a"),
            foreground: s("#f8f8f2"),
            background: s("#282a36"),
            black: s("#21222c"),
            red: s("#ff5555"),
            green: s("#50fa7b"),
            yellow: s("#f1fa8c"),
            blue: s("#6272a4"),
            magenta: s("#ff79c6"),
            cyan: s("#8be9fd"),
            white: s("#f8f8f2"),
            bright_black: s("#6272a4"),
            bright_red: s("#ff6e6e"),
            bright_green: s("#69ff94"),
            bright_yellow: s("#ffffa5"),
            bright_blue: s("#d6acff"),
            bright_magenta: s("#ff92df"),
            bright_cyan: s("#a4ffff"),
            bright_white: s("#ffffff"),
            bright_foreground: s("#ffffff"),
        }),
        "tokyo-night" => Some(ThemeConfig {
            bg_darkest: s("#1a1b26"),
            bg_sidebar: s("#16161e"),
            bg_toolbar: s("#16161e"),
            terminal_bg: s("#1a1b26"),
            bg_hover: s("#232433"),
            bg_sidebar_hover: s("#262a3d"),
            bg_active: s("#292e42"),
            text_primary: s("#a9b1d6"),
            text_dim: s("#565f89"),
            text_section: s("#444b6a"),
            accent: s("#7aa2f7"),
            border: s("#292e42"),
            foreground: s("#a9b1d6"),
            background: s("#1a1b26"),
            black: s("#16161e"),
            red: s("#f7768e"),
            green: s("#9ece6a"),
            yellow: s("#e0af68"),
            blue: s("#7aa2f7"),
            magenta: s("#bb9af7"),
            cyan: s("#7dcfff"),
            white: s("#a9b1d6"),
            bright_black: s("#565f89"),
            bright_red: s("#ff899d"),
            bright_green: s("#b5e87a"),
            bright_yellow: s("#e8c87e"),
            bright_blue: s("#8db8ff"),
            bright_magenta: s("#c8adff"),
            bright_cyan: s("#90d8ff"),
            bright_white: s("#c0caf5"),
            bright_foreground: s("#c0caf5"),
        }),
        "gruvbox-dark" => Some(ThemeConfig {
            bg_darkest: s("#1d2021"),
            bg_sidebar: s("#282828"),
            bg_toolbar: s("#282828"),
            terminal_bg: s("#282828"),
            bg_hover: s("#3c3836"),
            bg_sidebar_hover: s("#464240"),
            bg_active: s("#504945"),
            text_primary: s("#ebdbb2"),
            text_dim: s("#928374"),
            text_section: s("#7c6f64"),
            accent: s("#fe8019"),
            border: s("#3c3836"),
            foreground: s("#ebdbb2"),
            background: s("#282828"),
            black: s("#1d2021"),
            red: s("#cc241d"),
            green: s("#98971a"),
            yellow: s("#d79921"),
            blue: s("#458588"),
            magenta: s("#b16286"),
            cyan: s("#689d6a"),
            white: s("#ebdbb2"),
            bright_black: s("#928374"),
            bright_red: s("#fb4934"),
            bright_green: s("#b8bb26"),
            bright_yellow: s("#fabd2f"),
            bright_blue: s("#83a598"),
            bright_magenta: s("#d3869b"),
            bright_cyan: s("#8ec07c"),
            bright_white: s("#fbf1c7"),
            bright_foreground: s("#fbf1c7"),
        }),
        "nord" => Some(ThemeConfig {
            bg_darkest: s("#2e3440"),
            bg_sidebar: s("#3b4252"),
            bg_toolbar: s("#3b4252"),
            terminal_bg: s("#2e3440"),
            bg_hover: s("#434c5e"),
            bg_sidebar_hover: s("#495264"),
            bg_active: s("#4c566a"),
            text_primary: s("#eceff4"),
            text_dim: s("#4c566a"),
            text_section: s("#434c5e"),
            accent: s("#88c0d0"),
            border: s("#3b4252"),
            foreground: s("#eceff4"),
            background: s("#2e3440"),
            black: s("#3b4252"),
            red: s("#bf616a"),
            green: s("#a3be8c"),
            yellow: s("#ebcb8b"),
            blue: s("#81a1c1"),
            magenta: s("#b48ead"),
            cyan: s("#88c0d0"),
            white: s("#eceff4"),
            bright_black: s("#4c566a"),
            bright_red: s("#d08770"),
            bright_green: s("#a3be8c"),
            bright_yellow: s("#ebcb8b"),
            bright_blue: s("#81a1c1"),
            bright_magenta: s("#b48ead"),
            bright_cyan: s("#8fbcbb"),
            bright_white: s("#e5e9f0"),
            bright_foreground: s("#e5e9f0"),
        }),
        "solarized-dark" => Some(ThemeConfig {
            bg_darkest: s("#002b36"),
            bg_sidebar: s("#073642"),
            bg_toolbar: s("#073642"),
            terminal_bg: s("#002b36"),
            bg_hover: s("#0a4050"),
            bg_sidebar_hover: s("#0f4658"),
            bg_active: s("#124d5e"),
            text_primary: s("#839496"),
            text_dim: s("#586e75"),
            text_section: s("#4d6269"),
            accent: s("#268bd2"),
            border: s("#073642"),
            foreground: s("#839496"),
            background: s("#002b36"),
            black: s("#073642"),
            red: s("#dc322f"),
            green: s("#859900"),
            yellow: s("#b58900"),
            blue: s("#268bd2"),
            magenta: s("#d33682"),
            cyan: s("#2aa198"),
            white: s("#eee8d5"),
            bright_black: s("#586e75"),
            bright_red: s("#cb4b16"),
            bright_green: s("#859900"),
            bright_yellow: s("#b58900"),
            bright_blue: s("#268bd2"),
            bright_magenta: s("#6c71c4"),
            bright_cyan: s("#2aa198"),
            bright_white: s("#fdf6e3"),
            bright_foreground: s("#fdf6e3"),
        }),
        "catppuccin-latte" => Some(ThemeConfig {
            bg_darkest: s("#e6e9ef"),
            bg_sidebar: s("#eff1f5"),
            bg_toolbar: s("#eff1f5"),
            terminal_bg: s("#eff1f5"),
            bg_hover: s("#dce0e8"),
            bg_sidebar_hover: s("#ccd0da"),
            bg_active: s("#bcc0cc"),
            text_primary: s("#4c4f69"),
            text_dim: s("#8c8fa1"),
            text_section: s("#9ca0b0"),
            accent: s("#8839ef"),
            border: s("#ccd0da"),
            foreground: s("#4c4f69"),
            background: s("#eff1f5"),
            black: s("#5c5f77"),
            red: s("#d20f39"),
            green: s("#40a02b"),
            yellow: s("#df8e1d"),
            blue: s("#1e66f5"),
            magenta: s("#ea76cb"),
            cyan: s("#179299"),
            white: s("#acb0be"),
            bright_black: s("#6c6f85"),
            bright_red: s("#d20f39"),
            bright_green: s("#40a02b"),
            bright_yellow: s("#df8e1d"),
            bright_blue: s("#1e66f5"),
            bright_magenta: s("#8839ef"),
            bright_cyan: s("#04a5e5"),
            bright_white: s("#bcc0cc"),
            bright_foreground: s("#4c4f69"),
        }),
        "solarized-light" => Some(ThemeConfig {
            bg_darkest: s("#eee8d5"),
            bg_sidebar: s("#fdf6e3"),
            bg_toolbar: s("#fdf6e3"),
            terminal_bg: s("#fdf6e3"),
            bg_hover: s("#e0dcc8"),
            bg_sidebar_hover: s("#d8d4c0"),
            bg_active: s("#d0ccb8"),
            text_primary: s("#657b83"),
            text_dim: s("#93a1a1"),
            text_section: s("#839496"),
            accent: s("#268bd2"),
            border: s("#ddd6c1"),
            foreground: s("#657b83"),
            background: s("#fdf6e3"),
            black: s("#073642"),
            red: s("#dc322f"),
            green: s("#859900"),
            yellow: s("#b58900"),
            blue: s("#268bd2"),
            magenta: s("#d33682"),
            cyan: s("#2aa198"),
            white: s("#eee8d5"),
            bright_black: s("#586e75"),
            bright_red: s("#cb4b16"),
            bright_green: s("#859900"),
            bright_yellow: s("#b58900"),
            bright_blue: s("#268bd2"),
            bright_magenta: s("#6c71c4"),
            bright_cyan: s("#2aa198"),
            bright_white: s("#fdf6e3"),
            bright_foreground: s("#fdf6e3"),
        }),
        _ => None,
    }
}

/// Merge a preset with user overrides — user values take precedence over the preset.
pub fn apply_preset(preset: &ThemeConfig, user: &ThemeConfig) -> ThemeConfig {
    let m = |u: &Option<String>, p: &Option<String>| u.clone().or_else(|| p.clone());
    ThemeConfig {
        bg_darkest: m(&user.bg_darkest, &preset.bg_darkest),
        bg_sidebar: m(&user.bg_sidebar, &preset.bg_sidebar),
        bg_toolbar: m(&user.bg_toolbar, &preset.bg_toolbar),
        terminal_bg: m(&user.terminal_bg, &preset.terminal_bg),
        bg_hover: m(&user.bg_hover, &preset.bg_hover),
        bg_sidebar_hover: m(&user.bg_sidebar_hover, &preset.bg_sidebar_hover),
        bg_active: m(&user.bg_active, &preset.bg_active),
        text_primary: m(&user.text_primary, &preset.text_primary),
        text_dim: m(&user.text_dim, &preset.text_dim),
        text_section: m(&user.text_section, &preset.text_section),
        accent: m(&user.accent, &preset.accent),
        border: m(&user.border, &preset.border),
        foreground: m(&user.foreground, &preset.foreground),
        background: m(&user.background, &preset.background),
        black: m(&user.black, &preset.black),
        red: m(&user.red, &preset.red),
        green: m(&user.green, &preset.green),
        yellow: m(&user.yellow, &preset.yellow),
        blue: m(&user.blue, &preset.blue),
        magenta: m(&user.magenta, &preset.magenta),
        cyan: m(&user.cyan, &preset.cyan),
        white: m(&user.white, &preset.white),
        bright_black: m(&user.bright_black, &preset.bright_black),
        bright_red: m(&user.bright_red, &preset.bright_red),
        bright_green: m(&user.bright_green, &preset.bright_green),
        bright_yellow: m(&user.bright_yellow, &preset.bright_yellow),
        bright_blue: m(&user.bright_blue, &preset.bright_blue),
        bright_magenta: m(&user.bright_magenta, &preset.bright_magenta),
        bright_cyan: m(&user.bright_cyan, &preset.bright_cyan),
        bright_white: m(&user.bright_white, &preset.bright_white),
        bright_foreground: m(&user.bright_foreground, &preset.bright_foreground),
    }
}

pub fn setup_style(ctx: &egui::Context, colors: &Colors, dark_mode: bool) {
    log::info!("theme: setup_style dark_mode={dark_mode}");
    let mut style = (*ctx.style()).clone();
    style.visuals = if dark_mode { egui::Visuals::dark() } else { egui::Visuals::light() };
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
        black: cfg.black.as_deref().unwrap_or("#12131e").into(),
        red: cfg.red.as_deref().unwrap_or("#dd7755").into(),
        green: cfg.green.as_deref().unwrap_or("#04dbb5").into(),
        yellow: cfg.yellow.as_deref().unwrap_or("#f2e7b7").into(),
        blue: cfg.blue.as_deref().unwrap_or("#7aa5ff").into(),
        magenta: cfg.magenta.as_deref().unwrap_or("#bf9cf9").into(),
        cyan: cfg.cyan.as_deref().unwrap_or("#56d3c2").into(),
        white: cfg.white.as_deref().unwrap_or("#e4e3e9").into(),
        bright_black: cfg.bright_black.as_deref().unwrap_or("#666699").into(),
        bright_red: cfg.bright_red.as_deref().unwrap_or("#ff92cd").into(),
        bright_green: cfg.bright_green.as_deref().unwrap_or("#01eac0").into(),
        bright_yellow: cfg.bright_yellow.as_deref().unwrap_or("#fffca8").into(),
        bright_blue: cfg.bright_blue.as_deref().unwrap_or("#69c0fa").into(),
        bright_magenta: cfg.bright_magenta.as_deref().unwrap_or("#c17ff8").into(),
        bright_cyan: cfg.bright_cyan.as_deref().unwrap_or("#8bfde1").into(),
        bright_white: cfg.bright_white.as_deref().unwrap_or("#f4f2f9").into(),
        bright_foreground: Some(cfg.bright_foreground.as_deref().unwrap_or("#f4f2f9").into()),
        dim_foreground: fg.into(),
        dim_black: cfg.black.as_deref().unwrap_or("#12131e").into(),
        dim_red: cfg.red.as_deref().unwrap_or("#dd7755").into(),
        dim_green: cfg.green.as_deref().unwrap_or("#04dbb5").into(),
        dim_yellow: cfg.yellow.as_deref().unwrap_or("#f2e7b7").into(),
        dim_blue: cfg.blue.as_deref().unwrap_or("#7aa5ff").into(),
        dim_magenta: cfg.magenta.as_deref().unwrap_or("#bf9cf9").into(),
        dim_cyan: cfg.cyan.as_deref().unwrap_or("#56d3c2").into(),
        dim_white: cfg.white.as_deref().unwrap_or("#e4e3e9").into(),
    }))
}

pub fn terminal_dynamic_colors(colors: &Colors) -> HashMap<usize, [u8; 3]> {
    HashMap::from([
        (256, colors.terminal_fg_bytes),
        (257, colors.terminal_bg_bytes),
    ])
}

pub fn terminal_font(size: f32) -> TerminalFont {
    // Terminals MUST use the monospace family. Before the proportional-family
    // font swap, this function worked with `FontId::proportional` only by
    // accident — JetBrains Mono was registered as the primary font for both
    // families. The correct routing is through `FontId::monospace` so any
    // future change to the Proportional family (adding a real proportional
    // font for UI) doesn't break column alignment in `ls`, `top`, etc.
    TerminalFont::new(FontSettings {
        font_type: FontId::monospace(size),
    })
}

// System fonts tried at runtime as additional fallbacks (macOS only).
// Apple Symbols covers geometric shapes, Miscellaneous Technical (⌘ ⌥ ⏺ etc.),
// and Dingbats that CLI tools like Claude Code and Starship commonly use.
const SYSTEM_FALLBACK_FONTS: &[(&str, &str)] =
    &[("Apple Symbols", "/System/Library/Fonts/Apple Symbols.ttf")];

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
    fonts.font_data.insert(
        UNICODE_FALLBACK_FONT_NAME.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../fonts/NotoSans-Regular.ttf"
        ))),
    );
    // Proportional family: DejaVuSans (actually proportional) is primary so
    // UI text reads like a real app instead of a monospace terminal dump.
    // JetBrains Mono falls through second — if DejaVuSans lacks a glyph
    // (nerd-font icons, box drawing), the mono font provides coverage.
    // BREAKS IF: UI text looks monospace again (priorities swapped back, or
    // DejaVuSans removed from the bundle).
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, FALLBACK_FONT_NAME.to_owned());
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(1, FONT_NAME.to_owned());
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(2, UNICODE_FALLBACK_FONT_NAME.to_owned());
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
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(2, UNICODE_FALLBACK_FONT_NAME.to_owned());

    // Load system fonts as additional fallbacks after bundled fonts but before egui defaults.
    for (name, path) in SYSTEM_FALLBACK_FONTS {
        if let Ok(data) = std::fs::read(path) {
            fonts.font_data.insert(
                (*name).to_owned(),
                Arc::new(egui::FontData::from_owned(data)),
            );
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(2, (*name).to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(2, (*name).to_owned());
        }
    }

    fonts
}

pub fn setup_fonts(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
}
