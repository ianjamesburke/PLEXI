use crate::ui::shortcuts;
use crate::ui::style;
use crate::ui::theme::Colors;
use std::ops::Range;

#[derive(Clone, Copy)]
pub(crate) struct HintGroup<'a> {
    combos: HintCombos<'a>,
    label: &'a str,
}

#[derive(Clone, Copy)]
enum HintCombos<'a> {
    One(&'a [&'a str]),
    Many(&'a [&'a [&'a str]]),
    OwnedStrings(&'a [String]),
}

impl<'a> HintGroup<'a> {
    pub(crate) fn new(keys: &'a [&'a str], label: &'a str) -> Self {
        Self {
            combos: HintCombos::One(keys),
            label,
        }
    }

    pub(crate) fn alternatives(combos: &'a [&'a [&'a str]], label: &'a str) -> Self {
        Self {
            combos: HintCombos::Many(combos),
            label,
        }
    }

    pub(crate) fn from_strings(keys: &'a [String], label: &'a str) -> Self {
        Self {
            combos: HintCombos::OwnedStrings(keys),
            label,
        }
    }

    /// Rendered width of this group: chips, intra-group gaps, and label.
    /// Callers fitting hints into a constrained row measure with this
    /// instead of guessing width breakpoints.
    pub(crate) fn width(&self, ui: &egui::Ui) -> f32 {
        match self.combos {
            HintCombos::One(keys) => shortcuts::key_combo_list_width(ui, &[keys], Some(self.label)),
            HintCombos::Many(combos) => {
                shortcuts::key_combo_list_width(ui, combos, Some(self.label))
            }
            HintCombos::OwnedStrings(keys) => {
                shortcuts::key_combo_owned_width(ui, keys, Some(self.label))
            }
        }
    }

    fn show(&self, ui: &mut egui::Ui, colors: &Colors) -> (Vec<egui::Rect>, Option<egui::Rect>) {
        let row_h = shortcuts::chip_row_height(ui);
        ui.horizontal(|ui| {
            ui.set_min_height(row_h);
            ui.spacing_mut().item_spacing.x = 0.0;
            self.show_inline(ui, colors)
        })
        .inner
    }

    fn show_inline(
        &self,
        ui: &mut egui::Ui,
        colors: &Colors,
    ) -> (Vec<egui::Rect>, Option<egui::Rect>) {
        match self.combos {
            HintCombos::One(keys) => {
                shortcuts::key_combo_list_inline(ui, &[keys], Some(self.label), colors)
            }
            HintCombos::Many(combos) => {
                shortcuts::key_combo_list_inline(ui, combos, Some(self.label), colors)
            }
            HintCombos::OwnedStrings(keys) => {
                shortcuts::key_combo_owned_inline(ui, keys, Some(self.label), colors)
            }
        }
    }
}

/// Explicit layout mode for [`HintBar`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HintBarMode {
    /// Centered hint rows with `SPACE_MD` top breathing room (used by `ModalShell` overlays).
    ModalFooter,
    /// Full-width `bg_toolbar` footer band with optional `1px` top border (used by PGAP `FooterKeys` and built-in app footers).
    PaneFooterBand { divider: bool },
    /// Single-row right-aligned hint groups inside an existing horizontal status bar row (used by `FileBrowserApp`).
    InlineStatusBar,
}

#[derive(Clone, Debug)]
pub(crate) struct HintBarResponse {
    pub(crate) rect: egui::Rect,
    pub(crate) chip_rects: Vec<egui::Rect>,
    pub(crate) label_rects: Vec<egui::Rect>,
}

pub(crate) struct HintBar<'a> {
    groups: &'a [HintGroup<'a>],
    mode: HintBarMode,
}

