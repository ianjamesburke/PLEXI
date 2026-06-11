use crate::ui::shortcuts;
use crate::ui::style;
use crate::ui::theme::Colors;

pub(crate) struct HintGroup<'a> {
    keys: &'a [&'a str],
    label: &'a str,
}

impl<'a> HintGroup<'a> {
    pub(crate) fn new(keys: &'a [&'a str], label: &'a str) -> Self {
        Self { keys, label }
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
        let total: f32 = self
            .groups
            .iter()
            .map(|g| shortcuts::key_combo_list_width(ui, &[g.keys], Some(g.label)))
            .sum::<f32>()
            + style::SPACE_MD * self.groups.len().saturating_sub(1) as f32;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = style::SPACE_MD;
            ui.add_space(((full - total) / 2.0).max(0.0));
            for group in self.groups {
                shortcuts::key_combo_list(ui, &[group.keys], Some(group.label), colors);
            }
        });
    }
}
