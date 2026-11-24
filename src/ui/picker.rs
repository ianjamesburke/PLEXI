//! Picker painters and the input state of the palette's session over
//! `host::picker::PickerState`.
//!
//! The field is egui's own text edit and the rows are egui's own buttons, so
//! editing, selection, clicks and accessibility behave as everywhere else.
//! Order is kept by the host: `PlexiApp::route_palette_input` lets the field
//! see only the edits that precede the delivery's first picker command, and
//! applies that command after the field has drawn (stint 1391).

use crate::host::picker::{Hit, PickerCommand, PickerState, RowKey, Step};
use crate::ui::{
    list::{ListRow, ListRowPips},
    style,
    text_field::TextField,
    theme::Colors,
};

/// The palette's input state across passes: the field's draft, keyboard
/// input waiting for the field, and the pointer gestures its sessions own.
#[derive(Debug, Default)]
pub(crate) struct PickerInput {
    /// The native field's buffer. During IME composition it holds preedit,
    /// so it is published as the query only while nothing is composing.
    pub(crate) draft: String,
    /// An IME composition is open in the field.
    pub(crate) composing: bool,
    /// This delivery's keyboard input that follows a field edit or a pointer
    /// activation, with its position: applied after the field and rows drew.
    pub(crate) after_draw: Vec<(usize, egui::Event)>,
    /// Where this delivery released a button or asked for an accessibility
    /// action while the palette owned input: a row or scrim activation, if
    /// the draw finds one, acts there.
    pub(crate) activation_at: Option<usize>,
    /// This delivery's edits left for the field, removed once it has drawn.
    pub(crate) fed: Vec<egui::Event>,
    /// Where this delivery last moved the pointer while the palette owned it.
    pub(crate) hover_at: Option<usize>,
    /// Keyboard input whose edit needs another field pass, in order. It
    /// precedes the next delivery and expires with the session.
    pub(crate) waiting: Vec<egui::Event>,
    /// The focus-stack opening this session's palette stands for.
    pub(crate) opening: Option<crate::host::focus_stack::OpeningId>,
    /// The pass whose input the open session owned when `App::logic` began.
    pub(crate) owned_pass: Option<u64>,
    /// A Cmd+P found in this delivery: the host actions it resolved to, run
    /// once the old owner has finished the input before it.
    pub(crate) opener: Option<Vec<crate::host::keys::Action>>,
    /// The keyboard input after that Cmd+P, for the session it opens.
    pub(crate) opening_tail: Vec<egui::Event>,
    /// Buttons a session pressed and has not released: that gesture's moves
    /// and release stay the session's after it closes.
    pub(crate) held: Vec<egui::PointerButton>,
    /// Buttons pressed while the palette did not own input, not yet
    /// released: the press's owner keeps its moves and release.
    pub(crate) pane_held: Vec<egui::PointerButton>,
    /// A session began a phased scroll that has not ended.
    pub(crate) scrolling: bool,
    /// The rows as drawn and the modal frame, for tests that point at what
    /// the person saw.
    #[cfg(test)]
    pub(crate) drawn: Option<(Vec<(egui::Rect, RowKey)>, egui::Rect)>,
}

impl PickerInput {
    /// A new session over `query`.
    pub(crate) fn reset(&mut self, query: &str) {
        self.draft = query.to_owned();
        self.opening = None;
        self.composing = false;
        self.after_draw.clear();
        self.fed.clear();
        self.activation_at = None;
        self.hover_at = None;
        self.waiting.clear();
        #[cfg(test)]
        {
            self.drawn = None;
        }
    }

    /// The drawn rows' centers and the modal frame.
    #[cfg(test)]
    pub(crate) fn presented_layout(&self) -> Option<(Vec<egui::Pos2>, egui::Rect)> {
        self.drawn
            .as_ref()
            .map(|(rows, modal)| (rows.iter().map(|(rect, _)| rect.center()).collect(), *modal))
    }
}

/// The picker command a key press means, or `None` for a key the field
/// edits with (or nobody wants). Host chords are resolved before this.
pub(crate) fn picker_key(event: &egui::Event, composing: bool) -> Option<PickerCommand> {
    use egui::{Event, Key, Modifiers};
    let Event::Key {
        key,
        pressed: true,
        modifiers,
        ..
    } = event
    else {
        return None;
    };
    // The host's platform-aware exact matcher: a native Cmd chord also sets
    // `mac_cmd`, and Shift+Tab must not read as Tab.
    let is = |pattern: Modifiers| {
        crate::host::keys::modifiers_match_exact_for_os(
            modifiers,
            &pattern,
            cfg!(target_os = "macos"),
        )
    };
    match key {
        Key::Escape if is(Modifiers::NONE) => Some(PickerCommand::Close),
        Key::Tab if is(Modifiers::SHIFT) => Some(PickerCommand::PrevStop),
        Key::Tab if is(Modifiers::NONE) => Some(PickerCommand::NextStop),
        Key::ArrowDown if is(Modifiers::NONE) => Some(PickerCommand::Move(Step::Down)),
        Key::ArrowUp if is(Modifiers::NONE) => Some(PickerCommand::Move(Step::Up)),
        Key::J if is(Modifiers::COMMAND) => Some(PickerCommand::Move(Step::Down)),
        Key::K if is(Modifiers::COMMAND) => Some(PickerCommand::Move(Step::Up)),
        // Enter during composition confirms it and belongs to the IME.
        Key::Enter if is(Modifiers::NONE) && !composing => Some(PickerCommand::Accept),
        _ => None,
    }
}

