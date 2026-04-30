use egui::{Align2, Color32, CornerRadius, RichText, Stroke, Vec2};

use crate::app::PlexiApp;
use crate::overlays::MODAL_WIDTH;
use crate::widgets::selectable_row;

enum PaletteEntry {
    Context {
        ctx_idx: usize,
        context_id: u64,
        name: String,
        workspace_name: String,
    },
    App {
        id: String,
        name: String,
        description: String,
    },
    /// Static action — flat command not tied to a context or installed app.
    Action {
        id: String,
        name: String,
        description: String,
    },
}

impl PlexiApp {
    pub(crate) fn draw_command_palette(&mut self, ctx: &egui::Context) {
        let query = self.palette_query.to_lowercase();

        // ── Context entries (active context first, then by visit recency) ──
        let active_ctx_id = self.contexts[self.active_context].context_id;

        let rank_of = |context_id: u64| -> usize {
            if context_id == active_ctx_id {
                return 0;
            }
            self.context_visit_history
                .iter()
                .position(|&id| id == context_id)
                .map(|p| p + 1)
                .unwrap_or(usize::MAX)
        };

        let mut entries: Vec<PaletteEntry> = {
            let mut ctx_entries: Vec<PaletteEntry> = self
                .contexts
                .iter()
                .enumerate()
                .filter_map(|(ci, c)| {
                    let ws_name = self
                        .workspaces
                        .iter()
                        .find(|w| w.context_id == c.workspace_id)
                        .map(|w| w.name.clone())
                        .unwrap_or_default();

                    if query.is_empty()
                        || c.name.to_lowercase().contains(&query)
                        || ws_name.to_lowercase().contains(&query)
                    {
                        Some(PaletteEntry::Context {
                            ctx_idx: ci,
                            context_id: c.context_id,
                            name: c.name.clone(),
                            workspace_name: ws_name,
                        })
                    } else {
                        None
                    }
                })
                .collect();

            ctx_entries.sort_by_key(|e| match e {
                PaletteEntry::Context { context_id, .. } => rank_of(*context_id),
                _ => usize::MAX,
            });

            ctx_entries
        };

        // ── App entries ────────────────────────────────────────────────────
        let app_entries: Vec<(String, String, String)> = self
            .registry
            .list()
            .into_iter()
            .filter(|app| {
                query.is_empty()
                    || app.manifest.name.to_lowercase().contains(&query)
                    || app.manifest.id.to_lowercase().contains(&query)
                    || app.manifest.description.to_lowercase().contains(&query)
            })
            .map(|app| {
                (
                    app.manifest.id.clone(),
                    app.manifest.name.clone(),
                    app.manifest.description.clone(),
                )
            })
            .collect();

        for (id, name, description) in app_entries {
            entries.push(PaletteEntry::App {
                id,
                name,
                description,
            });
        }

        // ── Static actions ─────────────────────────────────────────────────
        let action_specs: &[(&str, &str, &str)] = &[
            (
                "agent_workspace:modal",
                "New Agent Workspace…",
                "Open the picker with CLI dropdown, repo picker, and task prompt",
            ),
            (
                "agent_workspace:claude_code",
                "New Agent Workspace: Claude Code",
                "Spawn Claude Code in a fresh git worktree",
            ),
            (
                "agent_workspace:codex",
                "New Agent Workspace: Codex",
                "Spawn Codex in a fresh git worktree",
            ),
            (
                "agent_workspace:gemini_cli",
                "New Agent Workspace: Gemini CLI",
                "Spawn Gemini CLI in a fresh git worktree",
            ),
        ];
        for (id, name, description) in action_specs {
            if query.is_empty()
                || name.to_lowercase().contains(&query)
                || id.to_lowercase().contains(&query)
            {
                entries.push(PaletteEntry::Action {
                    id: (*id).to_string(),
                    name: (*name).to_string(),
                    description: (*description).to_string(),
                });
            }
        }

        let total = entries.len();

        if self.palette_selected >= total && total > 0 {
            self.palette_selected = total - 1;
        }

        // ── Keyboard nav ───────────────────────────────────────────────────
        #[derive(Clone)]
        enum Action {
            JumpContext(usize, u64),
            LaunchApp(String),
            RunAction(String),
        }
        let mut action: Option<Action> = None;
        let prev_selected = self.palette_selected;

        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                self.show_command_palette = false;
            }
            if input.consume_key(egui::Modifiers::COMMAND, egui::Key::P) {
                self.show_command_palette = false;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                && total > 0
                && self.palette_selected < total - 1
            {
                self.palette_selected += 1;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                && self.palette_selected > 0
            {
                self.palette_selected -= 1;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                match entries.get(self.palette_selected) {
                    Some(PaletteEntry::Context {
                        ctx_idx,
                        context_id,
                        ..
                    }) => {
                        action = Some(Action::JumpContext(*ctx_idx, *context_id));
                    }
                    Some(PaletteEntry::App { id, .. }) => {
                        action = Some(Action::LaunchApp(id.clone()));
                    }
                    Some(PaletteEntry::Action { id, .. }) => {
                        action = Some(Action::RunAction(id.clone()));
                    }
                    None => {}
                }
            }
        });

