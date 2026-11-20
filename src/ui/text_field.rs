use crate::ui::theme::Colors;

pub(crate) fn styled_text_input(
    ui: &mut egui::Ui,
    buf: &mut String,
    hint: impl Into<egui::WidgetText>,
    id: egui::Id,
    colors: &Colors,
) -> egui::Response {
    styled_text_input_inner(ui, buf, hint, id, colors, false, None)
}

fn styled_text_input_inner(
    ui: &mut egui::Ui,
    buf: &mut String,
    hint: impl Into<egui::WidgetText>,
    id: egui::Id,
    colors: &Colors,
    password: bool,
    font_size: Option<f32>,
) -> egui::Response {
    let hint: egui::WidgetText = hint.into();
    let hint = hint.color(colors.text_primary.gamma_multiply(0.5));
    ui.scope(|ui| {
        // egui's caret is hidden (transparent, non-blinking); draw_text_caret
        // paints a glyph-height replacement on top.
        ui.visuals_mut().text_cursor.blink = false;
        ui.visuals_mut().text_cursor.stroke.color = egui::Color32::TRANSPARENT;
        ui.visuals_mut().extreme_bg_color = colors.bg_active;
        ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, colors.accent);
        ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, colors.border);
        // An explicit size comes from an owner that scales its content
        // (a pane's Cmd+/-); otherwise the field follows the global UI scale.
        let font_id = match font_size {
            Some(size) => egui::FontId::proportional(size),
            None => egui::TextStyle::Body.resolve(ui.style()),
        };
        let row_height = ui.fonts_mut(|f| f.row_height(&font_id));
        let edit = egui::TextEdit::singleline(buf)
            .id(id)
            .desired_width(f32::INFINITY)
            .text_color(colors.text_primary)
            .hint_text(hint)
            .font(font_id.clone())
            .margin(egui::Margin::symmetric(8, 5))
            .password(password);
        let output = edit.show(ui);
        draw_text_caret(
            ui,
            &output,
            font_id.size,
            row_height,
            egui::Stroke::new(1.0_f32, colors.accent),
        );
        output.response.response
    })
    .inner
}

/// Draw a glyph-height caret over a `TextEdit` whose built-in cursor was
/// hidden (`text_cursor.stroke.color = TRANSPARENT`, `text_cursor.blink =
/// false` in the surrounding scope).
///
/// egui paints its caret at full row height plus a 1.5px overshoot on each
/// end and exposes no height knob, so editors that want a caret matching the
/// glyph height must paint their own. The blink timer resets on any cursor
/// movement or edit so the caret stays solid while navigating. Never hide
/// egui's caret by painting background over it — that destroys glyph edges.
pub(crate) fn draw_text_caret(
    ui: &egui::Ui,
    output: &egui::widgets::text_edit::TextEditOutput,
    caret_height: f32,
    fallback_row_height: f32,
    stroke: egui::Stroke,
) {
    if !output.response.has_focus() {
        return;
    }
    let Some(cursor_range) = &output.cursor_range else {
        return;
    };
    let index = cursor_range.primary.index;
    let now = ui.ctx().input(|i| i.time);

    let state_id = output.response.id.with("caret_blink");
    let prev: Option<(usize, f64)> = ui.ctx().data(|d| d.get_temp(state_id));
    let moved = prev.is_none_or(|(last_index, _)| last_index != index);
    let blink_start = if moved || output.response.changed() || output.response.gained_focus() {
        now
    } else {
        prev.map_or(now, |(_, start)| start)
    };
    // Only take the data write lock when the stored state actually changes
    // (a blink_start reset or cursor move) — steady blinking writes nothing.
    if prev != Some((index, blink_start)) {
        ui.ctx()
            .data_mut(|d| d.insert_temp(state_id, (index, blink_start)));
    }

    // Match egui's TextEdit: no caret and no blink repaints while the OS
    // window is unfocused.
    if !ui.ctx().input(|i| i.focused) {
        return;
    }

    // Bare glyph height reads slightly short against the row; +2px keeps the
    // caret visible without returning to egui's full-row overshoot.
    let caret_height = caret_height + 2.0;

    let cursor_pos = output.galley.pos_from_cursor(cursor_range.primary);
    let row_h = if cursor_pos.height() > 0.01 {
        cursor_pos.height()
    } else {
        fallback_row_height
    };

    const ON: f64 = 0.5;
    const OFF: f64 = 0.5;
    let t = (now - blink_start) % (ON + OFF);
    if t < ON {
        let row_top = output.galley_pos.y + cursor_pos.min.y;
        let start_y = row_top + (row_h - caret_height) * 0.5;
        let cx = output.galley_pos.x + cursor_pos.center().x;
        ui.painter().line_segment(
            [
                egui::pos2(cx, start_y),
                egui::pos2(cx, start_y + caret_height),
            ],
            stroke,
        );
        ui.ctx().request_repaint_after_secs((ON - t) as f32);
    } else {
        ui.ctx().request_repaint_after_secs((ON + OFF - t) as f32);
    }
}

