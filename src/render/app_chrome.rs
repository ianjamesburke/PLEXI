use egui::{Color32, FontId, Response, RichText, Ui};

use crate::host::wasm_app::bindings::plexi::platform::types::FooterKeyEntry;
use crate::ui::theme::{self, Colors};
use crate::ui::style;

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

    pub(crate) fn paint_app_bar(&self, ui: &mut Ui, title: &str, subtitle: &str) {
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
                self.colors.text_dim,
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
    }

    pub(crate) fn paint_footer_keys(&self, ui: &mut Ui, entries: &[FooterKeyEntry], divider: bool) {
        let chip_row_h = chip_row_height(ui);
        let row_h = chip_row_h + 4.0;
        let chip_font = FontId::monospace(style::TEXT_HINT);
        let desc_font = FontId::proportional(style::TEXT_HINT);

        // Measure against the width the height pass used, so allocation and paint
        // agree on the wrapped row count.
        let avail_w = footer_keys_available_width(ui);
        let rows = footer_keys_rows(ui, entries, &chip_font, &desc_font, avail_w);
        let total_h = footer_keys_height(ui, entries, divider);

        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), total_h),
            egui::Sense::hover(),
        );

        let clip = ui.clip_rect();
        let full_rect = egui::Rect::from_min_size(
            egui::pos2(clip.min.x, rect.min.y),
            egui::vec2(clip.width(), total_h),
        );
        ui.painter()
            .rect_filled(full_rect, 0.0, self.toolbar_fill());

        if divider {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(full_rect.min, egui::vec2(full_rect.width(), 1.0)),
                0.0,
                self.border(),
            );
        }

        // Each row is centered horizontally (preserving the single-line centering
        // of #2111) and stacked top-to-bottom inside the band.
        let content_top = full_rect.min.y + if divider { 1.0 } else { 0.0 } + style::SPACE_SM;
        for (idx, (start, end, row_w)) in rows.iter().enumerate() {
            let row_top = content_top + idx as f32 * (row_h + style::SPACE_XS);
            let row_band = egui::Rect::from_min_size(
                egui::pos2(full_rect.min.x, row_top),
                egui::vec2(full_rect.width(), row_h),
            );
            let content_rect = footer_keys_content_rect(row_band, *row_w, chip_row_h, false);
            paint_footer_keys_row(
                ui,
                content_rect,
                &entries[*start..*end],
                chip_font.clone(),
                desc_font.clone(),
                self.colors,
            );
        }
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
        let galley = ui.fonts(|f| f.layout_no_wrap(text.to_owned(), font, color));
        painter.galley(pos, galley, color);
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

pub(crate) fn chip_row_height(ui: &egui::Ui) -> f32 {
    let text_h = ui.fonts(|f| {
        f.layout_no_wrap(
            "X".to_string(),
            FontId::monospace(style::TEXT_HINT),
            Color32::WHITE,
        )
        .size()
        .y
    });
    text_h + style::KEYCHIP_PAD_V * 2.0
}

/// Height of the footer-keys band, accounting for wrapping: in a pane too
/// narrow to hold every entry on one line, entries flow onto additional rows and
/// the band grows to fit (#2240). `ui.available_width()` is the same width the
/// paint pass lays out against, so the host-measured height the `Column` /
/// bottom-pin allocator reserves always matches what gets painted.
pub(crate) fn footer_keys_height(ui: &egui::Ui, entries: &[FooterKeyEntry], divider: bool) -> f32 {
    let chip_font = FontId::monospace(style::TEXT_HINT);
    let desc_font = FontId::proportional(style::TEXT_HINT);
    let avail_w = footer_keys_available_width(ui);
    let rows = footer_keys_rows(ui, entries, &chip_font, &desc_font, avail_w);
    let n = rows.len().max(1) as f32;
    let row_h = chip_row_height(ui) + 4.0;
    let base = style::SPACE_SM + n * row_h + (n - 1.0) * style::SPACE_XS + style::SPACE_SM;
    if divider {
        1.0 + base
    } else {
        base
    }
}

