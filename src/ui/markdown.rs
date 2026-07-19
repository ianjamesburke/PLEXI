//! Shared markdown rendering style for host and PGAP surfaces.

use crate::ui::style;
use crate::ui::theme::Colors;

/// Convert chat-style soft breaks to CommonMark hard breaks outside fenced
/// code blocks, preserving single newlines in assistant replies.
pub fn harden_soft_breaks(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.lines().count() * 2);
    let mut in_fence = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        }
        out.push_str(line);
        if !in_fence && !line.trim().is_empty() {
            out.push_str("  ");
        }
        out.push('\n');
    }
    out
}

pub fn show(
    ui: &mut egui::Ui,
    cache: &mut egui_commonmark::CommonMarkCache,
    colors: &Colors,
    text: &str,
    text_color: egui::Color32,
    base_size: f32,
) {
    ui.scope(|ui| {
        let s = ui.style_mut();
        s.visuals.override_text_color = Some(text_color);
        s.visuals.extreme_bg_color = colors.bg_active;
        s.visuals.code_bg_color = colors.bg_active;
        s.visuals.hyperlink_color = colors.accent;
        s.visuals.faint_bg_color = colors.bg_hover;
        s.visuals.widgets.noninteractive.corner_radius = style::RADIUS_MD;
        s.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, colors.border);
        s.spacing.item_spacing.y = style::SPACE_SM;
        s.text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(base_size));
        s.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::proportional(base_size * 1.3),
        );
        s.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::monospace(base_size * 0.9),
        );
        s.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::proportional(base_size * 0.85),
        );
        egui_commonmark::CommonMarkViewer::new().show(ui, cache, text);
    });
}

#[cfg(test)]
mod tests {
    use super::harden_soft_breaks;

    #[test]
    fn harden_soft_breaks_makes_single_breaks_hard_outside_fences() {
        let out = harden_soft_breaks("line one\nline two\n\nline three");
        assert_eq!(out, "line one  \nline two  \n\nline three  \n");
    }

    #[test]
    fn harden_soft_breaks_leaves_fenced_code_untouched() {
        let out = harden_soft_breaks("before\n```\nlet x = 1;\nlet y = 2;\n```\nafter");
        assert!(out.contains("before  \n"));
        assert!(
            out.contains("let x = 1;\nlet y = 2;\n"),
            "code lines must stay byte-identical"
        );
        assert!(out.contains("after  \n"));
    }
}
