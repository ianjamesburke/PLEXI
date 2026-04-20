use crate::shell;
use egui::{Align2, Color32, CornerRadius, RichText, Stroke, Vec2};

use crate::app::PlexiApp;
use crate::overlays::MODAL_WIDTH;
use crate::widgets::selectable_row;

enum PaletteEntry {
    Pane {
        ctx_idx: usize,
        ctx_name: String,
        tile_id: egui_tiles::TileId,
        name: String,
        cwd: String,
    },
    App {
        id: String,
        name: String,
        description: String,
    },
}

impl PlexiApp {
    pub(crate) fn draw_command_palette(&mut self, ctx: &egui::Context) {
        let query = self.palette_query.to_lowercase();

        // ── Pane entries ────────────────────────────────────────────────────
        let mut entries: Vec<PaletteEntry> = Vec::new();

        for (ci, context) in self.contexts.iter().enumerate() {
            for (&pane_id, pane) in &context.panes {
                let Some(t) = pane.as_terminal() else {
                    continue;
                };
                let Some(display_name) = t.name.clone() else {
                    continue;
                };
                let cwd = shell::get_pid_cwd(t.backend.child_pid())
                    .as_deref()
                    .map(crate::app::PlexiApp::abbreviate_home_path)
                    .unwrap_or_else(|| crate::app::PlexiApp::abbreviate_home_path(&context.path));
                if let Some(tile_id) = context.tree.tiles.find_pane(&pane_id) {
                    if query.is_empty()
                        || display_name.to_lowercase().contains(&query)
                        || context.name.to_lowercase().contains(&query)
                        || cwd.to_lowercase().contains(&query)
                    {
                        entries.push(PaletteEntry::Pane {
                            ctx_idx: ci,
                            ctx_name: context.name.clone(),
                            tile_id,
                            name: display_name,
                            cwd,
                        });
                    }
                }
            }
        }

        // Sort panes by visit history
        entries.sort_by(|a, b| {
            let rank = |e: &PaletteEntry| match e {
                PaletteEntry::Pane {
                    ctx_idx, tile_id, ..
                } => self
                    .pane_visit_history
                    .iter()
                    .position(|&(c, t)| c == *ctx_idx && t == *tile_id)
                    .unwrap_or(usize::MAX),
                PaletteEntry::App { .. } => usize::MAX,
            };
            rank(a).cmp(&rank(b))
        });

        // ── App entries (appended after panes) ─────────────────────────────
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

        let total = entries.len();

        // Clamp selection
        if self.palette_selected >= total && total > 0 {
            self.palette_selected = total - 1;
        }

        // ── Keyboard nav ───────────────────────────────────────────────────
        #[derive(Clone)]
        enum Action {
            JumpPane(usize, egui_tiles::TileId),
            LaunchApp(String),
        }
        let mut action: Option<Action> = None;
        let prev_selected = self.palette_selected;

        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
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
                    Some(PaletteEntry::Pane {
                        ctx_idx, tile_id, ..
                    }) => {
                        action = Some(Action::JumpPane(*ctx_idx, *tile_id));
                    }
                    Some(PaletteEntry::App { id, .. }) => {
                        action = Some(Action::LaunchApp(id.clone()));
                    }
                    None => {}
                }
            }
        });

        match action {
            Some(Action::JumpPane(ctx_idx, tile_id)) => {
                self.record_pane_visit(ctx_idx, tile_id);
                self.active_context = ctx_idx;
                self.contexts[ctx_idx].focused_pane = Some(tile_id);
                self.contexts[ctx_idx].zoomed_pane = None;
                self.contexts[ctx_idx].activate_tab_for(tile_id);
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
            None => {}
        }

        if !self.show_command_palette {
            return;
        }

        // ── Render ─────────────────────────────────────────────────────────
        let screen_rect = ctx.screen_rect();
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
                                .hint_text("Jump to pane or launch app...")
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
                                RichText::new("No matching panes or apps")
                                    .size(11.0)
                                    .color(self.colors.text_dim),
                            );
                            return;
                        }

                        let current_ctx = self.active_context;
                        let current_focused = self.contexts[self.active_context].focused_pane;
                        let mut shown_apps_header = false;
                        let mut click_action: Option<Action> = None;
                        let mut hover_select: Option<usize> = None;
                        let colors = self.colors;
                        // Only let hover drive selection when the pointer moved this frame.
                        // A stationary mouse means keyboard navigation owns the index.
                        let mouse_moved = ctx.input(|i| i.pointer.delta().length_sq() > 0.5);
                        // Scroll the selected row into view when keyboard navigation moved it.
                        let should_scroll = self.palette_selected != prev_selected;

                        egui::ScrollArea::vertical()
                            .max_height(400.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.set_width(MODAL_WIDTH);

                                for (i, entry) in entries.iter().enumerate() {
                                    let is_selected = i == self.palette_selected;

                                    match entry {
                                        PaletteEntry::Pane {
                                            ctx_idx,
                                            ctx_name,
                                            tile_id,
                                            name,
                                            cwd,
                                        } => {
                                            let is_current = *ctx_idx == current_ctx
                                                && current_focused == Some(*tile_id);
                                            let name_color = if is_current {
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
                                                        ui.label(
                                                            RichText::new(ctx_name.as_str())
                                                                .size(10.0)
                                                                .color(colors.text_dim),
                                                        );
                                                        ui.label(
                                                            RichText::new("\u{203A}")
                                                                .size(10.0)
                                                                .color(colors.text_dim),
                                                        );
                                                        ui.label(
                                                            RichText::new(name.as_str())
                                                                .size(12.0)
                                                                .color(name_color),
                                                        );
                                                    });
                                                    if !cwd.is_empty() {
                                                        ui.label(
                                                            RichText::new(cwd.as_str())
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
                                                    Some(Action::JumpPane(*ctx_idx, *tile_id));
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
                                Action::JumpPane(ctx_idx, tile_id) => {
                                    self.record_pane_visit(ctx_idx, tile_id);
                                    self.active_context = ctx_idx;
                                    self.contexts[ctx_idx].focused_pane = Some(tile_id);
                                    self.contexts[ctx_idx].zoomed_pane = None;
                                    self.contexts[ctx_idx].activate_tab_for(tile_id);
                                    self.show_command_palette = false;
                                    self.palette_query.clear();
                                }
                                Action::LaunchApp(id) => {
                                    self.show_command_palette = false;
                                    self.palette_query.clear();
                                    self.launch_app_by_id(&id);
                                }
                            }
                        }
                    });
            });
    }
}
