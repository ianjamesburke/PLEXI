use crate::ui::style;
use crate::ui::theme::Colors;
use crate::ui::widgets;

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

        // Full-bleed hairline divider between modal body and footer. The
        // painter is not clipped to the frame interior, so extending past the
        // modal's inner padding reaches the frame edges.
        ui.add_space(style::SPACE_SM);
        let (line_rect, _) =
            ui.allocate_exact_size(egui::vec2(full, 1.0), egui::Sense::hover());
        ui.painter().hline(
            egui::Rangef::new(
                line_rect.left() - f32::from(style::MODAL_PADDING_H),
                line_rect.right() + f32::from(style::MODAL_PADDING_H),
            ),
            line_rect.center().y,
            egui::Stroke::new(1.0, colors.border),
        );
        ui.add_space(style::SPACE_SM);

        // Center the hint groups in the footer.
        let total: f32 = self
            .groups
            .iter()
            .map(|g| widgets::key_combo_list_width(ui, &[g.keys], Some(g.label)))
            .sum::<f32>()
            + style::SPACE_MD * self.groups.len().saturating_sub(1) as f32;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = style::SPACE_MD;
            ui.add_space(((full - total) / 2.0).max(0.0));
            for group in self.groups {
                widgets::key_combo_list(ui, &[group.keys], Some(group.label), colors);
            }
        });
    }
}
