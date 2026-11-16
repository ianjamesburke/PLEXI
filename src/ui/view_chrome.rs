use egui::{Color32, FontId, Response, RichText, Ui};

use crate::host::wasm_view::bindings::plexi::platform::types::FooterKeyEntry;
use crate::ui::hints::{HintBar, HintGroup};
use crate::ui::style;
use crate::ui::theme::{self, Colors};

const APP_BAR_TITLE_SIZE: f32 = style::TEXT_TITLE;
const APP_BAR_BAND_H: f32 = APP_BAR_TITLE_SIZE + 2.0 * style::SPACE_XS;

pub(crate) struct ViewChrome<'a> {
    colors: &'a Colors,
}

#[derive(Clone, Copy)]
pub(crate) struct TextLabelStyle {
    pub size: f32,
    pub color: Color32,
    pub bold: bool,
    pub monospace: bool,
    pub wrap: bool,
}

impl<'a> ViewChrome<'a> {
    pub(crate) fn new(colors: &'a Colors) -> Self {
        Self { colors }
    }

    pub(crate) fn toolbar_fill(&self) -> Color32 {
        self.colors.bg_toolbar
    }

    pub(crate) fn border(&self) -> Color32 {
        self.colors.border
    }

    pub(crate) fn text_label(&self, ui: &mut Ui, text: &str, style: TextLabelStyle) -> Response {
        let TextLabelStyle {
            size,
            color,
            bold,
            monospace,
            wrap,
        } = style;
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

    pub(crate) fn paint_view_bar(&self, ui: &mut Ui, title: &str, subtitle: &str) -> egui::Rect {
        let has_subtitle = !subtitle.is_empty();
        let band_h = APP_BAR_BAND_H;
        static TRACE: std::sync::Once = std::sync::Once::new();
        TRACE.call_once(|| log::info!("ui: compact surface-header band height={APP_BAR_BAND_H}"));
        let total_h = view_bar_height(subtitle);
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), total_h),
            egui::Sense::hover(),
        );
        let clip = ui.clip_rect();
        let full_rect = egui::Rect::from_min_size(
            egui::pos2(clip.min.x, rect.min.y),
            egui::vec2(clip.width(), total_h),
        );
        let mut painter = ui.painter().clone();
        painter.set_opacity(1.0);
        let mut capture = super::layout_report::Capture::begin(&painter, "surface_header");
        painter.rect_filled(full_rect, 0.0, self.toolbar_fill());

        let text_x = full_rect.min.x + style::SPACE_MD;
        let max_w = (full_rect.width() - 2.0 * style::SPACE_MD).max(0.0);
        let text_clip = egui::Rect::from_min_size(
            egui::pos2(text_x, full_rect.min.y),
            egui::vec2(max_w, band_h),
        );
        // A long title yields half the row to optional status; a short title
        // leaves all remaining space to it. Both labels use measured elision.
        let title_galley = super::text::elided_galley(
            ui,
            title,
            theme::font_medium(APP_BAR_TITLE_SIZE),
            if has_subtitle { max_w / 2.0 } else { max_w },
            super::text::Side::Trailing,
        );
        let title_rect =
            egui::Rect::from_min_size(text_clip.min, egui::vec2(title_galley.size().x, band_h));
        let subtitle_left = (title_rect.right() + style::SPACE_SM).min(text_clip.right());
        let subtitle_rect =
            egui::Rect::from_min_max(egui::pos2(subtitle_left, text_clip.top()), text_clip.max);
        for (id, text, font, color, bounds) in [
            (
                "title",
                title,
                theme::font_medium(APP_BAR_TITLE_SIZE),
                self.colors.text_primary,
                title_rect,
            ),
            (
                "subtitle",
                subtitle,
                FontId::proportional(style::TEXT_HINT),
                self.colors.text_secondary(self.toolbar_fill()),
                subtitle_rect,
            ),
        ] {
            if text.is_empty() {
                continue;
            }
            let Ok(lane) = super::bounded::allocate(bounds, 0.0, 0.0, &[]) else {
                log::info!("ui: insufficient surface-header text space");
                continue;
            };
            let galley = super::text::elided_galley(
                ui,
                text,
                font,
                lane.text.width(),
                super::text::Side::Trailing,
            );
            capture.text_slot(id, &galley, lane.text);
            let pos = egui::pos2(
                lane.text.left(),
                lane.text.center().y - galley.size().y / 2.0,
            );
            let color = super::foreground::Surface::over(self.toolbar_fill(), self.toolbar_fill())
                .foreground(color, super::foreground::Role::SmallText);
            crate::ui::snap::galley_snapped(&painter.with_clip_rect(lane.text), pos, galley, color);
        }

        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(full_rect.min.x, full_rect.min.y + band_h),
                egui::vec2(full_rect.width(), 1.0),
            ),
            0.0,
            self.border(),
        );
        capture.finish(&painter, full_rect, self.toolbar_fill(), vec![], vec![]);
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
}

