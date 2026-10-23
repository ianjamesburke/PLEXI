use crate::config::ThemeConfig;
use egui::{Color32, FontId};
use egui_term::{ColorPalette, FontSettings, TerminalFont, TerminalTheme};
use std::collections::HashMap;
use std::sync::Arc;

pub const FONT_SIZE: f32 = 14.0;
const FONT_NAME: &str = "JetBrainsMono Nerd Font";
const BOLD_FONT_NAME: &str = "JetBrainsMono Nerd Font Bold";
const TERMINAL_BOLD_FAMILY: &str = "terminal-bold";
const UI_FONT_NAME: &str = "Inter";
const UI_FONT_MEDIUM_NAME: &str = "Inter Medium";
const FALLBACK_FONT_NAME: &str = "DejaVu Sans";
const UNICODE_FALLBACK_FONT_NAME: &str = "Noto Sans";
const EMOJI_FALLBACK_FONT_NAME: &str = "Noto Emoji";

/// Named family for medium-weight UI text (titles, section headers). egui has
/// no weight axis — a second family backed by Inter Medium is how weight
/// contrast works.
const UI_MEDIUM_FAMILY: &str = "ui-medium";

/// FontId for medium-weight UI text. Pair with `RichText::font(...)`.
pub fn font_medium(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name(UI_MEDIUM_FAMILY.into()))
}

fn parse_hex_or(s: &Option<String>, default: Color32) -> Color32 {
    let [r, g, b] = hex_to_bytes(s.as_deref(), [default.r(), default.g(), default.b()]);
    Color32::from_rgb(r, g, b)
}

