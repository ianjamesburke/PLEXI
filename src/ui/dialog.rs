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

    fn matches(&self, key: egui::Key, modifiers: egui::Modifiers) -> bool {
        key == self.key
            && if self.exact {
                modifiers.matches_exact(self.modifiers)
            } else {
                modifiers.matches_logically(self.modifiers)
            }
    }

    fn pressed(&self, ctx: &egui::Context) -> bool {
        ctx.input_mut(|i| {
            if self.exact
                && !i.events.iter().any(|event| {
                    matches!(event,
                        egui::Event::Key { key, pressed: true, modifiers, .. }
                            if self.matches(*key, *modifiers)
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
    /// [`ActionModal::destructive_input`] it is greyed through the arm delay,
    /// then takes only a fresh, real press with exact modifiers (stint 1513).
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

/// A text field body for [`ActionModal::show_field`] (stint 1559). The kit
/// owns the field's focus surface, select-all on its first focus in the
/// owner's open `session`, and its place above the button row.
pub(crate) struct DialogField<'b> {
    id: egui::Id,
    placeholder: &'b str,
    surface: crate::ui::focus::SurfaceKey,
    session: crate::ui::text_field::OpenSession,
    /// `None` is a single-line field; `Some(rows)` a multi-line one.
    rows: Option<usize>,
    buffer: &'b mut String,
}

impl<'b> DialogField<'b> {
    pub(crate) fn singleline(
        id: egui::Id,
        placeholder: &'b str,
        surface: crate::ui::focus::SurfaceKey,
        session: crate::ui::text_field::OpenSession,
        buffer: &'b mut String,
    ) -> Self {
        Self {
            id,
            placeholder,
            surface,
            session,
            rows: None,
            buffer,
        }
    }

    pub(crate) fn multiline(
        id: egui::Id,
        placeholder: &'b str,
        surface: crate::ui::focus::SurfaceKey,
        session: crate::ui::text_field::OpenSession,
        rows: usize,
        buffer: &'b mut String,
    ) -> Self {
        Self {
            rows: Some(rows),
            ..Self::singleline(id, placeholder, surface, session, buffer)
        }
    }

    fn show(self, ui: &mut egui::Ui, log_name: &str, colors: &Colors) {
        match self.rows {
            None => {
                crate::ui::text_field::TextField::singleline(self.id, self.placeholder)
                    .surface(self.surface)
                    .select_all_on_open(self.session)
                    .log_name(log_name)
                    .show(ui, self.buffer, colors);
            }
            Some(rows) => {
                crate::ui::text_field::TextArea::multiline(self.id, self.placeholder)
                    .font(egui::TextStyle::Body.resolve(ui.style()))
                    .desired_width(ui.available_width())
                    .rows(rows)
                    .surface(self.surface)
                    .select_all_on_open(self.session)
                    .log_name(log_name)
                    .show(ui, self.buffer, colors);
            }
        }
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
    focus_surface: Option<crate::ui::focus::SurfaceKey>,
    /// Set for a consent dialog (stint 1498); see [`Self::consent_input`].
    consent_input: Option<crate::broker::consent::ConsentInput>,
    /// `(synthetic frame, request key)` for a dialog with destructive actions
    /// (stint 1513); see [`Self::destructive_input`].
    destructive_request: Option<(bool, u64)>,
    /// See [`Self::banner`].
    banner: bool,
}

/// Memory slot holding the egui ids of a modal's actions from last frame, so
/// this frame can tell which action holds keyboard focus before it draws.
fn action_ids_slot(modal: &str) -> egui::Id {
    egui::Id::new(("plexi_action_modal_ids", modal))
}

/// How long after the window regains focus a dialog ignores activation keys
/// (stint 1785). It outlasts a 225 ms macOS initial key-repeat delay
/// (InitialKeyRepeat 15), so a key held through the blur repeats inside it;
/// a deliberate press after a refocus rarely comes sooner.
const REFOCUS_GUARD: std::time::Duration = std::time::Duration::from_millis(500);

/// The arm-delay guard of each modal's destructive actions, by modal id.
type DestructiveGuards = std::collections::HashMap<String, crate::broker::consent::ConsentGuard>;

fn destructive_guards_slot() -> egui::Id {
    egui::Id::new("plexi_action_modal_destructive_guards")
}

#[cfg(test)]
fn destructive_frame_epoch_slot() -> egui::Id {
    egui::Id::new("plexi_action_modal_destructive_frame_epoch")
}

/// Test hook: drive the arm delay from egui input time on this context.
/// Set this before the dialog first draws so the guard uses one time domain.
#[cfg(test)]
pub(crate) fn use_destructive_frame_clock_for_test(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with(destructive_frame_epoch_slot(), std::time::Instant::now);
    });
}

/// Test hook: every destructive dialog on screen has waited out the arm
/// delay, so the next real press is a deliberate one.
#[cfg(test)]
pub(crate) fn arm_destructive_dialogs_for_test(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        for guard in d
            .get_temp_mut_or_default::<DestructiveGuards>(destructive_guards_slot())
            .values_mut()
        {
            guard.backdate(crate::broker::consent::ARM_DELAY * 2);
        }
    });
}

impl<'a> ActionModal<'a> {
    pub(crate) fn new(id: &'a str, title: &'a str, actions: &'a [DialogAction<'a>]) -> Self {
        Self {
            id,
            title,
            width: style::MODAL_WIDTH_MD,
            actions,
            default_action: None,
            focus_surface: None,
            consent_input: None,
            destructive_request: None,
            banner: false,
        }
    }

    /// A non-blocking dialog (stint 1560): it sits at the bottom of the
    /// window with no scrim, so the workspace stays usable and a click
    /// outside never dismisses it.
    pub(crate) fn banner(mut self) -> Self {
        self.banner = true;
        self
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
        input: crate::broker::consent::ConsentInput,
    ) -> Self {
        self.consent_input = Some(input);
        self
    }

    /// Guard this dialog's destructive actions (stint 1513) with the consent
    /// guard: greyed for the arm delay after `request` first draws, then a
    /// fresh, real press with exact modifiers; never a key repeat, an injected
    /// key or an injected click. A different `request` is a replaced dialog
    /// and arms anew. Other actions (save, cancel) stay as they are.
    pub(crate) fn destructive_input(
        mut self,
        synthetic_frame: bool,
        request: impl std::hash::Hash,
    ) -> Self {
        self.destructive_request = Some((synthetic_frame, egui::Id::new(request).value()));
        self
    }

    /// This frame's destructive input, from the guard kept for this modal.
    fn present_destructive(
        &self,
        ctx: &egui::Context,
    ) -> Option<crate::broker::consent::ConsentInput> {
        let (synthetic, request) = self.destructive_request?;
        #[cfg(not(test))]
        let now = std::time::Instant::now();
        #[cfg(test)]
        let now = ctx
            .data(|d| d.get_temp::<std::time::Instant>(destructive_frame_epoch_slot()))
            .map_or_else(std::time::Instant::now, |epoch| {
                epoch + std::time::Duration::from_secs_f64(ctx.input(|i| i.time))
            });
        let pass = ctx.cumulative_pass_nr();
        let (input, remaining) = ctx.data_mut(|d| {
            let guard = d
                .get_temp_mut_or_default::<DestructiveGuards>(destructive_guards_slot())
                .entry(self.id.to_string())
                .or_default();
            guard.present(request, pass, now);
            (guard.input(now, synthetic), guard.remaining(now))
        });
        if !input.armed {
            let frame = std::time::Duration::from_secs_f32(ctx.input(|i| i.predicted_dt));
            ctx.request_repaint_after(remaining + frame);
        }
        Some(input)
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
        self.show_inner(ctx, colors, None, add_body)
    }

    /// A text-entry dialog (stint 1559): `field` above the button row. The
    /// dialog hangs below the top of the window with no scrim, so a stray
    /// click never throws away typed text, and it traps Tab in the order
    /// field, then the actions as given; the field takes focus on open.
    pub(crate) fn show_field(
        self,
        ctx: &egui::Context,
        colors: &Colors,
        field: DialogField<'_>,
    ) -> DialogResponse<'a> {
        self.show_inner(ctx, colors, Some(field), |_| {})
    }

    fn show_inner(
        self,
        ctx: &egui::Context,
        colors: &Colors,
        field: Option<DialogField<'_>>,
        add_body: impl FnOnce(&mut egui::Ui),
    ) -> DialogResponse<'a> {
        let field_id = field.as_ref().map(|field| field.id);
        let trap_key = self
            .focus_surface
            .or(field.as_ref().map(|field| field.surface));
        let destructive_input = self.present_destructive(ctx);
        let guarded_destructive =
            |action: &DialogAction<'_>| action.destructive && destructive_input.is_some();
        let action_shortcut = |action: &DialogAction<'a>, shortcut: DialogShortcut<'a>| {
            // A consent dialog matches modifiers exactly: Shift+Enter
            // ("Always allow") must never be taken by Enter ("Grant once"),
            // which `matches_logically` would allow (stint 1489).
            if guarded_destructive(action) || self.consent_input.is_some() {
                shortcut.exact()
            } else {
                shortcut
            }
        };
        let default_shortcut =
            DialogShortcut::new(&["Enter"], egui::Modifiers::NONE, egui::Key::Enter).exact();
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
        // egui clears held keys on blur, so a key held through a refocus
        // reads as a fresh press (stint 1785). For REFOCUS_GUARD after the
        // window regains focus, every activation press is dropped like a repeat.
        let now = ctx.input(|i| i.time);
        let refocus_slot = egui::Id::new(("plexi_action_modal_refocus", self.id));
        if ctx.input(|i| i.events.contains(&egui::Event::WindowFocused(true))) {
            ctx.data_mut(|d| d.insert_temp(refocus_slot, now));
        }
        let refocusing = ctx
            .data(|d| d.get_temp::<f64>(refocus_slot))
            .is_some_and(|at| now - at < REFOCUS_GUARD.as_secs_f64());
        // egui tracks held keys across frames, including across modal changes.
        // Drop repeats before either shortcut dispatch or egui button activation.
        // A field can still repeat typing, including spaces and newlines.
        ctx.input_mut(|input| {
            input.events.retain(|event| {
                let egui::Event::Key {
                    key,
                    pressed: true,
                    repeat,
                    modifiers,
                    ..
                } = event
                else {
                    return true;
                };
                if !(*repeat || refocusing) {
                    return true;
                }
                let button_activation = matches!(key, egui::Key::Enter | egui::Key::Space)
                    && (field_id.is_none() || focused_action.is_some());
                let default_activation =
                    self.default_action.is_some() && default_shortcut.matches(*key, *modifiers);
                let activation = *key == egui::Key::Escape
                    || button_activation
                    || default_activation
                    || self.actions.iter().any(|action| {
                        action
                            .shortcut
                            .iter()
                            .chain(&action.alternate)
                            .any(|shortcut| {
                                action_shortcut(action, *shortcut).matches(*key, *modifiers)
                            })
                    });
                if activation && !*repeat {
                    log::info!("ui: modal {} ignored {key:?} just after refocus", self.id);
                }
                !activation
            });
        });
        // Only a modal that traps focus explicitly routes Enter or Space to
        // the focused action.
        let mut selected = focused_action
            .filter(|_| trap_key.is_some())
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
                        .iter()
                        .chain(&action.alternate)
                        .any(|shortcut| action_shortcut(action, *shortcut).pressed(ctx))
                })
                .map(|action| action.id);
        }

        if selected.is_none() && self.default_action.is_some() && default_shortcut.pressed(ctx) {
            selected = self.default_action;
        }

        let mut shell = ModalShell::centered(self.id)
            .title(self.title)
            .width(self.width);
        if field.is_some() {
            shell = shell
                .anchor(
                    egui::Align2::CENTER_TOP,
                    egui::vec2(0.0, style::FIELD_MODAL_TOP),
                )
                .scrim(false);
        } else if self.banner {
            shell = shell
                .anchor(
                    egui::Align2::CENTER_BOTTOM,
                    egui::vec2(0.0, -style::SPACE_MD),
                )
                .scrim(false);
        }

        let mut action_ids: Vec<(String, egui::Id)> = Vec::new();
        let response = shell.show(ctx, colors, |ui| {
            add_body(ui);
            if let Some(field) = field {
                field.show(ui, self.id, colors);
            }
            ui.add_space(style::SPACE_MD + style::SPACE_SM);
            let row = ButtonRow {
                modal: self.id,
                actions: self.actions,
                colors,
                // A guarded dialog's buttons take Tab focus only inside a
                // focus trap (stint 1493); elsewhere a focused button would
                // take egui's Space/Enter click, which no guard sees (stints
                // 1498, 1513).
                focusable: trap_key.is_some()
                    || (self.consent_input.is_none() && destructive_input.is_none()),
                // A consent dialog draws its approving actions disabled until
                // it arms (stint 1498).
                consent: self.consent_input,
                destructive: destructive_input,
                surface: None,
            };
            if let Some(id) = row.show(ui, &mut action_ids).selected {
                selected = Some(id);
            }
        });
        if let Some(key) = trap_key {
            // A greyed action takes no focus, so Tab skips it until it arms.
            let waiting = |name: &str| {
                destructive_input.is_some_and(|input| !input.armed)
                    && self.actions.iter().any(|a| a.id == name && a.destructive)
            };
            action_ids.retain(|(name, _)| !waiting(name));
            let mut ids: Vec<egui::Id> = action_ids.iter().map(|(_, id)| *id).collect();
            if let Some(field_id) = field_id {
                // A field dialog enters at its field, then the actions as given.
                ids.insert(0, field_id);
            } else {
                // Keyboard focus enters at the safe action (stint 1505): the
                // Enter default, else the first refusal (Cancel), else the
                // first action.
                let entry = self.default_action.or_else(|| {
                    self.actions
                        .iter()
                        .find(|action| action.refuses)
                        .map(|action| action.id)
                });
                if let Some(at) =
                    entry.and_then(|entry| action_ids.iter().position(|(name, _)| name == entry))
                {
                    ids.rotate_left(at);
                }
            }
            crate::ui::focus::register_focus_trap(ctx, key, &ids);
        }
        if let Some(id) = selected.filter(|_| field_id.is_some()) {
            log::info!("ui: field modal {} chose {id}", self.id);
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
        // An injected key or click, or a press during the arm delay, never
        // runs a destructive action (stint 1513).
        if let Some(input) = destructive_input {
            let destructive = |id: &str| self.actions.iter().any(|a| a.id == id && a.destructive);
            if selected.is_some_and(|id| destructive(id) && !input.admits(false)) {
                log::info!(
                    "destructive_guard: modal {} ignored {:?} ({})",
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
    surface: Option<egui::Color32>,
) -> KitButton<'b> {
    let mut button = KitButton::new(action.label, colors)
        .kind(action.kind)
        .size(ButtonSize::Md);
    if let Some(surface) = surface {
        button = button.surface(surface);
    }
    if !hint.is_empty() {
        button = button.hint(hint);
    }
    button
}

/// The one `ActionModal` layout: a right-aligned row of kit buttons, each
/// carrying its shortcut inside it. `actions` are given in Tab order, and the
/// first sits rightmost: the row paints right to left, so paint order, the
/// focus trap's order and the one Tab order (stint 1514) are all the order
/// given. The layout depends only on the labels and the available width,
/// never on hover or press, so no state moves a button. A row that cannot
/// fit stacks full-width with the first action on top.
struct ButtonRow<'a, 'c> {
    modal: &'c str,
    actions: &'a [DialogAction<'a>],
    colors: &'c Colors,
    focusable: bool,
    consent: Option<crate::broker::consent::ConsentInput>,
    /// Destructive actions are drawn disabled until this arms (stint 1513).
    destructive: Option<crate::broker::consent::ConsentInput>,
    /// The fill the row sits on, when it is not the modal shell's.
    surface: Option<egui::Color32>,
}

/// What a [`ButtonRow`] drew: the clicked action and each action's report.
struct RowOutcome<'a> {
    selected: Option<&'a str>,
    actions: Vec<super::layout_report::Action>,
}

