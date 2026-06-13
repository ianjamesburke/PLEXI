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
        ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::new(1.0, colors.accent);
        ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::new(1.0, colors.border);
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
            egui::Stroke::new(1.5, colors.accent),
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

    // Bare glyph height reads slightly short against the row; +1px splits
    // the difference with egui's full-row caret.
    let caret_height = caret_height + 1.0;

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
            [egui::pos2(cx, start_y), egui::pos2(cx, start_y + caret_height)],
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
    focused: bool,
    password: bool,
    log_name: &'a str,
}

impl<'a> TextField<'a> {
    pub(crate) fn singleline(id: egui::Id, hint: impl Into<egui::WidgetText>) -> Self {
        Self {
            id,
            hint: hint.into(),
            focused: false,
            password: false,
            log_name: "text_field",
        }
    }

    pub(crate) fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
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
        if self.focused && !response.has_focus() {
            response.request_focus();
            log::info!("{}: focus requested for host TextField", self.log_name);
        }
        if self.focused || response.has_focus() {
            crate::ui::focus::register_overlay_focus(ui.ctx(), self.id);
        }
        response
    }
}
