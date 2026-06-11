use super::*;
use crate::ui::{
    hints::{HintBar, HintGroup},
    list::ListRow,
    overlay::ModalShell,
    widgets::{
        chrome_button, chrome_section, color_swatch, description_label, empty_state_panel,
        key_chip, selectable_row, status_chip, ButtonKind, TextField,
    },
};

impl PlexiApp {
    pub(crate) fn draw_ui_gallery(&mut self, ctx: &egui::Context) {
        if !self.show_ui_gallery {
            return;
        }

        let colors = self.colors.clone();
        // Escape is gated off while the nested text-entry demo is open so the
        // nested modal consumes it first — otherwise one press closes both.
        let response = ModalShell::centered("host_ui_gallery")
            .title("Host UI Gallery")
            .width(style::MODAL_WIDTH_NOTIFY)
            .escape(!self.ui_gallery_show_text_modal)
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
                        chrome_section(ui, "Modal shell", &colors, |ui| {
                            token_strip(ui, &colors);
                            hint_bar(ui, &colors);
                        });

                        chrome_section(ui, "Rows", &colors, |ui| {
                            ListRow::new("Normal row")
                                .secondary("Secondary metadata")
                                .chip("app")
                                .trailing_action("Open")
                                .show(ui, &colors);
                            ListRow::new("Selected row")
                                .secondary("Keyboard-selected state")
                                .chip("term")
                                .trailing_action("Run")
                                .selected(true)
                                .show(ui, &colors);
                            ListRow::new("Danger row")
                                .secondary("Destructive trailing action")
                                .chip("ctx")
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

                        chrome_section(ui, "Text fields", &colors, |ui| {
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

                        chrome_section(ui, "Modal patterns", &colors, |ui| {
                            if chrome_button(
                                ui,
                                "Open text-entry modal",
                                ButtonKind::Primary,
                                &colors,
                                180.0,
                            )
                            .clicked()
                            {
                                self.ui_gallery_show_text_modal = true;
                                log::info!("ui_gallery: opened text-entry modal demo");
                            }
                        });

                        chrome_section(ui, "Buttons and chips", &colors, |ui| {
                            ui.horizontal(|ui| {
                                chrome_button(ui, "Primary", ButtonKind::Primary, &colors, 80.0);
                                chrome_button(
                                    ui,
                                    "Secondary",
                                    ButtonKind::Secondary,
                                    &colors,
                                    92.0,
                                );
                                chrome_button(ui, "Danger", ButtonKind::Danger, &colors, 80.0);
                                ui.add_enabled_ui(false, |ui| {
                                    chrome_button(
                                        ui,
                                        "Disabled",
                                        ButtonKind::Secondary,
                                        &colors,
                                        80.0,
                                    );
                                });
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
                                status_chip(ui, "running", &colors);
                                status_chip(ui, "error", &colors);
                                status_chip(ui, "empty", &colors);
                            });
                        });

                        chrome_section(ui, "Empty states", &colors, |ui| {
                            empty_state_panel(
                                ui,
                                "No matching items",
                                Some("Empty chrome should stay compact and quiet."),
                                &colors,
                            );
                        });
                    });
            });

        // Ignore gallery click-away while the nested demo is open — a click
        // inside the nested modal must not tear down the gallery under it.
        if response.dismissed && !self.ui_gallery_show_text_modal {
            self.show_ui_gallery = false;
            log::info!("ui_gallery: closed by escape or click-away");
        }

        if self.ui_gallery_show_text_modal {
            let entry = ModalShell::centered("host_ui_gallery_text_modal")
                .title("Text entry")
                .width(style::MODAL_WIDTH_MD)
                .escape(true)
                .show(ctx, &colors, |ui| {
                    TextField::singleline(
                        egui::Id::new("host_ui_gallery_modal_field"),
                        "Type something...",
                    )
                    .focused(true)
                    .log_name("ui_gallery_modal")
                    .show(ui, &mut self.ui_gallery_modal_buf, &colors);
                    let hints = [
                        HintGroup::new(&["\u{23ce}"], "save"),
                        HintGroup::new(&["esc"], "dismiss"),
                    ];
                    HintBar::new(&hints).show(ui, &colors);
                });
            let enter = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            if entry.dismissed || enter {
                self.ui_gallery_show_text_modal = false;
                log::info!("ui_gallery: text-entry modal demo closed");
            }
        }
    }
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
