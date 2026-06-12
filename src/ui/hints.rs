use crate::ui::shortcuts;
use crate::ui::style;
use crate::ui::theme::Colors;

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

    pub(crate) fn show(self, ui: &mut egui::Ui, colors: &Colors) {
        let full = ui.available_width();

        // No divider above the hints — a hairline here splits a small modal
        // into a fake "footer zone" and makes the bottom read as dead space.
        // One MD gap separates the hints from the content above; the modal's
        // own bottom padding closes them out symmetrically.
        ui.add_space(style::SPACE_MD);

        // Center the hint groups in the footer.
        let total: f32 = self.groups.iter().map(|g| g.width(ui)).sum::<f32>()
            + style::SPACE_MD * self.groups.len().saturating_sub(1) as f32;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = style::SPACE_MD;
            ui.add_space(((full - total) / 2.0).max(0.0));
            for group in self.groups {
                group.show(ui, colors);
            }
        });
    }
}
