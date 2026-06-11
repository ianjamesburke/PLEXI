use egui::{Color32, CornerRadius, Pos2, RichText, Vec2};

use crate::ui::style;
use crate::ui::theme::Colors;

const KEYCAP_PAD_H: f32 = 5.0;
const KEYCAP_PAD_V: f32 = 2.0;
const INTRA_COMBO_GAP: f32 = 2.0;
const INTER_COMBO_GAP: f32 = 10.0;
const TRAILING_GAP: f32 = 10.0;

fn combo_chip_font() -> egui::FontId {
    egui::FontId::monospace(style::TEXT_CAPTION)
}

fn shortcut_key_color(colors: &Colors) -> egui::Color32 {
    colors.text_primary.gamma_multiply(0.78)
}

fn shortcut_label_color(colors: &Colors) -> egui::Color32 {
    colors.text_primary.gamma_multiply(0.70)
}

pub(crate) fn shortcut_hint_label(text: &str, colors: &Colors) -> egui::RichText {
    egui::RichText::new(text)
        .size(style::TEXT_HINT)
        .color(shortcut_label_color(colors))
}

pub(crate) fn key_chip(
    ui: &mut egui::Ui,
    label: &str,
    colors: &Colors,
    font_id: egui::FontId,
) -> egui::Response {
    let fg = shortcut_key_color(colors);
    let galley = ui.fonts(|f| f.layout_no_wrap(label.to_string(), font_id, fg));
    let text_w = galley.size().x;
    let text_h = galley.size().y;
    let chip_h = text_h + KEYCAP_PAD_V * 2.0;
    let chip_w = (text_w + KEYCAP_PAD_H * 2.0).max(chip_h);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(chip_w, chip_h), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(4), colors.bg_active);
    let text_pos = Pos2::new(rect.center().x - text_w / 2.0, rect.min.y + KEYCAP_PAD_V);
    painter.galley(text_pos, galley, fg);
    response
}

pub(crate) fn key_combo(ui: &mut egui::Ui, keys: &[&str], colors: &Colors) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = INTRA_COMBO_GAP;
        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                ui.label(
                    RichText::new("+")
                        .size(style::TEXT_HINT)
                        .color(shortcut_label_color(colors)),
                );
            }
            key_chip(ui, key, colors, combo_chip_font());
        }
    });
}

pub(crate) fn key_combo_list(
    ui: &mut egui::Ui,
    combos: &[&[&str]],
    trailing: Option<&str>,
    colors: &Colors,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, keys) in combos.iter().enumerate() {
            if i > 0 {
                ui.add_space(INTER_COMBO_GAP);
            }
            for (j, key) in keys.iter().enumerate() {
                if j > 0 {
                    ui.add_space(INTRA_COMBO_GAP);
                }
                key_chip(ui, key, colors, combo_chip_font());
            }
        }
        if let Some(text) = trailing {
            ui.add_space(TRAILING_GAP);
            ui.label(shortcut_hint_label(text, colors));
        }
    });
}

pub(crate) fn key_combo_list_width(
    ui: &egui::Ui,
    combos: &[&[&str]],
    trailing: Option<&str>,
) -> f32 {
    let measure = |text: &str, font: egui::FontId| {
        ui.fonts(|f| f.layout_no_wrap(text.to_string(), font, Color32::WHITE))
            .size()
    };
    let mut w = 0.0;
    for (i, keys) in combos.iter().enumerate() {
        if i > 0 {
            w += INTER_COMBO_GAP;
        }
        for (j, key) in keys.iter().enumerate() {
            if j > 0 {
                w += INTRA_COMBO_GAP;
            }
            let size = measure(key, combo_chip_font());
            let chip_h = size.y + KEYCAP_PAD_V * 2.0;
            w += (size.x + KEYCAP_PAD_H * 2.0).max(chip_h);
        }
    }
    if let Some(text) = trailing {
        w += TRAILING_GAP;
        w += measure(text, egui::FontId::proportional(style::TEXT_HINT)).x;
    }
    w
}
