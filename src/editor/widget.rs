//! Minimal egui surface for the editor core — the only egui-dependent file.
//!
//! Adapted from Ferrite <https://github.com/OlaProeis/Ferrite>
//! @ 3ba085c561670342d72c560efbf6b0b92b5c0b46 (editor.rs event translation,
//! rendering/{cursor,text}.rs), MIT. Diverges from upstream: translates egui
//! input into [`EditorCommand`]s and paints from [`ViewState`] layout; owns no
//! editing logic. Focus arbitration is the caller's job (`src/ui/AGENTS.md`
//! forbids direct `request_focus`).

use egui::text::CCursor;
use egui::{Event, ImeEvent, Key, Modifiers, Rect, Sense, Ui, Vec2};

use super::commands::{Document, EditorCommand, Movement};
use super::cursor::Cursor;
use super::movement::line_text;
use super::view::ViewState;

/// Renders a [`Document`] and translates egui input into [`EditorCommand`]s.
///
/// The widget processes keyboard/IME input every frame it is shown; callers
/// decide whether to show it (host focus model, stint 0474).
pub struct EditorWidget<'a> {
    doc: &'a mut Document,
    view: &'a mut ViewState,
}

impl<'a> EditorWidget<'a> {
    pub fn new(doc: &'a mut Document, view: &'a mut ViewState) -> Self {
        Self { doc, view }
    }

    pub fn show(self, ui: &mut Ui) -> egui::Response {
        let font_id = egui::TextStyle::Monospace.resolve(ui.style());
        let line_height = ui.fonts_mut(|f| f.row_height(&font_id));
        let (rect, response) =
            ui.allocate_exact_size(ui.available_size_before_wrap(), Sense::click_and_drag());

        self.view.line_height = line_height;
        self.view.viewport_height = rect.height();

        let mut commands: Vec<EditorCommand> = Vec::new();
        let mut copy_requested = false;
        let mut cut_requested = false;

        let events = ui.input(|i| i.events.clone());
        for event in &events {
            match event {
                Event::Text(text) => commands.push(EditorCommand::InsertText(text.clone())),
                Event::Paste(text) => commands.push(EditorCommand::InsertText(text.clone())),
                Event::Copy => copy_requested = true,
                Event::Cut => cut_requested = true,
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if let Some(cmd) = translate_key(*key, *modifiers) {
                        commands.push(cmd);
                    }
                }
                Event::Ime(ime) => match ime {
                    ImeEvent::Preedit(text) => {
                        commands.push(EditorCommand::ImePreedit(text.clone()));
                    }
                    ImeEvent::Commit(text) => {
                        commands.push(EditorCommand::ImeCommit(text.clone()));
                    }
                    ImeEvent::Disabled => commands.push(EditorCommand::ImeCancel),
                    ImeEvent::Enabled => {}
                },
                _ => {}
            }
        }

        if copy_requested || cut_requested {
            let selected = self.doc.selected_text();
            if !selected.is_empty() {
                ui.ctx().copy_text(selected);
                if cut_requested {
                    commands.push(EditorCommand::Backspace);
                }
            }
        }

        // Pointer: click to place, shift-click / drag to extend, double-click
        // word, triple-click line.
        let hit = |pos: egui::Pos2| self.hit_test(ui, &font_id, rect, pos);
        if let Some(pos) = response.interact_pointer_pos() {
            let cursor = hit(pos);
            if response.triple_clicked() {
                commands.push(EditorCommand::SelectLineAt(cursor.line));
            } else if response.double_clicked() {
                commands.push(EditorCommand::SelectWordAt(cursor));
            } else if response.drag_started() || response.clicked() {
                if ui.input(|i| i.modifiers.shift) {
                    commands.push(EditorCommand::ExtendTo(cursor));
                } else {
                    commands.push(EditorCommand::SetCursor(cursor));
                }
            } else if response.dragged() {
                commands.push(EditorCommand::ExtendTo(cursor));
            }
        }

        let edited = !commands.is_empty();
        for command in commands {
            self.doc.apply(command);
        }

