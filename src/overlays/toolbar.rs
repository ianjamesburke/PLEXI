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
                if host_ui_gallery_available()
                    && button::toolbar_button(
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

                let help = button::icon_button(ui, "?", "Keyboard shortcuts (\u{2318}/)", &self.colors);
                record_toolbar_rect(ui, "help", help.rect);
                if help.clicked() {
                    self.show_shortcuts = !self.show_shortcuts;
                }

                let version_label = if self.update_available.is_some() {
                    RichText::new(format!("\u{2191} v{}", self.display_version))
                        .size(10.0)
                        .color(self.colors.accent)
                } else {
                    RichText::new(format!("v{}", self.display_version))
                        .size(10.0)
                        .color(self.colors.text_dim)
                };
                let hover_text = if self.update_available.is_some() {
                    "Update available — click to open changelog"
                } else {
                    "Changelog"
                };
                let version = button::toolbar_button(ui, version_label, hover_text);
                record_toolbar_rect(ui, "version", version.rect);
                if version.clicked() {
                    self.show_changelog = !self.show_changelog;
                }

                self.sync_attention_queue();
                let asks = self.attention_queue.ask_count();
                let failed = self.attention_queue.failed_count();
                let total = self.attention_queue.total_count();

                let (icon, count_str, state) =
                    crate::app::attention_queue::format_attention_mailbox(asks, failed, total);
                let color = match state {
                    crate::app::attention_queue::AttentionMailboxVisualState::Empty => {
                        self.colors.text_dim
                    }
                    crate::app::attention_queue::AttentionMailboxVisualState::AsksPresent => {
                        self.colors.zoom_needs
                    }
                    crate::app::attention_queue::AttentionMailboxVisualState::Failed => {
                        self.colors.zoom_failed
                    }
                    crate::app::attention_queue::AttentionMailboxVisualState::ResultOrNotice => {
                        self.colors.accent
                    }
                };

                let label = format!("{icon} {count_str}");
                let hover_text = if total == 0 {
                    "Attention Queue (\u{2318}I) \u{2014} Queue is calm"
                } else {
                    "Attention Queue (\u{2318}I)"
                };

                let mailbox = button::toolbar_button(
                    ui,
                    RichText::new(label)
                        .size(style::TEXT_CAPTION)
                        .color(color),
                    hover_text,
                );
                record_toolbar_rect(ui, "attention_mailbox", mailbox.rect);
                record_toolbar_rect(ui, "attention_chip", mailbox.rect);
                ui.ctx().data_mut(|d| {
                    d.insert_temp(egui::Id::new("attention_chip_rect"), mailbox.rect);
                    d.insert_temp(egui::Id::new("attention_mailbox_rect"), mailbox.rect);
                });
                if mailbox.clicked() {
                    log::info!(
                        "attention_mailbox: activated via pointer click (asks={}, failed={}, total={})",
                        asks,
                        failed,
                        total
                    );
                    self.toggle_attention_queue_list();
                }
            });
        });
    }
}

/// Publish a toolbar control's rect so layout tests can check chrome for overlap.
fn record_toolbar_rect(ui: &egui::Ui, name: &'static str, rect: egui::Rect) {
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new(("toolbar_ctl_rect", name)), rect));
}

fn host_ui_gallery_available() -> bool {
    cfg!(debug_assertions)
        || crate::config::build_channel()
            .as_deref()
            .is_some_and(|channel| channel.starts_with("pr-"))
}