pub(crate) fn view_bar_height(_subtitle: &str) -> f32 {
    APP_BAR_BAND_H + 1.0
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

#[cfg(test)]
mod adoption_tests {
    use super::*;
    #[test]
    fn surface_header_compact_height_is_independent_of_subtitle() {
        let compact = style::TEXT_TITLE + 2.0 * style::SPACE_XS + 1.0;
        assert_eq!(view_bar_height("Sync complete"), compact);
        assert_eq!(view_bar_height(""), compact);
    }

    #[test]
    fn surface_header_text_shares_a_bounded_single_row() {
        for width in [180.0, 800.0] {
            for subtitle in [
                "",
                "Synchronization complete — all workspace files are up to date",
            ] {
                let ctx = egui::Context::default();
                theme::setup_fonts(&ctx);
                let colors = Colors::from_config(&theme::preset_colors("plexi-dusk").unwrap());
                let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
                let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                    ui.set_clip_rect(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 130.0),
                    ));
                    ViewChrome::new(&colors).paint_view_bar(ui, "Files", subtitle);
                });
                let reports = super::super::layout_report::snapshot(&ctx);
                let report = reports
                    .iter()
                    .find(|r| r.surface == "surface_header")
                    .unwrap();
                assert!(report.violations().is_empty(), "{:?}", report.violations());
                assert_eq!(report.texts.len(), if subtitle.is_empty() { 1 } else { 2 });
                if !subtitle.is_empty() {
                    let title = &report.texts[0];
                    let status = &report.texts[1];
                    assert!(title.paint.right() < status.paint.left());
                    assert!((title.paint.center().y - status.paint.center().y).abs() <= 1.0);
                    if width == 180.0 {
                        assert!(status.text.ends_with('…'), "{}", status.text);
                    } else {
                        assert_eq!(status.text, subtitle);
                    }
                }
            }
        }
    }

    #[test]
    fn adoption_view_bar_elides_instead_of_clipping() {
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let colors = crate::ui::theme::Colors::from_config(
            &crate::ui::theme::preset_colors("plexi-dusk").unwrap(),
        );
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(160.0);
            ui.set_clip_rect(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(180.0, 800.0),
            ));
            let capture =
                crate::ui::layout_report::Capture::begin(ui.painter(), "adoption-fixture");
            let rect = ViewChrome::new(&colors).paint_view_bar(
                ui,
                "A deliberately long application title with an important suffix",
                "/a/very/long/unbroken/subtitle/identifier/0123456789abcdef",
            );
            capture.finish(ui.painter(), rect, colors.bg_sidebar, vec![], vec![]);
        });
        let reports = crate::ui::layout_report::snapshot(&ctx);
        assert!(!reports.is_empty());
        let errors: Vec<_> = reports.iter().flat_map(|r| r.violations()).collect();
        assert!(errors.is_empty(), "{errors:?}");
    }
}