/// One open session of a text field's owner (stint 1559): the owner makes a
/// new one each time it opens (a dialog, an inline rename) and keeps it while
/// open, drawn or not, so an overlay that covers it and leaves is not a
/// reopen. A field with [`TextField::select_all_on_open`] selects its text on
/// its first focus in each session; focus that returns later (Tab back from
/// a dialog button, a click into the text, an overlay leaving) keeps the
/// user's caret and draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct OpenSession(u64);

impl OpenSession {
    pub(crate) fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

/// Whether this pass is the field's first focused pass in `session`.
///
/// Focus is read from memory rather than with egui's helpers, which are
/// unusable here, proven against the real build (stint 0545):
/// - `Response::gained_focus` compares against egui's `id_previous_frame`,
///   snapshotted at `begin_pass` — the post-frame reconciler
///   (`reconcile_egui_focus`, stint 0429) grants focus *after* every widget
///   has drawn, so by this widget's next pass the snapshot already names it
///   and the gained edge never fires.
/// - `Response::has_focus` is `i.focused && memory.has_focus(id)` — gated on
///   OS window focus. Reconciler focus deliberately ignores OS focus so that
///   CLI-injected input (`plexi pane send`, drive-host, harness) lands while
///   the window is blurred; an OS-gated edge detector reads `false` for the
///   entire blurred session and never fires.
fn first_focus_in_session(ctx: &egui::Context, id: egui::Id, session: OpenSession) -> bool {
    if !ctx.memory(|memory| memory.has_focus(id)) {
        return false;
    }
    let slot = id.with("plexi_select_all_session");
    ctx.data_mut(|data| {
        let first = data.get_temp::<OpenSession>(slot) != Some(session);
        data.insert_temp(slot, session);
        first
    })
}

fn select_all(ctx: &egui::Context, id: egui::Id, buf: &str, log_name: &str) {
    log::info!("{log_name}: select-all on first focus");
    if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(buf.chars().count()),
            )));
        state.store(ctx, id);
    }
}

fn rendered_text_key(id: egui::Id) -> egui::Id {
    id.with("plexi_rendered_text")
}

/// One owner for text and caret when a field's buffer is written outside its
/// TextEdit (stint 1093): a pane seeding search with the keystroke that opened
/// it, a completion, a clear. egui keeps the caret it last stored, which can
/// sit before the new text, so the next keystroke would land in front of it.
/// Text the field did not render last pass moves the caret to its end, once;
/// the TextEdit's own edits and a caret the user placed are left alone.
/// Returns the text rendered last pass, for the caller to update after show.
fn place_caret_after_owner_write(
    ctx: &egui::Context,
    id: egui::Id,
    buf: &str,
    log_name: &str,
) -> Option<String> {
    let rendered: Option<String> = ctx.data(|d| d.get_temp(rendered_text_key(id)));
    if rendered.as_deref() == Some(buf) {
        return rendered;
    }
    let end = buf.chars().count();
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(end),
        )));
    state.store(ctx, id);
    if rendered.is_some() {
        log::info!("{log_name}: caret placed after owner-written text ({end} chars)");
    }
    rendered
}

