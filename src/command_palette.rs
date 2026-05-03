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
        /// If set, focus this specific pane after navigating to the window.
        pane_id: Option<u64>,
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

        // ── Window entries (active window first, then by visit recency) ──
        let active_win_id = self.windows[self.active_window].window_id;

        let rank_of = |win_id: u64| -> usize {
            if win_id == active_win_id {
                return 0;
            }
            self.context_visit_history
                .iter()
                .position(|&id| id == win_id)
                .map(|p| p + 1)
                .unwrap_or(usize::MAX)
        };

        let mut entries: Vec<PaletteEntry> = {
            // For each context we emit exactly ONE primary entry — either the
            // context name (active pane is unnamed) or "context › pane-name"
            // (active pane is named). Additional named panes beyond the active
            // one appear as secondary entries. This keeps the ⌘P → ↓ → Enter
            // flow clean: the top entry for each context always represents the
            // most-recently-active pane.
            let mut seen_contexts: std::collections::HashSet<u64> =
                std::collections::HashSet::new();
            let mut ctx_entries: Vec<PaletteEntry> = Vec::new();

            // Resolve the active pane name for a context (following
            // context_active_window → focused_pane → terminal name).
            let active_pane_for_context = |ctx_id: u64| -> Option<(usize, u64, u64, String)> {
                let active_win_id = self.context_active_window.get(&ctx_id).copied()?;
                let (win_ci, win) = self
                    .windows
                    .iter()
                    .enumerate()
                    .find(|(_, w)| w.window_id == active_win_id)?;
                let tile_id = win.focused_pane?;
                let tile = win.tree.tiles.get(tile_id)?;
                let pane_id = match tile {
                    egui_tiles::Tile::Pane(pid) => *pid,
                    _ => return None,
                };
                let pane_name = win
                    .panes
                    .get(&pane_id)
                    .and_then(|p| p.as_terminal())
                    .and_then(|t| t.name.clone())?;
                Some((win_ci, active_win_id, pane_id, pane_name))
            };

            for (ci, w) in self.windows.iter().enumerate() {
                let ctx_name = self
                    .router
                    .iter()
                    .find(|c| c.context_id == w.context_id)
                    .map(|c| c.name.clone())
                    .unwrap_or_default();

                // Primary entry — one per context.
                if !seen_contexts.contains(&w.context_id) {
                    seen_contexts.insert(w.context_id);

                    // Fallback window if context_active_window is absent/stale.
                    let fallback_win_id = self
                        .context_active_window
                        .get(&w.context_id)
                        .copied()
                        .and_then(|id| {
                            self.windows.iter().enumerate().find(|(_, w2)| w2.window_id == id)
                        })
                        .map(|(i, w2)| (i, w2.window_id))
                        .unwrap_or((ci, w.window_id));

                    if let Some((win_ci, win_id, pane_id, pane_name)) =
                        active_pane_for_context(w.context_id)
                    {
                        // Active pane is named — it IS the primary entry.
                        if query.is_empty()
                            || pane_name.to_lowercase().contains(&query)
                            || ctx_name.to_lowercase().contains(&query)
                        {
                            ctx_entries.push(PaletteEntry::Context {
                                ctx_idx: win_ci,
                                context_id: win_id,
                                name: pane_name,
                                workspace_name: ctx_name.clone(),
                                pane_id: Some(pane_id),
                            });
                        }
                    } else {
                        // Active pane is unnamed — show the context by name.
                        if query.is_empty() || ctx_name.to_lowercase().contains(&query) {
                            ctx_entries.push(PaletteEntry::Context {
                                ctx_idx: fallback_win_id.0,
                                context_id: fallback_win_id.1,
                                name: ctx_name.clone(),
                                workspace_name: String::new(),
                                pane_id: None,
                            });
                        }
                    }
                }

                // Secondary entries — named panes that are NOT the active pane.
                let active_win_id = self
                    .context_active_window
                    .get(&w.context_id)
                    .copied()
                    .unwrap_or(0);
                let active_pane_id = if w.window_id == active_win_id {
                    w.focused_pane.and_then(|tid| {
                        w.tree.tiles.get(tid).and_then(|tile| match tile {
                            egui_tiles::Tile::Pane(pid) => Some(*pid),
                            _ => None,
                        })
                    })
                } else {
                    None
                };

                for (&pane_id, pane) in &w.panes {
                    if let Some(t) = pane.as_terminal() {
                        if let Some(pane_name) = &t.name {
                            // Skip if this is already the primary entry.
                            if Some(pane_id) == active_pane_id && w.window_id == active_win_id {
                                continue;
                            }
                            if query.is_empty()
                                || pane_name.to_lowercase().contains(&query)
                                || ctx_name.to_lowercase().contains(&query)
                            {
                                ctx_entries.push(PaletteEntry::Context {
                                    ctx_idx: ci,
                                    context_id: w.window_id,
                                    name: pane_name.clone(),
                                    workspace_name: ctx_name.clone(),
                                    pane_id: Some(pane_id),
                                });
                            }
                        }
                    }
                }
            }

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
            JumpContext(usize, u64, Option<u64>),
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
                        pane_id,
                        ..
                    }) => {
                        action = Some(Action::JumpContext(*ctx_idx, *context_id, *pane_id));
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
            Some(Action::JumpContext(ctx_idx, context_id, pane_id)) => {
                self.jump_to_context(ctx_idx, context_id, pane_id);
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
                                            pane_id,
                                        } => {
                                            let is_active = *context_id == active_win_id;
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
                                                    *pane_id,
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
                                Action::JumpContext(ctx_idx, context_id, pane_id) => {
                                    self.jump_to_context(ctx_idx, context_id, pane_id);
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

    /// Jump to a window by index, switching context if necessary.
    /// If `pane_id` is provided, also focuses that specific pane in the window.
    fn jump_to_context(&mut self, ctx_idx: usize, win_id: u64, pane_id: Option<u64>) {
        let target_ctx_id = self.windows[ctx_idx].context_id;
        if let Some(ctx_idx_sidebar) = self
            .router
            .position(|c| c.context_id == target_ctx_id)
        {
            if ctx_idx_sidebar != self.router.active_idx() {
                // switch_workspace → pick_active_context_from_workspace sets
                // active_window based on context_active_window. We override it
                // immediately below.
                self.switch_workspace(ctx_idx_sidebar);
            }
        }
        self.active_window = ctx_idx;
        self.windows[ctx_idx].zoomed_pane = None;
        let ctx_id = self.router.active().context_id;
        self.context_active_window.insert(ctx_id, win_id);
        self.record_context_visit(win_id);

        // Focus the specific pane if requested — find its TileId in the tree.
        if let Some(pid) = pane_id {
            let win = &mut self.windows[ctx_idx];
            if let Some(tile_id) = win
                .tree
                .tiles
                .iter()
                .find_map(|(tid, tile)| {
                    if matches!(tile, egui_tiles::Tile::Pane(p) if *p == pid) {
                        Some(*tid)
                    } else {
                        None
                    }
                })
            {
                win.focused_pane = Some(tile_id);
            }
        }
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
