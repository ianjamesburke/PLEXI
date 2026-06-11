use super::*;
use crate::ui::{
    hints::{HintBar, HintGroup},
    list::ListRow,
    overlay::ModalShell,
    widgets::{description_label, key_chip, selectable_row, TextField},
};

impl PlexiApp {
    pub(crate) fn draw_ui_gallery(&mut self, ctx: &egui::Context) {
        if !self.show_ui_gallery {
            return;
        }

        let colors = self.colors.clone();
        let response = ModalShell::centered("host_ui_gallery")
            .title("Host UI Gallery")
            .width(style::MODAL_WIDTH_NOTIFY)
            .escape(true)
            .show(ctx, &colors, |ui| {
                ui.label(
                    RichText::new("Chrome primitives")
                        .size(style::TEXT_HINT)
                        .color(colors.text_dim),
                );
                ui.add_space(style::SPACE_MD);

                egui::ScrollArea::vertical()
                    .max_height((ctx.screen_rect().height() - 180.0).max(280.0))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // available_width, not MODAL_WIDTH_NOTIFY — the scroll
                        // area reserves a scrollbar gutter; forcing the full
                        // modal width would push content back under the bar.
                        ui.set_width(ui.available_width());
                        gallery_section(ui, "Modal shell", &colors, |ui| {
                            token_strip(ui, &colors);
                            hint_bar(ui, &colors);
                        });

                        gallery_section(ui, "Rows", &colors, |ui| {
                            ListRow::new("Normal row")
                                .secondary("Secondary metadata")
                                .leading_chip("app")
                                .trailing_action("Open")
                                .show(ui, &colors);
                            ListRow::new("Selected row")
                                .secondary("Keyboard-selected state")
                                .leading_chip("term")
                                .trailing_action("Run")
                                .selected(true)
                                .show(ui, &colors);
                            ListRow::new("Danger row")
                                .secondary("Destructive trailing action")
                                .leading_chip("ctx")
                                .trailing_action("Delete")
                                .danger_trailing(true)
                                .show(ui, &colors);
                            let _ = selectable_row(ui, true, &colors, |ui| {
                                ui.label(
                                    RichText::new("Selectable row primitive")
                                        .size(style::TEXT_HINT)
                                        .color(colors.text_primary),
                                );
                                ui.scope(|ui| {
                                    ui.set_max_width(360.0);
                                    description_label(
                                        ui,
                                        "Truncated description label with a long path-like value",
                                        &colors,
                                    );
                                });
                            });
                        });

                        gallery_section(ui, "Text fields", &colors, |ui| {
                            TextField::singleline(
                                egui::Id::new("host_ui_gallery_text_normal"),
                                "Normal text field",
                            )
                            .show(
                                ui,
                                &mut self.ui_gallery_normal_buf,
                                &colors,
                            );
                            ui.add_space(style::SPACE_SM);
                            // No `.focused(true)` here — a hardcoded focus
                            // demo fights the user's clicks (two fields
                            // re-stealing focus from each other every frame).
                            // Focus follows clicks; the focused style shows on
                            // whichever field the user activates.
                            TextField::singleline(
                                egui::Id::new("host_ui_gallery_text_focused"),
                                "Click to focus — accent ring + cursor",
                            )
                            .log_name("ui_gallery")
                            .show(
                                ui,
                                &mut self.ui_gallery_focused_buf,
                                &colors,
                            );
                        });

                        gallery_section(ui, "Buttons and chips", &colors, |ui| {
                            ui.horizontal(|ui| {
                                button_sample(ui, "Primary", colors.accent, &colors);
                                button_sample(ui, "Neutral", colors.bg_active, &colors);
                                button_sample(ui, "Danger", colors.danger, &colors);
                                ui.add_enabled(
                                    false,
                                    egui::Button::new(
                                        RichText::new("Disabled")
                                            .size(style::TEXT_BODY)
                                            .color(colors.text_dim),
                                    )
                                    .min_size(egui::vec2(80.0, style::BUTTON_H_MD)),
                                );
                            });
                            ui.add_space(style::SPACE_SM);
                            ui.horizontal(|ui| {
                                key_chip(
                                    ui,
                                    "app",
                                    &colors,
                                    egui::FontId::monospace(style::TEXT_HINT),
                                );
                                key_chip(
                                    ui,
                                    "term",
                                    &colors,
                                    egui::FontId::monospace(style::TEXT_HINT),
                                );
                                status_chip(ui, "running", colors.accent, &colors);
                                status_chip(ui, "error", colors.danger, &colors);
                                status_chip(ui, "empty", colors.text_dim, &colors);
                            });
                        });

                        gallery_section(ui, "Empty states", &colors, |ui| {
                            egui::Frame::new()
                                .fill(colors.bg_toolbar)
                                .stroke(egui::Stroke::new(1.0, colors.border))
                                .corner_radius(style::RADIUS_MD)
                                .inner_margin(egui::Margin::symmetric(12, 10))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            RichText::new("No matching items")
                                                .size(style::TEXT_BODY)
                                                .color(colors.text_primary),
                                        );
                                        ui.label(
                                            RichText::new(
                                                "Empty chrome should stay compact and quiet.",
                                            )
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                        );
                                    });
                                });
                        });
                    });
            });

        if response.dismissed {
            self.show_ui_gallery = false;
            log::info!("ui_gallery: closed by escape or click-away");
        }
    }
}