pub(crate) struct TextField<'a> {
    id: egui::Id,
    hint: egui::WidgetText,
    surface: Option<crate::ui::focus::SurfaceKey>,
    select_all_on_open: Option<OpenSession>,
    password: bool,
    font_size: Option<f32>,
    log_name: &'a str,
}

impl<'a> TextField<'a> {
    pub(crate) fn singleline(id: egui::Id, hint: impl Into<egui::WidgetText>) -> Self {
        Self {
            id,
            hint: hint.into(),
            surface: None,
            select_all_on_open: None,
            password: false,
            font_size: None,
            log_name: "text_field",
        }
    }

    /// Register this field as its owner's default text surface (stint 0429):
    /// the post-frame reconciler grants it egui focus while `key` is the
    /// input owner and surrenders it when ownership moves elsewhere. Fields
    /// without a surface key are transient (egui click-to-focus only).
    pub(crate) fn surface(mut self, key: crate::ui::focus::SurfaceKey) -> Self {
        self.surface = Some(key);
        self
    }

    /// Select the whole text on the field's first focus in `session`;
    /// later focus keeps the caret (see [`OpenSession`]).
    pub(crate) fn select_all_on_open(mut self, session: OpenSession) -> Self {
        self.select_all_on_open = Some(session);
        self
    }

    pub(crate) fn password(mut self, password: bool) -> Self {
        self.password = password;
        self
    }

    /// Render the text at `size` instead of the global UI body size, for an
    /// owner whose content follows its own font size (stint 1110 PR 1).
    pub(crate) fn font_size(mut self, size: f32) -> Self {
        self.font_size = Some(size);
        self
    }

    pub(crate) fn log_name(mut self, log_name: &'a str) -> Self {
        self.log_name = log_name;
        self
    }

    pub(crate) fn show(
        self,
        ui: &mut egui::Ui,
        buf: &mut String,
        colors: &Colors,
    ) -> egui::Response {
        let rendered = place_caret_after_owner_write(ui.ctx(), self.id, buf, self.log_name);
        let response = styled_text_input_inner(
            ui,
            buf,
            self.hint,
            self.id,
            colors,
            self.password,
            self.font_size,
        );
        if rendered.as_deref() != Some(buf.as_str()) {
            let key = rendered_text_key(self.id);
            ui.ctx().data_mut(|d| d.insert_temp(key, buf.clone()));
        }
        if let Some(key) = self.surface {
            crate::ui::focus::register_default_text_surface(ui.ctx(), key, self.id);
        }
        if self
            .select_all_on_open
            .is_some_and(|session| first_focus_in_session(ui.ctx(), self.id, session))
        {
            select_all(ui.ctx(), self.id, buf, self.log_name);
        }
        response
    }
}

/// What one input event did to an [`OrderedLineField`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FieldEdit {
    /// Not a field event: its owner decides what it means.
    Unclaimed,
    /// The field took the event; the committed text did not change.
    Claimed,
    /// The committed text changed to this value.
    Committed(String),
}

/// A single-line field that edits one input event at a time (stint 1391), so
/// its owner can interleave edits with its own keys in arrival order. A
/// `TextEdit` cannot: it consumes a whole frame's events at once and reports
/// only the frame-final text. Editing is the native editor's
/// [`crate::editor::Document`]: caret, selection replacement, undo and IME
/// preedit behave as they do in the editor pane; preedit is never committed
/// text.
#[derive(Debug)]
pub(crate) struct OrderedLineField {
    doc: crate::editor::Document,
}

