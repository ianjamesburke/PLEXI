use crate::app_trait::{App, AppCommand, AppRenderContext};
use crate::secrets::SecretEntry;

pub struct SecretsApp {
    entries: Vec<SecretEntry>,
    selected: usize,
    scroll_offset: usize,
}

impl SecretsApp {
    pub fn new() -> Self {
        let entries = crate::secrets::list_all_secrets();
        Self {
            entries,
            selected: 0,
            scroll_offset: 0,
        }
    }

    fn refresh(&mut self) {
        self.entries = crate::secrets::list_all_secrets();
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
        self.scroll_offset = self.scroll_offset.min(self.entries.len().saturating_sub(1));
    }
}

impl App for SecretsApp {
    fn type_id(&self) -> &'static str {
        "secrets_manager"
    }

    fn display_name(&self) -> String {
        "Secrets".to_string()
    }

    fn handle_key(&mut self, input: &egui::InputState) -> bool {
        if input.modifiers.command {
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

        consumed
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let colors = ctx.colors;
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, colors.terminal_bg);

        let header_height = 44.0;
        let row_height = 52.0;

        // ── Header ──────────────────────────────────────────────────
        let header_rect = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), header_height));
        ui.painter().rect_filled(header_rect, 0.0, colors.bg_sidebar);

        // Bottom border under header
        ui.painter().line_segment(
            [
                egui::pos2(header_rect.left(), header_rect.bottom()),
                egui::pos2(header_rect.right(), header_rect.bottom()),
            ],
            egui::Stroke::new(1.0, colors.border),
        );

        let mut header_ui = ui.new_child(
            egui::UiBuilder::new().max_rect(header_rect.shrink2(egui::vec2(16.0, 0.0))),
        );
        header_ui.horizontal_centered(|ui| {
            ui.label(
                egui::RichText::new("Secrets")
                    .color(colors.accent)
                    .size(16.0)
                    .strong()
                    .family(egui::FontFamily::Proportional),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new("j/k navigate · r refresh · read-only")
                        .color(colors.text_dim)
                        .size(11.0)
                        .family(egui::FontFamily::Monospace),
                );
            });
        });

        // ── Scrollable list ──────────────────────────────────────────
        let list_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.top() + header_height + 1.0),
            rect.max,
        );

        let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(list_rect));

        egui::ScrollArea::vertical()
            .id_salt("secrets_list")
            .show(&mut list_ui, |ui| {
                if self.entries.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new("No secrets found")
                                .color(colors.text_dim)
                                .size(14.0)
                                .family(egui::FontFamily::Monospace),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new("Use `plexi secret set <key>` to store one")
                                .color(colors.text_dim.linear_multiply(0.6))
                                .size(11.0)
                                .family(egui::FontFamily::Monospace),
                        );
                    });
                    return;
                }

                for (idx, entry) in self.entries.iter().enumerate() {
                    let is_selected = idx == self.selected;

                    let row_resp = ui.allocate_rect(
                        egui::Rect::from_min_size(
                            ui.cursor().min,
                            egui::vec2(ui.available_width(), row_height),
                        ),
                        egui::Sense::click(),
                    );

                    if row_resp.clicked() {
                        self.selected = idx;
                    }

                    let row_rect = row_resp.rect;

                    // Row background
                    if is_selected {
                        ui.painter().rect_filled(row_rect, 0.0, colors.bg_active);
                    }

                    // Separator line
                    if idx > 0 {
                        ui.painter().line_segment(
                            [
                                egui::pos2(row_rect.left() + 16.0, row_rect.top()),
                                egui::pos2(row_rect.right(), row_rect.top()),
                            ],
                            egui::Stroke::new(1.0, colors.border.linear_multiply(0.5)),
                        );
                    }

                    // Key name
                    let key_pos = egui::pos2(row_rect.left() + 16.0, row_rect.top() + 12.0);
                    ui.painter().text(
                        key_pos,
                        egui::Align2::LEFT_TOP,
                        &entry.key,
                        egui::FontId::proportional(14.0),
                        if is_selected { colors.text_primary } else { colors.text_primary.linear_multiply(0.9) },
                    );

                    // app_id / directory (secondary line)
                    let subtitle = if entry.directory.is_empty() || entry.directory == "/" {
                        entry.app_id.clone()
                    } else {
                        format!("{} · {}", entry.app_id, entry.directory)
                    };
                    let sub_pos = egui::pos2(row_rect.left() + 16.0, row_rect.top() + 30.0);
                    ui.painter().text(
                        sub_pos,
                        egui::Align2::LEFT_TOP,
                        subtitle,
                        egui::FontId::monospace(11.0),
                        colors.text_dim.linear_multiply(0.75),
                    );

                    // Accent bar on selected row
                    if is_selected {
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                row_rect.min,
                                egui::vec2(3.0, row_rect.height()),
                            ),
                            0.0,
                            colors.accent,
                        );
                    }
                }
            });
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        vec![]
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