/// WCAG relative luminance.
pub(crate) fn luminance(c: Color32) -> f32 {
    fn linear(v: f32) -> f32 {
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * linear(c.r() as f32 / 255.0)
        + 0.7152 * linear(c.g() as f32 / 255.0)
        + 0.0722 * linear(c.b() as f32 / 255.0)
}

/// WCAG contrast ratio between two relative luminances.
pub(crate) fn contrast(a: f32, b: f32) -> f32 {
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

/// Per-channel sRGB blend from `a` (t = 0) to `b` (t = 1).
fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

#[derive(Clone, Copy, PartialEq)]
pub struct Colors {
    // Background layers
    pub bg_darkest: Color32,
    pub bg_sidebar: Color32,
    pub bg_toolbar: Color32,
    pub terminal_bg: Color32,
    pub tile_bg: Color32,
    pub code_bg: Color32,
    pub bg_hover: Color32,
    pub bg_elevated: Color32,
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
    pub state_working: Color32,
    pub state_waiting: Color32,
    pub state_blocked: Color32,
    pub state_needs_you: Color32,
    pub state_interrupted: Color32,
    pub state_error: Color32,
    // Motorways / Zoom / Attention Queue tokens (Tokyo Night palette)
    pub zoom_needs: Color32,
    pub zoom_failed: Color32,
    pub depth_tints: [Color32; 5],
    pub shadow_flat: Color32,
    pub scrim_decision: Color32,
    pub scrim_queue: Color32,
    pub needs_fill: Color32,
    pub needs_badge_fill: Color32,
    pub failed_fill: Color32,
    pub focus_fill: Color32,
    pub focus_glow: Color32,
    // Opacity multiplier for unfocused pips (default 0.72)
    pub pip_dim: f32,
    // Terminal fg/bg as bytes for dynamic colors
    pub terminal_fg_bytes: [u8; 3],
    pub terminal_bg_bytes: [u8; 3],
}

impl Colors {
    /// Readable foreground for text drawn on `fill` (accent buttons, danger
    /// buttons, swatches). Picks whichever of `text_primary` / `bg_darkest`
    /// has the higher WCAG contrast ratio against the fill; if neither
    /// palette color reaches 3:1 (e.g. a mid-luminance accent), falls back to
    /// pure black/white so text is never illegible on any theme — including
    /// user-customized accents. Never hard-code WHITE or `bg_darkest` on a
    /// colored fill.
    pub fn text_on(&self, fill: Color32) -> Color32 {
        let fill_lum = luminance(fill);
        let primary_contrast = contrast(luminance(self.text_primary), fill_lum);
        let darkest_contrast = contrast(luminance(self.bg_darkest), fill_lum);
        let (best, best_contrast) = if primary_contrast >= darkest_contrast {
            (self.text_primary, primary_contrast)
        } else {
            (self.bg_darkest, darkest_contrast)
        };
        if best_contrast >= 3.0 {
            best
        } else if contrast(0.0, fill_lum) > contrast(1.0, fill_lum) {
            Color32::BLACK
        } else {
            Color32::WHITE
        }
    }

    /// Foreground for *primary* labels that used to render in `text_dim`
    /// (sidebar context names, app-bar subtitles, pane-name bars). `text_dim`
    /// sits around 3:1 on most presets — fine for true-secondary metadata,
    /// too faint for a label the user reads constantly. Blends `text_dim`
    /// toward `text_primary` just far enough to reach WCAG 4.5:1 on `bg`
    /// (stint 0528), so themes whose `text_dim` already passes keep their
    /// look untouched.
    pub fn text_secondary(&self, bg: Color32) -> Color32 {
        const TARGET: f32 = 4.5;
        let bg_lum = luminance(bg);
        let mut result = self.text_dim;
        // Eight even blend steps toward text_primary; the first one meeting
        // the floor wins. If even text_primary misses the floor (a broken
        // custom theme), text_primary is still the best available answer.
        for step in 0..=8 {
            let t = step as f32 / 8.0;
            let candidate = lerp_color(self.text_dim, self.text_primary, t);
            if contrast(luminance(candidate), bg_lum) >= TARGET {
                result = candidate;
                break;
            }
            result = candidate;
        }
        result
    }

    /// Background for terminal pane name bars: `terminal_bg` darkened ~12%.
    /// Matches the zoom overlay, where the same bar is painted at 0.88
    /// opacity over the dark scrim — keeps the header tone identical in
    /// tiled and zoomed views across every theme.
    pub fn pane_header_bg(&self) -> Color32 {
        let [r, g, b, _] = self.terminal_bg.to_array();
        Color32::from_rgb(
            (r as f32 * 0.88) as u8,
            (g as f32 * 0.88) as u8,
            (b as f32 * 0.88) as u8,
        )
    }

    /// Surface depth tint for tree depth 0..4 (clamped at 4).
    pub fn depth_tint(&self, depth: usize) -> Color32 {
        self.depth_tints[depth.min(4)]
    }

    pub fn from_config(cfg: &ThemeConfig) -> Self {
        let bg_hover = parse_hex_or(&cfg.bg_hover, Color32::from_rgb(0x20, 0x23, 0x32));
        let states = state_palette(cfg);
        let zoom_needs = Color32::from_rgb(0xe0, 0xaf, 0x68);
        let zoom_failed = Color32::from_rgb(0xf7, 0x76, 0x8e);
        Self {
            state_working: parse_hex_or(
                &cfg.state_working
                    .clone()
                    .or_else(|| cfg.pip_working.clone()),
                states[0],
            ),
            state_waiting: parse_hex_or(
                &cfg.state_waiting.clone().or_else(|| cfg.pip_idle.clone()),
                states[1],
            ),
            state_blocked: parse_hex_or(
                &cfg.state_blocked
                    .clone()
                    .or_else(|| cfg.pip_blocked.clone()),
                states[2],
            ),
            state_needs_you: parse_hex_or(&cfg.state_needs_you, states[3]),
            state_interrupted: parse_hex_or(&cfg.state_interrupted, states[4]),
            state_error: parse_hex_or(&cfg.state_error, states[5]),
            bg_darkest: parse_hex_or(&cfg.bg_darkest, Color32::from_rgb(0x16, 0x16, 0x1e)),
            bg_sidebar: parse_hex_or(&cfg.bg_sidebar, Color32::from_rgb(0x16, 0x16, 0x1e)),
            bg_toolbar: parse_hex_or(&cfg.bg_toolbar, Color32::from_rgb(0x16, 0x16, 0x1e)),
            terminal_bg: parse_hex_or(&cfg.terminal_bg, Color32::from_rgb(0x1a, 0x1b, 0x26)),
            tile_bg: Color32::from_rgb(0x20, 0x23, 0x32),
            code_bg: Color32::from_rgb(0x13, 0x14, 0x1c),
            bg_hover,
            bg_elevated: bg_hover,
            bg_sidebar_hover: parse_hex_or(
                &cfg.bg_sidebar_hover,
                Color32::from_rgb(0x26, 0x2a, 0x3d),
            ),
            bg_active: parse_hex_or(&cfg.bg_active, Color32::from_rgb(0x29, 0x2e, 0x42)),
            text_primary: parse_hex_or(&cfg.text_primary, Color32::from_rgb(0xc0, 0xca, 0xf5)),
            text_dim: parse_hex_or(&cfg.text_dim, Color32::from_rgb(0x73, 0x7a, 0xa2)),
            text_section: parse_hex_or(&cfg.text_section, Color32::from_rgb(0x56, 0x5f, 0x89)),
            accent: parse_hex_or(&cfg.accent, Color32::from_rgb(0x7a, 0xa2, 0xf7)),
            border: parse_hex_or(&cfg.border, Color32::from_rgb(0x3b, 0x42, 0x61)),
            danger: parse_hex_or(&cfg.red, zoom_failed),
            success: parse_hex_or(&cfg.green, Color32::from_rgb(0x9e, 0xce, 0x6a)),
            warning: parse_hex_or(&cfg.yellow, zoom_needs),
            zoom_needs,
            zoom_failed,
            depth_tints: [
                Color32::from_rgb(0x1a, 0x1b, 0x26),
                Color32::from_rgb(0x20, 0x24, 0x36),
                Color32::from_rgb(0x29, 0x2e, 0x42),
                Color32::from_rgb(0x31, 0x3c, 0x58),
                Color32::from_rgb(0x39, 0x4b, 0x70),
            ],
            shadow_flat: Color32::from_rgb(0x0c, 0x0e, 0x14),
            scrim_decision: Color32::from_rgba_unmultiplied(12, 14, 20, 204),
            scrim_queue: Color32::from_rgba_unmultiplied(12, 14, 20, 140),
            needs_fill: Color32::from_rgba_unmultiplied(224, 175, 104, 31),
            needs_badge_fill: Color32::from_rgba_unmultiplied(224, 175, 104, 36),
            failed_fill: Color32::from_rgba_unmultiplied(247, 118, 142, 26),
            focus_fill: Color32::from_rgba_unmultiplied(122, 162, 247, 36),
            focus_glow: Color32::from_rgba_unmultiplied(122, 162, 247, 38),
            pip_dim: cfg.pip_dim.unwrap_or(0.72),
            terminal_fg_bytes: hex_to_bytes(cfg.foreground.as_deref(), [0xc0, 0xca, 0xf5]),
            terminal_bg_bytes: hex_to_bytes(cfg.background.as_deref(), [0x1a, 0x1b, 0x26]),
        }
    }

    /// Serialize the semantic color roles to a `role -> #rrggbb` map for the
    /// SDK `Init` payload. Apps read these via `ctx.theme.<role>` so app-drawn
    /// chrome tracks the host theme (light/dark + user `[theme]` overrides).
    /// Both the SDK-semantic names (bg/surface/muted/...) and ANSI aliases
    /// (red/green/yellow) are emitted so apps can pull whichever they need.
    pub fn to_theme_map(self) -> std::collections::HashMap<String, String> {
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
            ("state_working", self.state_working),
            ("state_waiting", self.state_waiting),
            ("state_blocked", self.state_blocked),
            ("state_needs_you", self.state_needs_you),
            ("state_interrupted", self.state_interrupted),
            ("state_error", self.state_error),
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

/// State presets retain a separate shape channel; mono intentionally shares a color.
pub const STATE_PRESETS: &[&str] = &["default", "okabe-ito", "high-contrast", "mono"];

fn state_palette(cfg: &ThemeConfig) -> [Color32; 6] {
    let sidebar = parse_hex_or(&cfg.bg_sidebar, Color32::from_rgb(24, 24, 37));
    let terminal = parse_hex_or(&cfg.terminal_bg, Color32::from_rgb(41, 42, 68));
    let light = luminance(sidebar) > 0.4;
    let mono = if light {
        Color32::BLACK
    } else {
        Color32::WHITE
    };
    let preset = cfg.state_preset.as_deref().unwrap_or("okabe-ito");
    // Light themes need separated lightness levels, not uniform darkening:
    // otherwise orange, vermillion and yellow converge under red/green CVD.
    if light && preset != "mono" {
        return [
            [26, 54, 70],
            [104, 72, 0],
            [173, 103, 142],
            [64, 28, 0],
            [0, 95, 69],
            [132, 125, 36],
        ]
        .map(|[r, g, b]| Color32::from_rgb(r, g, b));
    }
    let rgb = match preset {
        "mono" => return [mono; 6],
        "high-contrast" => [
            [0, 114, 178],
            [240, 228, 66],
            [204, 121, 167],
            [213, 94, 0],
            [0, 158, 115],
            [230, 159, 0],
        ],
        _ => [
            [0, 114, 178],
            [230, 159, 0],
            [204, 121, 167],
            [213, 94, 0],
            [0, 158, 115],
            [240, 228, 66],
        ],
    };
    rgb.map(|[r, g, b]| {
        let c = Color32::from_rgb(r, g, b);
        // Preserve hue while adapting the Okabe-Ito swatch to both adjacent surfaces.
        for step in 0..=100 {
            let candidate = lerp_color(c, mono, step as f32 / 100.0);
            if [sidebar, terminal]
                .iter()
                .all(|bg| contrast(luminance(candidate), luminance(*bg)) >= 3.1)
            {
                return candidate;
            }
        }
        mono
    })
}

/// Resolve the active `Colors` from a loaded config: merges the user's
/// `[theme]` overrides over the named preset (or pure user config if no
/// preset). Shared by the GUI and the headless `app render` path.
pub fn colors_from_config(config: &crate::config::PlexiConfig) -> Colors {
    let user_theme = config.theme.clone().unwrap_or_default();
    let preset_name = user_theme
        .preset
        .as_deref()
        .or(config.theme_preset.as_deref());
    let cfg = match preset_name {
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
        "catppuccin-mocha" | "mocha" => Some("catppuccin-mocha"),
        "catppuccin-latte" | "latte" => Some("catppuccin-latte"),
        "dracula" => Some("dracula"),
        "tokyo-night" => Some("tokyo-night"),
        "tokyo-day" | "tokyonight-day" => Some("tokyo-day"),
        "gruvbox-dark" => Some("gruvbox-dark"),
        "gruvbox-light" => Some("gruvbox-light"),
        "nord" => Some("nord"),
        "solarized-dark" => Some("solarized-dark"),
        "solarized-light" => Some("solarized-light"),
        "matrix" => Some("matrix"),
        "bios" | "retro" | "retro-bios" => Some("bios"),
        "plexi-night" | "plexinight" => Some("plexi-night"),
        "plexi-day" | "plexiday" => Some("plexi-day"),
        _ => None,
    }
}

/// Returns true for presets with a light background — used to flip egui's dark_mode flag.
pub fn is_light_preset(name: &str) -> bool {
    matches!(
        canonical_preset_name(name),
        Some("catppuccin-latte")
            | Some("solarized-light")
            | Some("gruvbox-light")
            | Some("tokyo-day")
            | Some("plexi-day")
    )
}

/// Returns the list of available preset names.
///
/// Test-only: the WCAG contrast tests sweep every preset. Production code
/// reaches presets by name through [`preset_colors`], so a non-test build has
/// no caller and `-D warnings` rejects one.
#[cfg(test)]
pub fn preset_names() -> &'static [&'static str] {
    &[
        "catppuccin-mocha",
        "catppuccin-latte",
        "dracula",
        "tokyo-night",
        "tokyo-day",
        "gruvbox-dark",
        "gruvbox-light",
        "nord",
        "solarized-dark",
        "solarized-light",
        "matrix",
        "bios",
        "plexi-night",
        "plexi-day",
    ]
}

/// Returns a fully-populated ThemeConfig for the named preset, or None if unknown.
pub fn preset_colors(name: &str) -> Option<ThemeConfig> {
    let s = |v: &str| Some(v.to_string());
    let canonical = canonical_preset_name(name)?;
    match canonical {
        "catppuccin-mocha" => Some(ThemeConfig {
            preset: None,

            bg_darkest: s("#11111b"),
            bg_sidebar: s("#181825"),
            bg_toolbar: s("#181825"),
            terminal_bg: s("#1e1e2e"),
            bg_hover: s("#313244"),
            bg_sidebar_hover: s("#45475a"),
            bg_active: s("#313244"),
            text_primary: s("#cdd6f4"),
            text_dim: s("#a6adc8"),
            text_section: s("#9399b2"),
            accent: s("#89b4fa"),
            border: s("#313244"),
            foreground: s("#cdd6f4"),
            background: s("#1e1e2e"),
            black: s("#45475a"),
            red: s("#f38ba8"),
            green: s("#a6e3a1"),
            yellow: s("#f9e2af"),
            blue: s("#89b4fa"),
            magenta: s("#f5c2e7"),
            cyan: s("#94e2d5"),
            white: s("#bac2de"),
            bright_black: s("#6c7086"),
            bright_red: s("#f38ba8"),
            bright_green: s("#a6e3a1"),
            bright_yellow: s("#f9e2af"),
            bright_blue: s("#89b4fa"),
            bright_magenta: s("#f5c2e7"),
            bright_cyan: s("#94e2d5"),
            bright_white: s("#cdd6f4"),
            bright_foreground: s("#cdd6f4"),
            ..ThemeConfig::default()
        }),
        "dracula" => Some(ThemeConfig {
            preset: None,

            bg_darkest: s("#1e1f29"),
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
            ..ThemeConfig::default()
        }),
        "tokyo-night" => Some(ThemeConfig {
            preset: None,

            bg_darkest: s("#16161e"),
            bg_sidebar: s("#16161e"),
            bg_toolbar: s("#16161e"),
            terminal_bg: s("#1a1b26"),
            bg_hover: s("#202332"),
            bg_sidebar_hover: s("#262a3d"),
            bg_active: s("#292e42"),
            text_primary: s("#c0caf5"),
            text_dim: s("#737aa2"),
            text_section: s("#565f89"),
            accent: s("#7aa2f7"),
            border: s("#3b4261"),
            foreground: s("#c0caf5"),
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
            ..ThemeConfig::default()
        }),
        "tokyo-day" => Some(ThemeConfig {
            preset: None,

            bg_darkest: s("#b0b3c0"),
            bg_sidebar: s("#d5d6db"),
            bg_toolbar: s("#d5d6db"),
            terminal_bg: s("#e1e2e7"),
            bg_hover: s("#cbccd5"),
            bg_sidebar_hover: s("#c4c8da"),
            bg_active: s("#c4c8da"),
            text_primary: s("#3156ac"),
            text_dim: s("#6172b0"),
            text_section: s("#848cb5"),
            accent: s("#2e7de9"),
            border: s("#c4c8da"),
            foreground: s("#3760bf"),
            background: s("#e1e2e7"),
            black: s("#e9e9ec"),
            red: s("#f52a65"),
            green: s("#587539"),
            yellow: s("#8c6c3e"),
            blue: s("#2e7de9"),
            magenta: s("#9854f1"),
            cyan: s("#007197"),
            white: s("#6172b0"),
            bright_black: s("#a1a6c5"),
            bright_red: s("#f52a65"),
            bright_green: s("#587539"),
            bright_yellow: s("#8c6c3e"),
            bright_blue: s("#2e7de9"),
            bright_magenta: s("#9854f1"),
            bright_cyan: s("#007197"),
            bright_white: s("#3760bf"),
            bright_foreground: s("#3760bf"),
            ..ThemeConfig::default()
        }),
        "gruvbox-dark" => Some(ThemeConfig {
            preset: None,

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
            ..ThemeConfig::default()
        }),
        "gruvbox-light" => Some(ThemeConfig {
            preset: None,

            bg_darkest: s("#f2e5bc"),
            bg_sidebar: s("#fbf1c7"),
            bg_toolbar: s("#fbf1c7"),
            terminal_bg: s("#fbf1c7"),
            bg_hover: s("#ebdbb2"),
            bg_sidebar_hover: s("#d5c4a1"),
            bg_active: s("#bdae93"),
            text_primary: s("#3c3836"),
            text_dim: s("#5c534a"),
            text_section: s("#7c6f64"),
            accent: s("#d65d0e"),
            border: s("#d5c4a1"),
            foreground: s("#3c3836"),
            background: s("#fbf1c7"),
            black: s("#fbf1c7"),
            red: s("#cc241d"),
            green: s("#98971a"),
            yellow: s("#d79921"),
            blue: s("#458588"),
            magenta: s("#b16286"),
            cyan: s("#689d6a"),
            white: s("#7c6f64"),
            bright_black: s("#928374"),
            bright_red: s("#9d0006"),
            bright_green: s("#79740e"),
            bright_yellow: s("#b57614"),
            bright_blue: s("#076678"),
            bright_magenta: s("#8f3f71"),
            bright_cyan: s("#427b58"),
            bright_white: s("#3c3836"),
            bright_foreground: s("#3c3836"),
            ..ThemeConfig::default()
        }),
        "nord" => Some(ThemeConfig {
            preset: None,

            bg_darkest: s("#242933"),
            bg_sidebar: s("#3b4252"),
            bg_toolbar: s("#3b4252"),
            terminal_bg: s("#2e3440"),
            bg_hover: s("#434c5e"),
            bg_sidebar_hover: s("#495264"),
            bg_active: s("#4c566a"),
            text_primary: s("#eceff4"),
            text_dim: s("#8fa1bc"),
            text_section: s("#7b8fa8"),
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
            ..ThemeConfig::default()
        }),
        "solarized-dark" => Some(ThemeConfig {
            preset: None,

            bg_darkest: s("#00212b"),
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
            ..ThemeConfig::default()
        }),
        "catppuccin-latte" => Some(ThemeConfig {
            preset: None,

            bg_darkest: s("#dce0e8"),
            bg_sidebar: s("#eff1f5"),
            bg_toolbar: s("#eff1f5"),
            terminal_bg: s("#eff1f5"),
            bg_hover: s("#e6e9ef"),
            bg_sidebar_hover: s("#ccd0da"),
            bg_active: s("#ccd0da"),
            text_primary: s("#4c4f69"),
            text_dim: s("#6c6f85"),
            text_section: s("#5c5f77"),
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
            ..ThemeConfig::default()
        }),
        "solarized-light" => Some(ThemeConfig {
            preset: None,

            bg_darkest: s("#eee8d5"),
            bg_sidebar: s("#fdf6e3"),
            bg_toolbar: s("#fdf6e3"),
            terminal_bg: s("#fdf6e3"),
            bg_hover: s("#e0dcc8"),
            bg_sidebar_hover: s("#d8d4c0"),
            bg_active: s("#d0ccb8"),
            text_primary: s("#586e75"),
            text_dim: s("#586e75"),
            text_section: s("#657b83"),
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
            ..ThemeConfig::default()
        }),
        "matrix" => Some(ThemeConfig {
            preset: None,

            bg_darkest: s("#000000"),
            bg_sidebar: s("#001100"),
            bg_toolbar: s("#001100"),
            terminal_bg: s("#001a00"),
            bg_hover: s("#002200"),
            bg_sidebar_hover: s("#003300"),
            bg_active: s("#003d00"),
            text_primary: s("#00ff41"),
            text_dim: s("#00aa28"),
            text_section: s("#007018"),
            accent: s("#00ff41"),
            border: s("#003300"),
            foreground: s("#00ff41"),
            background: s("#001a00"),
            black: s("#000000"),
            red: s("#ff3333"),
            green: s("#00ff41"),
            yellow: s("#aaff00"),
            blue: s("#0055ff"),
            magenta: s("#cc44ff"),
            cyan: s("#00ffaa"),
            white: s("#aaffaa"),
            bright_black: s("#005500"),
            bright_red: s("#ff6666"),
            bright_green: s("#39ff14"),
            bright_yellow: s("#ccff33"),
            bright_blue: s("#3399ff"),
            bright_magenta: s("#dd77ff"),
            bright_cyan: s("#33ffcc"),
            bright_white: s("#ccffcc"),
            bright_foreground: s("#ccffcc"),
            ..ThemeConfig::default()
        }),
        "bios" => Some(ThemeConfig {
            preset: None,

            bg_darkest: s("#000055"),
            bg_sidebar: s("#000080"),
            bg_toolbar: s("#000080"),
            terminal_bg: s("#000080"),
            bg_hover: s("#0000aa"),
            bg_sidebar_hover: s("#0000bb"),
            bg_active: s("#0000cc"),
            text_primary: s("#ffffff"),
            text_dim: s("#aaaaaa"),
            text_section: s("#888888"),
            accent: s("#ffff00"),
            border: s("#aaaaaa"),
            foreground: s("#aaaaaa"),
            background: s("#000080"),
            black: s("#000000"),
            red: s("#aa0000"),
            green: s("#00aa00"),
            yellow: s("#aa5500"),
            blue: s("#0000aa"),
            magenta: s("#aa00aa"),
            cyan: s("#00aaaa"),
            white: s("#aaaaaa"),
            bright_black: s("#555555"),
            bright_red: s("#ff5555"),
            bright_green: s("#55ff55"),
            bright_yellow: s("#ffff55"),
            bright_blue: s("#5555ff"),
            bright_magenta: s("#ff55ff"),
            bright_cyan: s("#55ffff"),
            bright_white: s("#ffffff"),
            bright_foreground: s("#ffffff"),
            ..ThemeConfig::default()
        }),
        "plexi-night" => Some(ThemeConfig {
            preset: None,

            bg_darkest: s("#060609"),
            bg_sidebar: s("#0c0c14"),
            bg_toolbar: s("#0c0c14"),
            terminal_bg: s("#0e0e18"),
            bg_hover: s("#151520"),
            bg_sidebar_hover: s("#1a1a28"),
            bg_active: s("#1e1e30"),
            text_primary: s("#e8e8f0"),
            text_dim: s("#6060a0"),
            text_section: s("#404070"),
            accent: s("#7c3aed"),
            border: s("#1e1e30"),
            foreground: s("#e8e8f0"),
            background: s("#0e0e18"),
            black: s("#1a1a28"),
            red: s("#ef4444"),
            green: s("#4ade80"),
            yellow: s("#facc15"),
            blue: s("#60a5fa"),
            magenta: s("#a855f7"),
            cyan: s("#22d3ee"),
            white: s("#c8c8e0"),
            bright_black: s("#3a3a5c"),
            bright_red: s("#f87171"),
            bright_green: s("#86efac"),
            bright_yellow: s("#fde047"),
            bright_blue: s("#93c5fd"),
            bright_magenta: s("#c084fc"),
            bright_cyan: s("#67e8f9"),
            bright_white: s("#f0f0f8"),
            bright_foreground: s("#f0f0f8"),
            ..ThemeConfig::default()
        }),
        "plexi-day" => Some(ThemeConfig {
            preset: None,

            bg_darkest: s("#d8d8e8"),
            bg_sidebar: s("#ededf5"),
            bg_toolbar: s("#ededf5"),
            terminal_bg: s("#f5f5fb"),
            bg_hover: s("#e2e2ef"),
            bg_sidebar_hover: s("#d5d5e8"),
            bg_active: s("#c8c8e0"),
            text_primary: s("#1a1a2e"),
            text_dim: s("#6060a0"),
            text_section: s("#8080b0"),
            accent: s("#7c3aed"),
            border: s("#c8c8e0"),
            foreground: s("#1a1a2e"),
            background: s("#f5f5fb"),
            black: s("#2a2a40"),
            red: s("#dc2626"),
            green: s("#16a34a"),
            yellow: s("#ca8a04"),
            blue: s("#2563eb"),
            magenta: s("#7c3aed"),
            cyan: s("#0891b2"),
            white: s("#d8d8e8"),
            bright_black: s("#5a5a80"),
            bright_red: s("#ef4444"),
            bright_green: s("#22c55e"),
            bright_yellow: s("#eab308"),
            bright_blue: s("#3b82f6"),
            bright_magenta: s("#a855f7"),
            bright_cyan: s("#06b6d4"),
            bright_white: s("#f5f5fb"),
            bright_foreground: s("#0a0a1a"),
            ..ThemeConfig::default()
        }),
        _ => None,
    }
}

/// Merge a preset with user overrides — user values take precedence over the preset.
pub fn apply_preset(preset: &ThemeConfig, user: &ThemeConfig) -> ThemeConfig {
    let m = |u: &Option<String>, p: &Option<String>| u.clone().or_else(|| p.clone());
    ThemeConfig {
        preset: None,
        state_preset: m(&user.state_preset, &preset.state_preset),
        state_working: m(&user.state_working, &preset.state_working),
        state_waiting: m(&user.state_waiting, &preset.state_waiting),
        state_blocked: m(&user.state_blocked, &preset.state_blocked),
        state_needs_you: m(&user.state_needs_you, &preset.state_needs_you),
        state_interrupted: m(&user.state_interrupted, &preset.state_interrupted),
        state_error: m(&user.state_error, &preset.state_error),
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
        pip_working: m(&user.pip_working, &preset.pip_working),
        pip_idle: m(&user.pip_idle, &preset.pip_idle),
        pip_blocked: m(&user.pip_blocked, &preset.pip_blocked),
        pip_dim: user.pip_dim.or(preset.pip_dim),
    }
}

/// Map the full egui `Visuals` + `Spacing` from the Plexi theme. Every raw
/// egui widget the host still renders (buttons, combos, menus, scroll areas,
/// text edits) inherits the Plexi look from here instead of stock egui — this
/// is the single place where the default-egui appearance is retired.
pub fn setup_style(ctx: &egui::Context, colors: &Colors, dark_mode: bool) {
    use crate::ui::style as tokens;
    log::info!("theme: setup_style dark_mode={dark_mode}");
    let mut style = (*ctx.global_style()).clone();
    style.visuals = if dark_mode {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let hairline = egui::Stroke::new(1.0_f32, colors.border);
    let v = &mut style.visuals;

    // Surfaces — layered from the workspace floor up through raised chrome.
    v.panel_fill = colors.bg_darkest;
    v.window_fill = colors.bg_sidebar;
    v.extreme_bg_color = colors.bg_active; // TextEdit / ScrollArea / code block backgrounds.
    v.faint_bg_color = colors.bg_hover; // Striped rows / blockquote tints.
    v.code_bg_color = colors.bg_darkest;
    v.override_text_color = None;

    // Window / menu / popup chrome: token radii, hairline border, soft
    // long-throw shadows instead of egui's tight dark halo.
    v.window_corner_radius = tokens::RADIUS_LG;
    v.window_stroke = hairline;
    v.menu_corner_radius = tokens::RADIUS_MD;
    let shadow = |offset: [i8; 2], blur: u8| egui::Shadow {
        offset,
        blur,
        spread: 0,
        color: Color32::from_black_alpha(if dark_mode { 96 } else { 40 }),
    };
    v.window_shadow = shadow([0, 12], 32);
    v.popup_shadow = shadow([0, 6], 16);

    // Accent-driven feedback. Default egui ships its own blue selection and
    // green cursor that ignore the theme entirely.
    v.hyperlink_color = colors.accent;
    v.selection.bg_fill = colors.accent.gamma_multiply(0.35);
    v.selection.stroke = egui::Stroke::new(1.0_f32, colors.accent);
    v.text_cursor.stroke = egui::Stroke::new(1.0_f32, colors.accent);
    v.warn_fg_color = colors.warning;
    v.error_fg_color = colors.danger;
    v.slider_trailing_fill = true;

    // Widget states. `bg_fill` paints checkbox/radio/slider bodies,
    // `weak_bg_fill` paints button faces — keep them in lockstep so every
    // interactive surface steps through the same theme layers.
    let text = |width: f32| egui::Stroke::new(width, colors.text_primary);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = colors.bg_sidebar;
    w.noninteractive.weak_bg_fill = colors.bg_sidebar;
    w.noninteractive.bg_stroke = hairline; // Separators.
    w.noninteractive.fg_stroke =
        egui::Stroke::new(1.0_f32, colors.text_secondary(colors.bg_sidebar));
    w.inactive.bg_fill = colors.bg_elevated;
    w.inactive.weak_bg_fill = colors.bg_elevated;
    w.inactive.bg_stroke = hairline;
    w.inactive.fg_stroke = text(1.0);
    w.hovered.bg_fill = colors.bg_sidebar_hover;
    w.hovered.weak_bg_fill = colors.bg_sidebar_hover;
    w.hovered.bg_stroke = egui::Stroke::new(1.0_f32, colors.text_section);
    w.hovered.fg_stroke = text(1.5);
    w.active.bg_fill = colors.bg_active;
    w.active.weak_bg_fill = colors.bg_active;
    w.active.bg_stroke = egui::Stroke::new(1.0_f32, colors.accent);
    w.active.fg_stroke = text(2.0);
    w.open.bg_fill = colors.bg_active;
    w.open.weak_bg_fill = colors.bg_active;
    w.open.bg_stroke = hairline;
    w.open.fg_stroke = text(1.0);
    for state in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        state.corner_radius = tokens::RADIUS_SM;
        // Default egui grows widgets on hover/press; modern chrome stays put.
        state.expansion = 0.0;
    }

    style.spacing.item_spacing = egui::vec2(tokens::SPACE_SM, tokens::SPACE_XS);
    // Default egui button padding is (4, 1) — the single biggest "default
    // egui" tell. Real apps give labels room to breathe.
    style.spacing.button_padding = egui::vec2(tokens::SPACE_MD, 6.0);
    style.spacing.menu_margin = egui::Margin::same(tokens::SPACE_SM as i8);
    style.spacing.window_margin = egui::Margin::same(tokens::SPACE_MD as i8);
    // Overlay scrollbars: invisible until the scroll area is hovered.
    style.spacing.scroll = egui::style::ScrollStyle::floating();
    // Reserve a gutter for the floating bar so full-width content (text
    // fields, selectable rows) never runs underneath it at the right edge.
    style.spacing.scroll.floating_allocated_width = tokens::SPACE_MD;
    ctx.set_theme(if dark_mode {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);
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
    .with_bold_font(FontId::new(
        size,
        egui::FontFamily::Name(TERMINAL_BOLD_FAMILY.into()),
    ))
}

// System fonts tried at runtime as additional fallbacks (macOS only).
// Apple Symbols covers geometric shapes, Miscellaneous Technical (⌘ ⌥ ⏺ etc.),
// and Dingbats that CLI tools like Claude Code and Starship commonly use.
const SYSTEM_FALLBACK_FONTS: &[(&str, &str)] =
    &[("Apple Symbols", "/System/Library/Fonts/Apple Symbols.ttf")];

pub fn font_definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    // Regular weight (stint 0528): the Light cut anti-aliases into grey mush
    // at UI sizes (8–13 px); terminals and the editor read it constantly.
    fonts.font_data.insert(
        FONT_NAME.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../fonts/JetBrainsMonoNerdFont-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        BOLD_FONT_NAME.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../fonts/JetBrainsMonoNerdFont-Bold.ttf"
        ))),
    );
    fonts.font_data.insert(
        UI_FONT_NAME.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../fonts/Inter-Regular.otf"
        ))),
    );
    fonts.font_data.insert(
        UI_FONT_MEDIUM_NAME.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../fonts/Inter-Medium.otf"
        ))),
    );
    fonts.font_data.insert(
        FALLBACK_FONT_NAME.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../fonts/DejaVuSans.ttf"
        ))),
    );
    fonts.font_data.insert(
        UNICODE_FALLBACK_FONT_NAME.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../fonts/NotoSans-Regular.ttf"
        ))),
    );
    // Monochrome emoji fallback. egui/epaint rasterizes greyscale glyphs only,
    // so a color emoji font (Apple Color Emoji) would render as tofu; Noto Emoji
    // is the monochrome build that covers the emoji planes. Bound last in every
    // family so it never shadows a glyph an earlier font provides.
    fonts.font_data.insert(
        EMOJI_FALLBACK_FONT_NAME.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../fonts/NotoEmoji-Regular.ttf"
        ))),
    );
    // Proportional family: Inter first (stint 0528 — proportional UI text
    // renders in a real text face at Regular weight), with the Nerd Font
    // immediately behind it so icon/glyph fallback (powerline, dev glyphs)
    // keeps working in UI labels. Monospace/terminal text stays routed
    // through `FontFamily::Monospace` below.
    let proportional = fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default();
    proportional.insert(0, UI_FONT_NAME.to_owned());
    proportional.insert(1, FONT_NAME.to_owned());
    proportional.insert(2, FALLBACK_FONT_NAME.to_owned());
    proportional.insert(3, UNICODE_FALLBACK_FONT_NAME.to_owned());
    proportional.push(EMOJI_FALLBACK_FONT_NAME.to_owned());
    // Medium family: Inter Medium first, then the same fallback chain — this
    // is how weight contrast works, since egui has no weight axis.
    let mut medium = proportional.clone();
    medium.insert(0, UI_FONT_MEDIUM_NAME.to_owned());
    fonts
        .families
        .insert(egui::FontFamily::Name(UI_MEDIUM_FAMILY.into()), medium);
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
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push(EMOJI_FALLBACK_FONT_NAME.to_owned());

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

    let mut bold = fonts.families[&egui::FontFamily::Monospace].clone();
    bold.insert(0, BOLD_FONT_NAME.to_owned());
    fonts
        .families
        .insert(egui::FontFamily::Name(TERMINAL_BOLD_FAMILY.into()), bold);

    fonts
}

