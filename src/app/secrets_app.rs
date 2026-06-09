use crate::app::app_trait::{App, AppCommand, AppRenderContext};
use crate::ui::{style, widgets};
use std::path::PathBuf;

#[derive(Clone)]
struct ManagedSecret {
    /// Full Keychain account string: `plexi:user:<name>` or `plexi:<ws-id>:<name>`.
    account: String,
    /// Friendly display name (everything after the second `:`).
    name: String,
    /// `"global"` for user-scoped, or the workspace ID for workspace-scoped.
    scope: String,
}

impl ManagedSecret {
    fn from_account(account: String) -> Option<Self> {
        let (scope_part, name) = account.strip_prefix("plexi:")?.split_once(':')?;
        let scope = if scope_part == "user" {
            "global".to_string()
        } else {
            scope_part.to_string()
        };
        let name = name.to_string();
        Some(Self { account, name, scope })
    }
}

fn load_entries() -> Vec<ManagedSecret> {
    #[cfg(target_os = "macos")]
    {
        use crate::workspace::secrets::{MacKeychain, SecretStore};
        MacKeychain::new()
            .list_with_prefix("plexi:")
            .into_iter()
            .filter_map(ManagedSecret::from_account)
            .collect()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    List,
    Adding,
}

#[derive(Clone, Copy, PartialEq)]
enum FormField {
    Key,
    Value,
}

pub struct SecretsApp {
    entries: Vec<ManagedSecret>,
    selected: usize,
    cwd: PathBuf,

    mode: Mode,
    new_key: String,
    new_value: String,
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
        let entries = load_entries();
        Self {
            entries,
            selected: 0,
            cwd,
            mode: Mode::List,
            new_key: String::new(),
            new_value: String::new(),
            form_focus: FormField::Key,
            focus_requested: false,
            pending_delete: false,
            copy_pending: false,
            pending_cmds: Vec::new(),
            status_msg: None,
        }
    }

    fn refresh(&mut self) {
        self.entries = load_entries();
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
        log::info!("secrets_manager: refreshed — {} entries", self.entries.len());
    }

    fn begin_add(&mut self) {
        self.new_key.clear();
        self.new_value.clear();
        self.form_focus = FormField::Key;
        self.focus_requested = false;
        self.mode = Mode::Adding;
    }

    fn cancel_add(&mut self) {
        self.mode = Mode::List;
    }

    fn commit_add(&mut self) {
        let name = self.new_key.trim().to_string();
        let value = self.new_value.trim().to_string();

        if name.is_empty() || value.is_empty() {
            self.status_msg = Some("Name and value cannot be empty.".to_string());
            return;
        }

        #[cfg(target_os = "macos")]
        {
            use crate::workspace::secrets::{
                keychain_user_name, keychain_workspace_name, MacKeychain, SecretStore,
                WorkspaceConfig,
            };

            // Resolve scope: workspace-scoped if cwd is inside an initialized workspace.
            let workspace = crate::app::registry::resolve_workspace_root(&self.cwd)
                .and_then(|root| {
                    WorkspaceConfig::load(&root)
                        .ok()
                        .flatten()
                        .map(|cfg| (root, cfg))
                });

            let (account, scope) = match &workspace {
                Some((_, cfg)) => (keychain_workspace_name(&cfg.id, &name), cfg.id.clone()),
                None => (keychain_user_name(&name), "global".to_string()),
            };

            let store = MacKeychain::new();
            match store.set(&account, &value) {
                Ok(()) => {
                    self.entries.retain(|e| e.account != account);
                    self.entries.push(ManagedSecret {
                        account,
                        name: name.clone(),
                        scope: scope.clone(),
                    });
                    self.selected = self.entries.len().saturating_sub(1);
                    self.mode = Mode::List;
                    self.status_msg = Some(format!("Stored '{name}'."));
                    log::info!("secrets_manager: stored secret '{name}' scope={scope}");
                }
                Err(e) => {
                    self.status_msg = Some("Failed to store secret — check logs.".to_string());
                    log::error!("secrets_manager: store failed for '{name}': {e}");
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            log::warn!("secrets_manager: commit_add: Keychain not available on this platform");
        }
    }

    fn delete_selected(&mut self) {
        if let Some(entry) = self.entries.get(self.selected).cloned() {
            #[cfg(target_os = "macos")]
            {
                use crate::workspace::secrets::{MacKeychain, SecretStore};
                let store = MacKeychain::new();
                match store.delete(&entry.account) {
                    Ok(()) => {
                        let name = entry.name.clone();
                        self.entries.remove(self.selected);
                        if self.selected > 0 && self.selected >= self.entries.len() {
                            self.selected = self.entries.len().saturating_sub(1);
                        }
                        self.status_msg = Some(format!("Deleted '{name}'."));
                        log::info!("secrets_manager: deleted '{name}'");
                    }
                    Err(e) => {
                        self.status_msg = Some("Failed to delete — check logs.".to_string());
                        log::error!("secrets_manager: delete failed for '{}': {e}", entry.name);
                    }
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                log::warn!("secrets_manager: delete_selected: Keychain not available");
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
            return KeyDisposition::Passthrough;
        }

        if self.pending_delete && input.key_pressed(egui::Key::Escape) {
            self.pending_delete = false;
            self.status_msg = None;
            return KeyDisposition::Consumed;
        }

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
            } else if let Some(entry) = self.entries.get(self.selected) {
                self.pending_delete = true;
                let name = entry.name.clone();
                self.status_msg =
                    Some(format!("Delete '{name}'? Press d/y to confirm, Esc to cancel."));
            }
            consumed = true;
        }
        if input.key_pressed(egui::Key::Y) && self.pending_delete && !self.entries.is_empty() {
            self.delete_selected();
            self.pending_delete = false;
            consumed = true;
        }
        if input.key_pressed(egui::Key::C) && !self.entries.is_empty() {
            self.pending_delete = false;
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
        const FORM_H: f32 = 140.0;

        // Process clipboard copy request (needs ui.ctx() — not available in handle_key).
        if self.copy_pending {
            self.copy_pending = false;
            if let Some(entry) = self.entries.get(self.selected) {
                let account = entry.account.clone();
                let name = entry.name.clone();
                #[cfg(target_os = "macos")]
                {
                    use crate::workspace::secrets::{MacKeychain, SecretStore};
                    match MacKeychain::new().get(&account) {
                        Some(value) => {
                            ui.ctx().copy_text((*value).clone());
                            self.status_msg = Some(format!("'{name}' copied to clipboard."));
                            log::info!("secrets_manager: copied value for '{name}'");
                        }
                        None => {
                            self.status_msg =
                                Some(format!("Could not retrieve '{name}' — check logs."));
                            log::warn!(
                                "secrets_manager: MacKeychain::get returned None for account='{account}'"
                            );
                        }
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
                    // RTL: emit rightmost first → visual L→R: [n] new [d] del [c] copy [r] refresh
                    widgets::key_combo_list(ui, &[&["r"]], Some("refresh"), colors);
                    widgets::key_combo_list(ui, &[&["c"]], Some("copy"), colors);
                    widgets::key_combo_list(ui, &[&["d"]], Some("del"), colors);
                    widgets::key_combo_list(ui, &[&["n"]], Some("new"), colors);
                }
            });
        });

        // ── Status message ───────────────────────────────────────────────────
        let status_h = if let Some(msg) = &self.status_msg {
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
                    egui::RichText::new(msg.as_str())
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

                // Name field
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Name  ")
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
                    if val_resp.has_focus()
                        && ui
                            .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                    {
                        self.commit_add();
                    }
                });

                ui.add_space(style::SPACE_SM);

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
        let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        egui::ScrollArea::vertical()
            .id_salt("secrets_list")
            .show(&mut list_ui, |ui| {
                if self.entries.is_empty() {
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

                for (idx, entry) in self.entries.iter().enumerate() {
                    let is_selected = idx == self.selected;
                    let (resp, _) = widgets::selectable_row(ui, is_selected, colors, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(&entry.name)
                                        .size(style::TEXT_BODY)
                                        .color(colors.text_primary),
                                );
                                ui.scope(|ui| {
                                    ui.set_max_width(300.0);
                                    widgets::description_label(ui, &entry.scope, colors);
                                });
                            });
                        });
                    });

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