        // Scrolling: wheel when hovered, then keep the caret visible after
        // any command.
        let line_count = self.doc.buffer().line_count();
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.view.scroll_y -= scroll;
            }
        }
        self.view.clamp_scroll(line_count);
        if edited {
            self.view.scroll_to_line(self.doc.cursor().line, line_count);
        }

        self.paint(ui, &font_id, rect);
        response
    }

    /// Maps a pointer position to a document cursor via per-line galley
    /// hit-testing.
    fn hit_test(&self, ui: &Ui, font_id: &egui::FontId, rect: Rect, pos: egui::Pos2) -> Cursor {
        let line_count = self.doc.buffer().line_count();
        let y = pos.y - rect.top() + self.view.scroll_y;
        let line = ((y / self.view.line_height).floor().max(0.0) as usize)
            .min(line_count.saturating_sub(1));
        let text = line_text(self.doc.buffer(), line);
        let galley = ui.fonts_mut(|f| f.layout_no_wrap(text, font_id.clone(), egui::Color32::WHITE));
        let ccursor = galley.cursor_from_pos(Vec2::new(pos.x - rect.left(), 0.0));
        Cursor::new(line, ccursor.index)
    }

    fn paint(&self, ui: &Ui, font_id: &egui::FontId, rect: Rect) {
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals();
        let text_color = visuals.text_color();
        let selection_color = visuals.selection.bg_fill.linear_multiply(0.5);
        let caret_color = visuals.selection.stroke.color;
        let buffer = self.doc.buffer();
        let line_count = buffer.line_count();
        let selection = self.doc.selection();
        let (sel_start, sel_end) = selection.ordered();
        let caret = self.doc.cursor();
        let mut caret_rect: Option<Rect> = None;

        for line in self.view.visible_lines(line_count) {
            let top = rect.top() + self.view.line_top(line) - self.view.scroll_y;
            let text = line_text(buffer, line);
            let galley =
                ui.fonts_mut(|f| f.layout_no_wrap(text.clone(), font_id.clone(), text_color));
            let origin = egui::pos2(rect.left(), top);

            if selection.is_range() && selection.touches_line(line) {
                let char_count = text.chars().count();
                let from = if line == sel_start.line {
                    sel_start.column.min(char_count)
                } else {
                    0
                };
                // Interior lines extend one column past the end to show the
                // selected newline.
                let (to, extend_newline) = if line == sel_end.line {
                    (sel_end.column.min(char_count), false)
                } else {
                    (char_count, true)
                };
                let x0 = galley.pos_from_cursor(CCursor::new(from)).left();
                let mut x1 = galley.pos_from_cursor(CCursor::new(to)).left();
                if extend_newline {
                    x1 += self.view.line_height * 0.5;
                }
                painter.rect_filled(
                    Rect::from_min_max(
                        egui::pos2(origin.x + x0, top),
                        egui::pos2(origin.x + x1, top + self.view.line_height),
                    ),
                    0.0,
                    selection_color,
                );
            }

            painter.galley(origin, galley.clone(), text_color);

            if line == caret.line {
                let x = galley
                    .pos_from_cursor(CCursor::new(caret.column.min(text.chars().count())))
                    .left();
                let caret_x = origin.x + x;

                // IME preedit paints as underlined overlay text at the caret.
                if let Some(preedit) = self.doc.ime().preedit().filter(|p| !p.is_empty()) {
                    let preedit_galley = ui.fonts_mut(|f| {
                        f.layout_no_wrap(preedit.to_string(), font_id.clone(), text_color)
                    });
                    let width = preedit_galley.size().x;
                    painter.galley(egui::pos2(caret_x, top), preedit_galley, text_color);
                    painter.line_segment(
                        [
                            egui::pos2(caret_x, top + self.view.line_height - 1.0),
                            egui::pos2(caret_x + width, top + self.view.line_height - 1.0),
                        ],
                        egui::Stroke::new(1.0_f32, text_color),
                    );
                    caret_rect = Some(Rect::from_min_size(
                        egui::pos2(caret_x + width, top),
                        Vec2::new(1.0, self.view.line_height),
                    ));
                } else {
                    caret_rect = Some(Rect::from_min_size(
                        egui::pos2(caret_x, top),
                        Vec2::new(1.0, self.view.line_height),
                    ));
                }
            }
        }

        if let Some(caret_rect) = caret_rect {
            painter.rect_filled(caret_rect, 0.0, caret_color);
            // Position the OS IME candidate window at the caret.
            ui.ctx().output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    rect,
                    cursor_rect: caret_rect,
                });
            });
        }
    }
}

