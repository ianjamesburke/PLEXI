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
    exact: bool,
}

impl<'a> DialogShortcut<'a> {
    pub(crate) fn new(keys: &'a [&'a str], modifiers: egui::Modifiers, key: egui::Key) -> Self {
        Self {
            keys,
            modifiers,
            key,
            exact: false,
        }
    }

    fn exact(mut self) -> Self {
        self.exact = true;
        self
    }

    fn pressed(&self, ctx: &egui::Context) -> bool {
        ctx.input_mut(|i| {
            if self.exact
                && !i.events.iter().any(|event| {
                    matches!(event,
                        egui::Event::Key { key, pressed: true, modifiers, .. }
                            if *key == self.key && modifiers.matches_exact(self.modifiers)
                    )
                })
            {
                return false;
            }
            i.consume_key(self.modifiers, self.key)
        })
    }
}

pub(crate) struct DialogAction<'a> {
    pub(crate) id: &'a str,
    pub(crate) label: &'a str,
    pub(crate) kind: ButtonKind,
    pub(crate) shortcut: Option<DialogShortcut<'a>>,
    alternate: Option<DialogShortcut<'a>>,
}

impl<'a> DialogAction<'a> {
    pub(crate) fn new(id: &'a str, label: &'a str, kind: ButtonKind) -> Self {
        Self {
            id,
            label,
            kind,
            shortcut: None,
            alternate: None,
        }
    }

    pub(crate) fn shortcut(mut self, shortcut: DialogShortcut<'a>) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    fn alternate(mut self, shortcut: Option<DialogShortcut<'a>>) -> Self {
        self.alternate = shortcut;
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
    default_action: Option<&'a str>,
}

impl<'a> ActionModal<'a> {
    pub(crate) fn new(id: &'a str, title: &'a str, actions: &'a [DialogAction<'a>]) -> Self {
        Self {
            id,
            title,
            width: style::MODAL_WIDTH_MD,
            actions,
            default_action: None,
        }
    }