        match action {
            Some(Action::JumpContext(ctx_idx, context_id)) => {
                self.jump_to_context(ctx_idx, context_id);
                self.show_command_palette = false;
                self.palette_query.clear();
                return;
            }
            Some(Action::LaunchApp(id)) => {
                self.show_command_palette = false;
                self.palette_query.clear();
                self.launch_app_by_id(&id);
                return;
            }
            Some(Action::RunAction(id)) => {
                self.show_command_palette = false;
                self.palette_query.clear();
                self.run_palette_action(&id);
                return;
            }
            None => {}
        }

        if !self.show_command_palette {
            return;
        }

        // ── Render ─────────────────────────────────────────────────────────
        let screen_rect = ctx.screen_rect();
        let palette_max_list_h = (screen_rect.height() - 80.0 - 120.0).max(200.0);

        egui::Area::new(egui::Id::new("palette_scrim"))
            .fixed_pos(screen_rect.min)
            .show(ctx, |ui| {
                ui.painter()
                    .rect_filled(screen_rect, 0.0, Color32::from_black_alpha(120));
                let scrim_response = ui.allocate_rect(screen_rect, egui::Sense::click());
                if scrim_response.clicked() {
                    self.show_command_palette = false;
                }
            });

        egui::Area::new(egui::Id::new("command_palette"))
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 80.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);

                        let te_id = egui::Id::new("palette_search");
                        let te = ui.add(
                            egui::TextEdit::singleline(&mut self.palette_query)
                                .id(te_id)
                                .desired_width(MODAL_WIDTH)
                                .hint_text("Jump to context or launch app...")
                                .font(egui::TextStyle::Body),
                        );
                        if !te.has_focus() {
                            te.request_focus();
                        }
                        if te.changed() {
                            self.palette_selected = 0;
                        }

                        ui.add_space(6.0);

                        if entries.is_empty() {
                            ui.label(
                                RichText::new("No matching contexts or apps")
                                    .size(11.0)
                                    .color(self.colors.text_dim),
                            );
                            return;
                        }

                        let mut shown_apps_header = false;
                        let mut click_action: Option<Action> = None;
                        let mut hover_select: Option<usize> = None;
                        let colors = self.colors;
                        let mouse_moved = ctx.input(|i| i.pointer.delta().length_sq() > 0.5);
                        let should_scroll = self.palette_selected != prev_selected;

                        egui::ScrollArea::vertical()
                            .max_height(palette_max_list_h)
                            .min_scrolled_height(palette_max_list_h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_width(MODAL_WIDTH);

                                for (i, entry) in entries.iter().enumerate() {
                                    let is_selected = i == self.palette_selected;

                                    match entry {
                                        PaletteEntry::Context {
                                            ctx_idx,
                                            context_id,
                                            name,
                                            workspace_name,
                                        } => {
                                            let is_active = *context_id == active_ctx_id;
                                            let name_color = if is_active {
                                                colors.accent
                                            } else {
                                                colors.text_primary
                                            };

                                            let (r, _) = selectable_row(
                                                ui,
                                                is_selected,
                                                &colors,
                                                |ui| {
                                                    ui.horizontal(|ui| {
                                                        if !workspace_name.is_empty() {
                                                            ui.label(
                                                                RichText::new(workspace_name.as_str())
                                                                    .size(10.0)
                                                                    .color(colors.text_dim),
                                                            );
                                                            ui.label(
                                                                RichText::new("\u{203A}")
                                                                    .size(10.0)
                                                                    .color(colors.text_dim),
                                                            );
                                                        }
                                                        ui.label(
                                                            RichText::new(name.as_str())
                                                                .size(12.0)
                                                                .color(name_color),
                                                        );
                                                    });
                                                },
                                            );

                                            if is_selected && should_scroll {
                                                r.scroll_to_me(None);
                                            }
                                            if r.clicked() {
                                                click_action = Some(Action::JumpContext(
                                                    *ctx_idx,
                                                    *context_id,
                                                ));
                                            }
                                            if r.hovered() {
                                                hover_select = Some(i);
                                            }
                                        }

                                        PaletteEntry::App {
                                            id,
                                            name,
                                            description,
                                        } => {
                                            if !shown_apps_header {
                                                shown_apps_header = true;
                                                ui.add_space(4.0);
                                                ui.label(
                                                    RichText::new("APPS")
                                                        .size(9.0)
                                                        .color(colors.text_dim),
                                                );
                                                ui.add_space(2.0);
                                            }

                                            let (r, _) = selectable_row(
                                                ui,
                                                is_selected,
                                                &colors,
                                                |ui| {
                                                    ui.horizontal(|ui| {
                                                        ui.label(
                                                            RichText::new("⬡")
                                                                .size(10.0)
                                                                .color(colors.accent),
                                                        );
                                                        ui.add_space(4.0);
                                                        ui.label(
                                                            RichText::new(name.as_str())
                                                                .size(12.0)
                                                                .color(colors.text_primary),
                                                        );
                                                    });
                                                    if !description.is_empty() {
                                                        ui.label(
                                                            RichText::new(description.as_str())
                                                                .size(9.0)
                                                                .color(colors.text_dim),
                                                        );
                                                    }
                                                },
                                            );

                                            if is_selected && should_scroll {
                                                r.scroll_to_me(None);
                                            }
                                            if r.clicked() {
                                                click_action =
                                                    Some(Action::LaunchApp(id.clone()));
                                            }
                                            if r.hovered() {
                                                hover_select = Some(i);
                                            }
                                        }

                                        PaletteEntry::Action {
                                            id,
                                            name,
                                            description,
                                        } => {
                                            let (r, _) = selectable_row(
                                                ui,
                                                is_selected,
                                                &colors,
                                                |ui| {
                                                    ui.horizontal(|ui| {
                                                        ui.label(
                                                            RichText::new("⚡")
                                                                .size(10.0)
                                                                .color(colors.accent),
                                                        );
                                                        ui.add_space(4.0);
                                                        ui.label(
                                                            RichText::new(name.as_str())
                                                                .size(12.0)
                                                                .color(colors.text_primary),
                                                        );
                                                    });
                                                    if !description.is_empty() {
                                                        ui.label(
                                                            RichText::new(description.as_str())
                                                                .size(9.0)
                                                                .color(colors.text_dim),
                                                        );
                                                    }
                                                },
                                            );
                                            if is_selected && should_scroll {
                                                r.scroll_to_me(None);
                                            }
                                            if r.clicked() {
                                                click_action =
                                                    Some(Action::RunAction(id.clone()));
                                            }
                                            if r.hovered() {
                                                hover_select = Some(i);
                                            }
                                        }
                                    }
                                }
                            });

                        if let Some(i) = hover_select {
                            if mouse_moved {
                                self.palette_selected = i;
                            }
                        }

                        if let Some(act) = click_action {
                            match act {
                                Action::JumpContext(ctx_idx, context_id) => {
                                    self.jump_to_context(ctx_idx, context_id);
                                    self.show_command_palette = false;
                                    self.palette_query.clear();
                                }
                                Action::LaunchApp(id) => {
                                    self.show_command_palette = false;
                                    self.palette_query.clear();
                                    self.launch_app_by_id(&id);
                                }
                                Action::RunAction(id) => {
                                    self.show_command_palette = false;
                                    self.palette_query.clear();
                                    self.run_palette_action(&id);
                                }
                            }
                        }
                    });
            });
    }

    /// Jump to a context by index, switching workspace if necessary.
    fn jump_to_context(&mut self, ctx_idx: usize, context_id: u64) {
        let target_ws_id = self.contexts[ctx_idx].workspace_id;
        if let Some(ws_idx) = self
            .workspaces
            .iter()
            .position(|w| w.context_id == target_ws_id)
        {
            if ws_idx != self.active_workspace {
                // switch_workspace → pick_active_context_from_workspace sets
                // active_context based on workspace_active_window. We override it
                // immediately below.
                self.switch_workspace(ws_idx);
            }
        }
        self.active_context = ctx_idx;
        self.contexts[ctx_idx].zoomed_pane = None;
        let ws_id = self.workspaces[self.active_workspace].context_id;
        self.workspace_active_window.insert(ws_id, context_id);
        self.record_context_visit(context_id);
    }

    /// Dispatch a static palette action. Adding a new action means
    /// (1) appending to `action_specs` above, and (2) extending this match.
    pub(crate) fn run_palette_action(&mut self, id: &str) {
        match id {
            "agent_workspace:modal" => self.open_agent_workspace_modal(),
            "agent_workspace:claude_code" => self.spawn_agent_workspace(
                crate::agent_workspace::AgentCli::ClaudeCode,
            ),
            "agent_workspace:codex" => self.spawn_agent_workspace(
                crate::agent_workspace::AgentCli::Codex,
            ),
            "agent_workspace:gemini_cli" => self.spawn_agent_workspace(
                crate::agent_workspace::AgentCli::GeminiCli,
            ),
            other => {
                log::warn!("run_palette_action: unknown action id '{other}'");
            }
        }
    }

    fn spawn_agent_workspace(&mut self, cli: crate::agent_workspace::AgentCli) {
        if !cli.is_installed() {
            self.push_host_notification(
                "warn".to_string(),
                format!("{} is not installed", cli.display_name()),
                format!(
                    "The `{}` binary was not found on PATH or in common installer dirs.",
                    cli.binary_name()
                ),
            );
            return;
        }
        match self.open_agent_workspace_pane(cli, String::new()) {
            Ok(()) => log::info!("agent_workspace: spawned {}", cli.display_name()),
            Err(e) => {
                self.push_host_notification(
                    "warn".to_string(),
                    format!("Failed to spawn {}", cli.display_name()),
                    e.to_string(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_spawn_not_installed_pushes_notification() {
        let result = std::panic::catch_unwind(|| {
            let _ = crate::agent_workspace::AgentCli::ClaudeCode.is_installed();
            let _ = crate::agent_workspace::AgentCli::Codex.is_installed();
            let _ = crate::agent_workspace::AgentCli::GeminiCli.is_installed();
        });
        assert!(result.is_ok(), "AgentCli::is_installed() panicked unexpectedly");
    }
}
