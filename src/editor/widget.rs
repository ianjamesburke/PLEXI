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
    /// Stable widget id, so callers can register it with the host focus
    /// reconciler (`crate::ui::focus`). Auto-generated when unset.
    id: Option<egui::Id>,
    /// When false, keyboard/IME/clipboard events are ignored (the caller's
    /// focus model says another surface owns input); pointer placement still
    /// works so clicking the editor can re-claim focus.
    active: bool,
    /// Monospace font size override; defaults to the style's monospace size.
    font_size: Option<f32>,
    /// When true, Tab/Shift-Tab/Enter/Backspace route through the
    /// Markdown-aware commands (list continuation, line-wise indent,
    /// marker-aware backspace).
    markdown: bool,
    /// Char ranges to paint with a background (find matches), plus the index
    /// of the "current" range and the two fill colors (normal, current).
    highlights: Vec<(usize, usize)>,
    current_highlight: Option<usize>,
    highlight_bg: egui::Color32,
    current_highlight_bg: egui::Color32,
}

impl<'a> EditorWidget<'a> {
    pub fn new(doc: &'a mut Document, view: &'a mut ViewState) -> Self {
        Self {
            doc,
            view,
            id: None,
            active: true,
            font_size: None,
            markdown: false,
            highlights: Vec::new(),
            current_highlight: None,
            highlight_bg: egui::Color32::TRANSPARENT,
            current_highlight_bg: egui::Color32::TRANSPARENT,
        }
    }

    #[must_use]
    pub fn id(mut self, id: egui::Id) -> Self {
        self.id = Some(id);
        self
    }