    pub(crate) fn default_action(mut self, id: &'a str) -> Self {
        self.default_action = Some(id);
        self
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
                    || action
                        .alternate
                        .as_ref()
                        .is_some_and(|shortcut| shortcut.pressed(ctx))
            })
            .map(|action| action.id);

        if selected.is_none()
            && self.default_action.is_some()
            && DialogShortcut::new(&["Enter"], egui::Modifiers::NONE, egui::Key::Enter)
                .exact()
                .pressed(ctx)
        {
            selected = self.default_action;
        }

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
                    action.shortcut.as_ref().map_or(0.0, |shortcut| {
                        shortcuts::key_combo_width(ui, shortcut.keys)
                    })
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
    let keys = action
        .shortcut
        .as_ref()
        .map(|s| shortcuts::display_tokens(s.keys))
        .unwrap_or_default();
    let chip_h = shortcuts::chip_row_height(ui);
    let widths: Vec<_> = keys
        .iter()
        .map(|key| shortcuts::key_combo_width(ui, &[key.as_str()]))
        .collect();
    ui.set_min_width(widths.iter().copied().fold(style::BUTTON_H_MD, f32::max));
    let width = ui.available_width();
    let bounds = egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(width, style::MODAL_ACTION_ROW_H),
    );
    let slots = [super::bounded::Slot {
        id: "shortcut",
        width: chip_col_w,
        compact: None,
        required: true,
    }];
    let lane = super::bounded::allocate(
        bounds,
        style::PANE_HEADER_TITLE_MIN,
        style::KEYCHIP_DESC_GAP,
        &slots,
    );
    let inline = lane
        .as_ref()
        .is_ok_and(|r| r.text.width() >= style::PANE_HEADER_TITLE_MIN);
    let label_width = if inline {
        lane.as_ref().map_or(width, |r| r.text.width())
    } else {
        width
    };
    let galley = ui.fonts_mut(|f| {
        f.layout(
            action.label.into(),
            crate::ui::theme::font_medium(style::TEXT_CAPTION),
            egui::Color32::PLACEHOLDER,
            label_width,
        )
    });
    let mut key_positions = Vec::new();
    let (mut x, mut y) = (0.0, 0.0);
    for chip_width in widths {
        if x > 0.0 && x + chip_width > width {
            x = 0.0;
            y += chip_h + style::SPACE_XS;
        }
        key_positions.push(egui::vec2(x, y));
        x += chip_width + style::KEYCHIP_GAP;
    }
    let keys_h = if keys.is_empty() { 0.0 } else { y + chip_h };
    let label_offset = if inline {
        chip_col_w + style::KEYCHIP_DESC_GAP
    } else {
        0.0
    };
    let stacked_gap = if keys.is_empty() {
        0.0
    } else {
        style::SPACE_SM
    };
    let height = if inline {
        galley.size().y.max(chip_h) + style::SPACE_XS * 2.0
    } else {
        keys_h + stacked_gap + galley.size().y + style::SPACE_XS * 2.0
    };
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(
            width,
            height
                .max(style::MODAL_ACTION_ROW_H)
                .max(style::BUTTON_H_MD),
        ),
        egui::Sense::click(),
    );
    let mut capture = super::layout_report::Capture::begin(ui.painter(), "modal_action");
    let background = if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        ui.painter().rect_filled(
            rect.expand2(egui::vec2(style::SPACE_XS, 0.0)),
            style::RADIUS_SM,
            colors.bg_hover,
        );
        colors.bg_hover
    } else {
        colors.bg_sidebar
    };
    let key_top = if inline {
        rect.center().y - chip_h / 2.0
    } else {
        rect.top() + style::SPACE_XS
    };
    for (key, offset) in keys.iter().zip(key_positions) {
        shortcuts::paint_key_chip(
            ui.painter(),
            rect.left() + offset.x,
            key_top + offset.y + chip_h / 2.0,
            key,
            colors,
        );
    }
    let preferred = if matches!(action.kind, ButtonKind::Danger) {
        colors.danger
    } else {
        colors.text_primary
    };
    let color = super::foreground::Surface::over(colors.bg_sidebar, background)
        .foreground(preferred, super::foreground::Role::SmallText);
    let label_y = if inline {
        rect.center().y - galley.size().y / 2.0
    } else {
        rect.top() + style::SPACE_XS + keys_h + stacked_gap
    };
    let label_pos = egui::pos2(rect.left() + label_offset, label_y);
    capture.text_slot(
        "label",
        &galley,
        egui::Rect::from_min_size(label_pos, egui::vec2(label_width, galley.size().y)),
    );
    ui.painter()
        .galley_with_override_text_color(label_pos, galley, color);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), action.label)
    });
    if !inline {
        static REFLOW: std::sync::Once = std::sync::Once::new();
        REFLOW.call_once(|| log::info!("ui: reflowing modal action below shortcut chips"));
    }
    capture.finish(
        ui.painter(),
        rect,
        colors.bg_sidebar,
        vec![super::layout_report::Action {
            id: "label".into(),
            name: action.label.into(),
            painted_label: action.label.into(),
            hit: response.rect,
            minimum: egui::Vec2::splat(style::BUTTON_H_MD),
        }],
        vec![],
    );
    response
}

