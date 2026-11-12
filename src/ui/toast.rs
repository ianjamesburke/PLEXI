use egui::{Align2, RichText};

use crate::ui::style;
use crate::ui::theme::Colors;

pub(crate) const TOAST_CONTROL_H: f32 = style::BUTTON_H_SM;

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
                        ui.spacing_mut().item_spacing.x = style::SPACE_MD;
                        add_contents(ui)
                    })
                    .inner
            })
    }

    /// Toast actions are the default kit button (Primary, Md) on the toast's
    /// own surface. They stay Md rather than Sm (28): the layout contract
    /// reserves a 32px minimum hit target for every required action (see the
    /// `minimum` recorded in `action`), and a 28px control would violate it.
    /// The label never wraps: `action` reserves `KitButton::natural_width`.
    pub(crate) fn button<'l>(label: &'l str, colors: &Colors) -> crate::ui::button::KitButton<'l> {
        crate::ui::button::KitButton::new(label, colors).surface(colors.bg_sidebar)
    }

    /// Reserve the complete action before fitting the message. Reflow into
    /// two rows when the inline message lane becomes too small. The action
    /// keeps its natural width and never wraps; the message elides first.
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
            let action_size = egui::vec2(
                Self::button(label, colors).natural_width(ui).min(max_width),
                style::BUTTON_H_MD,
            );
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
            ui.painter().galley(
                egui::pos2(
                    message_rect.left(),
                    message_rect.center().y - message_galley.size().y / 2.0,
                ),
                message_galley,
                colors.text_dim,
            );
            let button_start = shape_count(ui);
            let response = ui
                .push_id(id, |ui| {
                    ui.put(
                        action_rect,
                        Self::button(label, colors).min_size(action_rect.size()),
                    )
                })
                .inner;
            // Record the galley the kit actually painted, not a re-measurement.
            match painted_text(ui, button_start, label) {
                Some(galley) => capture.text_slot("action", &galley, action_rect),
                None => log::error!("ui: toast action label {label:?} was not painted"),
            }
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
                    inactive: false,
                }],
                vec![],
            );
            response
        })
        .inner
    }
}

fn shape_count(ui: &egui::Ui) -> usize {
    let layer = ui.layer_id();
    ui.ctx()
        .graphics(|g| g.get(layer).map_or(0, |list| list.all_entries().len()))
}

