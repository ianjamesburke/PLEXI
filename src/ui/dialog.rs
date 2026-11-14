use crate::ui::button::{ButtonKind, ButtonSize, KitButton};
use crate::ui::overlay::ModalShell;
use crate::ui::shortcuts;
use crate::ui::style;
use crate::ui::theme::Colors;

/// The keys that trigger an action, and the chips that advertise them. The
/// action's own label is the only prose — a keyboard-first row carries no
/// separate hint text.
#[derive(Clone, Copy)]
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

    /// A guarded action (stints 1498, 1513) takes only a fresh press: an OS
    /// key repeat is consumed without counting. `exact` also refuses extra
    /// modifiers (Shift+Q is not Q).
    fn pressed_fresh(&self, ctx: &egui::Context, exact: bool) -> bool {
        let repeat_only = ctx.input(|i| {
            let mut matching = i.events.iter().filter(|event| {
                matches!(event,
                    egui::Event::Key { key, pressed: true, modifiers, .. }
                        if *key == self.key
                            && if exact {
                                modifiers.matches_exact(self.modifiers)
                            } else {
                                modifiers.matches_logically(self.modifiers)
                            }
                )
            });
            let mut any = false;
            let all_repeat = matching.all(|event| {
                any = true;
                matches!(event, egui::Event::Key { repeat: true, .. })
            });
            any && all_repeat
        });
        let pressed = if exact {
            Self {
                exact: true,
                ..*self
            }
            .pressed(ctx)
        } else {
            self.pressed(ctx)
        };
        pressed && !repeat_only
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
    /// A refusal (deny, cancel): in a consent dialog it works from real input
    /// even before the dialog arms (stint 1498); in a focus trap with no Enter
    /// default, keyboard focus enters at the first refusal (stint 1505).
    refuses: bool,
    /// Loses work (discard, quit without saving, close, overwrite): under
    /// [`ActionModal::destructive_input`] it takes only a fresh, real press
    /// with exact modifiers (stint 1513).
    destructive: bool,
}

impl<'a> DialogAction<'a> {
    /// Mark this action as a refusal; see [`ActionModal::consent_input`].
    pub(crate) fn refuses(mut self) -> Self {
        self.refuses = true;
        self
    }

    /// Mark this action as one that loses work (stint 1513).
    pub(crate) fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    pub(crate) fn new(id: &'a str, label: &'a str, kind: ButtonKind) -> Self {
        Self {
            id,
            label,
            kind,
            shortcut: None,
            alternate: None,
            refuses: false,
            destructive: false,
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
    /// Set for a consent dialog (stint 1498); see [`Self::consent_input`].
    consent_input: Option<crate::app::approval_keyboard::ConsentInput>,
    /// Set for a dialog with destructive actions (stint 1513); see
    /// [`Self::destructive_input`].
    destructive_input: Option<crate::app::approval_keyboard::ConsentInput>,
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
            consent_input: None,
            destructive_input: None,
        }
    }

    /// Keep Tab inside this modal's actions while `key` owns input (stint
    /// 1493), and let Enter or Space activate the Tab-focused action.
    pub(crate) fn focus_surface(mut self, key: crate::ui::focus::SurfaceKey) -> Self {
        self.focus_surface = Some(key);
        self
    }

    /// Whether this modal traps Tab and routes Enter to a focused action.
    #[cfg(test)]
    pub(crate) fn traps_focus(&self) -> bool {
        self.focus_surface.is_some()
    }

    /// Guard this dialog's decisions with the consent input guard (stint
    /// 1498): keys must be fresh presses; nothing counts on a synthetic frame;
    /// a refusal or dismiss counts from real input at once; an approving
    /// action counts only once armed, and is drawn disabled until then.
    pub(crate) fn consent_input(
        mut self,
        input: crate::app::approval_keyboard::ConsentInput,
    ) -> Self {
        self.consent_input = Some(input);
        self
    }

