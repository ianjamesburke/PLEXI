use super::*;
use crate::ui::button;

impl PlexiApp {
    pub(crate) fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Workspace dots
            let sidebar_contexts: Vec<usize> = (0..self.router.len()).collect();
            if sidebar_contexts.len() > 1 {
                let dot_radius = 4.0;
                let dot_spacing = 12.0;
                let total_width = (sidebar_contexts.len() as f32) * dot_spacing;
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(total_width, ui.available_height()),
                    egui::Sense::hover(),
                );
                let y = rect.center().y;
                let start_x = rect.left() + dot_radius;
                for (dot_i, ctx_i) in sidebar_contexts.iter().enumerate() {
                    let cx = start_x + (dot_i as f32) * dot_spacing;
                    let color = if *ctx_i == self.router.active_idx() {
                        self.colors.accent
                    } else {
                        self.colors.bg_active
                    };
                    ui.painter()
                        .circle_filled(egui::pos2(cx, y), dot_radius, color);
                }
                ui.add_space(4.0);
            }

            // Right side — help button + notification badge
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if host_ui_gallery_available() {
                    if button::toolbar_button(
                        ui,
                        RichText::new("UI")
                            .size(style::TEXT_CAPTION)
                            .color(self.colors.text_dim),
                        "Host UI gallery",
                    )
                    .clicked()
                    {
                        self.show_ui_gallery = !self.show_ui_gallery;
                        log::info!(
                            "ui_gallery: {} from debug toolbar",
                            if self.show_ui_gallery {
                                "opened"
                            } else {
                                "closed"
                            }
                        );
                    }
                }

                if button::icon_button(ui, "?", "Keyboard shortcuts (\u{2318}/)", &self.colors)
                    .clicked()
                {
                    self.show_shortcuts = !self.show_shortcuts;
                }

                let version_label = if self.update_available.is_some() {
                    RichText::new(format!("\u{2191} v{}", env!("CARGO_PKG_VERSION")))
                        .size(10.0)
                        .color(self.colors.accent)
                } else {
                    RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                        .size(10.0)
                        .color(self.colors.text_dim)
                };
                let hover_text = if self.update_available.is_some() {
                    "Update available — click to open changelog"
                } else {
                    "Changelog"
                };
                if button::toolbar_button(ui, version_label, hover_text).clicked() {
                    self.show_changelog = !self.show_changelog;
                }

                let notif_count = self.visible_notification_count();
                if notif_count > 0 {
                    let badge_text = if notif_count > 9 {
                        "9+".to_string()
                    } else {
                        notif_count.to_string()
                    };
                    if button::toolbar_button(
                        ui,
                        RichText::new(format!("\u{1F514} {badge_text}"))
                            .size(style::TEXT_CAPTION)
                            .color(self.colors.accent),
                        "Notifications (\u{2318}\u{21E7}A)",
                    )
                    .clicked()
                    {
                        self.show_notification_modal = !self.show_notification_modal;
                        if self.show_notification_modal && self.current_notify_id.is_none() {
                            self.current_notify_id = self.select_highest_priority();
                        }
                    }
                }
            });
        });
    }
}

fn host_ui_gallery_available() -> bool {
    cfg!(debug_assertions)
        || crate::config::build_channel()
            .as_deref()
            .is_some_and(|channel| channel.starts_with("pr-"))
}
