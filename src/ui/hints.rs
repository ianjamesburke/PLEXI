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
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = style::SPACE_MD;
            for group in self.groups {
                widgets::key_combo_list(ui, &[group.keys], Some(group.label), colors);
            }
        });
    }
}
