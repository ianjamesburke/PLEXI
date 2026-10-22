//! Native master/detail fleet pane. State comes from the host's pure projection.
use crate::agent::fleet::{AgentRow, AgentsView, DisplayState, FleetFilter};
use crate::app::app_trait::{App, AppCommand, AppRenderContext, KeyDisposition};

pub enum FleetAction {
    Seen(u64),
    Reply {
        pane_id: u64,
        from_pane_id: u64,
        text: String,
    },
    ReviewAttention {
        pane_id: u64,
    },
}

#[derive(Default)]
pub struct AgentsApp {
    view: AgentsView,
    filter: FleetFilter,
    selected: Option<u64>,
    replying: bool,
    focus_reply: bool,
    reply: String,
    commands: Vec<AppCommand>,
}
impl AgentsApp {
    fn rows(&self) -> Vec<&AgentRow> {
        self.view
            .tasks
            .iter()
            .flat_map(|t| &t.rows)
            .filter(|r| self.filter.matches(r))
            .collect()
    }
    fn jump(&mut self) {
        if let Some(pane_id) = self
            .selected
            .filter(|id| self.rows().iter().any(|r| r.pane_id == *id && r.live))
        {
            self.commands.push(AppCommand::ForwardPaneRequest {
                request: crate::protocol::AppRequest::FocusPane { pane_id },
            });
        }
    }
    fn answer(&mut self) {
        let row = self
            .rows()
            .into_iter()
            .find(|r| Some(r.pane_id) == self.selected);
        match row.and_then(|r| r.blocked_on.as_ref()) {
            Some(
                crate::agent::fleet::BlockedOn::Agent { .. }
                | crate::agent::fleet::BlockedOn::Message { .. },
            ) => {
                self.replying = true;
                self.focus_reply = true;
            }
            Some(_) => {
                if let Some(pane_id) = self.selected {
                    self.commands
                        .push(AppCommand::Fleet(FleetAction::ReviewAttention { pane_id }));
                }
            }
            None => {}
        }
    }
    fn dismiss(&mut self) {
        if let Some(row) = self
            .rows()
            .into_iter()
            .find(|r| Some(r.pane_id) == self.selected)
        {
            if row.display_state == DisplayState::DoneUnseen {
                self.commands
                    .push(AppCommand::Fleet(FleetAction::Seen(row.pane_id)));
            }
        }
    }
    fn indicator(
        row: &AgentRow,
        colors: &crate::ui::theme::Colors,
    ) -> (&'static str, egui::Color32) {
        match row.display_state {
            DisplayState::Blocked if row.needs_you => {
                ("!", crate::ui::activity::needs_you_color(colors))
            }
            DisplayState::Blocked => ("■", crate::ui::activity::blocked_color(colors)),
            DisplayState::Stalled | DisplayState::Failed => ("×", colors.state_error),
            DisplayState::DoneUnseen => ("○", colors.state_waiting),
            DisplayState::Working => ("◔", colors.state_working),
            DisplayState::Idle => ("○", colors.state_waiting),
        }
    }
}
impl App for AgentsApp {
    #[cfg(test)]
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn type_id(&self) -> &'static str {
        "agents"
    }
    fn display_name(&self) -> String {
        "Agents".into()
    }
    fn sync_agents_view(&mut self, view: &AgentsView) {
        self.view = view.clone();
        if !self.rows().iter().any(|r| Some(r.pane_id) == self.selected) {
            self.selected = self.rows().first().map(|r| r.pane_id);
            self.replying = false;
        }
    }
    fn semantic_state(&self) -> Option<serde_json::Value> {
        Some(
            serde_json::json!({"view":self.view, "selected":self.selected, "filter":self.filter, "replying":self.replying}),
        )
    }
    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        std::mem::take(&mut self.commands)
    }
    fn handle_key(&mut self, input: &crate::app::input_router::PlexiInput) -> KeyDisposition {
        if self.replying && input.key_pressed(egui::Key::Escape) {
            self.replying = false;
            return KeyDisposition::Consumed;
        }
        if input.modifiers().command
            || input.modifiers().ctrl
            || input.modifiers().alt
            || self.replying
        {
            return KeyDisposition::Passthrough;
        }
        // Native pane driving can deliver a key event; printable scene/OS input
        // can arrive as text. Treat the pair as one shortcut, never two actions.
        let pressed = |key, text: &str| {
            input.key_pressed(key)
                || input.events().iter().any(|event| {
                    matches!(event, egui::Event::Text(value) if value == text)
                })
        };
        if pressed(egui::Key::J, "j") || pressed(egui::Key::K, "k") {
            let rows = self.rows();
            let at = rows
                .iter()
                .position(|r| Some(r.pane_id) == self.selected)
                .unwrap_or(0);
            let next = if pressed(egui::Key::J, "j") {
                (at + 1).min(rows.len().saturating_sub(1))
            } else {
                at.saturating_sub(1)
            };
            self.selected = rows.get(next).map(|r| r.pane_id);
        } else if input.key_pressed(egui::Key::Enter) {
            self.jump();
        } else if pressed(egui::Key::A, "a") {
            self.answer();
        } else if pressed(egui::Key::X, "x") {
            self.dismiss();
        } else if pressed(egui::Key::F, "f") {
            self.filter = self.filter.next();
            self.selected = self.rows().first().map(|r| r.pane_id);
        } else {
            return KeyDisposition::Passthrough;
        }
        KeyDisposition::Consumed
    }
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &AppRenderContext<'_>,
        _pending_click: Option<crate::host::pane::PendingPaneClick>,
    ) {
        let colors = ctx.colors;
        egui::Frame::new()
            .fill(colors.terminal_bg)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Agents");
                    if ui.button(format!("{}  [f]", self.filter.label())).clicked() {
                        self.filter = self.filter.next();
                    }
                });
                ui.colored_label(
                    colors.text_dim,
                    "j/k select · Enter jump · a answer · x dismiss",
                );
                ui.separator();
                if self.rows().is_empty() {
                    ui.label("No matching agents");
                    return;
                }
                let width = ui.available_width();
                let height = ui.available_height();
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2((width * 0.46).max(120.0), height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_max_width((width * 0.46).max(120.0));
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                            egui::ScrollArea::vertical()
                                .id_salt("agents-tree")
                                .show(ui, |ui| {
                                    for task in &self.view.tasks {
                                        let rows: Vec<_> = task
                                            .rows
                                            .iter()
                                            .filter(|r| self.filter.matches(r))
                                            .collect();
                                        if rows.is_empty() {
                                            continue;
                                        }
                                        ui.strong(&task.title);
                                        ui.colored_label(colors.text_dim, task.counts());
                                        for row in rows {
                                            let (shape, color) = Self::indicator(row, colors);
                                            let label = format!(
                                                "{shape} {}  {}{}",
                                                row.role,
                                                row.display_state.label(),
                                                if row.needs_you { " · needs you" } else { "" }
                                            );
                                            if ui
                                                .selectable_label(
                                                    self.selected == Some(row.pane_id),
                                                    egui::RichText::new(label).color(color),
                                                )
                                                .clicked()
                                            {
                                                self.selected = Some(row.pane_id);
                                                self.replying = false;
                                            }
                                            if let Some(reason) = &row.blocked_on {
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(reason.label())
                                                            .color(colors.text_dim),
                                                    )
                                                    .truncate(),
                                                );
                                            }
                                            if row.display_state == DisplayState::Failed {
                                                if let Some(reason) = &row.tail {
                                                    ui.add(egui::Label::new(reason).truncate()).on_hover_text(reason);
                                                }
                                            }
                                            if !row.events.is_empty() {
                                                ui.small(format!(
                                                    "{} notifications",
                                                    row.events.len()
                                                ));
                                            }
                                        }
                                        ui.add_space(12.0);
                                    }
                                });
                        },
                    );
                    ui.separator();
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            let selected = self
                                .rows()
                                .into_iter()
                                .find(|r| Some(r.pane_id) == self.selected)
                                .cloned();
                            if let Some(row) = selected {
                                egui::ScrollArea::vertical().id_salt("agents-detail").show(
                                    ui,
                                    |ui| {
                                        ui.heading(&row.role);
                                        ui.label(format!(
                                            "{} · pane #{}",
                                            row.harness, row.pane_id
                                        ));
                                        ui.label(format!("Context: {}", row.context_name));
                                        ui.label(format!(
                                            "Parent: {}",
                                            row.parent_pane_id
                                                .map(|p| format!("#{p}"))
                                                .unwrap_or_else(|| "none".into())
                                        ));
                                        ui.label(format!(
                                            "State changed {}s ago",
                                            crate::platform::clock::now_secs()
                                                .saturating_sub(row.since)
                                        ));
                                        if let Some(reason) = &row.blocked_on {
                                            ui.label(reason.label());
                                        }
                                        ui.horizontal(|ui| {
                                            if ui
                                                .add_enabled(
                                                    row.live,
                                                    egui::Button::new("Jump [Enter]"),
                                                )
                                                .clicked()
                                            {
                                                self.jump();
                                            }
                                            if row.blocked_on.is_some()
                                                && ui.button("Answer [a]").clicked()
                                            {
                                                self.answer();
                                            }
                                            if row.display_state == DisplayState::DoneUnseen
                                                && ui.button("Dismiss [x]").clicked()
                                            {
                                                self.dismiss();
                                            }
                                        });
                                        if self.replying {
                                            ui.label("Reply to worker");
                                            let response = ui.add(
                                                egui::TextEdit::multiline(&mut self.reply)
                                                    .hint_text("Your answer"),
                                            );
                                            let surface = crate::ui::focus::SurfaceKey::Pane(ctx.pane_id);
                                            crate::ui::focus::register_text_surface(ui.ctx(), surface, response.id);
                                            if self.focus_reply {
                                                crate::ui::focus::claim_text_surface(ui.ctx(), surface, response.id);
                                                self.focus_reply = false;
                                            }
                                            if ui
                                                .add_enabled(
                                                    !self.reply.trim().is_empty(),
                                                    egui::Button::new("Send reply"),
                                                )
                                                .clicked()
                                            {
                                                self.commands.push(AppCommand::Fleet(
                                                    FleetAction::Reply {
                                                        pane_id: row.pane_id,
                                                        from_pane_id: ctx.pane_id,
                                                        text: std::mem::take(&mut self.reply),
                                                    },
                                                ));
                                                self.replying = false;
                                            }
                                        }
                                        if let Some(result) = &row.result {
                                            ui.separator();
                                            ui.strong("Result");
                                            ui.monospace(
                                                serde_json::to_string_pretty(result)
                                                    .unwrap_or_else(|e| {
                                                        format!("Result unavailable: {e}")
                                                    }),
                                            );
                                        }
                                        if let Some(tail) = &row.tail {
                                            ui.separator();
                                            ui.strong("Last output");
                                            ui.monospace(tail);
                                        }
                                        for event in &row.events {
                                            ui.separator();
                                            ui.strong(&event.title);
                                            ui.label(&event.body);
                                        }
                                    },
                                );
                            }
                        },
                    );
                });
            });
    }
}