/// The `ActionModal` button row outside a modal, for a surface whose
/// actions belong to it rather than to a dialog (the changelog's update
/// status). Returns the clicked action id and each action's layout report.
/// Shortcuts are hint text only here; the caller owns its keys.
pub(crate) fn action_row<'a>(
    ui: &mut egui::Ui,
    id: &str,
    actions: &'a [DialogAction<'a>],
    colors: &Colors,
    surface: egui::Color32,
) -> (Option<&'a str>, Vec<super::layout_report::Action>) {
    let outcome = ButtonRow {
        modal: id,
        actions,
        colors,
        focusable: true,
        consent: None,
        destructive: None,
        surface: Some(surface),
    }
    .show(ui, &mut Vec::new());
    (outcome.selected, outcome.actions)
}

impl<'a> ButtonRow<'a, '_> {
    fn show(self, ui: &mut egui::Ui, action_ids: &mut Vec<(String, egui::Id)>) -> RowOutcome<'a> {
        let Self {
            modal,
            actions,
            colors,
            focusable,
            consent,
            destructive,
            surface,
        } = self;
        let hints: Vec<String> = actions
            .iter()
            .map(|action| action.shortcut.as_ref().map(hint_text).unwrap_or_default())
            .collect();
        let widths: Vec<f32> = actions
            .iter()
            .zip(&hints)
            .map(|(action, hint)| confirm_button(action, hint, colors, surface).natural_width(ui))
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
        let mut add = |ui: &mut egui::Ui, action: &'a DialogAction<'a>, hint: &str| {
            let enabled = consent.is_none_or(|input| action.refuses || input.armed)
                && (!action.destructive || destructive.is_none_or(|input| input.armed));
            let mut button = confirm_button(action, hint, colors, surface).focusable(focusable);
            if stacked {
                button = button.min_width(ui.available_width());
            }
            // Each button's id is keyed on the modal and the action, never on
            // where it sits in the ui tree, so a row-to-stack reflow (a narrow
            // window, a raised UI scale) keeps Tab focus on the same action.
            let id = egui::Id::new(("plexi_action_modal_button", modal, action.id));
            let response = ui
                .scope_builder(egui::UiBuilder::new().id(id), |ui| {
                    ui.add_enabled(enabled, button)
                })
                .inner;
            action_ids.push((action.id.to_string(), response.id));
            reports.push(super::layout_report::Action {
                id: action.id.into(),
                name: action.label.into(),
                painted_label: action.label.into(),
                hit: response.rect,
                minimum: egui::Vec2::splat(style::BUTTON_H_MD),
                inactive: !enabled,
            });
            if response.clicked() {
                selected = Some(action.id);
            }
        };
        if stacked {
            ui.spacing_mut().item_spacing.y = style::SPACE_SM;
            for (action, hint) in actions.iter().zip(&hints) {
                add(ui, action, hint);
            }
        } else {
            // One button-high row: a bare `with_layout` takes the whole
            // remaining height and centres the buttons in it, which leaves a
            // tall gap in a modal whose body has a window-sized height budget.
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), style::BUTTON_H_MD),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = style::SPACE_SM;
                    for (action, hint) in actions.iter().zip(&hints) {
                        add(ui, action, hint);
                    }
                },
            );
        }
        let rect = egui::Rect::from_min_max(start, ui.min_rect().max);
        let backdrop = surface.unwrap_or(colors.bg_sidebar);
        capture.finish(ui.painter(), rect, backdrop, reports.clone(), vec![]);
        RowOutcome {
            selected,
            actions: reports,
        }
    }
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
        // Tab order Save (the default), Cancel, then Discard (stint 1505);
        // the row reads Discard, Cancel, Save left to right.
        let actions = [
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
            DialogAction::new("cancel", "Cancel", ButtonKind::Secondary)
                .shortcut(DialogShortcut::new(&["Esc"], Modifiers::NONE, Key::Escape).exact()),
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
            .destructive_input(synthetic_frame, (&self.path, self.conflicted, self.reload))
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

    #[test]
    fn refocus_modal_held_repeat_cannot_decide() {
        use egui::{Event, Key, Modifiers, RawInput};

        let colors = Colors::from_config(&crate::ui::theme::preset_colors("plexi-dusk").unwrap());
        let actions = [
            DialogAction::new("save", "Save", ButtonKind::Primary),
            DialogAction::new("cancel", "Cancel", ButtonKind::Secondary)
                .shortcut(DialogShortcut::new(&["Esc"], Modifiers::NONE, Key::Escape).exact()),
        ];
        let mut held_decisions = Vec::new();
        for (key_code, native_repeat) in [
            (Key::Enter, false),
            (Key::Enter, true),
            (Key::Escape, false),
            (Key::Escape, true),
        ] {
            let ctx = egui::Context::default();
            crate::ui::theme::setup_fonts(&ctx);
            // Egui input time, one 60 Hz frame per draw unless a test skips ahead.
            let clock = std::cell::Cell::new(0.0_f64);
            let draw = |shown, focused, events| {
                clock.set(clock.get() + 1.0 / 60.0);
                let mut result = (None, false);
                let _ = ctx.run_ui(
                    RawInput {
                        events,
                        focused,
                        time: Some(clock.get()),
                        ..Default::default()
                    },
                    |_| {
                        if shown {
                            let response = ActionModal::new("refocus-test", "Save?", &actions)
                                .default_action("save")
                                .show(&ctx, &colors, |_| {});
                            result = (response.selected, response.dismissed);
                        }
                    },
                );
                result
            };
            let press = |repeat| Event::Key {
                key: key_code,
                physical_key: Some(key_code),
                pressed: true,
                repeat,
                modifiers: Modifiers::NONE,
            };

            draw(false, true, vec![press(false)]);
            draw(true, true, vec![]);
            assert_eq!(draw(true, true, vec![press(false)]), (None, false));
            draw(true, false, vec![Event::WindowFocused(false)]);
            draw(true, true, vec![Event::WindowFocused(true)]);
            held_decisions.push((
                key_code,
                native_repeat,
                draw(true, true, vec![press(native_repeat)]),
            ));

            // Release while another window has focus; egui sees no key-up.
            draw(true, false, vec![Event::WindowFocused(false)]);
            draw(true, true, vec![Event::WindowFocused(true)]);
            assert_eq!(
                draw(true, true, vec![press(false)]),
                (None, false),
                "a press inside the refocus guard cannot tell held from fresh"
            );
            draw(
                true,
                true,
                vec![Event::Key {
                    key: key_code,
                    physical_key: Some(key_code),
                    pressed: false,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
            );
            clock.set(clock.get() + REFOCUS_GUARD.as_secs_f64());
            assert_eq!(
                draw(true, true, vec![press(false)]),
                if key_code == Key::Enter {
                    (Some("save"), false)
                } else {
                    (Some("cancel"), false)
                },
                "fresh {key_code:?} must decide the modal"
            );
        }
        assert!(
            held_decisions.iter().all(|(_, _, decision)| *decision == (None, false)),
            "held keys decided the modal after refocus (key, raw repeat, decision): {held_decisions:?}"
        );
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
            arm_destructive_dialogs_for_test(&ctx);
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
            // Leaving the screen for the held-key frame armed it anew.
            arm_destructive_dialogs_for_test(&ctx);
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
}