    /// Conventional confirm layout: one right-aligned row of kit buttons in
    /// the order given (left to right), each carrying its shortcut as a dim
    /// hint. Stacks full-width only when the row cannot fit the modal.
    /// Guard this dialog's destructive actions (stint 1513) with the consent
    /// input check in its no-arm-delay mode: a fresh, real press with exact
    /// modifiers; never a key repeat, an injected key or an injected click.
    /// Other actions (save, cancel) stay as they are.
    pub(crate) fn destructive_input(mut self, synthetic_frame: bool) -> Self {
        self.destructive_input =
            Some(crate::app::approval_keyboard::ConsentInput::without_arm_delay(synthetic_frame));
        self
    }

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
        let guarded_destructive =
            |action: &DialogAction<'_>| action.destructive && self.destructive_input.is_some();
        let press = |action: &DialogAction<'_>, shortcut: &DialogShortcut<'_>| {
            if guarded_destructive(action) {
                shortcut.pressed_fresh(ctx, true)
            } else if self.consent_input.is_some() {
                shortcut.pressed_fresh(ctx, false)
            } else {
                shortcut.pressed(ctx)
            }
        };
        // A Tab-focused action takes Enter and Space before any shortcut, so
        // Enter on a focused Cancel cancels instead of firing Close's Enter.
        let previous_ids: Vec<(String, egui::Id)> = ctx
            .data(|d| d.get_temp(action_ids_slot(self.id)))
            .unwrap_or_default();
        // Only a modal that traps focus routes Enter or Space to the focused
        // action; an untrapped modal keys exactly as before (stint 1493 critic).
        let focused_action = ctx
            .memory(|m| m.focused())
            .filter(|_| self.focus_surface.is_some())
            .and_then(|focused| {
                previous_ids
                    .iter()
                    .find(|(_, id)| *id == focused)
                    .and_then(|(name, _)| self.actions.iter().find(|a| a.id == name))
            });
        let mut selected = focused_action
            .filter(|action| {
                if guarded_destructive(action) {
                    take_fresh_activation(ctx)
                } else {
                    ctx.input_mut(|i| {
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                            || i.consume_key(egui::Modifiers::NONE, egui::Key::Space)
                    })
                }
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
                        .is_some_and(|shortcut| press(action, shortcut))
                        || action
                            .alternate
                            .as_ref()
                            .is_some_and(|shortcut| press(action, shortcut))
                })
                .map(|action| action.id);
        }

        if selected.is_none() {
            if let Some(default) = self.default_action {
                let enter =
                    DialogShortcut::new(&["Enter"], egui::Modifiers::NONE, egui::Key::Enter)
                        .exact();
                let default_guarded = self
                    .actions
                    .iter()
                    .any(|action| action.id == default && guarded_destructive(action));
                let pressed = if default_guarded {
                    enter.pressed_fresh(ctx, true)
                } else {
                    enter.pressed(ctx)
                };
                if pressed {
                    selected = self.default_action;
                }
            }
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

            // Rows take Tab focus only inside a focus trap (stint 1493);
            // elsewhere a focused row would take egui's Space/Enter click,
            // which no consent guard sees (stint 1498).
            let focusable = self.focus_surface.is_some();
            for action in self.actions {
                let enabled = self
                    .consent_input
                    .is_none_or(|input| action.refuses || input.armed);
                let response = ui
                    .add_enabled_ui(enabled, |ui| {
                        action_row(ui, action, chip_col_w, colors, focusable)
                    })
                    .inner;
                action_ids.push((action.id.to_string(), response.id));
                if response.clicked() {
                    selected = Some(action.id);
                }
            }
        });
        if let Some(key) = self.focus_surface {
            // Keyboard focus enters at the safe action (stint 1505): the Enter
            // default, else the first refusal (Cancel), else the first action.
            let entry = self.default_action.or_else(|| {
                self.actions
                    .iter()
                    .find(|action| action.refuses)
                    .map(|action| action.id)
            });
            let mut ids: Vec<egui::Id> = action_ids.iter().map(|(_, id)| *id).collect();
            if let Some(at) =
                entry.and_then(|entry| action_ids.iter().position(|(name, _)| name == entry))
            {
                ids.rotate_left(at);
            }
            crate::ui::focus::register_focus_trap(ctx, key, &ids);
        }
        ctx.data_mut(|d| d.insert_temp(action_ids_slot(self.id), action_ids));

        let mut dismissed = response.dismissed;
        if let Some(input) = self.consent_input {
            let refuses = |id: &str| self.actions.iter().any(|a| a.id == id && a.refuses);
            if selected.is_some_and(|id| !input.admits(refuses(id))) {
                log::info!(
                    "consent_guard: modal {} ignored {:?} ({})",
                    self.id,
                    selected,
                    if input.synthetic {
                        "synthetic input"
                    } else {
                        "not armed"
                    }
                );
                selected = None;
            }
            if dismissed && !input.admits(true) {
                log::info!(
                    "consent_guard: modal {} ignored dismiss (synthetic input)",
                    self.id
                );
                dismissed = false;
            }
        }
        // An injected key or click never runs a destructive action (stint 1513).
        if let Some(input) = self.destructive_input {
            let destructive = |id: &str| self.actions.iter().any(|a| a.id == id && a.destructive);
            if selected.is_some_and(|id| destructive(id) && !input.admits(false)) {
                log::info!(
                    "destructive_guard: modal {} ignored {:?} (synthetic input)",
                    self.id,
                    selected
                );
                selected = None;
            }
        }
        DialogResponse {
            selected,
            dismissed,
        }
    }
}

