use egui::text::LayoutJob;

use crate::ui::theme::Colors;

pub(crate) struct SyntaxHighlighter;

impl SyntaxHighlighter {
    pub(crate) fn highlight(
        ctx: &egui::Context,
        style: &egui::Style,
        code: &str,
        language: &str,
        colors: &Colors,
    ) -> LayoutJob {
        let mut style = style.clone();
        style.visuals.dark_mode = !is_light_theme(colors);
        style.visuals.override_text_color = Some(colors.text_primary);
        style.visuals.extreme_bg_color = colors.bg_active;
        style.visuals.code_bg_color = colors.bg_active;
        style.visuals.hyperlink_color = colors.accent;
        style.visuals.selection.bg_fill = colors.accent.gamma_multiply(0.35);
        style.visuals.selection.stroke.color = colors.text_primary;

        let theme = egui_extras::syntax_highlighting::CodeTheme::from_style(&style);
        egui_extras::syntax_highlighting::highlight(ctx, &style, &theme, code, language)
    }
}

fn is_light_theme(colors: &Colors) -> bool {
    luminance(colors.bg_darkest) > luminance(colors.text_primary)
}

fn luminance(color: egui::Color32) -> f32 {
    let rgba = egui::Rgba::from(color);
    0.2126 * rgba.r() + 0.7152 * rgba.g() + 0.0722 * rgba.b()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_theme_detection_tracks_background_luminance() {
        let mut cfg = crate::config::ThemeConfig::default();
        cfg.bg_darkest = Some("#f6f6f6".to_string());
        cfg.text_primary = Some("#111111".to_string());
        assert!(is_light_theme(&Colors::from_config(&cfg)));

        cfg.bg_darkest = Some("#111111".to_string());
        cfg.text_primary = Some("#f6f6f6".to_string());
        assert!(!is_light_theme(&Colors::from_config(&cfg)));
    }
}
