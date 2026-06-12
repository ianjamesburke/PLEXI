use egui::{Align2, Color32, Vec2};

use crate::ui::style;
use crate::ui::theme::Colors;

pub(crate) struct ModalShell<'a> {
    id: egui::Id,
    title: Option<&'a str>,
    width: f32,
    anchor: Align2,
    offset: Vec2,
    click_away: bool,
    escape: bool,
    /// Scrim darkness, 0.0..=1.0. 0.0 = no scrim (and no click-away
    /// detection); 1.0 = pure black. Default maps `style::SCRIM_ALPHA`.
    scrim_strength: f32,
    order: egui::Order,
}

impl<'a> ModalShell<'a> {
    pub(crate) fn centered(id: impl std::hash::Hash) -> Self {
        Self {
            id: egui::Id::new(id),
            title: None,
            width: style::MODAL_WIDTH_MD,
            anchor: Align2::CENTER_CENTER,
            offset: Vec2::ZERO,
            click_away: true,
            escape: false,
            scrim_strength: f32::from(style::SCRIM_ALPHA) / 255.0,
            order: egui::Order::Foreground,
        }
    }

    /// Render the modal above other Foreground overlays (e.g. notifications
    /// that must outrank any open modal). The scrim rises with it.
    pub(crate) fn order(mut self, order: egui::Order) -> Self {
        self.order = order;
        self
    }

    /// Re-anchor the modal (e.g. `Align2::CENTER_TOP` + y-offset for input
    /// popovers that shouldn't cover the workspace center).
    pub(crate) fn anchor(mut self, anchor: Align2, offset: Vec2) -> Self {
        self.anchor = anchor;
        self.offset = offset;
        self
    }

    /// Disable the dimming scrim. Quick inline popovers (rename, edit
    /// description) keep the workspace visible. Note: click-away dismissal
    /// is detected via the scrim, so scrim-less modals dismiss only via
    /// Escape/Enter handled by the caller or `.escape(true)`.
    pub(crate) fn scrim(mut self, enabled: bool) -> Self {
        self.scrim_strength = if enabled {
            f32::from(style::SCRIM_ALPHA) / 255.0
        } else {
            0.0
        };
        self
    }

    /// Set scrim darkness explicitly (0.0 = none, 1.0 = black). Use for
    /// modals that should dim more or less than the product default.
    pub(crate) fn scrim_strength(mut self, strength: f32) -> Self {
        self.scrim_strength = strength.clamp(0.0, 1.0);
        self
    }

    pub(crate) fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    pub(crate) fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub(crate) fn escape(mut self, enabled: bool) -> Self {
        self.escape = enabled;
        self
    }

    pub(crate) fn click_away(mut self, enabled: bool) -> Self {
        self.click_away = enabled;
        self
    }

    pub(crate) fn show<R>(
        self,
        ctx: &egui::Context,
        colors: &Colors,
        add_body: impl FnOnce(&mut egui::Ui) -> R,
    ) -> ModalResponse {
        let mut dismissed = self.escape
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));

        let screen = ctx.screen_rect();
        if self.scrim_strength > 0.0 {
            let alpha = (self.scrim_strength * 255.0).round() as u8;
            // Scrim sits one layer below the modal so an elevated modal
            // (Order::Tooltip) still dims ordinary Foreground overlays.
            let scrim_order = if self.order == egui::Order::Foreground {
                egui::Order::Middle
            } else {
                egui::Order::Foreground
            };
            egui::Area::new(self.id.with("scrim"))
                .fixed_pos(screen.min)
                .order(scrim_order)
                // fade_in(false): overlays must appear instantly — egui's
                // default fade reads as input lag on launcher-style modals.
                .fade_in(false)
                .show(ctx, |ui| {
                    ui.painter()
                        .rect_filled(screen, 0.0, Color32::from_black_alpha(alpha));
                    let clicked = ui.allocate_rect(screen, egui::Sense::click()).clicked();
                    if self.click_away && clicked {
                        dismissed = true;
                    }
                });
        }

        egui::Area::new(self.id.with("overlay"))
            .anchor(self.anchor, self.offset)
            .order(self.order)
            .fade_in(false)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(colors.bg_sidebar)
                    .stroke(egui::Stroke::new(1.0, colors.border))
                    .corner_radius(style::RADIUS_LG)
                    .shadow(egui::Shadow {
                        offset: [0, 12],
                        blur: 32,
                        spread: 0,
                        color: Color32::from_black_alpha(96),
                    })
                    .inner_margin(egui::Margin::symmetric(
                        style::MODAL_PADDING_H,
                        style::MODAL_PADDING_V,
                    ))
                    .show(ui, |ui| {
                        // width(0.0) = auto-size to content (toasts, pills).
                        if self.width > 0.0 {
                            ui.set_width(self.width);
                        }
                        if let Some(title) = self.title {
                            ui.label(
                                egui::RichText::new(title)
                                    .font(crate::ui::theme::font_medium(style::TEXT_TITLE))
                                    .color(colors.text_primary),
                            );
                            ui.add_space(style::SPACE_MD);
                        }
                        add_body(ui)
                    })
                    .inner
            });

        ModalResponse { dismissed }
    }
}

pub(crate) struct ModalResponse {
    pub(crate) dismissed: bool,
}
