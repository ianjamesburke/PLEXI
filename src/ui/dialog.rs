use crate::ui::button::{ButtonKind, ButtonSize, KitButton};
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
    button_row: bool,
    focus_surface: Option<crate::ui::focus::SurfaceKey>,
}

/// Memory slot holding the egui ids of a modal's actions from last frame, so
/// this frame can tell which action holds keyboard focus before it draws.
fn action_ids_slot(modal: &str) -> egui::Id {
    egui::Id::new(("plexi_action_modal_ids", modal))
}

impl<'a> ActionModal<'a> {
    pub(crate) fn new(id: &'a str, title: &'a str, actions: &'a [DialogAction<'a>]) -> Self {
        Self {
            id,
            title,
            width: style::MODAL_WIDTH_MD,
            actions,
            default_action: None,
            button_row: false,
            focus_surface: None,
        }
    }

    /// Keep Tab inside this modal's actions while `key` owns input (stint
    /// 1493), and let Enter or Space activate the Tab-focused action.
    pub(crate) fn focus_surface(mut self, key: crate::ui::focus::SurfaceKey) -> Self {
        self.focus_surface = Some(key);
        self
    }

    /// Conventional confirm layout: one right-aligned row of kit buttons in
    /// the order given (left to right), each carrying its shortcut as a dim
    /// hint. Stacks full-width only when the row cannot fit the modal.
    pub(crate) fn button_row(mut self) -> Self {
        self.button_row = true;
        self
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
        // A Tab-focused action takes Enter and Space before any shortcut, so
        // Enter on a focused Cancel cancels instead of firing Close's Enter.
        let previous_ids: Vec<(String, egui::Id)> = ctx
            .data(|d| d.get_temp(action_ids_slot(self.id)))
            .unwrap_or_default();
        let focused_action = ctx.memory(|m| m.focused()).and_then(|focused| {
            previous_ids
                .iter()
                .find(|(_, id)| *id == focused)
                .and_then(|(name, _)| self.actions.iter().find(|a| a.id == name))
        });
        let mut selected = focused_action
            .filter(|_| {
                ctx.input_mut(|i| {
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                        || i.consume_key(egui::Modifiers::NONE, egui::Key::Space)
                })
            })
            .map(|action| {
                log::info!(
                    "ui: modal {} activated focused action {}",
                    self.id,
                    action.id
                );
                action.id
            });
        if selected.is_none() {
            selected = self
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
        }

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

        let mut action_ids: Vec<(String, egui::Id)> = Vec::new();
        let response = shell.show(ctx, colors, |ui| {
            add_body(ui);
            ui.add_space(style::SPACE_MD);

            if self.button_row {
                ui.add_space(style::SPACE_SM);
                if let Some(id) = button_row(ui, self.actions, colors, &mut action_ids) {
                    selected = Some(id);
                }
                return;
            }

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
                let response = action_row(ui, action, chip_col_w, colors);
                action_ids.push((action.id.to_string(), response.id));
                if response.clicked() {
                    selected = Some(action.id);
                }
            }
        });
        if let Some(key) = self.focus_surface {
            let ids: Vec<egui::Id> = action_ids.iter().map(|(_, id)| *id).collect();
            crate::ui::focus::register_focus_trap(ctx, key, &ids);
        }
        ctx.data_mut(|d| d.insert_temp(action_ids_slot(self.id), action_ids));

        DialogResponse {
            selected,
            dismissed: response.dismissed,
        }
    }
}

/// The shortcut spelled as one compact hint, e.g. `⌘S`, `D`, `Esc`.
fn hint_text(shortcut: &DialogShortcut<'_>) -> String {
    shortcuts::display_tokens(shortcut.keys).concat()
}

fn confirm_button<'b>(
    action: &'b DialogAction<'_>,
    hint: &'b str,
    colors: &Colors,
) -> KitButton<'b> {
    let mut button = KitButton::new(action.label, colors)
        .kind(action.kind)
        .size(ButtonSize::Md);
    if !hint.is_empty() {
        button = button.hint(hint);
    }
    button
}

