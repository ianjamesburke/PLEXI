use crate::ui::shortcuts;
use crate::ui::style;
use crate::ui::theme::Colors;
use std::ops::Range;

pub(crate) struct HintGroup<'a> {
    combos: HintCombos<'a>,
    label: &'a str,
}

enum HintCombos<'a> {
    One(&'a [&'a str]),
    Many(&'a [&'a [&'a str]]),
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

    /// Rendered width of this group: chips, intra-group gaps, and label.
    /// Callers fitting hints into a constrained row measure with this
    /// instead of guessing width breakpoints.
    pub(crate) fn width(&self, ui: &egui::Ui) -> f32 {
        match self.combos {
            HintCombos::One(keys) => shortcuts::key_combo_list_width(ui, &[keys], Some(self.label)),
            HintCombos::Many(combos) => {
                shortcuts::key_combo_list_width(ui, combos, Some(self.label))
            }
        }
    }

    fn show(&self, ui: &mut egui::Ui, colors: &Colors) {
        match self.combos {
            HintCombos::One(keys) => {
                shortcuts::key_combo_list(ui, &[keys], Some(self.label), colors)
            }
            HintCombos::Many(combos) => {
                shortcuts::key_combo_list(ui, combos, Some(self.label), colors);
            }
        }
    }
}

pub(crate) struct HintBar<'a> {
    groups: &'a [HintGroup<'a>],
}

impl<'a> HintBar<'a> {
    pub(crate) fn new(groups: &'a [HintGroup<'a>]) -> Self {
        Self { groups }
    }

    /// Extra vertical space needed beyond the first row at `available_width`.
    /// Fixed-height callers can reserve this without duplicating the packing
    /// policy that `show` uses.
    pub(crate) fn additional_height(&self, ui: &egui::Ui, available_width: f32) -> f32 {
        self.rows(ui, available_width).len().saturating_sub(1) as f32
            * (hint_row_height(ui) + ui.spacing().item_spacing.y)
    }

    pub(crate) fn show(self, ui: &mut egui::Ui, colors: &Colors) {
        let full = ui.available_width();

        // No divider above the hints — a hairline here splits a small modal
        // into a fake "footer zone" and makes the bottom read as dead space.
        // One MD gap separates the hints from the content above; the modal's
        // own bottom padding closes them out symmetrically.
        ui.add_space(style::SPACE_MD);

        let widths: Vec<f32> = self.groups.iter().map(|group| group.width(ui)).collect();
        for row in hint_rows(&widths, full, style::SPACE_MD) {
            let total = widths[row.clone()].iter().sum::<f32>()
                + style::SPACE_MD * row.len().saturating_sub(1) as f32;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = style::SPACE_MD;
                ui.add_space(((full - total) / 2.0).max(0.0));
                for group in &self.groups[row] {
                    group.show(ui, colors);
                }
            });
        }
    }

    fn rows(&self, ui: &egui::Ui, available_width: f32) -> Vec<Range<usize>> {
        let widths: Vec<f32> = self.groups.iter().map(|group| group.width(ui)).collect();
        hint_rows(&widths, available_width, style::SPACE_MD)
    }
}

fn hint_row_height(ui: &egui::Ui) -> f32 {
    ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(
                "M".to_owned(),
                egui::FontId::monospace(style::TEXT_CAPTION),
                egui::Color32::WHITE,
            )
            .size()
            .y
    }) + style::KEYCHIP_PAD_V * 2.0
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
