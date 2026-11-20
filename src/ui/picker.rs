//! Picker painters and key translation over `host::picker::PickerState`.
//!
//! Painters read the state and return `PickerCommand`s; they never mutate
//! it. Keys are translated in the focus owner's `handle_key` through
//! [`key_commands`], never in a draw function.

use crate::host::picker::{PickerCommand, PickerState, RowKey};
use crate::ui::{
    list::{ListRow, ListRowPips},
    style,
    text_field::TextField,
    theme::Colors,
};

/// Translate this frame's picker keys. Consumed keys never reach the text
/// field, so none of them edits the query.
pub(crate) fn key_commands(input: &mut crate::app::input_router::PlexiInput) -> Vec<PickerCommand> {
    use egui::{Key, Modifiers};
    let mut commands = Vec::new();
    if input.consume_key(Modifiers::NONE, Key::Escape) {
        commands.push(PickerCommand::Close);
        return commands;
    }
    // Shift first: a bare-Tab pattern also matches Shift+Tab logically.
    while input.consume_key(Modifiers::SHIFT, Key::Tab) {
        commands.push(PickerCommand::PrevStop);
    }
    while input.consume_key(Modifiers::NONE, Key::Tab) {
        commands.push(PickerCommand::NextStop);
    }
    while input.consume_key(Modifiers::NONE, Key::ArrowDown)
        || input.consume_key(Modifiers::COMMAND, Key::J)
    {
        commands.push(PickerCommand::Move(1));
    }
    while input.consume_key(Modifiers::NONE, Key::ArrowUp)
        || input.consume_key(Modifiers::COMMAND, Key::K)
    {
        commands.push(PickerCommand::Move(-1));
    }
    if input.consume_key_exact(Modifiers::NONE, Key::Enter) {
        commands.push(PickerCommand::Accept);
    }
    commands
}

/// The query field, the owner's default text surface. An edit comes back as
/// `QueryEdited`, the only command that changes the query. Stop chips join
/// here when a second stop exists (stint 1392).
pub(crate) fn field(
    ui: &mut egui::Ui,
    state: &PickerState,
    id: egui::Id,
    surface: crate::ui::focus::SurfaceKey,
    log_name: &str,
    colors: &Colors,
) -> Vec<PickerCommand> {
    let mut commands = Vec::new();
    let mut text = state.query().to_owned();
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
                    .log_name(log_name)
                    .show(ui, &mut text, colors)
            },
        )
        .inner;
    // The field is the picker's only focus stop: Tab and Shift+Tab are
    // picker keys and must never walk egui focus to a widget behind it.
    crate::ui::focus::register_focus_trap(ui.ctx(), surface, &[id]);
    if response.changed() && text != state.query() {
        commands.push(PickerCommand::QueryEdited(text));
    }
    commands
}

/// Result rows with group headers. Hover selects only when the pointer
/// moved; a click accepts the row exactly as Enter would. `pips` decorates
/// a row by key with the shared pane-state lane.
pub(crate) fn results(
    ui: &mut egui::Ui,
    state: &PickerState,
    max_height: f32,
    empty_text: &str,
    pips: &dyn Fn(&RowKey) -> Option<ListRowPips>,
    colors: &Colors,
) -> Vec<PickerCommand> {
    let mut commands = Vec::new();
    if state.rows().is_empty() {
        let text = if state.is_loading() {
            "Loading"
        } else {
            empty_text
        };
        ui.scope(|ui| {
            ui.set_max_width(ui.available_width());
            crate::ui::labels::description_label(ui, text, colors);
        });
        return commands;
    }
    let reveal_id = ui.id().with("picker_reveal_epoch");
    let last_reveal = ui.ctx().data(|d| d.get_temp::<u64>(reveal_id));
    let reveal = last_reveal != Some(state.reveal_epoch());
    ui.ctx()
        .data_mut(|d| d.insert_temp(reveal_id, state.reveal_epoch()));
    let pointer_moved = ui.ctx().input(|i| i.pointer.delta().length_sq() > 0.5);
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
                        crate::ui::labels::list_group_label(ui, row.group, colors);
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
                if row.unavailable.is_some() {
                    continue;
                }
                if response.row_clicked() {
                    commands.push(PickerCommand::Click(row.key.clone()));
                } else if response.row_hovered() && pointer_moved && !is_selected {
                    commands.push(PickerCommand::Hover(row.key.clone()));
                }
            }
        });
    commands
}