pub fn setup_fonts(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
    log::info!("terminal renderer: regular/bold JetBrains Mono with geometric shade cells");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ThemeConfig;

    #[test]
    fn terminal_bold_face_preserves_regular_cell_metrics() {
        let ctx = egui::Context::default();
        setup_fonts(&ctx);
        let _ = ctx.run_ui(Default::default(), |ui| {
            ui.fonts_mut(|fonts| {
                for size in [12.0, 18.0, 28.0] {
                    let terminal = terminal_font(size);
                    let regular = terminal.font_type();
                    let bold = terminal.font_type_for_bold(true);
                    assert_ne!(regular.family, bold.family);
                    assert_eq!(fonts.row_height(&regular), fonts.row_height(&bold));
                    for c in "mMW08[]".chars() {
                        assert_eq!(fonts.glyph_width(&regular, c), fonts.glyph_width(&bold, c));
                    }
                }
            });
        });
    }

    /// Legacy pip color overrides remain aliases for the new semantic tokens.
    #[test]
    fn pip_colors_fall_back_to_semantic_and_accept_overrides() {
        let default_cfg = ThemeConfig::default();
        let colors = Colors::from_config(&default_cfg);
        assert_ne!(colors.state_working, colors.state_waiting);
        assert!((colors.pip_dim - 0.72).abs() < f32::EPSILON);

        let override_cfg = ThemeConfig {
            pip_working: Some("#ff0000".to_string()),
            pip_idle: Some("#00ff00".to_string()),
            pip_blocked: Some("#0000ff".to_string()),
            pip_dim: Some(0.7),
            ..ThemeConfig::default()
        };
        let oc = Colors::from_config(&override_cfg);
        assert_eq!(oc.state_working, Color32::from_rgb(0xff, 0x00, 0x00));
        assert_eq!(oc.state_waiting, Color32::from_rgb(0x00, 0xff, 0x00));
        assert_eq!(oc.state_blocked, Color32::from_rgb(0x00, 0x00, 0xff));
        assert!((oc.pip_dim - 0.7).abs() < f32::EPSILON);
    }

    /// Every preset's accent button must render legible text: text_on must
    /// return a color with at least 3:1 WCAG contrast against the accent and
    /// danger fills. Guards the whole preset table, including future entries.
    #[test]
    fn text_on_is_legible_on_every_preset_accent_and_danger() {
        // Ratio between two colors, over the module's own WCAG helpers.
        let ratio = |a: Color32, b: Color32| contrast(luminance(a), luminance(b));
        for name in preset_names() {
            let cfg = preset_colors(name).unwrap();
            let colors = Colors::from_config(&cfg);
            for fill in [colors.accent, colors.danger] {
                let text = colors.text_on(fill);
                assert!(
                    ratio(text, fill) >= 3.0,
                    "{name}: text_on({fill:?}) = {text:?} is below 3:1 contrast"
                );
            }
        }
    }

    /// Stint 0528 contrast floor: primary labels rendered in the secondary
    /// tone (inactive sidebar context names, app-bar subtitles, pane-name
    /// bars) must reach WCAG 4.5:1 on their actual backgrounds for every
    /// preset. `text_dim` alone sits near 3:1 on most presets.
    #[test]
    fn text_secondary_meets_wcag_aa_on_every_preset() {
        for name in preset_names() {
            let cfg = preset_colors(name).unwrap();
            let colors = Colors::from_config(&cfg);
            for bg in [colors.bg_sidebar, colors.bg_toolbar] {
                let fg = colors.text_secondary(bg);
                let ratio = contrast(luminance(fg), luminance(bg));
                // A theme whose text_primary itself sits below 4.5:1 (e.g.
                // tokyo-night's #a9b1d6 on #16161e is 4.41:1) can't do better
                // than its own ceiling — require the floor or that ceiling.
                let ceiling = contrast(luminance(colors.text_primary), luminance(bg));
                let target = 4.5_f32.min(ceiling) - 0.01;
                assert!(
                    ratio >= target,
                    "{name}: text_secondary on {bg:?} = {fg:?} is {ratio:.2}:1, \
                     below required {target:.2}:1"
                );
            }
        }
    }

    #[test]
    fn catppuccin_presets_use_official_flavor_tokens() {
        let mocha = preset_colors("catppuccin-mocha").unwrap();
        assert_eq!(mocha.background.as_deref(), Some("#1e1e2e"));
        assert_eq!(mocha.foreground.as_deref(), Some("#cdd6f4"));
        assert_eq!(mocha.bg_darkest.as_deref(), Some("#11111b"));
        assert_eq!(mocha.bg_sidebar.as_deref(), Some("#181825"));
        assert_eq!(mocha.terminal_bg.as_deref(), Some("#1e1e2e"));
        assert_eq!(mocha.red.as_deref(), Some("#f38ba8"));
        assert_eq!(mocha.green.as_deref(), Some("#a6e3a1"));
        assert_eq!(mocha.blue.as_deref(), Some("#89b4fa"));

        let latte = preset_colors("catppuccin-latte").unwrap();
        assert_eq!(latte.background.as_deref(), Some("#eff1f5"));
        assert_eq!(latte.foreground.as_deref(), Some("#4c4f69"));
        assert_eq!(latte.bg_darkest.as_deref(), Some("#dce0e8"));
        assert_eq!(latte.bg_sidebar.as_deref(), Some("#eff1f5"));
        assert_eq!(latte.terminal_bg.as_deref(), Some("#eff1f5"));
        assert_eq!(latte.red.as_deref(), Some("#d20f39"));
        assert_eq!(latte.green.as_deref(), Some("#40a02b"));
        assert_eq!(latte.blue.as_deref(), Some("#1e66f5"));
    }

    #[test]
    fn concrete_light_presets_remain_selectable() {
        for name in [
            "catppuccin-latte",
            "solarized-light",
            "gruvbox-light",
            "tokyo-day",
            "plexi-day",
        ] {
            assert!(preset_colors(name).is_some(), "missing light preset {name}");
            assert!(is_light_preset(name), "{name} must be classified as light");
        }
    }

    #[test]
    fn catppuccin_latte_secondary_text_is_legible_on_primary_surfaces() {
        // Ratio between two colors, over the module's own WCAG helpers.
        let ratio = |a: Color32, b: Color32| contrast(luminance(a), luminance(b));

        for name in &[
            "catppuccin-latte",
            "tokyo-day",
            "gruvbox-light",
            "solarized-light",
        ] {
            let cfg = preset_colors(name).unwrap();
            let colors = Colors::from_config(&cfg);
            // bg_sidebar is where inactive row text (text_dim) appears — must be readable.
            assert!(
                ratio(colors.text_primary, colors.bg_sidebar) >= 4.5,
                "{name}: text_primary below 4.5:1 on bg_sidebar"
            );
            assert!(
                ratio(colors.text_dim, colors.bg_sidebar) >= 3.0,
                "{name}: text_dim below 3:1 on bg_sidebar (inactive row text illegible)"
            );
            // bg_active is the chip background — text_on must achieve 3:1 there.
            let chip_text = colors.text_on(colors.bg_active);
            assert!(
                ratio(chip_text, colors.bg_active) >= 3.0,
                "{name}: chip text_on(bg_active) below 3:1 (chip text illegible)"
            );
        }
    }

    #[test]
    fn setup_style_locks_theme_against_os_system_theme_override() {
        let ctx = egui::Context::default();
        let colors = Colors::from_config(&ThemeConfig::default());
        setup_style(&ctx, &colors, true);

        let _ = ctx.run_ui(
            egui::RawInput {
                system_theme: Some(egui::Theme::Light),
                ..Default::default()
            },
            |_ui| {},
        );

        assert!(ctx.global_style().visuals.dark_mode);
        assert_eq!(
            ctx.global_style().visuals.widgets.inactive.bg_fill,
            colors.bg_elevated
        );
    }
}