/// Right-aligned row of kit buttons, left to right as `actions` are ordered.
/// The layout depends only on the labels and the available width, never on
/// hover or press, so no state moves a button. A row that cannot fit stacks
/// full-width with the last action (the primary one) on top.
fn button_row<'a>(
    ui: &mut egui::Ui,
    actions: &'a [DialogAction<'a>],
    colors: &Colors,
    action_ids: &mut Vec<(String, egui::Id)>,
) -> Option<&'a str> {
    let hints: Vec<String> = actions
        .iter()
        .map(|action| action.shortcut.as_ref().map(hint_text).unwrap_or_default())
        .collect();
    let widths: Vec<f32> = actions
        .iter()
        .zip(&hints)
        .map(|(action, hint)| confirm_button(action, hint, colors).natural_width(ui))
        .collect();
    let needed =
        widths.iter().sum::<f32>() + style::SPACE_SM * widths.len().saturating_sub(1) as f32;
    let stacked = needed > ui.available_width();
    if stacked {
        static STACKED: std::sync::Once = std::sync::Once::new();
        STACKED.call_once(|| log::info!("ui: stacking confirm buttons, row does not fit"));
    }
    let capture = super::layout_report::Capture::begin(ui.painter(), "modal_action");
    let mut selected = None;
    let mut reports = Vec::new();
    let start = ui.cursor().min;
    let mut add = |ui: &mut egui::Ui, index: usize, full_width: bool| {
        let action = &actions[index];
        let mut button = confirm_button(action, &hints[index], colors);
        if full_width {
            button = button.min_width(ui.available_width());
        }
        let response = ui.add(button);
        action_ids.push((action.id.to_string(), response.id));
        reports.push(super::layout_report::Action {
            id: action.id.into(),
            name: action.label.into(),
            painted_label: action.label.into(),
            hit: response.rect,
            minimum: egui::Vec2::splat(style::BUTTON_H_MD),
            inactive: false,
        });
        if response.clicked() {
            selected = Some(action.id);
        }
    };
    if stacked {
        ui.spacing_mut().item_spacing.y = style::SPACE_SM;
        for index in (0..actions.len()).rev() {
            add(ui, index, true);
        }
    } else {
        // One button-high row: a bare `with_layout` takes the whole remaining
        // height and centres the buttons in it, which leaves a tall gap in a
        // modal whose body has a window-sized height budget.
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), style::BUTTON_H_MD),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = style::SPACE_SM;
                for index in (0..actions.len()).rev() {
                    add(ui, index, false);
                }
            },
        );
    }
    // Painted right to left; Tab order is the given (left to right) order.
    action_ids.sort_by_key(|(id, _)| actions.iter().position(|a| a.id == id));
    let rect = egui::Rect::from_min_max(start, ui.min_rect().max);
    capture.finish(ui.painter(), rect, colors.bg_sidebar, reports, vec![]);
    selected
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
    if response.has_focus() {
        // The kit focus ring (stint 1460), drawn inside so the modal never
        // clips it.
        ui.painter().rect_stroke(
            rect.expand2(egui::vec2(style::SPACE_XS, 0.0)),
            style::RADIUS_SM,
            egui::Stroke::new(crate::ui::button::FOCUS_RING_WIDTH, colors.accent),
            egui::StrokeKind::Inside,
        );
    }
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
            inactive: false,
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
        let discard_label = if self.reload {
            "Reload from disk"
        } else {
            "Discard"
        };
        let actions = [
            DialogAction::new("cancel", "Cancel", ButtonKind::Secondary)
                .shortcut(DialogShortcut::new(&["Esc"], Modifiers::NONE, Key::Escape).exact()),
            DialogAction::new("discard", discard_label, ButtonKind::Danger)
                .shortcut(if self.reload {
                    DialogShortcut::new(&["R"], Modifiers::NONE, Key::R).exact()
                } else {
                    DialogShortcut::new(&["D"], Modifiers::NONE, Key::D).exact()
                })
                .alternate((!self.reload).then(|| {
                    DialogShortcut::new(&["cmd", "D"], Modifiers::COMMAND, Key::D).exact()
                })),
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
        ];
        let name = std::path::Path::new(&self.path)
            .file_name()
            .map_or_else(|| self.path.clone(), |n| n.to_string_lossy().into_owned());
        let question = if self.conflicted {
            format!("{name} changed on disk. Overwriting replaces those changes.")
        } else {
            format!("Save changes to {name}?")
        };
        let response = ActionModal::new("unsaved_changes", "Unsaved changes", &actions)
            .focus_surface(crate::ui::focus::SurfaceKey::Overlay(
                crate::app::input_owner::OverlaySurface::Layer(
                    crate::app::FocusKind::UnsavedChanges,
                ),
            ))
            .width(style::MODAL_WIDTH_SM)
            .default_action("save")
            .button_row()
            .show(ctx, colors, |ui| {
                super::typography::modal_body(ui, &question, colors);
                let path = super::text::elide(
                    ui,
                    &self.path,
                    egui::FontId::proportional(style::TEXT_HINT),
                    ui.available_width(),
                    super::text::Side::Middle,
                );
                super::typography::caption(ui, path, colors);
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
