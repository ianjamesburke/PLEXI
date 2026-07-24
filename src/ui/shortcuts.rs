use egui::{Color32, CornerRadius, Pos2, RichText, Vec2};

use crate::ui::style;
use crate::ui::theme::Colors;

const INTER_COMBO_GAP: f32 = 10.0;

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
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), font_id, fg));
    let text_w = galley.size().x;
    let text_h = galley.size().y;
    let chip_h = text_h + style::KEYCHIP_PAD_V * 2.0;
    let chip_w = (text_w + style::KEYCHIP_PAD_H * 2.0)
        .max(chip_h)
        .max(style::KEYCHIP_MIN_W);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(chip_w, chip_h), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(4), colors.bg_active);
    let text_pos = Pos2::new(
        rect.center().x - text_w / 2.0,
        rect.min.y + style::KEYCHIP_PAD_V,
    );
    crate::ui::snap::galley_snapped(painter, text_pos, galley, fg);
    response
}

pub(crate) fn key_combo(ui: &mut egui::Ui, keys: &[&str], colors: &Colors) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = style::KEYCHIP_GAP;
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
    key_combo_list_with_body(ui, combos, colors, |ui| {
        if let Some(text) = trailing {
            ui.label(shortcut_hint_label(text, colors));
        }
    });
}

pub(crate) fn key_combo_list_with_body<R>(
    ui: &mut egui::Ui,
    combos: &[&[&str]],
    colors: &Colors,
    add_trailing: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, keys) in combos.iter().enumerate() {
            if i > 0 {
                ui.add_space(INTER_COMBO_GAP);
            }
            for (j, key) in keys.iter().enumerate() {
                if j > 0 {
                    ui.add_space(style::KEYCHIP_GAP);
                }
                key_chip(ui, key, colors, combo_chip_font());
            }
        }
        ui.add_space(style::KEYCHIP_DESC_GAP);
        add_trailing(ui)
    })
}

/// Paint one key chip with its left edge at `left`, vertically centered on
/// `center_y`, without allocating layout space. Returns the chip width.
/// For painter-driven rows that place their own chips (modal action rows).
pub(crate) fn key_chip_painted(
    ui: &egui::Ui,
    left: f32,
    center_y: f32,
    label: &str,
    colors: &Colors,
) -> f32 {
    let fg = shortcut_key_color(colors);
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), combo_chip_font(), fg));
    let text_w = galley.size().x;
    let chip_h = galley.size().y + style::KEYCHIP_PAD_V * 2.0;
    let chip_w = (text_w + style::KEYCHIP_PAD_H * 2.0)
        .max(chip_h)
        .max(style::KEYCHIP_MIN_W);
    let rect = egui::Rect::from_min_size(
        Pos2::new(left, center_y - chip_h / 2.0),
        Vec2::new(chip_w, chip_h),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(4), colors.bg_active);
    let text_pos = Pos2::new(
        rect.center().x - text_w / 2.0,
        rect.min.y + style::KEYCHIP_PAD_V,
    );
    crate::ui::snap::galley_snapped(painter, text_pos, galley, fg);
    chip_w
}

/// Paint a whole key combo starting at `left`, centered on `center_y`.
/// Returns the total painted width. Pairs with [`key_combo_width`], which
/// measures the same geometry without painting.
pub(crate) fn key_combo_painted(
    ui: &egui::Ui,
    left: f32,
    center_y: f32,
    keys: &[&str],
    colors: &Colors,
) -> f32 {
    let mut x = left;
    for (i, key) in keys.iter().enumerate() {
        if i > 0 {
            x += style::KEYCHIP_GAP;
        }
        x += key_chip_painted(ui, x, center_y, key, colors);
    }
    x - left
}

/// Rendered width of one combo's chips and their gaps. Callers aligning a
/// column of combos (modal action rows) measure every combo with this and
/// lay the labels out past the widest.
pub(crate) fn key_combo_width(ui: &egui::Ui, keys: &[&str]) -> f32 {
    key_combo_list_width(ui, &[keys], None)
}

pub(crate) fn key_combo_list_width(
    ui: &egui::Ui,
    combos: &[&[&str]],
    trailing: Option<&str>,
) -> f32 {
    let measure = |text: &str, font: egui::FontId| {
        ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font, Color32::WHITE))
            .size()
    };
    let mut w = 0.0;
    for (i, keys) in combos.iter().enumerate() {
        if i > 0 {
            w += INTER_COMBO_GAP;
        }
        for (j, key) in keys.iter().enumerate() {
            if j > 0 {
                w += style::KEYCHIP_GAP;
            }
            let size = measure(key, combo_chip_font());
            let chip_h = size.y + style::KEYCHIP_PAD_V * 2.0;
            w += (size.x + style::KEYCHIP_PAD_H * 2.0)
                .max(chip_h)
                .max(style::KEYCHIP_MIN_W);
        }
    }
    if let Some(text) = trailing {
        w += style::KEYCHIP_DESC_GAP;
        w += measure(text, egui::FontId::proportional(style::TEXT_HINT)).x;
    }
    w
}
