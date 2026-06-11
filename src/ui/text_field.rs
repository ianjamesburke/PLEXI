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
        ui.visuals_mut().text_cursor.stroke.width = 1.5;
        ui.visuals_mut().text_cursor.stroke.color = colors.accent;
        ui.visuals_mut().extreme_bg_color = colors.bg_active;
        ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::new(1.0, colors.accent);
        ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::new(1.0, colors.border);
        let edit = egui::TextEdit::singleline(buf)
            .id(id)
            .desired_width(f32::INFINITY)
            .hint_text(hint)
            .font(egui::TextStyle::Body)
            .margin(egui::Margin::symmetric(8, 5))
            .password(password);
        ui.add(edit)
    })
    .inner
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
