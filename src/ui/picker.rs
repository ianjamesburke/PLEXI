//! Picker painters and the ordered input adapter over
//! `host::picker::PickerState`.
//!
//! Painters read the state and publish what they presented; they never
//! mutate it or emit commands. Input reaches the picker through
//! [`PickerInput::route`], one event at a time in arrival order: the field
//! edits at the event's position, keys bind where they sit, and a pointer
//! event is hit tested against the layout the person last saw (stint 1391).

use crate::host::picker::{Hit, PickerCommand, PickerState, RowKey, Stamp, Step};
use crate::ui::{
    list::{ListRow, ListRowPips},
    style,
    text_field::{FieldEdit, FieldGeometry, OrderedLineField},
    theme::Colors,
};

/// A press that moves further than this before release is a drag, not a
/// click (egui's own click threshold).
const CLICK_SLOP: f32 = 6.0;

/// The layout of the last presented (not discarded) pass.
#[derive(Debug, Clone)]
pub(crate) struct Presented {
    /// Hit geometry is valid only while the rows keep this stamp.
    pub stamp: Stamp,
    /// Clipped rects of the actionable rows, with their keys.
    pub rows: Vec<(egui::Rect, RowKey)>,
    /// The modal's frame; a press and release outside it is a scrim click.
    pub modal: egui::Rect,
    pub field: FieldGeometry,
}

#[derive(Debug, Clone)]
enum PressOn {
    Row(Hit),
    Field,
    Modal,
    Scrim,
}

#[derive(Debug, Clone)]
struct Press {
    origin: egui::Pos2,
    on: PressOn,
}

/// What [`PickerInput::route`] made of one event.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Routed {
    Command(PickerCommand),
    /// The picker owned the event and nothing changed.
    Claimed,
    /// Not the picker's: the host's shortcut policy may still want it.
    Unclaimed,
}

/// The palette's input owner state: the field and the last presentation.
#[derive(Debug)]
pub(crate) struct PickerInput {
    pub(crate) field: OrderedLineField,
    presented: Option<Presented>,
    pointer: Option<egui::Pos2>,
    press: Option<Press>,
    next_ordinal: u64,
    /// The pass whose input the open session claimed in `App::logic`.
    pub(crate) owned_pass: Option<u64>,
    /// Events that followed the Cmd+P which opened the session, in order.
    pub(crate) carry: Vec<egui::Event>,
}

impl Default for PickerInput {
    fn default() -> Self {
        Self {
            field: OrderedLineField::new(""),
            presented: None,
            pointer: None,
            press: None,
            next_ordinal: 0,
            owned_pass: None,
            carry: Vec::new(),
        }
    }
}

impl PickerInput {
    /// A new session over `query`: earlier geometry and presses are void.
    pub(crate) fn reset(&mut self, query: &str) {
        self.field = OrderedLineField::new(query);
        self.presented = None;
        self.press = None;
    }

    /// Record what the painter showed in a pass that will be presented.
    pub(crate) fn present(&mut self, presented: Presented) {
        self.presented = Some(presented);
    }

    /// Number one delivery's events in arrival order.
    pub(crate) fn ordinals(&mut self, events: Vec<egui::Event>) -> Vec<(u64, egui::Event)> {
        events
            .into_iter()
            .map(|event| {
                self.next_ordinal += 1;
                (self.next_ordinal, event)
            })
            .collect()
    }

    fn hit_row(&self, pos: egui::Pos2) -> Option<Hit> {
        let presented = self.presented.as_ref()?;
        presented
            .rows
            .iter()
            .find(|(rect, _)| rect.contains(pos))
            .map(|(_, key)| Hit {
                stamp: presented.stamp,
                key: key.clone(),
            })
    }

    fn press_on(&self, pos: egui::Pos2) -> PressOn {
        let Some(presented) = self.presented.as_ref() else {
            return PressOn::Modal;
        };
        if presented.field.rect.contains(pos) {
            PressOn::Field
        } else if let Some(hit) = self.hit_row(pos) {
            PressOn::Row(hit)
        } else if presented.modal.contains(pos) {
            PressOn::Modal
        } else {
            PressOn::Scrim
        }
    }