    #[must_use]
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    #[must_use]
    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = Some(size);
        self
    }

    #[must_use]
    pub fn markdown(mut self, markdown: bool) -> Self {
        self.markdown = markdown;
        self
    }

    #[must_use]
    pub fn highlights(
        mut self,
        ranges: Vec<(usize, usize)>,
        current: Option<usize>,
        bg: egui::Color32,
        current_bg: egui::Color32,
    ) -> Self {
        self.highlights = ranges;
        self.current_highlight = current;
        self.highlight_bg = bg;
        self.current_highlight_bg = current_bg;
        self
    }

    pub fn show(self, ui: &mut Ui) -> egui::Response {
        let font_id = match self.font_size {
            Some(size) => egui::FontId::monospace(size),
            None => egui::TextStyle::Monospace.resolve(ui.style()),
        };
        let line_height = ui.fonts_mut(|f| f.row_height(&font_id));
        let (auto_id, rect) = ui.allocate_space(ui.available_size_before_wrap());
        let id = self.id.unwrap_or(auto_id);
        let response = ui.interact(rect, id, Sense::click_and_drag());

        self.view.line_height = line_height;
        self.view.viewport_height = rect.height();
        self.view.viewport_width = rect.width();
        let page_rows = ((rect.height() / line_height).floor().max(1.0)) as usize;

        let mut commands: Vec<EditorCommand> = Vec::new();
        let mut copy_requested = false;
        let mut cut_requested = false;

        if self.active {
            // Lock focus-traversal keys (Tab, arrows) to this widget so egui
            // never re-purposes them for widget navigation while editing.
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: false,
                    },
                );
            });
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
                        if let Some(cmd) = translate_key(*key, *modifiers, page_rows, self.markdown)
                        {
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
            let scroll = ui.input(|i| i.smooth_scroll_delta);
            if scroll != egui::Vec2::ZERO {
                self.view.scroll_y -= scroll.y;
                self.view.scroll_x = (self.view.scroll_x - scroll.x).max(0.0);
            }
        }
        self.view.clamp_scroll(line_count);
        if edited {
            let caret = self.doc.cursor();
            self.view.scroll_to_line(caret.line, line_count);
            // Keep the caret horizontally visible on unwrapped long lines.
            let text = line_text(self.doc.buffer(), caret.line);
            let galley =
                ui.fonts_mut(|f| f.layout_no_wrap(text.clone(), font_id.clone(), egui::Color32::WHITE));
            let caret_x = galley
                .pos_from_cursor(CCursor::new(caret.column.min(text.chars().count())))
                .left();
            self.view.scroll_to_x(caret_x);
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
        let ccursor =
            galley.cursor_from_pos(Vec2::new(pos.x - rect.left() + self.view.scroll_x, 0.0));
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
            let origin = egui::pos2(rect.left() - self.view.scroll_x, top);

            // Find-match highlight fills under the text and selection.
            if !self.highlights.is_empty() {
                let line_start = buffer.line_to_char(line);
                let char_count = text.chars().count();
                let line_end_char = line_start + char_count;
                for (i, &(hs, he)) in self.highlights.iter().enumerate() {
                    if he <= line_start || hs >= line_end_char {
                        continue;
                    }
                    let from = hs.saturating_sub(line_start).min(char_count);
                    let to = (he - line_start).min(char_count);
                    let x0 = galley.pos_from_cursor(CCursor::new(from)).left();
                    let x1 = galley.pos_from_cursor(CCursor::new(to)).left();
                    let bg = if Some(i) == self.current_highlight {
                        self.current_highlight_bg
                    } else {
                        self.highlight_bg
                    };
                    painter.rect_filled(
                        Rect::from_min_max(
                            egui::pos2(origin.x + x0, top),
                            egui::pos2(origin.x + x1, top + self.view.line_height),
                        ),
                        0.0,
                        bg,
                    );
                }
            }

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

            if self.active && line == caret.line {
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
/// for keys the editor does not handle. `page_rows` is the viewport height in
/// lines, used by PageUp/PageDown. `markdown` routes Tab/Enter/Backspace to
/// the Markdown-aware commands.
fn translate_key(
    key: Key,
    modifiers: Modifiers,
    page_rows: usize,
    markdown: bool,
) -> Option<EditorCommand> {
    let extend = modifiers.shift;
    let word = modifiers.alt;
    let line = modifiers.command;
    let mv = |movement: Movement| Some(EditorCommand::Move { movement, extend });
    if markdown {
        match key {
            Key::Tab if modifiers.shift => return Some(EditorCommand::MarkdownOutdent),
            Key::Tab if modifiers.is_none() => return Some(EditorCommand::MarkdownIndent),
            Key::Enter if modifiers.is_none() => return Some(EditorCommand::MarkdownNewline),
            Key::Backspace if modifiers.is_none() => {
                return Some(EditorCommand::MarkdownBackspace)
            }
            _ => {}
        }
    }
    match key {
        Key::Tab if modifiers.shift => Some(EditorCommand::Outdent),
        Key::Tab if modifiers.is_none() => Some(EditorCommand::Indent),
        Key::PageUp => mv(Movement::PageUp(page_rows)),
        Key::PageDown => mv(Movement::PageDown(page_rows)),
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
    fn tab_indents_and_shift_tab_outdents() {
        let mut h = harness("line");
        h.key_press(Key::Tab);
        h.step();
        assert_eq!(semantic(&h).text, "    line");
        h.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
        h.step();
        assert_eq!(semantic(&h).text, "line");
    }

    fn inactive_harness(text: &str) -> egui_kittest::Harness<'static, TestState> {
        let state = TestState {
            doc: Document::new(text),
            view: ViewState::default(),
        };
        egui_kittest::Harness::new_ui_state(
            |ui, state: &mut TestState| {
                EditorWidget::new(&mut state.doc, &mut state.view)
                    .active(false)
                    .show(ui);
            },
            state,
        )
    }

    #[test]
    fn inactive_widget_ignores_keyboard_and_text() {
        let mut h = inactive_harness("keep");
        h.event(Event::Text("nope".into()));
        h.step();
        h.key_press(Key::Backspace);
        h.step();
        assert_eq!(semantic(&h).text, "keep");
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