#[cfg(test)]
pub(crate) fn simulate_cvd(c: Color32, kind: usize) -> Color32 {
    // Machado et al. (2009), severity 1.0, linear RGB matrices.
    let matrices = [
        [
            [0.152286, 1.052583, -0.204868],
            [0.114503, 0.786281, 0.099216],
            [-0.003882, -0.048116, 1.051998],
        ],
        [
            [0.367322, 0.860646, -0.227968],
            [0.280085, 0.672501, 0.047413],
            [-0.011820, 0.042940, 0.968881],
        ],
        [
            [1.255528, -0.076749, -0.178779],
            [-0.078411, 0.930809, 0.147602],
            [0.004733, 0.691367, 0.303900],
        ],
    ];
    let rgba = egui::Rgba::from(c);
    let input = [rgba.r(), rgba.g(), rgba.b()];
    let result = matrices[kind].map(|row| {
        row.iter()
            .zip(input)
            .map(|(a, b)| a * b)
            .sum::<f32>()
            .clamp(0.0, 1.0)
    });
    egui::Rgba::from_rgb(result[0], result[1], result[2]).into()
}

#[cfg(test)]
mod state_palette_tests {
    use super::*;
    fn tokens(c: Colors) -> [Color32; 6] {
        [
            c.state_working,
            c.state_waiting,
            c.state_blocked,
            c.state_needs_you,
            c.state_interrupted,
            c.state_error,
        ]
    }
    fn lab(c: Color32) -> [f32; 3] {
        let rgb = egui::Rgba::from(c);
        let [r, g, b] = [rgb.r(), rgb.g(), rgb.b()];
        let f = |v: f32| {
            if v > 0.008856 {
                v.cbrt()
            } else {
                7.787 * v + 16.0 / 116.0
            }
        };
        let x = f((0.4124564 * r + 0.3575761 * g + 0.1804375 * b) / 0.95047);
        let y = f(0.2126729 * r + 0.7151522 * g + 0.0721750 * b);
        let z = f((0.0193339 * r + 0.119192 * g + 0.9503041 * b) / 1.08883);
        [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
    }
    #[test]
    fn state_presets_contrast_and_cvd() {
        for theme in ["plexi-night", "plexi-day"] {
            for preset in STATE_PRESETS {
                let mut cfg = preset_colors(theme).unwrap();
                cfg.state_preset = Some((*preset).into());
                let colors = Colors::from_config(&cfg);
                let states = tokens(colors);
                for color in states {
                    for bg in [colors.bg_sidebar, colors.terminal_bg] {
                        assert!(
                            contrast(luminance(color), luminance(bg)) >= 3.0,
                            "{theme}/{preset}: {color:?} against {bg:?}"
                        );
                    }
                }
                if *preset != "mono" {
                    assert!(
                        (lab(states[0])[0] - lab(states[1])[0]).abs() >= 8.0,
                        "{theme}/{preset}: working/waiting lightness separation"
                    );
                }
                if *preset == "mono" {
                    assert!(states.iter().all(|c| *c == states[0]));
                    continue; // Mono uses distinct glyphs, never color separation.
                }
                for kind in 0..3 {
                    let simulated = states.map(|c| lab(simulate_cvd(c, kind)));
                    for i in 0..6 {
                        for j in i + 1..6 {
                            let delta = simulated[i]
                                .iter()
                                .zip(simulated[j])
                                .map(|(a, b)| (a - b).powi(2))
                                .sum::<f32>()
                                .sqrt();
                            assert!(
                                delta >= 10.0,
                                "{theme}/{preset}, CVD={kind}, pair={i}/{j}, deltaE={delta}"
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn state_override_wins_over_legacy_and_preset() {
        let cfg = ThemeConfig {
            state_working: Some("#abcdef".into()),
            pip_working: Some("#123456".into()),
            state_preset: Some("mono".into()),
            ..Default::default()
        };
        assert_eq!(
            Colors::from_config(&cfg).state_working,
            Color32::from_rgb(0xab, 0xcd, 0xef)
        );
    }
    #[test]
    fn luminance_uses_srgb_once() {
        assert!((luminance(Color32::from_rgb(128, 128, 128)) - 0.21586).abs() < 0.001);
    }
}
