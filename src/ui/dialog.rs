use crate::ui::button::ButtonKind;
use crate::ui::overlay::ModalShell;
use crate::ui::shortcuts;
use crate::ui::style;
use crate::ui::theme::Colors;

/// The keys that trigger an action, and the chips that advertise them. The
/// action's own label is the only prose — a keyboard-first row carries no
/// separate hint text.
pub(crate) struct DialogShortcut<'a> {
    pub(crate) keys: &'a [&'a str],
    modifiers: egui::Modifiers,
    key: egui::Key,
}

impl<'a> DialogShortcut<'a> {
    pub(crate) fn new(keys: &'a [&'a str], modifiers: egui::Modifiers, key: egui::Key) -> Self {
        Self {
            keys,
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
    pub(crate) shortcut: Option<DialogShortcut<'a>>,
}

impl<'a> DialogAction<'a> {
    pub(crate) fn new(id: &'a str, label: &'a str, kind: ButtonKind) -> Self {
        Self {
            id,
            label,
            kind,
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

            // Every label starts past the widest combo so the chip column and
            // the label column each read as one edge — the modal has a single
            // left alignment from title through body to actions.
            let chip_col_w = self
                .actions
                .iter()
                .map(|action| {
                    action
                        .shortcut
                        .as_ref()
                        .map_or(0.0, |shortcut| shortcuts::key_combo_width(ui, shortcut.keys))
                })
                .fold(0.0_f32, f32::max);

            for action in self.actions {
                if action_row(ui, action, chip_col_w, colors).clicked() {
                    selected = Some(action.id);
                }
            }
        });

        DialogResponse {
            selected,
            dismissed: response.dismissed,
        }
    }
}

/// One full-width action row: its shortcut chips are the primary affordance,
/// the label names what they do, and the whole row is the hit target. Rows
/// share the modal's left edge, so a modal never mixes left-aligned prose
/// with centered chrome.
fn action_row(
    ui: &mut egui::Ui,
    action: &DialogAction<'_>,
    chip_col_w: f32,
    colors: &Colors,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), style::MODAL_ACTION_ROW_H),
        egui::Sense::click(),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        // The fill bleeds into the modal's gutter so the chips can sit flush
        // with the title and body text. Indenting the chips instead would
        // reintroduce a second left edge inside the modal.
        ui.painter().rect_filled(
            rect.expand2(egui::vec2(style::SPACE_XS, 0.0)),
            style::RADIUS_SM,
            colors.bg_hover,
        );
    }

    let center_y = rect.center().y;
    let chip_x = rect.left();
    if let Some(shortcut) = &action.shortcut {
        shortcuts::key_combo_painted(ui, chip_x, center_y, shortcut.keys, colors);
    }

    // Destructive actions stay distinct without a filled button: the label
    // carries the danger color while the row geometry matches its siblings.
    let label_color = match action.kind {
        ButtonKind::Danger => colors.danger,
        _ => colors.text_primary,
    };
    let galley = ui.fonts_mut(|f| {
        f.layout_no_wrap(
            action.label.to_string(),
            crate::ui::theme::font_medium(style::TEXT_CAPTION),
            label_color,
        )
    });
    let label_pos = egui::Pos2::new(
        chip_x + chip_col_w + style::KEYCHIP_DESC_GAP,
        center_y - galley.size().y / 2.0,
    );
    crate::ui::snap::galley_snapped(ui.painter(), label_pos, galley, label_color);

    response
}
