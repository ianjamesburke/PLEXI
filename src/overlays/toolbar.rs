use super::*;
use crate::ui::button;

impl PlexiApp {
    pub(crate) fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        let row = ui.max_rect();
        let mailbox_rect = ui.horizontal(|ui| {
            // Right side — help button + notification badge
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if host_ui_gallery_available() {
                    let gallery = button::toolbar_button(
                        ui,
                        RichText::new("UI")
                            .size(style::TEXT_CAPTION)
                            .color(self.colors.text_dim),
                        "Host UI gallery",
                    );
                    record_toolbar_rect(ui, "gallery", gallery.rect);
                    if gallery.clicked() {
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
                        self.colors.zoom_notice
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
                    d.insert_temp(egui::Id::new("attention_mailbox_color"), color);
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
                mailbox.rect
            }).inner
        }).inner;

        #[cfg(target_os = "macos")]
        {
            let right = mailbox_rect.left() - ui.spacing().item_spacing.x;
            let available = (right - row.left()).max(0.0);
            let font = crate::ui::theme::font_medium(style::TEXT_CAPTION);
            let title = self.last_sent_window_title.as_deref().unwrap_or("");
            let visible = crate::ui::text::elide(
                ui,
                title,
                font.clone(),
                available,
                crate::ui::text::Side::Trailing,
            );
            let galley = ui.fonts_mut(|fonts| {
                fonts.layout_no_wrap(visible.clone(), font, egui::Color32::PLACEHOLDER)
            });
            let rect = titlebar_title_rect(
                row,
                ui.ctx().content_rect().center().x,
                right,
                galley.size(),
            );
            if let Some(rect) = rect {
                ui.painter().galley(rect.min, galley, self.colors.text_dim);
            }
            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    egui::Id::new("titlebar_title_rect"),
                    rect.unwrap_or(egui::Rect::NOTHING),
                );
                d.insert_temp(egui::Id::new("titlebar_visible_title"), visible);
            });
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (row, mailbox_rect);
    }
}

#[cfg(target_os = "macos")]
fn titlebar_title_rect(
    row: egui::Rect,
    center_x: f32,
    right: f32,
    size: egui::Vec2,
) -> Option<egui::Rect> {
    if size.x <= 0.0 || right - row.left() < size.x {
        return None;
    }
    let left = (center_x - size.x / 2.0).clamp(row.left(), right - size.x);
    Some(egui::Rect::from_min_size(
        egui::pos2(left, row.center().y - size.y / 2.0),
        size,
    ))
}

#[cfg(all(test, target_os = "macos"))]
mod titlebar_tests {
    use super::*;

    #[test]
    fn titlebar_centers_then_shifts_and_hides() {
        let row = egui::Rect::from_min_max(egui::pos2(80.0, 0.0), egui::pos2(800.0, 28.0));
        let size = egui::vec2(200.0, 12.0);
        let centered = titlebar_title_rect(row, 400.0, 700.0, size).unwrap();
        assert_eq!(centered.center().x, 400.0);
        let shifted = titlebar_title_rect(row, 400.0, 450.0, size).unwrap();
        assert_eq!(shifted.right(), 450.0);
        assert!(titlebar_title_rect(row, 400.0, 200.0, size).is_none());
        assert!(titlebar_title_rect(row, 400.0, 700.0, egui::Vec2::ZERO).is_none());
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
