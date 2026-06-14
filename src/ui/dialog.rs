use crate::ui::button::{self, ButtonKind};
use crate::ui::hints::{HintBar, HintGroup};
use crate::ui::overlay::ModalShell;
use crate::ui::style;
use crate::ui::theme::Colors;

pub(crate) struct DialogShortcut<'a> {
    pub(crate) keys: &'a [&'a str],
    pub(crate) label: &'a str,
    modifiers: egui::Modifiers,
    key: egui::Key,
}

impl<'a> DialogShortcut<'a> {
    pub(crate) fn new(
        keys: &'a [&'a str],
        label: &'a str,
        modifiers: egui::Modifiers,
        key: egui::Key,
    ) -> Self {
        Self {
            keys,
            label,
            modifiers,
            key,
        }
    }

    fn pressed(&self, ctx: &egui::Context) -> bool {
        ctx.input_mut(|i| i.consume_key(self.modifiers, self.key))
    }
}

pub(crate) struct DialogAction<'a> {
    pub(crate) id: &'a str,
    pub(crate) label: &'a str,
    pub(crate) kind: ButtonKind,
    pub(crate) min_width: f32,
    pub(crate) shortcut: Option<DialogShortcut<'a>>,
}

impl<'a> DialogAction<'a> {
    pub(crate) fn new(id: &'a str, label: &'a str, kind: ButtonKind) -> Self {
        Self {
            id,
            label,
            kind,
            min_width: 0.0,
            shortcut: None,
        }
    }

    pub(crate) fn shortcut(mut self, shortcut: DialogShortcut<'a>) -> Self {
        self.shortcut = Some(shortcut);
        self
    }
}

pub(crate) struct DialogResponse<'a> {
    pub(crate) selected: Option<&'a str>,
    pub(crate) dismissed: bool,
}

pub(crate) struct ActionModal<'a> {
    id: &'a str,
    title: &'a str,
    width: f32,
    actions: &'a [DialogAction<'a>],
}

impl<'a> ActionModal<'a> {
    pub(crate) fn new(id: &'a str, title: &'a str, actions: &'a [DialogAction<'a>]) -> Self {
        Self {
            id,
            title,
            width: style::MODAL_WIDTH_MD,
            actions,
        }
    }

    pub(crate) fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub(crate) fn show(
        self,
        ctx: &egui::Context,
        colors: &Colors,
        add_body: impl FnOnce(&mut egui::Ui),
    ) -> DialogResponse<'a> {
        let mut selected = self
            .actions
            .iter()
            .find(|action| {
                action
                    .shortcut
                    .as_ref()
                    .is_some_and(|shortcut| shortcut.pressed(ctx))
            })
            .map(|action| action.id);

        let shell = ModalShell::centered(self.id)
            .title(self.title)
            .width(self.width);

        let response = shell.show(ctx, colors, |ui| {
            add_body(ui);
            ui.add_space(style::SPACE_MD);
            ui.horizontal(|ui| {
                for action in self.actions {
                    if button::chrome_button(
                        ui,
                        action.label,
                        action.kind,
                        colors,
                        action.min_width,
                    )
                    .clicked()
                    {
                        selected = Some(action.id);
                    }
                    ui.add_space(style::SPACE_SM);
                }
            });

            let hints: Vec<HintGroup<'a>> = self
                .actions
                .iter()
                .filter_map(|action| {
                    action
                        .shortcut
                        .as_ref()
                        .map(|shortcut| HintGroup::new(shortcut.keys, shortcut.label))
                })
                .collect();
            if !hints.is_empty() {
                HintBar::new(&hints).show(ui, colors);
            }
        });

        DialogResponse {
            selected,
            dismissed: response.dismissed,
        }
    }
}
