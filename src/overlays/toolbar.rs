use super::*;
use crate::app_protocol::UiNode;

/// Build a `UiNode::Text` for the toolbar version label.
///
/// `update_available` controls whether the label renders with the accent
/// colour and an up-arrow prefix (update pending) or the dim colour and a
/// plain `vX.Y.Z` format (up-to-date).
///
/// Extracted so the node construction is testable without a live egui context.
pub(crate) fn build_version_label_node(
    version: &str,
    update_available: bool,
    colors: &crate::theme::Colors,
) -> UiNode {
    let color_hex = |c: egui::Color32| {
        format!("#{:02x}{:02x}{:02x}{:02x}", c.r(), c.g(), c.b(), c.a())
    };
    let (text, color) = if update_available {
        (
            format!("\u{2191} v{version}"),
            color_hex(colors.accent),
        )
    } else {
        (
            format!("v{version}"),
            color_hex(colors.text_dim),
        )
    };
    UiNode::Text {
        text,
        size: 10.0,
        color,
        bold: false,
        monospace: false,
    }
}

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
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("?").size(12.0).color(self.colors.text_dim),
                        )
                        .frame(false)
                        .min_size(egui::vec2(24.0, 0.0)),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Keyboard shortcuts (\u{2318}/)")
                    .clicked()
                {
                    self.show_shortcuts = !self.show_shortcuts;
                }

                let update_avail = self.update_available.is_some();
                let version_node =
                    build_version_label_node(env!("CARGO_PKG_VERSION"), update_avail, &self.colors);
                let version_rich = if let UiNode::Text { text, size, color, bold, .. } = &version_node {
                    let mut r = RichText::new(text.as_str()).size(*size);
                    if let Some(c) = crate::process_app::render::parse_color(color) {
                        r = r.color(c);
                    }
                    if *bold { r = r.strong(); }
                    r
                } else {
                    RichText::new(format!("v{}", env!("CARGO_PKG_VERSION"))).size(10.0)
                };
                let hover_text = if update_avail {
                    "Update available — click to open changelog"
                } else {
                    "Changelog"
                };
                if ui
                    .add(egui::Button::new(version_rich).frame(false))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(hover_text)
                    .clicked()
                {
                    self.show_changelog = !self.show_changelog;
                }

                let notif_count = self.visible_notification_count();
                if notif_count > 0 {
                    let badge_text = if notif_count > 9 {
                        "9+".to_string()
                    } else {
                        notif_count.to_string()
                    };
                    let btn = egui::Button::new(
                        RichText::new(format!("\u{1F514} {badge_text}"))
                            .size(12.0)
                            .color(self.colors.accent),
                    )
                    .frame(false);
                    if ui
                        .add(btn)
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("Notifications (\u{2318}\u{21E7}A)")
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod toolbar_component_tree_tests {
    use super::*;
    use crate::config::ThemeConfig;
    use crate::theme::Colors;

    fn test_colors() -> Colors {
        Colors::from_config(&ThemeConfig::default())
    }

    /// Up-to-date: label is `v1.2.3`, color is text_dim, no up-arrow.
    #[test]
    fn version_label_node_up_to_date() {
        let colors = test_colors();
        let node = build_version_label_node("1.2.3", false, &colors);
        if let UiNode::Text { text, size, bold, monospace, .. } = node {
            assert_eq!(text, "v1.2.3");
            assert_eq!(size, 10.0);
            assert!(!bold);
            assert!(!monospace);
        } else {
            panic!("expected UiNode::Text");
        }
    }

    /// Update available: label prefixed with ↑ and uses accent color.
    #[test]
    fn version_label_node_update_available() {
        let colors = test_colors();
        let node = build_version_label_node("2.0.0", true, &colors);
        if let UiNode::Text { text, size, color, bold, monospace } = node {
            assert!(text.contains("2.0.0"), "version must appear in label");
            assert!(text.contains('\u{2191}'), "up-arrow must appear when update is available");
            assert_eq!(size, 10.0);
            assert!(!bold);
            assert!(!monospace);
            // Color must be the accent — non-empty hex string.
            assert!(!color.is_empty(), "color must be set for update-available state");
        } else {
            panic!("expected UiNode::Text");
        }
    }
}