/// A pane's pending file-loss decision, rendered by the host focus owner.
#[derive(Clone, Debug)]
pub struct UnsavedChangesDialog {
    pub path: String,
    pub conflicted: bool,
    pub reload: bool,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsavedChangesChoice {
    Save,
    Overwrite,
    Discard,
    Reload,
    Cancel,
}

impl UnsavedChangesDialog {
    pub(crate) fn show(
        &self,
        ctx: &egui::Context,
        colors: &Colors,
    ) -> Option<UnsavedChangesChoice> {
        use egui::{Key, Modifiers};
        let actions =
            [
                DialogAction::new(
                    "save",
                    if self.conflicted {
                        "Overwrite disk"
                    } else {
                        "Save"
                    },
                    ButtonKind::Primary,
                )
                .shortcut(DialogShortcut::new(&["cmd", "S"], Modifiers::COMMAND, Key::S).exact()),
                DialogAction::new(
                    "discard",
                    if self.reload {
                        "Reload from disk"
                    } else {
                        "Discard edits"
                    },
                    ButtonKind::Danger,
                )
                .shortcut(if self.reload {
                    DialogShortcut::new(&["R"], Modifiers::NONE, Key::R).exact()
                } else {
                    DialogShortcut::new(&["D"], Modifiers::NONE, Key::D).exact()
                })
                .alternate((!self.reload).then(|| {
                    DialogShortcut::new(&["cmd", "D"], Modifiers::COMMAND, Key::D).exact()
                })),
                DialogAction::new("cancel", "Cancel", ButtonKind::Secondary)
                    .shortcut(DialogShortcut::new(&["Esc"], Modifiers::NONE, Key::Escape).exact()),
            ];
        let response = ActionModal::new("unsaved_changes", "Unsaved changes", &actions)
            .default_action("save")
            .show(ctx, colors, |ui| {
                super::typography::body(ui, &self.path, colors);
                super::typography::caption(
                    ui,
                    if self.conflicted {
                        "The file changed on disk. Overwriting replaces those changes."
                    } else {
                        "Save your changes before leaving this document?"
                    },
                    colors,
                );
                if let Some(error) = &self.error {
                    super::typography::caption(ui, error, colors);
                }
            });
        match response.selected {
            Some("save") => Some(if self.conflicted {
                UnsavedChangesChoice::Overwrite
            } else {
                UnsavedChangesChoice::Save
            }),
            Some("discard") => Some(if self.reload {
                UnsavedChangesChoice::Reload
            } else {
                UnsavedChangesChoice::Discard
            }),
            Some("cancel") => Some(UnsavedChangesChoice::Cancel),
            _ if response.dismissed => Some(UnsavedChangesChoice::Cancel),
            _ => None,
        }
    }
}

#[cfg(test)]
mod adoption_tests {
    use super::*;
    #[test]
    fn adoption_modal_keeps_pointer_and_shortcut_activation() {
        for keyboard in [false, true] {
            let ctx = egui::Context::default();
            crate::ui::theme::setup_fonts(&ctx);
            let colors =
                Colors::from_config(&crate::ui::theme::preset_colors("plexi-dusk").unwrap());
            let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
            let actions = [DialogAction::new(
                "restore",
                "Restore and open the complete document",
                ButtonKind::Primary,
            )
            .shortcut(DialogShortcut::new(
                &["Enter"],
                egui::Modifiers::NONE,
                egui::Key::Enter,
            ))];
            let mut hit = egui::Rect::NOTHING;
            let mut selected = false;
            for stage in 0..3 {
                let mut input = egui::RawInput::default();
                if stage > 0 {
                    if keyboard {
                        input.events.push(egui::Event::Key {
                            key: egui::Key::Enter,
                            physical_key: None,
                            pressed: stage == 1,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        });
                    } else {
                        input.events.push(egui::Event::PointerMoved(hit.center()));
                        input.events.push(egui::Event::PointerButton {
                            pos: hit.center(),
                            button: egui::PointerButton::Primary,
                            pressed: stage == 1,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                }
                let _ = ctx.run_ui(input, |_| {
                    selected |= ActionModal::new("reflow-action", "Restore", &actions)
                        .width(160.0)
                        .show(&ctx, &colors, |_| {})
                        .selected
                        == Some("restore");
                });
                hit = crate::ui::layout_report::snapshot(&ctx)
                    .iter()
                    .find(|r| r.surface == "modal_action")
                    .unwrap()
                    .actions[0]
                    .hit;
            }
            assert!(selected, "keyboard={keyboard}");
        }
    }

    #[test]
    fn adoption_modal_action_wraps_full_label() {
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let colors = crate::ui::theme::Colors::from_config(
            &crate::ui::theme::preset_colors("plexi-dusk").unwrap(),
        );
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(160.0);
            ui.set_clip_rect(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(180.0, 800.0),
            ));
            let capture =
                crate::ui::layout_report::Capture::begin(ui.painter(), "adoption-fixture");
            let action = DialogAction::new(
                "restore",
                "Wiederherstellen und in einem neuen Fenster öffnen",
                ButtonKind::Danger,
            )
            .shortcut(DialogShortcut::new(
                &["cmd", "shift", "enter"],
                egui::Modifiers::COMMAND,
                egui::Key::Enter,
            ));
            let width = shortcuts::key_combo_width(ui, action.shortcut.as_ref().unwrap().keys);
            let rect = action_row(ui, &action, width, &colors).rect;
            capture.finish(ui.painter(), rect, colors.bg_sidebar, vec![], vec![]);
        });
        let reports = crate::ui::layout_report::snapshot(&ctx);
        assert!(!reports.is_empty());
        let errors: Vec<_> = reports.iter().flat_map(|r| r.violations()).collect();
        assert!(errors.is_empty(), "{errors:?}");
    }
}
