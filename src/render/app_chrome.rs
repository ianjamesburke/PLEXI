use egui::{Color32, FontId, Response, RichText, Ui};

use crate::app_protocol::{FooterKeyEntry, SelectListItem};
use crate::process_app::render::parse_color;
use crate::ui::theme::{self, Colors};
use crate::ui::{button, style};

const APP_BAR_TITLE_SIZE: f32 = style::TEXT_TITLE;
const APP_BAR_SINGLE_BAND_H: f32 = 34.0;
const APP_BAR_DOUBLE_BAND_H: f32 = 48.0;
const TEXT_EDIT_SINGLELINE_H: f32 = style::BUTTON_H_MD;
const TEXT_EDIT_MULTILINE_H: f32 = 96.0;
pub(crate) const CARD_CHILD_GAP: f32 = style::SPACE_XS;

pub(crate) struct TextEditChromeResponse {
    pub(crate) response: Response,
    pub(crate) frame_clicked: bool,
}

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

    pub(crate) fn surface_fill(&self) -> Color32 {
        self.colors.bg_active
    }

    pub(crate) fn border(&self) -> Color32 {
        self.colors.border
    }

    pub(crate) fn text_color(&self, explicit: &str, tone: &str) -> Color32 {
        if !explicit.is_empty() {
            return parse_color(explicit).unwrap_or_else(|| resolve_tone(explicit, self.colors));
        }
        resolve_tone(tone, self.colors)
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

    pub(crate) fn paint_footer(&self, ui: &mut Ui, text: &str, color: &str) {
        let total_h = footer_height();
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
        painter.rect_filled(
            egui::Rect::from_min_size(full_rect.min, egui::vec2(full_rect.width(), 1.0)),
            0.0,
            self.border(),
        );
        let text_color = if color.is_empty() {
            self.colors.text_dim
        } else {
            parse_color(color).unwrap_or(self.colors.text_dim)
        };
        let content_rect = egui::Rect::from_min_size(
            egui::pos2(full_rect.min.x + style::SPACE_MD, full_rect.min.y + 1.0),
            egui::vec2(
                (full_rect.width() - 2.0 * style::SPACE_MD).max(0.0),
                total_h - 1.0,
            ),
        );
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(content_rect), |ui| {
            ui.with_layout(
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| {
                    ui.label(
                        RichText::new(text)
                            .size(style::TEXT_CAPTION)
                            .color(text_color),
                    );
                },
            );
        });
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

    pub(crate) fn paint_action_bar_background(&self, ui: &Ui, rect: egui::Rect) {
        ui.painter().rect_filled(rect, 0.0, self.colors.bg_darkest);
    }

    pub(crate) fn paint_badge(&self, ui: &mut Ui, label: &str, fill: &str, fg: &str) {
        let fill_color = if fill.is_empty() || fill == "neutral" {
            self.surface_fill()
        } else {
            parse_color(fill).unwrap_or_else(|| resolve_tone(fill, self.colors))
        };
        let fg_color = if fg.is_empty() {
            self.colors.text_on(fill_color)
        } else {
            parse_color(fg).unwrap_or(self.colors.text_primary)
        };
        egui::Frame::new()
            .fill(fill_color)
            .stroke(egui::Stroke::new(1.0, self.border()))
            .corner_radius(egui::CornerRadius::same(style::RADIUS_BADGE as u8))
            .inner_margin(egui::Margin::symmetric(
                style::BADGE_PAD_H as i8,
                style::BADGE_PAD_V as i8,
            ))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(label)
                        .color(fg_color)
                        .size(style::TEXT_CAPTION),
                );
            });
    }

    pub(crate) fn paint_dot(&self, ui: &mut Ui, color: &str, size: f32) {
        let dot_size = if size > 0.0 { size } else { 8.0 };
        let fill = self.text_color(color, "accent");
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(dot_size, dot_size), egui::Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), dot_size / 2.0, fill);
    }

    pub(crate) fn paint_section(&self, ui: &mut Ui, title: &str) {
        let total_h = section_height();
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), total_h),
            egui::Sense::hover(),
        );
        let label_y = rect.min.y + style::SPACE_SM;
        self.paint_no_wrap_text(
            ui,
            ui.painter(),
            egui::pos2(rect.min.x, label_y),
            &title.to_uppercase(),
            FontId::proportional(style::TEXT_HINT),
            self.colors.text_dim,
        );
        let line_y = label_y + style::TEXT_HINT + style::SPACE_XS;
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(rect.min.x, line_y),
                egui::vec2(rect.width(), 1.0),
            ),
            0.0,
            self.border(),
        );
    }

    pub(crate) fn paint_divider(&self, ui: &mut Ui, color: &str) {
        let fill = if color.is_empty() {
            self.border()
        } else {
            self.text_color(color, "border")
        };
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, fill);
    }

    pub(crate) fn card<R>(
        &self,
        ui: &mut Ui,
        padding: f32,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> egui::InnerResponse<R> {
        let pad = card_padding(padding);
        egui::Frame::new()
            .fill(self.surface_fill())
            .stroke(egui::Stroke::new(1.0, self.border()))
            .corner_radius(style::RADIUS_MD)
            .inner_margin(egui::Margin::same(pad as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                add_contents(ui)
            })
    }

    pub(crate) fn select_list(&self, ui: &mut Ui, items: &[SelectListItem], selected_idx: usize) {
        if items.is_empty() {
            ui.label(
                RichText::new("No items")
                    .size(style::TEXT_HINT)
                    .color(self.colors.text_dim),
            );
            return;
        }

        let avail = ui.available_size();
        egui::ScrollArea::vertical()
            .max_height(avail.y)
            .show(ui, |ui| {
                for (i, item) in items.iter().enumerate() {
                    let selected = i == selected_idx;
                    let row_h = if item.description.is_empty() {
                        style::LIST_ROW_DENSE_H + 6.0
                    } else {
                        style::LIST_ROW_H
                    };
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(avail.x, row_h),
                        egui::Sense::hover(),
                    );
                    if selected {
                        crate::ui::list::paint_selection(ui.painter(), rect, self.colors);
                    } else if response.hovered() {
                        ui.painter().rect_filled(
                            crate::ui::list::selection_inset(rect),
                            style::RADIUS_SM,
                            self.colors.bg_hover,
                        );
                    }

                    let text_x = rect.min.x + style::LIST_ROW_PAD_H;
                    let mut max_w = rect.width() - style::LIST_ROW_PAD_H * 2.0;
                    if !item.trailing.is_empty() {
                        let trailing = ui.fonts(|f| {
                            f.layout_no_wrap(
                                item.trailing.clone(),
                                FontId::proportional(style::TEXT_HINT),
                                self.colors.text_dim,
                            )
                        });
                        let tr_x = rect.max.x - style::LIST_ROW_PAD_H - trailing.size().x;
                        ui.painter().galley(
                            egui::pos2(tr_x, rect.center().y - trailing.size().y / 2.0),
                            trailing,
                            self.colors.text_dim,
                        );
                        max_w = tr_x - style::LIST_ROW_GAP - text_x;
                    }

                    let primary_color = if selected {
                        self.colors.text_primary
                    } else {
                        self.colors.text_primary
                    };
                    if item.description.is_empty() {
                        self.paint_no_wrap_text(
                            ui,
                            ui.painter(),
                            egui::pos2(text_x, rect.center().y - style::TEXT_CAPTION / 2.0),
                            &crate::ui::list::elide_to_width(
                                ui,
                                &item.name,
                                theme::font_medium(style::TEXT_CAPTION),
                                max_w.max(0.0),
                            ),
                            theme::font_medium(style::TEXT_CAPTION),
                            primary_color,
                        );
                    } else {
                        let block_h = style::TEXT_CAPTION + 2.0 + style::TEXT_HINT;
                        let title_y = rect.center().y - block_h / 2.0;
                        self.paint_no_wrap_text(
                            ui,
                            ui.painter(),
                            egui::pos2(text_x, title_y),
                            &crate::ui::list::elide_to_width(
                                ui,
                                &item.name,
                                theme::font_medium(style::TEXT_CAPTION),
                                max_w.max(0.0),
                            ),
                            theme::font_medium(style::TEXT_CAPTION),
                            primary_color,
                        );
                        self.paint_no_wrap_text(
                            ui,
                            ui.painter(),
                            egui::pos2(text_x, title_y + style::TEXT_CAPTION + 2.0),
                            &crate::ui::list::elide_to_width(
                                ui,
                                &item.description,
                                FontId::proportional(style::TEXT_HINT),
                                max_w.max(0.0),
                            ),
                            FontId::proportional(style::TEXT_HINT),
                            self.colors.text_dim,
                        );
                    }
                }
            });
    }

    pub(crate) fn text_edit(
        &self,
        ui: &mut Ui,
        widget_id: egui::Id,
        placeholder: &str,
        buffer: &mut String,
        multiline: bool,
        max_length: usize,
    ) -> TextEditChromeResponse {
        let height = text_edit_height(multiline);
        let width = ui.available_width().max(0.0);
        let (rect, frame_response) =
            ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
        let hovered = frame_response.hovered();
        let stroke = if hovered {
            egui::Stroke::new(1.0, self.colors.text_section)
        } else {
            egui::Stroke::new(1.0, self.border())
        };
        ui.painter()
            .rect_filled(rect, style::RADIUS_SM, self.surface_fill());
        ui.painter()
            .rect_stroke(rect, style::RADIUS_SM, stroke, egui::StrokeKind::Inside);

        let inner = rect.shrink2(egui::vec2(style::SPACE_SM, 0.0));
        let response = ui
            .allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
                ui.set_clip_rect(inner);
                ui.visuals_mut().text_cursor.blink = false;
                ui.visuals_mut().text_cursor.stroke.color = Color32::TRANSPARENT;
                let font_id = FontId::proportional(style::TEXT_BODY);
                let row_height = ui.fonts(|f| f.row_height(&font_id));
                let hint = RichText::new(placeholder)
                    .color(self.colors.text_dim.linear_multiply(0.45))
                    .size(style::TEXT_BODY);
                let output = if multiline {
                    let mut edit = egui::TextEdit::multiline(buffer)
                        .id(widget_id)
                        .font(font_id.clone())
                        .text_color(self.colors.text_primary)
                        .desired_width(f32::INFINITY)
                        .frame(false)
                        .hint_text(hint);
                    if max_length > 0 {
                        edit = edit.char_limit(max_length);
                    }
                    edit.show(ui)
                } else {
                    ui.add_space((inner.height() - row_height).max(0.0) / 2.0);
                    let mut edit = egui::TextEdit::singleline(buffer)
                        .id(widget_id)
                        .font(font_id.clone())
                        .text_color(self.colors.text_primary)
                        .desired_width(f32::INFINITY)
                        .frame(false)
                        .hint_text(hint);
                    if max_length > 0 {
                        edit = edit.char_limit(max_length);
                    }
                    edit.show(ui)
                };
                crate::ui::text_field::draw_text_caret(
                    ui,
                    &output,
                    style::TEXT_BODY,
                    row_height,
                    egui::Stroke::new(1.0, self.colors.accent),
                );
                output.response
            })
            .inner;

        TextEditChromeResponse {
            response,
            frame_clicked: frame_response.clicked(),
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

pub(crate) fn button_height() -> f32 {
    style::BUTTON_H_MD
}

pub(crate) fn action_bar_height() -> f32 {
    button_height() + style::SPACE_SM
}

pub(crate) fn button_kind(button_style: &str) -> button::ButtonKind {
    match button_style {
        "primary" => button::ButtonKind::Accent,
        "danger" => button::ButtonKind::Danger,
        "ghost" => button::ButtonKind::Ghost,
        _ => button::ButtonKind::Secondary,
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

pub(crate) fn footer_height() -> f32 {
    style::SPACE_MD + 1.0 + style::SPACE_MD + style::TEXT_CAPTION + 5.0
}

pub(crate) fn text_edit_height(multiline: bool) -> f32 {
    if multiline {
        TEXT_EDIT_MULTILINE_H
    } else {
        TEXT_EDIT_SINGLELINE_H
    }
}

pub(crate) fn card_padding(padding: f32) -> f32 {
    if padding > 0.0 {
        padding
    } else {
        style::SPACE_MD
    }
}

pub(crate) fn section_height() -> f32 {
    style::SPACE_SM + style::TEXT_HINT + style::SPACE_XS + 1.0 + style::SPACE_XS
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

fn resolve_tone(tone: &str, colors: &Colors) -> Color32 {
    match tone {
        "hint" | "dim" | "muted" => colors.text_dim,
        "neutral" | "surface" => colors.bg_active,
        "border" => colors.border,
        "danger" | "error" => colors.danger,
        "success" => colors.success,
        "warning" => colors.warning,
        "accent" => colors.accent,
        "section" => colors.text_section,
        _ => colors.text_primary,
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
            bg_active: Some("#040506".into()),
            border: Some("#070809".into()),
            text_primary: Some("#0a0b0c".into()),
            text_dim: Some("#0d0e0f".into()),
            accent: Some("#101112".into()),
            ..ThemeConfig::default()
        };
        let colors = Colors::from_config(&cfg);
        let chrome = AppChrome::new(&colors);

        assert_eq!(chrome.toolbar_fill(), colors.bg_toolbar);
        assert_eq!(chrome.surface_fill(), colors.bg_active);
        assert_eq!(chrome.border(), colors.border);
        assert_eq!(chrome.text_color("", ""), colors.text_primary);
        assert_eq!(chrome.text_color("", "hint"), colors.text_dim);
        assert_eq!(chrome.text_color("", "accent"), colors.accent);
        assert_eq!(chrome.text_color("accent", ""), colors.accent);
        assert_eq!(chrome.text_color("neutral", ""), colors.bg_active);
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

    #[test]
    fn semantic_app_chrome_uses_host_component_metrics() {
        assert_eq!(button_height(), style::BUTTON_H_MD);
        assert_eq!(action_bar_height(), style::BUTTON_H_MD + style::SPACE_SM);
        assert_eq!(card_padding(0.0), style::SPACE_MD);
        assert_eq!(card_padding(style::SPACE_XL), style::SPACE_XL);
        assert_eq!(text_edit_height(false), style::BUTTON_H_MD);
        assert!(matches!(button_kind("primary"), button::ButtonKind::Accent));
        assert!(matches!(button_kind("danger"), button::ButtonKind::Danger));
        assert!(matches!(button_kind("ghost"), button::ButtonKind::Ghost));
        assert!(matches!(button_kind(""), button::ButtonKind::Secondary));
    }
}