/// Where the field was painted, for pointer caret placement next frame.
#[derive(Debug, Clone)]
pub(crate) struct FieldGeometry {
    pub rect: egui::Rect,
    galley: std::sync::Arc<egui::Galley>,
    galley_pos: egui::Pos2,
    /// Preedit characters inserted at the caret in the painted galley.
    preedit_at: Option<(usize, usize)>,
}

impl FieldGeometry {
    /// The committed-text char index under `pos`, or `None` inside preedit.
    fn char_at(&self, pos: egui::Pos2) -> Option<usize> {
        let index = self.galley.cursor_from_pos(pos - self.galley_pos).index;
        match self.preedit_at {
            Some((at, len)) if index > at && index < at + len => None,
            Some((at, len)) if index >= at + len => Some(index - len),
            _ => Some(index),
        }
    }
}

impl OrderedLineField {
    /// A field holding `text` with the caret at its end.
    pub(crate) fn new(text: &str) -> Self {
        let mut doc = crate::editor::Document::new(text);
        let end = crate::editor::movement::char_to_cursor(doc.buffer(), text.chars().count());
        doc.apply(crate::editor::EditorCommand::SetCursor(end));
        Self { doc }
    }

    /// An IME composition is in progress.
    pub(crate) fn composing(&self) -> bool {
        self.doc.ime().preedit().is_some_and(|p| !p.is_empty())
    }

