use crate::shell;
use egui::{Align, Align2, Color32, CornerRadius, Layout, Rect, RichText, Stroke, Vec2};

use crate::app::PlexiApp;
use crate::overlays::MODAL_WIDTH;

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
                let Some(display_name) = pane.name.clone() else { continue };
                let cwd = shell::get_pid_cwd(pane.backend.child_pid())
                    .as_deref()
                    .map(crate::app::PlexiApp::abbreviate_home_path)
                    .unwrap_or_else(|| {
                        crate::app::PlexiApp::abbreviate_home_path(&context.path)
                    });
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
                PaletteEntry::Pane { ctx_idx, tile_id, .. } => self
                    .pane_visit_history
                    .iter()
                    .position(|&(c, t)| c == *ctx_idx && t == *tile_id)
                    .unwrap_or(usize::MAX),
                PaletteEntry::App { .. } => usize::MAX,
            };
            rank(a).cmp(&rank(b))
        });

        // ── App entries (appended after panes, sorted by MRU) ─────────────
        // Collect outside the borrow of self.registry to avoid borrow conflicts
        let mut app_entries: Vec<(String, String, String)> = self
            .registry
            .list()
            .into_iter()
            .filter(|app| {
                query.is_empty()
                    || app.manifest.name.to_lowercase().contains(&query)
                    || app.manifest.id.to_lowercase().contains(&query)
                    || app.manifest.description.to_lowercase().contains(&query)
            })
            .map(|app| (app.manifest.id.clone(), app.manifest.name.clone(), app.manifest.description.clone()))
            .collect();

        // Sort by MRU: apps in visit history first (by recency), then alphabetical
        let mru = &self.app_visit_history;
        app_entries.sort_by(|(id_a, name_a, _), (id_b, name_b, _)| {
            let rank_a = mru.iter().position(|s| s == id_a).unwrap_or(usize::MAX);
            let rank_b = mru.iter().position(|s| s == id_b).unwrap_or(usize::MAX);
            rank_a.cmp(&rank_b).then_with(|| name_a.cmp(name_b))
        });

        for (id, name, description) in app_entries {
            entries.push(PaletteEntry::App { id, name, description });
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

        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                self.show_command_palette = false;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                && total > 0 && self.palette_selected < total - 1
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
                    Some(PaletteEntry::Pane { ctx_idx, tile_id, .. }) => {
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
                ui.painter().rect_filled(screen_rect, 0.0, Color32::from_black_alpha(120));
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
                        if !te.has_focus() { te.request_focus(); }
                        if te.changed() { self.palette_selected = 0; }

                        ui.add_space(6.0);

                        let current_ctx = self.active_context;
                        let current_focused = self.contexts[self.active_context].focused_pane;

                        // Fixed list height — never adjusts with content so the modal
                        // stays the same size regardless of how many items are visible.
                        let screen_h = ctx.screen_rect().height();
                        let list_height = (screen_h * 0.60).max(320.0).min(screen_h - 200.0);

                        // Track whether we've drawn the Apps section header
                        let mut shown_apps_header = false;
                        let mut click_action: Option<Action> = None;
                        let mut selected_rect: Option<egui::Rect> = None;

                        let scroll_id = egui::Id::new("palette_scroll");
                        egui::ScrollArea::vertical()
                            .id_salt(scroll_id)
                            .max_height(list_height)
                            .min_scrolled_height(list_height)
                            .show(ui, |ui| {

                        for (i, entry) in entries.iter().enumerate() {
                            let is_selected = i == self.palette_selected;

                            match entry {
                                PaletteEntry::Pane { ctx_idx, ctx_name, tile_id, name, cwd } => {
                                    let is_current = *ctx_idx == current_ctx
                                        && current_focused == Some(*tile_id);
                                    let fill = if is_selected { self.colors.bg_active } else { Color32::TRANSPARENT };

                                    let row_rect = Rect::from_min_size(ui.cursor().min, Vec2::new(MODAL_WIDTH, 36.0));
                                    ui.painter().rect_filled(row_rect, CornerRadius::same(4), fill);
                                    if is_selected { selected_rect = Some(row_rect); }

                                    ui.allocate_ui_with_layout(
                                        Vec2::new(MODAL_WIDTH, 36.0),
                                        Layout::left_to_right(Align::Center),
                                        |ui| {
                                            ui.add_space(8.0);
                                            ui.vertical(|ui| {
                                                ui.add_space(2.0);
                                                ui.horizontal(|ui| {
                                                    ui.label(RichText::new(ctx_name).size(10.0).color(self.colors.text_dim));
                                                    ui.label(RichText::new("\u{203A}").size(10.0).color(self.colors.text_dim));
                                                    let name_color = if is_current { self.colors.accent } else { self.colors.text_primary };
                                                    ui.label(RichText::new(name).size(12.0).color(name_color));
                                                });
                                                if !cwd.is_empty() {
                                                    ui.label(RichText::new(cwd).size(9.0).color(self.colors.text_dim));
                                                }
                                            });
                                        },
                                    );

                                    let r = ui.interact(row_rect, egui::Id::new(("palette_row", i)), egui::Sense::click());
                                    if r.clicked() {
                                        click_action = Some(Action::JumpPane(*ctx_idx, *tile_id));
                                    }
                                    if r.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                                }

                                PaletteEntry::App { id, name, description } => {
                                    // Section header on first app entry
                                    if !shown_apps_header {
                                        shown_apps_header = true;
                                        ui.add_space(4.0);
                                        ui.label(RichText::new("APPS").size(9.0).color(self.colors.text_dim));
                                        ui.add_space(2.0);
                                    }

                                    let fill = if is_selected { self.colors.bg_active } else { Color32::TRANSPARENT };
                                    let row_rect = Rect::from_min_size(ui.cursor().min, Vec2::new(MODAL_WIDTH, 36.0));
                                    ui.painter().rect_filled(row_rect, CornerRadius::same(4), fill);
                                    if is_selected { selected_rect = Some(row_rect); }

                                    ui.allocate_ui_with_layout(
                                        Vec2::new(MODAL_WIDTH, 36.0),
                                        Layout::left_to_right(Align::Center),
                                        |ui| {
                                            ui.add_space(8.0);
                                            ui.vertical(|ui| {
                                                ui.add_space(2.0);
                                                ui.horizontal(|ui| {
                                                    ui.label(RichText::new("⬡").size(10.0).color(self.colors.accent));
                                                    ui.add_space(4.0);
                                                    ui.label(RichText::new(name).size(12.0).color(self.colors.text_primary));
                                                });
                                                if !description.is_empty() {
                                                    let desc = if description.len() > 58 {
                                                        format!("{}…", &description[..58])
                                                    } else {
                                                        description.clone()
                                                    };
                                                    ui.label(RichText::new(desc).size(9.0).color(self.colors.text_dim));
                                                }
                                            });
                                        },
                                    );

                                    let r = ui.interact(row_rect, egui::Id::new(("palette_row", i)), egui::Sense::click());
                                    if r.clicked() {
                                        click_action = Some(Action::LaunchApp(id.clone()));
                                    }
                                    if r.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                                }
                            }
                        }

                        if entries.is_empty() {
                            ui.label(RichText::new("No matching panes or apps").size(11.0).color(self.colors.text_dim));
                        }

                        }); // end ScrollArea

                        // Scroll to keep selected row visible
                        if let Some(rect) = selected_rect {
                            ui.scroll_to_rect(rect, None);
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
