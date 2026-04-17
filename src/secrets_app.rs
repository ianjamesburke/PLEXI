use crate::app_trait::{App, AppCommand, AppRenderContext};
use crate::secrets::SecretEntry;
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
    form_focus: FormField,
    focus_requested: bool,

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
            form_focus: FormField::Key,
            focus_requested: false,
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
            // Optimistic update: push directly instead of re-dumping the keychain
            // (dump-keychain triggers a macOS permission prompt every call).
            // Remove any existing entry with the same key+dir to avoid duplicates.
            self.entries.retain(|e| !(e.key == key && e.directory == dir && e.app_id == APP_ID_USER));
            self.entries.push(SecretEntry {
                app_id: APP_ID_USER.to_string(),
                directory: dir,
                key: key.clone(),
                workspace_root: None, // v1/v2 legacy path — no workspace scoping
            });
            self.selected = self.entries.len().saturating_sub(1);
            self.mode = Mode::List;
            self.status_msg = Some(format!("Stored '{key}'. Press r to sync."));
        } else {
            self.status_msg = Some("Failed to store secret - check logs.".to_string());
        }
    }

    fn delete_selected(&mut self) {
        if let Some(entry) = self.entries.get(self.selected).cloned() {
            if crate::secrets::delete_secret(&entry.key, &entry.app_id, &entry.directory) {
                // Optimistic removal — no keychain dump needed.
                self.entries.remove(self.selected);
                if self.selected > 0 && self.selected >= self.entries.len() {
                    self.selected = self.entries.len().saturating_sub(1);
                }
                self.status_msg = Some(format!("Deleted '{}'.", entry.key));
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

    fn handle_key(&mut self, input: &egui::InputState) -> bool {
        if self.mode == Mode::Adding {
            // Escape cancels the form
            if input.key_pressed(egui::Key::Escape) {
                self.cancel_add();
                return true;
            }
            // Let all other keys go to TextEdit widgets in ui()
            return false;
        }

        // List mode
        if input.modifiers.command || input.modifiers.alt {
            return false;
        }

        let mut consumed = false;

        if input.key_pressed(egui::Key::J) || input.key_pressed(egui::Key::ArrowDown) {
            if !self.entries.is_empty() && self.selected < self.entries.len() - 1 {
                self.selected += 1;
            }
            consumed = true;
        }
        if input.key_pressed(egui::Key::K) || input.key_pressed(egui::Key::ArrowUp) {
            if self.selected > 0 {
                self.selected -= 1;
            }
            consumed = true;
        }
        if input.key_pressed(egui::Key::R) {
            self.refresh();
            consumed = true;
        }
        if input.key_pressed(egui::Key::N) {
            self.begin_add();
            consumed = true;
        }
        if input.key_pressed(egui::Key::D) && !self.entries.is_empty() {
            self.delete_selected();
            consumed = true;
        }

        consumed
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let colors = ctx.colors;
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, colors.terminal_bg);

        const HEADER_H: f32 = 44.0;
        const FORM_H: f32 = 164.0;
        const ROW_H: f32 = 52.0;

        // ── Header ──────────────────────────────────────────────────────────
        let header_rect = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), HEADER_H));
        ui.painter().rect_filled(header_rect, 0.0, colors.bg_sidebar);
        ui.painter().line_segment(
            [egui::pos2(header_rect.left(), header_rect.bottom()),
             egui::pos2(header_rect.right(), header_rect.bottom())],
            egui::Stroke::new(1.0, colors.border),
        );

        let hint = if self.mode == Mode::Adding {
            "Esc=cancel"
        } else {
            "n=new · d=delete · r=refresh · j/k=navigate"
        };

        let mut header_ui = ui.new_child(
            egui::UiBuilder::new().max_rect(header_rect.shrink2(egui::vec2(16.0, 0.0))),
        );
        header_ui.horizontal_centered(|ui| {
            ui.label(egui::RichText::new("Secrets").color(colors.accent).size(16.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(hint).color(colors.text_dim).size(11.0)
                    .family(egui::FontFamily::Monospace));
            });
        });

        // ── Status message ───────────────────────────────────────────────────
        if let Some(msg) = &self.status_msg.clone() {
            let status_rect = egui::Rect::from_min_size(
                egui::pos2(rect.left(), rect.top() + HEADER_H),
                egui::vec2(rect.width(), 28.0),
            );
            ui.painter().rect_filled(status_rect, 0.0, colors.bg_active);
            ui.painter().text(
                egui::pos2(status_rect.left() + 16.0, status_rect.center().y),
                egui::Align2::LEFT_CENTER,
                msg,
                egui::FontId::monospace(11.0),
                colors.text_dim,
            );
        }

        let status_h = if self.status_msg.is_some() { 28.0 } else { 0.0 };
        let list_top = rect.top() + HEADER_H + status_h + 1.0;

        // ── Add form (when active) ───────────────────────────────────────────
        if self.mode == Mode::Adding {
            let form_rect = egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.bottom() - FORM_H),
                rect.max,
            );
            ui.painter().rect_filled(form_rect, 0.0, colors.bg_sidebar);
            ui.painter().line_segment(
                [egui::pos2(form_rect.left(), form_rect.top()),
                 egui::pos2(form_rect.right(), form_rect.top())],
                egui::Stroke::new(1.0, colors.border),
            );

            let mut form_ui = ui.new_child(
                egui::UiBuilder::new().max_rect(form_rect.shrink2(egui::vec2(20.0, 16.0))),
            );

            form_ui.vertical(|ui| {
                ui.label(egui::RichText::new("New Secret").color(colors.accent).size(13.0).strong());
                ui.add_space(10.0);

                // Key field
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Key   ").color(colors.text_dim).size(11.0)
                        .family(egui::FontFamily::Monospace));
                    let key_te = egui::TextEdit::singleline(&mut self.new_key)
                        .desired_width(f32::INFINITY)
                        .font(egui::FontId::monospace(12.0))
                        .text_color(colors.text_primary)
                        .frame(true)
                        .hint_text("e.g. OPENAI_API_KEY");
                    let key_resp = ui.add(key_te);
                    // Request focus on the key field when form opens
                    if !self.focus_requested {
                        key_resp.request_focus();
                        self.focus_requested = true;
                    }
                    if key_resp.has_focus()  {
                        self.form_focus = FormField::Key;
                    }
                    // Tab advances to value field
                    if key_resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Tab)) {
                        self.form_focus = FormField::Value;
                    }
                });

                ui.add_space(6.0);

                // Value field (masked)
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Value ").color(colors.text_dim).size(11.0)
                        .family(egui::FontFamily::Monospace));
                    let val_te = egui::TextEdit::singleline(&mut self.new_value)
                        .desired_width(f32::INFINITY)
                        .font(egui::FontId::monospace(12.0))
                        .text_color(colors.text_primary)
                        .password(true)
                        .frame(true)
                        .hint_text("secret value");
                    let val_resp = ui.add(val_te);
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

                ui.add_space(6.0);

                // Directory field
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Dir   ").color(colors.text_dim).size(11.0)
                        .family(egui::FontFamily::Monospace));
                    let dir_te = egui::TextEdit::singleline(&mut self.new_dir)
                        .desired_width(f32::INFINITY)
                        .font(egui::FontId::monospace(12.0))
                        .text_color(colors.text_primary)
                        .frame(true)
                        .hint_text("/path/to/project");
                    let dir_resp = ui.add(dir_te);
                    if self.form_focus == FormField::Dir && !dir_resp.has_focus() {
                        dir_resp.request_focus();
                    }
                    if dir_resp.has_focus() {
                        self.form_focus = FormField::Dir;
                    }
                    // Enter on Dir field saves
                    if dir_resp.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                        self.commit_add();
                        return;
                    }
                });

                ui.add_space(10.0);

                // Enter from value field also saves (common shortcut)
                if self.form_focus == FormField::Value {
                    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                        self.commit_add();
                        return;
                    }
                }

                ui.horizontal(|ui| {
                    if ui.button(egui::RichText::new("Save").color(colors.accent).size(12.0)).clicked() {
                        self.commit_add();
                    }
                    ui.add_space(8.0);
                    if ui.button(egui::RichText::new("Cancel").color(colors.text_dim).size(12.0)).clicked() {
                        self.cancel_add();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new("Tab to advance · Enter to save · Esc to cancel")
                            .color(colors.text_dim.linear_multiply(0.6)).size(10.0)
                            .family(egui::FontFamily::Monospace));
                    });
                });
            });

            // List occupies space above form
            let list_rect = egui::Rect::from_min_max(
                egui::pos2(rect.left(), list_top),
                egui::pos2(rect.right(), form_rect.top() - 1.0),
            );
            self.draw_list(ui, colors, list_rect, ROW_H);
        } else {
            // Full-height list
            let list_rect = egui::Rect::from_min_max(
                egui::pos2(rect.left(), list_top),
                rect.max,
            );
            self.draw_list(ui, colors, list_rect, ROW_H);
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
    fn draw_list(&self, ui: &mut egui::Ui, colors: &crate::theme::Colors, rect: egui::Rect, row_h: f32) {
        let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        egui::ScrollArea::vertical()
            .id_salt("secrets_list")
            .show(&mut list_ui, |ui| {
                if self.entries.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("No secrets stored")
                            .color(colors.text_dim).size(14.0));
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("Press n to add one")
                            .color(colors.text_dim.linear_multiply(0.6)).size(11.0)
                            .family(egui::FontFamily::Monospace));
                    });
                    return;
                }

                for (idx, entry) in self.entries.iter().enumerate() {
                    let is_selected = idx == self.selected;

                    let row_resp = ui.allocate_rect(
                        egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), row_h)),
                        egui::Sense::click(),
                    );

                    // Note: click to select is read-only here (mutable borrow issues across the loop)
                    let row_rect = row_resp.rect;

                    if is_selected {
                        ui.painter().rect_filled(row_rect, 0.0, colors.bg_active);
                    }

                    if idx > 0 {
                        ui.painter().line_segment(
                            [egui::pos2(row_rect.left() + 16.0, row_rect.top()),
                             egui::pos2(row_rect.right(), row_rect.top())],
                            egui::Stroke::new(1.0, colors.border.linear_multiply(0.4)),
                        );
                    }

                    // Accent bar
                    if is_selected {
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(row_rect.min, egui::vec2(3.0, row_rect.height())),
                            0.0,
                            colors.accent,
                        );
                    }

                    ui.painter().text(
                        egui::pos2(row_rect.left() + 16.0, row_rect.top() + 12.0),
                        egui::Align2::LEFT_TOP,
                        &entry.key,
                        egui::FontId::proportional(14.0),
                        colors.text_primary,
                    );

                    let subtitle = if entry.directory.is_empty() || entry.directory == "/" {
                        format!("{}", entry.app_id)
                    } else {
                        format!("{} · {}", entry.app_id, entry.directory)
                    };
                    ui.painter().text(
                        egui::pos2(row_rect.left() + 16.0, row_rect.top() + 30.0),
                        egui::Align2::LEFT_TOP,
                        subtitle,
                        egui::FontId::monospace(11.0),
                        colors.text_dim.linear_multiply(0.75),
                    );
                }
            });
    }
}