    /// Apply one event. `copy` receives text the person copied or cut.
    pub(crate) fn apply(&mut self, event: &egui::Event, copy: &mut dyn FnMut(String)) -> FieldEdit {
        use crate::editor::commands::Movement;
        use crate::editor::EditorCommand as C;
        let before = self.doc.text();
        let command = match event {
            egui::Event::Text(text) => {
                if text.is_empty() || text.chars().any(char::is_control) {
                    return FieldEdit::Claimed;
                }
                C::InsertText(text.clone())
            }
            egui::Event::Paste(text) => {
                let line = text.replace(['\r', '\n'], " ");
                if line.is_empty() {
                    return FieldEdit::Claimed;
                }
                C::InsertText(line)
            }
            egui::Event::Copy | egui::Event::Cut => {
                if !self.doc.selection().is_range() {
                    return FieldEdit::Claimed;
                }
                copy(self.doc.selected_text());
                if matches!(event, egui::Event::Copy) {
                    return FieldEdit::Claimed;
                }
                C::Backspace
            }
            egui::Event::Ime(ime) => match ime {
                egui::ImeEvent::Enabled => return FieldEdit::Claimed,
                egui::ImeEvent::Preedit(text) if text == "\n" || text == "\r" => {
                    return FieldEdit::Claimed
                }
                egui::ImeEvent::Preedit(text) if text.is_empty() => C::ImeCancel,
                egui::ImeEvent::Preedit(text) => C::ImePreedit(text.clone()),
                egui::ImeEvent::Commit(text) if text == "\n" || text == "\r" => {
                    return FieldEdit::Claimed
                }
                egui::ImeEvent::Commit(text) => C::ImeCommit(text.clone()),
                egui::ImeEvent::Disabled => C::ImeCancel,
            },
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                let extend = modifiers.shift;
                let mv = |movement| C::Move { movement, extend };
                let command_key = modifiers.command && !modifiers.alt;
                match key {
                    egui::Key::ArrowLeft if modifiers.command => mv(Movement::LineStart),
                    egui::Key::ArrowLeft if modifiers.alt => mv(Movement::WordLeft),
                    egui::Key::ArrowLeft => mv(Movement::Left),
                    egui::Key::ArrowRight if modifiers.command => mv(Movement::LineEnd),
                    egui::Key::ArrowRight if modifiers.alt => mv(Movement::WordRight),
                    egui::Key::ArrowRight => mv(Movement::Right),
                    egui::Key::Home => mv(Movement::LineStart),
                    egui::Key::End => mv(Movement::LineEnd),
                    egui::Key::Backspace if command_key && !modifiers.shift => C::KillToLineStart,
                    egui::Key::Backspace if modifiers.alt && !modifiers.command => {
                        C::DeleteWordBackward
                    }
                    egui::Key::Backspace => C::Backspace,
                    egui::Key::Delete if command_key && !modifiers.shift => C::KillToLineEnd,
                    egui::Key::Delete if modifiers.alt && !modifiers.command => {
                        C::DeleteWordForward
                    }
                    egui::Key::Delete => C::DeleteForward,
                    egui::Key::A if command_key && !modifiers.shift => C::SelectAll,
                    egui::Key::Z if command_key && modifiers.shift => C::Redo,
                    egui::Key::Z if command_key => C::Undo,
                    egui::Key::Y if command_key => C::Redo,
                    _ => return FieldEdit::Unclaimed,
                }
            }
            _ => return FieldEdit::Unclaimed,
        };
        self.doc.apply(command);
        let after = self.doc.text();
        if after == before {
            FieldEdit::Claimed
        } else {
            FieldEdit::Committed(after)
        }
    }

    /// A pointer press (`extend` false) or drag (`extend` true) at `pos`.
    pub(crate) fn place_caret(&mut self, geometry: &FieldGeometry, pos: egui::Pos2, extend: bool) {
        if self.composing() {
            return;
        }
        let Some(index) = geometry.char_at(pos) else {
            return;
        };
        let cursor = crate::editor::movement::char_to_cursor(self.doc.buffer(), index);
        self.doc.apply(if extend {
            crate::editor::EditorCommand::ExtendTo(cursor)
        } else {
            crate::editor::EditorCommand::SetCursor(cursor)
        });
    }

    /// Paint the field with the look of [`TextField::singleline`] and
    /// register it as `surface`'s default text surface. Paints only: every
    /// edit already happened through [`Self::apply`].
    pub(crate) fn show(
        &self,
        ui: &mut egui::Ui,
        id: egui::Id,
        hint: &str,
        surface: crate::ui::focus::SurfaceKey,
        colors: &Colors,
    ) -> FieldGeometry {
        use crate::editor::movement::cursor_to_char;
        let font_id = egui::TextStyle::Body.resolve(ui.style());
        let row_height = ui.fonts_mut(|f| f.row_height(&font_id));
        let margin = egui::vec2(8.0, 5.0);
        let size = egui::vec2(ui.available_width(), row_height + 2.0 * margin.y);
        let (_, rect) = ui.allocate_space(size);
        ui.memory_mut(|m| m.interested_in_focus(id, ui.layer_id()));
        let response = ui
            .interact(rect, id, egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::Text);
        crate::ui::focus::register_default_text_surface(ui.ctx(), surface, id);
        let focused = ui.memory(|m| m.has_focus(id));

        let text = self.doc.text();
        let buffer = self.doc.buffer();
        let (start, end) = self.doc.selection().ordered();
        let (sel_start, sel_end) = (cursor_to_char(buffer, start), cursor_to_char(buffer, end));
        let caret = cursor_to_char(buffer, self.doc.cursor());
        let preedit = self.doc.ime().preedit().filter(|p| !p.is_empty());
        let mut job = egui::text::LayoutJob::default();
        let plain = egui::TextFormat::simple(font_id.clone(), colors.text_primary);
        let byte_at = |chars: usize| {
            text.char_indices()
                .nth(chars)
                .map_or(text.len(), |(b, _)| b)
        };
        let preedit_at = preedit.map(|p| (caret, p.chars().count()));
        match preedit {
            Some(p) => {
                job.append(&text[..byte_at(caret)], 0.0, plain.clone());
                let mut underlined = plain.clone();
                underlined.underline = egui::Stroke::new(1.0_f32, colors.text_primary);
                job.append(p, 0.0, underlined);
                job.append(&text[byte_at(caret)..], 0.0, plain.clone());
            }
            None if text.is_empty() => job.append(
                hint,
                0.0,
                egui::TextFormat::simple(font_id.clone(), colors.text_primary.gamma_multiply(0.5)),
            ),
            None => job.append(&text, 0.0, plain.clone()),
        }
        job.wrap.max_rows = 1;
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        let inner = rect.shrink2(margin);
        let caret_display = caret + preedit_at.map_or(0, |(_, len)| len);
        let caret_x = if text.is_empty() && preedit.is_none() {
            0.0
        } else {
            galley
                .pos_from_cursor(egui::text::CCursor::new(caret_display))
                .center()
                .x
        };
        // Keep the caret visible in a long query.
        let scroll = (caret_x - inner.width() + 2.0).max(0.0);
        let galley_pos = egui::pos2(
            inner.min.x - scroll,
            inner.center().y - galley.size().y / 2.0,
        );

        let visuals = ui.style().interact(&response);
        let stroke = if focused {
            ui.visuals().selection.stroke
        } else {
            egui::Stroke::new(1.0_f32, colors.border)
        };
        let painter = ui.painter_at(rect);
        painter.rect(
            rect,
            visuals.corner_radius,
            colors.bg_active,
            stroke,
            egui::StrokeKind::Inside,
        );
        if focused && sel_start != sel_end && preedit.is_none() {
            let from = galley.pos_from_cursor(egui::text::CCursor::new(sel_start));
            let to = galley.pos_from_cursor(egui::text::CCursor::new(sel_end));
            let selection =
                egui::Rect::from_x_y_ranges(from.min.x..=to.min.x, from.min.y..=from.max.y)
                    .translate(galley_pos.to_vec2());
            painter.rect_filled(selection, 0.0, ui.visuals().selection.bg_fill);
        }
        painter.galley(galley_pos, galley.clone(), colors.text_primary);

        if focused {
            let caret_rect = egui::Rect::from_center_size(
                egui::pos2(galley_pos.x + caret_x, inner.center().y),
                egui::vec2(1.0, font_id.size + 2.0),
            );
            paint_blinking_caret(ui, id, caret_display, &painter, caret_rect, colors.accent);
            // The OS places its IME candidate window here; without it the
            // platform never enables composition for this field.
            let to_global = ui
                .ctx()
                .layer_transform_to_global(ui.layer_id())
                .unwrap_or_default();
            ui.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    rect: to_global * rect,
                    cursor_rect: to_global * caret_rect,
                });
            });
        }
        response.widget_info(|| {
            egui::WidgetInfo::text_edit(ui.is_enabled(), text.as_str(), text.as_str(), hint)
        });
        FieldGeometry {
            rect,
            galley,
            galley_pos,
            preedit_at,
        }
    }
}

