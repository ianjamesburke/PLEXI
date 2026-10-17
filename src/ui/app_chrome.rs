use egui::{Color32, FontId, Response, RichText, Ui};

use crate::host::wasm_app::bindings::plexi::platform::types::FooterKeyEntry;
use crate::ui::hints::{HintBar, HintGroup};
use crate::ui::style;
use crate::ui::theme::{self, Colors};

const APP_BAR_TITLE_SIZE: f32 = style::TEXT_TITLE;
const APP_BAR_SINGLE_BAND_H: f32 = 34.0;
const APP_BAR_DOUBLE_BAND_H: f32 = 48.0;

pub(crate) struct AppChrome<'a> {
    colors: &'a Colors,
}

impl<'a> AppChrome<'a> {
    pub(crate) fn new(colors: &'a Colors) -> Self {
        Self { colors }
    }

    pub(crate) fn toolbar_fill(&self) -> Color32 {
        self.colors.bg_toolbar
    }

    pub(crate) fn border(&self) -> Color32 {
        self.colors.border
    }

    // Arg-struct refactor is a design change tracked in stint 0661.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn text_label(
        &self,
        ui: &mut Ui,
        text: &str,
        size: f32,
        color: Color32,
        bold: bool,
        monospace: bool,
        wrap: bool,
    ) -> Response {
        let mut rich = RichText::new(text).size(size).color(color);
        if monospace {
            rich = rich.monospace();
            if bold {
                rich = rich.strong();
            }
        } else if bold {
            rich = rich.font(theme::font_medium(size));
        }
        let mut label = egui::Label::new(rich).selectable(true);
        if wrap {
            label = label.wrap();
        }
        ui.add(label)
    }

    pub(crate) fn paint_app_bar(&self, ui: &mut Ui, title: &str, subtitle: &str) -> egui::Rect {
        let has_subtitle = !subtitle.is_empty();
        let band_h = app_bar_band_height(has_subtitle);
        let total_h = app_bar_height(subtitle);
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), total_h),
            egui::Sense::hover(),
        );
        let clip = ui.clip_rect();
        let full_rect = egui::Rect::from_min_size(
            egui::pos2(clip.min.x, rect.min.y),
            egui::vec2(clip.width(), total_h),
        );
        let painter = ui.painter();
        painter.rect_filled(full_rect, 0.0, self.toolbar_fill());

        let text_x = full_rect.min.x + style::SPACE_MD;
        let max_w = (full_rect.width() - 2.0 * style::SPACE_MD).max(0.0);
        let text_clip = egui::Rect::from_min_size(
            egui::pos2(text_x, full_rect.min.y),
            egui::vec2(max_w, band_h),
        );
        let text_painter = painter.with_clip_rect(text_clip);

        if has_subtitle {
            let block_h = APP_BAR_TITLE_SIZE + style::SPACE_XS + style::TEXT_HINT;
            let title_y = full_rect.min.y + (band_h - block_h).max(0.0) / 2.0;
            self.paint_no_wrap_text(
                ui,
                &text_painter,
                egui::pos2(text_x, title_y),
                title,
                theme::font_medium(APP_BAR_TITLE_SIZE),
                self.colors.text_primary,
            );
            self.paint_no_wrap_text(
                ui,
                &text_painter,
                egui::pos2(text_x, title_y + APP_BAR_TITLE_SIZE + style::SPACE_XS),
                subtitle,
                FontId::proportional(style::TEXT_HINT),
                self.colors.text_secondary(self.colors.bg_toolbar),
            );
        } else {
            let title_y = full_rect.min.y + (band_h - APP_BAR_TITLE_SIZE).max(0.0) / 2.0;
            self.paint_no_wrap_text(
                ui,
                &text_painter,
                egui::pos2(text_x, title_y),
                title,
                theme::font_medium(APP_BAR_TITLE_SIZE),
                self.colors.text_primary,
            );
        }

        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(full_rect.min.x, full_rect.min.y + band_h),
                egui::vec2(full_rect.width(), 1.0),
            ),
            0.0,
            self.border(),
        );
        full_rect
    }

    pub(crate) fn paint_footer_keys(&self, ui: &mut Ui, entries: &[FooterKeyEntry], divider: bool) {
        let groups: Vec<HintGroup<'_>> = entries
            .iter()
            .map(|e| HintGroup::from_strings(&e.keys, &e.description))
            .collect();
        HintBar::new(&groups)
            .pane_footer_band(divider)
            .show(ui, self.colors);
    }

    fn paint_no_wrap_text(
        &self,
        ui: &Ui,
        painter: &egui::Painter,
        pos: egui::Pos2,
        text: &str,
        font: FontId,
        color: Color32,
    ) {
        let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font, color));
        crate::ui::snap::galley_snapped(painter, pos, galley, color);
    }
}

pub(crate) fn app_bar_height(subtitle: &str) -> f32 {
    app_bar_band_height(!subtitle.is_empty()) + 1.0
}

fn app_bar_band_height(has_subtitle: bool) -> f32 {
    if has_subtitle {
        APP_BAR_DOUBLE_BAND_H
    } else {
        APP_BAR_SINGLE_BAND_H
    }
}

#[cfg(test)]
pub(crate) fn chip_row_height(ui: &egui::Ui) -> f32 {
    crate::ui::hints::hint_row_height(ui)
}

/// Height of the footer-keys band, accounting for wrapping: in a pane too
/// narrow to hold every entry on one line, entries flow onto additional rows and
/// the band grows to fit (#2240). `ui.available_width()` is the same width the
/// paint pass lays out against, so the host-measured height the `Column` /
/// bottom-pin allocator reserves always matches what gets painted.
pub(crate) fn footer_keys_height(ui: &egui::Ui, entries: &[FooterKeyEntry], divider: bool) -> f32 {
    let groups: Vec<HintGroup<'_>> = entries
        .iter()
        .map(|e| HintGroup::from_strings(&e.keys, &e.description))
        .collect();
    HintBar::new(&groups).band_height(ui, divider)
}