/// Usable content width for footer-key rows: the available width minus the
/// left/right band insets [`footer_keys_content_rect`] applies.
fn footer_keys_available_width(ui: &egui::Ui) -> f32 {
    (ui.available_width() - style::SPACE_MD * 2.0).max(0.0)
}

/// Width of a single footer entry (its key chips + gaps + description) as an
/// atomic, un-wrappable unit. Color does not affect glyph advance, so a fixed
/// color is used — this lets the free-function height path measure without a
/// `Colors` handle.
fn footer_entry_width(
    ui: &Ui,
    entry: &FooterKeyEntry,
    chip_font: &FontId,
    desc_font: &FontId,
) -> f32 {
    let chip_row_h = chip_row_height(ui);
    let mut w: f32 = 0.0;
    for (ki, key) in entry.keys.iter().enumerate() {
        if ki > 0 {
            w += style::KEYCHIP_GAP;
        }
        let tw = ui.fonts(|f| {
            f.layout_no_wrap(key.clone(), chip_font.clone(), Color32::WHITE)
                .size()
                .x
        });
        w += (tw + style::KEYCHIP_PAD_H * 2.0)
            .max(chip_row_h)
            .max(style::KEYCHIP_MIN_W);
    }
    w += 4.0;
    w += ui.fonts(|f| {
        f.layout_no_wrap(entry.description.clone(), desc_font.clone(), Color32::WHITE)
            .size()
            .x
    });
    w
}

/// Greedily pack footer entries into rows no wider than `avail_w`. Each row is
/// `(start, end, row_width)` over `entries[start..end]`. An entry that alone
/// exceeds `avail_w` still occupies its own row (never dropped). The height and
/// paint passes both call this so their row counts always agree.
fn footer_keys_rows(
    ui: &Ui,
    entries: &[FooterKeyEntry],
    chip_font: &FontId,
    desc_font: &FontId,
    avail_w: f32,
) -> Vec<(usize, usize, f32)> {
    let mut rows = Vec::new();
    let mut i = 0;
    while i < entries.len() {
        let start = i;
        let mut row_w = 0.0;
        while i < entries.len() {
            let ew = footer_entry_width(ui, &entries[i], chip_font, desc_font);
            let sep = if i > start { style::SPACE_MD } else { 0.0 };
            // Always keep at least one entry per row, even if it overflows alone.
            if i > start && row_w + sep + ew > avail_w {
                break;
            }
            row_w += sep + ew;
            i += 1;
        }
        rows.push((start, i, row_w));
    }
    rows
}

pub(crate) fn footer_keys_content_rect(
    full_rect: egui::Rect,
    content_w: f32,
    chip_row_h: f32,
    divider: bool,
) -> egui::Rect {
    let left_bound = full_rect.min.x + style::SPACE_MD;
    let right_bound = full_rect.max.x - style::SPACE_MD;
    let available_w = (right_bound - left_bound).max(0.0);
    let width = content_w.min(available_w);
    let left = if content_w <= available_w {
        (full_rect.center().x - width / 2.0).clamp(left_bound, right_bound - width)
    } else {
        left_bound
    };
    let chrome_top = full_rect.min.y + if divider { 1.0 } else { 0.0 };
    let chrome_h = (full_rect.max.y - chrome_top).max(0.0);
    let top = chrome_top + (chrome_h - chip_row_h).max(0.0) / 2.0;
    egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(width, chip_row_h))
}