    /// Translate one event at its position in the stream. Host shortcut
    /// policy (the toggle chord, close chord, Quick Note) runs before this.
    pub(crate) fn route(&mut self, event: &egui::Event, copy: &mut dyn FnMut(String)) -> Routed {
        use egui::{Event, Key, Modifiers};
        if let Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        {
            let plain = modifiers.is_none();
            let command = match key {
                Key::Escape if plain => Some(PickerCommand::Close),
                Key::Tab if *modifiers == Modifiers::SHIFT => Some(PickerCommand::PrevStop),
                Key::Tab if plain => Some(PickerCommand::NextStop),
                Key::ArrowDown if plain => Some(PickerCommand::Move(Step::Down)),
                Key::ArrowUp if plain => Some(PickerCommand::Move(Step::Up)),
                Key::J if *modifiers == Modifiers::COMMAND => Some(PickerCommand::Move(Step::Down)),
                Key::K if *modifiers == Modifiers::COMMAND => Some(PickerCommand::Move(Step::Up)),
                // Enter during composition confirms it and belongs to the IME.
                Key::Enter if plain && self.field.composing() => return Routed::Claimed,
                Key::Enter if plain => Some(PickerCommand::Accept),
                _ => None,
            };
            if let Some(command) = command {
                return Routed::Command(command);
            }
        }
        match event {
            Event::PointerMoved(pos) => self.pointer_moved(*pos),
            Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                ..
            } => self.pointer_button(*pos, *pressed),
            Event::PointerGone => {
                self.pointer = None;
                self.press = None;
                Routed::Unclaimed
            }
            // Scrolling moves the rows under the pointer: the presented
            // geometry no longer says what a click would hit.
            Event::MouseWheel { .. } => {
                if let Some(presented) = self.presented.as_mut() {
                    presented.rows.clear();
                }
                self.press = None;
                Routed::Unclaimed
            }
            _ => match self.field.apply(event, copy) {
                FieldEdit::Unclaimed => Routed::Unclaimed,
                FieldEdit::Claimed => Routed::Claimed,
                FieldEdit::Committed(text) => Routed::Command(PickerCommand::QueryEdited(text)),
            },
        }
    }

    fn pointer_moved(&mut self, pos: egui::Pos2) -> Routed {
        let moved = self.pointer != Some(pos);
        self.pointer = Some(pos);
        if !moved {
            return Routed::Unclaimed;
        }
        if let Some(Press {
            on: PressOn::Field, ..
        }) = &self.press
        {
            if let Some(presented) = &self.presented {
                let geometry = presented.field.clone();
                self.field.place_caret(&geometry, pos, true);
            }
            return Routed::Claimed;
        }
        match self.hit_row(pos) {
            Some(hit) => Routed::Command(PickerCommand::Hover(hit)),
            None => Routed::Unclaimed,
        }
    }

    fn pointer_button(&mut self, pos: egui::Pos2, pressed: bool) -> Routed {
        self.pointer = Some(pos);
        if pressed {
            let on = self.press_on(pos);
            if matches!(on, PressOn::Field) {
                if let Some(presented) = &self.presented {
                    let geometry = presented.field.clone();
                    self.field.place_caret(&geometry, pos, false);
                }
            }
            self.press = Some(Press { origin: pos, on });
            return Routed::Claimed;
        }
        let Some(press) = self.press.take() else {
            return Routed::Unclaimed;
        };
        if press.origin.distance(pos) > CLICK_SLOP {
            return Routed::Claimed;
        }
        match (press.on, self.press_on(pos)) {
            (PressOn::Row(down), PressOn::Row(up)) if down == up => {
                Routed::Command(PickerCommand::Click(up))
            }
            (PressOn::Scrim, PressOn::Scrim) => Routed::Command(PickerCommand::Close),
            _ => Routed::Claimed,
        }
    }
}

/// The query field, the owner's default text surface. Stop chips join here
/// when a second stop exists (stint 1392).
pub(crate) fn field(
    ui: &mut egui::Ui,
    state: &PickerState,
    field: &OrderedLineField,
    id: egui::Id,
    surface: crate::ui::focus::SurfaceKey,
    colors: &Colors,
) -> FieldGeometry {
    // The field paints above the scroll body so its caret is never covered.
    let geometry = ui
        .scope_builder(
            egui::UiBuilder::new().layer_id(egui::LayerId::new(
                egui::Order::Tooltip,
                id.with("paint_layer"),
            )),
            |ui| field.show(ui, id, state.active_stop().placeholder, surface, colors),
        )
        .inner;
    // The field is the picker's only focus stop: Tab and Shift+Tab are
    // picker keys and must never walk egui focus to a widget behind it.
    crate::ui::focus::register_focus_trap(ui.ctx(), surface, &[id]);
    geometry
}

/// Result rows with group headers. Returns the clipped rect of every
/// actionable row as painted, for the next delivery's hit tests. `pips`
/// decorates a row by key with the shared pane-state lane.
pub(crate) fn results(
    ui: &mut egui::Ui,
    state: &PickerState,
    max_height: f32,
    empty_text: &str,
    pips: &dyn Fn(&RowKey) -> Option<ListRowPips>,
    colors: &Colors,
) -> Vec<(egui::Rect, RowKey)> {
    let mut hits = Vec::new();
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
        return hits;
    }
    let reveal_id = ui.id().with("picker_reveal_epoch");
    let last_reveal = ui.ctx().data(|d| d.get_temp::<u64>(reveal_id));
    let reveal = last_reveal != Some(state.reveal_epoch());
    ui.ctx()
        .data_mut(|d| d.insert_temp(reveal_id, state.reveal_epoch()));
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
                let response = list_row.show(ui, colors);
                if is_selected {
                    response.scroll_into_view(ui, reveal);
                }
                if row.unavailable.is_none() {
                    let clipped = response.rect().intersect(ui.clip_rect());
                    if clipped.is_positive() {
                        hits.push((clipped, row.key.clone()));
                    }
                }
            }
        });
    hits
}

#[cfg(test)]
#[path = "picker_tests.rs"]
mod tests;