/// The laid-out galley of the first text shape painted after `start` whose
/// text is `label`.
fn painted_text(ui: &egui::Ui, start: usize, label: &str) -> Option<std::sync::Arc<egui::Galley>> {
    let layer = ui.layer_id();
    ui.ctx().graphics(|g| {
        g.get(layer)?
            .all_entries()
            .skip(start)
            .find_map(|entry| match &entry.shape {
                egui::Shape::Text(text) if text.galley.job.text == label => {
                    Some(text.galley.clone())
                }
                _ => None,
            })
    })
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

#[cfg(test)]
mod tests {
    /// The only line that names the default kit button; the parity assertions
    /// below are independent of how it is constructed.
    fn default_kit_button(ui: &mut egui::Ui, colors: &crate::ui::theme::Colors) -> egui::Response {
        ui.add(crate::ui::button::KitButton::new("Undo", colors))
    }

    struct Painted {
        size: egui::Vec2,
        fill: egui::Color32,
        stroke: egui::epaint::Stroke,
        corner_radius: egui::CornerRadius,
        font: egui::FontId,
        text_color: egui::Color32,
    }

    fn painted(output: &egui::FullOutput, bounds: egui::Rect) -> Painted {
        let rect = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(rect) if rect.rect.contains(bounds.center()) => Some(rect),
                _ => None,
            })
            .min_by(|a, b| a.rect.area().total_cmp(&b.rect.area()))
            .expect("button background rect");
        let text = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(text)
                    if text.galley.job.text == "Undo"
                        && bounds.contains(text.visual_bounding_rect().center()) =>
                {
                    Some(text)
                }
                _ => None,
            })
            .expect("button label");
        let section = &text.galley.job.sections[0];
        Painted {
            size: rect.rect.size(),
            fill: rect.fill,
            stroke: rect.stroke.into(),
            corner_radius: rect.corner_radius,
            font: section.format.font_id.clone(),
            text_color: text.override_text_color.unwrap_or(section.format.color),
        }
    }

    /// Ian's ruling (stint 1216 round 2): the toast action never wraps. It
    /// keeps the default kit button's natural width at every screen width and
    /// message length; the message shrinks and elides instead.
    #[test]
    fn toast_action_never_wraps_and_keeps_natural_width() {
        let host = crate::testing::HostHarness::new();
        let ctx = &host.app.ctx;
        let long = "Moved a note with a very long title that cannot possibly fit beside the action to Trash";
        for name in crate::ui::theme::preset_names() {
            let colors = crate::ui::theme::Colors::from_config(
                &crate::ui::theme::preset_colors(name).unwrap(),
            );
            crate::ui::theme::setup_style(ctx, &colors);
            for screen_width in [900.0_f32, 420.0, 300.0] {
                for message in ["Moved note to Trash", long] {
                    for label in ["Undo", "Restore deleted note"] {
                        let case =
                            format!("{name} screen={screen_width} message_len={}", message.len())
                                + label;
                        let mut natural = egui::Vec2::ZERO;
                        let mut action = egui::Rect::NOTHING;
                        let mut output = None;
                        for frame in 0..4 {
                            output = Some(ctx.run_ui(
                                egui::RawInput {
                                    time: Some(frame as f64),
                                    screen_rect: Some(egui::Rect::from_min_size(
                                        egui::Pos2::ZERO,
                                        egui::vec2(screen_width, 600.0),
                                    )),
                                    ..Default::default()
                                },
                                |ui| {
                                    egui::Area::new(egui::Id::new(("natural", label)))
                                        .fixed_pos(egui::pos2(0.0, 0.0))
                                        .show(ui.ctx(), |ui| {
                                            natural = ui
                                                .add(super::ToastShell::button(label, &colors))
                                                .rect
                                                .size();
                                        });
                                    action = super::ToastShell::bottom("never_wrap")
                                        .action(ctx, &colors, message, label, None)
                                        .rect;
                                },
                            ));
                        }
                        let output = output.unwrap();
                        let galley = output
                            .shapes
                            .iter()
                            .find_map(|s| match &s.shape {
                                egui::Shape::Text(text)
                                    if text.galley.job.text == label
                                        && action
                                            .contains(text.visual_bounding_rect().center()) =>
                                {
                                    Some(text.galley.clone())
                                }
                                _ => None,
                            })
                            .unwrap_or_else(|| panic!("{case}: action label not painted"));
                        assert_eq!(
                            action.height(),
                            crate::ui::style::BUTTON_H_MD,
                            "{case}: hit height"
                        );
                        assert_eq!(galley.rows.len(), 1, "{case}: the action label wrapped");
                        assert_eq!(
                            action.width(),
                            natural.x,
                            "{case}: the action keeps its natural width"
                        );
                    }
                }
            }
        }
    }

    /// Reported mismatch (2026-11-10): the toast action button differed from
    /// the default kit button in corner radius, stroke, padding, height class,
    /// font and fill. Every one of those is asserted here for every theme
    /// preset, and again with a user theme whose preferred foreground has too
    /// little contrast.
    #[test]
    fn toast_action_matches_default_kit_button() {
        let host = crate::testing::HostHarness::new();
        let ctx = &host.app.ctx;
        for name in crate::ui::theme::preset_names() {
            for low_contrast in [false, true] {
                let mut colors = crate::ui::theme::Colors::from_config(
                    &crate::ui::theme::preset_colors(name).unwrap(),
                );
                crate::ui::theme::setup_style(ctx, &colors);
                if low_contrast {
                    // Both surfaces must run the same correction against their
                    // painted fill.
                    colors.text_primary = colors.bg_active;
                }
                let case = format!("{name} low_contrast={low_contrast}");
                let mut bounds = (egui::Rect::NOTHING, egui::Rect::NOTHING);
                let mut output = None;
                for frame in 0..3 {
                    output = Some(ctx.run_ui(
                        egui::RawInput {
                            time: Some(frame as f64),
                            ..Default::default()
                        },
                        |ui| {
                            egui::CentralPanel::default().show_inside(ui, |ui| {
                                bounds.0 = default_kit_button(ui, &colors).rect;
                            });
                            bounds.1 = super::ToastShell::bottom("parity")
                                .action(ctx, &colors, "Moved note to Trash", "Undo", None)
                                .rect;
                        },
                    ));
                }
                let output = output.unwrap();
                let (default, toast) = (painted(&output, bounds.0), painted(&output, bounds.1));
                assert_eq!(
                    bounds.1.height(),
                    crate::ui::button::ButtonSize::Md.height(),
                    "{case}: toast action keeps the Md height class (the layout contract's 32px minimum hit target)"
                );
                assert_eq!(bounds.0.height(), bounds.1.height(), "{case}: height class");
                assert_eq!(default.size, toast.size, "{case}: padding (natural size)");
                assert_eq!(
                    default.corner_radius, toast.corner_radius,
                    "{case}: corner radius"
                );
                assert_eq!(default.stroke, toast.stroke, "{case}: stroke");
                assert_eq!(default.fill, toast.fill, "{case}: fill");
                assert_eq!(
                    default.fill, colors.accent,
                    "{case}: fill source is the kit's"
                );
                assert_eq!(default.font, toast.font, "{case}: font");
                assert_eq!(
                    default.text_color, toast.text_color,
                    "{case}: contrast-corrected foreground"
                );
            }
        }
    }
}