/// Paint a caret that is solid after any caret move and then blinks, as
/// [`draw_text_caret`] does for `TextEdit`.
fn paint_blinking_caret(
    ui: &egui::Ui,
    id: egui::Id,
    index: usize,
    painter: &egui::Painter,
    rect: egui::Rect,
    color: egui::Color32,
) {
    const ON: f64 = 0.5;
    const OFF: f64 = 0.5;
    let now = ui.ctx().input(|i| i.time);
    let state_id = id.with("caret_blink");
    let prev: Option<(usize, f64)> = ui.ctx().data(|d| d.get_temp(state_id));
    let blink_start = match prev {
        Some((last, start)) if last == index => start,
        _ => now,
    };
    if prev != Some((index, blink_start)) {
        ui.ctx()
            .data_mut(|d| d.insert_temp(state_id, (index, blink_start)));
    }
    if !ui.ctx().input(|i| i.focused) {
        return;
    }
    let t = (now - blink_start) % (ON + OFF);
    if t < ON {
        painter.rect_filled(rect, 0.0, color);
        ui.ctx().request_repaint_after_secs((ON - t) as f32);
    } else {
        ui.ctx().request_repaint_after_secs((ON + OFF - t) as f32);
    }
}

pub(crate) struct TextArea<'a> {
    id: egui::Id,
    hint: egui::WidgetText,
    surface: Option<crate::ui::focus::SurfaceKey>,
    select_all_on_open: Option<OpenSession>,
    rows: usize,
    desired_width: f32,
    max_height: Option<f32>,
    font_id: egui::FontId,
    text_color: Option<egui::Color32>,
    hint_color: Option<egui::Color32>,
    margin: egui::Margin,
    frame: bool,
    vertical_align: egui::Align,
    lock_focus: bool,
    return_key: Option<egui::KeyboardShortcut>,
    log_name: &'a str,
}

