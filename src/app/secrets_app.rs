use crate::app::app_trait::{App, AppCommand, AppRenderContext};
use crate::secrets::SecretEntry;
use crate::ui::{style, widgets};
use std::path::PathBuf;

/// Match the CLI app_id so `plexi secret list` sees manually-stored entries.
const APP_ID_USER: &str = "plexi-run";

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    List,
    Adding,
}

#[derive(Clone, Copy, PartialEq)]
enum FormField {
    Key,
    Value,
    Dir,
}

pub struct SecretsApp {
    entries: Vec<SecretEntry>,
    selected: usize,
    cwd: PathBuf,

    mode: Mode,
    new_key: String,
    new_value: String,
    new_dir: String,
    new_inject: bool,
    form_focus: FormField,
    focus_requested: bool,

    /// First `d` sets this; second `d` or `y` confirms the delete.
    pending_delete: bool,
    /// Set in handle_key; processed in ui() where ui.ctx() is available.
    copy_pending: bool,

    pending_cmds: Vec<AppCommand>,
    status_msg: Option<String>,
}

impl SecretsApp {
    pub fn new(cwd: PathBuf) -> Self {
        let entries = crate::secrets::list_all_secrets();
        let dir = cwd.to_string_lossy().to_string();
        Self {
            entries,
            selected: 0,
            cwd,
            mode: Mode::List,
            new_key: String::new(),
            new_value: String::new(),
            new_dir: dir,
            new_inject: false,
            form_focus: FormField::Key,
            focus_requested: false,
            pending_delete: false,
            copy_pending: false,
            pending_cmds: Vec::new(),
            status_msg: None,
        }
    }

    fn refresh(&mut self) {
        self.entries = crate::secrets::list_all_secrets();
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }

    fn begin_add(&mut self) {
        self.new_key.clear();
        self.new_value.clear();
        self.new_dir = self.cwd.to_string_lossy().to_string();
        self.new_inject = false;
        self.form_focus = FormField::Key;
        self.focus_requested = false;
        self.mode = Mode::Adding;
    }

    fn cancel_add(&mut self) {
        self.mode = Mode::List;
    }

    fn commit_add(&mut self) {
        let key = self.new_key.trim().to_string();
        let value = self.new_value.trim().to_string();
        let dir = self.new_dir.trim().to_string();

        if key.is_empty() || value.is_empty() {
            self.status_msg = Some("Key and value cannot be empty.".to_string());
            return;
        }

        if crate::secrets::store_secret(&key, &value, APP_ID_USER, &dir) {
            if self.new_inject {
                crate::secrets::toggle_inject_secret(&key, APP_ID_USER, &dir);
            }
            // Optimistic update: push directly instead of re-dumping the keychain
            // (dump-keychain triggers a macOS permission prompt every call).
            self.entries
                .retain(|e| !(e.key == key && e.directory == dir && e.app_id == APP_ID_USER));
            self.entries.push(SecretEntry {
                app_id: APP_ID_USER.to_string(),
                directory: dir,
                key: key.clone(),
                workspace_root: None, // v1/v2 legacy path — no workspace scoping
                inject: self.new_inject,
            });
            self.selected = self.entries.len().saturating_sub(1);
            self.mode = Mode::List;
            self.status_msg = Some(format!("Stored '{key}'. Press r to sync."));
            log::info!("secrets_manager: stored key '{key}'");
        } else {
            self.status_msg = Some("Failed to store secret - check logs.".to_string());
        }
    }

    fn toggle_inject_selected(&mut self) {
        if let Some(entry) = self.entries.get(self.selected).cloned() {
            match crate::secrets::toggle_inject_secret(&entry.key, &entry.app_id, &entry.directory)
            {
                Some(new_inject) => {
                    if let Some(e) = self.entries.get_mut(self.selected) {
                        e.inject = new_inject;
                    }
                    let label = if new_inject { "inject enabled" } else { "inject disabled" };
                    self.status_msg = Some(format!("'{}' — {label}.", entry.key));
                    log::info!(
                        "secrets_manager: toggled inject for '{}' -> {new_inject}",
                        entry.key
                    );
                }
                None => {
                    self.status_msg =
                        Some("Secret not found in index — press r to refresh".to_string());
                }
            }
        }
    }

