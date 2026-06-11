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
    scrim: bool,
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
            scrim: true,
        }
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
        self.scrim = enabled;
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
        if self.scrim {
            egui::Area::new(self.id.with("scrim"))
                .fixed_pos(screen.min)
                .order(egui::Order::Middle)
                .show(ctx, |ui| {
                    ui.painter().rect_filled(
                        screen,
                        0.0,
                        Color32::from_black_alpha(style::SCRIM_ALPHA),
                    );
                    let clicked = ui.allocate_rect(screen, egui::Sense::click()).clicked();
                    if self.click_away && clicked {
                        dismissed = true;
                    }
                });
        }

        egui::Area::new(self.id.with("overlay"))
            .anchor(self.anchor, self.offset)
            .order(egui::Order::Foreground)
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
                        ui.set_width(self.width);
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