impl<'a> HintBar<'a> {
    pub(crate) fn new(groups: &'a [HintGroup<'a>]) -> Self {
        Self {
            groups,
            mode: HintBarMode::ModalFooter,
        }
    }

    pub(crate) fn pane_footer_band(mut self, divider: bool) -> Self {
        self.mode = HintBarMode::PaneFooterBand { divider };
        self
    }

    pub(crate) fn inline_status_bar(mut self) -> Self {
        self.mode = HintBarMode::InlineStatusBar;
        self
    }

    /// Total height of a `PaneFooterBand` at the current `ui.available_width()`,
    /// accounting for wrapped rows and the optional 1px top divider.
    pub(crate) fn band_height(&self, ui: &egui::Ui, divider: bool) -> f32 {
        let avail_w = footer_band_available_width(ui);
        let rows = self.rows(ui, avail_w);
        let n = rows.len().max(1) as f32;
        let row_h = hint_row_height(ui) + 4.0;
        let base = style::SPACE_SM + n * row_h + (n - 1.0) * style::SPACE_XS + style::SPACE_SM;
        if divider {
            1.0 + base
        } else {
            base
        }
    }

    pub(crate) fn show(self, ui: &mut egui::Ui, colors: &Colors) {
        let _ = self.show_measured(ui, colors);
    }

    pub(crate) fn show_measured(self, ui: &mut egui::Ui, colors: &Colors) -> HintBarResponse {
        match self.mode {
            HintBarMode::ModalFooter => self.show_modal_footer(ui, colors),
            HintBarMode::PaneFooterBand { divider } => {
                self.show_pane_footer_band(ui, colors, divider)
            }
            HintBarMode::InlineStatusBar => self.show_inline_status_bar(ui, colors),
        }
    }

    fn show_modal_footer(self, ui: &mut egui::Ui, colors: &Colors) -> HintBarResponse {
        let full = ui.available_width();

        // No divider above the hints — a hairline here splits a small modal
        // into a fake "footer zone" and makes the bottom read as dead space.
        // One MD gap separates the hints from the content above; the modal's
        // own bottom padding closes them out symmetrically.
        ui.add_space(style::SPACE_MD);

        let widths: Vec<f32> = self.groups.iter().map(|group| group.width(ui)).collect();
        let mut resp = HintBarResponse {
            rect: egui::Rect::NOTHING,
            chip_rects: Vec::new(),
            label_rects: Vec::new(),
        };
        for row in hint_rows(&widths, full, style::SPACE_MD) {
            let total = widths[row.clone()].iter().sum::<f32>()
                + style::SPACE_MD * row.len().saturating_sub(1) as f32;
            let row_resp = ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = style::SPACE_MD;
                ui.add_space(((full - total) / 2.0).max(0.0));
                for group in &self.groups[row] {
                    let (chips, label) = group.show(ui, colors);
                    resp.chip_rects.extend(chips);
                    if let Some(l) = label {
                        resp.label_rects.push(l);
                    }
                }
            });
            resp.rect = resp.rect.union(row_resp.response.rect);
        }
        resp
    }

    fn show_pane_footer_band(
        self,
        ui: &mut egui::Ui,
        colors: &Colors,
        divider: bool,
    ) -> HintBarResponse {
        let chip_row_h = hint_row_height(ui);
        let row_h = chip_row_h + 4.0;
        let avail_w = footer_band_available_width(ui);
        let widths: Vec<f32> = self.groups.iter().map(|group| group.width(ui)).collect();
        let rows = hint_rows(&widths, avail_w, style::SPACE_MD);
        let total_h = self.band_height(ui, divider);

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
            .rect_filled(full_rect, 0.0, colors.bg_toolbar);

        if divider {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(full_rect.min, egui::vec2(full_rect.width(), 1.0)),
                0.0,
                colors.border,
            );
        }

        let mut resp = HintBarResponse {
            rect: full_rect,
            chip_rects: Vec::new(),
            label_rects: Vec::new(),
        };

        let content_top = full_rect.min.y + if divider { 1.0 } else { 0.0 } + style::SPACE_SM;
        for (idx, row) in rows.into_iter().enumerate() {
            let row_w = widths[row.clone()].iter().sum::<f32>()
                + style::SPACE_MD * row.len().saturating_sub(1) as f32;
            let row_top = content_top + idx as f32 * (row_h + style::SPACE_XS);
            let row_band = egui::Rect::from_min_size(
                egui::pos2(full_rect.min.x, row_top),
                egui::vec2(full_rect.width(), row_h),
            );
            let content_rect = footer_band_row_content_rect(row_band, row_w, chip_row_h, false);
            let row_slice = &self.groups[row];
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(content_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.set_clip_rect(content_rect.intersect(clip));
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (gi, group) in row_slice.iter().enumerate() {
                        if gi > 0 {
                            ui.add_space(style::SPACE_MD);
                        }
                        let (chips, label) = group.show_inline(ui, colors);
                        resp.chip_rects.extend(chips);
                        if let Some(l) = label {
                            resp.label_rects.push(l);
                        }
                    }
                },
            );
        }

        resp
    }

    fn show_inline_status_bar(self, ui: &mut egui::Ui, colors: &Colors) -> HintBarResponse {
        let mut resp = HintBarResponse {
            rect: egui::Rect::NOTHING,
            chip_rects: Vec::new(),
            label_rects: Vec::new(),
        };
        if self.groups.is_empty() {
            return resp;
        }
        let widths: Vec<f32> = self.groups.iter().map(|group| group.width(ui)).collect();
        let total = widths.iter().sum::<f32>()
            + style::SPACE_MD * self.groups.len().saturating_sub(1) as f32;
        let rem = ui.available_width().max(0.0);
        let row_h = ui
            .max_rect()
            .height()
            .max(shortcuts::chip_row_height(ui));
        let slot_rect = egui::Rect::from_min_size(
            egui::pos2(ui.max_rect().max.x - rem, ui.max_rect().min.y),
            egui::vec2(rem, row_h),
        );
        let lead = (rem - total).max(0.0);
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(slot_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if lead > 0.0 {
                    ui.add_space(lead);
                }
                for (gi, group) in self.groups.iter().enumerate() {
                    if gi > 0 {
                        ui.add_space(style::SPACE_MD);
                    }
                    let (chips, label) = group.show_inline(ui, colors);
                    for c in &chips {
                        resp.rect = resp.rect.union(*c);
                    }
                    if let Some(l) = label {
                        resp.rect = resp.rect.union(l);
                        resp.label_rects.push(l);
                    }
                    resp.chip_rects.extend(chips);
                }
            },
        );
        resp
    }

    pub(crate) fn rows(&self, ui: &egui::Ui, available_width: f32) -> Vec<Range<usize>> {
        let widths: Vec<f32> = self.groups.iter().map(|group| group.width(ui)).collect();
        hint_rows(&widths, available_width, style::SPACE_MD)
    }
}