fn paint_footer_keys_row(
    ui: &Ui,
    content_rect: egui::Rect,
    entries: &[FooterKeyEntry],
    chip_font: FontId,
    desc_font: FontId,
    colors: &Colors,
) {
    let painter = ui.painter().with_clip_rect(content_rect);
    let key_color = colors.text_primary.gamma_multiply(0.78);
    let desc_color = colors.text_primary.gamma_multiply(0.70);
    let mut x = content_rect.min.x;

    for (ei, entry) in entries.iter().enumerate() {
        for (ki, key) in entry.keys.iter().enumerate() {
            if ki > 0 {
                x += style::KEYCHIP_GAP;
            }
            let galley = ui.fonts(|f| f.layout_no_wrap(key.clone(), chip_font.clone(), key_color));
            let text_size = galley.size();
            let chip_h = text_size.y + style::KEYCHIP_PAD_V * 2.0;
            let chip_w = (text_size.x + style::KEYCHIP_PAD_H * 2.0)
                .max(chip_h)
                .max(style::KEYCHIP_MIN_W);
            let chip_rect = egui::Rect::from_min_size(
                egui::pos2(x, content_rect.center().y - chip_h / 2.0),
                egui::vec2(chip_w, chip_h),
            );
            painter.rect_filled(chip_rect, egui::CornerRadius::same(4), colors.bg_active);
            painter.galley(
                egui::pos2(
                    chip_rect.center().x - text_size.x / 2.0,
                    chip_rect.center().y - text_size.y / 2.0,
                ),
                galley,
                key_color,
            );
            x += chip_w;
        }

        x += 4.0;
        let desc_galley = ui
            .fonts(|f| f.layout_no_wrap(entry.description.clone(), desc_font.clone(), desc_color));
        let desc_size = desc_galley.size();
        painter.galley(
            egui::pos2(x, content_rect.center().y - desc_size.y / 2.0),
            desc_galley,
            desc_color,
        );
        x += desc_size.x;

        if ei + 1 < entries.len() {
            x += style::SPACE_MD;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ThemeConfig;

    #[test]
    fn semantic_app_chrome_colors_follow_host_theme_tokens() {
        let cfg = ThemeConfig {
            bg_toolbar: Some("#010203".into()),
            border: Some("#070809".into()),
            ..ThemeConfig::default()
        };
        let colors = Colors::from_config(&cfg);
        let chrome = AppChrome::new(&colors);

        assert_eq!(chrome.toolbar_fill(), colors.bg_toolbar);
        assert_eq!(chrome.border(), colors.border);
    }

    /// #2240: footer keys pack onto one row when they fit and wrap onto
    /// additional rows in a narrow band, and the measured height grows with the
    /// wrapped row count so `Column`/bottom-pin reserves enough space.
    #[test]
    fn footer_keys_wrap_in_narrow_band() {
        let entries: Vec<FooterKeyEntry> = (0..6)
            .map(|i| FooterKeyEntry {
                keys: vec![format!("^{i}")],
                description: format!("action {i}"),
            })
            .collect();
        let chip_font = FontId::monospace(style::TEXT_HINT);
        let desc_font = FontId::proportional(style::TEXT_HINT);

        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        egui::CentralPanel::default().show(&ctx, |ui| {
            // Wide band: everything fits on a single row.
            let wide = footer_keys_rows(ui, &entries, &chip_font, &desc_font, 10_000.0);
            assert_eq!(wide.len(), 1, "all entries fit on one row when wide");
            assert_eq!(wide[0], (0, entries.len(), wide[0].2));

            // Narrow band: entries wrap onto multiple rows, none dropped.
            let narrow = footer_keys_rows(ui, &entries, &chip_font, &desc_font, 90.0);
            assert!(narrow.len() > 1, "entries must wrap in a narrow band");
            let covered: usize = narrow.iter().map(|(s, e, _)| e - s).sum();
            assert_eq!(covered, entries.len(), "every entry appears exactly once");
            for w in narrow.windows(2) {
                assert_eq!(w[0].1, w[1].0, "rows are contiguous");
            }

            // Height reflects the wrapped row count: taller than a single row.
            let one_row = style::SPACE_SM + (chip_row_height(ui) + 4.0) + style::SPACE_SM;
            assert!(
                footer_keys_height(ui, &entries, false) >= one_row,
                "wrapped footer height must be at least one row tall"
            );
        });
        let _ = ctx.end_pass();
    }
}