fn gallery_section(
    ui: &mut egui::Ui,
    title: &str,
    colors: &crate::ui::theme::Colors,
    body: impl FnOnce(&mut egui::Ui),
) {
    ui.label(
        RichText::new(title)
            .size(style::TEXT_CAPTION)
            .color(colors.accent),
    );
    ui.add_space(style::SPACE_XS);
    body(ui);
    ui.add_space(style::SPACE_XL);
}

fn token_strip(ui: &mut egui::Ui, colors: &crate::ui::theme::Colors) {
    ui.horizontal(|ui| {
        color_swatch(ui, "toolbar", colors.bg_toolbar, colors);
        color_swatch(ui, "active", colors.bg_active, colors);
        color_swatch(ui, "accent", colors.accent, colors);
        color_swatch(ui, "danger", colors.danger, colors);
        color_swatch(ui, "border", colors.border, colors);
    });
}

fn color_swatch(
    ui: &mut egui::Ui,
    label: &str,
    fill: egui::Color32,
    colors: &crate::ui::theme::Colors,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(84.0, 30.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, style::RADIUS_MD, fill);
    ui.painter().rect_stroke(
        rect,
        style::RADIUS_MD,
        egui::Stroke::new(1.0, colors.border),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(style::TEXT_HINT),
        colors.text_primary,
    );
}

fn hint_bar(ui: &mut egui::Ui, colors: &crate::ui::theme::Colors) {
    // The real modal-footer treatment: each label attached to its combo,
    // centered, divider above.
    let hints = [
        HintGroup::new(&["\u{2318}", "P"], "palette"),
        HintGroup::new(&["\u{2318}", "/"], "help"),
        HintGroup::new(&["esc"], "dismiss"),
    ];
    HintBar::new(&hints).show(ui, colors);
}

fn button_sample(
    ui: &mut egui::Ui,
    label: &str,
    fill: egui::Color32,
    colors: &crate::ui::theme::Colors,
) {
    let button = egui::Button::new(
        RichText::new(label)
            .size(style::TEXT_BODY)
            .color(colors.text_primary),
    )
    .fill(fill)
    .stroke(egui::Stroke::new(1.0, colors.border))
    .min_size(egui::vec2(80.0, style::BUTTON_H_MD));
    ui.add(button);
}

fn status_chip(
    ui: &mut egui::Ui,
    label: &str,
    color: egui::Color32,
    colors: &crate::ui::theme::Colors,
) {
    let text = RichText::new(label).size(style::TEXT_HINT).color(color);
    egui::Frame::new()
        .fill(colors.bg_active)
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(style::RADIUS_BADGE)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(text);
        });
}
