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
        let width = (ctx.content_rect().width() - style::SPACE_MD * 2.0 - 32.0)
            .clamp(0.0, style::TOAST_MAX_WIDTH);
        let inner_width = (width - 34.0).max(0.0);
        egui::Area::new(egui::Id::new(self.id))
            .anchor(self.anchor, self.offset)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(colors.bg_sidebar)
                    .stroke(style::hairline_border(colors.border))
                    .corner_radius(style::RADIUS_SM)
                    .shadow(style::elevation_shadow())
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.set_max_width(inner_width);
                        ui.spacing_mut().interact_size.y = TOAST_CONTROL_H;
                        ui.spacing_mut().item_spacing.x = style::SPACE_MD;
                        ui.spacing_mut().button_padding = egui::vec2(12.0, 4.0);
                        add_contents(ui)
                    })
                    .inner
            })
    }

    /// Reserve the complete action before fitting the message. Reflow into
    /// two rows when the inline message lane becomes too small. An oversized
    /// translated action wraps with its name intact instead of eliding.
    pub(crate) fn action(
        self,
        ctx: &egui::Context,
        colors: &Colors,
        message: &str,
        label: &str,
        error: Option<&str>,
    ) -> egui::Response {
        let id = egui::Id::new(self.id).with("action");
        self.show(ctx, colors, |ui| {
            let mut capture = crate::ui::layout_report::Capture::begin(ui.painter(), "toast");
            let font = egui::FontId::proportional(style::TEXT_CAPTION);
            let max_width = ui.available_width();
            let padding = ui.spacing().button_padding;
            let action_text = ui.fonts_mut(|f| {
                f.layout(
                    label.into(),
                    font.clone(),
                    colors.text_primary,
                    (max_width - padding.x * 2.0).max(1.0),
                )
            });
            let action_size = (action_text.size() + padding * 2.0)
                .max(egui::vec2(style::BUTTON_H_MD, style::BUTTON_H_MD));
            let message_width = crate::ui::text::elided_galley(
                ui,
                message,
                font.clone(),
                max_width,
                crate::ui::text::Side::Trailing,
            )
            .size()
            .x;
            let width = (message_width + style::SPACE_MD + action_size.x).min(max_width);
            let bounds =
                egui::Rect::from_min_size(ui.cursor().min, egui::vec2(width, action_size.y));
            let slots = [crate::ui::bounded::Slot {
                id: "action",
                width: action_size.x,
                compact: None,
                required: true,
            }];
            let layout = crate::ui::bounded::allocate(
                bounds,
                style::SPACE_XL * 3.0,
                style::SPACE_MD,
                &slots,
            );
            let (message_rect, action_rect) = match layout {
                Ok(row) if row.text.width() >= style::SPACE_XL * 3.0 => {
                    debug_assert_eq!(row.slots[0].id, "action");
                    debug_assert_eq!(
                        row.slots[0].presentation,
                        crate::ui::bounded::Presentation::Full
                    );
                    (row.text, row.slots[0].rect)
                }
                _ => {
                    let message_rect = egui::Rect::from_min_size(
                        bounds.min,
                        egui::vec2(width, style::BUTTON_H_MD),
                    );
                    let action_rect = egui::Rect::from_min_size(
                        message_rect.left_bottom() + egui::vec2(0.0, style::SPACE_SM),
                        action_size,
                    );
                    if !ctx.data(|d| d.get_temp::<bool>(id.with("reflow")).unwrap_or(false)) {
                        log::info!("ui: action toast reflow width={width:.1}");
                        ctx.data_mut(|d| d.insert_temp(id.with("reflow"), true));
                    }
                    (message_rect, action_rect)
                }
            };
            ui.advance_cursor_after_rect(message_rect.union(action_rect));
            let message_galley = crate::ui::text::elided_galley(
                ui,
                message,
                font,
                message_rect.width(),
                crate::ui::text::Side::Trailing,
            );
            capture.text_slot("message", &message_galley, message_rect);
            capture.text_slot("action", &action_text, action_rect);
            ui.painter().galley(
                egui::pos2(
                    message_rect.left(),
                    message_rect.center().y - message_galley.size().y / 2.0,
                ),
                message_galley,
                colors.text_dim,
            );
            let response = ui
                .push_id(id, |ui| {
                    ui.put(
                        action_rect,
                        egui::Button::new(action_text)
                            .fill(colors.bg_active)
                            .min_size(action_rect.size()),
                    )
                })
                .inner;
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
            });
            if let Some(error) = error {
                toast_caption(ui, error, colors);
            }
            capture.finish(
                ui.painter(),
                ui.min_rect(),
                colors.bg_sidebar,
                vec![crate::ui::layout_report::Action {
                    id: "action".into(),
                    name: label.into(),
                    painted_label: label.into(),
                    hit: response.rect,
                    minimum: egui::vec2(style::BUTTON_H_MD, style::BUTTON_H_MD),
                }],
                vec![],
            );
            response
        })
        .inner
    }
}

pub(crate) fn toast_caption(ui: &mut egui::Ui, text: impl Into<String>, colors: &Colors) {
    let text = text.into();
    let fitted = crate::ui::text::elide(
        ui,
        &text,
        egui::FontId::proportional(style::TEXT_CAPTION),
        ui.available_width(),
        crate::ui::text::Side::Trailing,
    );
    ui.add(
        egui::Label::new(
            RichText::new(fitted)
                .size(style::TEXT_CAPTION)
                .color(colors.text_dim),
        )
        .wrap_mode(egui::TextWrapMode::Extend),
    )
    .on_hover_text(text);
}

pub(crate) struct ProgressDots {
    active: u8,
    total: u8,
}

impl ProgressDots {
    pub(crate) fn new(active: u8, total: u8) -> Self {
        Self { active, total }
    }

    pub(crate) fn width(&self) -> f32 {
        self.total.saturating_sub(1) as f32 * 12.0 + 8.0
    }

    pub(crate) fn show(self, ui: &mut egui::Ui, colors: &Colors) -> egui::Response {
        let width = self.width();
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
