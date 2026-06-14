use egui::{Align2, RichText};

use crate::ui::style;
use crate::ui::theme::Colors;

pub(crate) const TOAST_CONTROL_H: f32 = 28.0;

pub(crate) struct ToastShell<'a> {
    id: &'a str,
    anchor: Align2,
    offset: egui::Vec2,
}

impl<'a> ToastShell<'a> {
    pub(crate) fn bottom(id: &'a str) -> Self {
        Self {
            id,
            anchor: Align2::CENTER_BOTTOM,
            offset: egui::vec2(0.0, -40.0),
        }
    }

    pub(crate) fn offset(mut self, offset: egui::Vec2) -> Self {
        self.offset = offset;
        self
    }

    pub(crate) fn show<R>(
        self,
        ctx: &egui::Context,
        colors: &Colors,
        add_contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> egui::InnerResponse<R> {
        egui::Area::new(egui::Id::new(self.id))
            .anchor(self.anchor, self.offset)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(colors.bg_sidebar)
                    .stroke(egui::Stroke::new(1.0, colors.border))
                    .corner_radius(style::RADIUS_SM)
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.spacing_mut().interact_size.y = TOAST_CONTROL_H;
                        ui.spacing_mut().item_spacing.x = style::SPACE_MD;
                        ui.spacing_mut().button_padding = egui::vec2(12.0, 4.0);
                        add_contents(ui)
                    })
                    .inner
            })
    }
}

pub(crate) fn toast_caption(ui: &mut egui::Ui, text: impl Into<String>, colors: &Colors) {
    ui.label(
        RichText::new(text)
            .size(style::TEXT_CAPTION)
            .color(colors.text_dim),
    );
}

pub(crate) struct ProgressDots {
    active: u8,
    total: u8,
}

impl ProgressDots {
    pub(crate) fn new(active: u8, total: u8) -> Self {
        Self { active, total }
    }

    pub(crate) fn show(self, ui: &mut egui::Ui, colors: &Colors) -> egui::Response {
        let width = self.total.saturating_sub(1) as f32 * 12.0 + 8.0;
        let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 8.0), egui::Sense::hover());

        for i in 0..self.total {
            let color = if i < self.active {
                colors.accent
            } else {
                colors.bg_active
            };
            let center = egui::pos2(rect.left() + 4.0 + i as f32 * 12.0, rect.center().y);
            ui.painter().circle_filled(center, 4.0, color);
        }

        response
    }
}
