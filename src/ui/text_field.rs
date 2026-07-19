use crate::ui::theme::Colors;

pub(crate) fn styled_text_input(
    ui: &mut egui::Ui,
    buf: &mut String,
    hint: impl Into<egui::WidgetText>,
    id: egui::Id,
    colors: &Colors,
) -> egui::Response {
    styled_text_input_inner(ui, buf, hint, id, colors, false)
}

fn styled_text_input_inner(
    ui: &mut egui::Ui,
    buf: &mut String,
    hint: impl Into<egui::WidgetText>,
    id: egui::Id,
    colors: &Colors,
    password: bool,
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
        let font_id = egui::TextStyle::Body.resolve(ui.style());
        let row_height = ui.fonts(|f| f.row_height(&font_id));
        let edit = egui::TextEdit::singleline(buf)
            .id(id)
            .desired_width(f32::INFINITY)
            .hint_text(hint)
            .font(egui::TextStyle::Body)
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
        output.response
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
    let index = cursor_range.primary.ccursor.index;
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

    let cursor_pos = output.galley.pos_from_cursor(&cursor_range.primary);
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

pub(crate) struct TextField<'a> {
    id: egui::Id,
    hint: egui::WidgetText,
    surface: Option<crate::ui::focus::SurfaceKey>,
    select_all_on_focus: bool,
    password: bool,
    log_name: &'a str,
}

impl<'a> TextField<'a> {
    pub(crate) fn singleline(id: egui::Id, hint: impl Into<egui::WidgetText>) -> Self {
        Self {
            id,
            hint: hint.into(),
            surface: None,
            select_all_on_focus: false,
            password: false,
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

    pub(crate) fn select_all_on_focus(mut self, select_all_on_focus: bool) -> Self {
        self.select_all_on_focus = select_all_on_focus;
        self
    }

    pub(crate) fn password(mut self, password: bool) -> Self {
        self.password = password;
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
        let response = styled_text_input_inner(ui, buf, self.hint, self.id, colors, self.password);
        if let Some(key) = self.surface {
            crate::ui::focus::register_default_text_surface(ui.ctx(), key, self.id);
        }
        if self.select_all_on_focus && response.gained_focus() {
            log::info!("{}: select-all on focus gain", self.log_name);
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), self.id) {
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(buf.len()),
                    )));
                state.store(ui.ctx(), self.id);
            }
        }
        response
    }
}

pub(crate) struct TextArea<'a> {
    id: egui::Id,
    hint: egui::WidgetText,
    surface: Option<crate::ui::focus::SurfaceKey>,
    select_all_on_focus: bool,
    rows: usize,
    desired_width: f32,
    max_height: Option<f32>,
    font_id: egui::FontId,
    text_color: Option<egui::Color32>,
    hint_color: Option<egui::Color32>,
    margin: egui::Margin,
    frame: bool,
    log_name: &'a str,
}

impl<'a> TextArea<'a> {
    pub(crate) fn multiline(id: egui::Id, hint: impl Into<egui::WidgetText>) -> Self {
        Self {
            id,
            hint: hint.into(),
            surface: None,
            select_all_on_focus: false,
            rows: 3,
            desired_width: f32::INFINITY,
            max_height: None,
            font_id: egui::FontId::proportional(crate::ui::style::TEXT_BODY),
            text_color: None,
            hint_color: None,
            margin: egui::Margin::symmetric(8, 5),
            frame: true,
            log_name: "text_area",
        }
    }

    /// See [`TextField::surface`].
    pub(crate) fn surface(mut self, key: crate::ui::focus::SurfaceKey) -> Self {
        self.surface = Some(key);
        self
    }

    pub(crate) fn select_all_on_focus(mut self, select_all_on_focus: bool) -> Self {
        self.select_all_on_focus = select_all_on_focus;
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

    pub(crate) fn monospace(mut self, size: f32) -> Self {
        self.font_id = egui::FontId::monospace(size);
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
        let select_all_on_focus = self.select_all_on_focus;
        let log_name = self.log_name;
        let response = if let Some(max_height) = self.max_height {
            egui::ScrollArea::vertical()
                .max_height(max_height)
                .show(ui, |ui| self.show_editor(ui, buf, colors))
                .inner
        } else {
            self.show_editor(ui, buf, colors)
        };

        if let Some(key) = surface {
            crate::ui::focus::register_default_text_surface(ui.ctx(), key, id);
        }
        if select_all_on_focus && response.gained_focus() {
            log::info!("{log_name}: select-all on focus gain");
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(buf.len()),
                    )));
                state.store(ui.ctx(), id);
            }
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

            let row_height = ui.fonts(|f| f.row_height(&self.font_id));
            let output = egui::TextEdit::multiline(buf)
                .id(self.id)
                .font(self.font_id.clone())
                .text_color(self.text_color.unwrap_or(colors.text_primary))
                .desired_width(self.desired_width)
                .desired_rows(self.rows)
                .frame(self.frame)
                .margin(self.margin)
                .hint_text(hint)
                .show(ui);
            draw_text_caret(
                ui,
                &output,
                self.font_id.size,
                row_height,
                egui::Stroke::new(1.0_f32, colors.accent),
            );
            output.response
        })
        .inner
    }
}