pub(crate) fn hint_row_height(ui: &egui::Ui) -> f32 {
    shortcuts::chip_row_height(ui)
}

pub(crate) fn footer_band_available_width(ui: &egui::Ui) -> f32 {
    (ui.available_width() - style::SPACE_MD * 2.0).max(0.0)
}

pub(crate) fn footer_band_row_content_rect(
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

fn hint_rows(widths: &[f32], available: f32, gap: f32) -> Vec<Range<usize>> {
    let mut rows = Vec::new();
    let mut start = 0;
    let mut row_width = 0.0;

    for (index, width) in widths.iter().enumerate() {
        let next_width = if index == start {
            *width
        } else {
            row_width + gap + width
        };
        if index > start && next_width > available {
            rows.push(start..index);
            start = index;
            row_width = *width;
        } else {
            row_width = next_width;
        }
    }
    if start < widths.len() {
        rows.push(start..widths.len());
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_packing_keeps_groups_intact_and_uses_one_row_when_they_fit() {
        assert_eq!(
            hint_rows(&[72.0, 64.0, 80.0], 240.0, style::SPACE_MD),
            vec![0..3]
        );
        assert_eq!(
            hint_rows(&[72.0, 64.0, 80.0], 160.0, style::SPACE_MD),
            vec![0..2, 2..3]
        );
    }
}