    fn delete_selected(&mut self) {
        if let Some(entry) = self.entries.get(self.selected).cloned() {
            if crate::secrets::delete_secret(&entry.key, &entry.app_id, &entry.directory) {
                self.entries.remove(self.selected);
                if self.selected > 0 && self.selected >= self.entries.len() {
                    self.selected = self.entries.len().saturating_sub(1);
                }
                self.status_msg = Some(format!("Deleted '{}'.", entry.key));
                log::info!("secrets_manager: deleted key '{}'", entry.key);
            } else {
                self.status_msg = Some("Failed to delete - check logs.".to_string());
            }
        }
    }
}

impl App for SecretsApp {
    fn type_id(&self) -> &'static str {
        "secrets_manager"
    }

    fn display_name(&self) -> String {
        "Secrets".to_string()
    }

    fn sync_cwd(&mut self, new_cwd: &std::path::Path) {
        self.cwd = new_cwd.to_path_buf();
    }

    fn handle_key(&mut self, input: &egui::InputState) -> crate::app::app_trait::KeyDisposition {
        use crate::app::app_trait::KeyDisposition;
        if self.mode == Mode::Adding {
            if input.key_pressed(egui::Key::Escape) {
                self.cancel_add();
                return KeyDisposition::Consumed;
            }
            // Let all other keys go to TextEdit widgets in ui()
            return KeyDisposition::Passthrough;
        }

        // Cancel pending delete with Escape before checking modifiers.
        if self.pending_delete && input.key_pressed(egui::Key::Escape) {
            self.pending_delete = false;
            self.status_msg = None;
            return KeyDisposition::Consumed;
        }

        // List mode — don't intercept modified keys.
        if input.modifiers.command || input.modifiers.alt {
            return KeyDisposition::Passthrough;
        }

        let mut consumed = false;

        if input.key_pressed(egui::Key::J) || input.key_pressed(egui::Key::ArrowDown) {
            if !self.entries.is_empty() && self.selected < self.entries.len() - 1 {
                self.selected += 1;
                self.pending_delete = false;
            }
            consumed = true;
        }
        if input.key_pressed(egui::Key::K) || input.key_pressed(egui::Key::ArrowUp) {
            if self.selected > 0 {
                self.selected -= 1;
                self.pending_delete = false;
            }
            consumed = true;
        }
        if input.key_pressed(egui::Key::R) {
            self.refresh();
            self.pending_delete = false;
            consumed = true;
        }
        if input.key_pressed(egui::Key::N) {
            self.pending_delete = false;
            self.begin_add();
            consumed = true;
        }
        if input.key_pressed(egui::Key::D) && !self.entries.is_empty() {
            if self.pending_delete {
                self.delete_selected();
                self.pending_delete = false;
            } else {
                self.pending_delete = true;
                let key = self.entries[self.selected].key.clone();
                self.status_msg =
                    Some(format!("Delete '{key}'? Press d/y to confirm, Esc to cancel."));
            }
            consumed = true;
        }
        if input.key_pressed(egui::Key::Y) && self.pending_delete && !self.entries.is_empty() {
            self.delete_selected();
            self.pending_delete = false;
            consumed = true;
        }
        if input.key_pressed(egui::Key::I) && !self.entries.is_empty() {
            self.pending_delete = false;
            self.toggle_inject_selected();
            consumed = true;
        }
        if input.key_pressed(egui::Key::C) && !self.entries.is_empty() {
            self.copy_pending = true;
            consumed = true;
        }

        if consumed { KeyDisposition::Consumed } else { KeyDisposition::Passthrough }
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let colors = ctx.colors;
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, colors.terminal_bg);

        const HEADER_H: f32 = 44.0;
        const FORM_H: f32 = 188.0;

        // Process clipboard copy request (needs ui.ctx() — not available in handle_key).
        if self.copy_pending {
            self.copy_pending = false;
            if let Some(entry) = self.entries.get(self.selected) {
                let key = entry.key.clone();
                let app_id = entry.app_id.clone();
                let dir = entry.directory.clone();
                match crate::secrets::retrieve_secret(&key, &app_id, &dir) {
                    Some(value) => {
                        ui.ctx().copy_text((*value).clone());
                        self.status_msg = Some(format!("'{}' copied to clipboard.", key));
                        log::info!("secrets_manager: copied value for key '{key}'");
                    }
                    None => {
                        self.status_msg =
                            Some(format!("Could not retrieve '{key}' — check logs."));
                        log::warn!(
                            "secrets_manager: retrieve_secret returned None for key '{key}'"
                        );
                    }
                }
            }
        }

        // ── Header ──────────────────────────────────────────────────────────
        let header_rect = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), HEADER_H));
        ui.painter().rect_filled(header_rect, 0.0, colors.bg_sidebar);
        ui.painter().line_segment(
            [
                egui::pos2(header_rect.left(), header_rect.bottom()),
                egui::pos2(header_rect.right(), header_rect.bottom()),
            ],
            egui::Stroke::new(1.0, colors.border),
        );

        let mut header_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(header_rect.shrink2(egui::vec2(style::SPACE_MD, 0.0))),
        );
        header_ui.horizontal_centered(|ui| {
            ui.label(
                egui::RichText::new("Secrets")
                    .color(colors.accent)
                    .size(style::TEXT_BODY)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.mode == Mode::Adding {
                    widgets::key_combo_list(ui, &[&["Esc"]], Some("cancel"), colors);
                } else {
                    widgets::key_combo_list(
                        ui,
                        &[&["n"], &["d"], &["c"], &["i"], &["r"]],
                        Some("new · del · copy · inject · refresh"),
                        colors,
                    );
                }
            });
        });

        // ── Status message ───────────────────────────────────────────────────
        let status_h = if let Some(msg) = self.status_msg.clone() {
            let status_rect = egui::Rect::from_min_size(
                egui::pos2(rect.left(), rect.top() + HEADER_H),
                egui::vec2(rect.width(), 28.0),
            );
            ui.painter().rect_filled(status_rect, 0.0, colors.bg_active);
            let mut status_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(status_rect.shrink2(egui::vec2(style::SPACE_MD, 0.0))),
            );
            status_ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new(msg)
                        .size(style::TEXT_HINT)
                        .color(colors.text_dim)
                        .family(egui::FontFamily::Monospace),
                );
            });
            28.0_f32
        } else {
            0.0_f32
        };

        let list_top = rect.top() + HEADER_H + status_h + 1.0;

        // ── Add form (when active) ───────────────────────────────────────────
        if self.mode == Mode::Adding {
            let form_rect =
                egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - FORM_H), rect.max);
            ui.painter().rect_filled(form_rect, 0.0, colors.bg_sidebar);
            ui.painter().line_segment(
                [
                    egui::pos2(form_rect.left(), form_rect.top()),
                    egui::pos2(form_rect.right(), form_rect.top()),
                ],
                egui::Stroke::new(1.0, colors.border),
            );

            let mut form_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(form_rect.shrink2(egui::vec2(20.0, style::SPACE_SM * 2.0))),
            );

            form_ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("New Secret")
                        .color(colors.accent)
                        .size(style::TEXT_CAPTION)
                        .strong(),
                );
                ui.add_space(style::SPACE_SM);

                let styled_input = |ui: &mut egui::Ui, edit: egui::TextEdit| -> egui::Response {
                    ui.scope(|ui| {
                        ui.visuals_mut().text_cursor.stroke.width = 1.5;
                        ui.visuals_mut().text_cursor.stroke.color = colors.accent;
                        ui.visuals_mut().extreme_bg_color = colors.bg_active;
                        ui.visuals_mut().widgets.active.bg_stroke =
                            egui::Stroke::new(1.0, colors.accent);
                        ui.visuals_mut().widgets.inactive.bg_stroke =
                            egui::Stroke::new(1.0, colors.border);
                        ui.add(edit)
                    })
                    .inner
                };

                // Key field
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Key   ")
                            .color(colors.text_dim)
                            .size(style::TEXT_HINT)
                            .family(egui::FontFamily::Monospace),
                    );
                    let key_resp = styled_input(
                        ui,
                        egui::TextEdit::singleline(&mut self.new_key)
                            .desired_width(f32::INFINITY)
                            .font(egui::FontId::monospace(style::TEXT_CAPTION))
                            .text_color(colors.text_primary)
                            .frame(true)
                            .margin(egui::Margin::symmetric(8, 5))
                            .hint_text("e.g. OPENAI_API_KEY"),
                    );
                    if !self.focus_requested {
                        key_resp.request_focus();
                        self.focus_requested = true;
                    }
                    if key_resp.has_focus() {
                        self.form_focus = FormField::Key;
                    }
                    if key_resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Tab)) {
                        self.form_focus = FormField::Value;
                    }
                });

                ui.add_space(style::SPACE_XS);

                // Value field (masked)
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Value ")
                            .color(colors.text_dim)
                            .size(style::TEXT_HINT)
                            .family(egui::FontFamily::Monospace),
                    );
                    let val_resp = styled_input(
                        ui,
                        egui::TextEdit::singleline(&mut self.new_value)
                            .desired_width(f32::INFINITY)
                            .font(egui::FontId::monospace(style::TEXT_CAPTION))
                            .text_color(colors.text_primary)
                            .password(true)
                            .frame(true)
                            .margin(egui::Margin::symmetric(8, 5))
                            .hint_text("secret value"),
                    );
                    if self.form_focus == FormField::Value && !val_resp.has_focus() {
                        val_resp.request_focus();
                    }
                    if val_resp.has_focus() {
                        self.form_focus = FormField::Value;
                    }
                    if val_resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Tab)) {
                        self.form_focus = FormField::Dir;
                    }
                });

                ui.add_space(style::SPACE_XS);

                // Directory field
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Dir   ")
                            .color(colors.text_dim)
                            .size(style::TEXT_HINT)
                            .family(egui::FontFamily::Monospace),
                    );
                    let dir_resp = styled_input(
                        ui,
                        egui::TextEdit::singleline(&mut self.new_dir)
                            .desired_width(f32::INFINITY)
                            .font(egui::FontId::monospace(style::TEXT_CAPTION))
                            .text_color(colors.text_primary)
                            .frame(true)
                            .margin(egui::Margin::symmetric(8, 5))
                            .hint_text("/path/to/project"),
                    );
                    if self.form_focus == FormField::Dir && !dir_resp.has_focus() {
                        dir_resp.request_focus();
                    }
                    if dir_resp.has_focus() {
                        self.form_focus = FormField::Dir;
                    }
                    if dir_resp.has_focus()
                        && ui
                            .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                    {
                        self.commit_add();
                    }
                });

                ui.add_space(style::SPACE_XS);

                // Inject toggle
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Inject")
                            .color(colors.text_dim)
                            .size(style::TEXT_HINT)
                            .family(egui::FontFamily::Monospace),
                    );
                    ui.add_space(style::SPACE_XS);
                    ui.checkbox(
                        &mut self.new_inject,
                        egui::RichText::new(
                            "inject as env var into new terminal panes (e.g. for API keys)",
                        )
                        .color(colors.text_dim.linear_multiply(0.65))
                        .size(style::TEXT_HINT)
                        .family(egui::FontFamily::Monospace),
                    );
                });

                ui.add_space(style::SPACE_SM);

                if self.form_focus == FormField::Value
                    && ui
                        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                {
                    self.commit_add();
                }

                ui.horizontal(|ui| {
                    if ui
                        .button(
                            egui::RichText::new("Save")
                                .color(colors.accent)
                                .size(style::TEXT_CAPTION),
                        )
                        .clicked()
                    {
                        self.commit_add();
                    }
                    ui.add_space(style::SPACE_SM);
                    if ui
                        .button(
                            egui::RichText::new("Cancel")
                                .color(colors.text_dim)
                                .size(style::TEXT_CAPTION),
                        )
                        .clicked()
                    {
                        self.cancel_add();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new("Tab to advance · Enter to save · Esc to cancel")
                                .color(colors.text_dim.linear_multiply(0.6))
                                .size(style::TEXT_HINT)
                                .family(egui::FontFamily::Monospace),
                        );
                    });
                });
            });

            let list_rect = egui::Rect::from_min_max(
                egui::pos2(rect.left(), list_top),
                egui::pos2(rect.right(), form_rect.top() - 1.0),
            );
            self.draw_list(ui, colors, list_rect);
        } else {
            let list_rect = egui::Rect::from_min_max(egui::pos2(rect.left(), list_top), rect.max);
            self.draw_list(ui, colors, list_rect);
        }
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        std::mem::take(&mut self.pending_cmds)
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "selected": self.selected }))
    }

    fn restore_state(&mut self, state: &serde_json::Value) {
        if let Some(s) = state.get("selected").and_then(|v| v.as_u64()) {
            self.selected = s as usize;
        }
    }
}