/// Input the field edits with: text, clipboard, IME and key presses that
/// are no picker command.
pub(crate) fn is_field_input(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Text(_)
            | egui::Event::Paste(_)
            | egui::Event::Copy
            | egui::Event::Cut
            | egui::Event::Ime(_)
            | egui::Event::Key { pressed: true, .. }
    )
}

/// The query field, the owner's default text surface: egui's own single-line
/// text edit over `draft`. Stop chips join here when a second stop exists
/// (stint 1392).
pub(crate) fn field(
    ui: &mut egui::Ui,
    state: &PickerState,
    draft: &mut String,
    id: egui::Id,
    surface: crate::ui::focus::SurfaceKey,
    colors: &Colors,
) -> egui::Response {
    // The field paints above the scroll body so its caret is never covered.
    let response = ui
        .scope_builder(
            egui::UiBuilder::new().layer_id(egui::LayerId::new(
                egui::Order::Tooltip,
                id.with("paint_layer"),
            )),
            |ui| {
                TextField::singleline(id, state.active_stop().placeholder)
                    .surface(surface)
                    .log_name("command_palette")
                    .show(ui, draft, colors)
            },
        )
        .inner;
    // The field is the picker's only focus stop: Tab and Shift+Tab are
    // picker keys and must never walk egui focus to a widget behind it.
    crate::ui::focus::register_focus_trap(ui.ctx(), surface, &[id]);
    response
}

/// What the person did to the drawn rows this pass.
#[derive(Debug, Default)]
pub(crate) struct RowsResponse {
    /// A row egui reports clicked (pointer or accessibility), with the
    /// stamp of the answer that drew it.
    pub clicked: Option<Hit>,
    /// The row under the pointer; it selects only if the pointer moved in
    /// this delivery.
    pub hovered: Option<Hit>,
    /// Every actionable row's clipped rect, for tests.
    pub rects: Vec<(egui::Rect, RowKey)>,
}

/// Result rows with group headers, each egui's own clickable row. A row's
/// widget id is its key plus the stamp of the answer that drew it, so a
/// press or accessibility target from an earlier answer can never click the
/// row that now holds its place. `pips` decorates a row by key with the
/// shared pane-state lane.
pub(crate) fn results(
    ui: &mut egui::Ui,
    state: &PickerState,
    max_height: f32,
    empty_text: &str,
    pips: &dyn Fn(&RowKey) -> Option<ListRowPips>,
    colors: &Colors,
) -> RowsResponse {
    let mut out = RowsResponse::default();
    if state.rows().is_empty() {
        if state.is_loading() {
            // The first answer arrives on the next logic pass: hold the
            // list's height so the modal does not jump when it lands.
            ui.add_space(max_height);
        } else {
            ui.scope(|ui| {
                ui.set_max_width(ui.available_width());
                crate::ui::labels::description_label(ui, empty_text, colors);
            });
        }
        return out;
    }
    let reveal_id = ui.id().with("picker_reveal_epoch");
    let last_reveal = ui.ctx().data(|d| d.get_temp::<u64>(reveal_id));
    // Spent only when the selected row receives it: while a new query
    // loads, the old rows have no selection and the reveal waits for one.
    let reveal = last_reveal != Some(state.reveal_epoch());
    let stamp = state.rows_stamp();
    egui::ScrollArea::vertical()
        // animated(false): required by scroll_row_into_view — see src/ui/list.rs.
        .animated(false)
        .id_salt("picker_results")
        .max_height(max_height)
        .min_scrolled_height(max_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // available_width: the scroll area reserves a scrollbar gutter.
            ui.set_width(ui.available_width());
            let mut group: Option<&str> = None;
            for row in state.rows() {
                if group != Some(row.group) {
                    group = Some(row.group);
                    if !row.group.is_empty() {
                        ui.add_space(style::SPACE_XS);
                        crate::ui::typography::caption(ui, row.group, colors);
                        ui.add_space(style::SPACE_XS);
                    }
                }
                let is_selected = state.selected() == Some(&row.key);
                let secondary = row.unavailable.unwrap_or(row.secondary.as_str());
                let mut list_row = ListRow::new(&row.title)
                    .metadata_chips(&row.chips)
                    .secondary(secondary)
                    .selected(is_selected);
                if let Some(row_pips) = pips(&row.key) {
                    list_row = list_row.pane_pips(row_pips);
                }
                let response = ui
                    .push_id((&row.key, stamp.generation, stamp.revision), |ui| {
                        list_row.show(ui, colors)
                    })
                    .inner;
                if is_selected {
                    response.scroll_into_view(ui, reveal);
                    if reveal && !ui.ctx().will_discard() {
                        ui.ctx()
                            .data_mut(|d| d.insert_temp(reveal_id, state.reveal_epoch()));
                    }
                }
                if row.unavailable.is_some() {
                    continue;
                }
                let hit = || Hit {
                    stamp,
                    key: row.key.clone(),
                };
                if response.row_clicked() {
                    out.clicked = Some(hit());
                }
                if response.row_hovered() {
                    out.hovered = Some(hit());
                }
                let clipped = response.rect().intersect(ui.clip_rect());
                if clipped.is_positive() {
                    out.rects.push((clipped, row.key.clone()));
                }
            }
        });
    out
}

#[cfg(test)]
#[path = "picker_tests.rs"]
mod tests;
