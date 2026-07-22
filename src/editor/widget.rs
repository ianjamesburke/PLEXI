//! Minimal egui surface for the editor core — the only egui-dependent file.
//!
//! Adapted from Ferrite <https://github.com/OlaProeis/Ferrite>
//! @ 3ba085c561670342d72c560efbf6b0b92b5c0b46 (editor.rs event translation,
//! rendering/{cursor,text}.rs), MIT. Diverges from upstream: translates egui
//! input into [`EditorCommand`]s and paints from [`ViewState`] layout; owns no
//! editing logic. Focus arbitration is the caller's job (`src/ui/AGENTS.md`
//! forbids direct `request_focus`).

use egui::text::{CCursor, LayoutJob, TextFormat};
use egui::{Color32, Event, ImeEvent, Key, Modifiers, Rect, Sense, Ui, Vec2};

use super::commands::{Document, EditorCommand, Movement};
use super::cursor::Cursor;
use super::highlight::{SpanProvider, TokenKind};
use super::mode::EditorMode;
use super::movement::line_text;
use super::view::ViewState;

/// Horizontal padding on each side of the gutter's line numbers.
const GUTTER_PAD: f32 = 6.0;

/// Theme colors for code-mode chrome and syntax token kinds. Callers build
/// this from host design tokens (`src/ui/theme.rs`); the editor core never
/// picks concrete colors itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CodeTheme {
    pub gutter_text: Color32,
    pub current_line_bg: Color32,
    pub keyword: Color32,
    pub string: Color32,
    pub comment: Color32,
    pub number: Color32,
    pub ty: Color32,
    pub function: Color32,
    pub punctuation: Color32,
}

impl CodeTheme {
    /// Fallback derived from egui visuals, for callers without host tokens
    /// (tests): plain chrome, no token coloring beyond text/weak.
    fn from_visuals(visuals: &egui::Visuals) -> Self {
        let text = visuals.text_color();
        let weak = visuals.weak_text_color();
        Self {
            gutter_text: weak,
            current_line_bg: visuals.faint_bg_color,
            keyword: text,
            string: text,
            comment: weak,
            number: text,
            ty: text,
            function: text,
            punctuation: weak,
        }
    }

