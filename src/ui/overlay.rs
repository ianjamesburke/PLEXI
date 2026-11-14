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
    /// Wrap the body in a height-capped scroll area (see `own_scroll`).
    fit_height: bool,
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
            fit_height: true,
        }
    }

    /// The body manages its own scrolling and height cap (launcher lists), so
    /// the shell must not add a scroll area that would re-key its state.
    pub(crate) fn own_scroll(mut self) -> Self {
        self.fit_height = false;
        self
    }

    /// Render the modal above other Foreground overlays (e.g. notifications
    /// that must outrank any open modal). The scrim rises with it.
    #[allow(dead_code)]
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
        self.show_frame(ctx, colors, add_body)
    }

    pub(crate) fn show_scroll_body<R>(
        self,
        ctx: &egui::Context,
        colors: &Colors,
        max_height: f32,
        id_salt: impl std::hash::Hash,
        add_body: impl FnOnce(&mut egui::Ui) -> R,
    ) -> ModalResponse {
        self.show_frame(ctx, colors, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(id_salt)
                .max_height(max_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    add_body(ui);
                });
        })
    }

    fn show_frame<R>(
        self,
        ctx: &egui::Context,
        colors: &Colors,
        add_body: impl FnOnce(&mut egui::Ui) -> R,
    ) -> ModalResponse {
        let mut dismissed = self.escape
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));

        let screen = ctx.content_rect();
        // Modal widths are preferred content widths, not permission to draw
        // outside a narrow viewport. Preserve one spacing token of edge
        // clearance in addition to the frame's horizontal padding.
        let available_content_width =
            (screen.width() - 2.0 * (style::SPACE_MD + f32::from(style::MODAL_PADDING_H))).max(0.0);
        let content_width = self.width.min(available_content_width);
        // Frame padding and edge clearance come off the screen height; the
        // title row is measured where it is painted.
        let max_frame_content_height =
            screen.height() - 2.0 * (style::SPACE_MD + f32::from(style::MODAL_PADDING_V));
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
                    // Click-away only: a focusable scrim takes Tab, and
                    // Space or Enter on it would dismiss the modal (stint 1498).
                    let clicked = ui.allocate_rect(screen, egui::Sense::CLICK).clicked();
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
                // A brand-new Area spends its first frame in an invisible
                // sizing pass, so the scrim would appear one frame before the
                // modal. Discard this pass and re-run the frame so both paint
                // together. Only fires the frame a modal opens.
                if ui.is_sizing_pass() {
                    ctx.request_discard("modal first-frame sizing pass");
                }
                egui::Frame::new()
                    .fill(colors.bg_sidebar)
                    .stroke(style::hairline_border(colors.border))
                    .corner_radius(style::RADIUS_LG)
                    .shadow(style::elevation_shadow())
                    .inner_margin(egui::Margin::symmetric(
                        style::MODAL_PADDING_H,
                        style::MODAL_PADDING_V,
                    ))
                    .show(ui, |ui| {
                        // width(0.0) = auto-size to content (toasts, pills).
                        if self.width > 0.0 {
                            ui.set_width(content_width);
                            within_width(ui, content_width, |ui| {
                                self.show_content(ui, colors, max_frame_content_height, add_body)
                            })
                        } else {
                            self.show_content(ui, colors, max_frame_content_height, add_body)
                        }
                    })
                    .inner
            });

        ModalResponse { dismissed }
    }

    fn show_content<R>(
        &self,
        ui: &mut egui::Ui,
        colors: &Colors,
        max_frame_content_height: f32,
        add_body: impl FnOnce(&mut egui::Ui) -> R,
    ) -> R {
        let fit_height = self.width > 0.0 && self.fit_height;
        if fit_height {
            // The Area's Ui is only as tall as last frame's
            // content, seeded from egui's default area size, so
            // a scroll area sized from the available height
            // never grows past that seed and clips rows the
            // window has room for. Give the frame its real
            // budget before anything is painted: `set_max_height`
            // moves the cursor to the top of the new max rect,
            // so after the title it would paint the body over it.
            ui.set_max_height(max_frame_content_height);
        }
        if let Some(title) = self.title {
            crate::ui::typography::modal_title(ui, title, colors);
            ui.add_space(style::SPACE_MD);
        }
        if fit_height {
            // A body taller than the window scrolls.
            let title_height = ui.cursor().min.y - ui.min_rect().min.y;
            let max_body_height =
                (max_frame_content_height - title_height).max(style::MODAL_BODY_MIN_H);
            if ui.is_sizing_pass() {
                log::info!(
                    "modal {:?}: body height budget {max_body_height:.1} (screen {:.1})",
                    self.title.unwrap_or("untitled"),
                    ui.ctx().content_rect().height()
                );
            }
            egui::ScrollArea::vertical()
                .id_salt("modal_body")
                .max_height(max_body_height)
                .auto_shrink([false, true])
                .show(ui, |ui| add_body(ui))
                .inner
        } else {
            add_body(ui)
        }
    }
}

/// Lay `add` out in `width`, clipping anything wider instead of letting it
/// grow the modal: a centred modal wider than the window runs off both
/// edges, footer hints included (stint 1462).
fn within_width<R>(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(width, ui.available_height()));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("modal_content")
            .max_rect(rect)
            .layout(*ui.layout()),
    );
    // Layout stays at `width`; the clip reaches into the frame's side
    // padding so a focus ring outside an edge button is drawn whole.
    let clip = ui.clip_rect();
    let pad = f32::from(style::MODAL_PADDING_H);
    child.set_clip_rect(egui::Rect::from_x_y_ranges(
        rect.x_range().expand(pad).intersection(clip.x_range()),
        clip.y_range(),
    ));
    let result = add(&mut child);
    let used = child.min_rect();
    static LOGGED: std::sync::Once = std::sync::Once::new();
    if used.width() > width + 0.5 {
        LOGGED.call_once(|| {
            log::info!(
            "modal: content {:.1}pt wide clipped to {width:.1}pt so the modal stays in the window",
            used.width()
        )
        });
    }
    ui.advance_cursor_after_rect(egui::Rect::from_min_max(
        used.min,
        egui::pos2(used.min.x + width, used.max.y),
    ));
    result
}

pub(crate) struct ModalResponse {
    pub(crate) dismissed: bool,
}