/// Maps a key press (with modifiers) to an editor command. Returns `None`
/// for keys the editor does not handle.
fn translate_key(key: Key, modifiers: Modifiers) -> Option<EditorCommand> {
    let extend = modifiers.shift;
    let word = modifiers.alt;
    let line = modifiers.command;
    let mv = |movement: Movement| Some(EditorCommand::Move { movement, extend });
    match key {
        Key::ArrowLeft if line => mv(Movement::LineStart),
        Key::ArrowLeft if word => mv(Movement::WordLeft),
        Key::ArrowLeft => mv(Movement::Left),
        Key::ArrowRight if line => mv(Movement::LineEnd),
        Key::ArrowRight if word => mv(Movement::WordRight),
        Key::ArrowRight => mv(Movement::Right),
        Key::ArrowUp if line => mv(Movement::DocStart),
        Key::ArrowUp => mv(Movement::Up),
        Key::ArrowDown if line => mv(Movement::DocEnd),
        Key::ArrowDown => mv(Movement::Down),
        Key::Home => mv(Movement::LineStart),
        Key::End => mv(Movement::LineEnd),
        Key::Backspace => Some(EditorCommand::Backspace),
        Key::Delete => Some(EditorCommand::DeleteForward),
        Key::Enter => Some(EditorCommand::InsertNewline),
        Key::A if modifiers.command => Some(EditorCommand::SelectAll),
        Key::Z if modifiers.command && modifiers.shift => Some(EditorCommand::Redo),
        Key::Z if modifiers.command => Some(EditorCommand::Undo),
        Key::Y if modifiers.command => Some(EditorCommand::Redo),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::EditorSemanticState;

    struct TestState {
        doc: Document,
        view: ViewState,
    }

    fn harness(text: &str) -> egui_kittest::Harness<'static, TestState> {
        let state = TestState {
            doc: Document::new(text),
            view: ViewState::default(),
        };
        egui_kittest::Harness::new_ui_state(
            |ui, state: &mut TestState| {
                EditorWidget::new(&mut state.doc, &mut state.view).show(ui);
            },
            state,
        )
    }

    fn semantic(h: &egui_kittest::Harness<'static, TestState>) -> EditorSemanticState {
        h.state().doc.semantic_state(h.state().view.scroll_y)
    }

    #[test]
    fn typing_keys_and_text_events_edit_document() {
        let mut h = harness("");
        h.event(Event::Text("hello".into()));
        h.step();
        h.key_press(Key::Enter);
        h.step();
        h.event(Event::Text("world".into()));
        h.step();

        let state = semantic(&h);
        assert_eq!(state.text, "hello\nworld");
        assert_eq!(state.cursor, Cursor::new(1, 5));

        // Backspace deletes; cmd+Z undoes the whole coalesced group.
        h.key_press(Key::Backspace);
        h.step();
        assert_eq!(semantic(&h).text, "hello\nworl");
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.step();
        assert_eq!(semantic(&h).text, "hello\nworld");
        assert!(semantic(&h).can_redo);
    }

    #[test]
    fn shift_arrows_select_and_semantic_state_agrees() {
        let mut h = harness("abc def");
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.step();
        let state = semantic(&h);
        assert_eq!(state.selection.ordered().1, Cursor::new(0, 7));

        h.key_press(Key::ArrowRight); // collapse to end
        h.step();
        h.key_press_modifiers(Modifiers::SHIFT | Modifiers::ALT, Key::ArrowLeft);
        h.step();
        let state = semantic(&h);
        assert_eq!(state.cursor, Cursor::new(0, 4));
        assert!(state.selection.is_range());
    }

    #[test]
    fn ime_events_compose_and_commit() {
        let mut h = harness("");
        h.event(Event::Ime(ImeEvent::Enabled));
        h.event(Event::Ime(ImeEvent::Preedit("かん".into())));
        h.step();
        let state = semantic(&h);
        assert_eq!(state.ime_composition, Some("かん".to_string()));
        assert_eq!(state.text, "");

        h.event(Event::Ime(ImeEvent::Commit("漢字".into())));
        h.event(Event::Ime(ImeEvent::Disabled));
        h.step();
        let state = semantic(&h);
        assert_eq!(state.text, "漢字");
        assert_eq!(state.ime_composition, None);
        assert_eq!(state.cursor, Cursor::new(0, 2));
    }

    #[test]
    fn paste_event_inserts_text() {
        let mut h = harness("ab");
        h.key_press(Key::ArrowRight);
        h.step();
        h.event(Event::Paste("XY".into()));
        h.step();
        assert_eq!(semantic(&h).text, "aXYb");
    }
}