impl SecretsApp {
    fn draw_list(
        &mut self,
        ui: &mut egui::Ui,
        colors: &crate::ui::theme::Colors,
        rect: egui::Rect,
    ) {
        struct RowData {
            key: String,
            subtitle: String,
            inject: bool,
        }

        let rows: Vec<RowData> = self
            .entries
            .iter()
            .map(|e| RowData {
                key: e.key.clone(),
                subtitle: if e.directory.is_empty() || e.directory == "/" {
                    e.app_id.clone()
                } else {
                    format!("{} · {}", e.app_id, e.directory)
                },
                inject: e.inject,
            })
            .collect();

        let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        egui::ScrollArea::vertical()
            .id_salt("secrets_list")
            .show(&mut list_ui, |ui| {
                if rows.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new("No secrets stored")
                                .color(colors.text_dim)
                                .size(style::TEXT_BODY),
                        );
                        ui.add_space(style::SPACE_SM);
                        ui.label(
                            egui::RichText::new(
                                "Use `plexi secret set` or press n to add the first one",
                            )
                            .color(colors.text_dim.linear_multiply(0.6))
                            .size(style::TEXT_HINT)
                            .family(egui::FontFamily::Monospace),
                        );
                    });
                    return;
                }

                let mut clicked_idx: Option<usize> = None;

                for (idx, row) in rows.iter().enumerate() {
                    let is_selected = idx == self.selected;
                    let (resp, _) = widgets::selectable_row(ui, is_selected, colors, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(&row.key)
                                        .size(style::TEXT_BODY)
                                        .color(colors.text_primary),
                                );
                                ui.scope(|ui| {
                                    ui.set_max_width(300.0);
                                    widgets::description_label(ui, &row.subtitle, colors);
                                });
                            });
                            if row.inject {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            egui::RichText::new("→env")
                                                .size(style::TEXT_HINT)
                                                .color(colors.accent)
                                                .family(egui::FontFamily::Monospace),
                                        );
                                    },
                                );
                            }
                        });
                    });

                    // Accent bar drawn over the row rect after layout is known.
                    if is_selected {
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                resp.rect.min,
                                egui::vec2(3.0, resp.rect.height()),
                            ),
                            0.0,
                            colors.accent,
                        );
                    }

                    if resp.clicked() {
                        clicked_idx = Some(idx);
                    }
                }

                if let Some(idx) = clicked_idx {
                    self.selected = idx;
                    self.pending_delete = false;
                }
            });
    }
}