impl<'a> TextArea<'a> {
    pub(crate) fn multiline(id: egui::Id, hint: impl Into<egui::WidgetText>) -> Self {
        Self {
            id,
            hint: hint.into(),
            surface: None,
            select_all_on_open: None,
            rows: 3,
            desired_width: f32::INFINITY,
            max_height: None,
            font_id: egui::FontId::proportional(crate::ui::style::TEXT_BODY),
            text_color: None,
            hint_color: None,
            margin: egui::Margin::symmetric(8, 5),
            frame: true,
            vertical_align: egui::Align::Min,
            lock_focus: false,
            // Preserve egui TextEdit's multiline default unless a semantic
            // variant reserves Enter for its owner (for example, composer).
            return_key: Some(egui::KeyboardShortcut::new(
                egui::Modifiers::NONE,
                egui::Key::Enter,
            )),
            log_name: "text_area",
        }
    }

    /// A one-row composer that grows with wrapped content and keeps Tab in
    /// the editor. Plain Enter is reserved for the owning surface's submit
    /// handler; Shift+Enter is the editor's newline shortcut.
    pub(crate) fn composer(id: egui::Id, hint: impl Into<egui::WidgetText>) -> Self {
        Self::multiline(id, hint)
            .rows(1)
            .frame(false)
            .margin(egui::Margin {
                left: 6,
                right: 4,
                top: 2,
                bottom: 2,
            })
            .vertical_align(egui::Align::Center)
            .lock_focus(true)
            .return_key(egui::KeyboardShortcut::new(
                egui::Modifiers::SHIFT,
                egui::Key::Enter,
            ))
            .log_name("composer")
    }

    /// See [`TextField::surface`].
    pub(crate) fn surface(mut self, key: crate::ui::focus::SurfaceKey) -> Self {
        self.surface = Some(key);
        self
    }

    /// Select the whole text on the field's first focus in `session`;
    /// later focus keeps the caret (see [`OpenSession`]).
    pub(crate) fn select_all_on_open(mut self, session: OpenSession) -> Self {
        self.select_all_on_open = Some(session);
        self
    }

    pub(crate) fn rows(mut self, rows: usize) -> Self {
        self.rows = rows;
        self
    }

    pub(crate) fn desired_width(mut self, desired_width: f32) -> Self {
        self.desired_width = desired_width;
        self
    }

    pub(crate) fn max_height(mut self, max_height: f32) -> Self {
        self.max_height = Some(max_height);
        self
    }

    pub(crate) fn font(mut self, font_id: egui::FontId) -> Self {
        self.font_id = font_id;
        self
    }

    pub(crate) fn text_color(mut self, text_color: egui::Color32) -> Self {
        self.text_color = Some(text_color);
        self
    }

    pub(crate) fn hint_color(mut self, hint_color: egui::Color32) -> Self {
        self.hint_color = Some(hint_color);
        self
    }

    pub(crate) fn margin(mut self, margin: egui::Margin) -> Self {
        self.margin = margin;
        self
    }