/// A Tab-focused destructive row (stint 1513): every Enter and Space press
/// is removed from the frame, whatever its modifiers or repeat flag, so egui
/// cannot turn one into a click on the focused row; only a fresh press with
/// no modifiers activates it.
fn take_fresh_activation(ctx: &egui::Context) -> bool {
    ctx.input_mut(|i| {
        let mut fresh = false;
        i.events.retain(|event| match event {
            egui::Event::Key {
                key: egui::Key::Enter | egui::Key::Space,
                pressed: true,
                repeat,
                modifiers,
                ..
            } => {
                fresh |= !*repeat && modifiers.matches_exact(egui::Modifiers::NONE);
                false
            }
            _ => true,
        });
        fresh
    })
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
    // Painted right to left, and `action_ids` keeps that order: it is the
    // focus trap's order, the one Tab order (stint 1514), so Tab moves from
    // the primary action on the right towards the left.
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
    focusable: bool,
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
        if focusable {
            egui::Sense::click()
        } else {
            egui::Sense::CLICK
        },
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
            inactive: !ui.is_enabled(),
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
        synthetic_frame: bool,
    ) -> Option<UnsavedChangesChoice> {
        use egui::{Key, Modifiers};
        let discard_label = if self.reload {
            "Reload from disk"
        } else {
            "Discard"
        };
        // Left to right: Discard, Cancel, Save. The row draws right to left,
        // so Tab meets Save (the default), Cancel, then Discard (stint 1505).
        let actions = [
            DialogAction::new("discard", discard_label, ButtonKind::Danger)
                .destructive()
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
            {
                let save = DialogAction::new(
                    "save",
                    if self.conflicted {
                        "Overwrite disk"
                    } else {
                        "Save"
                    },
                    ButtonKind::Primary,
                )
                .shortcut(DialogShortcut::new(&["cmd", "S"], Modifiers::COMMAND, Key::S).exact());
                // "Overwrite disk" replaces changes made outside: destructive.
                if self.conflicted {
                    save.destructive()
                } else {
                    save
                }
            },
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
            .destructive_input(synthetic_frame)
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

    fn key(key: egui::Key, modifiers: egui::Modifiers, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        }
    }

    /// One frame of the dialog with `events`; `shown` false runs the frame
    /// without it (a key held before the dialog appeared).
    fn frame(
        ctx: &egui::Context,
        dialog: &UnsavedChangesDialog,
        synthetic: bool,
        shown: bool,
        events: Vec<egui::Event>,
    ) -> Option<UnsavedChangesChoice> {
        let colors = Colors::from_config(&crate::ui::theme::preset_colors("plexi-dusk").unwrap());
        let modifiers = events
            .iter()
            .find_map(|event| match event {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let mut choice = None;
        let _ = ctx.run_ui(
            egui::RawInput {
                events,
                modifiers,
                ..Default::default()
            },
            |_| {
                if shown {
                    choice = dialog.show(ctx, &colors, synthetic);
                }
            },
        );
        choice
    }

    /// Stint 1513: every destructive action in "Unsaved changes" (Discard by
    /// D and Cmd+D, Reload by R, Overwrite by Enter) takes only a fresh, real
    /// press with exact modifiers; Save stays instant.
    #[test]
    fn unsaved_changes_destructive_actions_take_only_a_fresh_real_press() {
        use egui::{Key, Modifiers};
        let cases = [
            (
                false,
                false,
                Key::D,
                Modifiers::NONE,
                UnsavedChangesChoice::Discard,
            ),
            (
                false,
                false,
                Key::D,
                Modifiers::COMMAND,
                UnsavedChangesChoice::Discard,
            ),
            (
                false,
                true,
                Key::R,
                Modifiers::NONE,
                UnsavedChangesChoice::Reload,
            ),
            (
                true,
                false,
                Key::Enter,
                Modifiers::NONE,
                UnsavedChangesChoice::Overwrite,
            ),
        ];
        for (conflicted, reload, k, mods, expect) in cases {
            let what = format!("{k:?} {mods:?} conflicted={conflicted} reload={reload}");
            let ctx = egui::Context::default();
            crate::ui::theme::setup_fonts(&ctx);
            let dialog = UnsavedChangesDialog {
                path: "/tmp/draft.txt".to_string(),
                conflicted,
                reload,
                error: None,
            };
            for _ in 0..2 {
                frame(&ctx, &dialog, false, true, vec![]);
            }
            let tap = |mods| vec![key(k, mods, true), key(k, mods, false)];
            assert_eq!(
                frame(&ctx, &dialog, true, true, tap(mods)),
                None,
                "injected {what}"
            );
            let extra = mods | Modifiers::SHIFT;
            assert_eq!(
                frame(&ctx, &dialog, false, true, tap(extra)),
                None,
                "extra Shift {what}"
            );
            // Held from before the dialog: the press it sees is a key repeat.
            frame(&ctx, &dialog, false, false, vec![key(k, mods, true)]);
            assert_eq!(
                frame(&ctx, &dialog, false, true, vec![key(k, mods, true)]),
                None,
                "held {what}"
            );
            frame(&ctx, &dialog, false, true, vec![key(k, mods, false)]);
            assert_eq!(
                frame(&ctx, &dialog, false, true, tap(mods)),
                Some(expect),
                "fresh {what}"
            );
        }

        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let dialog = UnsavedChangesDialog {
            path: "/tmp/draft.txt".to_string(),
            conflicted: false,
            reload: false,
            error: None,
        };
        frame(&ctx, &dialog, false, true, vec![]);
        let save = vec![
            key(Key::S, Modifiers::COMMAND, true),
            key(Key::S, Modifiers::COMMAND, false),
        ];
        assert_eq!(
            frame(&ctx, &dialog, true, true, save),
            Some(UnsavedChangesChoice::Save),
            "Save stays instant"
        );

        // Cmd+S is Overwrite disk when conflicted: injected, it is refused.
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let conflicted = UnsavedChangesDialog {
            path: "/tmp/draft.txt".to_string(),
            conflicted: true,
            reload: false,
            error: None,
        };
        frame(&ctx, &conflicted, false, true, vec![]);
        let overwrite = vec![
            key(Key::S, Modifiers::COMMAND, true),
            key(Key::S, Modifiers::COMMAND, false),
        ];
        assert_eq!(
            frame(&ctx, &conflicted, true, true, overwrite),
            None,
            "an injected Cmd+S overwrote the disk"
        );
    }
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
            let rect = action_row(ui, &action, width, &colors, false).rect;
            capture.finish(ui.painter(), rect, colors.bg_sidebar, vec![], vec![]);
        });
        let reports = crate::ui::layout_report::snapshot(&ctx);
        assert!(!reports.is_empty());
        let errors: Vec<_> = reports.iter().flat_map(|r| r.violations()).collect();
        assert!(errors.is_empty(), "{errors:?}");
    }
}