    fn color_for(&self, kind: TokenKind, plain: Color32) -> Color32 {
        match kind {
            TokenKind::Plain => plain,
            TokenKind::Keyword => self.keyword,
            TokenKind::String => self.string,
            TokenKind::Comment => self.comment,
            TokenKind::Number => self.number,
            TokenKind::Type => self.ty,
            TokenKind::Function => self.function,
            TokenKind::Punctuation => self.punctuation,
        }
    }
}

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
    /// Presentation/input mode: plain, Markdown structure commands, or code
    /// chrome (gutter, current-line highlight, syntax spans).
    mode: EditorMode,
    /// Syntax-highlight span source for code mode. `None` paints plain text
    /// (the fallback for unknown languages).
    span_provider: Option<&'a mut dyn SpanProvider>,
    /// Code-mode chrome/token colors; derived from egui visuals when unset.
    code_theme: Option<CodeTheme>,
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
            mode: EditorMode::PlainText,
            span_provider: None,
            code_theme: None,
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
    pub fn mode(mut self, mode: EditorMode) -> Self {
        self.mode = mode;
        self
    }

    /// Syntax-highlight span source for code mode. Ignored outside
    /// [`EditorMode::Code`].
    #[must_use]
    pub fn span_provider(mut self, provider: &'a mut dyn SpanProvider) -> Self {
        self.span_provider = Some(provider);
        self
    }

    /// Code-mode chrome/token colors (host theme tokens).
    #[must_use]
    pub fn code_theme(mut self, theme: CodeTheme) -> Self {
        self.code_theme = Some(theme);
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

    pub fn show(mut self, ui: &mut Ui) -> egui::Response {
        let font_id = match self.font_size {
            Some(size) => egui::FontId::monospace(size),
            None => egui::TextStyle::Monospace.resolve(ui.style()),
        };
        let line_height = ui.fonts_mut(|f| f.row_height(&font_id));
        let (auto_id, rect) = ui.allocate_space(ui.available_size_before_wrap());
        let id = self.id.unwrap_or(auto_id);
        let response = ui.interact(rect, id, Sense::click_and_drag());

        let gutter_width = self.gutter_width(ui, &font_id, self.doc.buffer().line_count());
        self.view.line_height = line_height;
        self.view.viewport_height = rect.height();
        self.view.viewport_width = rect.width() - gutter_width;
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
                        if let Some(cmd) = translate_key(*key, *modifiers, page_rows, &self.mode) {
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
        let hit = |pos: egui::Pos2| self.hit_test(ui, &font_id, rect, gutter_width, pos);
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

        self.paint(ui, &font_id, rect, gutter_width);
        response
    }

    /// Width of the line-number gutter in code mode; zero otherwise.
    fn gutter_width(&self, ui: &Ui, font_id: &egui::FontId, line_count: usize) -> f32 {
        if !self.mode.is_code() {
            return 0.0;
        }
        let digits = (line_count.max(1).ilog10() as usize + 1).max(2);
        let char_width = ui.fonts_mut(|f| f.glyph_width(font_id, '0'));
        digits as f32 * char_width + 2.0 * GUTTER_PAD
    }

    /// Maps a pointer position to a document cursor via per-line galley
    /// hit-testing.
    fn hit_test(
        &self,
        ui: &Ui,
        font_id: &egui::FontId,
        rect: Rect,
        gutter_width: f32,
        pos: egui::Pos2,
    ) -> Cursor {
        let line_count = self.doc.buffer().line_count();
        let y = pos.y - rect.top() + self.view.scroll_y;
        let line = ((y / self.view.line_height).floor().max(0.0) as usize)
            .min(line_count.saturating_sub(1));
        let text = line_text(self.doc.buffer(), line);
        let galley = ui.fonts_mut(|f| f.layout_no_wrap(text, font_id.clone(), egui::Color32::WHITE));
        let ccursor = galley.cursor_from_pos(Vec2::new(
            pos.x - rect.left() - gutter_width + self.view.scroll_x,
            0.0,
        ));
        Cursor::new(line, ccursor.index)
    }

    /// Lays out one line's galley: syntax-colored via the span provider in
    /// code mode, single-color otherwise (and as the unknown-language
    /// fallback).
    fn line_galley(
        &mut self,
        ui: &Ui,
        font_id: &egui::FontId,
        line: usize,
        text: &str,
        text_color: Color32,
        theme: &CodeTheme,
    ) -> std::sync::Arc<egui::Galley> {
        let spans = match (self.mode.is_code(), self.span_provider.as_deref_mut()) {
            (true, Some(provider)) => {
                provider.line_spans(self.doc.buffer(), line, self.doc.revision())
            }
            _ => &[],
        };
        if spans.is_empty() {
            return ui
                .fonts_mut(|f| f.layout_no_wrap(text.to_string(), font_id.clone(), text_color));
        }
        let mut job = LayoutJob::default();
        job.wrap.max_width = f32::INFINITY;
        for span in spans {
            let (start, end) = (span.start.min(text.len()), span.end.min(text.len()));
            if start >= end {
                continue;
            }
            job.append(
                &text[start..end],
                0.0,
                TextFormat::simple(font_id.clone(), theme.color_for(span.kind, text_color)),
            );
        }
        ui.fonts_mut(|f| f.layout_job(job))
    }

    fn paint(&mut self, ui: &Ui, font_id: &egui::FontId, rect: Rect, gutter_width: f32) {
        let visuals = ui.visuals();
        let text_color = visuals.text_color();
        let code_theme = self
            .code_theme
            .unwrap_or_else(|| CodeTheme::from_visuals(visuals));
        // Text clips at the gutter edge so horizontal scroll never paints
        // under the line numbers.
        let text_rect = Rect::from_min_max(
            egui::pos2(rect.left() + gutter_width, rect.top()),
            rect.max,
        );
        let painter = ui.painter_at(text_rect);
        let selection_color = visuals.selection.bg_fill.linear_multiply(0.5);
        let caret_color = visuals.selection.stroke.color;
        let line_count = self.doc.buffer().line_count();
        let selection = self.doc.selection();
        let (sel_start, sel_end) = selection.ordered();
        let caret = self.doc.cursor();
        let mut caret_rect: Option<Rect> = None;
        let gutter_painter = ui.painter_at(rect);
        let show_code_chrome = self.mode.is_code();

        for line in self.view.visible_lines(line_count) {
            let top = rect.top() + self.view.line_top(line) - self.view.scroll_y;
            let text = line_text(self.doc.buffer(), line);
            let galley = self.line_galley(ui, font_id, line, &text, text_color, &code_theme);
            let origin = egui::pos2(rect.left() + gutter_width - self.view.scroll_x, top);

            if show_code_chrome {
                // Current-line highlight under everything else.
                if line == caret.line {
                    painter.rect_filled(
                        Rect::from_min_max(
                            egui::pos2(text_rect.left(), top),
                            egui::pos2(rect.right(), top + self.view.line_height),
                        ),
                        0.0,
                        code_theme.current_line_bg,
                    );
                }
                // Right-aligned 1-based line number in the gutter.
                gutter_painter.text(
                    egui::pos2(rect.left() + gutter_width - GUTTER_PAD, top),
                    egui::Align2::RIGHT_TOP,
                    (line + 1).to_string(),
                    font_id.clone(),
                    code_theme.gutter_text,
                );
            }

            // Find-match highlight fills under the text and selection.
            if !self.highlights.is_empty() {
                let line_start = self.doc.buffer().line_to_char(line);
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
/// lines, used by PageUp/PageDown. [`EditorMode::Markdown`] routes
/// Tab/Enter/Backspace to the Markdown-aware commands; plain and code modes
/// share the plain map (Tab/Shift-Tab already indent/outdent there).
fn translate_key(
    key: Key,
    modifiers: Modifiers,
    page_rows: usize,
    mode: &EditorMode,
) -> Option<EditorCommand> {
    let extend = modifiers.shift;
    let word = modifiers.alt;
    let line = modifiers.command;
    let mv = |movement: Movement| Some(EditorCommand::Move { movement, extend });
    if mode.is_markdown() {
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

    struct ModeState {
        doc: Document,
        view: ViewState,
        mode: EditorMode,
        highlighter: Option<crate::editor::SyntaxHighlighter>,
    }

    fn mode_harness(text: &str, mode: EditorMode) -> egui_kittest::Harness<'static, ModeState> {
        let highlighter = mode
            .language()
            .and_then(crate::editor::SyntaxHighlighter::new);
        let state = ModeState {
            doc: Document::new(text),
            view: ViewState::default(),
            mode,
            highlighter,
        };
        egui_kittest::Harness::new_ui_state(
            |ui, state: &mut ModeState| {
                let mut widget = EditorWidget::new(&mut state.doc, &mut state.view)
                    .mode(state.mode.clone());
                if let Some(h) = &mut state.highlighter {
                    widget = widget.span_provider(h);
                }
                widget.show(ui);
            },
            state,
        )
    }

    #[test]
    fn code_mode_tab_indents_and_shift_tab_outdents() {
        let mut h = mode_harness(
            "fn main() {}",
            EditorMode::Code {
                language: "rs".into(),
            },
        );
        // Tab with a collapsed caret inserts one indent step at the caret.
        h.key_press(Key::Tab);
        h.step();
        assert_eq!(h.state().doc.text(), "    fn main() {}");
        // Shift-Tab outdents the line.
        h.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
        h.step();
        assert_eq!(h.state().doc.text(), "fn main() {}");
        // Undo through the shared history works identically in code mode.
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.step();
        assert_eq!(h.state().doc.text(), "    fn main() {}");
    }

    #[test]
    fn mode_change_never_alters_document_state() {
        // Build up text, selection, history, and IME preedit in code mode…
        let mut h = mode_harness(
            "",
            EditorMode::Code {
                language: "rs".into(),
            },
        );
        h.event(Event::Text("héllo 😀".into()));
        h.step();
        h.key_press(Key::Enter);
        h.step();
        h.event(Event::Text("wörld".into()));
        h.step();
        h.key_press_modifiers(Modifiers::SHIFT | Modifiers::ALT, Key::ArrowLeft);
        h.step();
        h.event(Event::Ime(ImeEvent::Preedit("かん".into())));
        h.step();
        let before = h.state().doc.semantic_state(0.0);

        // …then flip through every mode without any input events: nothing in
        // the document (text, selection, undo history, IME) may change.
        for mode in [
            EditorMode::PlainText,
            EditorMode::Markdown,
            EditorMode::Code {
                language: "py".into(),
            },
        ] {
            h.state_mut().mode = mode.clone();
            h.state_mut().highlighter = mode
                .language()
                .and_then(crate::editor::SyntaxHighlighter::new);
            h.step();
            let after = h.state().doc.semantic_state(0.0);
            assert_eq!(after, before, "mode {mode:?} altered document state");
        }

        // Undo still walks the same history after the mode flips.
        h.event(Event::Ime(ImeEvent::Disabled));
        h.step();
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.step();
        assert_eq!(h.state().doc.text(), "héllo 😀\n");
    }

    #[test]
    fn unknown_code_language_falls_back_to_plain_spans() {
        let mode = EditorMode::Code {
            language: "not-a-language".into(),
        };
        assert!(mode.language().and_then(crate::editor::SyntaxHighlighter::new).is_none());
        // The widget still renders and edits without a provider.
        let mut h = mode_harness("some text", mode);
        h.event(Event::Text("!".into()));
        h.step();
        assert_eq!(h.state().doc.text(), "!some text");
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