    pub(crate) fn frame(mut self, frame: bool) -> Self {
        self.frame = frame;
        self
    }

    fn vertical_align(mut self, vertical_align: egui::Align) -> Self {
        self.vertical_align = vertical_align;
        self
    }

    fn lock_focus(mut self, lock_focus: bool) -> Self {
        self.lock_focus = lock_focus;
        self
    }

    fn return_key(mut self, return_key: egui::KeyboardShortcut) -> Self {
        self.return_key = Some(return_key);
        self
    }

    pub(crate) fn log_name(mut self, log_name: &'a str) -> Self {
        self.log_name = log_name;
        self
    }

    pub(crate) fn show(
        self,
        ui: &mut egui::Ui,
        buf: &mut String,
        colors: &Colors,
    ) -> egui::Response {
        let id = self.id;
        let surface = self.surface;
        let select_all_on_open = self.select_all_on_open;
        let log_name = self.log_name;
        let response = if let Some(max_height) = self.max_height {
            egui::ScrollArea::vertical()
                .id_salt(id.with("scroll"))
                .max_height(max_height)
                .show(ui, |ui| self.show_editor(ui, buf, colors))
                .inner
        } else {
            self.show_editor(ui, buf, colors)
        };

        if let Some(key) = surface {
            crate::ui::focus::register_default_text_surface(ui.ctx(), key, id);
        }
        if select_all_on_open.is_some_and(|session| first_focus_in_session(ui.ctx(), id, session)) {
            select_all(ui.ctx(), id, buf, log_name);
        }
        response
    }

    fn show_editor(self, ui: &mut egui::Ui, buf: &mut String, colors: &Colors) -> egui::Response {
        let hint_color = self
            .hint_color
            .unwrap_or_else(|| colors.text_primary.gamma_multiply(0.5));
        let hint = self.hint.color(hint_color);
        ui.scope(|ui| {
            // egui's caret is hidden (transparent, non-blinking); draw_text_caret
            // paints a glyph-height replacement on top.
            ui.visuals_mut().text_cursor.blink = false;
            ui.visuals_mut().text_cursor.stroke.color = egui::Color32::TRANSPARENT;
            ui.visuals_mut().extreme_bg_color = colors.bg_active;
            ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, colors.accent);
            ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, colors.border);

            let row_height = ui.fonts_mut(|f| f.row_height(&self.font_id));
            let edit = egui::TextEdit::multiline(buf)
                .id(self.id)
                .font(self.font_id.clone())
                .text_color(self.text_color.unwrap_or(colors.text_primary))
                .desired_width(self.desired_width)
                .desired_rows(self.rows)
                .margin(self.margin)
                .vertical_align(self.vertical_align)
                .lock_focus(self.lock_focus)
                .return_key(self.return_key)
                .hint_text(hint);
            let edit = if self.frame {
                edit
            } else {
                edit.frame(egui::Frame::NONE)
            };
            let output = edit.show(ui);
            draw_text_caret(
                ui,
                &output,
                self.font_id.size,
                row_height,
                egui::Stroke::new(1.0_f32, colors.accent),
            );
            output.response.response
        })
        .inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composer_reserves_plain_enter_and_uses_shift_enter_for_newlines() {
        let area = TextArea::composer(egui::Id::new("composer"), "hint");

        assert_eq!(area.rows, 1);
        assert!(area.lock_focus);
        assert_eq!(
            area.return_key,
            Some(egui::KeyboardShortcut::new(
                egui::Modifiers::SHIFT,
                egui::Key::Enter,
            ))
        );
    }

    #[test]
    fn ordinary_text_area_keeps_plain_enter_newlines() {
        let area = TextArea::multiline(egui::Id::new("ordinary"), "hint");

        assert_eq!(
            area.return_key,
            Some(egui::KeyboardShortcut::new(
                egui::Modifiers::NONE,
                egui::Key::Enter,
            ))
        );
    }
}
